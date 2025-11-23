use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use regex::Regex;
use std::path::Path;
use std::process::{Command, Stdio};

const SECTOR_SIZE: u64 = 512;
const ALIGNMENT: u64 = 2048; // Sector alignment boundary
const MIN_ROOT_SIZE_GB: u64 = 8;
const MAX_ROOT_SIZE_GB: u64 = 64;

#[derive(Parser, Debug)]
#[command(author, version, about = "Shrink RPi root filesystem and create partitions", long_about = None)]
struct Args {
    /// Root filesystem size (e.g., 8G, 16G). Min: 8G, Max: 64G
    #[arg(short = 'r', long, value_name = "SIZE")]
    root_size: String,

    /// Swap partition size (e.g., 4G, 8G). Not created on SD cards
    #[arg(short = 's', long, value_name = "SIZE")]
    swap_size: Option<String>,

    /// /var partition size (e.g., 4G, 8G). Not created on SD cards
    #[arg(short = 'v', long, value_name = "SIZE")]
    var_size: Option<String>,

    /// Target device (e.g., /dev/mmcblk0, /dev/sda)
    #[arg(short = 'd', long, value_name = "DEVICE")]
    device: String,

    /// Dry run - show what would be done without making changes
    #[arg(long)]
    dry_run: bool,

    /// Force operation even on SD cards for swap/var
    #[arg(short = 'f', long)]
    force: bool,
}

#[derive(Debug)]
struct DiskInfo {
    device: String,
    size_bytes: u64,
    size_sectors: u64,
    is_sd_card: bool,
    root_partition: String,
}

#[derive(Debug)]
struct PartitionLayout {
    root_size_bytes: u64,
    swap_size_bytes: u64,
    var_size_bytes: u64,
    home_size_bytes: u64,
    root_start: u64,
    root_end: u64,
    swap_start: u64,
    swap_end: u64,
    var_start: u64,
    var_end: u64,
    home_start: u64,
    home_end: u64,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Check if running as root
    if !is_root() {
        bail!("This program must be run as root");
    }

    println!("RPi Filesystem Shrink Tool");
    println!("==========================\n");

    // Check and install dependencies
    check_dependencies(args.dry_run)?;

    // Parse sizes
    let root_size = parse_size(&args.root_size)?;
    validate_root_size(root_size)?;

    let swap_size = args.swap_size.as_ref().map(|s| parse_size(s)).transpose()?;
    let var_size = args.var_size.as_ref().map(|s| parse_size(s)).transpose()?;

    // Get disk information
    let disk_info = get_disk_info(&args.device)?;
    println!("Disk Information:");
    println!("  Device: {}", disk_info.device);
    println!("  Size: {} GB ({} bytes)", disk_info.size_bytes / (1024 * 1024 * 1024), disk_info.size_bytes);
    println!("  Is SD Card: {}", disk_info.is_sd_card);
    println!("  Root Partition: {}\n", disk_info.root_partition);

    // Check SD card constraints
    if disk_info.is_sd_card && !args.force {
        if swap_size.is_some() {
            println!("WARNING: Swap partition not recommended on SD cards (use -f to force)");
        }
        if var_size.is_some() {
            println!("WARNING: Separate /var partition not recommended on SD cards (use -f to force)");
        }
    }

    // Calculate partition layout
    let layout = calculate_partition_layout(
        &disk_info,
        root_size,
        if disk_info.is_sd_card && !args.force { None } else { swap_size },
        if disk_info.is_sd_card && !args.force { None } else { var_size },
    )?;

    print_layout(&layout);

    if args.dry_run {
        println!("\n=== DRY RUN MODE - No changes will be made ===");
        return Ok(());
    }

    // Confirm with user
    println!("\nWARNING: This will modify your disk partitions!");
    println!("Press Enter to continue or Ctrl+C to cancel...");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;

    // Perform the operations
    println!("\n=== Starting partition operations ===\n");

    // Step 1: Unmount root filesystem (if possible)
    println!("Step 1: Checking filesystem...");
    check_filesystem(&disk_info.root_partition)?;

