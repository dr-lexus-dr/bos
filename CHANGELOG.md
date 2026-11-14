# Changelog

All notable changes to bOS are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions use
semantic versioning for the kernel and bootloader.

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
