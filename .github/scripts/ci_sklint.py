from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[2]
EXCLUDED_DIRS = {
    ".git",
    "target",
    "build",
    "dist",
    "candidate-dist",
    "release-dist",
    "wheelhouse",
    "__pycache__",
    "node_modules",
    ".pytest_cache",
    ".venv",
}


def canonical_version() -> str:
    with (ROOT / "pyproject.toml").open("rb") as f:
        return tomllib.load(f)["project"]["version"]


def all_versions() -> dict[str, str]:
    with (ROOT / "Cargo.toml").open("rb") as f:
        cargo = tomllib.load(f)["workspace"]["package"]["version"]
    init_text = (ROOT / "python" / "sklint" / "__init__.py").read_text(encoding="utf-8")
    match = re.search(r'^__version__\s*=\s*"([^"]+)"', init_text, re.MULTILINE)
    if not match:
        raise SystemExit("Could not read python/sklint/__init__.py version")
    vscode = json.loads((ROOT / "vscode" / "package.json").read_text(encoding="utf-8"))["version"]
    return {
        "pyproject": canonical_version(),
        "cargo": cargo,
        "python_package": match.group(1),
        "vscode": vscode,
    }


def verify_versions() -> None:
    versions = all_versions()
    expected = os.environ.get("PACKAGE_VERSION", versions["pyproject"])
    print("version_state=" + json.dumps(versions, sort_keys=True))
    bad = {name: value for name, value in versions.items() if value != expected}
    if bad:
        raise SystemExit(f"Version mismatch; expected {expected}: {bad}")
    print(f"SKLint version consistency: PASS ({expected})")


def source_manifest(output: Path) -> str:
    rows: list[str] = []
    for path in ROOT.rglob("*"):
        if not path.is_file():
            continue
        rel = path.relative_to(ROOT)
        if any(part in EXCLUDED_DIRS or part.endswith(".egg-info") for part in rel.parts):
            continue
        rel_posix = rel.as_posix()
        if rel_posix.startswith("docs/reports/") or rel_posix.startswith("vscode/out/"):
            continue
        data = path.read_bytes()
        rows.append(f"{rel_posix}|{len(data)}|{hashlib.sha256(data).hexdigest()}")
    rows.sort()
    text = "\n".join(rows) + "\n"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(text, encoding="utf-8", newline="\n")
    digest = hashlib.sha256(text.encode("utf-8")).hexdigest()
    print(f"canonical_files={len(rows)}", file=sys.stderr)
    print(f"source_tree_sha256={digest}", file=sys.stderr)
    return digest


def verify_provenance(binary: Path, expected_target: str) -> None:
    version = canonical_version()
    revision = os.environ.get("SKLINT_BUILD_REVISION", f"v{version}")
    tree_sha = os.environ["SKLINT_SOURCE_TREE_SHA256"]
    commit = os.environ["SKLINT_SOURCE_COMMIT"]
    expected_rustc = os.environ.get("EXPECTED_RUSTC")
    out = subprocess.check_output([str(binary), "--version", "--verbose"], text=True)
    print(out, end="")
    required = {
        f"sklint {version}",
        f"revision: {revision}",
        f"source-tree-sha256: {tree_sha}",
        f"source-commit: {commit}",
        "source-dirty: false",
        f"target: {expected_target}",
    }
    if expected_rustc:
        required.add(f"rustc: {expected_rustc}")
    lines = {line.strip() for line in out.splitlines()}
    missing = sorted(required - lines)
    if missing:
        raise SystemExit(f"Missing provenance lines: {missing}")
    print("SKLint provenance: PASS")


def expected_wheel_name(platform_tag: str) -> str:
    return f"sklint-{canonical_version()}-py3-none-{platform_tag}.whl"


def verify_dist(directory: Path, platform_tag: str, expect_sdist: bool) -> None:
    expected = {expected_wheel_name(platform_tag)}
    if expect_sdist:
        expected.add(f"sklint-{canonical_version()}.tar.gz")
    actual = {p.name for p in directory.iterdir() if p.is_file()}
    print("dist_actual=" + json.dumps(sorted(actual)))
    print("dist_expected=" + json.dumps(sorted(expected)))
    if actual != expected:
        raise SystemExit(f"Unexpected distribution set: {sorted(actual)}")
    print("SKLint distribution set: PASS")


def verify_publish_set(directory: Path) -> None:
    version = canonical_version()
    expected = {
        f"sklint-{version}.tar.gz",
        f"sklint-{version}-py3-none-win_amd64.whl",
        f"sklint-{version}-py3-none-manylinux_2_35_x86_64.whl",
        f"sklint-{version}-py3-none-manylinux_2_35_aarch64.whl",
        f"sklint-{version}-py3-none-musllinux_1_2_x86_64.whl",
        f"sklint-{version}-py3-none-musllinux_1_2_aarch64.whl",
    }
    actual = {p.name for p in directory.iterdir() if p.is_file()}
    print("publish_actual=" + json.dumps(sorted(actual)))
    print("publish_expected=" + json.dumps(sorted(expected)))
    if actual != expected:
        raise SystemExit(f"Unexpected publish set: {sorted(actual)}")
    print("SKLint exact publish set: PASS")


def smoke() -> None:
    version = canonical_version()
    exe = Path(sys.executable).with_name("sklint.exe" if os.name == "nt" else "sklint")
    if not exe.is_file():
        raise SystemExit(f"Installed sklint executable not found: {exe}")
    subprocess.run([str(exe), "--version"], check=True)
    subprocess.run([sys.executable, "-m", "sklint", "--version"], check=True)
    subprocess.run([str(exe), "check", str(ROOT / "examples" / "good.py")], check=True)
    bad = subprocess.run([str(exe), "check", str(ROOT / "examples" / "bad.py")])
    if bad.returncode == 0:
        raise SystemExit("Expected examples/bad.py to produce diagnostics")
    verbose = subprocess.check_output([str(exe), "--version", "--verbose"], text=True)
    if f"sklint {version}" not in verbose:
        raise SystemExit("Installed binary reports wrong version")
    print("SKLint installed-package smoke: PASS")


def main() -> int:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("verify-versions")
    p_manifest = sub.add_parser("source-manifest")
    p_manifest.add_argument("--output", required=True, type=Path)
    p_prov = sub.add_parser("verify-provenance")
    p_prov.add_argument("--binary", required=True, type=Path)
    p_prov.add_argument("--target", required=True)
    p_dist = sub.add_parser("verify-dist")
    p_dist.add_argument("--dir", required=True, type=Path)
    p_dist.add_argument("--platform-tag", required=True)
    p_dist.add_argument("--expect-sdist", action="store_true")
    p_pub = sub.add_parser("verify-publish-set")
    p_pub.add_argument("--dir", required=True, type=Path)
    sub.add_parser("smoke")
    args = parser.parse_args()

    if args.cmd == "verify-versions":
        verify_versions()
    elif args.cmd == "source-manifest":
        print(source_manifest(args.output))
    elif args.cmd == "verify-provenance":
        verify_provenance(args.binary, args.target)
    elif args.cmd == "verify-dist":
        verify_dist(args.dir, args.platform_tag, args.expect_sdist)
    elif args.cmd == "verify-publish-set":
        verify_publish_set(args.dir)
    elif args.cmd == "smoke":
        smoke()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
