# RPi Filesystem Shrink Tool

A Rust program to shrink the Raspberry Pi root filesystem and create additional partitions for swap, /var, and /home on a larger drive.

**IMPORTANT:** This tool must be run on an **INACTIVE** disk (e.g., from a LiveUSB or another system), not on the currently running system.

## Features

- Shrinks root filesystem to a specified size (8G-64G)
- Creates optional swap partition (specify with `-s SIZE`) - **NOT allowed on SD cards**
- Creates optional btrfs /var partition (specify with `-v SIZE`) - **NOT allowed on SD cards**
- **Always creates ext4 /home partition** with remaining space
- **Always migrates data** from /var and /home to new partitions
- **Always updates /etc/fstab** with new partition UUIDs
- Displays all CLI arguments and pauses for confirmation
- Ensures proper 2048-sector alignment for all partitions
- Automatically checks and installs required dependencies
- Detects and prevents running on active root disk
- Blocks swap/var on SD cards (wear protection)
- Dry-run mode to preview changes

## Requirements

The program automatically checks for and installs (if missing):
- `parted` - Partition manipulation
- `resize2fs` - ext4 filesystem resizing (from e2fsprogs)
- `mkfs.ext4` - ext4 filesystem creation (from e2fsprogs)
- `mkfs.btrfs` - btrfs filesystem creation (from btrfs-progs)
- `mkswap` - Swap partition creation (from util-linux)
- `rsync` - Data migration
- `mount` / `umount` - Mounting partitions
- `blkid` - UUID detection (from util-linux)

## Building

```bash
cargo build --release
```

The binary will be located at `target/release/rpi-fs-shrink`.

## Usage

**WARNING: This program modifies disk partitions. Always backup your data first!**

**CRITICAL: This tool must be run on an INACTIVE disk!**
- Boot from a LiveUSB or another system
- The target disk should NOT be the currently running root filesystem
- Use `--allow-active-disk` to override this check (NOT RECOMMENDED)

```bash
# Must run as root from LiveUSB or another system
sudo ./target/release/rpi-fs-shrink -d DEVICE -r ROOT_SIZE [OPTIONS]
```

### Required Arguments

- `-d, --device DEVICE` - Target device (e.g., `/dev/mmcblk0`, `/dev/sda`)
- `-r, --root-size SIZE` - Root filesystem size (e.g., `8G`, `16G`, `32G`)
  - Minimum: 8G
  - Maximum: 64G
  - On SD cards, max is limited (e.g., 8G max on 16G SD card)

### Optional Arguments

- `-s, --swap-size SIZE` - Swap partition size (e.g., `4G`, `8G`)
  - Optional - only created if specified
  - Recommended: 1-2x RAM size
  - **BLOCKED on SD cards** (excessive wear concern)

- `-v, --var-size SIZE` - /var partition size (e.g., `4G`, `8G`)
  - Optional - only created if specified
  - Uses btrfs filesystem
  - **BLOCKED on SD cards** (excessive wear concern)

- `--dry-run` - Show what would be done without making changes
- `--allow-active-disk` - Override inactive disk check (DANGEROUS - NOT RECOMMENDED)

### Size Format

Sizes can be specified with units:
- `8G` or `8GB` - 8 Gigabytes
- `512M` or `512MB` - 512 Megabytes
- `4096K` or `4096KB` - 4096 Kilobytes

## Examples

### Example 1: 16GB SD Card (from LiveUSB)

Shrink root to 8G, create /home with remaining space.

**Note:** SD cards do NOT support swap or /var partitions due to wear concerns.

```bash
# Boot from LiveUSB, then run:
sudo ./target/release/rpi-fs-shrink -d /dev/mmcblk0 -r 8G
```

The tool will display:
```
Command Line Arguments:
  Device: /dev/mmcblk0
  Root size: 8G
  Swap size: None
  Var size: None
  Dry run: false
  Allow active disk: false

Press Enter to continue...
```

Result:
- `/dev/mmcblk0p1` - Boot (unchanged)
- `/dev/mmcblk0p2` - Root (/) - 8GB ext4
- `/dev/mmcblk0p3` - /home - ~8GB ext4

