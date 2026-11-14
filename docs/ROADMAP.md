# bOS: план развития до 75/100

**Дата:** 2026-11-14  
**База:** `4b29ff1` + текущие незакоммиченные изменения (я их не трогал)  
**Цель:** превратить bOS из учебного bring-up проекта в небольшую, но настоящую ОС для Dell Latitude E7270 и QEMU.

---

## 0. Коротко

- **Сейчас:** ~30/100 как повседневная ОС. Это выше прежних 25, потому что ring-3, bFS и smoltcp DHCP/ICMP реально работают. Но всё это живёт только в QEMU (или на железе без диска/сети) и запускается из встроенного kernel-shell.
- **75/100 — это не Linux.** Это ОС, которая ставится на E7270, грузится в юзерспейс, хранит файлы на NVMe/SATA, работает с внутренней клавиатурой, USB и Ethernet, умеет процессы/пайпы, имеет тесты, CI и базовую изоляцию. Браузера, Wi-Fi, GPU-ускорения и POSIX-совместимости в этих 75 нет.
- **Оценка:** ~55 person-weeks чистой работы + 40% резерва = **~75 person-weeks (~1.5 person-years)**. Один человек по 20 ч/нед — ~3 года. Два разработчика full-time — ~10–12 месяцев. Это не проект на выходные.
- **Критический путь:** M0 → M1 (процессы/syscall) → M2 (диск) → M3/M4 (железо/сеть) → M5 (надёжность/безопасность) → M6 (UX/релиз).
- **Главный риск:** расползание scope. Wi-Fi, браузер, GPU, POSIX, Windows/macOS-совместимость нужно явно вынести за 75.

---

## 1. Что такое 75/100

Это проверяемый Definition of Done, а не «стало красивее».

### 1.1. Пользовательские сценарии

1. **Установка и загрузка на E7270**
   - GPT: ESP + корневой bFS-раздел на внутреннем NVMe/SATA.
   - bLOADER грузит ядро и initrd с ESP, ядро монтирует корень с диска.
   - Загрузка до shell — до 5 секунд. `reboot` и `halt` реально выключают машину через ACPI.
   - Secure Boot выключен (bLOADER не подписан). Опционально — собственная подпись в M6.

2. **Ежедневная работа**
   - PID 1 — юзерспейсный `init`, shell и coreutils — обычные ring-3 процессы.
   - Пайпы, редиректы, фоновые задачи, Ctrl-C, `wait`, коды возврата.
   - Файлы в `/users/<name>` сохраняются после перезагрузки и выключения питания.
   - Редактор `babo`, калькулятор, `ls/cat/cp/mv/rm/find/df`, установка пакетов.

3. **Железо E7270**
   - Внутренняя клавиатура (PS/2 через EC) и тачпад (PS/2 или I2C HID — решить по факту).
   - USB-клавиатура/мышь и hotplug через xHCI.
   - Ethernet I219-LM: DHCP, DNS, ping, TCP HTTP.
   - Внешний монитор через GOP, консоль 1920×1080.
   - Wi-Fi **не нужен** для 75 (см. non-goals).

4. **Сеть**
   - `ifconfig`, `ping`, `bnet http://...` скачивает файл на диск.
   - DHCP renew, устойчивость к выдёргиванию кабеля, без потери пакетов под нагрузкой.

5. **Надёжность**
   - 72-часовой soak без паники.
   - Выключение питания во время записи не ломает ФС: `fsck` чинит, данные не теряются молча.
   - Паника пишет crash-log, который можно достать после перезагрузки.
   - OOM убивает процесс, а не ядро.

6. **Безопасность (базовая, не «secure OS»)**
   - Ring-3 изоляция, W^X, NX, SMEP/SMAP.
   - Все указатели из юзерспейса проверяются; нельзя прочитать память ядра через syscall.
   - Kernel-shell остаётся только как recovery-режим, не как обычная оболочка.
   - Без полноценного multi-user/POSIX-прав; достаточно uid 0/1000 и режимов файлов.

