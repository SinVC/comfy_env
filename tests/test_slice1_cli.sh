#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GOV_BIN="${GOV_BIN:-$ROOT_DIR/target/debug/gov}"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

assert_contains() {
    local path="$1"
    local expected="$2"
    if ! grep -Fq "$expected" "$path"; then
        echo "expected '$expected' in $path" >&2
        cat "$path" >&2 || true
        exit 1
    fi
}

WORK_DIR="$TMP_ROOT/work"
FAKE_BIN="$TMP_ROOT/fake-bin"
mkdir -p "$WORK_DIR/state/ops" "$WORK_DIR/state/work" "$FAKE_BIN"

cat >"$WORK_DIR/config.toml" <<EOF
[runtime]
python = "3.12"
prod_env = ".venv-prod"

[tx]
timeout_seconds = 30

[tx.smoke_test]
program = "$FAKE_BIN/smoke-pass"
args = []
EOF

cat >"$WORK_DIR/pyproject.toml" <<'EOF'
[project]
name = "demo"
version = "0.1.0"

[dependency-groups]
core = []
torch = []
overrides = []
EOF

printf 'base-lock\n' >"$WORK_DIR/uv.lock"
printf '[]\n' >"$WORK_DIR/state/plugins.json"

cat >"$FAKE_BIN/uv" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

cmd="${1:-}"
case "$cmd" in
  lock)
    printf 'locked\n' > uv.lock
    ;;
  sync)
    ;;
  --version)
    echo "uv 0.fake"
    ;;
  python)
    echo "/usr/bin/python3"
    ;;
  *)
    echo "unsupported uv command" >&2
    exit 1
    ;;
esac
EOF
chmod +x "$FAKE_BIN/uv"

cat >"$FAKE_BIN/smoke-pass" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
chmod +x "$FAKE_BIN/smoke-pass"

PATH="$FAKE_BIN:$PATH" GOV_UV_BIN="$FAKE_BIN/uv" "$GOV_BIN" pin add numpy==1.26.4 >"$TMP_ROOT/pin-add.out"
assert_contains "$TMP_ROOT/pin-add.out" "Pins added."
assert_contains "$WORK_DIR/pyproject.toml" 'numpy==1.26.4'

PATH="$FAKE_BIN:$PATH" GOV_UV_BIN="$FAKE_BIN/uv" "$GOV_BIN" op list >"$TMP_ROOT/op-list.out"
assert_contains "$TMP_ROOT/op-list.out" "pin_add"

OP_ID="$(find "$WORK_DIR/state/ops" -mindepth 1 -maxdepth 1 -type d | head -n 1 | xargs -n 1 basename)"
PATH="$FAKE_BIN:$PATH" GOV_UV_BIN="$FAKE_BIN/uv" "$GOV_BIN" undo "$OP_ID" >"$TMP_ROOT/undo.out"
assert_contains "$TMP_ROOT/undo.out" "Undo completed"

PATH="$FAKE_BIN:$PATH" GOV_UV_BIN="$FAKE_BIN/uv" "$GOV_BIN" op inspect "$OP_ID" >"$TMP_ROOT/op-inspect.out"
assert_contains "$TMP_ROOT/op-inspect.out" "status: undone"

if grep -Fq 'numpy==1.26.4' "$WORK_DIR/pyproject.toml"; then
    echo "expected undo to restore pyproject.toml" >&2
    exit 1
fi