The tool will automatically:
1. Shrink the root filesystem
2. Create /home partition
3. Mount partitions at /mnt/root, /mnt/home
4. Migrate /home data to new partition
5. Update /etc/fstab with UUIDs
6. Unmount all partitions
7. Disk is ready to boot!

If you try to use `-s` or `-v` on an SD card, the tool will error and stop.

### Example 2: 128GB SSD with Swap and /var (from LiveUSB)

Shrink root to 16G, add 8G swap, 16G /var, rest for /home:

```bash
# Boot from LiveUSB with the target SSD connected
sudo ./target/release/rpi-fs-shrink -d /dev/sda -r 16G -s 8G -v 16G
```

The tool will display:
```
Command Line Arguments:
  Device: /dev/sda
  Root size: 16G
  Swap size: 8G
  Var size: 16G
  Dry run: false
  Allow active disk: false

Press Enter to continue...
```

Result:
- `/dev/sda1` - Boot (unchanged)
- `/dev/sda2` - Root (/) - 16GB ext4
- `/dev/sda3` - Swap - 8GB
- `/dev/sda4` - /var - 16GB btrfs
- `/dev/sda5` - /home - ~88GB ext4

The tool will automatically:
1. Shrink root filesystem
2. Create all partitions
3. Mount partitions
4. Migrate /var data to new btrfs partition
5. Migrate /home data to new ext4 partition
6. Update /etc/fstab with UUIDs
7. Unmount all partitions
8. Disk is ready to boot!

### Example 3: 128GB SSD with Swap only (no /var)

Shrink root to 16G, add 8G swap, rest for /home:

```bash
sudo ./target/release/rpi-fs-shrink -d /dev/sda -r 16G -s 8G
```

Result:
- `/dev/sda1` - Boot (unchanged)
- `/dev/sda2` - Root (/) - 16GB ext4
- `/dev/sda3` - Swap - 8GB
- `/dev/sda4` - /home - ~104GB ext4

### Example 4: Dry Run

Preview changes without modifying disk:

```bash
sudo ./target/release/rpi-fs-shrink -d /dev/mmcblk0 -r 8G --dry-run
```

## How It Works

1. **Display Arguments & Pause** - Shows all CLI arguments and waits for Enter key
2. **Dependency Check** - Verifies required tools are installed
3. **Inactive Disk Check** - Ensures target is not the active root disk
4. **Device Analysis** - Detects SD card, gets disk size and partition info
5. **Layout Calculation** - Calculates partition boundaries with 2048-sector alignment
6. **Filesystem Check** - Runs e2fsck on root filesystem
7. **Filesystem Shrink** - Shrinks ext4 filesystem using resize2fs
8. **Partition Resize** - Resizes root partition using parted
9. **Partition Creation** - Creates new partitions:
   - Swap partition (if `-s` specified)
   - /var partition with btrfs (if `-v` specified)
   - /home partition with ext4 (remaining space)
10. **Data Migration** (always performed):
    - Creates mount points: /mnt/root, /mnt/var (if needed), /mnt/home
    - Mounts all partitions
    - Migrates /var data (if /var partition created)
    - Migrates /home data
    - Updates /etc/fstab with UUIDs
    - Unmounts all partitions

## Partition Alignment

All partitions are aligned on 2048-sector boundaries (1MB) for optimal performance with modern storage devices.

### Sector-Based Partition Mathematics

The tool uses **512-byte sectors** for all calculations with **2048-sector alignment** (1MB boundaries).

#### Constants
- **Sector Size**: 512 bytes
- **Alignment Boundary**: 2048 sectors (1MB)
- **Alignment Bytes**: 2048 × 512 = 1,048,576 bytes (1MB)

#### Partition Sizing Algorithm

