# bOS 1.0: принятый план реализации

Цель: полноценная терминальная ОС с собственным Rust-ядром для QEMU и
Dell Latitude E7270. База — v0.6.0 и сохранённые локальные файловые syscall.
Это план, а не заявление о готовности. Текущий прогресс: [STATUS.md](STATUS.md).
Прежняя дорожная карта сохранена в [ROADMAP-0.6.0.md](ROADMAP-0.6.0.md);
её проценты, сроки и исключения TLS/SMP больше не являются критериями релиза.

## Обязательные результаты

- Самостоятельная установка с Live USB на выделенный диск, без Linux.
- Нативный init PID 1, bSH, приложения в изолированных процессах, пайпы,
  редиректы, фоновые задачи, Ctrl-C, несколько ядер CPU.
- bFS v2, большие файлы, журнал, fsck, восстановление после сбоя.
- USB HID/hubs/hotplug и USB BOT, Ethernet, DHCP/DNS/TCP/HTTPS.
- Нативный bPT с GitHub Release, подписанным каталогом, атомарной установкой.
- Повторяемая сборка, аппаратная приёмка E7270 и 72-часовой soak.

GUI, браузер, Wi-Fi, звук, Bluetooth, suspend/resume, dual boot и совместимость
с бинарниками Linux/Windows/macOS не входят в 1.0. Secure Boot отключён.

## Этап A: база и память

Сохранить регрессии 0.6.0; аудит аллокаторов (выравнивание, overflow,
coalescing, повторное освобождение, частично выполненный mapping), fallible
пользовательские операции с ENOMEM и резерв ядра. CI: ELF/user pointers,
утечки памяти/дескрипторов, serial и идентификатор сборки.

Аппаратный профиль собирается на E7270 через
`python3 scripts/collect-hardware.py --out target/e7270-profile` из Linux.
Скрипт не меняет устройства; ошибки доступа к ACPI фиксируются в отчёте.
GOP и фактическая работа устройств дополнительно проверяются загрузкой bOS.

Приёмка: регрессии проходят, повторные запуски и циклы памяти не дают
устойчивого роста ресурсов. Одних тестов bitmap недостаточно для закрытия A.

## Этап B: процессы и пользовательская среда

BSP round-robin 10 мс, полноценный контекст/CR3/TSS/FPU, kernel stacks,
idle, sleep/wake без busy polling. Владение PID/cwd/fd/argv/env, parent,
zombie/reap. spawn/exec/waitpid/exit/pipe/dup2/poll и группы процессов;
fork и произвольные signal handlers отложены.

Один безопасный dispatcher; сохранить int 0x80, добавить SYSCALL с iretq.
libbos, нативные init/bSH/coreutils; kernel-shell только recovery.
Приёмка: >=3 процесса, реальные пайпы, Ctrl-C CPU-bound программы,
изоляция и час stress без паники.

## Этап C: платформа, ввод, SMP

Общие MMIO/DMA/IRQ/PCI, минимальные IRQ handlers с отложенной работой.
ACPI и порт ACPICA для AML, APIC timer/RTC/reboot/shutdown. PS/2 IRQ,
xHCI control/bulk/interrupt, hubs/hotplug/HID, USB BOT/SCSI.
INIT/SIPI, per-CPU GDT/TSS/очереди, reschedule IPI, балансировка,
TLB shootdown и документированный порядок блокировок.

Приёмка: 1/2/4 CPU QEMU и все логические CPU E7270, hotplug,
ввод и выключение на железе. Совпадение PCI-семейства не означает поддержку.

## Этап D: диски и bFS v2

Блочный слой с GPT, read/write/flush, таймаутами и ограниченными очередями;
VirtIO/AHCI/NVMe/USB BOT. Ограниченный кэш вместо тома целиком в RAM.
bFS v2: 64-bit sizes/extents, metadata checksums, ordered redo journal,
commit/checkpoint flush, replay/fsync/atomic rename, общий код формата и fsck.
bFS v1 — чтение и перенос копированием, без миграции на месте.
Невосстановимые повреждения — read-only/recovery.

Приёмка: файл >1 ГиБ, 10k операций, ENOSPC, fault injection каждой границы
журнала. Подтверждённый fsync сохраняется; неподтверждённые записи могут
потеряться без разрушения структуры ФС.

## Этап E: Ethernet, HTTPS, bPT

Драйвер фактического Ethernet E7270; smoltcp TCP/UDP/DNS/loopback,
DHCP renew, link recovery, сокеты в общей fd table. Системная энтропия,
RTC и подтверждённое время; без них TLS отказывает безопасно.
embedded-tls + rustls-webpki, trusted roots, hostname/chain/date validation;
никакой собственной криптографии и отключения проверки сертификатов.

Нативные bnet/bPT: HTTPS/SNI, ограниченные redirects без downgrade,
потоковые загрузки. Каталог ABI/version/dependencies/size/SHA256/GitHub URL,
подпись Ed25519 и встроенный ключ. .bpk = tar + manifest, только обычные
файлы, без install scripts/path traversal. Staging, атомарное переключение
версии, восстановление после прерванной установки.
Приёмка: GitHub install/upgrade/remove, отказ неверных cert/signature/hash,
обрыв сети и питания.

## Этап F: установка и релиз

Нативный текстовый установщик Live bOS: модель/serial/размер диска,
запрет установки на загрузочный носитель, подтверждение полной переразметки.
GPT, ESP 512 МиБ + bFS v2; kernel/initrd/recovery/root UUID на ESP.
Рабочий и предыдущий boot slot; обновление неактивного слота и откат.
Повтор установки после прерывания. UID 0/1000 и права файлов;
обычная оболочка не root.

Портировать babo/calculator/coreutils и добавить ps/top/free/df/mount/fsck/
date/ifconfig/bnet/bpt. Live ISO, USB image, checksums и документация.
Приёмка: чистая установка E7270, загрузка без USB, recovery/rollback,
72 часа смешанной нагрузки QEMU и E7270, воспроизводимая сборка.

## Порядок и совместимость

A → B → C → D → E → F; каждый этап заканчивается проверенной сборкой.
Существующие номера syscall сохраняются, новые структуры фиксируются до
клиентов; dup2 разделяет open-file description. W^X/NX/guard pages и
проверка указателей обязательны; SMEP/SMAP по CPUID. BootInfo version bump
при несовместимом изменении. Неизвестный обязательный feature bFS запрещает
запись. Linux host-бинарники не считаются нативными пакетами.

Сроки переоцениваются по завершении B. Непроверенные E7270, SMP, TLS и
нативная установка не могут быть заменены заглушками для объявления 1.0.
