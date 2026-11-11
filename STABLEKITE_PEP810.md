PEP 810 imports use the real ranged lexer and module parser. The grammar adapter
consumes the soft `lazy` modifier token and passes every other token unchanged to
RustPython. The complete AST retains aliases, expressions, nested statements and
original byte offsets. `PythonAst.lazy_imports` records the modifier and full
import ranges; `is_lazy_import` exposes this metadata without converting source
text or changing the user's import execution behavior.

The adapter validates the actual Python 3.15 module-scope restrictions, including
functions, classes, all try regions, wildcard imports and future imports. Ordinary
identifiers, strings, comments and continued or parenthesized imports retain
their meaning. The existing static rules analyze the complete tree, so numeric
and other negative controls remain active. Errors after a lazy import retain
their actual lexer/parser offsets, and lexical failure terminates token reading.

The Python launcher defaults `SKLINT_PYTHON` to its actual `sys.executable` and
preserves an explicit override. The external syntax oracle compiles without
executing source, including the compiler's context checks. Native parsing of
supported PEP 810 syntax requires no external oracle. Other valid grammar not
understood by the embedded parser continues to report its explicit AST status.

Public regressions use their own minimal strict configuration. Installed-package
smoke tests cover eager/lazy positive and numeric negative inputs, normal oracle
discovery with an empty PATH, and an explicitly absent oracle as a failure
control. No interpreter version, binary identity, diagnostic or test deadline is
fabricated or suppressed. The distribution version is `0.1.56`.
