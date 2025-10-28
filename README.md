# crpart
Repartition the Computado Rita rootfs partition to make swap, /var, /home partitions

The manual steps are
install btrfs-progs
use fdisk to get the partition sizes in blocks, usally 512 bytes
use resie2fs to shrink the ext4 file system, uses 4K blocks to about, min size about 8G, PART2
use fdisk to shrink the rootfs partition to the new ext4 file system size.
  delete the rootfs, add new partition using the same starting sector and ending at end of resize2fs
  keep the superblock
make the swap partition (optional)  start after the last block of the previous partition 12G for 4G, 24G for 8G  PART3 
make an extended partition for rest of disk, start after the last block of the previous partition. PART4
make /var partition start at the first block of the estended partition, on small SD card this is optional, about 1/4 disk size PART5
make /home partition start after the last block of the previous partition, the rest of the disk PART6
write changes
mkswap PART3
mkfs.btrfs PART5
mkfs.ext4 PART6

