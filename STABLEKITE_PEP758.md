# Python 3.14 exception lists on every supported analyzer host

SKLint 0.1.57 accepts PEP 758 exception expression lists in both except
and except* clauses, with or without tuple parentheses. This is implemented
in the embedded Rust grammar and does not depend on the host Python parser.

The checksummed crates.io rustpython-parser 0.4.0 source is retained under
vendor/rustpython-parser, with private component version
0.4.0+stablekite.1. Only the two exception productions without an as clause
change from Test to GenericList<Test>. The as productions remain unchanged.
The default lalrpop feature regenerates the real parser from this grammar
during an ordinary Cargo or Python package build. The original MIT license,
upstream source identity and patch description are included with the vendor.

The resulting exception type is a complete ExprTuple with original source
byte ranges and ordinary expression children. This handles conditional and
lambda expressions, subscripts, continuations and trailing commas through
the real expression grammar. No token substitution or source rewriting
implements PEP 758, and no external Python syntax oracle is bypassed.

The existing PEP 810 import parser, semantic rules, diagnostic policy,
formatter algorithms and all 512 pre-existing tests remain unchanged.
Seven additional tests cover full AST and UTF-8 ranges, grammar and
as-clause negatives, composition with PEP 810 and PEP 695, unchanged semantic
diagnostics, formatter fixed points, and refusal to change invalid code.
Diagnostic comparisons account for the two missing parentheses in the
original source range while retaining every diagnostic and fix.

Unparenthesized multiple types followed by as remain syntax errors.
The analyzer continues to report all other existing style and semantic
diagnostics. Recognizing valid grammar does not disable any rule.