**1. Root Partition Shrink:**
```
User specifies: -r 16G

Step 1: Convert to bytes
  root_size_bytes = 16 × 1024³ = 17,179,869,184 bytes

Step 2: Convert to sectors
  root_size_sectors = 17,179,869,184 ÷ 512 = 33,554,432 sectors

Step 3: Calculate aligned end boundary
  root_start = (existing partition start, e.g., 2048)
  root_end_raw = root_start + root_size_sectors
  root_end_aligned = align_down(root_end_raw) - 1

  align_down(sector) = (sector ÷ 2048) × 2048

  Example: If root_end_raw = 33,556,480
    aligned = (33,556,480 ÷ 2048) × 2048 = 33,554,432
    root_end = 33,554,432 - 1 = 33,554,431
```

**2. Swap Partition (if specified):**
```
User specifies: -s 8G

Step 1: Convert to sectors
  swap_size_sectors = (8 × 1024³) ÷ 512 = 16,777,216 sectors

Step 2: Align start on 2048 boundary
  swap_start = align_up(root_end + 1)

  align_up(sector) = ((sector + 2048 - 1) ÷ 2048) × 2048

  Example: If root_end = 33,554,431
    swap_start_raw = 33,554,432
    swap_start = ((33,554,432 + 2047) ÷ 2048) × 2048
               = (33,556,479 ÷ 2048) × 2048
               = 16,385 × 2048 = 33,556,480

Step 3: Align end on 2048 boundary
  swap_end_raw = swap_start + swap_size_sectors
  swap_end = align_up(swap_end_raw) - 1

  Example: swap_end_raw = 33,556,480 + 16,777,216 = 50,333,696
    aligned = ((50,333,696 + 2047) ÷ 2048) × 2048
            = 50,333,696 (already aligned)
    swap_end = 50,333,696 - 1 = 50,333,695
```

**3. /var Partition (if specified):**
```
User specifies: -v 16G

Step 1: Convert to sectors
  var_size_sectors = (16 × 1024³) ÷ 512 = 33,554,432 sectors

Step 2: Start after previous partition (swap or root)
  If swap exists:
    var_start = align_up(swap_end + 1)
  Else:
    var_start = align_up(root_end + 1)

  Example with swap: var_start = align_up(50,333,696)
    = ((50,333,696 + 2047) ÷ 2048) × 2048
    = 50,333,696 (already aligned)

Step 3: Align end
  var_end = align_up(var_start + var_size_sectors) - 1
```

**4. /home Partition (always created):**
```
/home gets all remaining space to maximize available storage

Step 1: Start after previous partition
  If var exists:
    home_start = align_up(var_end + 1)
  Else if swap exists:
    home_start = align_up(swap_end + 1)
  Else:
    home_start = align_up(root_end + 1)

Step 2: End at disk boundary
  home_end = disk_total_sectors - 1

  Example: 128GB disk = 250,069,680 sectors
    home_end = 250,069,679

Step 3: Calculate actual size
  home_size_sectors = home_end - home_start + 1
  home_size_bytes = home_size_sectors × 512
```

#### Alignment Benefits

**Why 2048-sector (1MB) alignment?**

1. **Modern Disks**: Advanced Format drives use 4KB physical sectors
   - 2048 × 512 = 1MB aligns with multiple 4KB sectors

2. **SSD Performance**: SSD erase blocks are typically 512KB-4MB
   - 1MB alignment ensures no partition crosses erase block boundaries

3. **RAID Optimization**: Common RAID stripe sizes are 64KB, 128KB, 256KB
   - 1MB is divisible by all common stripe sizes

4. **Filesystem Block Alignment**: Most filesystems use 4KB blocks
   - 1MB = 256 × 4KB blocks, perfectly aligned

#### Example Calculation: 128GB SSD

