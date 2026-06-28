# Основная информация

#### **SKLint (`sklint`)** — быстрый линтер, форматировщик и статический анализатор Python-кода для проектных требований **StableKite**, написанный на **Rust**.  
#### Инструмент работает как самостоятельная CLI-утилита, Python-пакет и VSCode расширение.

Автор: **StableKite**  
Сайт: <https://stablekite.com>  
Email: <stablekite@stablekite.com>

# Теоретическая информация

SKLint не заменяет Ruff, Pyright, Pylint, wemake, flake8 и pydoclint.  
Его задача — добавлять только те проверки, которые нужны проекту и не закрываются существующими инструментами полностью.  
Все основные правила, suppressions, автоисправления и форматтер реализованы в Rust-ядре `sklint-core`, а CLI, Python wrapper и VSCode расширение вызывают одно и то же API.

# Общая информация о проекте

Файлы проекта имеют следующую структуру:

```text
sklint/
├── 📁 crates/          # Rust workspace: core + CLI
├── 📁 docs/            # Русская документация проекта и правил
├── 📁 examples/        # Файлы для ручной проверки и VSCode smoke test
├── 📁 python/          # Python wrapper для установки через pip/pipx
├── 📁 scripts/         # Проверочные скрипты
├── 📁 vscode/          # исходники VSCode extension
├── Cargo.toml
├── pyproject.toml
├── setup.py
└── README.md
```

**Подробная документация по проекту:** [`docs/README.ru.md`](docs/README.ru.md)  
**Примеры использования:** [`docs/examples.ru.md`](docs/examples.ru.md)  
**Документация предупреждений:** [`docs/rules.ru.md`](docs/rules.ru.md)

# Полезная информация

## Установка

```bash
python -m pip install .
sklint --version
python -m sklint --version
```

Для изолированной CLI-установки:

```bash
pipx install .
```

Сборка wheel:

```bash
python -m pip wheel . -w dist --no-deps
python -m pip install dist/sklint-*.whl
```

## CLI

```bash
sklint check examples
sklint check --format json examples/bad.py
sklint check --fix examples/bad.py
sklint format --check examples
sklint format examples/bad.py
sklint rules
sklint explain SK601
```

## Конфигурация

```toml
[tool.sklint]
strict = false
select = []
ignore = []

[tool.ruff.lint]
external = ["SK"]
```

`external = ["SK"]` нужен, если Ruff используется вместе со SKLint и в коде встречаются suppressions вида `# noqa: SKxxx`.

## VSCode

Расширение находится в папке `vscode/`. VSIX собирается в `dist/` командой `python scripts/package-vscode.py`. Release-архив содержит готовый `vscode/out/extension.js`, поэтому для упаковки VSIX Node.js/npm не нужны, пока TypeScript-часть не менялась.  
Для ручной проверки Windows, WSL, CLI, Python wheel и VSCode используется:

```powershell
.\scripts\check-windows.ps1
```

Минимальные настройки VSCode (путь к CLI обычно не нужен: расширение использует bundled binary и автоопределение):

```json
{
  "sklint.strict": false,
  "sklint.select": [],
  "sklint.ignore": [],
  "sklint.run": "onType",
  "sklint.formatting.enabled": false,
  "ruff.configuration": {
    "lint": {
      "external": ["SK"]
    }
  }
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

## Suppressions

```python
value = build_value()  # noqa: SK601, SK900
value = build_value()  # sklint: ignore SK601, SK900
value = build_value()  # pyright: ignore[reportAny]  # noqa: SK401
```

Для докстрингов `SK6xx` подавление ставится на последней строке докстринга:

```python
def parse_value(raw: str) -> int:
    """
    Ожидание преобразования строки в число
    """  # noqa: SK601

    return int(raw)
```

В VSCode расширенная markdown-справка показывается при наведении на `SKxxx` внутри `# noqa: SKxxx` или `# sklint: ignore SKxxx`.

# Совместимость

Текущий релиз проверяется для **Windows**, **Linux/WSL**, **Python 3.8+**, **Rust stable**, **VSCode**, **Ruff** и **Pylance/Pyright**.

SKLint поставляется как релизный CLI, Python-пакет и VSCode-расширение. Публичные команды CLI, формат suppressions и конфигурация поддерживаются как стабильный интерфейс; новые правила добавляются отдельными релизами.
