Version 0.1.54 resolves raised exception values from proven inherited property
annotations. A guarded `raise self.failure` can therefore match `Raises: OSError`
when `failure` is an `OSError | None` property. A mismatched `Raises: ValueError`
still produces SKD503. Property and local-variable names have no special meaning.

The analysis uses parsed Python syntax, declared builtin exception types, complete
C3 lookup across project-local imports/re-exports, simple None guards, early
returns, and local aliases. It supports union, Optional, and quoted annotations.
Known value types are kept separate from unresolved names before Raises matching.
An unrelated parameter named like an exception cannot reinterpret a proven type.

Explicit unknown overrides, shadowed property decorators or type names, uncertain
imports, reassigned receivers/aliases, and unsupported control flow retain normal
diagnostics. An unchecked nullable property is not silently treated as non-null.
The new inference does not invent types for Any, object or unknown custom classes.
Existing parameter, class-annotation, exception-handler and factory behavior is
retained; the WinError factory checks introduced in 0.1.53 remain enabled.

For the property graph, literal sys.platform and os.name guards use the actual
native analyzer platform when their module binding is proven. No target is guessed
from filenames or property names. Unknown/shadowed conditions remain unresolved.
This graph is separate from the existing dataclass-field import graph.

Complete project models are re-resolved when requested. Imported parse trees keep
their existing content-hash cache, so editing a parent's property annotation cannot
leave a stale exception type in an otherwise unchanged child.
