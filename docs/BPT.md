# bPT · bOS Package Tool

bPT manages packages in a bOS workspace from Linux. It installs files under `applications/<package>/`, stores its database in `system/bpt-data/`, and writes operations to `system/logs/bpt.log`.

bPT управляет пакетами в рабочей папке bOS из Linux. Пакеты устанавливаются в `applications/<имя>/`, база хранится в `system/bpt-data/`, журнал — в `system/logs/bpt.log`.

## Build and use / Сборка и запуск

```sh
cargo +stable build --locked --release -p bpt
system/bpt update
system/bpt install hello sysinfo
applications/hello/hello.sh
system/bpt show hello
system/bpt remove hello
```

The release also provides a Linux x86-64 binary. Run it from the bOS checkout or set `BOS_ROOT` to the workspace path. `BPT_REGISTRY` overrides the catalog URL.

В релизе есть готовый бинарник для Linux x86-64. Запускайте его из папки проекта или укажите её через `BOS_ROOT`. Переменная `BPT_REGISTRY` переопределяет URL каталога.

## Commands / Команды

| Command | English | Русский |
| --- | --- | --- |
| `update` | Refresh the package index | Обновить индекс |
| `list [category]` | List catalog packages | Показать каталог |
| `list --installed` | List installed packages | Показать установленные |
| `list --upgradable` | List packages with a changed catalog version | Показать пакеты с новой версией в каталоге |
| `search <query>`, `show <name>` | Search and view details | Найти пакет и посмотреть сведения |
| `install <spec>...`, `reinstall <name>` | Install or reinstall packages | Установить или переустановить пакеты |
| `upgrade [name...]` | Apply catalog version changes | Применить изменения версий из каталога |
| `remove <name>...`, `autoremove` | Remove packages or unused automatic dependencies | Удалить пакеты или ненужные зависимости |
| `depends <name>`, `check` | View dependencies or check the installed state | Показать зависимости или проверить установку |
| `download <spec>`, `clean` | Save the GitHub source archive or clear the index cache | Скачать архив исходников или очистить кеш индекса |

## GitHub sources / Источники GitHub

```sh
system/bpt install https://github.com/dr-lexus-dr/bos/blob/v0.1.0/docs/INSTALL.md
system/bpt install 'dr-lexus-dr/bos@v0.1.0:docs/ARCH.md'
system/bpt install 'dr-lexus-dr/bos@v0.1.0:docs'
```

The format is `owner/repo[@ref][:path]`. Without a path, bPT installs the whole repository. Without a ref, it uses the default branch. Use the short form for branch names containing slashes. GitHub `blob` and `tree` URLs are supported.

Формат — `owner/repo[@ref][:path]`. Без пути устанавливается весь репозиторий. Без ссылки на ветку, тег или коммит используется ветка по умолчанию. Для веток с косой чертой используйте короткий формат. Поддерживаются ссылки GitHub `blob` и `tree`.

## Publish a package / Добавление пакета

Put the package in `~BPT-PROGS/<name>/` and add an entry to `~BPT-PROGS/registry.json`:

```json
{
  "name": "hello",
  "version": "1.0.0",
  "description": "Print a greeting from bPT",
  "category": "utilities",
  "repo": "dr-lexus-dr/bos",
  "ref": "bpt-v0.1.0",
  "path": "~BPT-PROGS/hello",
  "bin": ["hello.sh"],
  "requires": []
}
```

`path` can point to a file or directory. `bin` lists paths relative to the installed package that should be executable. `requires` lists catalog package names. Optionally set `sha256` to verify the downloaded archive. Use a release tag or commit to pin package contents.

`path` указывает на файл или каталог. `bin` задаёт исполняемые файлы относительно установленного пакета. В `requires` указываются имена зависимостей из каталога. Поле `sha256` позволяет проверить архив. Для фиксации содержимого используйте тег или коммит.

## Scope / Возможности первой версии

bPT borrows APT's workflow for GitHub packages. It supports named dependencies and optional SHA-256 checks. Debian packages, signed APT repositories, version constraints, and native bSH execution are outside this release. bPT extracts regular files and directories, rejects links and special files in the selected package, and does not execute package scripts during installation.

bPT использует знакомые по APT команды для пакетов с GitHub. В первой версии поддерживаются зависимости по имени и опциональная проверка SHA-256. Поддержка `.deb`, подписанных репозиториев APT, ограничений версий зависимостей и запуск внутри bSH пока отсутствуют. При установке bPT извлекает обычные файлы и каталоги, отклоняет ссылки и специальные файлы и не запускает скрипты пакета.
