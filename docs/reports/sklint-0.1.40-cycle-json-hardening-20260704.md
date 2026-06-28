# SKLint 0.1.40 — formatter cycle and JSON hardening

## Исправленные дефекты

### Циклические зависимости SK505

Форматировщик строит граф зависимостей между top-level определениями и между методами класса. Если предполагаемая перестановка находится внутри циклической компоненты, структурный fix пропускается. Остальные безопасные исправления файла продолжают применяться.

Минимальный сценарий `build -> Box -> build` теперь сохраняет стабильный порядок и не блокирует исправления SK401, SK611 и правил пустых строк.

### Конфликты правил пустых строк

- SK301 допускает пустую строку после многострочного docstring метода, требуемую SK613.
- SK302 не удаляет пустые строки внутри дочерних определений и их docstring.
- SK303 рассматривает только непосредственные методы класса и не конфликтует с conditional overload-методами.
- Вложенные multiline strings не интерпретируются как реальные объявления `def` / `class`.

### Честный результат форматтера

После форматирования выполняется финальный анализ. `FormatReport` содержит число оставшихся безопасных исправлений.

- `sklint format --check` возвращает ненулевой код, если файл изменился бы или в нём остались неприменённые safe-fixes.
- обычный `sklint format` также не сообщает успех, если safe-fixes остались.

### Валидный JSON

При `sklint check --fix --format json` progress-сообщения `... formatted` отправляются в stderr. stdout содержит один валидный JSON-документ.

### Clippy all-targets

Release gate теперь запускает:

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Тестовые конфигурации переписаны без `field_reassign_with_default`.

## Полный прогон StableKite

Проверена внешняя копия библиотеки, исходная библиотека не изменялась.

- файлов `.py` / `.pyi`: 342;
- `sklint check --fix --format json .`: stdout успешно разобран `json.loads`;
- последующий `sklint format --check .`: exit code 0;
- после форматирования: 2197 diagnostics, из них safe-fixes — 0;
- `python -m compileall -q .`: успешно;
- функции, классы, сигнатуры, декораторы и аннотированные поля сохранены;
- новых загрузок `_` и `item_annotation` не создано;
- новых `TODO: описание` не создано.

## Регрессионные проверки

- Rust unit tests: 132 passed;
- `cargo fmt --all -- --check`: passed;
- `cargo clippy --workspace --all-targets -- -D warnings`: passed;
- `scripts/check-release.sh`: `release checks passed`;
- Python wheel: built and installed;
- VSCode TypeScript: compiled;
- VSIX: built;
- Broken pipe probe: passed;
- Pyright probes: passed.