### 1.2. Non-goals для 75

| Не делаем | Почему |
|---|---|
| Wi-Fi (Intel 8260) | Нужны firmware-блобы, 802.11 state machine, WPA2/3 supplicant, regulatory. Отдельные 6–12 месяцев. Используем Ethernet/USB-tethering. |
| Браузер, TLS, видео | Слишком большой объём. Для 75 достаточно HTTP-клиента. |
| GPU-ускорение | GOP framebuffer достаточен для консоли и редактора. |
| Звук, Bluetooth, кардридер, отпечаток | Не влияют на «usable OS» и стоят недель. |
| POSIX/Linux binary compat | `compat/win32`, `compat/macho`, ELF-совместимость — не цель. Свой ABI проще. |
| Secure Boot | Для hobby-ОС достаточно выключить. Подпись — stretch в M6. |
| SMP-масштабирование | 2–4 ядра полезны, но не обязательны для 75. Включаем в M5, не раньше. |
| Multi-user, квоты, ACL | Достаточно одного пользователя и базовых прав. |

> Шкала субъективная. 75 означает «энтузиаст может поставить и пользоваться на одном ноутбуке», а не «production-ready».

---

## 2. Честная текущая картина

Это не README, а то, что реально в коде.

| Подсистема | Что уже есть | Чего нет | Ключевые файлы |
|---|---|---|---|
| Boot/UEFI | bLOADER, GPT ESP, загрузка kernel+initrd, GOP-консоль, serial-лог | Подписи, boot menu, передача root-раздела, ACPI RSDP | `bootloader/src/main.rs`, `bloader_shared/src/lib.rs` |
| Kernel core | GDT/IDT/TSS, paging, heap, PIC/APIC, PIT, panic | ACPI, SMP INIT/SIPI (есть только `_start_ap_rust`), watchdog | `kernel/src/main.rs`, `arch/x86_64/*`, `mm/*` |
| Процессы | `sched` (таблицы, runqueue), `proc` (структуры), `user::run` запускает один статический ELF | Реального context switch и преемпции нет; `syscall::init` пуст; `int 0x80` — заглушка; PID 1 нет | `sched/mod.rs`, `proc/mod.rs`, `syscall/mod.rs`, `user.rs`, `user_entry.asm` |
| Syscall ABI | `dispatch()` с номерами, `write/exit/read/open/...` | Нет входа из ring 3, нет валидации указателей, нет `copy_from_user` | `syscall/mod.rs` |
| ФС | ramfs как `/`, bFS (1512 LOC) с CRC superblock, `rename`, `truncate`, initrd | bFS только в RAM; persistence только virtio-blk; нет journal/fsck в ядре; нет GPT | `fs/mod.rs`, `fs/bfs.rs` |
| Драйверы | PCI, PS/2 polling, serial, GOP, PIT/PIC/APIC, virtio-blk, e1000 (100e/10d3), rtl8139 | AHCI/NVMe — probe-only; USB — stub; I219-LM не матчится; Wi-Fi нет | `drivers/*` |
| Сеть | smoltcp 0.12, DHCP + ICMP, polling | TCP/UDP/DNS/sockets, IRQ, e1000e/I219, userland API | `drivers/net.rs`, `drivers/e1000.rs` |
| Юзерспейс | `userspace/hello`, `.bh` ELF запускается через `user::run` | `init` — 6-строчная заглушка; `bsh` — host-сборка; coreutils — host | `userspace/*` |
| Тесты | `scripts/test-qemu.py`: boot, babo, calculator, bFS corruption, 3 NIC; 20 `#[test]` в host-крейтах | Нет CI, fuzz, power-cut, матрицы RAM/диска, coverage | `scripts/test-qemu.py` |
| Безопасность | ring-3 + NX для user-страниц | `from_raw_parts` без `access_ok`; нет SMEP/SMAP/KPTI; kernel shell в ring 0 | `user.rs`, `syscall/mod.rs` |

