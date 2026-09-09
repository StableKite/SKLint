#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="${SKLINT_BUILD_VERSION:-0.1.50}"
REVISION="${SKLINT_BUILD_REVISION:-0.1.50-candidate.10}"
EXPECTED_RUSTC="rustc 1.98.0 (88d9e12ae 2026-08-18)"
OUT_DIR="${SKLINT_CANDIDATE_OUT_DIR:-$ROOT/candidate-dist}"
MANIFEST="$OUT_DIR/SOURCE_MANIFEST.txt"
STATE="$OUT_DIR/CANDIDATE_STATE.txt"

mkdir -p "$OUT_DIR"
rm -rf "$ROOT/build" "$ROOT/dist" "$ROOT/python/sklint.egg-info"

command -v rustc >/dev/null
command -v cargo >/dev/null
command -v readelf >/dev/null
command -v python >/dev/null

ACTUAL_RUSTC="$(rustc --version)"
if [[ "$ACTUAL_RUSTC" != "$EXPECTED_RUSTC" ]]; then
    echo "ERROR: expected $EXPECTED_RUSTC, got $ACTUAL_RUSTC" >&2
    exit 1
fi

python - "$ROOT" "$MANIFEST" <<'PY'
from __future__ import annotations

import hashlib
import sys
from pathlib import Path

root = Path(sys.argv[1]).resolve()
out = Path(sys.argv[2]).resolve()
excluded_dirs = {".git", "target", "build", "dist", "candidate-dist", "__pycache__"}
rows: list[str] = []
for path in root.rglob("*"):
    if not path.is_file():
        continue
    rel = path.relative_to(root)
    if any(part in excluded_dirs or part.endswith(".egg-info") for part in rel.parts):
        continue
    data = path.read_bytes()
    rows.append(f"{rel.as_posix()}|{len(data)}|{hashlib.sha256(data).hexdigest()}")
rows.sort()
text = "\n".join(rows) + "\n"
out.write_text(text, encoding="utf-8", newline="\n")
print(hashlib.sha256(text.encode("utf-8")).hexdigest())
PY

TREE_SHA="$(python - "$MANIFEST" <<'PY'
import hashlib, sys
from pathlib import Path
p = Path(sys.argv[1])
print(hashlib.sha256(p.read_bytes()).hexdigest())
PY
)"

export SKLINT_BUILD_VERSION="$VERSION"
export SKLINT_BUILD_REVISION="$REVISION"
export SKLINT_SOURCE_TREE_SHA256="$TREE_SHA"
export SKLINT_SOURCE_COMMIT="${SKLINT_SOURCE_COMMIT:-tree-sha256:$TREE_SHA}"
export SKLINT_SOURCE_DIRTY="${SKLINT_SOURCE_DIRTY:-false}"
export SKLINT_CARGO_PROFILE=release

python scripts/audit-rule-docs.py
python scripts/audit-fix-coverage.py
python scripts/audit-pydoclint-port.py
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release --locked -p sklint

BIN="$ROOT/target/release/sklint"
if [[ ! -x "$BIN" ]]; then
    echo "ERROR: release binary not found: $BIN" >&2
    exit 1
fi

MAX_GLIBC="$(readelf --version-info "$BIN" \
    | grep -oE 'GLIBC_[0-9]+(\.[0-9]+)*' \
    | sort -Vu \
    | tail -n1 || true)"

version_le() {
    local left="$1" right="$2"
    [[ "$(printf '%s\n%s\n' "$left" "$right" | sort -V | head -n1)" == "$left" ]]
}

if [[ -n "$MAX_GLIBC" ]]; then
    GLIBC_NUMBER="${MAX_GLIBC#GLIBC_}"
else
    GLIBC_NUMBER="unknown"
fi

if [[ "$GLIBC_NUMBER" != "unknown" ]] && version_le "$GLIBC_NUMBER" "2.35"; then
    PLATFORM_TAG="manylinux_2_35_x86_64"
else
    PLATFORM_TAG="linux_x86_64"
fi
export SKLINT_WHEEL_PLATFORM_TAG="$PLATFORM_TAG"

# setup.py invokes cargo build again with --locked; Cargo reuses the verified
# release output and embeds the same provenance environment.
python setup.py bdist_wheel

WHEEL="$(find "$ROOT/dist" -maxdepth 1 -type f -name "sklint-${VERSION}-*.whl" -print -quit)"
if [[ -z "$WHEEL" ]]; then
    echo "ERROR: wheel was not produced" >&2
    exit 1
fi

cp "$WHEEL" "$OUT_DIR/"
WHEEL_OUT="$OUT_DIR/$(basename "$WHEEL")"
WHEEL_SHA="$(sha256sum "$WHEEL_OUT" | awk '{print $1}')"
BIN_SHA="$(sha256sum "$BIN" | awk '{print $1}')"

VERIFY_VENV="$OUT_DIR/verify-venv"
rm -rf "$VERIFY_VENV"
python -m venv "$VERIFY_VENV"
"$VERIFY_VENV/bin/python" -m pip install --no-index "$WHEEL_OUT" >/dev/null
VERIFY_OUTPUT="$("$VERIFY_VENV/bin/sklint" --version --verbose)"
printf '%s\n' "$VERIFY_OUTPUT"

if ! grep -Fq "sklint $VERSION" <<<"$VERIFY_OUTPUT"; then
    echo "ERROR: installed wheel reports wrong version" >&2
    exit 1
fi
if ! grep -Fq "source-tree-sha256: $TREE_SHA" <<<"$VERIFY_OUTPUT"; then
    echo "ERROR: installed wheel provenance has wrong source tree hash" >&2
    exit 1
fi

rm -rf "$VERIFY_VENV"

cat > "$STATE" <<STATE
version=$VERSION
revision=$REVISION
rustc=$ACTUAL_RUSTC
source_tree_sha256=$TREE_SHA
source_commit=$SKLINT_SOURCE_COMMIT
source_dirty=$SKLINT_SOURCE_DIRTY
binary_sha256=$BIN_SHA
max_glibc=${MAX_GLIBC:-none}
wheel_platform_tag=$PLATFORM_TAG
wheel=$(basename "$WHEEL_OUT")
wheel_sha256=$WHEEL_SHA
STATE

printf '%s  %s\n' "$WHEEL_SHA" "$(basename "$WHEEL_OUT")" > "$OUT_DIR/SHA256SUMS.txt"

echo
echo "SKLint Linux wheel candidate PASS"
echo "source-tree-sha256: $TREE_SHA"
echo "max-glibc: ${MAX_GLIBC:-none}"
echo "wheel: $WHEEL_OUT"
echo "wheel-sha256: $WHEEL_SHA"
