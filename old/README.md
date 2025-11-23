# crpart
Repartition the Computado Rita rootfs partition to make swap, /var, /home partitions

The manual steps are<br>
install btrfs-progs<br>
use fdisk to get the partition sizes in blocks, usally 512 bytes<br>
use resie2fs to shrink the ext4 file system, uses 4K blocks to about, min size about 8G, PART2<br>
use fdisk to shrink the rootfs partition to the new ext4 file system size.<br>
  delete the rootfs, add new partition using the same starting sector and ending at end of resize2fs<br>
  keep the superblock<br>
make the swap partition (optional)  start after the last block of the previous partition 12G for 4G, 24G for 8G  PART3 <br>
make an extended partition for rest of disk, start after the last block of the previous partition. PART4<br>
make /var partition start at the first block of the estended partition, on small SD card this is optional, about 1/4 disk size PART5<br>
make /home partition start after the last block of the previous partition, the rest of the disk PART6<br>
write changes<br>
mkswap PART3<br>
mkfs.btrfs PART5<br>
mkfs.ext4 PART6<br>

