#!/usr/bin/env bash
#
# Build versioned release assets: Live ISO, Install ISO, ESP archive, checksums.
#
# Usage: bash scripts/release.sh [VERSION]
# The version defaults to kernel/Cargo.toml. Artifacts land in
# target/release-<VERSION>/ and are safe to attach to a GitHub release.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="${1:-$(awk -F'"' '/^version = / { print $2; exit }' kernel/Cargo.toml)}"
[[ -n "$VERSION" ]] || { echo "cannot read version" >&2; exit 1; }
OUT="target/release-$VERSION"
mkdir -p "$OUT"

echo "[release] building bOS $VERSION"
bash scripts/build.sh --release

echo "[release] writing ISOs to $OUT"
python3 scripts/mkiso.py \
    --out "$OUT/bos-$VERSION-x86_64-live.iso"
python3 scripts/mkiso.py --install \
    --out "$OUT/bos-$VERSION-x86_64-install.iso"

# NvVars is QEMU/OVMF state, not part of the release payload.
tar -C target/esp --exclude=NvVars -czf \
    "$OUT/bos-$VERSION-x86_64-esp.tar.gz" .

(
    cd "$OUT"
    sha256sum ./*.iso ./*.tar.gz > SHA256SUMS
)

echo "[release] assets:"
find "$OUT" -maxdepth 1 -type f -printf '  %f (%s bytes)\n' | sort
echo "[release] next: git tag -a v$VERSION -m \"bOS $VERSION\" && git push origin v$VERSION"
