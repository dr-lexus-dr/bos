#!/usr/bin/env python3
"""Exercise bOS's in-guest renderer through its real window, network and keyboard."""

import argparse
import importlib.util
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from PIL import Image
from qemu_console import Console, ROOT

spec = importlib.util.spec_from_file_location("live", ROOT / "scripts/test-live.py")
live = importlib.util.module_from_spec(spec)
spec.loader.exec_module(live)
BASE = "http://10.0.2.100"
PAGE = b"""<!doctype html><html><head><title>Browser integration</title><link rel="stylesheet" href="/site.css"></head><body><div id="panel"></div><button onclick="document.getElementById('panel').style.backgroundColor='#33845a'">Change color</button><form action="/submit" method="post"><input name="q"><input type="checkbox" name="checked"><button>Send</button></form><div id="long"></div><script src="/site.js"></script></body></html>"""


def serve():
    first = sys.stdin.buffer.readline().split()
    headers = {}
    for line in sys.stdin.buffer:
        if line in (b"\r\n", b"\n"):
            break
        k, _, v = line.partition(b":")
        headers[k.lower()] = v.strip()
    body = sys.stdin.buffer.read(int(headers.get(b"content-length", 0)))
    path = first[1] if len(first) > 1 else b""
    routes = {
        b"/index": ("text/html", PAGE),
        b"/site.css": (
            "text/css",
            b"body{margin:0}#panel{width:160px;height:80px;background:#123456}#long{height:2200px;background:#bc6a29}",
        ),
        b"/site.js": (
            "text/javascript",
            b"document.getElementById('panel').style.backgroundColor='#a84275'",
        ),
    }
    if path == b"/site.js":
        time.sleep(5)
    status = b"200 OK"
    if path == b"/submit" and first[0] == b"POST" and body == b"checked=on&q=bOS":
        typ, data = (
            "text/html",
            b"<title>Form submitted</title><h1>The guest posted bOS</h1>",
        )
    elif path in routes:
        typ, data = routes[path]
    else:
        status = b"404 Not Found"
        typ, data = "text/plain", b"Not found"
    sys.stdout.buffer.write(
        b"HTTP/1.1 "
        + status
        + b"\r\nContent-Type: "
        + typ.encode()
        + b"\r\nContent-Length: "
        + str(len(data)).encode()
        + b"\r\nConnection: close\r\n\r\n"
        + data
    )
    sys.stdout.buffer.flush()


