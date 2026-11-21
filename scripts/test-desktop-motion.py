#!/usr/bin/env python3
"""Record the real desktop while using a PS/2 mouse to move and minimize a window."""

import argparse
import importlib.util
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
from PIL import Image, ImageChops
from qemu_console import Console, ROOT

spec = importlib.util.spec_from_file_location("live", ROOT / "scripts/test-live.py")
live = importlib.util.module_from_spec(spec)
spec.loader.exec_module(live)


def run(output):
    with tempfile.TemporaryDirectory(
        dir="/dev/shm" if Path("/dev/shm").is_dir() else None
    ) as tmp:
        for ext in ("in", "out"):
            os.mkfifo(tmp + "/q." + ext)
        cmd = [
            "bash",
            "scripts/run-qemu.sh",
            "--live",
            "-display",
            "none",
            "-no-reboot",
            "-chardev",
            f"pipe,id=q,path={tmp}/q",
            "-mon",
            "chardev=q,mode=control",
        ]
        with (
            open(tmp + "/q.out", "r+b", buffering=0) as rd,
            open(tmp + "/q.in", "r+b", buffering=0) as wr,
            subprocess.Popen(
                cmd,
                cwd=ROOT,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
            ) as p,
        ):
            q = live.Monitor(rd, wr)
            c = Console(p)
            frames = []
            stamps = []

            def events(*ev):
                q.command("input-send-event", events=list(ev))

            def button(down):
                events({"type": "btn", "data": {"button": "left", "down": down}})

            def move(dx, dy):
                while dx or dy:
                    a = max(-32, min(32, dx))
                    b = max(-32, min(32, dy))
                    events(
                        {"type": "rel", "data": {"axis": "x", "value": a}},
                        {"type": "rel", "data": {"axis": "y", "value": b}},
                    )
                    dx -= a
                    dy -= b
                    time.sleep(0.012)

            def snap():
                path = Path(tmp) / "frame.ppm"
                q.command("screendump", filename=str(path))
                frames.append(path.read_bytes())
                stamps.append(time.monotonic())

            def capture(n):
                for _ in range(n):
                    snap()
                    time.sleep(0.009)

            try:
                c.expect(b"Desktop ready:", 120)
                c.expect(b"bOS 1.0")
                time.sleep(0.25)
                # Start on the browser title bar. Each PS/2 delta is one desktop pixel.
                snap()
                move(250, -134)
                capture(3)
                button(True)
                time.sleep(0.07)
                for _ in range(18):
                    move(-12, 3)
                    snap()
                button(False)
                capture(5)
                # The actual minimize button moves with the window.
                move(252, 0)
                begin = len(frames)
                snap()
                button(True)
                capture(2)
                button(False)
                capture(22)
                # Restore the browser from its dock icon at x=575, y=755.
                move(575 - (890 - 216 + 252), 755 - (266 + 54))
                button(True)
                time.sleep(0.035)
                button(False)
                capture(16)
                import io

                frames = [
                    Image.open(io.BytesIO(data)).resize(
                        (768, 480), Image.Resampling.LANCZOS
                    )
                    for data in frames
                ]
                diff = sum(
                    ImageChops.difference(
                        frames[i - 1].crop((220, 145, 680, 420)),
                        frames[i].crop((220, 145, 680, 420)),
                    ).getbbox()
                    is not None
                    for i in range(begin + 1, begin + 24)
                )
                q.key("f12")
                c.expect(b"All commands available")
                stats = re.findall(rb"Compositor: [^\r\n]+", c.raw)
                assert stats and b"motion=" in stats[-1], (
                    "missing guest compositor measurements"
                )
                print(stats[-1].decode())
                print(f"{diff} distinct minimize frames sampled from QEMU")
                if output:
                    output.parent.mkdir(parents=True, exist_ok=True)
                    durations = [
                        max(18, min(160, round((b - a) * 1000)))
                        for a, b in zip(stamps, stamps[1:])
                    ] + [500]
                    frames[0].save(
                        output,
                        save_all=True,
                        append_images=frames[1:],
                        duration=durations,
                        loop=0,
                        quality=85,
                        method=4,
                    )
                assert diff >= 3, f"only {diff} distinct minimize frames"
            finally:
                if p.poll() is None:
                    p.terminate()
                p.wait()
                c.raw.extend(p.stdout.read())
                (ROOT / "target/live/compositor.log").write_bytes(c.raw)


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--record", type=Path)
    run(ap.parse_args().record)