### 2.1. Что это значит

- Ядро **не** «пустое»: paging, heap, ELF, bFS, smoltcp и ring-3 уже есть. Это хорошая база.
- Но **ОС как система** ещё нет: нет процессов, диска на реальном железе, ввода и сети вне QEMU.
- Поэтому план не «переписать всё», а **включить и соединить** существующие куски, заменив заглушки.

---

## 3. Модель оценки

Веса и оценки субъективные, но явные. Меняешь приоритет — меняй таблицу.

| Критерий | Вес | Сейчас | Цель 75 | Кто двигает |
|---|---:|---:|---:|---|
| Boot/firmware | 10% | 7.5 | 9.0 | M0, M2, M3 |
| Ядро (MM, IRQ, таймеры, SMP) | 10% | 5.5 | 8.0 | M1, M5 |
| Процессы/syscall/юзерспейс | 15% | 2.0 | 8.0 | M1 |
| Хранилище/ФС | 12% | 3.0 | 8.5 | M2 |
| Драйверы/железо | 15% | 1.5 | 7.0 | M3, M4 |
| Сеть | 8% | 2.5 | 7.0 | M4 |
| Безопасность/изоляция | 8% | 1.0 | 6.0 | M1, M5 |
| Надёжность/тесты | 10% | 2.0 | 7.5 | M0, M5 |
| UX/приложения | 8% | 6.0 | 7.5 | M6 |
| Покрытие целевого железа | 4% | 1.0 | 8.0 | M3, M4 |
| **Итого** | **100%** | **~32** | **~77** | |

Что даёт основные баллы:

- **+15–18**: процессы, syscall, userspace (M1). Без этого это не ОС, а монитор.
- **+10–12**: NVMe/AHCI + bFS root + installer (M2).
- **+10–12**: xHCI/HID, ACPI, I219-LM (M3/M4).
- **+8–10**: тесты, CI, crash-consistency, безопасность (M0/M5).
- **+3–4**: UX/приложения/документация (M6).

---

## 4. Принципы

1. **QEMU-first, E7270 — еженедельно.** Каждый коммит проверяется в QEMU; раз в неделю — загрузка с USB на ноутбуке.
2. **Один milestone = shippable состояние.** После каждого этапа есть тег, демо и список того, что реально работает.
3. **Никаких big-bang переписываний.** bFS, ELF-loader и smoltcp переиспользуем.
4. **Тесты — часть фичи, не «потом».** Новый драйвер без QEMU-теста не считается готовым.
5. **Fault injection по умолчанию.** Битой ELF, битый bFS, кончившееся место, выдернутый кабель, OOM.
6. **Безопасность с M1, а не в конце.** Валидация указателей — часть syscall ABI.
7. **Scope заморожен.** Всё из non-goals не обсуждается до 75.
8. **Измеряем, а не верим.** Boot time, throughput, panic count, fuzz hours.

---

## 5. Дорожная карта

### M0 — Фундамент, правда и CI (2–3 person-weeks)

**Цель:** чтобы любой следующий шаг проверялся автоматически, а падения были диагностируемы.

**Задачи:**

- [x] CI: `.github/workflows/ci.yml` — release build, host-тесты, QEMU smoke под OVMF, RAM-матрица 512 MiB / 1 GiB / 4 GiB, serial-логи и ESP как артефакты. `fmt`/`clippy`, AHCI/NVMe и конфигурация 2 GiB / 8 GiB ещё не включены.
- [x] Переделать `scripts/test-qemu.py` в переиспользуемый harness: таймауты, сохранение serial-лога, код возврата, матрица.
- [x] Crash-log: panic + регистры + backtrace пишутся best-effort в `/users/.crash.log`; команда `crashlog` читает лог после ребута. Отдельный raw-раздел 1 MiB не делали.
- [x] `dmesg`: ring buffer 64 KiB в ядре. Уровни логов и `loglevel=` ещё не сделаны.
- [x] Починить лимит 16 GiB и панику на 32 GiB в `kernel/src/mm/phys.rs`; битмап покрывает 64 GiB. 32 GiB на локальном хосте не воспроизводились.
- [x] Замер boot time (TSC от входа в ядро до shell), команда `bootstat`.
- [x] `docs/STATUS.md`: матрица «работает / не работает / чем проверено».
- [x] Зафиксировать baseline тегом `v0.6.0` после коммита.

