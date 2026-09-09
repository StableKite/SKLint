# Основная информация

#### **SKLint (`sklint`)** — быстрый линтер, форматировщик и статический анализатор Python-кода для проектных требований **StableKite**, написанный на **Rust**.  
#### Инструмент работает как самостоятельная CLI-утилита, Python-пакет и VSCode расширение.

Автор: **StableKite**  
Сайт: <https://stablekite.com>  
Email: <stablekite@stablekite.com>

# Теоретическая информация

SKLint не заменяет Ruff, Pyright, Pylint, wemake или flake8. Встроенный Rust-native docstring semantic analyzer заменяет standalone pydoclint; legacy pydoclint-compatible names сохраняются только для migration compatibility.
Для редких future-grammar gaps внешний CPython используется только как изолированный syntax oracle (`-I -S`) с жёстким timeout; конкретный interpreter можно закрепить через `SKLINT_PYTHON`.
Его задача — добавлять только те проверки, которые нужны проекту и не закрываются существующими инструментами полностью.  
Все основные правила, suppressions, автоисправления и форматтер реализованы в Rust-ядре `sklint-core`, а CLI, Python wrapper и VSCode расширение вызывают одно и то же API.

> Linux release note: release-бинарники собираются на Ubuntu 22.04 и проверяются на ceiling `GLIBC_2.35`. Wheel можно маркировать `manylinux_2_35_x86_64` только после такой же проверки symbol ceiling; более новый host сам по себе не даёт права на этот tag.

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
# Python `assert`, unittest-style assertions and conventional `assert_*` helpers
# are built-in oracle contexts. Project-specific helpers support `*` wildcards:
# assertion_helpers = ["verify", "verify_*", "expect_*"]
# Additional explicit exception-boundary functions for SK506.
# exception_boundary_functions = ["_rollback_*", "probe_*"]
# Docstring semantics are first-class SKLint options. Legacy
# [tool.pydoclint] / [tool.sklint.pydoclint] sections are migration aliases.
allow_init_docstring = true
should_document_private_class_attributes = false
# Optional for wrapper-heavy projects that intentionally document exceptions
# propagated by callees even when SKLint cannot prove the callee flow.
allow_documented_propagated_exceptions = false

# `ctypes.Structure`/`Union`: explicit Python annotations win over raw `_fields_`
# storage types; ambiguous storage-only SKD605 diagnostics are never safe-fixed.

[tool.ruff.lint]
external = ["SK"]
```

`external = ["SK"]` нужен, если Ruff используется вместе со SKLint и в коде встречаются suppressions вида `# noqa: SKxxx`.

`SK901` считает непосредственный числовой literal именованного keyword argument самодокументируемым (`timeout=0.2`), но продолжает анализировать числа внутри выражения (`timeout=BASE_TIMEOUT * 2`). Python `assert`, стандартные assertion-методы `unittest`/`unittest.mock`, обычные `assert_*` и helpers из `assertion_helpers` считаются oracle-context; для method-style assertions учитывается terminal callable name независимо от формы receiver (`mock.assert_called_with(...)`, `mocks["x"].assert_called_with(...)`, `factory().assert_called_with(...)` эквивалентны): прямые expected literals, expected-data containers и арифметика, непосредственно описывающая ожидаемое значение, не требуют фиктивных констант. Oracle-context не распространяется сквозь вложенный обычный runtime-вызов: в `verify_equal(result, runtime_call(timeout=123))` аргументы `runtime_call` снова проверяются по обычной call-context policy. Поэтому `sleep(0.2)`, `range(40)`, `retry(5)`, `int(bits, 2)`, `round(value, 3)` и `timeout=BASE_TIMEOUT * 2` внутри runtime calls остаются под SK901. Это намеренная context-sensitive oracle policy, а не test-wide exemption.

`SK506` по-прежнему запрещает `try`/`except` в обычном runtime/hot path, но разрешает явную обработку исключений в lifecycle/cleanup boundaries (`close`, `shutdown`, `cleanup`, `teardown`, `rollback`, `release`, `__exit__`, `__aexit__`) и в приватных helpers, используемых только такими boundaries. Дополнительные project-specific границы задаются через `exception_boundary_functions`. `SK510` при этом не ослабляется: `contextlib.suppress(...)` остаётся запрещённым в strict mode.

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

В strict-режиме глобальные подавления в прологе файла запрещены правилом `SK805`. Это касается не только SKLint, но и распространённых директив других анализаторов: `# ruff: noqa`, `# flake8: noqa`, `# pylint: disable=...`, `# pyright: report...=false`, `# type: ignore`, `# mypy: ignore-errors` и аналогичных file-wide suppressions. Локальные подавления на конкретных строках кода остаются допустимыми.

Для `SK901` есть узкое statement-scope расширение обычного локального suppression: если явный selector `SK901` стоит на первой или закрывающей строке многострочного simple statement, он подавляет `SK901` diagnostics внутри только этого statement. Это предназначено для намеренных test/data vectors и fixtures. Suppression на средней continuation-строке остаётся line-local, а `def`/`class`/`if`/`for`/`with`/`try`/`match` не становятся function/block scope. Selector считается использованным для `SK900` только если реально подавлен хотя бы один `SK901`.

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
