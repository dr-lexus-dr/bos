#!/usr/bin/env python3
"""Check FAT read/write and language persistence across two read-only live boots."""

import os
import subprocess
import tempfile
from pathlib import Path

from qemu_console import Console, ROOT

PROMPT = b"% "


def check(bits, transport, directory):
    disk = directory / f"fat{bits}.img"
    with disk.open("xb") as image:
        image.truncate(1_474_560 if bits == 12 else 16 * 1024**2)
    subprocess.run(["mkfs.fat", "-F", str(bits), "-n", "BOSDATA", disk], check=True)
    env = {**os.environ, "BOS_NIC": "none"}
    args = ["bash", "scripts/run-qemu.sh", "--live", "-display", "none", "-no-reboot"]
    if transport == "virtio":
        env["BOS_DATA"] = str(disk)
    else:
        env.pop("BOS_DATA", None)
        args.extend(["-drive", f"if=ide,format=raw,file={disk}"])
    for boot, language in enumerate([b"en", b"ru"]):
        with subprocess.Popen(args, cwd=ROOT, env=env, stdin=subprocess.PIPE,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT) as guest:
            tty = Console(guest)
            try:
                tty.expect(b"Desktop ready:", 100)
                tty.expect(b"language=" + language)
                tty.keys(b"\x11")
                tty.expect(b"All commands available")
                tty.expect(PROMPT)
                tty.command("mount", f"{transport}0 on /users type fat".encode(), PROMPT)
                if boot == 0:
                    tty.command("language ru", b"ru", PROMPT)
                    tty.command("mkdir sample", prompt=PROMPT)
                    tty.command("echo first > sample/keep.txt", prompt=PROMPT)
                    tty.command("echo second >> sample/keep.txt", prompt=PROMPT)
                    tty.command("mv sample/keep.txt sample/note.txt", prompt=PROMPT)
                    tty.command("sync", prompt=PROMPT)
                content = tty.command("cat sample/note.txt", b"first", PROMPT)
                assert b"second" in content
                if boot == 1:
                    tty.command("rm sample/note.txt", prompt=PROMPT)
                    tty.command("rmdir sample", prompt=PROMPT)
            finally:
                guest.terminate()
                guest.wait(timeout=10)
                tty.raw.extend(guest.stdout.read())
                (ROOT / "target/live" / f"fat{bits}-{transport}-boot{boot}.log").write_bytes(tty.raw)
    print(f"PASS FAT{bits} on {transport}, file operations and persistent Russian desktop")


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="bos-fat-") as tmp:
        check(12, "virtio", Path(tmp))
        check(16, "ata", Path(tmp))
