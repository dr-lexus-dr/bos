#!/bin/bash
set -e
# bOS: run in QEMU (UEFI via OVMF). Usage: run-qemu.sh [esp-dir] [users.img]
ESP="${1:-target/esp}"
OVMF_CODE="${OVMF_CODE:-/usr/share/OVMF/OVMF_CODE_4M.fd}"
DATA_DISK="${2:-}"
DATA_ARGS=()
if [[ -n "$DATA_DISK" ]]; then
  [[ -f "$DATA_DISK" ]] || { echo "Missing bFS data image: $DATA_DISK" >&2; exit 1; }
  DATA_ARGS=(-drive "if=none,id=users,format=raw,file=$DATA_DISK" -device virtio-blk-pci,disable-modern=on,drive=users)
fi
exec qemu-system-x86_64 \
  -machine q35 -cpu max -smp 4 -m "${BOS_MEMORY:-512M}" \
  -drive if=pflash,format=raw,readonly=on,file="$OVMF_CODE" \
  -drive file=fat:rw:"$ESP",format=raw,if=none,id=esp \
  -device ide-hd,drive=esp,bootindex=0 \
  -netdev user,id=n0 -device e1000,netdev=n0 \
  "${DATA_ARGS[@]}" -serial stdio -display none
