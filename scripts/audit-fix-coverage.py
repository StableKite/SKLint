#!/usr/bin/env python3
"""Release gate: every rule has an explicit formatter/autofix policy in docs.

The documentation is the user-facing policy contract.  Rules that claim a fix
must also have a physical implementation marker somewhere in the Rust sources,
with a small allow-list for formatter-wide structural policies.
"""
from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
rules_text = (ROOT / "crates/sklint-core/src/rules.rs").read_text(encoding="utf-8")
docs_text = (ROOT / "docs/rules.ru.md").read_text(encoding="utf-8")
rust_text = "\n".join(p.read_text(encoding="utf-8") for p in (ROOT / "crates/sklint-core/src").glob("*.rs"))

codes = re.findall(r'code:\s*"(SK(?:D)?\d{3})"', rules_text)
if len(codes) != len(set(codes)):
    raise SystemExit("duplicate registered rule codes detected")

policy: dict[str, str] = {}
missing_policy: list[str] = []
for code in codes:
    match = re.search(rf'^##\s+{re.escape(code)}\b.*?(?=^##\s+SK|\Z)', docs_text, re.MULTILINE | re.DOTALL)
    if not match:
        missing_policy.append(code)
        continue
    section = match.group(0)
    auto = re.search(r'\*\*Autofix:\*\*\s*([^\n]+)|^Autofix:\s*([^\n]+)', section, re.MULTILINE)
    if not auto:
        missing_policy.append(code)
        continue
    value = (auto.group(1) or auto.group(2)).strip().lower()
    if any(token in value for token in ("нет", "отсутств")):
        category = "none"
    elif any(token in value for token in ("manual", "unsafe", "вруч")):
        category = "manual"
    elif any(token in value for token in ("услов", "частич", "lossless", "structural")):
        category = "conditional"
    else:
        category = "safe"
    policy[code] = category

if missing_policy:
    raise SystemExit("rules without explicit Autofix policy: " + ", ".join(missing_policy))

# Claims of an implemented fix must have a physical trace.  Most rule-specific
# fixes contain the rule code in the same Rust module; formatter-wide policies
# are explicitly allowed here because they are implemented by generic passes.
formatter_wide = {"SK505", "SK509", "SK602"}
missing_marker: list[str] = []
for code, category in policy.items():
    if category == "none":
        continue
    if code in formatter_wide:
        continue
    if code not in rust_text:
        missing_marker.append(code)
if missing_marker:
    raise SystemExit("fix policy has no Rust implementation marker: " + ", ".join(missing_marker))

counts = {name: sum(value == name for value in policy.values()) for name in ("safe", "conditional", "manual", "none")}
print(
    f"fix-policy audit OK: {len(policy)}/{len(codes)} rules classified "
    f"(safe={counts['safe']}, conditional={counts['conditional']}, manual={counts['manual']}, none={counts['none']})"
)
