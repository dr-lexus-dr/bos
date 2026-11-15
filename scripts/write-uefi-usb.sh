#!/bin/bash
set -euo pipefail
trap 'rc=$?; echo "ERROR: line $LINENO: $BASH_COMMAND (exit $rc)" >&2; exit "$rc"' ERR
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
D=/dev/sdc
check_field() {
  local field="$1" expected="$2" actual
  actual="$(lsblk -dnro "$field" "$D" | xargs)"
  [[ "$field" != SIZE ]] || actual="$(lsblk -dnbo SIZE "$D" | xargs)"
  if [[ "$field" == MODEL ]]; then actual="${actual//x20/ }"; actual="${actual//\\/}"; fi
  if [[ "$actual" != "$expected" ]]; then
    echo "ERROR: $D $field='$actual'; expected '$expected'" >&2
    exit 11
  fi
}
check_field TRAN usb
check_field RM 1
check_field SERIAL 0701934063B2FA77
check_field MODEL "USB DISK 2.0"
check_field SIZE 7751073792
# Inhibit desktop automount for this USB serial only, including new partitions.
[[ "$EUID" == 0 ]] || { echo "Run with sudo" >&2; exit 1; }
for file in EFI/BOOT/BOOTX64.EFI EFI/bOS/kernel.bin EFI/bOS/initrd.img; do
  [[ -s "$ROOT/target/esp/$file" ]] || { echo "Missing $file" >&2; exit 1; }
done
RULE="$(mktemp /run/udev/rules.d/99-bos-usb-XXXXXX.rules)"
MNT=""
cleanup() {
  if [[ -n "$MNT" ]] && mountpoint -q "$MNT"; then umount "$MNT" || true; fi
  rm -f "$RULE"
  udevadm control --reload-rules || true
  udevadm trigger --action=change --subsystem-match=block --sysname-match='sdc*' || true
  udevadm settle || true
}
trap cleanup EXIT
printf '%s\n' 'SUBSYSTEM=="block", ENV{ID_SERIAL_SHORT}=="0701934063B2FA77", ENV{UDISKS_IGNORE}="1"' > "$RULE"
udevadm control --reload-rules
udevadm trigger --action=change --subsystem-match=block --sysname-match='sdc*'
udevadm settle
unmount_usb() {
  for source in "${D}1" "$D"; do
    if findmnt -rn -S "$source" >/dev/null; then
      umount "$source" || { echo "ERROR: $source busy; close file manager windows using USB" >&2; exit 12; }
    fi
  done
}
unmount_usb
# Clear the stale ISO9660 signature, then make a simple MBR EFI partition.
/usr/sbin/wipefs --all "$D" || { echo "ERROR: could not clear signatures on $D; inspect lsblk/findmnt" >&2; exit 14; }
/usr/sbin/parted --script "$D" mklabel msdos mkpart primary fat32 1MiB 100% set 1 boot on
/usr/sbin/sfdisk --part-type "$D" 1 ef
/usr/sbin/partprobe "$D"
/usr/bin/udevadm settle
for i in {1..30}; do [[ -b /dev/sdc1 ]] && break; sleep 1; done
[[ -b /dev/sdc1 ]] || { echo "ERROR: /dev/sdc1 was not created" >&2; exit 13; }
unmount_usb
/usr/sbin/mkfs.vfat -F 32 -n BOS_EFI /dev/sdc1
MNT="$(mktemp -d /mnt/bos-usb-XXXXXX)"
mount /dev/sdc1 "$MNT"
mkdir -p "$MNT/EFI/BOOT" "$MNT/EFI/bOS"
cp "$ROOT/target/esp"/EFI/BOOT/BOOTX64.EFI "$MNT/EFI/BOOT/BOOTX64.EFI"
cp "$ROOT/target/esp"/EFI/bOS/kernel.bin "$MNT/EFI/bOS/kernel.bin"
cp "$ROOT/target/esp"/EFI/bOS/initrd.img "$MNT/EFI/bOS/initrd.img"
sync
cmp "$ROOT/target/esp"/EFI/BOOT/BOOTX64.EFI "$MNT/EFI/BOOT/BOOTX64.EFI"
cmp "$ROOT/target/esp"/EFI/bOS/kernel.bin "$MNT/EFI/bOS/kernel.bin"
cmp "$ROOT/target/esp"/EFI/bOS/initrd.img "$MNT/EFI/bOS/initrd.img"
umount "$MNT"
rmdir "$MNT"
MNT=""
printf 'UEFI USB (MBR/FAT32, EFI type 0xEF) prepared and verified on %s\n' "$D"
