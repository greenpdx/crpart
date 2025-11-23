# RPi Filesystem Shrink Tool

A Rust program to shrink the Raspberry Pi root filesystem and create additional partitions for swap, /var, and /home on a larger drive.

**IMPORTANT:** This tool must be run on an **INACTIVE** disk (e.g., from a LiveUSB or another system), not on the currently running system.

## Features

- Shrinks root filesystem to a specified size (8G-64G)
- Creates optional swap partition (specify with `-s SIZE`)
- Creates optional btrfs /var partition (specify with `-v SIZE`)
- Creates ext4 /home partition with remaining space
- **Always migrates data** from /var and /home to new partitions
- **Always updates /etc/fstab** with new partition UUIDs
- Displays all CLI arguments and pauses for confirmation
- Ensures proper 2048-sector alignment for all partitions
- Automatically checks and installs required dependencies
- Detects and prevents running on active root disk
- Dry-run mode to preview changes
- SD card detection with warnings

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
  - Warning shown if used on SD cards

- `-v, --var-size SIZE` - /var partition size (e.g., `4G`, `8G`)
  - Optional - only created if specified
  - Uses btrfs filesystem
  - Warning shown if used on SD cards

- `--dry-run` - Show what would be done without making changes
- `--allow-active-disk` - Override inactive disk check (DANGEROUS - NOT RECOMMENDED)

### Size Format

Sizes can be specified with units:
- `8G` or `8GB` - 8 Gigabytes
- `512M` or `512MB` - 512 Megabytes
- `4096K` or `4096KB` - 4096 Kilobytes

## Examples

### Example 1: 16GB SD Card (from LiveUSB)

Shrink root to 8G, create /home with remaining space (no swap, no /var):

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
2. Create partitions
3. Mount them at /mnt/root, /mnt/home
4. Migrate /home data to new partition
5. Update /etc/fstab with UUIDs
6. Unmount all partitions
7. Disk is ready to boot!

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

## Constraints

- **Must run on inactive disk** (boot from LiveUSB or another system)
- Root filesystem must be between 8G and 64G
- /home partition must be at least half the disk size
- On SD cards:
  - Root size is limited by total disk size
  - Swap/var partitions will show warnings (but are allowed)

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

## Safety Features

- **CLI arguments display and pause** - Shows all settings before proceeding
- **Inactive disk detection** - Prevents running on active root filesystem
- Requires root privileges
- SD card detection with warnings (for swap/var)
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