**Статус M0 на 2026-11-14:** закрыт частично. Проверены 512M/1G/4G, host-тесты
и QEMU smoke; 32G, реальное падение для crash-log и зелёный CI на GitHub ещё
не подтверждены. Подробности — в [STATUS.md](STATUS.md).

**Acceptance:**

- [~] CI зелёный на 3 конфигурациях QEMU: workflow создан, локально проверены 512M/1G/4G; первый прогон на GitHub ещё не выполнен.
- [~] Паника даёт читаемый лог после перезагрузки: чтение и очистка проверены, автоматическая запись не проверена fault-injection тестом.
- [~] 32 GiB RAM в QEMU загружается без паники: битмап и арифметика готовы, но 32 GiB не запускались.
- [x] Boot time измеряется и записан.
- [x] `docs/STATUS.md` соответствует коду, а не README.

**Файлы:** `.github/workflows/ci.yml`, `scripts/test-qemu.py`, `kernel/src/mm/phys.rs`, `kernel/src/logger.rs`, `kernel/src/main.rs`, `docs/STATUS.md`.

---

### M1 — Процессы, syscall ABI, юзерспейс (8–10 person-weeks)

**Цель:** bOS перестаёт быть kernel-shell с одноразовым ring-3 запуском и становится многопроцессной ОС.

**Задачи:**

- [ ] **ADR-001: syscall ABI.** Выбрать `SYSCALL/SYSRET` (MSR STAR/LSTAR/FMASK, swapgs), `int 0x80` оставить для совместимости. Зафиксировать номера, структуры, коды ошибок в `docs/abi.md`.
- [ ] **ADR-002: модель создания процессов.** Рекомендация: сначала `spawn(path, argv, env, fds)` + `wait`, без `fork`; пайпы через `pipe` + `dup2` + `spawn`. `fork` с COW — позже, если понадобится.
- [ ] Включить `sched`: настоящий context switch (asm), idle thread, преемпция по таймеру, sleep/wake, приоритеты; stress-тест на 1 час.
- [ ] Включить `proc`: таблица PID, process/thread, fd table, cwd, argv/env, exit/zombie/reap, `wait`.
- [ ] `spawn/exec/wait/exit`, `pipe`, `dup2`, `kill`; Ctrl-C (SIGINT).
- [ ] `copy_from_user` / `copy_to_user` / `access_ok` с page walk; убрать `from_raw_parts` из syscall; тесты с заведомо плохими указателями.
- [ ] Ужесточить ELF-loader: пересечение сегментов, выравнивание, entry, guard page под стеком, argv/env/auxv, W^X.
- [ ] `libbos` — userland-крейт: `_start`, syscall-обёртки, аллокатор, stdio, fs, process, time, args. Цель `x86_64-bos` (JSON уже есть в `kernel/x86_64-bos.json`, нужно вынести/адаптировать).
- [ ] `userspace/init` — настоящий PID 1; `userspace/shell` и coreutils переписать на `libbos`; kernel `bsh` оставить как recovery (`--recovery`).
- [ ] `ps`, `kill`, минимальный `top`.

**Acceptance:**

- PID 1 работает в ring 3; одновременно живут ≥3 процесса.
- `cat file | wc -c` работает через реальные пайпы.
- Ctrl-C прерывает CPU-bound процесс.
- Попытка прочитать адрес ядра из юзерспейса возвращает `EFAULT`, а не данные.
- 1 час stress (spawn/kill/pipe/alloc) без паники.
- В обычной загрузке нет kernel-shell.

