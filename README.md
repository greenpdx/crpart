# RPi Filesystem Shrink Tool

A Rust program to shrink the Raspberry Pi root filesystem and create additional partitions for swap, /var, and /home on a larger drive.

## Features

- Shrinks root filesystem to a specified size (8G-64G)
- Creates optional swap partition (not on SD cards by default)
- Creates optional btrfs /var partition (not on SD cards by default)
- Creates ext4 /home partition with remaining space
- Ensures proper 2048-sector alignment for all partitions
- Automatically checks and installs required dependencies
- Dry-run mode to preview changes
- SD card detection with appropriate warnings

## Requirements

The program automatically checks for and installs (if missing):
- `parted` - Partition manipulation
- `resize2fs` - ext4 filesystem resizing (from e2fsprogs)
- `mkfs.ext4` - ext4 filesystem creation (from e2fsprogs)
- `mkfs.btrfs` - btrfs filesystem creation (from btrfs-progs)
- `mkswap` - Swap partition creation (from util-linux)

## Building

```bash
cargo build --release
```

The binary will be located at `target/release/rpi-fs-shrink`.

## Usage

**WARNING: This program modifies disk partitions. Always backup your data first!**

```bash
# Must run as root
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
  - Not created on SD cards unless `-f` is used
  - Recommended: 1-2x RAM size

- `-v, --var-size SIZE` - /var partition size (e.g., `4G`, `8G`)
  - Not created on SD cards unless `-f` is used
  - Uses btrfs filesystem

- `-f, --force` - Force creation of swap/var on SD cards
- `--dry-run` - Show what would be done without making changes

### Size Format

Sizes can be specified with units:
- `8G` or `8GB` - 8 Gigabytes
- `512M` or `512MB` - 512 Megabytes
- `4096K` or `4096KB` - 4096 Kilobytes

## Examples

### Example 1: 16GB SD Card

Shrink root to 8G, create /home with remaining space:

```bash
sudo ./target/release/rpi-fs-shrink -d /dev/mmcblk0 -r 8G
```

Result:
- `/dev/mmcblk0p1` - Boot (unchanged)
- `/dev/mmcblk0p2` - Root (/) - 8GB ext4
- `/dev/mmcblk0p3` - /home - ~8GB ext4

### Example 2: 128GB SSD

Shrink root to 16G, add 8G swap, 16G /var, rest for /home:

```bash
sudo ./target/release/rpi-fs-shrink -d /dev/sda -r 16G -s 8G -v 16G
```

Result:
- `/dev/sda1` - Boot (unchanged)
- `/dev/sda2` - Root (/) - 16GB ext4
- `/dev/sda3` - Swap - 8GB
- `/dev/sda4` - /var - 16GB btrfs
- `/dev/sda5` - /home - ~88GB ext4

### Example 3: Dry Run

Preview changes without modifying disk:

```bash
sudo ./target/release/rpi-fs-shrink -d /dev/mmcblk0 -r 8G -s 4G --dry-run
```

### Example 4: Force Swap on SD Card

Create swap on SD card (not recommended but possible):

```bash
sudo ./target/release/rpi-fs-shrink -d /dev/mmcblk0 -r 8G -s 2G -f
```

## How It Works

1. **Dependency Check** - Verifies required tools are installed
2. **Device Analysis** - Detects SD card, gets disk size and partition info
3. **Layout Calculation** - Calculates partition boundaries with 2048-sector alignment
4. **Filesystem Check** - Runs e2fsck on root filesystem
5. **Filesystem Shrink** - Shrinks ext4 filesystem using resize2fs
6. **Partition Resize** - Resizes root partition using parted
7. **Partition Creation** - Creates new partitions:
   - Swap partition (if requested)
   - /var partition with btrfs (if requested)
   - /home partition with ext4 (remaining space)

## Partition Alignment

All partitions are aligned on 2048-sector boundaries (1MB) for optimal performance with modern storage devices.

## Constraints

- Root filesystem must be between 8G and 64G
- /home partition must be at least half the disk size
- On SD cards:
  - Root size is limited by total disk size
  - Swap/var partitions require `-f` flag (not recommended)

## Post-Installation Steps

After running the tool successfully:

1. **Update /etc/fstab** to mount new partitions:

   ```bash
   # Get UUIDs
   sudo blkid

   # Edit /etc/fstab
   sudo nano /etc/fstab
   ```

   Add entries like:
   ```
   UUID=xxxx-xxxx  none  swap  sw  0  0
   UUID=yyyy-yyyy  /var  btrfs defaults  0  2
   UUID=zzzz-zzzz  /home ext4  defaults  0  2
   ```

2. **Migrate /var data** (if created):
   ```bash
   sudo mkdir /mnt/newvar
   sudo mount /dev/sdaX /mnt/newvar
   sudo rsync -avx /var/ /mnt/newvar/
   ```

3. **Migrate /home data** (if needed):
   ```bash
   sudo mkdir /mnt/newhome
   sudo mount /dev/sdaY /mnt/newhome
   sudo rsync -avx /home/ /mnt/newhome/
   ```

4. **Reboot** to verify all partitions mount correctly:
   ```bash
   sudo reboot
   ```

## Troubleshooting

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

## Safety Features

- Requires root privileges
- SD card detection with warnings
- Dry-run mode for testing
- Interactive confirmation before making changes
- Validates all size constraints
- Checks filesystem integrity before resizing

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
