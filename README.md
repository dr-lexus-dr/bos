# bOS — Minimal Operating System

bOS — экспериментальная терминальная ОС для x86-64 UEFI. Ядро написано на Rust без стандартной библиотеки. Оболочка bSH работает внутри ядра. Архитектура описана в [docs/ARCH.md](docs/ARCH.md).

## Загрузка в терминал

Установи Rust через rustup, GNU tar, QEMU x86-64 и OVMF. Затем запусти из корня проекта.

```sh
bash scripts/build.sh
bash scripts/run-qemu.sh
```

Версия Rust nightly и целевые платформы указаны в `rust-toolchain.toml`.
Путь к прошивке можно задать через `OVMF_CODE`.

bSH запускается в домашнем каталоге `/users/default`. Команда `cd` без аргументов возвращает туда.

```text
whoami
pwd
help
ls /
cd /users/default/documents
echo "First line" > notes.txt
edit -a notes.txt
```

Консоль принимает команды в ASCII. Например, введите `Second line` в
редакторе, затем `.save`. Команда `.quit` отменяет ввод. Без `-a` редактор
заменяет содержимое при сохранении. Для просмотра используйте `cat notes.txt`.

При загрузке в корне bOS есть три каталога.

```text
/
├── system/                 системные файлы
│   ├── logs/
│   ├── mounts/
│   └── tmp/
├── users/
│   └── default/            домашняя папка пользователя
│       ├── downloads/
│       ├── documents/      здесь лежит notes.txt
│       ├── pictures/
│       ├── music/
│       └── videos/
└── applications/           приложения
```

Корень использует ramfs. Дополнительный том bFS можно подключить командой
`mount ramdisk /system/mounts/data bFS`. Начальные файлы загружаются из initrd,
а изменения в RAM пропадают после перезагрузки.

Загрузочный UEFI ISO можно собрать из того же ESP с помощью `mkfs.vfat`,
`mtools` и Python-пакета `pycdlib`.

```sh
python3 scripts/mkiso.py
```

Результат — `target/bos-live.iso`. Каталог `target/` и данные пользователей исключены из Git. Пустой `notes.txt` создаётся при загрузке.

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
