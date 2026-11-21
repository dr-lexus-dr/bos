#!/usr/bin/env python3
"""Build the boot resource and preserve notices from the libraries it contains."""

import os
import json
from pathlib import Path
import struct
import subprocess
import sys
import zlib

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "target"
module = (OUT / "browser.wasm").read_bytes()
encoded = zlib.compress(module, 6)
bundle = struct.pack("<4sII", b"BWEB", len(encoded), len(module)) + encoded
(OUT / "browser.bweb.tmp").write_bytes(bundle)
os.replace(OUT / "browser.bweb.tmp", OUT / "browser.bweb")
cmd = sys.argv[1]
go_env = {**os.environ, "GOOS": "wasip1", "GOARCH": "wasm", "CGO_ENABLED": "0"}
packages = subprocess.check_output(
    [
        cmd,
        "list",
        "-modfile=" + str(OUT / "browser-sdk/bos.mod"),
        "-deps",
        "-f",
        "{{if .Module}}{{.Module.Path}}|{{.Module.Version}}|{{.Module.Dir}}{{end}}",
        ".",
    ],
    cwd=ROOT / "userspace/browser",
    env=go_env,
    text=True,
)
goroot = Path(subprocess.check_output([cmd, "env", "GOROOT"], text=True).strip())
parts = [
    "bOS 1.0 — bundled software notices",
    "The source for this image is available at https://github.com/dr-lexus-dr/bOS/tree/v1.0.0.",
]
seen = set()


def add(label, path):
    if path.is_file():
        data = path.read_text(errors="replace").strip()
        if data not in seen:
            parts.extend([label, data])
            seen.add(data)


add("bOS", ROOT / "LICENSE")
add("Go runtime and standard library", goroot / "LICENSE")
for rel in (
    "desktop-font.LICENSE",
    "inter.LICENSE",
    "papirus/NOTICE",
    "lzma/NOTICE",
):
    add(rel, ROOT / "third_party" / rel)
for row in sorted(set(packages.splitlines())):
    if not row or row.startswith("bos.local/"):
        continue
    name, version, directory = row.split("|")
    base = Path(directory)
    parts.append(
        name
        + " "
        + (version or ("v0.48.0" if name == "github.com/go-webengine/engine" else ""))
    )
    for path in sorted(base.iterdir()):
        if path.name.lower().startswith(
            ("license", "licence", "copyright", "copying", "notice", "ofl")
        ):
            add(str(path.name), path)
    if name in ("github.com/go-opentype/fonts", "github.com/go-gfx/gfx"):
        for path in sorted(base.rglob("*")):
            if path.name.lower().startswith(
                ("ofl", "license", "licence", "copying", "notice", "copyright")
            ):
                add(name + "/" + str(path.relative_to(base)), path)
# Include the Rust libraries linked into the native kernel and EFI loader.
firmware = {}
for package, target, feature in (
    ("bos_kernel", "x86_64-unknown-none", "live"),
    ("bloader", "x86_64-unknown-uefi", "live-efi"),
):
    metadata = json.loads(
        subprocess.check_output(
            [
                "cargo",
                "metadata",
                "--format-version",
                "1",
                "--locked",
                "--filter-platform",
                target,
                "--features",
                package + "/" + feature,
            ],
            cwd=ROOT,
        )
    )
    crates = {item["id"]: item for item in metadata["packages"]}
    nodes = {item["id"]: item for item in metadata["resolve"]["nodes"]}
    wanted = [item["id"] for item in crates.values() if item["name"] == package]
    visited = set()
    while wanted:
        ident = wanted.pop()
        if ident in visited:
            continue
        visited.add(ident)
        crate = crates[ident]
        if crate["source"]:
            firmware[(crate["name"], crate["version"])] = crate
        for dep in nodes[ident]["deps"]:
            if any(kind["kind"] is None for kind in dep["dep_kinds"]):
                wanted.append(dep["pkg"])
for (name, version), crate in sorted(firmware.items()):
    parts.append(f"{name} {version}  {crate['license'] or ''}")
    base = Path(crate["manifest_path"]).parent
    for path in sorted(base.iterdir()):
        if path.name.lower().startswith(
            ("license", "licence", "copyright", "copying", "notice")
        ):
            add(name + "/" + path.name, path)
(OUT / "browser-notices.txt").write_text("\n\n".join(parts) + "\n")
