#!/usr/bin/env bash
# The .wasm executable ships on the boot medium; Go is only a build dependency.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root/userspace/browser"
if [[ -n ${BOS_GO:-} ]]; then go_cmd="$BOS_GO"
elif command -v go >/dev/null; then go_cmd=$(command -v go)
elif [[ -x "$root/target/toolchain/go/bin/go" ]]; then go_cmd="$root/target/toolchain/go/bin/go"
else echo 'Install Go 1.27.4 or newer, or set BOS_GO to the Go executable.' >&2; exit 1
fi
mkdir -p "$root/target"
directory=$("$go_cmd" list -m -f '{{.Dir}}' github.com/go-webengine/engine)
if [[ -z "$directory" ]]; then "$go_cmd" mod download; directory=$("$go_cmd" list -m -f '{{.Dir}}' github.com/go-webengine/engine); fi
sdk="$root/target/browser-sdk"
source_tree="$sdk/engine-v0.48.0"
mkdir -p "$sdk"
if [[ ! -f "$source_tree/go.mod" ]]; then
    cp -R "$directory" "$source_tree"
    chmod u+w "$source_tree"
fi
for file in css/lightdark.go css/media.go js/window.go settle_scripts.go interactive.go js/session.go js/events.go; do
    chmod u+w "$(dirname "$source_tree/$file")"
    install -m 644 "$directory/$file" "$source_tree/$file"
done
for feature in appearance origin scroll events; do
    patch --quiet --fuzz=0 --directory="$source_tree" -p1 < "$root/third_party/patches/webengine-$feature.patch"
done
chmod u+w "$source_tree/layout"
install -m 644 "$root/third_party/webengine/scroll_layout.go" "$source_tree/layout/bos_scroll.go"
install -m 644 "$root/third_party/webengine/origin.go" "$source_tree/bos_origin.go"
install -m 644 "$root/third_party/webengine/viewport.go" "$source_tree/bos_viewport.go"
cp go.mod "$sdk/bos.mod"
cp go.sum "$sdk/bos.sum"
"$go_cmd" mod edit -modfile="$sdk/bos.mod" -replace="github.com/go-webengine/engine=../../target/browser-sdk/engine-v0.48.0"
GOOS=wasip1 GOARCH=wasm CGO_ENABLED=0 "$go_cmd" build -modfile="$sdk/bos.mod" -mod=readonly -trimpath -buildvcs=false -ldflags='-s -w' -o "$root/target/browser-go.wasm" .
cargo build --release --locked --manifest-path "$root/Cargo.toml" -p bos_browser_instrument
tool="$root/target/release/bos_browser_instrument"
if [[ ! "$root/target/browser.wasm" -nt "$tool" || ! "$root/target/browser.wasm" -nt "$root/target/browser-go.wasm" ]]; then
    "$tool" "$root/target/browser-go.wasm" "$root/target/browser.wasm.tmp"
    mv "$root/target/browser.wasm.tmp" "$root/target/browser.wasm"
fi

python3 "$root/scripts/browser-bundle.py" "$go_cmd"