    // Step 2: Shrink root filesystem
    println!("\nStep 2: Shrinking root filesystem to {} bytes...", layout.root_size_bytes);
    shrink_root_filesystem(&disk_info.root_partition, layout.root_size_bytes)?;

    // Step 3: Resize root partition
    println!("\nStep 3: Resizing root partition...");
    resize_root_partition(&disk_info, layout.root_end)?;

    // Step 4: Create swap partition (if requested)
    if layout.swap_size_bytes > 0 {
        println!("\nStep 4: Creating swap partition...");
        create_swap_partition(&disk_info, layout.swap_start, layout.swap_end)?;
    }

    // Step 5: Create /var partition (if requested)
    if layout.var_size_bytes > 0 {
        println!("\nStep 5: Creating /var partition...");
        create_var_partition(&disk_info, layout.var_start, layout.var_end)?;
    }

    // Step 6: Create /home partition
    println!("\nStep 6: Creating /home partition...");
    create_home_partition(&disk_info, layout.home_start, layout.home_end)?;

    println!("\n=== Success! ===");
    println!("\nPartitions created successfully!");
    println!("\nNext steps:");
    if layout.swap_size_bytes > 0 {
        println!("  1. Add swap to /etc/fstab");
    }
    if layout.var_size_bytes > 0 {
        println!("  2. Mount /var partition and migrate data");
    }
    println!("  3. Mount /home partition and migrate user data");
    println!("  4. Reboot to verify changes");

    Ok(())
}

fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

fn check_dependencies(dry_run: bool) -> Result<()> {
    println!("Checking dependencies...");

    let dependencies = vec![
        ("parted", "parted"),
        ("resize2fs", "e2fsprogs"),
        ("mkfs.ext4", "e2fsprogs"),
        ("mkfs.btrfs", "btrfs-progs"),
        ("mkswap", "util-linux"),
    ];

    let mut missing = Vec::new();

    for (cmd, package) in &dependencies {
        if !command_exists(cmd) {
            println!("  Missing: {} (package: {})", cmd, package);
            missing.push(*package);
        } else {
            println!("  Found: {}", cmd);
        }
    }

    if !missing.is_empty() {
        if dry_run {
            println!("\nWould install: {:?}", missing);
        } else {
            println!("\nInstalling missing dependencies...");
            install_packages(&missing)?;
        }
    }

    println!();
    Ok(())
}

