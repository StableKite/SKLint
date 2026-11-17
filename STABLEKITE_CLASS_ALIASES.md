Version 0.1.60 follows proven name-to-name module assignments when resolving inherited
attributes. Public aliases such as `PublicOwner = _Owner`, including imported
and annotated assignment chains, retain the original class's declarations.
Requested re-exports are followed even when their module defines other classes.

For example, an inherited declaration of `_event_closing: bool` remains visible
when a child imports the parent's public alias. Assigning to that existing
attribute in a regular child method does not produce SK701.

The model resolves an alias only after its source class or import has been
bound. Rebinding, deletion, unknown conditional bindings, star imports and
attribute mutation leave the affected binding unproven. Such bindings retain
the ordinary diagnostics. This conservative rule also applies when an alias's
source name is reassigned later. Import cycles stop without selecting an
unproven ancestor.

Imported modules are parsed without execution. Changes to an imported alias
are reflected by the next analysis through the existing content-based module
cache. Rule configuration and formatter output remain unchanged.
