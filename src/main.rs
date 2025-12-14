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

    /// Skip inactive disk check (dangerous - allows running on active root disk)
    #[arg(long)]
    allow_active_disk: bool,
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

#[derive(Debug, Clone)]
struct CreatedPartitions {
    root_device: String,
    swap_device: Option<String>,
    var_device: Option<String>,
    home_device: String,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Check if running as root
    if !is_root() {
        bail!("This program must be run as root");
    }

    println!("RPi Filesystem Shrink Tool");
    println!("==========================\n");

    // Display command line arguments
    println!("Command Line Arguments:");
    println!("  Device: {}", args.device);
    println!("  Root size: {}", args.root_size);
    if let Some(ref swap) = args.swap_size {
        println!("  Swap size: {}", swap);
    } else {
        println!("  Swap size: None");
    }
    if let Some(ref var) = args.var_size {
        println!("  Var size: {}", var);
    } else {
        println!("  Var size: None");
    }
    println!("  Dry run: {}", args.dry_run);
    println!("  Allow active disk: {}", args.allow_active_disk);
    println!("\nPress Enter to continue...");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;

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

    // Check if disk is the active root disk
    if !args.allow_active_disk && is_active_root_disk(&disk_info.device)? {
        bail!(
            "ERROR: {} appears to be the active root disk!\n\
            This program must be run on an INACTIVE disk (e.g., from a LiveUSB).\n\
            Use --allow-active-disk to override this check (NOT RECOMMENDED).",
            disk_info.device
        );
    }

    // Check SD card constraints - block swap and var on SD cards
    if disk_info.is_sd_card {
        if swap_size.is_some() {
            bail!("ERROR: Swap partition is not allowed on SD cards.\nSD cards have limited write cycles and swap would cause excessive wear.");
        }
        if var_size.is_some() {
            bail!("ERROR: Separate /var partition is not allowed on SD cards.\nSD cards have limited write cycles and separate /var would cause excessive wear.");
        }
    }

