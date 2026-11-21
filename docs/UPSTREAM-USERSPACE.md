# nash и uutils/coreutils

Host-сборка (`bash scripts/build.sh --host-only`) собирает настоящие upstream
nash и uutils/coreutils. Исходники загружаются в `target/upstream`, коммиты
закреплены в `scripts/build-upstream-userspace.sh`. MIT-лицензии сохранены в
`third_party/`. Нужны Git, сеть при первой сборке и `rustup` со stable Rust.
Патч `third_party/patches/nash-linux-rlimit.patch` исправляет тип аргумента
getrlimit/setrlimit для Linux libc.
Upstream собирается отдельно от workspace и nightly прошивки, с `--locked`.

uutils собирается с набором Unix для host и `feat_os_unix_musl` для bOS. Полный список
включённых утилит сохраняется в `target/debug/host/coreutils.list`.

Бинарники и ссылки команд находятся в `target/debug/host/bin` (либо
`target/release/host/bin`). Старые host-входы `bsh` и `bcoreutils` передают
управление nash и coreutils через exec с сохранением аргументов и кода выхода.
Они добавляют каталог upstream-команд в начало PATH.

Пример:

```sh
bash scripts/build.sh --host-only
./target/debug/bsh -c 'printf "hello\n" | wc -c'
./target/debug/bcoreutils ls --version
python3 scripts/test-upstream-userspace.py
```

## Нативная bOS

Добавлен `kernel/src/user/linux.rs`: static PIE/musl, SYSCALL, TLS,
argv/env/auxv, приватные страницы, fork/exec/wait, файловые операции и poll.
`bash scripts/build-upstream-userspace.sh --native` собирает бинарники в
`target/native/bin`. Полная сборка теперь включает этот каталог в initrd;
обычная загрузка открывает рабочий стол. Профиль `nash` запускает
`/bin/nash` через native-init. Полноэкранная bSH доступна по F12.

QEMU/KVM: подтверждены загрузка nash через PID 1, дочерний nash -c,
`coreutils ls --version` и конвейер `printf abc | coreutils wc -c` (3).
Слой пока неполный: нет потоков/shared-memory clone,
полноценной доставки POSIX-сигналов, сокетов и полного job control. Все 106
утилит собраны, но полный набор их режимов ещё не проверен.

Регрессионная проверка в самой ОС: `python3 scripts/test-native-upstream.py`.
При доступном KVM можно задать `BOS_ACCEL=kvm`.

В `/bin` установлены 106 команд uutils, `sh` (ссылка на nash), `bfetch`
и настоящая musl-сборка bpt. Проверены bfetch, bpt --version, stat и id -u.
Сетевая установка bpt требует дальнейшей реализации sockets/TLS runtime ABI;
наличие бинарника не означает работоспособность всех сетевых команд.
Для C-кода TLS-зависимости bpt используется GCC (переменная
`CC_x86_64_unknown_linux_musl` позволяет переопределить компилятор).
