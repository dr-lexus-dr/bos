#!/usr/bin/env python3
"""Smoke-test the release ESP, a disposable bFS disk, and Ethernet."""
import hashlib
import re
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

from qemu_console import PROMPT, boot

ROOT = Path(__file__).resolve().parents[1]
PERSISTENT = b"/users is persistent on " + {"virtio": b"virtio0", "ahci": b"ahci0", "nvme": b"nvme0"}[os.environ.get("BOS_STORAGE", "virtio")]


def fixture_esp(work):
    esp = work / "esp"
    shutil.copytree(ROOT / "target/esp/EFI", esp / "EFI")
    apps = work / "applications"
    apps.mkdir()
    (apps / "bad-elf.bh").write_bytes(b"not an ELF")
    source = work / "fault.S"
    source.write_text(".text\n.global _start\n_start: mov 0, %rax\n ud2\n")
    obj = work / "fault.o"
    subprocess.run(["as", "--64", str(source), "-o", str(obj)], check=True)
    subprocess.run(
        ["ld", "-m", "elf_x86_64", "-T", "userspace/hello/link.ld", str(obj), "-o", str(apps / "fault.bh")],
        cwd=ROOT, check=True,
    )
    subprocess.run(["as", "--64", str(ROOT / "tests/user/file-io.S"), "-o", str(obj)], check=True)
    subprocess.run(
        ["ld", "-m", "elf_x86_64", "-T", "userspace/hello/link.ld", str(obj), "-o", str(apps / "file-io.bh")],
        cwd=ROOT, check=True,
    )
    # Same resource pressure, but terminate by page fault with handles open.
    source.write_text((ROOT / "tests/user/file-io.S").read_text().replace(
        "    mov $60, %eax\n    xor %edi, %edi\n    int $0x80",
        "    mov 0, %rax\n    ud2"))
    subprocess.run(["as", "--64", str(source), "-o", str(obj)], check=True)
    subprocess.run(
        ["ld", "-m", "elf_x86_64", "-T", "userspace/hello/link.ld", str(obj), "-o", str(apps / "file-fault.bh")],
        cwd=ROOT, check=True,
    )
    for index in (1, 2):
        source.write_text((ROOT / "tests/user/spin.S").read_text().replace(
            "0x123456789abcdef0", "0x123456789abcdef0" if index == 1 else "0x76543210fedcba98"))
        subprocess.run(["as", "--64", str(source), "-o", str(obj)], check=True)
        subprocess.run(["ld", "-m", "elf_x86_64", "-T", "userspace/hello/link.ld", str(obj),
                        "-o", str(apps / f"spin{index}.bh")], cwd=ROOT, check=True)
    subprocess.run(
        ["tar", "-rf", str(esp / "EFI/bOS/initrd.img"), "-C", str(work),
         "applications/bad-elf.bh", "applications/fault.bh", "applications/file-io.bh", "applications/file-fault.bh", "applications/spin1.bh", "applications/spin2.bh"], check=True,
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


def check_m0(shell):
    """Boot timing, the dmesg ring, and the crash-log reader."""
    bootstat = shell.command("bootstat", b"boot:")
    assert b"tsc:" in bootstat and b"dmesg:" in bootstat, bootstat
    uptime = shell.command("uptime", b"boot:")
    assert b"ticks:" in uptime, uptime

    log = shell.command("dmesg", b"=== bOS Kernel Starting ===")
    assert b"Physical memory manager initialized" in log, log[-2000:]

    cleared = shell.command("dmesg -c", b"=== bOS Kernel Starting ===")
    assert b"Physical memory manager initialized" in cleared
    shell.command("dmesg", b"kernel log is empty")
    shell.command("crashlog", b"no persisted kernel panic")


def check_network(esp):
    for nic, model in (
        ("e1000", b"Intel 82540EM"),
        ("e1000e", b"Intel 82574L"),
        ("rtl8139", b"Realtek RTL8139"),
    ):
        with boot(esp, nic=nic, label=f"nic-{nic}") as (shell, _):
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

    with boot(esp, nic="none", label="nic-none") as (shell, _):
        shell.command("ifconfig", b"No supported Ethernet adapter")
        shell.command("ping 10.0.2.2", b"no supported Ethernet adapter")


def check_preemption(shell):
    shell.keys(b"runall /applications/spin1.bh /applications/spin2.bh /applications/native-demo.bh /applications/fault.bh\r")
    # The first two programs never exit or yield. Reaching the third proves preemption.
    first = shell.expect(b"libbos native test passed")
    assert first.count(b"SPIN READY") == 2, first
    time.sleep(1)
    shell.keys(b"\x03")
    result = shell.expect(PROMPT)
    assert b"spin1.bh: status 130" in result, result
    assert b"spin2.bh: status 130" in result, result
    assert b"native-demo.bh: status 0" in result, result
    assert b"fault.bh: status -15" in result, result
    assert b"CONTEXT CORRUPTED" not in first + result
    shell.command("/applications/hello.bh", b"Hello from bOS user mode!")
    shell.command("memtest", b"memory self-test passed")


def check_babo(shell):
    shell.command("babo documents/editor-test.txt", prompt=b"^S Save")
    shell.keys(b"alpha\rbravo\x13")
    shell.expect(b"Saved 11 bytes")
    shell.keys(b"\x11")
    shell.expect(PROMPT)
    shell.command("cat documents/editor-test.txt", b"\r\nalpha\r\nbravo")
    shell.command("babo documents/editor-test.txt", prompt=b"^S Save")
    shell.keys(b"\x1b[3~A\x1a\x19\x17bravo\r")
    shell.expect(b"Match found")
    shell.keys(b"\x1b[F!\x13")
    shell.expect(b"Saved 12 bytes")
    shell.keys(b"\x11")
    shell.expect(PROMPT)
    shell.command("cat documents/editor-test.txt", b"\r\nAlpha\r\nbravo!")
    shell.command("babo documents/editor-test.txt", prompt=b"^S Save")
    shell.keys(b"discarded\x11")
    shell.expect(b"Unsaved changes")
    shell.keys(b"d\r")
    shell.expect(PROMPT)
    result = shell.command("cat documents/editor-test.txt", b"Alpha")
    assert b"discarded" not in result


def check_native_boot():
    prompt=b"bSH> "
    with boot(ROOT/"target/esp",recovery=False,prompt=prompt,label="native-pid1") as (shell,output):
        assert b"init: PID 1 in ring 3" in output, output
        shell.command("/applications/process-test.bh",b"PROCESS PIPE TEST PASSED",prompt=prompt)
        shell.command("/applications/native-demo.bh",b"libbos native test passed",prompt=prompt)
        shell.command("cat /users/default/documents/native.txt | wc -c",b"\r\n21\r\n",prompt=prompt)
        shell.command("exit",b"init: restarting shell",prompt=prompt)
        shell.command("echo restarted",b"\r\nrestarted\r\n",prompt=prompt)
        shell.keys(b"\x03")
        restarted=shell.expect(prompt)
        assert b"init: restarting shell" in restarted, restarted


def main():
    check_native_boot()
    with tempfile.TemporaryDirectory(prefix="bos-test-") as directory:
        work = Path(directory)
        esp = fixture_esp(work)
        disk = work / "users.img"
        subprocess.run([ROOT / "target/release/mkusers", disk], check=True)
        # Native fsync must persist data through a power cycle, on either transport.
        with boot(esp,disk,recovery=False,prompt=b"bSH> ",label="native-disk-write") as (shell,output):
            assert PERSISTENT in output
            shell.command("/applications/native-demo.bh",b"libbos native test passed",prompt=b"bSH> ")
        with boot(esp,disk,recovery=False,prompt=b"bSH> ",label="native-disk-read") as (shell,output):
            assert PERSISTENT in output
            shell.command("cat /users/default/documents/native.txt",b"native Rust file I/O",prompt=b"bSH> ")

        with boot(esp, label="boot-ram") as (shell, output):
            assert b"No data disk; /users is in RAM" in output
            assert b"\x1b[97mbOS kernel console (bSH) v" in shell.raw
            shell.command("memtest", b"memory self-test passed")
            check_m0(shell)
            shell.command("/applications/process-test.bh", b"PROCESS PIPE TEST PASSED")
            check_preemption(shell)
            check_babo(shell)
            shell.command("/applications/native-demo.bh", b"libbos native test passed")
            shell.command("cat documents/native.txt", b"native Rust file I/O")
            shell.command("lspci", b"8086:100e Ethernet")
            shell.command("calculator 2 + 3 * 4", b"\r\n14\r\n")
            assert b"\x1b[93m14\r\n" in shell.raw
            assert b"\x1b[92mbSH:/users/default# " in shell.raw
            shell.command("calculator 1 / 0", b"division by zero")
            assert b"\x1b[91mcalculator: division by zero" in shell.raw
            shell.command("echo plain > documents/colors.txt")
            shell.command("wc -c documents/colors.txt", b"6 ")

            shell.command(b"bfe\t", b"Shell: bSH")
            shell.command("echo history", b"\r\nhistory\r\n")
            shell.command(b"\x1b[A", b"\r\nhistory\r\n")
            shell.command(b"echo ac\x1b[Db", b"\r\nabc\r\n")
            shell.command("/applications/hello.bh", b"Hello from bOS user mode!")
            # Warm up the allocator and file once, then verify physical frames
            # are restored across repeated normal and faulting program teardown.
            shell.command("/applications/file-io.bh", b"FILE TEST PASSED")
            before = int(re.search(rb"frames-free: (\d+)", shell.command("mem")).group(1))
            for _ in range(20):
                result = shell.command("/applications/file-io.bh", b"FILE TEST PASSED")
                assert b"FILE TEST FAILED" not in result
                result = shell.command("/applications/file-fault.bh", b"user fault (vector 14)")
                assert b"FILE TEST PASSED" in result and b"FILE TEST FAILED" not in result
            after = int(re.search(rb"frames-free: (\d+)", shell.command("mem")).group(1))
            assert before == after, (before, after)
            shell.command("cat documents/user-io.txt", b"FILE-OK")
            shell.command("memtest", b"memory self-test passed")
            shell.command("run /applications/bad-elf.bh", b"expected a static x86-64 ELF")
            shell.command("/applications/fault.bh", b"user fault (vector 14)")
            shell.command("/applications/hello.bh", b"Hello from bOS user mode!")

        with boot(esp, disk, label="boot-persist-1") as (shell, output):
            assert PERSISTENT in output
            shell.command("crashlog", b"no persisted kernel panic")
            shell.command("babo documents/notes.txt", prompt=b"^S Save")
            shell.keys(b"saved\r\x13")
            shell.expect(b"Saved 6 bytes and synced to disk")
            shell.keys(b"\x11")
            shell.expect(PROMPT)
            shell.command("touch documents/remove-me")
            shell.command("echo renamed > documents/before")
            shell.command("mv documents/before downloads/after")
            shell.command("sync")
            shell.command("halt", prompt=b"halted.\r\n")

        with boot(esp, disk, label="boot-persist-2") as (shell, output):
            assert PERSISTENT in output
            shell.command("cat documents/notes.txt", b"\r\nsaved\r\n")
            shell.command("cat downloads/after", b"\r\nrenamed\r\n")
            shell.command("rm documents/remove-me")
            shell.command("halt", prompt=b"halted.\r\n")

        with boot(esp, disk, label="boot-persist-3") as (shell, _):
            shell.command("cat documents/remove-me", b"No such file")

        with disk.open("r+b") as image:
            image.write(b"BAD!")
        original = hashlib.sha256(disk.read_bytes()).digest()
        with boot(esp, disk, label="boot-bad-disk") as (shell, output):
            assert b"Data disk rejected; /users is in RAM" in output
            shell.command("halt", prompt=b"halted.\r\n")
        assert hashlib.sha256(disk.read_bytes()).digest() == original
        check_network(esp)

    print("Live boot, native Rust, preemption/FPU/Ctrl-C, memory, file ABI, bFS persistence and Ethernet passed")


if __name__ == "__main__":
    main()
