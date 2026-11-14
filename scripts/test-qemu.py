#!/usr/bin/env python3
"""Smoke-test the release ESP, a disposable bFS disk, and Ethernet at 512 MiB."""
import hashlib
import os
from pathlib import Path
import select
import shutil
import subprocess
import tempfile
import time
from contextlib import contextmanager

ROOT = Path(__file__).resolve().parents[1]
PROMPT = b"bSH:/users/default# "


class Console:
    def __init__(self, process):
        self.process = process
        self.output = bytearray()

    def expect(self, token):
        deadline = time.monotonic() + 45
        while token not in self.output:
            if not select.select([self.process.stdout], [], [], max(0, deadline - time.monotonic()))[0]:
                raise AssertionError(f"Timed out waiting for {token!r}\n{self.output[-3000:]!r}")
            data = os.read(self.process.stdout.fileno(), 8192)
            if not data:
                raise AssertionError(f"QEMU exited waiting for {token!r}\n{self.output[-3000:]!r}")
            self.output.extend(data)
        offset = self.output.index(token)
        response = bytes(self.output[:offset])
        del self.output[:offset + len(token)]
        return response

    def command(self, line, expected=b"", prompt=PROMPT):
        data = line.encode() if isinstance(line, str) else line
        self.process.stdin.write(data + b"\r")
        self.process.stdin.flush()
        response = self.expect(prompt)
        if expected not in response:
            raise AssertionError(f"{line!r}: missing {expected!r} in {response!r}")
        return response


@contextmanager
def boot(esp, disk=None, *, nic="e1000"):
    command = ["bash", "scripts/run-qemu.sh", str(esp)]
    if disk is not None:
        command.append(str(disk))
    start = time.monotonic()
    with subprocess.Popen(
        command, cwd=ROOT, env={**os.environ, "BOS_MEMORY": "512M", "BOS_NIC": nic},
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
    ) as process:
        console = Console(process)
        try:
            output = console.expect(PROMPT)
            print(f"bSH ready after {time.monotonic() - start:.1f}s", flush=True)
            yield console, output
        finally:
            if process.poll() is None:
                process.terminate()
            process.wait(timeout=10)


def fixture_esp(work):
    esp = work / "esp"
    shutil.copytree(ROOT / "target/esp/EFI", esp / "EFI")
    apps = work / "applications"
    apps.mkdir()
    (apps / "bad-elf").write_bytes(b"not an ELF")
    source = work / "fault.S"
    source.write_text(".text\n.global _start\n_start: mov 0, %rax\n ud2\n")
    obj = work / "fault.o"
    subprocess.run(["as", "--64", str(source), "-o", str(obj)], check=True)
    subprocess.run(
        ["ld", "-m", "elf_x86_64", "-T", "userspace/hello/link.ld", str(obj), "-o", str(apps / "fault")],
        cwd=ROOT, check=True,
    )
    subprocess.run(
        ["tar", "-rf", str(esp / "EFI/bOS/initrd.img"), "-C", str(work),
         "applications/bad-elf", "applications/fault"], check=True,
    )
    return esp


def wait_for_dhcp(shell):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        status = shell.command("ifconfig")
        if b"inet 10.0.2.15/24 (DHCP)" in status:
            assert b"gateway 10.0.2.2" in status
            return status
        time.sleep(0.1)
    raise AssertionError(f"No DHCP lease\n{status!r}")


def check_network(esp):
    for nic, model in (
        ("e1000", b"Intel 82540EM"),
        ("e1000e", b"Intel 82574L"),
        ("rtl8139", b"Realtek RTL8139"),
    ):
        with boot(esp, nic=nic) as (shell, _):
            assert model in wait_for_dhcp(shell)
            # Enough traffic to wrap the RTL8139's 8 KiB RX ring.
            for _ in range(140 if nic == "rtl8139" else 1):
                shell.command("ping 10.0.2.2", b"reply from 10.0.2.2")
            if nic == "e1000":
                shell.command("ifconfig static 10.0.2.15/24 192.168.0.1", b"gateway must be another host")
                shell.command("ifconfig", b"inet 10.0.2.15/24 (DHCP)")
                shell.command("ifconfig static 10.0.2.15/24 10.0.2.2", b"inet 10.0.2.15/24 (static)")
                shell.command("ping 10.0.2.2", b"reply from 10.0.2.2")
                shell.command("ifconfig dhcp")
                wait_for_dhcp(shell)

    with boot(esp, nic="none") as (shell, _):
        shell.command("ifconfig", b"No supported Ethernet adapter")
        shell.command("ping 10.0.2.2", b"no supported Ethernet adapter")


def main():
    with tempfile.TemporaryDirectory(prefix="bos-test-") as directory:
        work = Path(directory)
        esp = fixture_esp(work)
        disk = work / "users.img"
        subprocess.run([ROOT / "target/release/mkusers", disk], check=True)
        with boot(esp) as (shell, output):
            assert b"No data disk; /users is in RAM" in output
            assert b"\x1b[96mbOS kernel console (bSH) v" in output
            # userspace shell is default; drop to bSH for the kernel-side checks
            shell.command("exit")
            shell.command("lspci", b"8086:100e Ethernet")
            shell.command("calculator 2 + 3 * 4", b"\r\n14\r\n")
            shell.command("bfetch", b"Shell: bSH")
            shell.command("echo history", b"\r\nhistory\r\n")
            shell.command("echo history", b"\r\nhistory\r\n")
            shell.command("echo abc", b"\r\nabc\r\n")
            shell.command("/applications/hello", b"Hello from bOS user mode!")
            shell.command("run /applications/bad-elf", b"expected a static x86-64 ELF")
            shell.command("/applications/fault", b"user fault (vector 14)")
            shell.command("/applications/hello", b"Hello from bOS user mode!")

        with boot(esp, disk) as (shell, output):
            assert b"/users is persistent on virtio0" in output
            shell.command("exit")
            shell.command("echo saved > documents/notes.txt")
            shell.command("touch documents/remove-me")
            shell.command("echo renamed > documents/before")
            shell.command("mv documents/before downloads/after")
            shell.command("sync")
            shell.command("halt", prompt=b"halted.\r\n")

        with boot(esp, disk) as (shell, output):
            assert b"/users is persistent on virtio0" in output
            shell.command("cat documents/notes.txt", b"\r\nsaved\r\n")
            shell.command("cat downloads/after", b"\r\nrenamed\r\n")
            shell.command("rm documents/remove-me")
            shell.command("halt", prompt=b"halted.\r\n")

        with boot(esp, disk) as (shell, _):
            shell.command("cat documents/remove-me", b"cannot open file")

        with disk.open("r+b") as image:
            image.write(b"BAD!")
        original = hashlib.sha256(disk.read_bytes()).digest()
        with boot(esp, disk) as (shell, output):
            assert b"Data disk rejected; /users is in RAM" in output
            shell.command("halt", prompt=b"halted.\r\n")
        assert hashlib.sha256(disk.read_bytes()).digest() == original
        check_network(esp)

    print("Live boot, line editing, ring-3 execution, bFS persistence and Ethernet passed")


if __name__ == "__main__":
    main()
