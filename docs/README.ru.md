# Основная информация

#### **SKLint (`sklint`)** — минималистичный линтер, форматировщик и статический анализатор Python-кода, написанный на **Rust** и предназначенный для проектных правил **StableKite**.  
#### Инструмент работает как самостоятельная консольная утилита, как Python-пакет с командой `sklint`, а также как VSCode расширение с минимальным TypeScript-слоем.

Автор: **StableKite**  
Сайт: <https://stablekite.com>  
Email: <stablekite@stablekite.com>

# Теоретическая информация о линтере

SKLint ориентирован на быстрый статический анализ исходного текста Python без запуска пользовательского кода.  
В основе лежит Rust-ядро `sklint-core`, которое получает текст файла, конфигурацию и путь файла, после чего возвращает список диагностик, безопасных исправлений и служебной информации для VSCode.  
CLI, Python wrapper и VSCode расширение не реализуют правила самостоятельно, а вызывают одно и то же ядро, чтобы поведение в терминале, `pip`-установке и редакторе оставалось одинаковым.

В проекте принято разделять предупреждения по группам кодов:

| Группа | Назначение |
|--------|------------|
| `SK0xx` | Базовые синтаксические и файловые правила |
| `SK1xx` | Strict-only правила общего назначения |
| `SK2xx` | Комментарии и вывод в runtime |
| `SK3xx` | Пустые строки и вертикальное форматирование |
| `SK4xx` | Скобки, присваивания и точечные style-правила, не дублирующие Ruff |
| `SK5xx` | Импорты, platform-ветвления и runtime-структура |
| `SK6xx` | Докстринги, Google style и описания публичного API |
| `SK7xx` | Pyright-aware статический анализ |
| `SK8xx` | Strict-only performance simplifications |
| `SK9xx` | Служебные предупреждения, например неактуальные suppressions |

SKLint намеренно не должен заменять Ruff, Pyright, Pylint, wemake, flake8 или pydoclint.  
Если правило уже полностью покрыто внешним инструментом, оно не добавляется в SKLint. В SKLint остаются только проектные требования, дополнительные проверки и те случаи, которые существующие инструменты не закрывают полностью.

# Общая информация о проекте

Файлы проекта имеют следующую структуру:

```text
sklint/
├── 📁 crates/
│   ├── 📁 sklint-core/
│   │   ├── Cargo.toml
│   │   └── 📁 src/
│   │       ├── analyzer.rs
│   │       ├── blank_lines.rs
│   │       ├── comments.rs
│   │       ├── config.rs
│   │       ├── diagnostic.rs
│   │       ├── docstrings.rs
│   │       ├── dynamic_attrs.rs
│   │       ├── formatter.rs
│   │       ├── rules.rs
│   │       ├── suppression.rs
│   │       └── syntax_rules.rs
│   └── 📁 sklint-cli/
│       ├── Cargo.toml
│       └── 📁 src/
│           └── main.rs
├── 📁 docs/
│   ├── README.ru.md
│   ├── examples.ru.md
│   └── rules.ru.md
├── 📁 examples/
│   ├── bad.py
│   ├── bad.pyi
│   ├── good.py
│   └── pyproject.toml
├── 📁 python/
│   └── 📁 sklint/
│       ├── __init__.py
│       └── __main__.py
├── 📁 scripts/
│   ├── check-release.sh
│   ├── check-windows.ps1
│   └── package-vscode.py
├── 📁 vscode/
│   ├── package.json
│   ├── README.ru.md
│   └── 📁 src/
│       └── extension.ts
├── Cargo.toml
├── pyproject.toml
├── setup.py
└── README.md
```

В проекте выделены следующие основные части:

**Rust-ядро**  
`sklint-core` содержит все правила, suppressions, конфигурацию, форматтер и API анализа. Это основной источник истины для CLI и VSCode.

**Консольная утилита**  
`sklint-cli` предоставляет команды `check`, `format`, `rules`, `explain` и умеет работать с `.py`, `.pyi`, папками и stdin.

**Python-пакет**  
Пакет `sklint` устанавливает тонкий Python wrapper и нативный Rust-бинарь, чтобы инструмент можно было запускать через `sklint` и `python -m sklint`.

**VSCode расширение**  
Расширение запускает Rust CLI в runtime, показывает diagnostics, quick fixes, formatter и hover-справку по `SKxxx` внутри `# noqa`. Исходники расширения хранятся в `vscode/src`, а готовый VSIX собирается в `dist/` отдельной командой.

# Полезная информация

## Установка

**Установка из исходников:**

```bash
python -m pip install .
```

**Изолированная установка как CLI:**

```bash
pipx install .
```

**Сборка wheel:**

```bash
python -m pip wheel . -w dist --no-deps
python -m pip install dist/sklint-*.whl
```

**Проверка установленной версии:**

```bash
sklint --version
python -m sklint --version
```

Во время сборки wheel запускается `cargo build -p sklint --release`, а полученный исполняемый файл кладётся внутрь Python-пакета.  
Для отладки можно собрать пакет вокруг debug-бинаря:

```bash
SKLINT_CARGO_PROFILE=debug python -m pip wheel . -w dist --no-deps
```

## Базовое использование CLI

**Проверка проекта:**

```bash
sklint check .
```

**Проверка с JSON-выводом:**

```bash
sklint check --format json examples/bad.py
```

**Проверка stdin:**

```bash
cat module.py | sklint check --format json --stdin-filename module.py -
```

**Применение безопасных исправлений:**

```bash
sklint check --fix examples/bad.py
sklint format examples/bad.py
```