**Файлы:** `kernel/src/sched/mod.rs`, `proc/mod.rs`, `syscall/mod.rs`, `user.rs`, `user_entry.asm`, `arch/x86_64/{gdt,idt,tss}.rs`, `mm/paging.rs`, `userspace/init`, `userspace/shell`, новый `userspace/libbos`, `docs/abi.md`.

**Риск:** это самый большой и важный этап. Если M1 не сделан, остальное — косметика.

---

### M2 — Хранилище, bFS root, установщик (8–10 person-weeks)

**Цель:** bOS ставится на внутренний диск и переживает выключение питания.

**Задачи:**

- [ ] Block layer: реестр устройств, GPT/MBR parser, разделы, flush, обработка ошибок, кэш; дополнить `drivers/block.rs`.
- [ ] **AHCI:** HBA init, port reset, command list/FIS, IDENTIFY, read/write, таймауты, hotplug. QEMU `ich9-ahci`.
- [ ] **NVMe:** admin/IO queues, PRP, IDENTIFY, read/write, flush, reset, таймауты. QEMU `nvme`; E7270.
- [ ] bFS: монтирование с GPT-раздела (type GUID/label), `fsck` (host + kernel), journal **или** copy-on-write метаданных, atomic rename, `fsync`, ENOSPC, большие файлы.
- [ ] Bootloader: передать в `BootInfo` disk/partition/root UUID, ACPI RSDP, Boot menu (bOS / recovery), таймаут.
- [ ] Installer: Linux-скрипт `scripts/bos-install.sh` (GPT: ESP 512 MiB + bFS root), копирование kernel/initrd/apps, `boot.cfg`; документация. Позже — установщик внутри bOS.
- [ ] Тесты: power-cut (kill QEMU в случайный момент записи), 10k create/delete, заполнение диска, битый bFS, `fsck`-repair.

**Acceptance:**

- Установка и загрузка с QEMU SATA/NVMe и с внутреннего диска E7270.
- Файлы переживают `kill -9` QEMU во время записи; `fsck` чинит.
- `df` показывает правду; файл >1 GiB читается/пишется.
- virtio-blk больше не обязателен.

**Файлы:** `kernel/src/drivers/{block,ahci,nvme}.rs`, `drivers/pci.rs`, `fs/mod.rs`, `fs/bfs.rs`, `bootloader/src/main.rs`, `bloader_shared/src/lib.rs`, `scripts/bos-install.sh`, `fs/bfs/src/lib.rs`.

**Решение на M2:** если journal упирается в сроки — перейти на ext2 (проверенный формат) или на metadata-COW. Решать на 2-й неделе M2, не в конце.

---

### M3 — Платформа, ACPI, ввод и USB (10–12 person-weeks)

**Цель:** E7270 становится пригодным для работы без внешней клавиатуры и с нормальным выключением.

**Задачи:**

- [ ] Driver framework: MMIO mapping, DMA-аллокатор (contiguous, <4 GiB), IRQ registration, MSI/MSI-X, PCIe ECAM (MCFG), BAR sizing.
- [ ] ACPI: RSDP/XSDT/FADT/MADT/MCFG/HPET; shutdown/reboot; IRQ overrides; PCI routing; RTC; батарея/температура — stretch.
- [ ] **xHCI:** controller init, slots, endpoints, transfer rings, interrupts, hubs, hotplug; HID boot keyboard/mouse; USB mass storage (BOT/SCSI) — stretch.
- [ ] **PS/2:** i8042 init, IRQ1/IRQ12, Set-1/Set-2, key repeat, раскладки US/RU; тачпад.
- [ ] Display: GOP modes, scrollback, 32bpp, fallback; EDID — stretch.
- [ ] Timer: HPET/TSC calibration, APIC timer, monotonic clock.

**Acceptance:**

- Внутренняя клавиатура E7270 работает без polling-хаков; тачпад — PS/2 или USB-мышь.
- USB-клавиатура/мышь подключаются на ходу.
- `reboot` и `halt` реально выключают ноутбук.
- Консоль 1920×1080, ввод через IRQ, а не busy-poll.
- QEMU xHCI-тест в CI.

