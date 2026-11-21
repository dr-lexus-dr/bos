# Нативный ABI bOS: текущая рабочая копия

Статические ELF64 работают в ring 3 на BSP. `int 0x80`: номер в rax,
аргументы rdi/rsi/rdx, результат rax, ошибки отрицательные errno.
Номера совместимы с прежними бинарниками bOS; это не Linux binary ABI.
Обычная загрузка открывает рабочий стол. Маркер `/system/native` в расширенном initrd запускает нативный PID 1 и bSH, а `/system/nash` выбирает nash. Из графической сессии F12 открывает полноэкранную bSH.


| Номер | Вызов | Аргументы / результат |
| --- | --- | --- |
| 0 | read | fd, writable buffer, length; число байт, 0 = EOF |
| 1 | write | fd, buffer, length; число байт |
| 2 | open | UTF-8 NUL path, flags, reserved; fd |
| 3 | close | fd; 0 |
| 8 | lseek | fd, signed offset, whence 0/1/2; позиция |
| 22 | pipe | writable pointer to two u32 fd; 0 |
| 33 | dup2 | old fd, new fd; new fd |
| 39 | getpid | PID текущего процесса |
| 60 | exit | status low 8 bits |
| 61 | waitpid | pid или -1, optional i64 status pointer, flags 0/1; PID, 0 = WNOHANG |
| 74 | fsync | файловый fd; текущая реализация синхронизирует все тома |
| 110 | getppid | PID родителя; PID 1 сообщает 0 |
| 162 | sync | синхронизация всех томов |
| 1004 | spawn | UTF-8 NUL path, FdMapping array pointer, mapping count; child PID |

## Процессы

До 64 записей процесса в сеансе. PID монотонно увеличивается; успешно
собранные waitpid слоты переиспользуются. PID 1 собирает сирот, пока
оболочка работает, и перезапускает её после выхода. Fault ребёнка передаётся
как отрицательный статус `-vector-1`; обычный exit — 0..255.

`spawn` запускает статический ELF без argv/env. `FdMapping` — два u32
`source`, `target`. До 64 уникальных target; некорректный source = EBADF,
повтор target или превышение лимита = EINVAL. Вначале наследуются fd 0/1/2,
затем применяются mappings. Остальные fd закрыты в ребёнке. Источники всегда
читаются из исходной таблицы родителя, а не из промежуточной таблицы ребёнка.
Дублированные файлы разделяют позицию; pipe endpoints живут до последнего
дескриптора. Независимого cwd пока нет: используется общий cwd VFS.

`waitpid` получает статус только своего ребёнка. Неверный status pointer
возвращает EFAULT без reap. Нет ребёнка = ECHILD. Ожидание блокирует процесс;
при WNOHANG (flags=1) возвращает 0, если соответствующий ребёнок жив.
Файлы закрываются при exit/fault, страницы и таблицы освобождаются при reap.
Родитель, выходя, передаёт детей PID 1 (если он жив).

## I/O и планирование

Каждый read/write ограничен 4096 байтами. libbos::write_all обрабатывает
частичные операции. Пути ограничены 4096 байтами вместе с NUL.
Флаги open: RDONLY=0, WRONLY=1, RDWR=2, CREAT=0x40, EXCL=0x80,
TRUNC=0x200, APPEND=0x400, DIRECTORY=0x10000.

Pipe имеет кольцо 4096 байт. Запись до 4096 байт атомарна, при недостатке
места writer блокируется. Пустое кольцо блокирует reader, пока есть writers;
после закрытия последнего writer остаток читается, затем EOF. Нет readers =
EPIPE. Неверное направление = EBADF, seek по pipe = ESPIPE.

Консольный read блокируется до байта serial/PS2. Ожидающие syscall сохраняются
как операции в процессе; kernel stack не удерживается. Если все живые процессы
ждут, CPU выполняет `sti; hlt; cli`; APIC-таймер будит планировщик.
Round-robin, квант ~10 мс, отдельные CR3, GPR/x87/SSE. AVX отключён.
Kernel execution невытесняемое; глобальное состояние BSP несовместимо с SMP.

Ctrl-C завершает сеанс оболочки и его задачи, но сохраняет native PID 1,
который запускает новую оболочку. Это ещё не POSIX foreground process groups.

## Проверки и ограничения

Нативный process-test проверяет pipe 8192 байт через кольцо 4096, EOF/EPIPE,
порядок данных, общую позицию dup2, bad wait pointer, missing executable,
invalid mapping и 80 spawn/wait с переиспользованием ресурсов.
Host-набор тестирует именно kernel pipe implementation, включая wraparound
и параллельных producer/consumer.

Не реализованы SYSCALL, exec, argv/env, poll/socket ABI, права пользователей,
полноценный job control, SMP. libbos — начальный runtime, не готовая libc.
Восстановительная kernel bSH выбирается параметром UEFI `recovery` либо
файлом `/system/recovery` в initrd. QEMU regression harness добавляет marker
только в свою временную копию ESP; обычный образ запускает native init.

## ClaudeOS i386 ABI

Статические ELF32 `ET_EXEC` для `EM_386` исполняются в режиме совместимости x86-64, в ring 3 и в отдельных адресных пространствах. Область программы начинается с `0x40000000`. При входе `[esp]` содержит argc, `[esp+4]` — указатель на завершающийся нулём массив argv. Номер вызова передаётся в EAX, аргументы — в EBX, ECX и EDX, результат возвращается в EAX после `int 0x80`.

| EAX | Операция |
| ---: | --- |
| 0 | exit(status) |
| 1, 2 | write(fd, buf, size), read(fd, buf, size) |
| 3, 4 | open(path), close(fd) |
| 5 | getpid() |
| 6, 7 | sleep_ms(ms), ticks_ms() |
| 8 | sbrk(increment) |
| 9, 10 | yield(), getkey() |
| 11 | stat_size(path) |

В `/bin` live-образа доступны hello, primes, guess, counter, wc, spin и crash. Примеры также доступны в `/applications/i386`. Ядро принимает внешние программы с тем же соглашением. Поддержка не распространяется на Linux i386 ABI. [Заголовок и примеры](../userspace/compat32)
