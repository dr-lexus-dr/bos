# bOS

**bSH · bFS · bPT**

*Boot to a shell. Work with files. Manage GitHub packages with bPT.*

*Загрузитесь в оболочку. Работайте с файлами. Управляйте пакетами GitHub через bPT.*

[English](#english) · [Русский](#русский) · [How to install / Установка](docs/INSTALL.md) · [Status / Статус](docs/STATUS.md) · [Changelog](CHANGELOG.md)

## English

**bOS is a small, terminal-first operating system for x86-64 UEFI.** Boot straight into bSH, explore the filesystem, edit a note, or mount a bFS volume. The Rust kernel runs without the standard library, and a fresh session starts in `/users/default` with three directories at the root: `/system`, `/users`, and `/applications`.

bOS 0.2.0 can save `/users` to a dedicated bFS disk in QEMU and run a static ELF64 program in user mode. Live sessions without a data disk use RAM. The Linux tools ship as separate programs.

bOS 0.3.0 adds ANSI foreground and background colors to the framebuffer console. Use `lspci` to see the PCI IDs of devices detected by bOS.

bOS 0.4.0 adds DHCPv4 and `ping` on the Intel 82540EM. bOS 0.5.0 adds Intel 82574L, Realtek RTL8139 and manual IPv4 configuration. bOS 0.6.0 adds the M0 reliability baseline: `dmesg`, boot timing, a best-effort crash log, a 64 GiB frame bitmap, repeated QEMU RAM tests, and CI. See the [status matrix](docs/STATUS.md) and [changelog](CHANGELOG.md).

### Boot diagnostics

```text
dmesg          # kernel log ring (64 KiB)
dmesg -c       # print and clear the ring
bootstat       # boot time, TSC frequency, log size
uptime         # timer ticks and milliseconds since kernel entry
crashlog       # read a panic persisted by the previous boot
crashlog -c    # clear it
```

A crash log is written only after `/users` is mounted. Writing uses a
non-blocking VFS lock, so a panic while the filesystem is busy may be reported
on the serial console but not persisted. The current limits are listed in
[STATUS.md](docs/STATUS.md).

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

The launcher defaults to 1 GiB and `-cpu max`. Set `BOS_MEMORY` (for example, `4G`) to change RAM or `OVMF_CODE` if the firmware is installed at a different path. For the ISO images, use the same build output:

```sh
python3 scripts/mkiso.py
python3 scripts/mkiso.py --install
bash scripts/release.sh   # versioned ISOs + SHA256SUMS
```

The outputs are `target/bos-live.iso` and `target/bos-install.iso`. Release assets land in `target/release-0.6.0/`. Boot the **Live ISO** as a UEFI CD-ROM. The **Install ISO** includes a Linux script for installing bOS onto a dedicated drive. See the [installation guide](docs/INSTALL.md).

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
/applications/hello.bh
ls /
cd documents
notepad
```

Type a line in the editor, then `.save` to write it or `.quit` to discard it. Run `cat notes.txt` to read the file. Use `cd` to return home. Up and Down browse command history, Left and Right move the cursor, and Tab completes commands and paths. The prompt is green, regular output is white, and blue and yellow highlight details. Errors are red. Colors appear in the serial terminal and on the framebuffer screen.

### babo editor

Run `babo [file]` in bSH. With no filename it opens `/users/default/documents/notes.txt`.
It provides line numbers, horizontal and vertical scrolling, search, go-to-line,
undo/redo, line cut/paste, and confirmation before discarding changes.
UTF-8 text is preserved, and UTF-8 input is accepted over the serial terminal.
Files can be up to 128 KiB. Saving stages a new file before replacing the original.

| Shortcut | Action |
| --- | --- |
| Ctrl+S / Ctrl+O | Save and flush the mounted filesystem |
| Ctrl+Q / Ctrl+X | Quit |
| Ctrl+W / Ctrl+N | Search / next match |
| Ctrl+Z / Ctrl+Y | Undo / redo |
| Ctrl+K / Ctrl+U | Cut line / paste |
| Ctrl+G | Go to line |
| Home / End / PgUp / PgDn | Navigate |

## Русский

**bOS — компактная терминальная операционная система для x86-64 UEFI.** После загрузки открывается bSH, где можно работать с файлами, редактировать заметки и подключать тома bFS. Ядро написано на Rust без стандартной библиотеки. Сеанс начинается в `/users/default`, а в корне находятся три каталога.

```text
/
├── system/         журналы, временные файлы и точки монтирования
├── users/
│   └── default/    documents, downloads, pictures, music, videos
└── applications/   приложения
```

В bOS 0.2.0 каталог `/users` можно хранить на отдельном диске bFS в QEMU. Команда `/applications/hello.bh` запускает ELF64 в пользовательском режиме. Исполняемые пользовательские программы bOS используют расширение `.bh`. Без диска данных изменения хранятся в RAM. Утилиты для Linux собираются отдельно.

В bOS 0.3.0 консоль framebuffer отображает 16 цветов ANSI для текста и фона. Команда `lspci` показывает PCI ID устройств, обнаруженных bOS.

В 0.4.0 появились DHCPv4 и `ping` для Intel 82540EM. Версия 0.5.0 добавляет Intel 82574L, Realtek RTL8139 и ручную настройку IPv4. Версия 0.6.0 добавляет базовую диагностику M0: `dmesg`, замер времени загрузки, best-effort crash-log, битмап на 64 ГиБ, повторяемую RAM-матрицу QEMU и CI. См. [матрицу статуса](docs/STATUS.md) и [changelog](CHANGELOG.md).

### Диагностика загрузки

```text
dmesg          # кольцевой журнал ядра, 64 КиБ
dmesg -c       # вывести и очистить журнал
bootstat       # время загрузки, частота TSC, размер журнала
uptime         # тики таймера и миллисекунды с момента входа в ядро
crashlog       # прочитать панику, сохранённую предыдущей загрузкой
crashlog -c    # очистить crash-log
```

Crash-log записывается только после монтирования `/users`. Запись использует
неблокирующую блокировку VFS, поэтому паника при занятой файловой системе
может остаться только в serial-консоли. Актуальные ограничения перечислены в
[STATUS.md](docs/STATUS.md).

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

По умолчанию QEMU получает 1 ГиБ и запускается с `-cpu max`. Объём памяти можно задать через `BOS_MEMORY` (например, `4G`), путь к OVMF — через `OVMF_CODE`. Чтобы собрать оба образа:

```sh
python3 scripts/mkiso.py
python3 scripts/mkiso.py --install
bash scripts/release.sh   # версионированные ISO + SHA256SUMS
```

Образы появятся в `target/bos-live.iso` и `target/bos-install.iso`. Релизные файлы — в `target/release-0.6.0/`. **Live ISO** загружается как UEFI CD-ROM. **Install ISO** содержит скрипт для установки на отдельный накопитель из Linux. См. [инструкцию по установке](docs/INSTALL.md).

### Постоянный диск bFS

```sh
target/release/mkusers target/users.img
bash scripts/run-qemu.sh target/esp target/users.img
```

Создайте образ один раз и используйте его при следующих запусках. Перед закрытием QEMU выполните `sync`. Команды bSH `reboot` и `halt` также сбрасывают изменения на диск. В 0.2.0 проверен отдельный диск VirtIO в QEMU. bFS кэширует том в RAM и работает без журнала.

В bSH попробуйте `help`, `whoami`, `pwd` и `ls /`. Команда `bitstat` показывает разрядность ядра bOS, а `bfetch` выводит версию ОС, процессор, память и параметры консоли. Введите `calculator.bh` для запуска встроенного калькулятора или `calculator 2 + 3 * 4` для одного вычисления. В интерактивном режиме команда `quit` возвращает в bSH. Для записи заметки откройте `notepad`, введите текст и завершите командой `.save`. Команда `cd` возвращает в домашний каталог. Стрелки вверх и вниз листают историю команд, влево и вправо перемещают курсор. Tab дополняет команды и пути. Приглашение зелёное, основной текст белый, детали выделяются синим и жёлтым, ошибки красные. Оформление работает в последовательном терминале и на экране framebuffer.

### Редактор babo

Введите `babo [файл]` в bSH. Без имени открываются заметки
`/users/default/documents/notes.txt`. Редактор показывает номера строк,
поддерживает прокрутку, поиск, переход к строке, отмену и повтор изменений,
вырезание и вставку строки. Перед выходом с несохранёнными изменениями
появляется подтверждение. Ограничение файла — 128 КиБ.

`Ctrl+S` сохраняет, `Ctrl+Q` закрывает, `Ctrl+W` ищет,
`Ctrl+N` переходит к следующему совпадению, `Ctrl+Z` отменяет,
`Ctrl+Y` повторяет, `Ctrl+K` вырезает строку, `Ctrl+U` вставляет,
`Ctrl+G` переходит к строке. Для выбора сохранения или отмены при выходе
введите `s` или `d` и нажмите Enter. Файлы UTF-8 сохраняются,
ввод UTF-8 поддерживается через последовательный терминал.

Проверка release-сборки: `bash scripts/test-host.sh`, `python3 scripts/test-qemu.py` и `python3 scripts/test-boot-matrix.py`. Первый запускает host-тесты, второй — загрузку, ELF64, bFS, редактор, диагностику M0 и Ethernet, третий — загрузку с 512 МиБ, 1 ГиБ и 4 ГиБ RAM.

Подробнее об устройстве системы см. в [описании архитектуры](docs/ARCH.md). Что реально проверено, а что только планируется, — в [STATUS.md](docs/STATUS.md).

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
applications/hello/hello.bh
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
system/network wget <url> [-O file]
                                  # wget-style download syntax
system/network git <args...>     # run Git on the Linux host
system/network interfaces         # list network interfaces
system/network port <host> <port> # check if a TCP port is open
system/network traceroute <host>  # trace route to host
```

The kernel side lives in `drivers/net` (e1000/rtl8139/virtio-net stubs);
ARP/IPv4/TCP and `SYS_NET_*` syscalls are next (see `docs/ARCH.md`).

`bnet` runs on the Linux host and uses the host network. `wget` is a download
alias; `git` forwards arguments to the host's Git installation. For example:

```sh
system/network wget https://example.com/archive.tar.gz -O archive.tar.gz
system/network git clone https://github.com/owner/repository.git
```

Build: `cargo build -p bnet` (sources in `userspace/bnet/`).

## Сборка утилит для Linux

```sh
cargo +stable build -p bpt -p bnet -p btext
```

`bnet` запускается в Linux и использует сеть хоста. Команда `wget` скачивает
файл (`-O` задаёт путь), а `git` передаёт аргументы установленному в Linux
исполняемому файлу Git:

```sh
system/network wget https://example.com/archive.tar.gz -O archive.tar.gz
system/network git clone https://github.com/owner/repository.git
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
