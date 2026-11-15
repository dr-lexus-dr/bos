# bOS: текущая реализация

База: v0.6.0 (`603a690`) + локальные изменения. **0.7.0 ещё не готова к выпуску; bOS 1.0 также не завершена.**
Принятый план: [ROADMAP.md](ROADMAP.md). Контракт: [abi.md](abi.md).

## Уже работает

- Проверенные исправления heap/frame allocator, firmware holes, alignment,
  coalescing, overflow, invalid frees; memtest и резерв страниц для ядра.
- Нативный PID 1 при обычной загрузке. Spawn native-shell, сбор сирот и
  завершившихся детей, перезапуск оболочки после exit или Ctrl-C.
- Spawn, waitpid/WNOHANG, getpid/getppid, dup2, pipe, read/write/open/close/
  seek/fsync/sync. До 64 записей процесса, монотонные PID и reuse после reap.
- Отдельные CR3, GPR/x87/SSE, BSP-вытеснение; ожидающие read/write/waitpid
  не busy-loop в пользовательском коде. Idle CPU спит через hlt.
- Pipe 4096 байт: атомарные записи, backpressure, EOF, EPIPE. Дубликаты
  дескрипторов разделяют позицию/endpoint. FD закрываются при exit/fault,
  память ребёнка освобождается при reap; bad wait pointer не поглощает status.
- Нативный bSH: запуск исполняемых файлов, echo, cat FILE, cat FILE | wc -c,
  sync, exit. Библиотека libbos; native Rust программы и process-test.
- Kernel bSH остаётся как recovery; сборка `--recovery` выбирает её явно.
- Постоянный `/users` на выделенном VirtIO, SATA/AHCI либо NVMe-диске в QEMU.
  AHCI выполняет IDENTIFY, DMA read/write, flush, проверяет границы, имеет
  таймауты и прекращает работу после ошибки. Редактор корректно определяет
  постоянный AHCI mount. bFS пока v1.
- NVMe: реальные admin/I/O queues, active namespace discovery, PRP/PRP-list,
  read/write/flush, ограничение MDTS, uncached MMIO выше 4 ГиБ, таймауты и
  прекращение I/O после ошибки. Поддержаны LBA 512–4096 байт без metadata/PI;
  интерфейс VFS остаётся в секторах 512 байт. Проверены 512 и 4096 в QEMU.
- Сохранены M0-диагностика, редактор и Ethernet-регрессии трёх NIC.
- CI расширен на три транспорта хранения; его конфигурация не означает,
  что удалённый GitHub Actions уже запускался.

## Проверки

- Release-сборка успешна. Debug-сборка на nightly-2025-06-01 сейчас
  останавливается в LLVM при сборке sha2 для UEFI; не считается проверенной.
- Host: **36 тестов**, включая настоящий kernel pipe implementation:
  wraparound, PIPE_BUF, last-reader/last-writer, 40 000 байт между потоками.
- Полный QEMU smoke проходит для VirtIO, AHCI и NVMe.
- NVMe matrix: namespace 7, MDTS=1/4, оба режима PRP, wrap очередей,
  512/4096-byte LBA, fsync/power cycle, неизменность диска без метки bOS-users.
- Native boot проверяет PID 1, перезапуск shell, pipe `cat | wc`, Ctrl-C.
- process-test: pipe 8192 через буфер 4096, EOF/EPIPE, общая позиция dup2,
  bad wait pointer, invalid mapping, missing executable, 80 spawn/wait/reap.
- Native fsync проверен записью, остановкой QEMU и чтением на новой загрузке.
- 20 exit + 20 fault с заполненными fd-таблицами возвращают физические кадры.
- Live ISO `target/bos-0.7.0-dev-live.iso` отдельно загружен в QEMU:
  native PID 1 и process/pipe regression прошли. Это промежуточный образ,
  встроенная версия пока 0.6.0; он не содержит нативного установщика.
- 512M/1G/4G: recovery diagnostics и native PID 1/process-test проходят.

```sh
bash scripts/build.sh --release
bash scripts/test-host.sh
python3 scripts/test-qemu.py
BOS_STORAGE=ahci BOS_TEST_LOG_DIR=target/qemu-logs-ahci python3 scripts/test-qemu.py
BOS_STORAGE=nvme BOS_TEST_LOG_DIR=target/qemu-logs-nvme python3 scripts/test-qemu.py
BOS_TEST_LOG_DIR=target/qemu-logs-nvme-matrix python3 scripts/test-nvme.py
python3 scripts/test-boot-matrix.py
```

Тестовый harness работает с временными копиями ESP. Recovery marker не
добавляется в обычный собранный образ. Время загрузки/serial сохраняются в
`target/qemu-logs` или BOS_TEST_LOG_DIR.

## Что остаётся незавершённым

| Область | Реальное ограничение |
| --- | --- |
| Процессы | Нет exec, argv/env, независимого cwd, poll и полноценного job control |
| SMP | Только BSP; глобальные ACTIVE/BATCH и общий TSS нельзя использовать одновременно с AP |
| USB | xHCI/HID/BOT остаются заглушками; PS/2/serial сейчас polling, без RU layout |
| Диски | Один labelled raw bFS-диск; нет GPT, NCQ/hotplug/reset recovery; NVMe polling и одна очередь I/O, без проверки на физическом SSD |
| bFS v2 | Не реализована; v1 кэширует том в RAM, ограничена 32 МиБ, без журнала |
| Сеть | DHCP/ICMP работают; пользовательские TCP/DNS/TLS/HTTPS и нативный bPT отсутствуют |
| Установка | Нативного установщика, boot slots и rollback пока нет |
| Изоляция | Нет uid/permissions; OOM в ряде infallible kernel allocations ещё может вызвать panic |
| Приёмка | E7270, аппаратный AHCI/NVMe/PS2, SMP и 72-часовой soak не проверены |

Нельзя считать указанные подсистемы готовыми по наличию файлов с их именами.
Аппаратный профиль E7270 собирается скриптом collect-hardware.py из Linux;
результаты текущего хоста не заменяют проверку ноутбука.

## Проверка загрузки на E7270 (15 ноября 2026)

Пользователь подтвердил загрузку с USB до native PID 1 и bSH после изменения
открытия GOP: GetProtocol вместо Exclusive. Эксклюзивный запрос мог отключать
драйвер экранной консоли прошивки. Ранний paging использует 2-МиБ страницы
и отображает первые 64 ГиБ; ядро размещается ниже 4 ГиБ без AnyPages fallback.
QEMU smoke с `BOS_CPU=max,pdpe1gb=off` прошёл, включая процессы, bFS и сеть.
Диагностические сообщения Boot stage пока задерживаются на 3 секунды.

После первого аппаратного запуска обнаружен поток ложного ввода. Исправлены
чтение отсутствующего UART (LSR=0xff), ненужные UART IRQ и фильтрация PS/2
AUX/error bytes. Native serial input и process-test прошли в QEMU; встроенная
клавиатура E7270 после исправления ещё не подтверждена пользователем.

USB готовится через `sudo bash scripts/write-uefi-usb.sh`: MBR, FAT32,
EFI type 0xef и fallback loader EFI/BOOT/BOOTX64.EFI. Скрипт предназначен
для конкретной флешки /dev/sdc, serial 0701934063B2FA77, размер 7751073792;
проверяет устройство, временно запрещает automount через udev, затем снимает
ограничение, копирует EFI/kernel/initrd и сравнивает содержимое. Диск стирается.
Это host-инструмент Linux, а не нативный установщик bOS.
