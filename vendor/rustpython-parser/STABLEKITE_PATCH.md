# Private PEP 758 grammar patch

This is rustpython-parser 0.4.0 from the checksummed crates.io archive, with
private version 0.4.0+stablekite.1. The only code changes are the two no-as
except grammar productions: they use GenericList<Test<"all">> instead of
a single Test. The as productions are unchanged. The default lalrpop feature
regenerates the parser from the actual grammar during every ordinary build.
No source rewriting, syntax-oracle bypass, SKLint rule change, or formatter
algorithm change implements this feature. Original upstream license and
other source files are retained.
