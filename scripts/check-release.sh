#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo build -p sklint

BIN="$ROOT/target/debug/sklint"
"$BIN" --version
"$BIN" check examples/good.py
if "$BIN" check examples; then
  echo "expected diagnostics for examples/bad.py" >&2
  exit 1
fi
if "$BIN" check --format json examples/bad.py; then
  echo "expected JSON diagnostics for examples/bad.py" >&2
  exit 1
fi
if "$BIN" format --check examples/bad.py; then
  echo "expected format --check to report changes for examples/bad.py" >&2
  exit 1
fi

PYTHON_BIN="${PYTHON:-python3}"
SMOKE_DIR="$(mktemp -d)"
"$PYTHON_BIN" -m venv "$SMOKE_DIR/venv"
"$SMOKE_DIR/venv/bin/python" -m pip install --upgrade pip wheel >/dev/null
"$SMOKE_DIR/venv/bin/python" -m pip wheel . -w "$SMOKE_DIR/wheelhouse" --no-deps
WHEEL="$(ls -t "$SMOKE_DIR"/wheelhouse/sklint-*.whl | head -n 1)"
"$SMOKE_DIR/venv/bin/python" -m pip install --force-reinstall "$WHEEL"
"$SMOKE_DIR/venv/bin/sklint" --version
"$SMOKE_DIR/venv/bin/python" -m sklint --version
"$SMOKE_DIR/venv/bin/sklint" check examples/good.py

(
  cd vscode
  npm install
  npm run compile
)
python3 scripts/package-vscode.py >/dev/null
test -f "dist/sklint-$(python3 - <<'PY'
import json
print(json.load(open('vscode/package.json', encoding='utf-8'))['version'])
PY
).vsix"

if command -v npx >/dev/null 2>&1; then
  PROBE="$(mktemp -d)"
  cat > "$PROBE/pyright_clean_self_dynamic.py" <<'PY'
class Box:
    def attach(self) -> None:
        self.generated = 1

    def read(self) -> int:
        return self.generated
PY
  cat > "$PROBE/pyright_reports_object_dynamic.py" <<'PY'
class Box:
    pass

box = Box()
box.generated = 1
print(box.generated)
PY
  echo '{ "typeCheckingMode": "strict" }' > "$PROBE/pyrightconfig.json"
  (cd "$PROBE" && npx pyright pyright_clean_self_dynamic.py)
  if (cd "$PROBE" && npx pyright pyright_reports_object_dynamic.py); then
    echo "expected pyright to report object dynamic attribute access" >&2
    exit 1
  fi
fi

echo "release checks passed"