**Файлы:** `drivers/{usb,pci,timer,console}.rs`, `arch/x86_64/{acpi,apic}.rs` (новые), `mm/dma.rs` (новый), `bootloader` (RSDP).

**Риск:** xHCI — самый сложный драйвер в плане. Timebox 4 недели; если не успеваем, 75 всё ещё возможен на PS/2 + Ethernet, но USB HID останется stretch. USB mass storage можно не делать: установка с Linux-хоста.

---

### M4 — Сеть (6–8 person-weeks)

**Цель:** Ethernet на реальном E7270 и нормальный TCP-клиент.

**Задачи:**

- [ ] **e1000e / I219-LM:** PCIe MMIO, DMA rings, IRQ, link status, EEPROM MAC, варианты 82574/82583/I219. QEMU `e1000e`.
- [ ] smoltcp: включить `socket-tcp`, `socket-udp`, DNS; poll из IRQ/таймера; DHCP renew; loopback.
- [ ] Socket ABI: `socket/bind/connect/listen/accept/send/recv/close/poll`, non-blocking, таймауты, DNS.
- [ ] Порт `bnet` на bOS: HTTP GET, скачивание файла; `ping`, `ifconfig`, `netstat`.
- [ ] Устойчивость: выдернутый кабель, DHCP renew, MTU, retransmit.

**Acceptance:**

- E7270 получает DHCP, резолвит DNS, качает HTTP-файл 10 MiB.
- ≥100 Mbps, без потерь под нагрузкой.
- Кабель можно выдернуть/воткнуть — сеть восстанавливается.
- QEMU e1000e в CI.

**Файлы:** `drivers/e1000.rs` (переписать в e1000e), `drivers/net.rs`, `kernel/Cargo.toml` (features smoltcp), `userspace/bnet`, `docs/abi.md` (socket ABI).

---

### M5 — Надёжность, безопасность, SMP (8–10 person-weeks)

**Цель:** системе можно доверять больше, чем демо.

**Задачи:**

- [ ] SMEP/SMAP, W^X ядра, rodata, guard pages, аудит `unsafe`.
- [ ] Fuzz: syscall, ELF, bFS; 1M кейсов без паники.
- [ ] OOM: убивать процесс, не ядро; лимиты; `mmap/munmap` (если ещё нет).
- [ ] Crash dump, watchdog, recovery boot.
- [ ] SMP: AP trampoline, INIT/SIPI, per-CPU GDT/TSS/IDT, APIC, балансировка; либо явно «APs parked» с документацией.
- [ ] Изоляция: отдельные page tables уже есть; рассмотреть KPTI; kernel-shell только recovery; uid/euid, режимы файлов.

**Acceptance:**

- 24-часовой soak без паники.
- 1M fuzz-кейсов без паники.
- SMEP/SMAP включены; user не читает/не пишет память ядра.
- 4 CPU online (или документированное решение single-core).
- OOM убивает процесс.

**Файлы:** `arch/x86_64/{smp,apic}.rs`, `mm/*`, `syscall/mod.rs`, `user.rs`, `fuzz/*`, `.github/workflows/ci.yml`.

---

### M6 — UX, приложения, релиз (6–8 person-weeks)

**Цель:** этим можно пользоваться, а не только показывать.

**Задачи:**

- [ ] Userspace shell: jobs, пайпы, редиректы, history, completion, скрипты.
- [ ] Порт `babo`, `calculator`, coreutils; формат пакета `.bpk`, `bpt` install/remove.
- [ ] Boot menu, installer UI, recovery, `dmesg`, `top`, `free`, `mount`, `fsck`, `update`.
- [ ] Документация: user guide, developer guide, architecture, porting, ABI.
- [ ] Релизные артефакты: USB-образ, ISO, reproducible build.
- [ ] Установка на чистый E7270, 72-часовой soak.

