#!/usr/bin/env python3
"""Exercise the actual GUI, NIC, HTTP framing and TLS rejection on a read-only live disk."""

import argparse
import hashlib
import json
import os
import select
import ssl
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from qemu_console import ROOT, Console

OUT = ROOT / "target/live"
IMAGE = OUT / "bos-live.img"
FIRST = b"<html><head><title>Live desk test</title><style>body{margin:0}a{display:block;margin:12px 16px}</style></head><body><a href='next'>Follow this link</a><h1>Local server</h1></body></html>"
NEXT = b"<title>Second page</title><p>The guest followed the link.</p>"


def serve_http():
    request = sys.stdin.buffer.readline().split()
    while sys.stdin.buffer.readline() not in (b"", b"\r\n"):
        pass
    path = request[1] if len(request) > 1 else b""
    if path == b"/start":
        reply = b"HTTP/1.1 302 Found\r\nLocation: /articles/index\r\nContent-Length: 0\r\n\r\n"
    elif path == b"/articles/index":
        reply = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: text/html\r\n\r\n"
        for chunk in (FIRST[:27], FIRST[27:65], FIRST[65:]):
            reply += b"%x\r\n" % len(chunk) + chunk + b"\r\n"
        reply += b"0\r\n\r\n"
    elif path == b"/articles/next":
        reply = b"HTTP/1.1 200 OK\r\nContent-Length: %d\r\n\r\n" % len(NEXT) + NEXT
    elif path == b"/long":
        prefix = b"<title>Long page</title><p>Readable beginning.</p>"
        reply = b"HTTP/1.1 200 OK\r\nContent-Length: 65537\r\n\r\n" + prefix.ljust(
            65537
        )
    elif path == b"/limit":
        body = b"<title>Limit page</title>" + (b"<p>Repeated navigation</p>" * 3000)
        reply = b"HTTP/1.1 200 OK\r\nContent-Length: 65536\r\n\r\n" + body[:65536]
    else:
        reply = b"HTTP/1.0 404 Not Found\r\n\r\nmissing page"
    sys.stdout.buffer.write(reply)
    sys.stdout.buffer.flush()


def serve_tls(directory):
    # guestfwd supplies a separate stdin/stdout stream for each TCP connection.
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.load_cert_chain(Path(directory) / "cert", Path(directory) / "key")
    incoming, outgoing = ssl.MemoryBIO(), ssl.MemoryBIO()
    conn = ctx.wrap_bio(incoming, outgoing, server_side=True)
    while True:
        try:
            conn.do_handshake()
            return
        except ssl.SSLWantReadError:
            sys.stdout.buffer.write(outgoing.read())
            sys.stdout.buffer.flush()
            data = os.read(0, 16384)
            if not data:
                return
            incoming.write(data)
        except ssl.SSLError:
            return


class Monitor:
    def __init__(self, reader, writer):
        self.reader = reader
        self.writer = writer
        assert "QMP" in self.read()
        self.command("qmp_capabilities")

    def read(self):
        if not select.select([self.reader], [], [], 10)[0]:
            raise TimeoutError("QEMU monitor did not answer")
        return json.loads(self.reader.readline())

    def command(self, name, **args):
        self.writer.write(
            json.dumps({"execute": name, "arguments": args}).encode() + b"\n"
        )
        while True:
            result = self.read()
            if "return" in result:
                return result["return"]
            if "error" in result:
                raise AssertionError(result)

    def key(self, *keys):
        self.command(
            "send-key",
            keys=[{"type": "qcode", "data": key} for key in keys],
            **{"hold-time": 25},
        )
        time.sleep(0.12)

    def move(self, dx, dy):
        for axis, delta in (("x", dx), ("y", dy)):
            while delta:
                n = max(-32, min(32, delta))
                self.command(
                    "input-send-event",
                    events=[{"type": "rel", "data": {"axis": axis, "value": n}}],
                )
                delta -= n
                time.sleep(0.016)

    def click(self, dx, dy):
        self.move(dx, dy)
        time.sleep(0.3)
        for pressed in (True, False):
            self.command(
                "input-send-event",
                events=[{"type": "btn", "data": {"button": "left", "down": pressed}}],
            )
            time.sleep(0.35)


