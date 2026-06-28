# SKLint для VSCode

#### Расширение **SKLint** подключает Rust CLI к VSCode и показывает предупреждения, быстрые исправления, formatter и hover-справку по `SKxxx` suppressions.  
#### Основная логика находится в Rust-ядре, TypeScript-слой остаётся минимальным.

## Полезная информация

- Подробная документация проекта: [`../docs/README.ru.md`](../docs/README.ru.md)
- Примеры использования: [`../docs/examples.ru.md`](../docs/examples.ru.md)
- Правила и автофиксы: [`../docs/rules.ru.md`](../docs/rules.ru.md)

## Минимальные настройки

### Автоопределение CLI

Обычно путь к `sklint` настраивать не нужно. Расширение использует bundled binary из VSIX, workspace `target/release` или `target/debug`, виртуальное окружение и `PATH`. Старый несуществующий `sklint.executablePath` игнорируется.


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

## Проверка расширения

Для полной проверки Windows CLI, WSL CLI, Python wheel и VSCode окон используется:

```powershell
.\scripts\check-windows.ps1
```

Скрипт сначала собирает VSIX в `dist/` через `scripts/package-vscode.py`, затем устанавливает его через `code --install-extension --force` и открывает `examples/bad.py` для ручной проверки diagnostics, quick fixes и suppressions hover.
