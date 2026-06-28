# SKLint 0.1.42 — Unicode-safe identifier boundaries for SK801

## Исправленный блокер

В SKLint 0.1.41 границы текстового поиска идентификаторов определялись только по ASCII:

```rust
ch == '_' || ch.is_ascii_alphanumeric()
```

Из-за этого корректное имя `xя` ошибочно воспринималось как отдельное использование `x`, и safe-fix `SK801` мог создать синтаксически неверный код `get_value()я`.

## Реализация

- Добавлен общий модуль `identifier.rs` с таблицами Python-compatible `XID_Start` и `XID_Continue` для Unicode 15.1.
- `syntax_rules.rs` и `formatter.rs` используют одну и ту же Unicode-проверку границ.
- Маска кода без строк и комментариев больше не искажает UTF-8-символы побайтным преобразованием.
- Парсер простых присваиваний теперь принимает корректные Unicode-имена временных переменных.
- Внешняя зависимость не добавлялась: Rust-ядро остаётся self-contained и dependency-free.

## Регрессионные случаи

Форматтер не изменяет следующие конструкции:

```python
x = get_value()
return xя
```

```python
x = get_value()
return яx
```

```python
x = get_value()
return x́
```

При этом корректное единственное использование Unicode-временной переменной продолжает встраиваться:

```python
значение = get_value()
return значение
```

преобразуется в:

```python
return get_value()
```

## Проверки

- `cargo fmt --all -- --check`
- `cargo test --workspace`: 141 тест, 0 ошибок
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo build -p sklint --release`
- ручные Unicode-пробы с последующим `python -m py_compile`
- полный `scripts/check-release.sh`, включая wheel, VSIX, JSON smoke и Pyright boundary probes