**Проверка форматирования без изменения файлов:**

```bash
sklint format --check examples
```

**Просмотр правил:**

```bash
sklint rules
sklint explain SK601
```

Подробные примеры использования находятся в [`docs/examples.ru.md`](./examples.ru.md).

## Конфигурация проекта

Основная конфигурация хранится в `pyproject.toml`:

```toml
[tool.sklint]
strict = false
select = []
ignore = []
```

Настройка `strict = true` включает дополнительную группу strict-only правил.  
`select` и `ignore` работают по префиксам, в стиле Ruff: можно выбрать `SK6`, `SK601` или отключить конкретный код.

## Совместимость с Ruff

Если Ruff используется вместе со SKLint и в коде есть suppressions вида `# noqa: SK403`, нужно указать Ruff, что префикс `SK` принадлежит внешнему линтеру:

```toml
[tool.ruff.lint]
external = ["SK"]
```

В VSCode настройка задаётся через inline-конфигурацию Ruff:

```json
{
  "ruff.configuration": {
    "lint": {
      "external": ["SK"]
    }
  }
}
```

Без этой настройки Ruff может показывать `SKxxx: Rule not found` при наведении на `# noqa`.

## Сборка VSCode расширения

```bash
python scripts/package-vscode.py
```

Готовый VSIX появится в `dist/`. В release-архиве уже лежит `vscode/out/extension.js`, поэтому Node.js/npm не нужны для простой упаковки VSIX, если TypeScript-часть не менялась. Если `vscode/out/extension.js` отсутствует или был удалён, установите Node.js LTS и пересоберите расширение:

```bash
cd vscode
npm install
npm run compile
cd ..
python scripts/package-vscode.py
```

Файлы `*.vsix`, `node_modules/` и Rust `target/` не хранятся в репозитории, потому что являются артефактами сборки.

## Настройки VSCode

Минимальная конфигурация расширения:

```json
{
  "sklint.strict": false,
  "sklint.select": [],
  "sklint.ignore": [],
  "sklint.run": "onType",
  "sklint.formatting.enabled": false
}
```


### Автоопределение CLI в VSCode

Расширение не требует обязательной настройки пути к `sklint`. Оно ищет исполняемый файл в таком порядке:

1. валидный `sklint.executablePath` или устаревший alias `sklint.path`, если они явно заданы;
2. `target/release/sklint(.exe)` и `target/debug/sklint(.exe)` в текущем workspace;
3. `.venv` / `venv` внутри workspace;
4. bundled binary внутри установленного VSIX;
5. команда `sklint` из `PATH`.

Если в настройках остался старый абсолютный путь и файл больше не существует, расширение игнорирует его и продолжает автоопределение.

`pyproject.toml` имеет приоритет над fallback-настройками VSCode.  
Inline-комментарии в файле применяются последними и действуют только на текущий файл.

## Подавления предупреждений

SKLint поддерживает несколько форм suppressions:

```python
value = build_value()  # noqa: SK601, SK900
value = build_value()  # sklint: ignore SK601, SK900
value = build_value()  # pyright: ignore[reportAny]  # noqa: SK401
```

Для всего файла:

```python
# sklint: noqa
# sklint: noqa: SK601, SK900
```

Для блока:

```python
# sklint: disable=SK601


def function() -> None:
    """
    Описание с подавленным правилом
    """

# sklint: enable=SK601
```

Для докстрингов `SK6xx` suppression указывается на закрывающей строке докстринга или на последней непустой строке содержимого докстринга:

```python
def parse_value(raw: str) -> int:
    """
    Ожидание преобразования строки в число
    """  # noqa: SK601

    return int(raw)


def normalize_value(raw: str) -> str:
    """
    ожидание нормализации строки  # noqa: SK617
    """

    return raw.strip()
```

Подавление можно ставить после уже существующего служебного комментария, например после `# pyright: ignore[...]`; SKLint найдёт последующий `# noqa: SKxxx` или `# sklint: ignore SKxxx`. При наведении на `SKxxx` внутри `# noqa: SKxxx` или `# sklint: ignore SKxxx` VSCode показывает расширенную markdown-справку по правилу.

## Форматтер

Форматтер использует безопасные исправления из Rust-ядра. Он не придумывает смысл за разработчика, но может:

- удалить пробелы в конце строк;
- нормализовать пустые строки;
- переставить структурированные секции докстринга;
- переписать `Attributes` по фактическому порядку dataclass-полей;
- исправить типы `Attributes` по аннотациям;
- создать `TODO`-заготовки там, где описание необходимо заполнить вручную;
- свернуть простые strict-only конструкции, если преобразование очевидно.

В VSCode форматтер выключен по умолчанию:

```json
{
  "sklint.formatting.enabled": false
}
```

Для включения:

```json
{
  "sklint.formatting.enabled": true
}
```

## Правила

Полная документация по предупреждениям находится в [`docs/rules.ru.md`](./rules.ru.md).  
Каждое правило содержит код, уровень, наличие автофикса, описание и пример требования.

# Совместимость

Текущий релиз проверяется со следующими компонентами:

- **Windows** и **Linux/WSL**
- **Python 3.8+** для wrapper-пакета
- **Rust stable**, предоставленный в тестовом toolchain
- **VSCode** с обычным Windows workspace и WSL Remote workspace
- **Ruff**, **Pylance/Pyright** при совместном использовании

SKLint поставляется как релизный CLI, Python-пакет и VSCode-расширение. Публичные команды CLI, формат suppressions и конфигурация поддерживаются как стабильный интерфейс; новые правила и анализаторы добавляются отдельными релизами.
