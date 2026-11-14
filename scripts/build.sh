#!/usr/bin/env bash
#
# bOS full build: host tools + UEFI bootloader + kernel + ESP staging.
#
# Layout produced (matches bloader paths \EFI\BOOT\BOOTX64.EFI, \EFI\bOS\...):
#   <esp>/EFI/BOOT/BOOTX64.EFI   <- copy of bloader.efi
#   <esp>/EFI/bOS/kernel.bin     <- bos_kernel ELF
#   <esp>/EFI/bOS/initrd.img     <- tar with the bundled bOS files
#
# Usage:
#   bash scripts/build.sh [--release] [--esp DIR]
#                         [--skip-bootloader] [--skip-kernel] [--skip-initrd]
#                         [--host-only] [-h|--help]
#
# Examples:
#   bash scripts/build.sh                      # debug build, ESP -> target/esp
#   bash scripts/build.sh --release            # release build for everything
#   bash scripts/build.sh --host-only          # only bfs/binit/bsh/bcoreutils
#   bash scripts/build.sh --esp /tmp/esp-test  # custom ESP staging dir
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PROFILE="debug"
CARGO_PROFILE_FLAG=""          # empty = debug, "--release" = release
ESP="$ROOT/target/esp"
SKIP_BOOTLOADER=0
SKIP_KERNEL=0
SKIP_INITRD=0
HOST_ONLY=0

UEFI_TARGET="x86_64-unknown-uefi"
KERNEL_TARGET="x86_64-unknown-none"
HOST_PKGS=(bfs binit bsh bcoreutils)
HOST_BINS=(binit bsh bcoreutils)

log()  { printf '[build] %s\n' "$*"; }
die()  { printf '[build] ERROR: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "required tool '$1' not found in PATH"; }

usage() { awk '/^# bOS full build/{p=1} /^set -euo pipefail/{p=0} p' "$0" | sed 's/^# \?//'; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        --release)          PROFILE="release"; CARGO_PROFILE_FLAG="--release"; shift ;;
        --esp)              ESP="${2:?--esp needs a directory}"; shift 2 ;;
        --esp=*)            ESP="${1#--esp=}"; shift ;;
        --skip-bootloader)  SKIP_BOOTLOADER=1; shift ;;
        --skip-kernel)      SKIP_KERNEL=1; shift ;;
        --skip-initrd)      SKIP_INITRD=1; shift ;;
        --host-only)        HOST_ONLY=1; shift ;;
        -h|--help)          usage; exit 0 ;;
        *)                  die "unknown option '$1' (try --help)" ;;
    esac
done

if [[ "$HOST_ONLY" -eq 1 ]]; then
    SKIP_BOOTLOADER=1
    SKIP_KERNEL=1
    SKIP_INITRD=1
fi

need cargo
need rustc
log "toolchain: $(rustc --version) (profile: $PROFILE)"

# --- 0. toolchain + targets required for firmware builds -----------------------
# Note: `rustc --print target-list` lists every target rustc *knows about*,
# not the ones with installed std — so probe the sysroot for libcore instead.
target_has_std() {
    local sysroot tdir
    sysroot="$(rustc --print sysroot 2>/dev/null)" || return 1
    tdir="$sysroot/lib/rustlib/$1/lib"
    [[ -d "$tdir" ]] && compgen -G "$tdir/libcore*.rlib" >/dev/null
}

if [[ "$SKIP_BOOTLOADER" -eq 0 || "$SKIP_KERNEL" -eq 0 ]]; then
    if ! rustc --version | grep -qi nightly; then
        die "firmware builds need nightly Rust ($(rustc --version) detected). Install rustup (https://rustup.rs), then: cd $ROOT && rustup toolchain install && rustup target add $UEFI_TARGET $KERNEL_TARGET"
    fi
    missing_targets=()
    for t in "$UEFI_TARGET" "$KERNEL_TARGET"; do
        target_has_std "$t" || missing_targets+=("$t")
    done
    if [[ "${#missing_targets[@]}" -gt 0 ]]; then
        if command -v rustup >/dev/null 2>&1; then
            log "installing missing targets: ${missing_targets[*]}"
            rustup target add "${missing_targets[@]}" \
                || die "rustup target add failed; install manually: rustup target add ${missing_targets[*]}"
        else
            die "missing rust target std: ${missing_targets[*]}. With rustup: rustup target add ${missing_targets[*]}"
        fi
    fi
fi

# --- 1. host tools (bfs lib, binit/bsh/bcoreutils bins) -----------------------
log "[1/4] host tools: ${HOST_PKGS[*]} ($PROFILE)..."
# shellcheck disable=SC2086
cargo build --locked $CARGO_PROFILE_FLAG -p bfs -p binit -p bsh -p bcoreutils
for b in "${HOST_BINS[@]}"; do
    [[ -x "target/$PROFILE/$b" ]] || die "host binary missing after build: target/$PROFILE/$b"
