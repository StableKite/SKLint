# Standard-library exception factories

`ctypes.WinError()` constructs an `OSError` instance (often a more specific
subclass such as `PermissionError`). A truthful `Raises: OSError` section
previously produced SKD503 because the checker compared the factory's name
with the exception class.

The Python AST layer now refines a raised call only when lexical imports and
unambiguous aliases establish its identity as `ctypes.WinError`. Parameters,
conflicting definitions, assignments, deletion, loop/context/handler/pattern
bindings and explicit module attribute mutations prevent that inference.
Nested functions capture their enclosing function bindings; methods do not
capture class attributes. Unknown or conflicting bindings retain the existing
diagnostics. This is a static binding model, not execution of consumer imports.

The ordinary function-facts API remains available. The documentation rules
retain their required Raises sections and still reject an incorrect exception
class. The AST visitor feature uses the same locked RustPython version and
adds no dependency version changes.

The regression genuinely failed before the fix. The Windows private source
passed all 498 workspace tests, strict Clippy, rustfmt and the original source
audits. A normal release binary passed the three original sk examples with
their unchanged complete configuration: the truthful factory case is accepted,
the wrong `ValueError` documentation remains rejected, and every unrelated
diagnostic is unchanged. The native Windows error objects are unchanged.

Version 0.1.53 distinguishes this source from the delivered 0.1.52 package.
The review workflow keeps every original platform and publication validation
gate. Local qualification is not a completed remote matrix, default delivery
or package-index release. Root SDK files are not modified.

Reference: [Python ctypes.WinError documentation](https://docs.python.org/3.15/library/ctypes.html#ctypes.WinError).
