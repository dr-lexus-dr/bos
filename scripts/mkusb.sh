#!/usr/bin/env bash
#
# bOS: install to a dedicated disk (UEFI, no Secure Boot signature).
#
# What it does (ERASES the whole disk!):
#   1. GPT label + single EFI System Partition (FAT32, type EF00)
#   2. copies <esp>/ (EFI/BOOT/BOOTX64.EFI, EFI/bOS/...) onto it
#
# Usage:
#   sudo bash INSTALLER /dev/sdX [--esp DIR] [--yes]
#
#   /dev/sdX must be the WHOLE disk (not a partition like /dev/sdX1).
#   Find it with: lsblk -d -o NAME,SIZE,MODEL,TRAN
#
# Requirements: sfdisk (util-linux), dosfstools (mkfs.vfat), root.
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ESP="$SCRIPT_DIR/../target/esp"
if [[ -d "$SCRIPT_DIR/EFI" ]]; then
    ESP="$SCRIPT_DIR"
fi
YES=0
DEV=""

log()  { printf '[mkusb] %s\n' "$*"; }
die()  { printf '[mkusb] ERROR: %s\n' "$*" >&2; exit 1; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        --esp)   ESP="${2:?--esp needs a directory}"; shift 2 ;;
        --esp=*) ESP="${1#--esp=}"; shift ;;
        --yes)   YES=1; shift ;;
        -h|--help)
            awk '/^# bOS: install/{p=1} /^set -euo pipefail/{p=0} p' "$0" | sed 's/^# \?//'; exit 0 ;;
        /dev/*)  DEV="$1"; shift ;;
        *)       die "unknown argument '$1' (try --help)" ;;
    esac
done

[[ -n "$DEV" ]] || die "no device given (example: sudo bash scripts/mkusb.sh /dev/sdb --yes)"
[[ -b "$DEV" ]] || die "$DEV is not a block device"
DEV="$(realpath "$DEV")"
[[ "$(lsblk -dnro TYPE "$DEV")" == "disk" ]] || die "select a whole disk, such as /dev/sdb or /dev/nvme0n1"
for file in EFI/BOOT/BOOTX64.EFI EFI/bOS/kernel.bin EFI/bOS/initrd.img; do
    [[ -s "$ESP/$file" ]] || die "missing $ESP/$file"
done
[[ "$(id -u)" -eq 0 ]] || die "must run as root (sudo)"
command -v sfdisk >/dev/null || die "sfdisk not found (apt install util-linux)"
command -v mkfs.vfat >/dev/null || die "mkfs.vfat not found (apt install dosfstools)"

# A mounted filesystem, active swap, or mapped device can be using this disk.
mountpoints="$(lsblk -nr -o MOUNTPOINTS "$DEV")"
types="$(lsblk -nr -o TYPE "$DEV")"
if [[ -n "$mountpoints" ]] || [[ -n "$(printf '%s\n' "$types" | awk '$0 != "disk" && $0 != "part"')" ]]; then
    die "$DEV is in use — unmount filesystems and deactivate swap or mapped devices first"
fi

log "target disk:"
lsblk -d -o NAME,SIZE,MODEL,TRAN "$DEV" || true
log "ESP source: $ESP"
log "THIS ERASES ALL DATA ON $DEV"

if [[ "$YES" -ne 1 ]]; then
    read -r -p "[mkusb] type YES to continue: " ans
    [[ "$ans" == "YES" ]] || die "aborted"
fi

log "partitioning $DEV (GPT + ESP)..."
sfdisk --wipe always "$DEV" <<'EOF'
label: gpt
size=+, type=uefi
EOF
if command -v udevadm >/dev/null; then
    udevadm settle
fi

# Partition node: /dev/sdb1, or /dev/mmcblk0p1 / /dev/nvme0n1p1 style.
if [[ "$DEV" =~ [0-9]$ ]]; then PART="${DEV}p1"; else PART="${DEV}1"; fi
[[ -b "$PART" ]] || die "partition node $PART not found after partitioning"
[[ "$(lsblk -dnro PKNAME "$PART")" == "${DEV##*/}" ]] || die "$PART is not a partition of $DEV"

log "mkfs.vfat $PART ..."
mkfs.vfat -F 32 -n BOS_EFI "$PART" >/dev/null

MNT="$(mktemp -d /tmp/bos-usb.XXXXXX)"
trap 'umount "$MNT" 2>/dev/null || true; rmdir "$MNT" 2>/dev/null || true' EXIT
mount "$PART" "$MNT"
log "copying $(du -sh "$ESP" | cut -f1) ..."
mkdir -p "$MNT/EFI/BOOT" "$MNT/EFI/bOS"
cp "$ESP/EFI/BOOT/BOOTX64.EFI" "$MNT/EFI/BOOT/"
cp "$ESP/EFI/bOS/"{kernel.bin,initrd.img} "$MNT/EFI/bOS/"
sync
# Show what landed on the stick:
find "$MNT" -type f -printf '  %P (%s bytes)\n' | sort
umount "$MNT"
trap - EXIT
rmdir "$MNT"

log "done. Boot notes:"
log "  - plug into a USB2 port if USB3 boot fails on old boards"
log "  - enter firmware setup: enable UEFI boot, DISABLE Secure Boot"
log "    (bLOADER is not signed), then pick the stick in the boot menu"
log "  - serial log needs a debug cable; otherwise watch the screen"
