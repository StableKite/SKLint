#!/usr/bin/env python3
"""Differentially compare pydoclint DOC diagnostics with SKLint SKD rules.

The comparison intentionally focuses on code + source line semantics. Message
text differs because SKLint owns its diagnostics, while the DOC/SKD code
mapping is expected to remain one-to-one for upstream-compatible behavior.
Intentional SKLint extensions can be whitelisted explicitly.
"""

from __future__ import annotations

import argparse
import collections
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
from typing import Iterable

UPSTREAM_RE = re.compile(r"^(.*):(\d+):\s+(DOC\d{3}):", re.MULTILINE)


def parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("paths", nargs="+", type=Path, help="Python files/directories to compare")
    p.add_argument("--style", choices=("numpy", "google", "sphinx"), default="numpy")
    p.add_argument("--sklint", default="target/debug/sklint", help="SKLint executable")
    p.add_argument("--pydoclint", default="pydoclint", help="pydoclint executable")
    p.add_argument(
        "--upstream-arg",
        action="append",
        default=[],
        help="Additional raw pydoclint CLI argument; repeat as needed",
    )
    p.add_argument(
        "--sklint-arg",
        action="append",
        default=[],
        help="Additional raw SKLint check argument; repeat as needed",
    )
    p.add_argument(
        "--allow-file",
        type=Path,
        help=(
            "JSON whitelist for intentional differences. Expected shape: "
            "[{\"path\":...,\"line\":N,\"code\":\"SKDxxx\"}, ...]"
        ),
    )
    p.add_argument("--show-messages", action="store_true", help="Print raw tool output on mismatch")
    return p


def run(command: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
        check=False,
    )


def norm_path(value: str) -> str:
    value = value.strip().strip('"')
    try:
        return Path(value).resolve().as_posix()
    except OSError:
        return Path(value).as_posix()


def map_doc(code: str) -> str:
    return "SKD" + code[3:]


def upstream_diagnostics(text: str) -> collections.Counter[tuple[str, int, str]]:
    counter: collections.Counter[tuple[str, int, str]] = collections.Counter()
    for match in UPSTREAM_RE.finditer(text):
        counter[(norm_path(match.group(1)), int(match.group(2)), map_doc(match.group(3)))] += 1
    return counter


def sklint_diagnostics(text: str) -> collections.Counter[tuple[str, int, str]]:
    payload = json.loads(text or "{}")
    counter: collections.Counter[tuple[str, int, str]] = collections.Counter()
    for diag in payload.get("diagnostics", []):
        code = str(diag.get("code", ""))
        if not code.startswith("SKD"):
            continue
        counter[(norm_path(str(diag.get("path", ""))), int(diag.get("line", 0)), code)] += 1
    return counter


def load_allow(path: Path | None) -> collections.Counter[tuple[str, int, str]]:
    allowed: collections.Counter[tuple[str, int, str]] = collections.Counter()
    if path is None:
        return allowed
    data = json.loads(path.read_text(encoding="utf-8"))
    if isinstance(data, dict):
        data = data.get("allow", [])
    for item in data:
        allowed[(norm_path(str(item["path"])), int(item["line"]), str(item["code"]))] += int(
            item.get("count", 1)
        )
    return allowed


def subtract_allowed(
    delta: collections.Counter[tuple[str, int, str]],
    allowed: collections.Counter[tuple[str, int, str]],
) -> collections.Counter[tuple[str, int, str]]:
    result = delta.copy()
    for key, count in allowed.items():
        if key in result:
            result[key] = max(0, result[key] - count)
            if result[key] == 0:
                del result[key]
    return result


def print_counter(title: str, counter: collections.Counter[tuple[str, int, str]]) -> None:
    if not counter:
        return
    print(title)
    for (path, line, code), count in sorted(counter.items()):
        suffix = f" x{count}" if count != 1 else ""
        print(f"  {path}:{line}: {code}{suffix}")


def main(argv: Iterable[str] | None = None) -> int:
    args = parser().parse_args(argv)
    paths = [str(path) for path in args.paths]

    upstream_cmd = [
        args.pydoclint,
        "--quiet",
        f"--style={args.style}",
        "--show-filenames-in-every-violation-message=True",
        *args.upstream_arg,
        *paths,
    ]
    sklint_cmd = [
        args.sklint,
        "check",
        "--format",
        "json",
        "--vscode-strict",
        "true",
        f"--style={args.style}",
        *args.sklint_arg,
        *paths,
    ]

    upstream = run(upstream_cmd)
    if upstream.returncode not in (0, 1):
        print("pydoclint invocation failed:", shlex.join(upstream_cmd), file=sys.stderr)
        print(upstream.stdout, file=sys.stderr)
        print(upstream.stderr, file=sys.stderr)
        return 2

    sklint = run(sklint_cmd)
    if sklint.returncode not in (0, 1):
        print("SKLint invocation failed:", shlex.join(sklint_cmd), file=sys.stderr)
        print(sklint.stdout, file=sys.stderr)
        print(sklint.stderr, file=sys.stderr)
        return 2

    upstream_text = upstream.stdout + "\n" + upstream.stderr
    try:
        expected = upstream_diagnostics(upstream_text)
        actual = sklint_diagnostics(sklint.stdout)
    except (ValueError, TypeError, json.JSONDecodeError) as exc:
        print(f"Unable to parse tool output: {exc}", file=sys.stderr)
        if args.show_messages:
            print("--- pydoclint ---", file=sys.stderr)
            print(upstream_text, file=sys.stderr)
            print("--- SKLint ---", file=sys.stderr)
            print(sklint.stdout, file=sys.stderr)
            print(sklint.stderr, file=sys.stderr)
        return 2

    missing = expected - actual
    extra = actual - expected
    allowed = load_allow(args.allow_file)
    missing = subtract_allowed(missing, allowed)
    extra = subtract_allowed(extra, allowed)

    if not missing and not extra:
        print(
            f"pydoclint parity OK: {sum(expected.values())} DOC/SKD diagnostics "
            f"across {len(paths)} path(s)"
        )
        return 0

    print_counter("Missing from SKLint:", missing)
    print_counter("Extra in SKLint:", extra)
    if args.show_messages:
        print("\n--- pydoclint raw output ---")
        print(upstream_text)
        print("--- SKLint raw output ---")
        print(sklint.stdout)
        if sklint.stderr:
            print("--- SKLint stderr ---")
            print(sklint.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