def test(args):
    (ROOT / "target/live").mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        for ext in ("in", "out"):
            os.mkfifo(tmp + "/q." + ext)
        accel = os.environ.get(
            "BOS_ACCEL", "kvm" if os.access("/dev/kvm", os.R_OK | os.W_OK) else "tcg"
        )
        net = "user,id=n,guestfwd=tcp:10.0.2.100:80-cmd:python3 scripts/test-browser.py --serve"
        cmd = [
            "qemu-system-x86_64",
            "-accel",
            accel,
            "-machine",
            "pc",
            "-cpu",
            "host" if accel == "kvm" else "max,vendor=GenuineIntel",
            "-m",
            "1G",
            "-smp",
            "1",
            "-device",
            "VGA,vgamem_mb=8",
            "-display",
            "none",
            "-serial",
            "stdio",
            "-no-reboot",
            "-chardev",
            f"pipe,id=q,path={tmp}/q",
            "-mon",
            "chardev=q,mode=control",
            "-drive",
            f"if=none,id=stick,format=raw,readonly=on,file={args.image.resolve()}",
            "-device",
            "qemu-xhci",
            "-device",
            "usb-storage,drive=stick,bootindex=1",
            "-netdev",
            net,
            "-device",
            args.nic + ",netdev=n,romfile=",
        ]
        if args.uefi:
            cmd += [
                "-drive",
                "if=pflash,format=raw,readonly=on,file="
                + os.environ.get("OVMF_CODE", "/usr/share/OVMF/OVMF_CODE_4M.fd"),
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
            c = Console(p)
            q = live.Monitor(rd, wr)

            def shot(name):
                path = ROOT / "target/live" / (name + ".ppm")
                q.command("screendump", filename=str(path))
                im = Image.open(path)
                im.save(path.with_suffix(".png"))
                return im

            try:
                c.expect(b"Desktop ready:", 130)
                print("Desktop booted", flush=True)
                c.keys(b"\x0c" + BASE.encode() + b"/index\r")
                c.expect(b"Web engine: decoding", 120)
                print("Loading engine", flush=True)
                c.expect(b"executable ready", 300)
                print("Engine running", flush=True)
                c.expect(b"Browser first paint:", 300)
                c.keys(b"\x0f")
                c.expect(b"Desktop overview: true", 5)
                c.keys(b"\x0f")
                c.expect(b"Desktop overview: false", 5)
                print(
                    "Opened and closed the overview during the script transfer",
                    flush=True,
                )
                c.expect(
                    b"Browser: 200 " + BASE.encode() + b"/index  Browser integration",
                    300,
                )
                print("Rendered page", flush=True)
                time.sleep(0.6)
                im = shot("browser-form")
                x = im.width * 53 // 100 + 1
                y = im.height * 31 // 100 + 77
                assert im.getpixel((x + 10, y + 10))[:3] == (168, 66, 117), (
                    "script did not paint the CSS box"
                )
                assert im.getpixel((x + 180, y + 10))[:3] == (255, 255, 255), (
                    "box width did not follow external CSS"
                )

                def await_color(color):
                    path = Path(tmp) / "viewport.ppm"
                    deadline = time.monotonic() + 30
                    while time.monotonic() < deadline:
                        q.command("screendump", filename=str(path))
                        with Image.open(path) as snapshot:
                            if snapshot.getpixel((x + 10, y + 10))[:3] == color:
                                return
                        time.sleep(0.15)
                    raise AssertionError(f"Viewport did not paint {color}")

                q.move(x + 25 - im.width // 2, y + 16 - im.height // 2)
                for button, color in (
                    ("wheel-down", (188, 106, 41)),
                    ("wheel-up", (168, 66, 117)),
                ):
                    for _ in range(3):
                        for pressed in (True, False):
                            q.command(
                                "input-send-event",
                                events=[
                                    {
                                        "type": "btn",
                                        "data": {"button": button, "down": pressed},
                                    }
                                ],
                            )
                        time.sleep(0.06)
                    await_color(color)
                q.key("pgdn")
                await_color((188, 106, 41))
                q.key("pgup")
                await_color((168, 66, 117))
                q.key("tab")
                q.key("ret")
                await_color((51, 132, 90))
                q.key("tab")
                c.keys(b"bOS")
                q.key("tab")
                q.key("spc")
                q.key("tab")
                q.key("ret")
                c.expect(b"/submit  Form submitted", 180)
                print("POST form submitted by guest", flush=True)
                if args.public:
                    c.keys(b"\x0c" + args.public.encode() + b"\r")
                    c.expect(b"Browser: 200 https://", 240)
                    time.sleep(0.5)
                    shot("browser-web")
                    print("Verified public HTTPS page", flush=True)
                q.key("f12")
                c.expect(b"All commands available")
                c.expect(b"% ")
                print(
                    c.command("mem", b"peak ", prompt=b"% ")
                    .decode(errors="replace")
                    .strip(),
                    flush=True,
                )
            finally:
                if p.poll() is None:
                    shot("browser-last")
                    p.terminate()
                p.wait()
                c.raw.extend(p.stdout.read())
                (
                    ROOT
                    / "target/live"
                    / ("browser-test-" + ("uefi" if args.uefi else "bios") + ".log")
                ).write_bytes(c.raw)


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--image",
        type=Path,
        default=ROOT / "target/live/bos-live.img",
        help="raw live disk to boot",
    )
    ap.add_argument("--serve", action="store_true")
    ap.add_argument("--uefi", action="store_true")
    ap.add_argument("--public")
    ap.add_argument("--nic", default="rtl8139")
    args = ap.parse_args()
    if args.serve:
        serve()
    else:
        test(args)
