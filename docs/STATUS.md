# bOS 1.0

The primary distribution is the BIOS/UEFI live disk. It opens the native desktop in English. Russian is available in Settings. The two languages, ten desktop applications, browser, optional FAT volumes and i386 environment are documented in the release README. [English](../README.md) · [Русский](../README.ru.md)

## Configurations and checks

| Configuration | Reproducible check |
| --- | --- |
| SeaBIOS, read-only USB, RTL8139 and Intel 82574L | `python3 scripts/test-live.py` and `python3 scripts/test-live.py --nic e1000e` |
| OVMF, read-only USB and Intel 82540EM | `python3 scripts/test-live.py --uefi --nic e1000` |
| DNS and public HTTPS with certificate and hostname validation | `python3 scripts/test-live.py --public https://example.com/` |
| FAT12 via VirtIO and FAT16 via legacy IDE, two boots each | `python3 scripts/test-live-storage.py` |
| In-guest HTML/CSS/JS rendering, form POST and interactive navigation | `python3 scripts/test-browser.py --uefi` |
| Window motion, PS/2 dragging, sampled changes and frame timings | `python3 scripts/test-desktop-motion.py` |
| Host-side memory, IPC, editor and text reader tests | `bash scripts/test-host.sh` |

The full UEFI development image includes a larger initrd. Its normal boot opens the desktop. Explicit initrd markers in `/system` select `bsh` for the full-screen shell, `recovery` for the recovery prompt, `native` for the ring-3 bOS shell, or `nash` for the static Linux userspace. [Architecture](ARCH.md) · [User ABI](abi.md)

Test logs and guest screenshots are written under `target/live` and `target/qemu-logs`. The resource measurements include guest RAM and the compositor timing method. [Measurements](../CONSTRAINTS.md)
