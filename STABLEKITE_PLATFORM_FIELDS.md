Version 0.1.55 recognizes actual inherited state declarations through proven
native-platform import routes. A class can select a real parent using literal
sys.platform or os.name guards and retain the fields declared by that parent.

The lookup reuses the proven lexical bindings and C3 member graph. An unknown
binding or opaque external parent remains a barrier. Fields from an inactive
branch are not added, and an undeclared field still produces SK701. No field,
class or filename is privileged by its spelling.

The ordinary dataclass field order and transform graph are unchanged. The
separate cross-platform question about a property that exists in only one
platform world remains open; this change does not invent missing members.
