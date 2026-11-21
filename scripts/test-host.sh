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

mkdir -p target/host-tests
for module in free_list; do
    rustc --edition=2021 --test "kernel/src/mm/$module.rs" -o "target/host-tests/$module"
    "target/host-tests/$module"
done

rustc --edition=2021 --test userspace/native-demo/src/shell_line.rs -o target/host-tests/shell-line
target/host-tests/shell-line

rustc --edition=2021 --test tests/web-reader.rs -o target/host-tests/web-reader
target/host-tests/web-reader

exec cargo test --locked \
    -p bos-ipc-tests \
    -p libbos \
    -p bos-memory-tests \
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