done
log "host tools OK: $(ls target/$PROFILE/{binit,bsh,bcoreutils} | tr '\n' ' ')"

# --- 2. UEFI bootloader --------------------------------------------------------
if [[ "$SKIP_BOOTLOADER" -eq 0 ]]; then
    log "[2/4] bootloader (bloader -> $UEFI_TARGET/$PROFILE)..."
    # shellcheck disable=SC2086
    cargo build --locked $CARGO_PROFILE_FLAG -p bloader --target "$UEFI_TARGET"
    BLOADER_EFI="target/$UEFI_TARGET/$PROFILE/bloader.efi"
    [[ -f "$BLOADER_EFI" ]] || die "bootloader artifact missing: $BLOADER_EFI"
    log "bootloader OK: $BLOADER_EFI ($(stat -c%s "$BLOADER_EFI") bytes)"
else
    log "[2/4] bootloader skipped"
fi

# --- 2b. userspace shell (bossh) -----------------------------------------------
if [[ "$SKIP_KERNEL" -eq 0 ]]; then
    log "userspace shell (bossh -> x86_64-unknown-none/$PROFILE)..."
    (cd "$ROOT/userspace/bossh" && cargo +nightly-2025-06-01 build --locked $CARGO_PROFILE_FLAG --target x86_64-unknown-none) \
        || die "bossh build failed"
    BOSsh="$ROOT/userspace/bossh/target/x86_64-unknown-none/$PROFILE/bossh"
    [[ -f "$BOSsh" ]] || die "bossh artifact missing: $BOSsh"
    mkdir -p "$ROOT/target/user"
    cp -f "$BOSsh" "$ROOT/target/user/shell.bh"
    log "bossh OK: $BOSsh"
fi

# --- 3. kernel -----------------------------------------------------------------
if [[ "$SKIP_KERNEL" -eq 0 ]]; then
    log "[3/4] kernel (bos_kernel -> $KERNEL_TARGET/$PROFILE)..."
    # shellcheck disable=SC2086
    cargo build --locked $CARGO_PROFILE_FLAG -p bos_kernel --target "$KERNEL_TARGET"
    KERNEL_BIN="target/$KERNEL_TARGET/$PROFILE/bos_kernel"
    [[ -f "$KERNEL_BIN" ]] || die "kernel artifact missing: $KERNEL_BIN"
    log "kernel OK: $KERNEL_BIN ($(stat -c%s "$KERNEL_BIN") bytes)"
else
    log "[3/4] kernel skipped"
fi

# --- 4. ESP staging ------------------------------------------------------------
if [[ "$SKIP_BOOTLOADER" -eq 0 || "$SKIP_KERNEL" -eq 0 || "$SKIP_INITRD" -eq 0 ]]; then
    log "[4/4] staging ESP at $ESP ..."
    mkdir -p "$ESP/EFI/BOOT" "$ESP/EFI/bOS"

    if [[ "$SKIP_BOOTLOADER" -eq 0 ]]; then
        cp -f "target/$UEFI_TARGET/$PROFILE/bloader.efi" "$ESP/EFI/BOOT/BOOTX64.EFI"
    fi
    if [[ "$SKIP_KERNEL" -eq 0 ]]; then
        cp -f "target/$KERNEL_TARGET/$PROFILE/bos_kernel" "$ESP/EFI/bOS/kernel.bin"
    fi
    if [[ "$SKIP_INITRD" -eq 0 ]]; then
        need tar
        need as
        need ld
        mkdir -p target/user
        as --64 userspace/hello/start.S -o target/user/hello.o
        ld -m elf_x86_64 -T userspace/hello/link.ld target/user/hello.o -o target/user/hello.bh
        mkdir -p target/user/applications
        cp -f target/user/shell.bh target/user/applications/shell.bh 2>/dev/null || true
        tar --format=ustar --mtime=@0 --owner=0 --group=0 \
            -cf "$ESP/EFI/bOS/initrd.img" \
            applications/calculator.bh applications/notepad.bh \
            system/init.sh system/bpt system/bpm system/network system/text-editor \
            users/default/.profile -C target/user --transform='s,^hello\.bh$,applications/hello.bh,' hello.bh \
            -C target/user applications/shell.bh
        tar --format=ustar --mtime=@0 --owner=0 --group=0 \
            -cf "$ESP/EFI/bOS/initrd.img" \
            applications/calculator.bh applications/notepad.bh \
            system/init.sh system/bpt system/bpm system/network system/text-editor \
            users/default/.profile -C target/user --transform='s,^hello\.bh$,applications/hello.bh,' hello.bh \\
            -C target/user applications/shell.bh
    fi

    log "ESP contents:"
    find "$ESP" -type f -printf '  %p (%s bytes)\n' | sort
else
    log "[4/4] ESP staging skipped (host-only build)"
fi

log "done. Next: bash scripts/run-qemu.sh ${ESP#$ROOT/}"
