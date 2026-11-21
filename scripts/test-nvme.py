#!/usr/bin/env python3
"""Exercise real NVMe queue wrap, PRP modes, namespace selection and safe refusal."""
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
from qemu_console import boot

ROOT = Path(__file__).resolve().parents[1]
PROMPT = b"bSH ~ % "


def main():
    os.environ["BOS_STORAGE"] = "nvme"
    os.environ["BOS_NVME_NSID"] = "7"
    with tempfile.TemporaryDirectory(prefix="bos-nvme-") as directory:
        disk = Path(directory) / "users.img"
        # MDTS=1 limits transfers to 8 KiB (direct PRP2); MDTS=4 permits the
        # driver's 64 KiB maximum (PRP list). Whole-volume fsync wraps queues.
        for mdts, lba in (("1", "512"), ("4", "512"), ("1", "4096")):
            os.environ["BOS_NVME_LBA"] = lba
            disk.unlink(missing_ok=True)
            os.environ["BOS_NVME_MDTS"] = mdts
            subprocess.run([ROOT / "target/release/mkusers", disk], check=True)
            with boot(ROOT / "target/esp", disk, recovery=False, prompt=PROMPT,
                      label=f"nvme-lba-{lba}-mdts-{mdts}-write") as (shell, output):
                assert b"NVMe data disk: namespace 7" in output
                assert b"/users is persistent on nvme0" in output
                shell.command("/applications/native-demo.bh", b"libbos native test passed", prompt=PROMPT)
            with boot(ROOT / "target/esp", disk, recovery=False, prompt=PROMPT,
                      label=f"nvme-lba-{lba}-mdts-{mdts}-read") as (shell, output):
                assert b"/users is persistent on nvme0" in output
                shell.command("cat /users/default/documents/native.txt", b"native Rust file I/O", prompt=PROMPT)

        with disk.open("r+b") as image:
            image.seek(88)
            image.write(b"otherdisk\0")
        original = hashlib.sha256(disk.read_bytes()).digest()
        with boot(ROOT / "target/esp", disk, recovery=False, prompt=PROMPT,
                  label="nvme-ignore-unlabelled") as (shell, output):
            assert b"No data disk; /users is in RAM" in output
            shell.command("sync", prompt=PROMPT)
        assert hashlib.sha256(disk.read_bytes()).digest() == original
    print("NVMe: NSID 7, 512/4096-byte LBAs, both PRP modes, queue wrap, persistence and safe refusal passed")


if __name__ == "__main__":
    main()