def test(args, temp):
    capture = OUT / (("uefi" if args.uefi else "bios") + "-" + args.nic)
    capture.mkdir(parents=True, exist_ok=True)
    image = args.image.resolve()
    initial = hashlib.sha256(image.read_bytes()).digest()
    assert image.open("rb").read(512)[510:] == b"\x55\xaa"
    subprocess.run(
        [
            "openssl",
            "req",
            "-x509",
            "-newkey",
            "ec",
            "-pkeyopt",
            "ec_paramgen_curve:P-256",
            "-nodes",
            "-keyout",
            temp + "/key",
            "-out",
            temp + "/cert",
            "-days",
            "2",
            "-subj",
            "/CN=10.0.2.100",
            "-addext",
            "subjectAltName=IP:10.0.2.100",
        ],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    for suffix in ("in", "out"):
        os.mkfifo(f"{temp}/qmp.{suffix}")
    net = "user,id=lan" + ("" if args.public else ",restrict=on")
    # Relative script path keeps the guestfwd helper usable in checkouts with spaces.
    net += ",guestfwd=tcp:10.0.2.100:80-cmd:python3 scripts/test-live.py --serve-http"
    net += (
        ",guestfwd=tcp:10.0.2.100:443-cmd:python3 scripts/test-live.py --serve-tls "
        + temp
    )
    memory = 1024
    accel = os.environ.get(
        "BOS_ACCEL", "kvm" if os.access("/dev/kvm", os.R_OK | os.W_OK) else "tcg"
    )
    cmd = [
        "qemu-system-x86_64",
        "-machine",
        "pc",
        "-cpu",
        "host" if accel == "kvm" else "max,vendor=GenuineIntel",
        "-accel",
        accel,
        "-m",
        f"{memory}M",
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
        f"pipe,id=qmp,path={temp}/qmp",
        "-mon",
        "chardev=qmp,mode=control",
        "-drive",
        f"if=none,id=stick,file={image},format=raw,readonly=on",
        "-device",
        "qemu-xhci",
        "-device",
        "usb-storage,drive=stick,bootindex=1",
        "-netdev",
        net,
        "-device",
        args.nic + ",netdev=lan,romfile=",
    ]
    if args.uefi:
        code = os.environ.get("OVMF_CODE", "/usr/share/OVMF/OVMF_CODE_4M.fd")
        cmd.extend(["-drive", f"if=pflash,format=raw,readonly=on,file={code}"])
    with (
        open(temp + "/qmp.out", "r+b", buffering=0) as reader,
        open(temp + "/qmp.in", "r+b", buffering=0) as writer,
        subprocess.Popen(
            cmd,
            cwd=ROOT,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
        ) as proc,
    ):
        console = Console(proc)
        try:
            qmp = Monitor(reader, writer)
            assert (
                qmp.command("query-memory-size-summary")["base-memory"]
                == memory * 1024**2
            )
            console.expect(b"Desktop ready: ", 100)
            dimensions = console.expect(b", PS/2 mouse true", 5)
            width, height = map(int, dimensions.split(b"x"))

            console.expect(b"language=en", 5)

            def shot(name):
                time.sleep(0.65)
                qmp.command("screendump", filename=str(capture / (name + ".ppm")))

            def launch(name):
                console.keys(b"\x0f")
                console.expect(b"Desktop overview: true")
                console.keys(name.encode() + b"\r")
                console.expect(b"Desktop app: " + name.encode())

            shot("desktop-en")
            qmp.key("meta_l")
            console.expect(b"Desktop overview: true")
            console.expect(b"Desktop frame: overview en")
            shot("overview-en")
            console.keys(b"\x0f")
            console.expect(b"Desktop overview: false")
            launch("Settings")
            console.keys(b"2")
            console.expect(b"language=ru")
            console.expect(b"Desktop frame: windows ru")
            shot("settings-ru")
            qmp.key("alt", "f4")
            console.expect(b"Desktop frame: windows ru 2")
            shot("desktop-ru")
            for app in (
                "Notes",
                "Calculator",
                "Widgets",
                "System Monitor",
                "Snake",
                "About",
                "Terminal",
            ):
                launch(app)
                if app == "Notes":
                    console.keys("Привет, bOS!".encode() + b"\x13")
                elif app == "Calculator":
                    console.keys(b"6*7\r")
                    console.expect(b"Calculator result: 42")
                elif app == "Terminal":
                    console.keys(b"pwd\r")
                    console.expect(b"Terminal command: pwd")

            # Back to the existing browser. Search accepts either language.
            def open_url(url, expected):
                console.keys(b"\x0c" + url.encode() + b"\r")
                console.expect(expected, 240)

            open_url(
                "http://10.0.2.100/start",
                b"200 http://10.0.2.100/articles/index  Live desk test",
            )
            time.sleep(0.25)
            # Physical PS/2 click on row 2, starting at the centre of the display.
            link_x = width * 53 // 100 + 45
            link_y = height * 31 // 100 + 77 + 21
            qmp.click(link_x - width // 2, link_y - height // 2)
            console.expect(b"200 http://10.0.2.100/articles/next  Second page", 90)
            console.keys(b"\x02")  # Ctrl+B
            console.expect(b"Live desk test")
            qmp.key("tab")
            qmp.key("ret")
            console.expect(b"Second page")
            open_url("http://10.0.2.100/long", b"200 http://10.0.2.100/long  Long page")
            open_url("https://10.0.2.100/", b"TLS certificate verification failed")
            open_url("http://10.0.2.100/limit", b"Limit page")
            open_url("http://10.0.2.100/start", b"Live desk test")
            if args.public:
                open_url(args.public, b"Browser: 200 https://")
            time.sleep(0.25)
            qmp.command("screendump", filename=str(capture / "desktop.ppm"))
            shot = (capture / "desktop.ppm").read_bytes()
            assert (
                shot.startswith(f"P6\n{width} {height}\n255\n".encode())
                and len(shot) >= width * height * 3
            )
            qmp.key("f12")
            console.expect(b"All commands available")
            console.expect(b"% ")
            console.command(
                "cat documents/Note.txt", "Привет, bOS!".encode(), prompt=b"% "
            )
            console.command("hello release", b"argv[1] = release", prompt=b"% ")
            console.command("primes 12", b"2 3 5 7 11", prompt=b"% ")
            console.command(
                "runall /bin/counter /bin/spin", b"/bin/spin: status 0", prompt=b"% "
            )
            console.command("crash", b"user fault (vector 6)", prompt=b"% ")
            console.command("hello", b"Hello from user space", prompt=b"% ")
            console.command("counter 120 &", b"[100]", prompt=b"% ")
            console.command("ps", b"counter", prompt=b"% ")
            console.command("kill 100", prompt=b"% ")
            console.command("joblog 100", b"exit 130", prompt=b"% ")
            console.command("ifconfig", b"(DHCP)", prompt=b"% ")
            console.command(
                "web http://10.0.2.100/articles/next",
                b"The guest followed the link.",
                prompt=b"% ",
            )
            prefix = console.command(
                "web http://10.0.2.100/long", b"[display limit reached]", prompt=b"% "
            )
            assert b"Readable beginning." in prefix
            response = console.command(
                "mem", b"Live kernel + allocated frames, peak ", prompt=b"% "
            )
            peak = int(response.split(b"peak ")[-1].split()[0])
            assert 0 < peak < memory * 1024**2
            console.keys(b"gui\r")
            console.expect(b"Desktop ready")
        finally:
            if sys.exc_info()[0] and "qmp" in locals() and proc.poll() is None:
                qmp.command("screendump", filename=str(capture / "failure.ppm"))
            proc.terminate()
            proc.wait(timeout=10)
            console.raw.extend(proc.stdout.read())
            (capture / "smoke.log").write_bytes(console.raw)
    assert hashlib.sha256(image.read_bytes()).digest() == initial
    print(
        f"PASS: {image.stat().st_size} image bytes, {memory} MiB guest, {peak} kernel + peak allocated bytes, {args.nic}, read-only USB, GUI/mouse/links, HTTP and TLS"
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--image", type=Path, default=IMAGE, help="raw live disk to boot"
    )
    parser.add_argument(
        "--public",
        nargs="?",
        const="https://www.rfc-editor.org/rfc/rfc9110.txt",
        metavar="HTTPS_URL",
        help="also open a public HTTPS document (defaults to RFC 9110)",
    )
    parser.add_argument(
        "--uefi",
        action="store_true",
        help="boot the same disk with OVMF, with RAM for the firmware",
    )
    parser.add_argument(
        "--nic", choices=["rtl8139", "e1000", "e1000e"], default="rtl8139"
    )
    parser.add_argument("--serve-http", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--serve-tls", metavar="DIR", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.public and not args.public.lower().startswith("https://"):
        parser.error("--public requires an HTTPS URL")
    if args.serve_http:
        serve_http()
    elif args.serve_tls:
        serve_tls(args.serve_tls)
    else:
        with tempfile.TemporaryDirectory(prefix="bos-live-") as temp:
            test(args, temp)
