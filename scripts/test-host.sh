#!/usr/bin/env bash
#
# Run every host-testable package in the workspace.
#
# The kernel and UEFI bootloader target firmware, so `cargo test --workspace`
# cannot be used: it tries to build those crates for the host.  This list is
# the set that builds and runs on Linux.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

exec cargo test --locked \
    -p bfs \
    -p binit \
    -p bsh \
    -p bcoreutils \
    -p bpt \
    -p bnet \
    -p btext \
    -p bloader_shared \
    -p bcompat-win32 \
    -p bcompat-macho \
    -p bcompat-elf \
    -p bdriver-fs \
    -p bdriver-net \
    -p blog \
    -p bconfig \
    "$@"