fn command_exists(cmd: &str) -> bool {
    Command::new("which")
        .arg(cmd)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn install_packages(packages: &[&str]) -> Result<()> {
    // Try apt-get first (Debian/Ubuntu/Raspbian)
    if command_exists("apt-get") {
        let status = Command::new("apt-get")
            .arg("update")
            .status()
            .context("Failed to run apt-get update")?;

        if !status.success() {
            bail!("apt-get update failed");
        }

        for package in packages {
            let status = Command::new("apt-get")
                .args(&["install", "-y", package])
                .status()
                .context(format!("Failed to install {}", package))?;

            if !status.success() {
                bail!("Failed to install {}", package);
            }
        }
        return Ok(());
    }

    // Try yum (RHEL/CentOS)
    if command_exists("yum") {
        for package in packages {
            let status = Command::new("yum")
                .args(&["install", "-y", package])
                .status()
                .context(format!("Failed to install {}", package))?;

            if !status.success() {
                bail!("Failed to install {}", package);
            }
        }
        return Ok(());
    }

    bail!("No supported package manager found (apt-get or yum)");
}

fn parse_size(size_str: &str) -> Result<u64> {
    let size_str = size_str.trim().to_uppercase();
    let re = Regex::new(r"^(\d+(?:\.\d+)?)\s*([KMGT]?)B?$")?;

    let caps = re
        .captures(&size_str)
        .ok_or_else(|| anyhow!("Invalid size format: {}", size_str))?;

    let number: f64 = caps[1].parse()?;
    let unit = caps.get(2).map_or("", |m| m.as_str());

    let multiplier: u64 = match unit {
        "" => 1,
        "K" => 1024,
        "M" => 1024 * 1024,
        "G" => 1024 * 1024 * 1024,
        "T" => 1024u64 * 1024 * 1024 * 1024,
        _ => bail!("Unknown size unit: {}", unit),
    };

    Ok((number * multiplier as f64) as u64)
}

fn validate_root_size(size: u64) -> Result<()> {
    let min_size = MIN_ROOT_SIZE_GB * 1024 * 1024 * 1024;
    let max_size = MAX_ROOT_SIZE_GB * 1024 * 1024 * 1024;

    if size < min_size {
        bail!("Root size must be at least {}G", MIN_ROOT_SIZE_GB);
    }

    if size > max_size {
        bail!("Root size must not exceed {}G", MAX_ROOT_SIZE_GB);
    }

    Ok(())
}

fn get_disk_info(device: &str) -> Result<DiskInfo> {
    // Normalize device path
    let device = if !device.starts_with("/dev/") {
        format!("/dev/{}", device)
    } else {
        device.to_string()
    };

    // Check if device exists
    if !Path::new(&device).exists() {
        bail!("Device {} does not exist", device);
    }

    // Determine if it's an SD card
    let is_sd_card = device.contains("mmcblk");

    // Get disk size using parted
    let output = Command::new("parted")
        .args(&[&device, "unit", "B", "print"])
        .output()
        .context("Failed to run parted")?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse disk size
    let size_re = Regex::new(r"Disk /[^:]+:\s*(\d+)B")?;
    let size_bytes = size_re
        .captures(&stdout)
        .and_then(|c| c[1].parse::<u64>().ok())
        .ok_or_else(|| anyhow!("Could not determine disk size"))?;

    let size_sectors = size_bytes / SECTOR_SIZE;

    // Determine root partition (usually partition 2 on RPi)
    let root_partition = if is_sd_card {
        format!("{}p2", device)
    } else {
        format!("{}2", device)
    };

    // Verify root partition exists
    if !Path::new(&root_partition).exists() {
        bail!("Root partition {} does not exist", root_partition);
    }

    Ok(DiskInfo {
        device,
        size_bytes,
        size_sectors,
        is_sd_card,
        root_partition,
    })
}

fn align_sector(sector: u64) -> u64 {
    ((sector + ALIGNMENT - 1) / ALIGNMENT) * ALIGNMENT
}

fn calculate_partition_layout(
    disk_info: &DiskInfo,
    root_size: u64,
    swap_size: Option<u64>,
    var_size: Option<u64>,
) -> Result<PartitionLayout> {
    let swap_size = swap_size.unwrap_or(0);
    let var_size = var_size.unwrap_or(0);

    // Convert to sectors
    let root_size_sectors = root_size / SECTOR_SIZE;
    let swap_size_sectors = swap_size / SECTOR_SIZE;
    let var_size_sectors = var_size / SECTOR_SIZE;

    // Get current root partition start sector
    let root_start = get_partition_start(&disk_info.device, 2)?;

    // Calculate partition boundaries (aligned)
    let root_end = align_sector(root_start + root_size_sectors) - 1;

    let swap_start = if swap_size > 0 {
        align_sector(root_end + 1)
    } else {
        0
    };
    let swap_end = if swap_size > 0 {
        align_sector(swap_start + swap_size_sectors) - 1
    } else {
        0
    };

    let var_start = if var_size > 0 {
        if swap_size > 0 {
            align_sector(swap_end + 1)
        } else {
            align_sector(root_end + 1)
        }
    } else {
        0
    };
    let var_end = if var_size > 0 {
        align_sector(var_start + var_size_sectors) - 1
    } else {
        0
    };

    let home_start = if var_size > 0 {
        align_sector(var_end + 1)
    } else if swap_size > 0 {
        align_sector(swap_end + 1)
    } else {
        align_sector(root_end + 1)
    };

    // Home partition gets the rest
    let home_end = disk_info.size_sectors - 1;

    let home_size_bytes = (home_end - home_start + 1) * SECTOR_SIZE;

    // Validate that /home is at least half the disk
    let min_home_size = disk_info.size_bytes / 2;
    if home_size_bytes < min_home_size {
        bail!(
            "Insufficient space for /home partition. Need at least {} GB, but only {} GB available after other partitions",
            min_home_size / (1024 * 1024 * 1024),
            home_size_bytes / (1024 * 1024 * 1024)
        );
    }

    Ok(PartitionLayout {
        root_size_bytes: root_size,
        swap_size_bytes: swap_size,
        var_size_bytes: var_size,
        home_size_bytes,
        root_start,
        root_end,
        swap_start,
        swap_end,
        var_start,
        var_end,
        home_start,
        home_end,
    })
}

fn get_partition_start(device: &str, partition_num: u32) -> Result<u64> {
    let output = Command::new("parted")
        .args(&[device, "unit", "s", "print"])
        .output()
        .context("Failed to run parted")?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse partition table
    let re = Regex::new(&format!(r"^\s*{}\s+(\d+)s", partition_num))?;
    for line in stdout.lines() {
        if let Some(caps) = re.captures(line) {
            return caps[1].parse::<u64>().context("Failed to parse start sector");
        }
    }

    bail!("Could not find partition {} start sector", partition_num)
}

fn print_layout(layout: &PartitionLayout) {
    println!("Partition Layout:");
    println!("  Root (/):");
    println!("    Size: {} GB", layout.root_size_bytes / (1024 * 1024 * 1024));
    println!("    Sectors: {} - {}", layout.root_start, layout.root_end);

    if layout.swap_size_bytes > 0 {
        println!("  Swap:");
        println!("    Size: {} GB", layout.swap_size_bytes / (1024 * 1024 * 1024));
        println!("    Sectors: {} - {}", layout.swap_start, layout.swap_end);
    }

    if layout.var_size_bytes > 0 {
        println!("  /var (btrfs):");
        println!("    Size: {} GB", layout.var_size_bytes / (1024 * 1024 * 1024));
        println!("    Sectors: {} - {}", layout.var_start, layout.var_end);
    }

    println!("  /home (ext4):");
    println!("    Size: {} GB", layout.home_size_bytes / (1024 * 1024 * 1024));
    println!("    Sectors: {} - {}", layout.home_start, layout.home_end);
}

fn check_filesystem(partition: &str) -> Result<()> {
    println!("  Checking filesystem on {}...", partition);

    let status = Command::new("e2fsck")
        .args(&["-f", "-y", partition])
        .status()
        .context("Failed to run e2fsck")?;

    if !status.success() {
        println!("  Warning: e2fsck returned non-zero status, continuing anyway...");
    }

    Ok(())
}

fn shrink_root_filesystem(partition: &str, new_size: u64) -> Result<()> {
    // Convert to 4K blocks (resize2fs uses 4K blocks)
    let blocks = new_size / 4096;

    println!("  Shrinking filesystem to {} 4K blocks...", blocks);

    let status = Command::new("resize2fs")
        .args(&[partition, &format!("{}K", blocks * 4)])
        .status()
        .context("Failed to run resize2fs")?;

    if !status.success() {
        bail!("resize2fs failed");
    }

    println!("  Filesystem shrunk successfully");
    Ok(())
}

fn resize_root_partition(disk_info: &DiskInfo, new_end_sector: u64) -> Result<()> {
    println!("  Resizing partition 2 to end at sector {}...", new_end_sector);

    // Get current partition info
    let start = get_partition_start(&disk_info.device, 2)?;

    // Use parted to resize the partition
    let commands = format!("rm 2\nmkpart primary ext4 {}s {}s\nquit\n", start, new_end_sector);

    let mut child = Command::new("parted")
        .args(&[&disk_info.device])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn parted")?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin.write_all(commands.as_bytes())?;
    }

    let output = child.wait_with_output()?;

    if !output.status.success() {
        bail!("Failed to resize partition: {}", String::from_utf8_lossy(&output.stderr));
    }

    // Inform kernel of partition changes
    let _ = Command::new("partprobe")
        .arg(&disk_info.device)
        .status();

    println!("  Partition resized successfully");
    Ok(())
}