**Acceptance:**

- Чистая установка <15 минут.
- Пользователь редактирует/сохраняет/запускает программы.
- Recovery работает.
- Документация позволяет собрать и поставить без тебя.

---

## 6. Порядок и зависимости

| Этап | Длительность | Зависит от | Можно параллелить |
|---|---:|---|---|
| M0 | 2–3 недели | — | — |
| M1 | 8–10 недель | M0 | M2 (вторым разработчиком) |
| M2 | 8–10 недель | M0 | M1 |
| M3 | 10–12 недель | M1 + M2 (driver framework) | M4 после framework |
| M4 | 6–8 недель | M3 (PCIe/DMA/IRQ) | M3, если есть второй человек |
| M5 | 8–10 недель | M1–M4 | — |
| M6 | 6–8 недель | всё | — |

**Критический путь (соло):** M0 → M1 → M2 → M3 → M4 → M5 → M6 = **48–61 неделя** чистой работы. С резервом — 70–85 недель.

**С двумя разработчиками:** M1 и M2 параллельно; M3 и M4 частично параллельно. Wall-clock ≈ **40–52 недели**, то есть 10–12 месяцев.

---

## 7. Оценка календаря

| Сценарий | Чистая работа | С резервом 40% | Календарь |
|---|---:|---:|---:|
| 1 человек, 10 ч/нед | ~55 pw | ~75 pw | ~5.5–7 лет |
| 1 человек, 20 ч/нед | ~55 pw | ~75 pw | ~2.7–3.5 года |
| 1 человек, 40 ч/нед | ~55 pw | ~75 pw | ~1.4–1.8 года |
| 2 человека, 40 ч/нед | ~55 pw | ~75 pw | ~9–12 месяцев |
| 3 человека, 40 ч/нед | ~55 pw | ~75 pw | ~7–9 месяцев (выше coordination overhead) |

> «person-week» = 35–40 продуктивных часов. Оценки не включают дизайн, ревью, закупку железа и burnout.

### Если 75 слишком долго

**60/100 за ~6–8 месяцев (соло, 20 ч/нед)** — реалистичный сокращённый путь:

- M0 + M1 + M2 + частично M3 (PS/2, ACPI, GOP) + частично M4 (e1000e/I219, TCP).
- Без USB mass storage, SMP, journal (fsck + atomic rename), полноценных приложений.
- Это уже «ставится на E7270, работает shell, диск и Ethernet», но без USB hotplug и полировки.

---

## 8. Риски и решения

| Риск | Вероятность | Влияние | mitigation |
|---|---|---|---|
| Scope creep (Wi-Fi, браузер, POSIX) | высокая | высокое | Заморозить non-goals; любое расширение — только после 75 |
| xHCI сложнее ожидаемого | высокая | высокое | Timebox 4 недели; fallback PS/2 + Ethernet; USB mass storage — stretch |
| NVMe-варианты/таймауты | средняя | высокое | Тест на E7270 и QEMU; AHCI fallback; таймауты и reset |
| bFS crash consistency | высокая | высокое | Journal или metadata-COW; decision point на 2-й неделе M2; иначе ext2 |
| DMA/IOMMU, >4 GiB | средняя | высокое | DMA-аллокатор <4 GiB, bounce buffers, тесты |
| I219-LM варианты | средняя | среднее | Семейство e1000e, тест на E7270 + QEMU |
| Rust nightly ломается | средняя | среднее | Pin toolchain, CI cache, обновление раз в квартал |
| Burnout | высокая | среднее | Milestones, демо, 20% буфер, не героичить |
| Железо сломается | низкая | высокое | Запасной SSD/USB, QEMU как основной стенд |
| Security regression | средняя | высокое | Fuzz, review, SMEP/SMAP, тесты на bad pointers |
| Производительность | средняя | низкое | Мерять boot/IO/IRQ; оптимизировать после 75 |

---

## 9. Первые две недели (конкретный backlog)

**День 1–2**

