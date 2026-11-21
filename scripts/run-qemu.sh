#!/bin/bash
set -e
if [[ $# -eq 0 || ${1:-} == --live ]]; then
  cd "$(dirname "$0")/.."
  case "${BOS_NIC:-rtl8139}" in
    rtl8139|e1000|e1000e) live_net=(-netdev user,id=lan -device "${BOS_NIC:-rtl8139},netdev=lan,romfile=") ;;
    none) live_net=(-nic none) ;;
    *) echo "Unknown Ethernet device: $BOS_NIC" >&2; exit 2 ;;
  esac
  live_data=()
  if [[ -n "${BOS_DATA:-}" ]]; then
    [[ -f "$BOS_DATA" ]] || { echo "Data image not found: $BOS_DATA" >&2; exit 1; }
    live_data=(-drive "if=none,id=users,format=raw,file=$BOS_DATA" -device virtio-blk-pci,disable-modern=on,drive=users)
  fi
  accel=${BOS_ACCEL:-auto}
  if [[ "$accel" == auto ]]; then
    if [[ -r /dev/kvm && -w /dev/kvm ]]; then accel=kvm; else accel=tcg; fi
  fi
  if [[ "$accel" == kvm ]]; then cpu=host; else cpu=max,vendor=GenuineIntel; fi
  exec qemu-system-x86_64 -machine pc -accel "$accel" -cpu "${BOS_CPU:-$cpu}" -smp 1 -m "${BOS_MEMORY:-1G}" -device VGA,vgamem_mb=8 \
    -drive file=target/live/bos-live.img,format=raw,if=none,id=stick,readonly=on \
    -device qemu-xhci -device usb-storage,drive=stick,bootindex=1 \
    "${live_net[@]}" "${live_data[@]}" \
    -serial stdio -boot c "${@:2}"
fi
if [[ ${1:-} == --full ]]; then shift; fi
# bOS: run in QEMU (UEFI via OVMF). Usage: run-qemu.sh [esp-dir] [users.img]
ESP="${1:-target/esp}"
OVMF_CODE="${OVMF_CODE:-/usr/share/OVMF/OVMF_CODE_4M.fd}"
MEMORY="${BOS_MEMORY:-1G}"
if [[ "$MEMORY" =~ ^([0-9]+)([kMGTPE])B$ ]]; then
  MEMORY="${BASH_REMATCH[1]}${BASH_REMATCH[2]}"
fi
NIC="${BOS_NIC:-e1000}"
case "$NIC" in
  e1000|e1000e|rtl8139) NET_ARGS=(-netdev user,id=n0 -device "$NIC,netdev=n0") ;;
  none) NET_ARGS=(-nic none) ;;
  *) echo "Unsupported BOS_NIC: $NIC (expected e1000, e1000e, rtl8139, or none)" >&2; exit 2 ;;
esac
DATA_DISK="${2:-}"
DATA_ARGS=()
if [[ -n "$DATA_DISK" ]]; then
  [[ -f "$DATA_DISK" ]] || { echo "Missing bFS data image: $DATA_DISK" >&2; exit 1; }
  case "${BOS_STORAGE:-virtio}" in
    virtio) DATA_ARGS=(-drive "if=none,id=users,format=raw,file=$DATA_DISK" -device virtio-blk-pci,disable-modern=on,drive=users) ;;
    ahci) DATA_ARGS=(-device ich9-ahci,id=data-ahci -drive "if=none,id=users,format=raw,file=$DATA_DISK" -device ide-hd,drive=users,bus=data-ahci.0) ;;
    nvme)
      NVME_NSID="${BOS_NVME_NSID:-1}"
      NVME_MDTS="${BOS_NVME_MDTS:-7}"
      NVME_LBA="${BOS_NVME_LBA:-512}"
      [[ "$NVME_NSID" =~ ^[1-9][0-9]{0,8}$ && "$NVME_MDTS" =~ ^[0-9]{1,2}$ && "$NVME_LBA" =~ ^(512|4096)$ ]] || { echo "Invalid NVMe test parameters" >&2; exit 2; }
      DATA_ARGS=(-drive "if=none,id=users,format=raw,file=$DATA_DISK" -device "nvme,id=data-nvme,serial=bos-users,mdts=$NVME_MDTS" -device "nvme-ns,bus=data-nvme,drive=users,nsid=$NVME_NSID,logical_block_size=$NVME_LBA,physical_block_size=$NVME_LBA") ;;

    *) echo "Unsupported BOS_STORAGE" >&2; exit 2 ;;
  esac
fi
ACCEL_ARGS=()
if [[ -n "${BOS_ACCEL:-}" ]]; then ACCEL_ARGS=(-accel "$BOS_ACCEL"); fi
exec qemu-system-x86_64 "${ACCEL_ARGS[@]}" \
  -machine q35 -cpu "${BOS_CPU:-max,vendor=GenuineIntel}" -smp 4 -m "$MEMORY" \
  -drive if=pflash,format=raw,readonly=on,file="$OVMF_CODE" \
  -drive file=fat:rw:"$ESP",format=raw,if=none,id=esp \
  -device ide-hd,drive=esp,bootindex=0 \
  "${NET_ARGS[@]}" \
  "${DATA_ARGS[@]}" -serial stdio -display none
