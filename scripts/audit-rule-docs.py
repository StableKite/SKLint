#!/usr/bin/env python3
"""Release gate: every registered SKLint rule must have a docs section."""
from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RULES = ROOT / "crates/sklint-core/src/rules.rs"
DOCS = ROOT / "docs/rules.ru.md"

registered = re.findall(r'code:\s*"(SK(?:D)?\d{3})"', RULES.read_text(encoding="utf-8"))
headings = re.findall(r'^##\s+(SK(?:D)?\d{3})\b', DOCS.read_text(encoding="utf-8"), re.MULTILINE)

if len(registered) != len(set(registered)):
    raise SystemExit("duplicate registered rule codes detected")
if len(headings) != len(set(headings)):
    raise SystemExit("duplicate rule documentation headings detected")

missing = sorted(set(registered) - set(headings))
unknown = sorted(set(headings) - set(registered))
if missing or unknown:
    if missing:
        print("missing docs:", ", ".join(missing))
    if unknown:
        print("unknown docs headings:", ", ".join(unknown))
    raise SystemExit(1)

print(f"rule-doc audit OK: {len(registered)}/{len(registered)} registered rules documented")