- [ ] Закоммитить текущие изменения (или отдельную ветку), поставить тег `v0.5.1-baseline`.
- [ ] Создать ветку `roadmap/m0`.
- [ ] Добавить CI skeleton: fmt, clippy, build, `scripts/test-qemu.py`.

**День 3–4**

- [ ] Прогнать QEMU-матрицу: 512 MiB / 2 GiB / 8 GiB, virtio-blk / AHCI / NVMe.
- [ ] Сохранять serial-лог и код возврата в artifacts.
- [ ] Починить 32 GiB panic в `mm/phys.rs`.

**День 5–7**

- [ ] Crash-log + `dmesg` ring buffer.
- [ ] `bootstat`.
- [ ] Написать `docs/abi.md` (черновик) и ADR-001 (SYSCALL vs int 0x80).
- [ ] ADR-002 (spawn vs fork).

**Неделя 2**

- [ ] Прототип context switch в `sched`: два kernel-thread, переключение по таймеру.
- [ ] `init` в ring 3 с `write/exit`.
- [ ] `copy_from_user` + тест «плохой указатель → EFAULT».
- [ ] Загрузить USB на E7270 и записать фактическое поведение (клавиатура, GOP, PS/2, диск).

**Definition of Done двух недель:**

- CI зелёный.
- Два kernel-thread переключаются.
- `init` работает в ring 3.
- Плохой указатель не читает память ядра.
- Паника оставляет лог.
- Есть честный `docs/STATUS.md`.

---

## 10. Метрики, которые надо снимать

| Метрика | Цель 75 |
|---|---|
| Boot to shell, E7270 | ≤5 s |
| Boot to shell, QEMU | ≤2 s |
| Panic per 24h soak | 0 |
| Syscall latency | <1 µs |
| Context switch | <5 µs |
| NVMe read | >500 MB/s |
| SATA read | >200 MB/s |
| Ethernet | ≥100 Mbps |
| Install time | <15 min |
| Tests | `cargo test` + QEMU matrix green |
| bFS/ELF/syscall coverage | >70% |
| Fuzz | ≥1M кейсов без паники |

---

## 11. Решения, которые нужны до M1

| ADR | Вопрос | Рекомендация |
|---|---|---|
| ADR-001 | Syscall ABI | `SYSCALL/SYSRET`; `int 0x80` — compat |
| ADR-002 | Создание процессов | `spawn` + `wait`; `fork` с COW позже |
| ADR-003 | Root FS | bFS + journal; ext2 — fallback |
| ADR-004 | Userland runtime | `no_std` + `libbos`; Rust std не портируем |
| ADR-005 | Сеть | smoltcp; свой стек не пишем |
| ADR-006 | Installer | Linux-скрипт; self-host позже |
| ADR-007 | SMP | single-core до M5; APs parked |
| ADR-008 | Security model | один пользователь, uid 0/1000, базовые режимы |

---

## 12. Что не делать в ближайшие 6 месяцев

- Wi-Fi, Bluetooth, звук, камера, отпечаток, кардридер.
- Браузер, TLS, видео, GPU.
- POSIX/Linux/Windows/macOS binary compat.
- Собственный TCP/IP стек, собственная ФС с нуля, собственная libc «как glibc».
- Микроядерный редизайн, формальная верификация, KASLR.
- Multi-user, ACL, квоты, containers.

---

## 13. Итог

75/100 — это не «дописать README», а **1.5 person-years** сфокусированной работы. Хорошая новость: в bOS уже есть половина фундамента (paging, heap, ELF, bFS, smoltcp, ring-3). Плохая: отсутствуют процессы, реальные драйверы и тесты — то, что превращает ядро в ОС.

Порядок работ выбран так, чтобы **каждый этап давал работающий результат**, а не «через год всё сразу». Если делать только одно — делать M1. Без процессов и syscall остальное не имеет смысла.

Следующий конкретный шаг: M0 backlog из раздела 9, начиная с CI и crash-log.
