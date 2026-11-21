# Changelog

All notable changes to bOS are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions use
semantic versioning for the kernel and bootloader.

## [1.0.0](https://github.com/dr-lexus-dr/bos/releases/tag/v1.0.0) — 2026-11-21

- Shipped an English-first native desktop with Russian translation, Papirus application icons, Inter typography, an application overview, a dock, a calendar and persistent appearance settings.
- Added window open, close, minimize and restore animations. Dragging and resizing reuse cached pixels and update the affected framebuffer rectangles.
- Added an in-guest browser using go-webengine, Goja and Wasmi. It lays out HTML/CSS, loads images and scripts, presents progressively and accepts links, keyboard input and form submissions over HTTP and verified HTTPS.
- Added ten native applications and an ANSI-color terminal with independent sessions, history search, completion, scrollback and access to full-screen bSH.
- Added the BIOS/UEFI live USB disk, FAT12/FAT16 data volumes, legacy ATA support, i386 ring-3 compatibility, seven example programs and preempted background jobs.
- Added guest checks for both languages, desktop motion, CSS/JS pixels, browser input, form POST, TLS, program fault isolation and persistent FAT profiles. Added a separate full UEFI userspace profile for nash and uutils.

## [0.6.0] — 2026-11-14

M0: honest diagnostics, a repeatable QEMU test harness, and CI.

### Added

- `dmesg` with a 64 KiB kernel log ring and `dmesg -c`.
- `bootstat` and `uptime` with TSC/PIT boot-time calibration.
- `crashlog` for reading and clearing a persisted kernel panic, plus a
  best-effort panic write to `/users/.crash.log`.
- A panic recursion guard so a fault during crash-log persistence cannot loop.
- Shared QEMU serial harness `scripts/qemu_console.py` with per-boot raw logs
  under `target/qemu-logs` (override with `BOS_TEST_LOG_DIR`).
- RAM matrix test for 512M, 1G, and 4G guests (`scripts/test-boot-matrix.py`).
- Host test runner `scripts/test-host.sh` covering 15 workspace packages.
- GitHub Actions workflow for host tests, release build, QEMU smoke tests,
  RAM matrix, serial-log artifacts, and the staged ESP.
- `docs/STATUS.md` with the verified/not-verified capability matrix.
- `docs/ROADMAP.md` with the path to 75/100.
- `scripts/release.sh` to produce versioned Live/Install ISOs, an ESP archive,
  and `SHA256SUMS` under `target/release-<version>/`.

### Changed

- `scripts/test-qemu.py` now uses the shared harness and checks `dmesg`,
  `bootstat`, `uptime`, and `crashlog` in addition to boot, bFS, ELF, editor,
  and Ethernet coverage.
- `scripts/run-qemu.sh` accepts `BOS_MEMORY`/`BOS_NIC` values and normalizes
  sizes such as `4G`.
- Version bumped from 0.5.0 to 0.6.0 for the kernel and bootloader.

### Fixed

- The physical-frame bitmap is now 2 MiB and covers 64 GiB. The previous
  16 GiB limit panicked while booting a 32 GiB QEMU guest.

### Verified

- Release build: `bash scripts/build.sh --release`.
- Host tests: 20 tests across 15 packages.
- QEMU smoke: live boot, line editing, ring-3 ELF, bFS persistence, M0
  diagnostics, and e1000/e1000e/rtl8139 Ethernet.
- QEMU RAM matrix: 512M, 1G, and 4G.

### Not included

E7270 internal NVMe/SATA installation, USB HID, ACPI shutdown, a userspace
PID 1, multi-process scheduling, Wi-Fi, a browser/TLS, GPU acceleration,
Secure Boot signing, and POSIX compatibility remain future milestones.
