#!/usr/bin/env bash
# Build pinned upstream tools for the Unix host or bOS Linux ABI (--native).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
PROFILE=debug
FLAGS=()
NATIVE=0
TARGET_FLAGS=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --release) PROFILE=release; FLAGS=(--release) ;;
        --native) NATIVE=1 ;;
        *) echo 'usage: build-upstream-userspace.sh [--release] [--native]' >&2; exit 2 ;;
    esac
    shift
done
CACHE="$ROOT/target/upstream"
TARGET="$ROOT/target/upstream-build"
BIN="$ROOT/target/$PROFILE/host/bin"
if [[ "$NATIVE" -eq 1 ]]; then
    PROFILE=release
    FLAGS=(--release)
    TARGET_FLAGS=(--target x86_64-unknown-linux-musl)
    TARGET="$ROOT/target/upstream-musl"
    BIN="$ROOT/target/native/bin"
    rustup +stable target add x86_64-unknown-linux-musl
fi
mkdir -p "$CACHE" "$BIN"
fetch() {
    local name="$1" url="$2" revision="$3"
    if [[ ! -d "$CACHE/$name/.git" ]]; then
        git init -q "$CACHE/$name"
        git -C "$CACHE/$name" remote add origin "$url"
    fi
    if ! git -C "$CACHE/$name" cat-file -e "$revision^{commit}" 2>/dev/null; then
        git -C "$CACHE/$name" fetch --depth 1 origin "$revision"
    fi
    git -C "$CACHE/$name" checkout --detach "$revision"
}
fetch nash https://github.com/fourtexec/nash 968615878ad66f2e05e4fb4e5ffa83ee1c936cdc
fetch coreutils https://github.com/uutils/coreutils 408b97624daf9973f0df9009b8762e84708ff6e4
# Apply the Linux libc resource-type compatibility fix idempotently.
if git -C "$ROOT/target/upstream/nash" apply --check "$ROOT/third_party/patches/nash-linux-rlimit.patch"; then
    git -C "$ROOT/target/upstream/nash" apply "$ROOT/third_party/patches/nash-linux-rlimit.patch"
else
    git -C "$ROOT/target/upstream/nash" apply --reverse --check "$ROOT/third_party/patches/nash-linux-rlimit.patch"
fi
# Upstream uses newer Rust than the firmware's pinned nightly.
for name in nash coreutils; do
    FEATURES=()
    if [[ "$name" == coreutils ]]; then
        if [[ "$NATIVE" -eq 1 ]]; then FEATURES=(--features feat_os_unix_musl); else FEATURES=(--features unix); fi
    fi
    (cd "$CACHE/$name" && cargo +stable build --locked "${FLAGS[@]}" \
        --target-dir "$TARGET/$name" "${TARGET_FLAGS[@]}" "${FEATURES[@]}" --bin "$name")
    artifact="$TARGET/$name/$PROFILE/$name"
    if [[ "$NATIVE" -eq 1 ]]; then artifact="$TARGET/$name/x86_64-unknown-linux-musl/$PROFILE/$name"; fi
    cp "$artifact" "$BIN/$name"
done
# Use exactly the utilities enabled in this build, including argv[0] dispatch.
"$BIN/coreutils" --list > "$BIN/../coreutils.list"
while IFS= read -r utility; do
    [[ "$utility" =~ ^[a-zA-Z0-9_\[-]+$ ]] || { echo "invalid utility: $utility" >&2; exit 1; }
    ln -sfn coreutils "$BIN/$utility"
done < "$BIN/../coreutils.list"
printf 'Host userspace: %s\n' "$BIN"

if [[ "$NATIVE" -eq 1 ]]; then
    # ring's portable x86-64 C objects link against the self-contained musl runtime.
    CC_x86_64_unknown_linux_musl="${CC_x86_64_unknown_linux_musl:-gcc}" \
        cargo +stable build --locked -p bpt --target x86_64-unknown-linux-musl
    strip -o "$BIN/bpt" "$ROOT/target/x86_64-unknown-linux-musl/debug/bpt"
    cp "$ROOT/userspace/bfetch/bfetch" "$BIN/bfetch"
    ln -sfn nash "$BIN/sh"
fi
