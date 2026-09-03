#!/usr/bin/env python3
"""Release gate for the Rust-native pydoclint compatibility surface."""
from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
rules = (ROOT / "crates/sklint-core/src/rules.rs").read_text(encoding="utf-8")
cargo = (ROOT / "crates/sklint-core/Cargo.toml").read_text(encoding="utf-8")
pydoclint = (ROOT / "crates/sklint-core/src/pydoclint.rs").read_text(encoding="utf-8")

upstream = {
    "001", "002", "003",
    "101", "102", "103", "104", "105", "106", "107", "108", "109", "110", "111",
    "201", "202", "203",
    "301", "302", "303", "304", "305", "306", "307",
    "402", "403", "404", "405",
    "501", "502", "503", "504",
    "601", "602", "603", "604", "605", "606", "607",
}
registered = set(re.findall(r'code:\s*"SKD(\d{3})"', rules))
missing = sorted(upstream - registered)
extra_upstream_range = sorted((registered - upstream) - {"608"})
if missing:
    raise SystemExit("missing upstream DOC families: " + ", ".join("DOC" + x for x in missing))
if extra_upstream_range:
    raise SystemExit("unexpected SKD families: " + ", ".join("SKD" + x for x in extra_upstream_range))
if "608" not in registered:
    raise SystemExit("SKD608 extension is missing")
if "401" in registered:
    raise SystemExit("deprecated DOC401/SKD401 must not be registered")
if "rustpython-parser" not in cargo:
    raise SystemExit("RustPython parser dependency is missing")
if "ast::Suite" not in pydoclint and "PythonAst" not in pydoclint:
    raise SystemExit("pydoclint port is not visibly backed by the Rust Python AST layer")
if not (ROOT / "scripts/pydoclint-diff.py").is_file():
    raise SystemExit("pydoclint differential harness is missing")

print(f"pydoclint-port audit OK: {len(upstream)}/{len(upstream)} upstream DOC families + SKD608")
