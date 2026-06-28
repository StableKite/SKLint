# Минимальные примеры использования

## 1. Проверка файла из консоли

```bash
sklint check examples/bad.py
```

Пример вывода:

```text
examples/bad.py:10:3: SK211 Cyrillic comment sentences must start with an uppercase letter
examples/bad.py:10:32: SK212 Comments must not end with a period
```

В консоли SKLint выводит английские сообщения, чтобы CLI оставался одинаковым в CI, pre-commit, pipx и локальных терминалах.  
Русские сообщения используются только в VSCode, если язык интерфейса VSCode русский.

## 2. Получение JSON-диагностик

```bash
sklint check --format json examples/bad.py
```

JSON-режим используется VSCode расширением и подходит для интеграции с другими инструментами:

```json
{
  "diagnostics": [
    {
      "code": "SK001",
      "message": "Trailing whitespace",
      "path": "examples/bad.py",
      "line": 1,
      "column": 10,
      "level": "information"
    }
  ]
}
```

## 3. Проверка stdin

```bash
cat examples/bad.py | sklint check --format json --stdin-filename examples/bad.py -
```

`--stdin-filename` нужен для поиска `pyproject.toml`, выбора правил по расширению файла и корректного отображения пути в diagnostics.

## 4. Применение автоисправлений

```bash
sklint check --fix examples/bad.py
```

Команда применяет безопасные исправления и после этого снова выводит оставшиеся предупреждения.  
Если часть проблем требует человеческого текста или смыслового решения, она остаётся в diagnostics.

## 5. Форматирование проекта

```bash
sklint format examples
```

Проверить, изменились бы файлы, но не записывать результат:

```bash
sklint format --check examples
```

## 6. Установка как Python-пакет

```bash
python -m pip install .
sklint --version
python -m sklint --version
```

Для установки как отдельной CLI-утилиты:

```bash
pipx install .
```

## 7. Конфигурация `pyproject.toml`

```toml
[tool.sklint]
strict = false
select = ["SK6"]
ignore = ["SK900"]

[tool.ruff.lint]
external = ["SK"]
```

Если `[tool.sklint]` отсутствует, упаковочный `pyproject.toml` не считается конфигурацией SKLint и не перебивает VSCode fallback-настройки.

## 8. Inline-конфигурация в файле

```python
# sklint: strict
# sklint: select=SK601,SK619; ignore=SK900
```

Такие директивы действуют только на текущий файл.

## 9. Подавление одной строки

```python
print("debug")  # noqa: SK201
```

При наведении в VSCode на `SK201` внутри `# noqa: SK201` откроется markdown-справка по правилу.

## 10. Подавление докстринга

```python
def parse_value(raw: str) -> int:
    """
    Ожидание преобразования строки в число
    """  # noqa: SK601

    return int(raw)
```

Для правил `SK6xx` подавление указывается на последней строке докстринга, а не на строке фактической подсветки.

## 11. Проверка `.pyi`

```bash
sklint check examples/bad.pyi
```

SKLint анализирует `.py` и `.pyi` одинаково, но отдельные правила могут учитывать stub-структуры.

## 12. VSCode

Обычная проверка проекта выполняется скриптом:

```powershell
.\scripts\check-windows.ps1
```

Скрипт собирает Rust CLI, проверяет тесты, собирает Python wheel, компилирует VSCode extension, создаёт VSIX в `dist/`, устанавливает его и открывает `examples/bad.py` в Windows и WSL VSCode.


## 11. Подавление после другого служебного комментария

```python
value=1  # pyright: ignore[reportAny]  # noqa: SK401
```

SKLint читает `# noqa: SK401` даже если перед ним уже стоит другой комментарий.
