#!/usr/bin/env python3
"""Shared QEMU serial-console harness for bOS smoke tests.

Every boot writes the raw serial stream to `target/qemu-logs` (override with
`BOS_TEST_LOG_DIR`).  That turns a failed CI run into a downloadable artifact
instead of an unreadable traceback.
"""
import os
import select
import subprocess
import time
from contextlib import contextmanager
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PROMPT = b"bSH:/users/default# "
LOG_DIR = Path(os.environ.get("BOS_TEST_LOG_DIR", ROOT / "target/qemu-logs"))
_boot_counter = 0


class Console:
    def __init__(self, process):
        self.process = process
        self.output = bytearray()
        self.raw = bytearray()
        self.escape = 0

    def expect(self, token, timeout=45):
        deadline = time.monotonic() + timeout
        while token not in self.output:
            if not select.select([self.process.stdout], [], [], max(0, deadline - time.monotonic()))[0]:
                raise AssertionError(f"Timed out waiting for {token!r}\n{self.output[-3000:]!r}")
            data = os.read(self.process.stdout.fileno(), 8192)
            if not data:
                raise AssertionError(f"QEMU exited waiting for {token!r}\n{self.output[-3000:]!r}")
            self.raw.extend(data)
            # ANSI sequences can arrive across reads. Keep assertions about
            # command text independent of the console's color scheme.
            for byte in data:
                if byte == 0x1b:
                    self.escape = 1
                elif self.escape == 1:
                    self.escape = 2 if byte == ord("[") else 0
                elif self.escape == 2:
                    if 0x40 <= byte <= 0x7e:
                        self.escape = 0
                else:
                    self.output.append(byte)
        offset = self.output.index(token)
        response = bytes(self.output[:offset])
        del self.output[:offset + len(token)]
        return response

    def keys(self, data):
        self.process.stdin.write(data)
        self.process.stdin.flush()

    def command(self, line, expected=b"", prompt=PROMPT, timeout=45):
        data = line.encode() if isinstance(line, str) else line
        self.process.stdin.write(data + b"\r")
        self.process.stdin.flush()
        response = self.expect(prompt, timeout=timeout)
        if expected not in response:
            raise AssertionError(f"{line!r}: missing {expected!r} in {response!r}")
        return response


@contextmanager
def boot(esp, disk=None, *, nic="e1000", memory=None, label=None, log=True):
    global _boot_counter
    _boot_counter += 1
    name = label or f"boot-{_boot_counter:02d}"
    command = ["bash", "scripts/run-qemu.sh", str(esp)]
    if disk is not None:
        command.append(str(disk))
    environment = {
        **os.environ,
        "BOS_MEMORY": memory or os.environ.get("BOS_TEST_MEMORY", "512M"),
        "BOS_NIC": nic,
    }
    start = time.monotonic()
    with subprocess.Popen(
        command, cwd=ROOT, env=environment,
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
    ) as process:
        console = Console(process)
        try:
            output = console.expect(PROMPT)
            print(f"{name}: bSH ready after {time.monotonic() - start:.1f}s", flush=True)
            yield console, output
        finally:
            if process.poll() is None:
                process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
            if log:
                LOG_DIR.mkdir(parents=True, exist_ok=True)
                (LOG_DIR / f"{name}.log").write_bytes(console.raw)