    // Calculate partition layout
    let layout = calculate_partition_layout(
        &disk_info,
        root_size,
        swap_size,
        var_size,
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
    let swap_device = if layout.swap_size_bytes > 0 {
        println!("\nStep 4: Creating swap partition...");
        Some(create_swap_partition(&disk_info, layout.swap_start, layout.swap_end)?)
    } else {
        None
    };

    // Step 5: Create /var partition (if requested)
    let var_device = if layout.var_size_bytes > 0 {
        println!("\nStep 5: Creating /var partition...");
        Some(create_var_partition(&disk_info, layout.var_start, layout.var_end)?)
    } else {
        None
    };

    // Step 6: Create /home partition
    println!("\nStep 6: Creating /home partition...");
    let home_device = create_home_partition(&disk_info, layout.home_start, layout.home_end)?;

    let created_partitions = CreatedPartitions {
        root_device: disk_info.root_partition.clone(),
        swap_device,
        var_device: var_device.clone(),
        home_device: home_device.clone(),
    };

    println!("\n=== Partitions created successfully! ===");

    // Step 7: Migrate data and update fstab (always enabled)
    println!("\n=== Starting data migration ===\n");

    println!("Step 7: Creating mount points...");
    create_mount_points()?;

    println!("\nStep 8: Mounting partitions...");
    mount_partitions(&created_partitions)?;

    if var_device.is_some() {
        println!("\nStep 9: Migrating /var data...");
        migrate_var_data()?;
    }

    println!("\nStep 10: Migrating /home data...");
    migrate_home_data()?;

    println!("\nStep 11: Updating /etc/fstab...");
    update_fstab(&created_partitions)?;

    println!("\nStep 12: Unmounting partitions...");
    unmount_all()?;

    println!("\n=== Migration complete! ===");
    println!("\nAll data has been migrated and fstab updated.");
    println!("You can now boot from this disk.");

    Ok(())
}

fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

fn is_active_root_disk(device: &str) -> Result<bool> {
    // Read /proc/mounts to find the root filesystem
    let mounts = std::fs::read_to_string("/proc/mounts")
        .context("Failed to read /proc/mounts")?;

    for line in mounts.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 && parts[1] == "/" {
            let root_device = parts[0];
            // Check if this device or any partition on it is the root
            if root_device.starts_with(device) || device.starts_with(root_device) {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

fn check_dependencies(dry_run: bool) -> Result<()> {
    println!("Checking dependencies...");

    let dependencies = vec![
        ("parted", "parted"),
        ("resize2fs", "e2fsprogs"),
        ("mkfs.ext4", "e2fsprogs"),
        ("mkfs.btrfs", "btrfs-progs"),
        ("mkswap", "util-linux"),
        ("rsync", "rsync"),
        ("mount", "mount"),
        ("umount", "mount"),
        ("blkid", "util-linux"),
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
                .args(["install", "-y", package])
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
                .args(["install", "-y", package])
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
        .args([&device, "unit", "B", "print"])
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
        format!("{}2", device)
    } else {
        format!("{}p2", device)
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
    sector.div_ceil(ALIGNMENT) * ALIGNMENT
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
        .args([device, "unit", "s", "print"])
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
        .args(["-f", "-y", partition])
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
        .args([partition, &format!("{}K", blocks * 4)])
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
        .args([&disk_info.device])
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
        .args([device, "print"])
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

fn create_swap_partition(disk_info: &DiskInfo, start: u64, end: u64) -> Result<String> {
    let part_num = get_next_partition_number(&disk_info.device)?;

    println!("  Creating swap partition {} from sector {} to {}...", part_num, start, end);

    let status = Command::new("parted")
        .args([
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
    Ok(swap_device)
}

fn create_var_partition(disk_info: &DiskInfo, start: u64, end: u64) -> Result<String> {
    let part_num = get_next_partition_number(&disk_info.device)?;

    println!("  Creating /var partition {} from sector {} to {}...", part_num, start, end);

    let status = Command::new("parted")
        .args([
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
        .args(["-f", &var_device])
        .status()
        .context("Failed to run mkfs.btrfs")?;

    if !status.success() {
        bail!("mkfs.btrfs failed");
    }

    println!("  /var partition created: {}", var_device);
    Ok(var_device)
}

fn create_home_partition(disk_info: &DiskInfo, start: u64, end: u64) -> Result<String> {
    let part_num = get_next_partition_number(&disk_info.device)?;

    println!("  Creating /home partition {} from sector {} to {}...", part_num, start, end);

    let status = Command::new("parted")
        .args([
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
        .args(["-F", &home_device])
        .status()
        .context("Failed to run mkfs.ext4")?;

    if !status.success() {
        bail!("mkfs.ext4 failed");
    }

    println!("  /home partition created: {}", home_device);
    Ok(home_device)
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

fn create_mount_points() -> Result<()> {
    let mount_points = vec!["/mnt/root", "/mnt/var", "/mnt/home"];

    for mount_point in mount_points {
        if !Path::new(mount_point).exists() {
            std::fs::create_dir_all(mount_point)
                .context(format!("Failed to create {}", mount_point))?;
            println!("  Created {}", mount_point);
        } else {
            println!("  {} already exists", mount_point);
        }
    }

    Ok(())
}

fn mount_partitions(partitions: &CreatedPartitions) -> Result<()> {
    // Mount root partition
    println!("  Mounting {} at /mnt/root...", partitions.root_device);
    let status = Command::new("mount")
        .args([&partitions.root_device, "/mnt/root"])
        .status()
        .context("Failed to mount root partition")?;

    if !status.success() {
        bail!("Failed to mount root partition");
    }

    // Mount /var partition if it exists
    if let Some(ref var_device) = partitions.var_device {
        println!("  Mounting {} at /mnt/var...", var_device);
        let status = Command::new("mount")
            .args([var_device.as_str(), "/mnt/var"])
            .status()
            .context("Failed to mount /var partition")?;

        if !status.success() {
            bail!("Failed to mount /var partition");
        }
    }

    // Mount /home partition
    println!("  Mounting {} at /mnt/home...", partitions.home_device);
    let status = Command::new("mount")
        .args([&partitions.home_device, "/mnt/home"])
        .status()
        .context("Failed to mount /home partition")?;

    if !status.success() {
        bail!("Failed to mount /home partition");
    }

    println!("  All partitions mounted successfully");
    Ok(())
}

fn migrate_var_data() -> Result<()> {
    println!("  Copying /mnt/root/var/* to /mnt/var/...");

    // Check if /mnt/root/var exists and has content
    if !Path::new("/mnt/root/var").exists() {
        println!("  /mnt/root/var does not exist, skipping migration");
        return Ok(());
    }

    // Use rsync to copy with progress
    let status = Command::new("rsync")
        .args([
            "-avx",
            "--progress",
            "/mnt/root/var/",
            "/mnt/var/",
        ])
        .status()
        .context("Failed to run rsync for /var")?;

    if !status.success() {
        bail!("rsync failed for /var");
    }

    println!("  Deleting /mnt/root/var/*...");
    let status = Command::new("rm")
        .args(["-rf", "/mnt/root/var/*"])
        .status()
        .context("Failed to delete /mnt/root/var/*")?;

    if !status.success() {
        bail!("Failed to delete /mnt/root/var/*");
    }

    println!("  /var migration complete");
    Ok(())
}

fn migrate_home_data() -> Result<()> {
    println!("  Copying /mnt/root/home/* to /mnt/home/...");

    // Check if /mnt/root/home exists and has content
    if !Path::new("/mnt/root/home").exists() {
        println!("  /mnt/root/home does not exist, skipping migration");
        return Ok(());
    }

    // Use rsync to copy with progress
    let status = Command::new("rsync")
        .args([
            "-avx",
            "--progress",
            "/mnt/root/home/",
            "/mnt/home/",
        ])
        .status()
        .context("Failed to run rsync for /home")?;

    if !status.success() {
        bail!("rsync failed for /home");
    }

    println!("  Deleting /mnt/root/home/*...");
    let status = Command::new("rm")
        .args(["-rf", "/mnt/root/home/*"])
        .status()
        .context("Failed to delete /mnt/root/home/*")?;

    if !status.success() {
        bail!("Failed to delete /mnt/root/home/*");
    }

    println!("  /home migration complete");
    Ok(())
}

fn get_uuid(device: &str) -> Result<String> {
    let output = Command::new("blkid")
        .args(["-s", "UUID", "-o", "value", device])
        .output()
        .context(format!("Failed to get UUID for {}", device))?;

    if !output.status.success() {
        bail!("Failed to get UUID for {}", device);
    }

    let uuid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if uuid.is_empty() {
        bail!("UUID is empty for {}", device);
    }

    Ok(uuid)
}

fn update_fstab(partitions: &CreatedPartitions) -> Result<()> {
    let fstab_path = "/mnt/root/etc/fstab";

    // Read existing fstab
    let mut fstab_content = std::fs::read_to_string(fstab_path)
        .context("Failed to read /mnt/root/etc/fstab")?;

    println!("  Getting UUIDs for new partitions...");

    // Get UUIDs for new partitions
    let mut new_entries = Vec::new();

    if let Some(ref swap_device) = partitions.swap_device {
        let uuid = get_uuid(swap_device)?;
        println!("    Swap: UUID={}", uuid);
        new_entries.push(format!("UUID={}  none  swap  sw  0  0", uuid));
    }

    if let Some(ref var_device) = partitions.var_device {
        let uuid = get_uuid(var_device)?;
        println!("    /var: UUID={}", uuid);
        new_entries.push(format!("UUID={}  /var  btrfs  defaults  0  2", uuid));
    }

    let home_uuid = get_uuid(&partitions.home_device)?;
    println!("    /home: UUID={}", home_uuid);
    new_entries.push(format!("UUID={}  /home  ext4  defaults  0  2", home_uuid));

    // Add new entries to fstab
    fstab_content.push_str("\n# Added by rpi-fs-shrink\n");
    for entry in new_entries {
        fstab_content.push_str(&format!("{}\n", entry));
    }

    // Write updated fstab
    std::fs::write(fstab_path, fstab_content)
        .context("Failed to write /mnt/root/etc/fstab")?;

    println!("  /etc/fstab updated successfully");
    Ok(())
}

fn unmount_all() -> Result<()> {
    let mount_points = vec!["/mnt/var", "/mnt/home", "/mnt/root"];

    for mount_point in mount_points {
        if Path::new(mount_point).exists() {
            println!("  Unmounting {}...", mount_point);
            let status = Command::new("umount")
                .arg(mount_point)
                .status();

            match status {
                Ok(s) if s.success() => {
                    println!("    {} unmounted", mount_point);
                }
                Ok(_) => {
                    println!("    Warning: Failed to unmount {} (may not be mounted)", mount_point);
                }
                Err(e) => {
                    println!("    Warning: Error unmounting {}: {}", mount_point, e);
                }
            }
        }
    }

    Ok(())
}
