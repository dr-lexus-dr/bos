# Boot bOS

## English

The bOS 1.0 release ships a raw x86-64 disk image. Download it with `SHA256SUMS`, verify it with `sha256sum -c SHA256SUMS`, and boot the image in QEMU or write it to a USB stick with a disk-image writer. Writing an image replaces the contents of the selected stick. Choose the stick in the BIOS or UEFI boot menu. For an unsigned UEFI image, use a machine that permits booting it with Secure Boot disabled. The desktop opens in English. [Download](https://github.com/dr-lexus-dr/bOS/releases/tag/v1.0.0) · [QEMU command, supported hardware and storage](../README.md)

Files live in RAM unless a separate bFS users volume or a FAT12/FAT16 volume labelled `BOSDATA` is attached. The boot medium itself can stay read-only.

For the optional full UEFI development profile, `bash scripts/release.sh --full 1.0.0` builds the live disk, the full Live and Install ISOs and an ESP archive. All files appear in `target/release-1.0.0/`. The optional Install ISO includes a Linux script for preparing a dedicated UEFI drive. [Full build and userspace](UPSTREAM-USERSPACE.md)

## Русский

В релизе bOS 1.0 доступен raw-образ диска для x86-64. Скачайте образ и `SHA256SUMS`, выполните `sha256sum -c SHA256SUMS` и откройте образ в QEMU либо запишите его на флешку как образ диска. Запись заменяет содержимое выбранной флешки. Выберите её в меню загрузки BIOS или UEFI. Для неподписанного образа UEFI компьютер должен допускать загрузку с отключённым Secure Boot. Рабочий стол открывается на английском. Для переключения на русский нажмите **EN** в верхней панели. [Скачать](https://github.com/dr-lexus-dr/bOS/releases/tag/v1.0.0) · [Команда QEMU и требования к оборудованию](../README.ru.md)

Файлы хранятся в ОЗУ. Чтобы сохранить их между загрузками, подключите отдельный том bFS либо FAT12/FAT16 с меткой `BOSDATA`. Загрузочную флешку можно оставить в режиме чтения.

Для расширенной UEFI-сборки выполните `bash scripts/release.sh --full 1.0.0`. В `target/release-1.0.0/` появятся Live-диск, образы Live и Install ISO и архив загрузочного раздела ESP. В Install ISO есть Linux-скрипт для подготовки выделенного UEFI-диска. [Расширенная сборка](UPSTREAM-USERSPACE.md)
