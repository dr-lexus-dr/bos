# bOS

[English](#english) · [Русский](#русский) · [How to install / Установка](docs/INSTALL.md)

## English

**bOS is a small, terminal-first operating system for x86-64 UEFI.** Boot straight into bSH, explore the filesystem, edit a note, or mount a bFS volume. The Rust kernel runs without the standard library, and a fresh session starts in `/users/default` with three directories at the root: `/system`, `/users`, and `/applications`.

The current release is an experimental preview. The root filesystem and bFS volumes run in RAM, so changes reset on reboot. The Linux tools ship as separate programs.

### Build and run

On Linux, install Rust via rustup, GNU tar, QEMU x86-64, and OVMF. To build ISO images, also install Python 3 with `pycdlib`, `dosfstools`, and `mtools`. The Rust version and targets come from `rust-toolchain.toml`.

```sh
bash scripts/build.sh --release
bash scripts/run-qemu.sh
```

Set `OVMF_CODE` if the firmware is installed at a different path. For the ISO images, use the same build output:

```sh
python3 scripts/mkiso.py
python3 scripts/mkiso.py --install
```

The outputs are `target/bos-live.iso` and `target/bos-install.iso`. Boot the **Live ISO** as a UEFI CD-ROM. The **Install ISO** includes a Linux script for installing bOS onto a dedicated drive. See the [installation guide](docs/INSTALL.md).

### First commands

```text
help
whoami
pwd
ls /
cd documents
edit -a notes.txt
```

Type a line in the editor, then `.save` to write it or `.quit` to discard it. Run `cat notes.txt` to read the file. Use `cd` to return home.

## Русский

**bOS — компактная терминальная операционная система для x86-64 UEFI.** После загрузки открывается bSH, где можно работать с файлами, редактировать заметки и подключать тома bFS. Ядро написано на Rust без стандартной библиотеки. Сеанс начинается в `/users/default`, а в корне находятся три каталога.

```text
/
├── system/         журналы, временные файлы и точки монтирования
├── users/
│   └── default/    documents, downloads, pictures, music, videos
└── applications/   приложения
```

Это экспериментальный выпуск. Корневая файловая система и тома bFS работают в RAM. Изменения сбрасываются после перезагрузки. Утилиты для Linux собираются отдельно.

### Сборка и запуск

На Linux установите Rust через rustup, GNU tar, QEMU x86-64 и OVMF. Для сборки ISO также понадобятся Python 3 с пакетом `pycdlib`, `dosfstools` и `mtools`. Версия Rust и целевые платформы указаны в `rust-toolchain.toml`.

```sh
bash scripts/build.sh --release
bash scripts/run-qemu.sh
```

Если OVMF установлен по другому пути, задайте его через `OVMF_CODE`. Чтобы собрать оба образа:

```sh
python3 scripts/mkiso.py
python3 scripts/mkiso.py --install
```

Образы появятся в `target/bos-live.iso` и `target/bos-install.iso`. **Live ISO** загружается как UEFI CD-ROM. **Install ISO** содержит скрипт для установки на отдельный накопитель из Linux. См. [инструкцию по установке](docs/INSTALL.md).

В bSH попробуйте `help`, `whoami`, `pwd` и `ls /`. Для записи заметки откройте `edit -a documents/notes.txt`, введите текст и завершите командой `.save`. Команда `cd` возвращает в домашний каталог.

Подробнее об устройстве системы см. в [описании архитектуры](docs/ARCH.md).

## Инструменты для Linux

Следующие утилиты запускаются в Linux из каталога проекта. Текущий режим
загрузки bOS использует встроенную оболочку `bSH`. Запуск пользовательских
процессов и сетевой стек внутри bOS пока в разработке.

## Package manager — `bpm`

Installs applications from the official repository:
**https://github.com/bOS-operating-system/bos-repository**

```
system/bpm update                 # refresh the registry from GitHub
system/bpm list                   # list available packages
system/bpm search <query>         # search name/description/category
system/bpm info <name>            # package details
system/bpm install <name>         # install into applications/
system/bpm install owner/repo     # ...or directly from any GitHub repo
system/bpm remove <name>          # uninstall
system/bpm installed              # list installed packages
system/bpm upgrade [name...]      # update installed packages
```

The official repository exposes a JSON index at its root, `registry.json`:

```json
{
  "packages": [
    {
      "name": "calculator",
      "version": "1.0.0",
      "description": "Simple calculator",
      "category": "utilities",
      "repo": "bOS-operating-system/bos-apps",
      "ref": "main",
      "path": "packages/calculator",
      "sha256": "optional tarball checksum",
      "bin": ["calculator.sh"],
      "license": "MIT",
      "homepage": "https://example.com",
      "requires": []
    }
  ]
}
```

- `repo` — GitHub `owner/repo` (packages may live in the official repository
  itself or in any other repository)
- `ref` — branch, tag or commit (default `main`)
- `path` — directory inside the repository (default `.` = whole repository)
- `bin` — files to mark executable after install
- `requires` — package names installed automatically first

Installed packages land in `applications/<name>`, the install database lives in
`system/bpm/installed.json`, the registry cache in `system/bpm/cache/`, and all
operations are logged to `system/logs/bpm.log`.

Environment: `BOS_ROOT` (bOS root, auto-detected), `BPM_REGISTRY`
(registry.json URL override).

Build: `cargo +stable build -p bpm` (sources in `userspace/bpm/`).

## Text editor — `btext`

The standard bOS text editor:

```
system/text-editor <file>     # open or create a file
```

Line-oriented editing with numbered lines: `p` print, `e <n> <text>` replace,
`i <n> <text>` insert, `a <text>` append, `d <n> [m]` delete,
`s/old/new[/g]` substitute, `/text` search, `w` save, `q` quit.
Run `btext <file>` and type `h` for the full command list.

Build: `cargo build -p btext` (sources in `userspace/btext/`).

## Internet support — `bnet`

```
system/network check              # interfaces, DNS, HTTP/HTTPS connectivity check
system/network ping <host> [n]    # ping a host
system/network dns <hostname>     # resolve a hostname
system/network http <url>         # HTTP GET with headers and timing
system/network download <url> [file]
                                  # download a URL with progress
system/network interfaces         # list network interfaces
system/network port <host> <port> # check if a TCP port is open
system/network traceroute <host>  # trace route to host
```

The kernel side lives in `drivers/net` (e1000/rtl8139/virtio-net stubs);
ARP/IPv4/TCP and `SYS_NET_*` syscalls are next (see `docs/ARCH.md`).

Build: `cargo build -p bnet` (sources in `userspace/bnet/`).

## Сборка утилит для Linux

```sh
cargo +stable build -p bpm -p bnet -p btext
```

## Шрифт консоли

Встроенная таблица глифов сгенерирована из `Lat15-VGA16.psf.gz` пакета `console-setup-linux`. Скрипт генерации находится в [scripts/genfont.py](scripts/genfont.py).
