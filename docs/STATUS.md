# bOS · статус

**Дата:** 2026-11-14  
**Версия:** 0.6.0 (M0)  
**Платформа проверки:** QEMU x86-64 + OVMF, Rust `nightly-2025-06-01`  
**Цель этого документа:** отделить то, что реально проверено, от планов из
[ROADMAP.md](ROADMAP.md) и от рекламных формулировок в README.

---

## 1. Коротко

bOS 0.6.0 — это учебная терминальная ОС, которая загружается по UEFI в QEMU,
даёт bSH, bFS-диск, запуск статического ELF64 в ring 3 и Ethernet на трёх
виртуальных контроллерах. M0 добавил диагностику загрузки, `dmesg`, crash-log,
QEMU-матрицу и CI.

Это **не** повседневная ОС для Dell Latitude E7270. Установка на внутренний
NVMe/SATA, USB HID, ACPI-выключение, PID 1 в юзерспейсе и многопроцессность
относятся к M1–M3 и ещё не реализованы.

Текущая честная оценка — около **32/100** как повседневная система. Оценка
75/100 и Definition of Done находятся в [ROADMAP.md](ROADMAP.md).

---

## 2. Матрица проверенного

| Возможность | Статус | Чем проверено |
| --- | --- | --- |
| UEFI-загрузка через OVMF | Работает | `scripts/test-qemu.py`, matrix 512M/1G/4G |
| bSH: ввод, история, Tab, стрелки | Работает | `scripts/test-qemu.py` |
| bFS в RAM | Работает | boot без диска данных |
| bFS на VirtIO-диске, 3 перезагрузки | Работает | persistence-сценарий в `test-qemu.py` |
| Отказ битого bFS-диска | Работает | образ с `BAD!` отвергается, диск не меняется |
| ELF64 в ring 3 | Работает, один процесс | `/applications/hello.bh` |
| Падение ring-3 процесса не убивает ядро | Работает | `/applications/fault.bh` → `user fault (vector 14)` |
| Ethernet e1000 / e1000e / rtl8139 | Работает в QEMU | DHCP + `ping 10.0.2.2` |
| Статический IPv4 | Работает в QEMU | `ifconfig static ...` |
| Цвета ANSI в serial и framebuffer | Работает | release-тест |
| Редактор `babo` | Работает | сохранение, поиск, undo/redo, отказ от изменений |
| `dmesg [-c]` | Работает | кольцевой буфер 64 KiB, проверка в `test-qemu.py` |
| `bootstat` и `uptime` | Работает | TSC/PIT-калибровка, вывод в тесте |
| `crashlog [-c]` | Чтение и очистка работают | проверено в тесте |
| Автоматическая запись panic в `/users/.crash.log` | Реализована best-effort | не проверена fault-injection тестом |
| RAM 512M / 1G / 4G | Работает | `scripts/test-boot-matrix.py` |
| RAM 32G | Не запускалась | нужен хост с >32 GiB; битмап рассчитан на 64 GiB |

---

## 3. M0: что закрыто

- [x] `scripts/test-qemu.py` переведён на общий harness
      `scripts/qemu_console.py`: таймауты, коды возврата, сохранение serial-лога.
- [x] Serial-логи пишутся в `target/qemu-logs` (переопределяется
      `BOS_TEST_LOG_DIR`).
- [x] `dmesg` с кольцевым буфером 64 KiB и `dmesg -c`.
- [x] `bootstat`: TSC/PIT-калибровка и время от входа в ядро.
- [x] `crashlog` и best-effort запись паники в `/users/.crash.log`.
- [x] Защита от рекурсивной паники.
- [x] Битмап физической памяти увеличен до 2 MiB: покрытие 64 GiB вместо
      16 GiB. Исправлена паника на больших объёмах RAM.
- [x] QEMU-матрица 512M / 1G / 4G.
- [x] `scripts/test-host.sh`: 15 host-пакетов, 20 тестов.
- [x] `.github/workflows/ci.yml`: host-тесты, release-сборка, QEMU smoke,
      RAM-матрица, выгрузка serial-логов и ESP.
- [x] `docs/STATUS.md` (этот файл).
- [ ] Загрузка 32 GiB в QEMU: математика и битмап готовы, но на локальном
      хосте с 11 GiB она не воспроизводится.

---

## 4. Что не работает

| Область | Состояние |
| --- | --- |
| Dell Latitude E7270 | Загрузка с USB проверена ранее; полная установка и железо — нет |
| Внутренний NVMe/SATA | Нет AHCI/NVMe-драйверов, нет GPT-парсера в ядре |
| Установщик на внутренний диск | Есть Linux-скрипт `scripts/mkusb.sh`; установка на E7270 не проверена |
| USB HID | Нет xHCI; внутренняя клавиатура E7270 пока недоступна |
| ACPI shutdown/reboot | Нет ACPI; `halt`/`reboot` работают как QEMU-заглушки |
| PID 1 в юзерспейсе | Нет; bSH остаётся kernel-shell |
| Многопроцессность, `fork`/`spawn`, пайпы | Нет; ring-3 запуск одноразовый |
| Wi-Fi, браузер, TLS, GPU, звук | Вне scope 0.6.0 |
| Пользователи, права, ACL | Нет |
| Secure Boot | bLOADER не подписан; Secure Boot должен быть выключен |
| Журналирование bFS | Нет; bFS буферизуется в RAM |

---

## 5. Как воспроизвести

```sh
bash scripts/build.sh --release
bash scripts/test-host.sh
python3 scripts/test-qemu.py
python3 scripts/test-boot-matrix.py
bash scripts/release.sh
```

Ожидаемый результат:

- host-тесты: `20 passed`;
- `test-qemu.py`: `Live boot, line editing, ring-3 execution, bFS persistence,
  M0 diagnostics and Ethernet passed`;
- matrix: три строки `boot and M0 diagnostics passed` для 512M, 1G и 4G.

Логи QEMU лежат в `target/qemu-logs/<label>.log`. Каталог можно переопределить
через `BOS_TEST_LOG_DIR`.

---

## 6. Известные ограничения

1. **`mem` показывает span, а не только usable RAM.** `TOTAL_MEMORY`
   считается как диапазон от первого до последнего conventional-региона,
   поэтому MMIO-дыры входят в «phys total». Свободная память и счётчик
   кадров при этом корректны. Это косметическая проблема, не источник
   паники.
2. **4G в QEMU — не то же самое, что 32G.** 4G проверяет адреса выше
   PCI-дыры и арифметику высоких адресов. 32G требует отдельного хоста.
3. **Crash-log не проверен реальным падением.** Чтение, очистка и формат
   проверены; запись из `panic_handler` использует `try_lock` и может быть
   пропущена, если VFS уже заблокирован. Это осознанный компромисс.
4. **CI — конфигурация, а не доказательство.** Workflow создан, но первый
   зелёный прогон на GitHub зависит от установленных OVMF/QEMU и появления
   коммита в `main`.
5. **E7270 остаётся целью, а не проверенной платформой.** Для неё нужны
   NVMe/SATA, PS/2 или USB HID, ACPI и отключённый Secure Boot.

---

## 7. Следующий шаг

M1 из [ROADMAP.md](ROADMAP.md): ADR-001 (syscall ABI), ADR-002 (модель
процессов), реальный планировщик, таблица процессов и PID 1 в ring 3.
Без M1 остальные milestone'ы не приближают к 75/100.
