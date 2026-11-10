# bOS

**bSH · bFS · bPT**

*Boot to a shell. Work with files. Manage GitHub packages with bPT.*

*Загрузитесь в оболочку. Работайте с файлами. Управляйте пакетами GitHub через bPT.*

[English](#english) · [Русский](#русский) · [How to install / Установка](docs/INSTALL.md)

## English

**bOS is a small, terminal-first operating system for x86-64 UEFI.** Boot straight into bSH, explore the filesystem, edit a note, or mount a bFS volume. The Rust kernel runs without the standard library, and a fresh session starts in `/users/default` with three directories at the root: `/system`, `/users`, and `/applications`.

bOS 0.2.0 can save `/users` to a dedicated bFS disk in QEMU and run a static ELF64 program in user mode. Live sessions without a data disk use RAM. The Linux tools ship as separate programs.

bOS 0.3.0 adds ANSI foreground and background colors to the framebuffer console. Use `lspci` to see the PCI IDs of devices detected by bOS.

bOS 0.4.0 adds DHCPv4 and `ping` on the Intel 82540EM. bOS 0.5.0 adds Intel 82574L, Realtek RTL8139 and manual IPv4 configuration.

### Ethernet in bSH

```text
lspci
ifconfig
ping 10.0.2.2
ifconfig static 10.0.2.15/24 10.0.2.2
ifconfig dhcp
```

The example uses addresses from QEMU's default virtual network. bOS selects one supported Ethernet controller as `eth0` and starts DHCP at boot. Manual settings apply to the current session. Use `BOS_NIC=e1000e` or `BOS_NIC=rtl8139` with `scripts/run-qemu.sh` to try another model.

| Controller | PCI ID | Verified in |
| --- | --- | --- |
| Intel 82540EM | `8086:100e` | QEMU `e1000` |
| Intel 82574L | `8086:10d3` | QEMU `e1000e` |
| Realtek RTL8139 | `10ec:8139` | QEMU `rtl8139` |

Physical cards still need validation. RTL8168/8111 and Wi-Fi require separate drivers.

### Build and run

On Linux, install Rust via rustup, GNU tar, GNU binutils, QEMU x86-64, and OVMF. To build ISO images, also install Python 3 with `pycdlib`, `dosfstools`, and `mtools`. The Rust version and targets come from `rust-toolchain.toml`.

```sh
bash scripts/build.sh --release
bash scripts/run-qemu.sh
```

The launcher defaults to 4 GiB and `-cpu max`. Set `BOS_MEMORY` (for example, `512M`) to change RAM or `OVMF_CODE` if the firmware is installed at a different path. For the ISO images, use the same build output:

```sh
python3 scripts/mkiso.py
python3 scripts/mkiso.py --install
```

The outputs are `target/bos-live.iso` and `target/bos-install.iso`. Boot the **Live ISO** as a UEFI CD-ROM. The **Install ISO** includes a Linux script for installing bOS onto a dedicated drive. See the [installation guide](docs/INSTALL.md).

### Save files between reboots

```sh
target/release/mkusers target/users.img
bash scripts/run-qemu.sh target/esp target/users.img
```

Create the disk once and reuse it on subsequent boots. Run `sync` before closing QEMU. The bSH commands `reboot` and `halt` also flush pending writes. The preview supports this setup with one legacy VirtIO block disk in QEMU. The bFS volume is buffered in RAM and has no journal.

### First commands

```text
help
whoami
pwd
bitstat
bfetch
calculator 2 + 3 * 4
/applications/hello
ls /
cd documents
notepad
```

Type a line in the editor, then `.save` to write it or `.quit` to discard it. Run `cat notes.txt` to read the file. Use `cd` to return home. Up and Down browse command history, Left and Right move the cursor, and Tab completes commands and paths.

## Русский

**bOS — компактная терминальная операционная система для x86-64 UEFI.** После загрузки открывается bSH, где можно работать с файлами, редактировать заметки и подключать тома bFS. Ядро написано на Rust без стандартной библиотеки. Сеанс начинается в `/users/default`, а в корне находятся три каталога.

```text
/
├── system/         журналы, временные файлы и точки монтирования
├── users/
│   └── default/    documents, downloads, pictures, music, videos
└── applications/   приложения
```

В bOS 0.2.0 каталог `/users` можно хранить на отдельном диске bFS в QEMU. Команда `/applications/hello` запускает ELF64 в пользовательском режиме. Без диска данных изменения хранятся в RAM. Утилиты для Linux собираются отдельно.

В bOS 0.3.0 консоль framebuffer отображает 16 цветов ANSI для текста и фона. Команда `lspci` показывает PCI ID устройств, обнаруженных bOS.

В 0.4.0 появились DHCPv4 и `ping` для Intel 82540EM. Версия 0.5.0 добавляет Intel 82574L, Realtek RTL8139 и ручную настройку IPv4.

### Ethernet в bSH

```text
lspci
ifconfig
ping 10.0.2.2
ifconfig static 10.0.2.15/24 10.0.2.2
ifconfig dhcp
```

Адреса в примере относятся к виртуальной сети QEMU. При загрузке bOS выбирает одну карту как `eth0` и запускает DHCP. Ручные настройки действуют до перезагрузки. Для выбора виртуальной карты передайте `BOS_NIC=e1000e` или `BOS_NIC=rtl8139` скрипту `scripts/run-qemu.sh`.

В QEMU проверены Intel 82540EM (`8086:100e`), Intel 82574L (`8086:10d3`) и Realtek RTL8139 (`10ec:8139`). На физических картах проверка ещё не проводилась. Для RTL8168/8111 и Wi-Fi нужны отдельные драйверы.

### Сборка и запуск

На Linux установите Rust через rustup, GNU tar, GNU binutils, QEMU x86-64 и OVMF. Для сборки ISO также понадобятся Python 3 с пакетом `pycdlib`, `dosfstools` и `mtools`. Версия Rust и целевые платформы указаны в `rust-toolchain.toml`.

```sh
bash scripts/build.sh --release
bash scripts/run-qemu.sh
```

По умолчанию QEMU получает 4 ГиБ и запускается с `-cpu max`. Объём памяти можно задать через `BOS_MEMORY` (например, `512M`), путь к OVMF — через `OVMF_CODE`. Чтобы собрать оба образа:

```sh
python3 scripts/mkiso.py
python3 scripts/mkiso.py --install
```

Образы появятся в `target/bos-live.iso` и `target/bos-install.iso`. **Live ISO** загружается как UEFI CD-ROM. **Install ISO** содержит скрипт для установки на отдельный накопитель из Linux. См. [инструкцию по установке](docs/INSTALL.md).

### Постоянный диск bFS

```sh
target/release/mkusers target/users.img
bash scripts/run-qemu.sh target/esp target/users.img
```

Создайте образ один раз и используйте его при следующих запусках. Перед закрытием QEMU выполните `sync`. Команды bSH `reboot` и `halt` также сбрасывают изменения на диск. В 0.2.0 проверен отдельный диск VirtIO в QEMU. bFS кэширует том в RAM и работает без журнала.

В bSH попробуйте `help`, `whoami`, `pwd` и `ls /`. Команда `bitstat` показывает разрядность ядра bOS, а `bfetch` выводит версию ОС, процессор, память и параметры консоли. Введите `calculator.sh` для запуска встроенного калькулятора или `calculator 2 + 3 * 4` для одного вычисления. В интерактивном режиме команда `quit` возвращает в bSH. Для записи заметки откройте `notepad`, введите текст и завершите командой `.save`. Команда `cd` возвращает в домашний каталог. Стрелки вверх и вниз листают историю команд, влево и вправо перемещают курсор. Tab дополняет команды и пути.

Проверка загрузки, ELF64 и сохранения файлов после перезапуска доступна через `python3 scripts/test-qemu.py` после release-сборки.

Подробнее об устройстве системы см. в [описании архитектуры](docs/ARCH.md).

## Инструменты для Linux

Следующие утилиты запускаются в Linux из каталога проекта. Текущий режим
загрузки bOS использует встроенную оболочку `bSH`. Утилита `bnet` использует
сеть Linux-хоста. Команды `ifconfig` и `ping` в bSH используют сеть ядра bOS.

## bPT · bOS Package Tool

**From a GitHub file to an installed package in one command.** bPT 0.1.0 is a Linux-host package manager for the bOS workspace. It offers an APT-style command line for the [official catalog](~BPT-PROGS/registry.json) and can install a regular file, directory or repository directly from public GitHub sources.

**От файла на GitHub до установленной программы одной командой.** bPT 0.1.0 запускается в Linux и устанавливает пакеты в каталог `applications/` проекта bOS. В [`~BPT-PROGS/`](~BPT-PROGS/) находятся официальный индекс и первые программы.

```sh
cargo +stable build --locked --release -p bpt
system/bpt update
system/bpt search hello
system/bpt install hello
applications/hello/hello.sh
system/bpt install https://github.com/dr-lexus-dr/bos/blob/v0.1.0/docs/INSTALL.md
system/bpt list --installed
```

Supported commands include `update`, `list`, `search`, `show`, `install`, `reinstall`, `upgrade`, `remove`, `autoremove`, `depends`, `check`, `download` and `clean`. See the [bPT guide / руководство по bPT](docs/BPT.md) for the catalog format and current limits.

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
cargo +stable build -p bpt -p bnet -p btext
```

## Шрифт консоли

Встроенная таблица глифов сгенерирована из `Lat15-VGA16.psf.gz` пакета `console-setup-linux`. Скрипт генерации находится в [scripts/genfont.py](scripts/genfont.py).

## bitstat on Linux / bitstat в Linux

The standalone utility reports the running x86 Linux kernel architecture. In bSH, `bitstat` reports the bOS kernel architecture.

Утилита показывает разрядность запущенного ядра Linux на x86. В bSH одноимённая команда показывает разрядность bOS.

```sh
cc -std=c11 -Wall -Wextra bitstat.c -o bitstat
./bitstat
```