```
Disk: 128GB = 250,069,680 sectors (128 × 1024³ ÷ 512)
User: -r 16G -s 8G -v 16G

Root Partition (partition 2):
  Start: 2048 (existing)
  Size:  16 × 1024³ ÷ 512 = 33,554,432 sectors
  End:   align_down(2048 + 33,554,432) - 1 = 33,554,431
  Bytes: 33,552,384 × 512 = 17,178,820,608 (~16GB)

Swap Partition (partition 3):
  Start: align_up(33,554,432) = 33,556,480
  Size:  8 × 1024³ ÷ 512 = 16,777,216 sectors
  End:   align_up(33,556,480 + 16,777,216) - 1 = 50,333,695
  Bytes: 16,777,216 × 512 = 8,589,934,592 (8GB)

/var Partition (partition 4):
  Start: align_up(50,333,696) = 50,333,696
  Size:  16 × 1024³ ÷ 512 = 33,554,432 sectors
  End:   align_up(50,333,696 + 33,554,432) - 1 = 83,888,127
  Bytes: 33,554,432 × 512 = 17,179,869,184 (16GB)

/home Partition (partition 5):
  Start: align_up(83,888,128) = 83,886,080
  End:   250,069,679 (disk end)
  Size:  166,183,600 sectors
  Bytes: 166,183,600 × 512 = 85,086,003,200 (~79.2GB)

Total allocated: ~120GB
Lost to alignment: <1MB per partition (~3-4MB total)
```

#### Maximizing Partition Size

The tool **maximizes partition utilization** while maintaining alignment:

1. **Always align partition start**: Round up to next 2048 boundary
2. **Align partition end**: Round up size to next 2048 boundary
3. **/home takes all remaining space**: No rounding, uses exact disk end
4. **Minimal alignment waste**: <1MB per partition

Total waste from alignment is typically **<0.01%** of disk capacity.

## Constraints

- **Must run on inactive disk** (boot from LiveUSB or another system)
- Root filesystem must be between 8G and 64G
- **/home partition is always created** (cannot run with root only)
- /home partition must be at least half the disk size
- **On SD cards:**
  - Root size is limited by total disk size
  - **Swap partitions are BLOCKED** (SD card wear protection)
  - **/var partitions are BLOCKED** (SD card wear protection)
  - Only root + /home partitions allowed

## After Running the Tool

1. **The disk is ready to boot!** - All data has been migrated and fstab updated
2. Shut down the LiveUSB and boot from the modified disk
3. Verify partitions are mounted: `df -h`
4. Check fstab: `cat /etc/fstab`
5. Verify swap (if created): `swapon --show`

## Troubleshooting

### "appears to be the active root disk"
- **This is a safety check!** The tool must run on an inactive disk
- Boot from a LiveUSB or another system
- Use `--allow-active-disk` to override (DANGEROUS - NOT RECOMMENDED)

### "Device does not exist"
- Verify device path with `lsblk`
- Ensure you're using the full device path (e.g., `/dev/mmcblk0`, not `/dev/mmcblk0p1`)

### "Must be run as root"
- Use `sudo` to run the program

### "Insufficient space for /home partition"
- Reduce size of root, swap, or /var partitions
- The program ensures /home is at least half the disk

### Filesystem check fails
- Boot from another device or LiveUSB
- Run manual filesystem check: `sudo e2fsck -f /dev/mmcblk0p2`

### Data migration fails
- Check available space on target partitions
- Verify rsync is installed
- Check /mnt/root/var and /mnt/root/home exist and are accessible

### "Swap partition is not allowed on SD cards"
- **This is intentional!** SD cards have limited write cycles
- Swap would cause excessive wear and shorten SD card life
- Use a real SSD/HDD if you need swap

### "Separate /var partition is not allowed on SD cards"
- **This is intentional!** SD cards have limited write cycles
- /var contains frequently written logs and cache
- Use a real SSD/HDD if you need separate /var

## Safety Features

- **CLI arguments display and pause** - Shows all settings before proceeding
- **Inactive disk detection** - Prevents running on active root filesystem
- **SD card wear protection** - Blocks swap/var on SD cards
- Requires root privileges
- Always creates /home partition (root-only not allowed)
- Dry-run mode for testing
- Interactive confirmation before making changes
- Validates all size constraints
- Checks filesystem integrity before resizing
- Automatic data migration with rsync
- UUID-based fstab entries for reliable mounting

## License

This project is open source and available under the MIT License.

## Credits

Based on the manual partitioning process for Raspberry Pi systems to optimize storage usage on larger drives.

## Original Manual Process

The original manual steps (saved in `old/README.md`) involved using fdisk and manual calculations. This tool automates that process with:
- Automatic size validation
- Proper alignment calculations
- Dependency management
- Error handling
- SD card detection
