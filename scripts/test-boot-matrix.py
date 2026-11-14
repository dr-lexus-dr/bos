#!/usr/bin/env python3
"""Boot the release ESP at several RAM sizes and exercise M0 diagnostics.

The physical-memory bitmap is sized for 64 GiB.  A 4 GiB guest is enough to
put RAM above the q35 PCI hole, so a regression in the high-memory path shows
up without asking the CI runner for 32 GiB of real memory.
"""
import argparse
import os
import re
from pathlib import Path

from qemu_console import boot

ROOT = Path(__file__).resolve().parents[1]


def host_memory_gib():
    try:
        text = Path("/proc/meminfo").read_text()
    except OSError:
        return None
    match = re.search(r"^MemTotal:\s+(\d+) kB$", text, re.MULTILINE)
    return int(match.group(1)) / 1024 / 1024 if match else None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "memory",
        nargs="*",
        default=os.environ.get("BOS_BOOT_MATRIX", "512M 1G 4G").split(),
        help="guest memory sizes passed to BOS_MEMORY (default: 512M 1G 4G)",
    )
    args = parser.parse_args()

    esp = ROOT / "target/esp"
    if not (esp / "EFI").is_dir():
        raise SystemExit(f"missing {esp}/EFI; run scripts/build.sh --release first")

    host = host_memory_gib()
    for memory in args.memory:
        if host is not None and memory.upper().endswith("G"):
            requested = int(memory[:-1])
            if requested + 2 > host:
                print(f"{memory}: skipping (host has {host:.1f} GiB)", flush=True)
                continue
        with boot(esp, memory=memory, label=f"matrix-{memory}") as (shell, output):
            assert b"No data disk; /users is in RAM" in output
            shell.command("mem", b"phys ")
            bootstat = shell.command("bootstat", b"boot:")
            assert b"tsc:" in bootstat and b"dmesg:" in bootstat, bootstat
            shell.command("dmesg", b"=== bOS Kernel Starting ===")
            shell.command("halt", prompt=b"halted.\r\n")
        print(f"{memory}: boot and M0 diagnostics passed", flush=True)


if __name__ == "__main__":
    main()
