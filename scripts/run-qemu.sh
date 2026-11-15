#!/bin/bash
set -e
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
exec qemu-system-x86_64 \
  -machine q35 -cpu max -smp 4 -m "$MEMORY" \
  -drive if=pflash,format=raw,readonly=on,file="$OVMF_CODE" \
  -drive file=fat:rw:"$ESP",format=raw,if=none,id=esp \
  -device ide-hd,drive=esp,bootindex=0 \
  "${NET_ARGS[@]}" \
  "${DATA_ARGS[@]}" -serial stdio -display none
