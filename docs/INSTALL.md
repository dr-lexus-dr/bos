# Install bOS · Установка bOS

## English

The **Live ISO** boots bSH directly in a virtual machine or from an optical disc. The **Install ISO** also boots bSH and contains a Linux installer that prepares a dedicated USB drive or disk for UEFI boot. Run the installer from a Linux session.

Release assets are produced with `bash scripts/release.sh`; a local plain build writes `target/bos-live.iso` and `target/bos-install.iso`.

1. Download the Install ISO and `SHA256SUMS` into the same directory. Verify the download with `sha256sum --ignore-missing -c SHA256SUMS`.
2. Connect an empty drive. Use `lsblk -d -o NAME,SIZE,MODEL,TRAN` to identify the whole device. The installer **erases the entire selected drive**. Unmount its partitions first.
3. Install the Linux packages providing `sfdisk` and `mkfs.vfat` (on Debian and Ubuntu, `fdisk` and `dosfstools`). Mount the ISO and run its installer. Replace `/dev/sdX` with the actual drive.

   ```sh
   sudo mkdir -p /mnt/bos-install
   sudo mount -o loop,ro bos-0.6.0-x86_64-install.iso /mnt/bos-install
   sudo bash /mnt/bos-install/install.sh /dev/sdX
   sudo umount /mnt/bos-install
   ```

4. Boot the drive from the firmware's UEFI boot menu with Secure Boot disabled. At the bSH prompt, try `help`, `whoami` and `ls /`.

The installer writes a GPT partition table and a FAT32 EFI System Partition. A physical boot uses a RAM session. For persistent files, use a separate bFS data disk in QEMU as described in the [README](../README.md#save-files-between-reboots).

## Русский

**Live ISO** загружает bSH в виртуальной машине или с оптического диска. **Install ISO** также загружает bSH и содержит скрипт для подготовки отдельной флешки или диска с загрузкой UEFI. Скрипт запускается из Linux.

Релизные файлы собираются командой `bash scripts/release.sh`; обычная локальная сборка создаёт `target/bos-live.iso` и `target/bos-install.iso`.

1. Скачайте Install ISO и `SHA256SUMS` в одну папку. Проверьте скачанный образ командой `sha256sum --ignore-missing -c SHA256SUMS`.
2. Подключите пустой накопитель и найдите его через `lsblk -d -o NAME,SIZE,MODEL,TRAN`. Скрипт **стирает весь выбранный диск**. Сначала размонтируйте его разделы.
3. Установите пакеты с `sfdisk` и `mkfs.vfat` (в Debian и Ubuntu это `fdisk` и `dosfstools`). Смонтируйте ISO и запустите скрипт. Вместо `/dev/sdX` укажите путь к накопителю целиком.

   ```sh
   sudo mkdir -p /mnt/bos-install
   sudo mount -o loop,ro bos-0.6.0-x86_64-install.iso /mnt/bos-install
   sudo bash /mnt/bos-install/install.sh /dev/sdX
   sudo umount /mnt/bos-install
   ```

4. Отключите Secure Boot и выберите накопитель в меню загрузки UEFI. В bSH попробуйте команды `help`, `whoami` и `ls /`.

Скрипт создаёт таблицу GPT и системный раздел EFI в FAT32. При загрузке на физическом компьютере сеанс работает в RAM. Постоянный диск bFS поддерживается в QEMU. См. [инструкцию](../README.md#постоянный-диск-bfs).