fn get_next_partition_number(device: &str) -> Result<u32> {
    let output = Command::new("parted")
        .args(&[device, "print"])
        .output()
        .context("Failed to run parted")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let re = Regex::new(r"^\s*(\d+)\s+")?;

    let mut max_num = 0;
    for line in stdout.lines() {
        if let Some(caps) = re.captures(line) {
            if let Ok(num) = caps[1].parse::<u32>() {
                max_num = max_num.max(num);
            }
        }
    }

    Ok(max_num + 1)
}

fn create_swap_partition(disk_info: &DiskInfo, start: u64, end: u64) -> Result<()> {
    let part_num = get_next_partition_number(&disk_info.device)?;

    println!("  Creating swap partition {} from sector {} to {}...", part_num, start, end);

    let status = Command::new("parted")
        .args(&[
            &disk_info.device,
            "mkpart",
            "primary",
            "linux-swap",
            &format!("{}s", start),
            &format!("{}s", end),
        ])
        .status()
        .context("Failed to create swap partition")?;

    if !status.success() {
        bail!("Failed to create swap partition");
    }

    // Inform kernel
    let _ = Command::new("partprobe").arg(&disk_info.device).status();

    // Format as swap
    let swap_device = get_partition_device(&disk_info.device, part_num)?;
    println!("  Formatting {} as swap...", swap_device);

    let status = Command::new("mkswap")
        .arg(&swap_device)
        .status()
        .context("Failed to run mkswap")?;

    if !status.success() {
        bail!("mkswap failed");
    }

    println!("  Swap partition created: {}", swap_device);
    Ok(())
}

