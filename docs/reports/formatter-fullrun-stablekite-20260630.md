# SKLint formatter full run on StableKite

StableKite was used as an external test project. The library source archive was not modified.

## Formatter validation

- `sklint format .` on a clean copy of StableKite: exit 0
- `python -m compileall -q .` after formatting: exit 0
- `sklint format --check .` after formatting: exit 0
- Diagnostics before formatting with this build: 4157
- Diagnostics after formatting: 1591

## Formatter bugs fixed

1. SK804 no longer breaks files with multiline imports or annotated `__all__: Final[...] = (...)` declarations.
2. SK609 no longer inserts constant docstring templates inside multiline `Final` assignments, docstrings, or `__all__` declarations; generated templates now preserve indentation.
3. SK801 no longer offers an unsafe autofix for single-use variables used in assignment targets.
4. SK802 now preserves original source text for return expressions instead of using masked string-literal text.
5. SK403 no longer offers unsafe splitting fixes for comprehension lines and only attaches a fix when the split can be parsed safely.
6. SK502 and SK503 keep diagnostics but no longer offer unsafe import/platform rewrites.
7. Syntax masking now handles backslash-continued strings, including f-strings.
8. Blank-line parsing no longer treats `def`/`class` text inside multiline strings as real Python objects.
9. Multiline function signatures are now recognised as docstring owners, preventing closing quotes from being misread as new docstrings.

## Remaining diagnostics

Remaining diagnostics are policy diagnostics, not formatter breakages. The formatter is idempotent on the formatted StableKite copy.
