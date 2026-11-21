# bOS Live resource measurements

The release disk contains both firmware loaders, the Rust kernel, ten desktop applications, fonts, artwork, certificate roots and the full WebAssembly browser executable. The engine is compressed on the boot medium and opened on the first navigation. The disk is about **17.5 MB**. The exact byte count and SHA-256 digest accompany the versioned download. [Release assets](https://github.com/dr-lexus-dr/bOS/releases/tag/v1.0.0)

## Memory

The launcher gives QEMU **1 GiB guest RAM** and an **8 MiB VGA adapter**. Guest measurements below use one x86-64 vCPU and a 1280 × 800 desktop. The shell's `mem` command reports the resident kernel (including BSS and its stack) plus the high-water mark of allocated physical frames. The reserved compressed browser executable, firmware reservations, adapter VRAM and the host emulator's memory are outside that counter. Memory depends on the document and the number of browser windows.

| Workload | Observed kernel and allocated-frame peak |
| --- | ---: |
| BIOS startup, Files and the browser start page, before opening a URL | about 23 MiB |
| Browsing a scripted form and public HTTPS pages | about 390 MiB (QEMU/OVMF, form and example.com) |

The native desktop keeps a wallpaper cache and a framebuffer-sized drawing surface. Moving a window reuses its captured pixels and backdrop and updates the affected rectangles. The browser keeps its layout tree and paints the visible viewport. Pages share a decoded engine module. Each browser window has a separate WebAssembly instance, DOM and cookie jar.

## Reproduce

```sh
bash scripts/build.sh --live
python3 scripts/test-desktop-motion.py --record target/live/motion.webp
python3 scripts/test-browser.py --uefi --public https://example.com/
python3 scripts/test-live.py --nic e1000e
python3 scripts/test-live-storage.py
```

The motion test uses actual PS/2 movement and framebuffer captures. The serial log reports the median and 95th percentile paint times over the latest 32 moving frames, plus the input-to-present 95th percentile from the desktop input queue. These counters measure the guest compositor. One KVM recording measured 1 ms median and 7 ms p95 for moving frames, and 19 ms p95 from queued input to presentation. Host display latency, panel scanout and firmware time are outside the interval.

The network tests use the guest Ethernet adapter. QEMU guest forwarding serves deterministic HTML, CSS, JavaScript and form endpoints. Public HTTPS uses ordinary NAT, DNS and certificate validation. The test suite also boots through OVMF, rejects a local untrusted certificate and verifies that the live boot medium stays byte-identical. Physical hardware coverage depends on the framebuffer, PS/2 input and compatible wired NICs. [Test matrix](docs/STATUS.md)