fn create_var_partition(disk_info: &DiskInfo, start: u64, end: u64) -> Result<()> {
    let part_num = get_next_partition_number(&disk_info.device)?;

    println!("  Creating /var partition {} from sector {} to {}...", part_num, start, end);

    let status = Command::new("parted")
        .args(&[
            &disk_info.device,
            "mkpart",
            "primary",
            "btrfs",
            &format!("{}s", start),
            &format!("{}s", end),
        ])
        .status()
        .context("Failed to create /var partition")?;

    if !status.success() {
        bail!("Failed to create /var partition");
    }

    // Inform kernel
    let _ = Command::new("partprobe").arg(&disk_info.device).status();

    // Format as btrfs
    let var_device = get_partition_device(&disk_info.device, part_num)?;
    println!("  Formatting {} as btrfs...", var_device);

    let status = Command::new("mkfs.btrfs")
        .args(&["-f", &var_device])
        .status()
        .context("Failed to run mkfs.btrfs")?;

    if !status.success() {
        bail!("mkfs.btrfs failed");
    }

    println!("  /var partition created: {}", var_device);
    Ok(())
}

fn create_home_partition(disk_info: &DiskInfo, start: u64, end: u64) -> Result<()> {
    let part_num = get_next_partition_number(&disk_info.device)?;

    println!("  Creating /home partition {} from sector {} to {}...", part_num, start, end);

    let status = Command::new("parted")
        .args(&[
            &disk_info.device,
            "mkpart",
            "primary",
            "ext4",
            &format!("{}s", start),
            &format!("{}s", end),
        ])
        .status()
        .context("Failed to create /home partition")?;

    if !status.success() {
        bail!("Failed to create /home partition");
    }

    // Inform kernel
    let _ = Command::new("partprobe").arg(&disk_info.device).status();

    // Format as ext4
    let home_device = get_partition_device(&disk_info.device, part_num)?;
    println!("  Formatting {} as ext4...", home_device);

    let status = Command::new("mkfs.ext4")
        .args(&["-F", &home_device])
        .status()
        .context("Failed to run mkfs.ext4")?;

    if !status.success() {
        bail!("mkfs.ext4 failed");
    }

    println!("  /home partition created: {}", home_device);
    Ok(())
}

fn get_partition_device(device: &str, partition_num: u32) -> Result<String> {
    let partition_device = if device.contains("mmcblk") || device.contains("nvme") {
        format!("{}p{}", device, partition_num)
    } else {
        format!("{}{}", device, partition_num)
    };

    // Wait a bit for the device to appear
    std::thread::sleep(std::time::Duration::from_secs(2));

    if !Path::new(&partition_device).exists() {
        bail!("Partition device {} does not exist after creation", partition_device);
    }

    Ok(partition_device)
}
