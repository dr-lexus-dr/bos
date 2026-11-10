#!/bin/bash
set -e
# bOS: run in QEMU (UEFI via OVMF)
ESP="${1:-target/esp}"
OVMF_CODE="${OVMF_CODE:-/usr/share/OVMF/OVMF_CODE_4M.fd}"
exec qemu-system-x86_64 \
  -machine q35 -cpu max -smp 4 -m 2G \
  -drive if=pflash,format=raw,readonly=on,file="$OVMF_CODE" \
  -drive file=fat:rw:"$ESP",format=raw,if=none,id=esp \
  -device ide-hd,drive=esp,bootindex=0 \
  -netdev user,id=n0 -device e1000,netdev=n0 \
  -serial stdio -display none
