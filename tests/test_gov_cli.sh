#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

copy_fresh_workspace() {
    local dst="$1"
    mkdir -p "$dst"
    tar -C "$ROOT_DIR" \
        --exclude='./config.toml' \
        --exclude='./pyproject.toml' \
        --exclude='./uv.lock' \
        --exclude='./state/plugins.json' \
        --exclude='./state/transactions' \
        --exclude='./state/logs' \
        --exclude='./state/conflicts' \
        --exclude='./state/work' \
        --exclude='./state/ops' \
        --exclude='./cache' \
        --exclude='./.venv-prod' \
        --exclude='./.venv-candidate' \
        -cf - . | tar -C "$dst" -xf -
}

copy_fresh_workspace "$TMP_ROOT/work"
WORK_DIR="$TMP_ROOT/work"
FAKE_BIN="$TMP_ROOT/fake-bin"
mkdir -p "$FAKE_BIN"
FAKE_PY_RESOLVERS="$TMP_ROOT/fake-python"
mkdir -p "$FAKE_PY_RESOLVERS"
export FAKE_PY_RESOLVERS

find_host_python() {
    local candidate
    local -a candidates=()

    if [ -n "${PYTHON_CMD:-}" ]; then
        candidates+=("${PYTHON_CMD}")
    fi
    for candidate in /c/Users/*/AppData/Roaming/uv/python/*/python.exe; do
        [ -e "$candidate" ] && candidates+=("$candidate")
    done
    for candidate in /c/Users/*/AppData/Local/Programs/Python/*/python.exe; do
        [ -e "$candidate" ] && candidates+=("$candidate")
    done
    if command -v python3 >/dev/null 2>&1; then
        candidates+=("$(command -v python3)")
    fi
    if command -v python >/dev/null 2>&1; then
        candidates+=("$(command -v python)")
    fi

    for candidate in "${candidates[@]}"; do
        [ -n "$candidate" ] || continue
        if "$candidate" -c 'import sys; print(sys.version)' >/dev/null 2>&1; then
            echo "$candidate"
            return 0
        fi
    done

    echo "no runnable host python found for tests" >&2
    exit 1
}

HOST_PYTHON="$(find_host_python)"
cat >"$FAKE_BIN/python3" <<EOF
#!/usr/bin/env bash
exec "$HOST_PYTHON" "\$@"
EOF
chmod +x "$FAKE_BIN/python3"
cat >"$FAKE_BIN/python" <<EOF
#!/usr/bin/env bash
exec "$HOST_PYTHON" "\$@"
EOF
chmod +x "$FAKE_BIN/python"

reset_local_state() {
    local dir="$1"
    rm -f "$dir/config.toml" "$dir/pyproject.toml" "$dir/uv.lock"
    rm -f "$dir/state/plugins.json"
    rm -rf "$dir/.venv-prod" "$dir/.venv-candidate"
}

reset_local_state "$WORK_DIR"

cat >"$FAKE_BIN/uv" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

while [ "${1:-}" = "--cache-dir" ]; do
    shift 2
done

cmd="${1:-}"
shift || true

update_dependency_group() {
    local action="$1"
    local group="$2"
    shift 2
    python3 - "$action" "$group" "$@" <<'PY'
import json
import pathlib
import re
import sys

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib

path = pathlib.Path("pyproject.toml")
text = path.read_text(encoding="utf-8")

try:
    data = tomllib.loads(text)
except Exception as exc:
    print(f"ERROR: failed to parse pyproject.toml: {exc}", file=sys.stderr)
    raise SystemExit(1)

groups = data.get("dependency-groups", {})
if not isinstance(groups, dict):
    groups = {}

action = sys.argv[1]
group = sys.argv[2]
items = [str(x).strip() for x in sys.argv[3:] if str(x).strip()]

def normalize_name(spec: str) -> str:
    token = re.split(r"[<>=!~;\[]", str(spec).strip(), maxsplit=1)[0].strip()
    if "@" in token:
        token = token.split("@", 1)[0].strip()
    return re.sub(r"[-_.]+", "-", token).lower().strip("-")

def entry_name(entry) -> str:
    if not isinstance(entry, str):
        return ""
    spec = entry.strip()
    if not spec:
        return ""
    return normalize_name(spec)

entries = groups.get(group, [])
if not isinstance(entries, list):
    entries = []

if action == "add":
    requested_order = []
    requested_specs = {}
    emitted = set()
    out = []

    for item in items:
        norm = normalize_name(item)
        if not norm:
            continue
        if norm not in requested_specs:
            requested_order.append(norm)
        requested_specs[norm] = item

    for norm in requested_order:
        matches = [entry for entry in entries if entry_name(entry) == norm]
        if len(matches) > 1:
            print(f"error: Cannot perform ambiguous update; found multiple entries for `{norm}`:", file=sys.stderr)
            for entry in matches:
                print(f"- `{entry}`", file=sys.stderr)
            raise SystemExit(2)

    for entry in entries:
        norm = entry_name(entry)
        if norm in requested_specs:
            if norm not in emitted:
                out.append(requested_specs[norm])
                emitted.add(norm)
            continue
        out.append(entry)

    for norm in requested_order:
        if norm not in emitted:
            out.append(requested_specs[norm])
            emitted.add(norm)

    groups[group] = out
elif action == "remove":
    wanted_order = []
    wanted = set()
    for item in items:
        norm = normalize_name(item)
        if not norm or norm in wanted:
            continue
        wanted_order.append(norm)
        wanted.add(norm)

    missing = [norm for norm in wanted_order if not any(entry_name(item) == norm for item in entries)]
    if missing:
        print(f"error: The dependency `{missing[0]}` could not be found in `dependency-groups.{group}`", file=sys.stderr)
        raise SystemExit(2)

    groups[group] = [item for item in entries if entry_name(item) not in wanted]
else:
    raise SystemExit(f"unsupported action: {action}")

def render_toml_key(value: str) -> str:
    if re.match(r"^[A-Za-z0-9_-]+$", value):
        return value
    return json.dumps(value)

def render_toml_value(value):
    if isinstance(value, str):
        return json.dumps(value)
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, int):
        return str(value)
    if isinstance(value, float):
        return repr(value)
    if isinstance(value, list):
        return "[" + ", ".join(render_toml_value(item) for item in value) + "]"
    if isinstance(value, dict):
        parts = [f"{render_toml_key(str(key))} = {render_toml_value(item)}" for key, item in value.items()]
        return "{ " + ", ".join(parts) + " }"
    raise SystemExit(f"unsupported dependency-group item type: {type(value).__name__}")

def render_group_assignment(rendered_key: str, values) -> str:
    if not isinstance(values, list):
        values = []
    if not values:
        return f"{rendered_key} = []"
    rendered = ",\n".join(f"    {render_toml_value(value)}" for value in values)
    return f"{rendered_key} = [\n{rendered},\n]"

def parse_key_token(token: str) -> str:
    if token.startswith('"'):
        return json.loads(token)
    if token.startswith("'"):
        return token[1:-1].replace("''", "'")
    return token

section_re = re.compile(r"(?ms)^\[dependency-groups\]\n.*?(?=^\[|\Z)")
match = section_re.search(text)
ordered_keys = []
rendered_keys = {}
if match:
    for raw_line in match.group(0).splitlines()[1:]:
        m = re.match(r"^\s*([A-Za-z0-9_.-]+|\"(?:[^\"\\]|\\.)*\"|'(?:[^']|'')*')\s*=", raw_line)
        if not m:
            continue
        key = parse_key_token(m.group(1))
        if key not in ordered_keys:
            ordered_keys.append(key)
            rendered_keys[key] = m.group(1)
for key in groups.keys():
    if key not in ordered_keys:
        ordered_keys.append(key)

section_lines = ["[dependency-groups]"]
for key in ordered_keys:
    values = groups.get(key, [])
    if not isinstance(values, list):
        values = []
    section_lines.append(render_group_assignment(rendered_keys.get(key, render_toml_key(str(key))), values))
new_section = "\n".join(section_lines) + "\n"

if match:
    text = text[:match.start()] + new_section + text[match.end():]
else:
    if text and not text.endswith("\n"):
        text += "\n"
    text += "\n" + new_section

path.write_text(text, encoding="utf-8")
PY
}

case "$cmd" in
    lock)
        check=false
        while [ $# -gt 0 ]; do
            case "$1" in
                --check)
                    check=true
                    shift
                    ;;
                --python)
                    shift 2
                    ;;
                *)
                    shift
                    ;;
            esac
        done
        if [ "$check" = true ]; then
            if [ "${FAKE_UV_LOCK_CHECK_FAIL:-0}" = "1" ]; then
                echo "lock check failed" >&2
                exit 1
            fi
            exit 0
        fi
        if [ "${FAKE_UV_LOCK_FAIL:-0}" = "1" ]; then
            echo "lock failed" >&2
            exit 1
        fi
        requires_python="$(python3 - <<'PY'
import pathlib
import re

text = pathlib.Path("pyproject.toml").read_text(encoding="utf-8")
match = re.search(r'^requires-python\s*=\s*"([^"]+)"', text, flags=re.MULTILINE)
print(match.group(1) if match else "")
PY
)"
        env_marker="$(python3 - <<'PY'
import pathlib

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib

data = tomllib.loads(pathlib.Path("pyproject.toml").read_text(encoding="utf-8"))
tool_uv = (data.get("tool") or {}).get("uv") or {}
envs = tool_uv.get("environments") or []
if isinstance(envs, str):
    envs = [envs]
print(envs[0] if envs else "")
PY
)"
        {
            echo 'version = 1'
            echo 'revision = 3'
            if [ -n "$requires_python" ]; then
                echo "requires-python = \"$requires_python\""
            fi
            if [ -n "$env_marker" ]; then
                echo 'resolution-markers = ['
                echo "    \"$env_marker\","
                echo ']'
            fi
        } > uv.lock
        ;;
    sync)
        if [ "${FAKE_UV_SYNC_FAIL:-0}" = "1" ]; then
            echo "sync failed" >&2
            exit 1
        fi
        env_path="${UV_PROJECT_ENVIRONMENT:-}"
        if [ -z "$env_path" ]; then
            echo "missing UV_PROJECT_ENVIRONMENT" >&2
            exit 1
        fi
        mkdir -p "$env_path/bin"
        cat >"$env_path/bin/python" <<'PYEOF'
#!/usr/bin/env bash
exit "${FAKE_PYTHON_EXIT_CODE:-0}"
PYEOF
        chmod +x "$env_path/bin/python"
        ;;
    add|remove)
        group=""
        positional=()
        while [ $# -gt 0 ]; do
            case "$1" in
                --group)
                    group="${2:-}"
                    shift 2
                    ;;
                --python|--index)
                    shift 2
                    ;;
                --frozen|--no-sync)
                    shift
                    ;;
                *)
                    positional+=("$1")
                    shift
                    ;;
            esac
        done
        if [ -z "$group" ]; then
            echo "missing group" >&2
            exit 1
        fi
        update_dependency_group "$cmd" "$group" "${positional[@]}"
        ;;
    export)
        if [ "${FAKE_UV_EXPORT_FAIL:-0}" = "1" ]; then
            echo "export failed" >&2
            exit 1
        fi
        output_file=""
        while [ $# -gt 0 ]; do
            case "$1" in
                --output-file)
                    output_file="${2:-}"
                    shift 2
                    ;;
                *)
                    shift
                    ;;
            esac
        done
        if [ -z "$output_file" ]; then
            echo "missing output file" >&2
            exit 1
        fi
        cat >"$output_file" <<'PYLOCK'
# fake pylock
[[packages]]
name = "demo"
version = "1.0.0"
PYLOCK
        ;;
    pip)
        subcmd="${1:-}"
        shift || true
        case "$subcmd" in
            freeze)
                printf 'demo==1.0.0\n'
                ;;
            *)
                ;;
        esac
        ;;
    python)
        subcmd="${1:-}"
        shift || true
        case "$subcmd" in
            find)
                request=""
                while [ $# -gt 0 ]; do
                    case "$1" in
                        --show-version|--no-python-downloads|--system)
                            shift
                            ;;
                        *)
                            request="$1"
                            shift
                            ;;
                    esac
                done
                if [ "${FAKE_UV_PYTHON_FIND_FAIL_REQUEST:-}" = "$request" ]; then
                    echo "python not found: $request" >&2
                    exit 1
                fi
                case "$request" in
                    *.*.*)
                        version="$request"
                        ;;
                    *.*)
                        version="${request}.9"
                        ;;
                    *)
                        version="${request}.0"
                        ;;
                esac
                resolved_path="$FAKE_PY_RESOLVERS/${version}"
                cat >"$resolved_path" <<PYEOF
#!/usr/bin/env bash
if [ "\${1:-}" = "-c" ]; then
    if [ "\${2:-}" = 'import sys; print(f"{sys.version_info[0]}.{sys.version_info[1]}")' ]; then
        echo "${version%.*}"
        exit 0
    fi
    exit 1
else
    exit 0
fi
PYEOF
                chmod +x "$resolved_path"
                echo "$resolved_path"
                ;;
            *)
                ;;
        esac
        ;;
    *)
        ;;
esac
EOF
chmod +x "$FAKE_BIN/uv"

assert_contains() {
    local file="$1"
    local pattern="$2"
    if ! grep -F -q "$pattern" "$file"; then
        echo "assertion failed: '$pattern' not found in $file" >&2
        exit 1
    fi
}

assert_not_contains() {
    local file="$1"
    local pattern="$2"
    if grep -F -q "$pattern" "$file"; then
        echo "assertion failed: '$pattern' unexpectedly found in $file" >&2
        exit 1
    fi
}

assert_occurrences() {
    local file="$1"
    local pattern="$2"
    local expected="$3"
    local actual
    actual="$(grep -F -c "$pattern" "$file" || true)"
    if [ "$actual" != "$expected" ]; then
        echo "assertion failed: expected '$pattern' to appear $expected time(s) in $file, got $actual" >&2
        exit 1
    fi
}

assert_dependency_group_assignment_count() {
    local file="$1"
    local group="$2"
    local expected="$3"
    python3 - "$file" "$group" "$expected" <<'PY'
import pathlib
import re
import sys

path = pathlib.Path(sys.argv[1])
group = sys.argv[2]
expected = int(sys.argv[3])
text = path.read_text(encoding="utf-8")
match = re.search(r"(?ms)^\[dependency-groups\]\n.*?(?=^\[|\Z)", text)
if not match:
    actual = 0
else:
    quoted = re.escape(group)
    pattern = rf"(?m)^\s*(?:{quoted}|\"{quoted}\"|'{quoted}')\s*="
    actual = len(re.findall(pattern, match.group(0)))
if actual != expected:
    raise SystemExit(f"assertion failed: expected dependency group '{group}' to appear {expected} time(s) in {path}, got {actual}")
PY
}

assert_toml_parses() {
    local file="$1"
    python3 - "$file" <<'PY'
import pathlib
import sys

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib

path = pathlib.Path(sys.argv[1])
tomllib.loads(path.read_text(encoding="utf-8"))
PY
}

assert_file_exists() {
    local path="$1"
    if [ ! -e "$path" ]; then
        echo "assertion failed: expected file to exist: $path" >&2
        exit 1
    fi
}

assert_not_exists() {
    local path="$1"
    if [ -e "$path" ]; then
        echo "assertion failed: expected path to be absent: $path" >&2
        exit 1
    fi
}

assert_tar_entry_exists() {
    local tar_path="$1"
    local entry="$2"
    python3 - "$tar_path" "$entry" <<'PY'
import pathlib
import sys
import tarfile

tar_path = pathlib.Path(sys.argv[1])
entry = sys.argv[2]
with tarfile.open(tar_path, "r") as archive:
    names = set(archive.getnames())
if entry not in names:
    raise SystemExit(f"assertion failed: expected tar entry to exist: {entry} in {tar_path}")
PY
}

assert_tar_entry_absent() {
    local tar_path="$1"
    local entry="$2"
    python3 - "$tar_path" "$entry" <<'PY'
import pathlib
import sys
import tarfile

tar_path = pathlib.Path(sys.argv[1])
entry = sys.argv[2]
with tarfile.open(tar_path, "r") as archive:
    names = set(archive.getnames())
if entry in names:
    raise SystemExit(f"assertion failed: expected tar entry to be absent: {entry} in {tar_path}")
PY
}

extract_bundle_tar() {
    local tar_path="$1"
    local dst="$2"
    python3 - "$tar_path" "$dst" <<'PY'
import pathlib
import sys
import tarfile

tar_path = pathlib.Path(sys.argv[1])
dst = pathlib.Path(sys.argv[2])
dst.mkdir(parents=True, exist_ok=True)
with tarfile.open(tar_path, "r") as archive:
    archive.extractall(dst)
bundle_dir = dst / "bundle"
if not bundle_dir.exists():
    raise SystemExit(f"expected extracted bundle root at {bundle_dir}")
PY
}

repack_bundle_tar() {
    local src_root="$1"
    local tar_path="$2"
    python3 - "$src_root" "$tar_path" <<'PY'
import pathlib
import sys
import tarfile

src_root = pathlib.Path(sys.argv[1])
tar_path = pathlib.Path(sys.argv[2])
bundle_dir = src_root / "bundle"
if not bundle_dir.exists():
    raise SystemExit(f"bundle directory not found for repack: {bundle_dir}")
with tarfile.open(tar_path, "w") as archive:
    archive.add(bundle_dir, arcname="bundle")
PY
}

update_bundle_manifest_checksum() {
    local bundle_root="$1"
    local rel="$2"
    python3 - "$bundle_root" "$rel" <<'PY'
import hashlib
import json
import pathlib
import sys

bundle_root = pathlib.Path(sys.argv[1])
rel = sys.argv[2]
target = bundle_root / rel
manifest_path = bundle_root / "manifest.json"
manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
manifest["files"]["sha256"][rel] = hashlib.sha256(target.read_bytes()).hexdigest()
manifest_path.write_text(json.dumps(manifest, indent=2, ensure_ascii=True) + "\n", encoding="utf-8")
PY
}

help_out="$TMP_ROOT/help.txt"
bash "$WORK_DIR/bin/gov" help >"$help_out"
assert_contains "$help_out" "gov install torch --index-url <url> [--torch <torch==version>] [--torchvision <torchvision==version>] [--torchaudio <torchaudio==version>]"
assert_contains "$help_out" "gov pin add <pkg==version>..."
assert_contains "$help_out" "gov pin list"
assert_contains "$help_out" "gov pin remove <pkg>..."
assert_contains "$help_out" "gov update run"
assert_contains "$help_out" "gov env export <output_tar>"
assert_contains "$help_out" "gov env import <bundle_tar> --comfyui-dir <abs-path> --python <python-spec>"
assert_not_contains "$help_out" "[--force]"

assert_not_exists "$WORK_DIR/pyproject.toml"
PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" pin list >"$TMP_ROOT/pin-list-pre-init.out"
assert_contains "$TMP_ROOT/pin-list-pre-init.out" "No pins in overrides group."
assert_not_exists "$WORK_DIR/pyproject.toml"

set +e
PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" init >"$TMP_ROOT/init-no-args.out" 2>"$TMP_ROOT/init-no-args.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected init without args to fail" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/init-no-args.err" "initial setup requires"

mkdir -p "$TMP_ROOT/ComfyUI"
PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" init --comfyui-dir "$TMP_ROOT/ComfyUI" --python 3.12 >"$TMP_ROOT/init.out"

assert_contains "$WORK_DIR/config.toml" "comfyui_dir = \"$TMP_ROOT/ComfyUI\""
assert_contains "$WORK_DIR/config.toml" "python = \"3.12\""
assert_contains "$WORK_DIR/pyproject.toml" "requires-python = \"==3.12.*\""
assert_contains "$WORK_DIR/pyproject.toml" "[tool.uv]"
assert_contains "$WORK_DIR/pyproject.toml" "sys_platform == 'linux' and platform_machine == 'x86_64'"
assert_contains "$WORK_DIR/uv.lock" "requires-python = \"==3.12.*\""
assert_contains "$WORK_DIR/uv.lock" "sys_platform == 'linux' and platform_machine == 'x86_64'"

BROKEN_LIST_DIR="$TMP_ROOT/broken-list-work"
cp -a "$WORK_DIR/." "$BROKEN_LIST_DIR/"
cat >"$BROKEN_LIST_DIR/pyproject.toml" <<'EOF'
[project]
name = "demo"
version = "0.1.0"

[dependency-groups]
overrides = [
    "numpy==1.26.4",
# invalid TOML on purpose
EOF
set +e
PATH="$FAKE_BIN:$PATH" bash "$BROKEN_LIST_DIR/bin/gov" pin list >"$TMP_ROOT/broken-pin-list.out" 2>"$TMP_ROOT/broken-pin-list.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected pin list with invalid pyproject.toml to fail" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/broken-pin-list.err" "failed to parse pyproject.toml"

BROKEN_REMOVE_DIR="$TMP_ROOT/broken-remove-work"
cp -a "$WORK_DIR/." "$BROKEN_REMOVE_DIR/"
cat >"$BROKEN_REMOVE_DIR/pyproject.toml" <<'EOF'
[project]
name = "demo"
version = "0.1.0"

[dependency-groups]
overrides = [
    "numpy==1.26.4",
# invalid TOML on purpose
EOF
set +e
PATH="$FAKE_BIN:$PATH" bash "$BROKEN_REMOVE_DIR/bin/gov" pin remove numpy >"$TMP_ROOT/broken-pin-remove.out" 2>"$TMP_ROOT/broken-pin-remove.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected pin remove with invalid pyproject.toml to fail" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/broken-pin-remove.err" "failed to parse pyproject.toml"

PRESERVE_DIR="$TMP_ROOT/preserve-work"
cp -a "$WORK_DIR/." "$PRESERVE_DIR/"
python3 - "$PRESERVE_DIR/pyproject.toml" <<'PY'
import pathlib
import re
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
replacement = """[dependency-groups]
core = [
    "numpy>=1.25.0",
]
dev = [
    { include-group = "core" },
    "pytest>=8",
]
torch = []
overrides = []
"""
text, count = re.subn(r"(?ms)^\[dependency-groups\]\n.*?(?=^\[|\Z)", replacement, text)
if count != 1:
    raise SystemExit("failed to replace dependency-groups section in preserve fixture")
path.write_text(text, encoding="utf-8")
PY
PATH="$FAKE_BIN:$PATH" bash "$PRESERVE_DIR/bin/gov" pin add transformers==4.44.0 >"$TMP_ROOT/preserve-pin-add.out" 2>"$TMP_ROOT/preserve-pin-add.err"
assert_contains "$TMP_ROOT/preserve-pin-add.out" "Pins added."
assert_contains "$PRESERVE_DIR/pyproject.toml" "{ include-group = \"core\" }"
assert_contains "$PRESERVE_DIR/pyproject.toml" "\"pytest>=8\""
assert_contains "$PRESERVE_DIR/pyproject.toml" "\"transformers==4.44.0\""
assert_not_contains "$PRESERVE_DIR/pyproject.toml" "\"{'include-group': 'core'}\""

QUOTED_DIR="$TMP_ROOT/quoted-work"
cp -a "$WORK_DIR/." "$QUOTED_DIR/"
python3 - "$QUOTED_DIR/pyproject.toml" <<'PY'
import pathlib
import re
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
replacement = """[dependency-groups]
core = []
torch = []
"overrides" = []
"""
text, count = re.subn(r"(?ms)^\[dependency-groups\]\n.*?(?=^\[|\Z)", replacement, text)
if count != 1:
    raise SystemExit("failed to replace dependency-groups section in quoted fixture")
path.write_text(text, encoding="utf-8")
PY
PATH="$FAKE_BIN:$PATH" bash "$QUOTED_DIR/bin/gov" pin add transformers==4.44.0 >"$TMP_ROOT/quoted-pin-add.out" 2>"$TMP_ROOT/quoted-pin-add.err"
assert_contains "$TMP_ROOT/quoted-pin-add.out" "Pins added."
assert_contains "$QUOTED_DIR/pyproject.toml" "\"overrides\" = ["
assert_contains "$QUOTED_DIR/pyproject.toml" "\"transformers==4.44.0\""
assert_dependency_group_assignment_count "$QUOTED_DIR/pyproject.toml" "overrides" "1"

PATH="$FAKE_BIN:$PATH" bash "$QUOTED_DIR/bin/gov" pin remove transformers >"$TMP_ROOT/quoted-pin-remove.out" 2>"$TMP_ROOT/quoted-pin-remove.err"
assert_contains "$TMP_ROOT/quoted-pin-remove.out" "Pins removed."
assert_contains "$QUOTED_DIR/pyproject.toml" "\"overrides\" = []"
assert_not_contains "$QUOTED_DIR/pyproject.toml" "\"transformers==4.44.0\""
assert_dependency_group_assignment_count "$QUOTED_DIR/pyproject.toml" "overrides" "1"

SPECIAL_DIR="$TMP_ROOT/special-work"
cp -a "$WORK_DIR/." "$SPECIAL_DIR/"
python3 - "$SPECIAL_DIR/pyproject.toml" <<'PY'
import pathlib
import re
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
replacement = """[dependency-groups]
core = []
torch = []
"overrides" = [
    'demo @ https://example.com/pkg].whl',
]
"""
text, count = re.subn(r"(?ms)^\[dependency-groups\]\n.*?(?=^\[|\Z)", replacement, text)
if count != 1:
    raise SystemExit("failed to replace dependency-groups section in special fixture")
path.write_text(text, encoding="utf-8")
PY
PATH="$FAKE_BIN:$PATH" bash "$SPECIAL_DIR/bin/gov" pin add transformers==4.44.0 >"$TMP_ROOT/special-pin-add.out" 2>"$TMP_ROOT/special-pin-add.err"
assert_contains "$TMP_ROOT/special-pin-add.out" "Pins added."
assert_contains "$SPECIAL_DIR/pyproject.toml" "\"overrides\" = ["
assert_contains "$SPECIAL_DIR/pyproject.toml" "\"transformers==4.44.0\""
assert_occurrences "$SPECIAL_DIR/pyproject.toml" "https://example.com/pkg].whl" "1"
assert_dependency_group_assignment_count "$SPECIAL_DIR/pyproject.toml" "overrides" "1"
assert_toml_parses "$SPECIAL_DIR/pyproject.toml"

PATH="$FAKE_BIN:$PATH" bash "$SPECIAL_DIR/bin/gov" pin remove transformers >"$TMP_ROOT/special-pin-remove.out" 2>"$TMP_ROOT/special-pin-remove.err"
assert_contains "$TMP_ROOT/special-pin-remove.out" "Pins removed."
assert_contains "$SPECIAL_DIR/pyproject.toml" "\"overrides\" = ["
assert_not_contains "$SPECIAL_DIR/pyproject.toml" "\"transformers==4.44.0\""
assert_occurrences "$SPECIAL_DIR/pyproject.toml" "https://example.com/pkg].whl" "1"
assert_dependency_group_assignment_count "$SPECIAL_DIR/pyproject.toml" "overrides" "1"
assert_toml_parses "$SPECIAL_DIR/pyproject.toml"

DUPLICATE_ADD_DIR="$TMP_ROOT/duplicate-add-work"
cp -a "$WORK_DIR/." "$DUPLICATE_ADD_DIR/"
python3 - "$DUPLICATE_ADD_DIR/pyproject.toml" <<'PY'
import pathlib
import re
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
replacement = """[dependency-groups]
core = []
torch = []
overrides = [
    "numpy==1.26.3",
    "numpy==1.26.4",
    "pillow==10.0.0",
]
"""
text, count = re.subn(r"(?ms)^\[dependency-groups\]\n.*?(?=^\[|\Z)", replacement, text)
if count != 1:
    raise SystemExit("failed to replace dependency-groups section in duplicate-add fixture")
path.write_text(text, encoding="utf-8")
PY
PATH="$FAKE_BIN:$PATH" bash "$DUPLICATE_ADD_DIR/bin/gov" pin add numpy==1.26.5 >"$TMP_ROOT/duplicate-pin-add.out" 2>"$TMP_ROOT/duplicate-pin-add.err"
assert_contains "$TMP_ROOT/duplicate-pin-add.out" "Pins added."
assert_contains "$DUPLICATE_ADD_DIR/pyproject.toml" "\"numpy==1.26.5\""
assert_not_contains "$DUPLICATE_ADD_DIR/pyproject.toml" "\"numpy==1.26.3\""
assert_not_contains "$DUPLICATE_ADD_DIR/pyproject.toml" "\"numpy==1.26.4\""
assert_occurrences "$DUPLICATE_ADD_DIR/pyproject.toml" "numpy==" "1"
assert_contains "$DUPLICATE_ADD_DIR/pyproject.toml" "\"pillow==10.0.0\""

LAST_WINS_DIR="$TMP_ROOT/last-wins-work"
cp -a "$WORK_DIR/." "$LAST_WINS_DIR/"
PATH="$FAKE_BIN:$PATH" bash "$LAST_WINS_DIR/bin/gov" pin add numpy==1.26.3 numpy==1.26.2 >"$TMP_ROOT/last-wins-pin-add.out" 2>"$TMP_ROOT/last-wins-pin-add.err"
assert_contains "$TMP_ROOT/last-wins-pin-add.out" "Pins added."
assert_contains "$LAST_WINS_DIR/pyproject.toml" "\"numpy==1.26.2\""
assert_not_contains "$LAST_WINS_DIR/pyproject.toml" "\"numpy==1.26.3\""
assert_occurrences "$LAST_WINS_DIR/pyproject.toml" "numpy==" "1"

REMOVE_ATOMIC_DIR="$TMP_ROOT/remove-atomic-work"
cp -a "$WORK_DIR/." "$REMOVE_ATOMIC_DIR/"
python3 - "$REMOVE_ATOMIC_DIR/pyproject.toml" <<'PY'
import pathlib
import re
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
replacement = """[dependency-groups]
core = []
torch = []
overrides = [
    "numpy==1.26.4",
    "pillow==10.0.0",
]
"""
text, count = re.subn(r"(?ms)^\[dependency-groups\]\n.*?(?=^\[|\Z)", replacement, text)
if count != 1:
    raise SystemExit("failed to replace dependency-groups section in remove-atomic fixture")
path.write_text(text, encoding="utf-8")
PY
remove_atomic_before="$(cat "$REMOVE_ATOMIC_DIR/pyproject.toml")"
set +e
PATH="$FAKE_BIN:$PATH" bash "$REMOVE_ATOMIC_DIR/bin/gov" pin remove numpy transformers >"$TMP_ROOT/remove-atomic.out" 2>"$TMP_ROOT/remove-atomic.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected pin remove with one missing package to fail atomically" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/remove-atomic.err" "could not be found"
if [ "$remove_atomic_before" != "$(cat "$REMOVE_ATOMIC_DIR/pyproject.toml")" ]; then
    echo "expected pyproject.toml to remain unchanged after atomic pin remove failure" >&2
    exit 1
fi

REMOVE_DUPLICATE_DIR="$TMP_ROOT/remove-duplicate-work"
cp -a "$WORK_DIR/." "$REMOVE_DUPLICATE_DIR/"
python3 - "$REMOVE_DUPLICATE_DIR/pyproject.toml" <<'PY'
import pathlib
import re
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
replacement = """[dependency-groups]
core = []
torch = []
overrides = [
    "numpy==1.26.3",
    "numpy==1.26.4",
    "pillow==10.0.0",
]
"""
text, count = re.subn(r"(?ms)^\[dependency-groups\]\n.*?(?=^\[|\Z)", replacement, text)
if count != 1:
    raise SystemExit("failed to replace dependency-groups section in remove-duplicate fixture")
path.write_text(text, encoding="utf-8")
PY
PATH="$FAKE_BIN:$PATH" bash "$REMOVE_DUPLICATE_DIR/bin/gov" pin remove numpy >"$TMP_ROOT/remove-duplicate.out" 2>"$TMP_ROOT/remove-duplicate.err"
assert_contains "$TMP_ROOT/remove-duplicate.out" "Pins removed."
assert_not_contains "$REMOVE_DUPLICATE_DIR/pyproject.toml" "\"numpy==1.26.3\""
assert_not_contains "$REMOVE_DUPLICATE_DIR/pyproject.toml" "\"numpy==1.26.4\""
assert_contains "$REMOVE_DUPLICATE_DIR/pyproject.toml" "\"pillow==10.0.0\""

PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" pin list >"$TMP_ROOT/pin-list-empty.out"
assert_contains "$TMP_ROOT/pin-list-empty.out" "No pins in overrides group."

PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" pin add numpy==1.26.4 >"$TMP_ROOT/pin-add-numpy.out" 2>"$TMP_ROOT/pin-add-numpy.err"
assert_contains "$TMP_ROOT/pin-add-numpy.out" "Pins added."
assert_contains "$WORK_DIR/pyproject.toml" "numpy==1.26.4"
assert_occurrences "$WORK_DIR/pyproject.toml" "numpy==1.26.4" "1"

PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" pin list >"$TMP_ROOT/pin-list-numpy.out"
assert_contains "$TMP_ROOT/pin-list-numpy.out" "numpy==1.26.4"

PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" pin add numpy==1.26.3 >"$TMP_ROOT/pin-add-numpy-replace.out" 2>"$TMP_ROOT/pin-add-numpy-replace.err"
assert_contains "$WORK_DIR/pyproject.toml" "numpy==1.26.3"
assert_not_contains "$WORK_DIR/pyproject.toml" "numpy==1.26.4"
assert_occurrences "$WORK_DIR/pyproject.toml" "numpy==1.26.3" "1"

set +e
PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" pin add 'numpy>=1.26' >"$TMP_ROOT/pin-add-invalid.out" 2>"$TMP_ROOT/pin-add-invalid.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected pin add with invalid format to fail" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/pin-add-invalid.err" "invalid pin format"
assert_contains "$WORK_DIR/pyproject.toml" "numpy==1.26.3"

set +e
PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" pin add torch==2.11.0 >"$TMP_ROOT/pin-add-torch.out" 2>"$TMP_ROOT/pin-add-torch.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected pin add of torch-family package to fail" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/pin-add-torch.err" "torch-family packages are managed by 'gov install torch'"

set +e
PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" pin remove transformers >"$TMP_ROOT/pin-remove-missing.out" 2>"$TMP_ROOT/pin-remove-missing.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected pin remove of missing package to fail" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/pin-remove-missing.err" "could not be found"

set +e
PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" pin remove torchaudio >"$TMP_ROOT/pin-remove-torchaudio.out" 2>"$TMP_ROOT/pin-remove-torchaudio.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected pin remove of torch-family package to fail" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/pin-remove-torchaudio.err" "torch-family packages are managed by 'gov install torch'"

PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" pin add pillow==10.0.0 >"$TMP_ROOT/pin-add-pillow.out" 2>"$TMP_ROOT/pin-add-pillow.err"
assert_contains "$TMP_ROOT/pin-add-pillow.err" "WARNING: pinning non-recommended package: pillow==10.0.0"
assert_contains "$WORK_DIR/pyproject.toml" "pillow==10.0.0"

PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" pin remove numpy >"$TMP_ROOT/pin-remove-numpy.out" 2>"$TMP_ROOT/pin-remove-numpy.err"
assert_contains "$TMP_ROOT/pin-remove-numpy.out" "Pins removed."
assert_not_contains "$WORK_DIR/pyproject.toml" "numpy==1.26.3"
assert_contains "$WORK_DIR/pyproject.toml" "pillow==10.0.0"

pin_lock_before="$(cat "$WORK_DIR/uv.lock")"
pin_project_before="$(cat "$WORK_DIR/pyproject.toml")"
set +e
FAKE_UV_LOCK_FAIL=1 PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" pin add transformers==4.44.0 >"$TMP_ROOT/pin-lockfail.out" 2>"$TMP_ROOT/pin-lockfail.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected pin add to fail when lock fails" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/pin-lockfail.err" "pin add failed during lock"
assert_contains "$WORK_DIR/pyproject.toml" "pillow==10.0.0"
assert_not_contains "$WORK_DIR/pyproject.toml" "transformers==4.44.0"
if [ "$pin_project_before" != "$(cat "$WORK_DIR/pyproject.toml")" ]; then
    echo "expected pyproject.toml to be restored after pin lock failure" >&2
    exit 1
fi
if [ "$pin_lock_before" != "$(cat "$WORK_DIR/uv.lock")" ]; then
    echo "expected uv.lock to be restored after pin lock failure" >&2
    exit 1
fi

pin_project_before="$(cat "$WORK_DIR/pyproject.toml")"
pin_lock_before="$(cat "$WORK_DIR/uv.lock")"
set +e
FAKE_UV_SYNC_FAIL=1 PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" pin add transformers==4.44.0 >"$TMP_ROOT/pin-syncfail.out" 2>"$TMP_ROOT/pin-syncfail.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected pin add to fail when sync fails" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/pin-syncfail.err" "prod sync failed during pin add"
if [ "$pin_project_before" != "$(cat "$WORK_DIR/pyproject.toml")" ]; then
    echo "expected pyproject.toml to be restored after pin sync failure" >&2
    exit 1
fi
if [ "$pin_lock_before" != "$(cat "$WORK_DIR/uv.lock")" ]; then
    echo "expected uv.lock to be restored after pin sync failure" >&2
    exit 1
fi

pin_project_before="$(cat "$WORK_DIR/pyproject.toml")"
pin_lock_before="$(cat "$WORK_DIR/uv.lock")"
set +e
FAKE_PYTHON_EXIT_CODE=1 PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" pin add transformers==4.44.0 >"$TMP_ROOT/pin-smokefail.out" 2>"$TMP_ROOT/pin-smokefail.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected pin add to fail when smoke test fails" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/pin-smokefail.err" "smoke test failed during pin add"
if [ "$pin_project_before" != "$(cat "$WORK_DIR/pyproject.toml")" ]; then
    echo "expected pyproject.toml to be restored after pin smoke-test failure" >&2
    exit 1
fi
if [ "$pin_lock_before" != "$(cat "$WORK_DIR/uv.lock")" ]; then
    echo "expected uv.lock to be restored after pin smoke-test failure" >&2
    exit 1
fi

PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" init --comfyui-dir "$TMP_ROOT/ComfyUI" --python 3.12.9 >"$TMP_ROOT/init-patch.out"
assert_contains "$WORK_DIR/config.toml" "python = \"3.12\""
assert_not_contains "$WORK_DIR/config.toml" "python = \"3.12.9\""

set +e
FAKE_UV_PYTHON_FIND_FAIL_REQUEST=3.12.8 PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" init --comfyui-dir "$TMP_ROOT/ComfyUI" --python 3.12.8 >"$TMP_ROOT/init-missing-patch.out" 2>"$TMP_ROOT/init-missing-patch.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected init to fail when exact python patch cannot be resolved" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/init-missing-patch.err" "failed to resolve python request: 3.12.8"

status_out="$TMP_ROOT/status.out"
bash "$WORK_DIR/bin/gov" status >"$status_out"
assert_contains "$status_out" "config_ready: yes"
assert_contains "$status_out" "python: 3.12"
assert_contains "$status_out" "torch_ready: no"

set +e
PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" install >"$TMP_ROOT/install.err" 2>&1
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected install without torch to fail" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/install.err" "managed torch dependencies are not installed"

PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" install torch --index-url https://download.pytorch.org/whl/cu130 --torch torch==2.11.1 --torchvision torchvision==0.26.1 >"$TMP_ROOT/install-torch-custom.out"
assert_contains "$WORK_DIR/pyproject.toml" "torch==2.11.1"
assert_contains "$WORK_DIR/pyproject.toml" "torchvision==0.26.1"
assert_contains "$WORK_DIR/pyproject.toml" "\"torchaudio\""

set +e
PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" install torch --index-url https://download.pytorch.org/whl/cu130 --torch torchvision==0.26.1 >"$TMP_ROOT/install-torch-bad-flag.out" 2>"$TMP_ROOT/install-torch-bad-flag.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected install torch with mismatched package flag to fail" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/install-torch-bad-flag.err" "torch flag must target package 'torch'"

mkdir -p "$TMP_ROOT/ComfyUI/custom_nodes/demo-node"
cat >"$TMP_ROOT/ComfyUI/custom_nodes/demo-node/__init__.py" <<'EOF'
print("demo")
EOF
(
    cd "$TMP_ROOT/ComfyUI/custom_nodes/demo-node"
    git init >/dev/null 2>&1
    git config user.email "tests@example.invalid"
    git config user.name "Test User"
    git add __init__.py
    git commit -m "init" >/dev/null 2>&1
)
cat >"$TMP_ROOT/ComfyUI/custom_nodes/demo-node/__init__.py" <<'EOF'
print("demo runtime snapshot")
EOF
cat >"$TMP_ROOT/ComfyUI/custom_nodes/demo-node/runtime-extra.txt" <<'EOF'
present in working tree but not committed
EOF
mkdir -p "$TMP_ROOT/ComfyUI/custom_nodes/linked-node"
cat >"$TMP_ROOT/ComfyUI/custom_nodes/linked-node/__init__.py" <<'EOF'
print("linked")
EOF
cat >"$TMP_ROOT/ComfyUI/custom_nodes/linked-node/.git" <<'EOF'
gitdir: /tmp/fake-linked-node
EOF
cat >"$WORK_DIR/state/plugins.json" <<'EOF'
[
  {
    "id": "demo-node",
    "git_url": "https://example.invalid/demo-node.git",
    "ref": "main",
    "install_relpath": "custom_nodes/demo-node",
    "group": "node-demo-node",
    "managed_deps": [],
    "enabled": true,
    "created_at": "2026-04-04T00:00:00Z",
    "updated_at": "2026-04-04T00:00:00Z"
  },
  {
    "id": "linked-node",
    "git_url": "https://example.invalid/linked-node.git",
    "ref": "main",
    "install_relpath": "custom_nodes/linked-node",
    "group": "node-linked-node",
    "managed_deps": [],
    "enabled": true,
    "created_at": "2026-04-04T00:00:00Z",
    "updated_at": "2026-04-04T00:00:00Z"
  }
]
EOF

BUNDLE_TAR="$TMP_ROOT/bundle.tar"
PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" env export "$BUNDLE_TAR" >"$TMP_ROOT/env-export.out"
assert_file_exists "$BUNDLE_TAR"
assert_tar_entry_exists "$BUNDLE_TAR" "bundle"
assert_tar_entry_exists "$BUNDLE_TAR" "bundle/manifest.json"
assert_tar_entry_exists "$BUNDLE_TAR" "bundle/pyproject.toml"
assert_tar_entry_exists "$BUNDLE_TAR" "bundle/uv.lock"
assert_tar_entry_exists "$BUNDLE_TAR" "bundle/pylock.toml"
assert_tar_entry_exists "$BUNDLE_TAR" "bundle/state/plugins.json"
assert_tar_entry_exists "$BUNDLE_TAR" "bundle/custom_nodes/demo-node/__init__.py"
assert_tar_entry_exists "$BUNDLE_TAR" "bundle/custom_nodes/demo-node/runtime-extra.txt"
assert_tar_entry_absent "$BUNDLE_TAR" "bundle/custom_nodes/demo-node/.git"
assert_tar_entry_exists "$BUNDLE_TAR" "bundle/custom_nodes/linked-node/__init__.py"
assert_tar_entry_absent "$BUNDLE_TAR" "bundle/custom_nodes/linked-node/.git"
assert_tar_entry_exists "$BUNDLE_TAR" "bundle/audit/prod-freeze.txt"
assert_tar_entry_exists "$BUNDLE_TAR" "bundle/audit/export-summary.json"

set +e
FAKE_UV_EXPORT_FAIL=1 PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" env export "$TMP_ROOT/bundle-export-fail.tar" >"$TMP_ROOT/env-export-fail.out" 2>"$TMP_ROOT/env-export-fail.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected env export failure when uv export fails" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/env-export-fail.err" "export failed"

rm -rf "$TMP_ROOT/ComfyUI/custom_nodes/demo-node"
set +e
PATH="$FAKE_BIN:$PATH" bash "$WORK_DIR/bin/gov" env export "$TMP_ROOT/bundle-missing-node.tar" >"$TMP_ROOT/env-export-missing.out" 2>"$TMP_ROOT/env-export-missing.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected env export failure when plugin source is missing" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/env-export-missing.err" "plugin source directory missing"

copy_fresh_workspace "$TMP_ROOT/import-work"
IMPORT_WORK_DIR="$TMP_ROOT/import-work"
reset_local_state "$IMPORT_WORK_DIR"
IMPORT_COMFY="$TMP_ROOT/ComfyUI-import"
mkdir -p "$IMPORT_COMFY"
PATH="$FAKE_BIN:$PATH" bash "$IMPORT_WORK_DIR/bin/gov" env import "$BUNDLE_TAR" --comfyui-dir "$IMPORT_COMFY" --python 3.12 >"$TMP_ROOT/env-import.out"
assert_file_exists "$IMPORT_WORK_DIR/pyproject.toml"
assert_file_exists "$IMPORT_WORK_DIR/uv.lock"
assert_file_exists "$IMPORT_WORK_DIR/state/plugins.json"
assert_file_exists "$IMPORT_WORK_DIR/.venv-prod/bin/python"
assert_file_exists "$IMPORT_COMFY/custom_nodes/demo-node/__init__.py"
assert_file_exists "$IMPORT_COMFY/custom_nodes/demo-node/runtime-extra.txt"
assert_file_exists "$IMPORT_COMFY/custom_nodes/linked-node/__init__.py"
assert_not_exists "$IMPORT_COMFY/custom_nodes/linked-node/.git"
assert_contains "$IMPORT_WORK_DIR/config.toml" "comfyui_dir = \"$IMPORT_COMFY\""
assert_contains "$IMPORT_WORK_DIR/config.toml" "python = \"3.12\""
assert_contains "$IMPORT_WORK_DIR/state/plugins.json" "\"demo-node\""
assert_contains "$IMPORT_WORK_DIR/state/plugins.json" "\"linked-node\""

REEXPORT_TAR="$TMP_ROOT/bundle-reexport.tar"
PATH="$FAKE_BIN:$PATH" bash "$IMPORT_WORK_DIR/bin/gov" env export "$REEXPORT_TAR" >"$TMP_ROOT/env-reexport.out"
assert_tar_entry_exists "$REEXPORT_TAR" "bundle/custom_nodes/demo-node/__init__.py"
assert_tar_entry_exists "$REEXPORT_TAR" "bundle/custom_nodes/demo-node/runtime-extra.txt"
assert_tar_entry_exists "$REEXPORT_TAR" "bundle/custom_nodes/linked-node/__init__.py"
assert_tar_entry_absent "$REEXPORT_TAR" "bundle/custom_nodes/demo-node/.git"
assert_tar_entry_absent "$REEXPORT_TAR" "bundle/custom_nodes/linked-node/.git"

copy_fresh_workspace "$TMP_ROOT/import-python-mismatch-work"
PY_MISMATCH_WORK_DIR="$TMP_ROOT/import-python-mismatch-work"
reset_local_state "$PY_MISMATCH_WORK_DIR"
PY_MISMATCH_COMFY="$TMP_ROOT/ComfyUI-import-python-mismatch"
mkdir -p "$PY_MISMATCH_COMFY"
set +e
PATH="$FAKE_BIN:$PATH" bash "$PY_MISMATCH_WORK_DIR/bin/gov" env import "$BUNDLE_TAR" --comfyui-dir "$PY_MISMATCH_COMFY" --python 3.11 >"$TMP_ROOT/env-import-python-mismatch.out" 2>"$TMP_ROOT/env-import-python-mismatch.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected env import failure for python minor mismatch" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/env-import-python-mismatch.err" "bundle requires-python is ==3.12.*"
assert_not_exists "$PY_MISMATCH_WORK_DIR/state/plugins.json"

cp "$BUNDLE_TAR" "$TMP_ROOT/bundle-python-spacing.tar"
extract_bundle_tar "$TMP_ROOT/bundle-python-spacing.tar" "$TMP_ROOT/bundle-python-spacing-edit"
python3 - "$TMP_ROOT/bundle-python-spacing-edit/bundle" <<'PY'
import pathlib
import re
import sys

bundle_root = pathlib.Path(sys.argv[1])
project_path = bundle_root / "pyproject.toml"
text = project_path.read_text(encoding="utf-8")
text = re.sub(
    r'requires-python\s*=\s*"==3\.12\.\*"',
    'requires-python = "== 3.12.*"',
    text,
    count=1,
)
project_path.write_text(text, encoding="utf-8")
PY
update_bundle_manifest_checksum "$TMP_ROOT/bundle-python-spacing-edit/bundle" "pyproject.toml"
repack_bundle_tar "$TMP_ROOT/bundle-python-spacing-edit" "$TMP_ROOT/bundle-python-spacing.tar"
copy_fresh_workspace "$TMP_ROOT/import-python-spacing-work"
PY_SPACING_WORK_DIR="$TMP_ROOT/import-python-spacing-work"
reset_local_state "$PY_SPACING_WORK_DIR"
PY_SPACING_COMFY="$TMP_ROOT/ComfyUI-import-python-spacing"
mkdir -p "$PY_SPACING_COMFY"
PATH="$FAKE_BIN:$PATH" bash "$PY_SPACING_WORK_DIR/bin/gov" env import "$TMP_ROOT/bundle-python-spacing.tar" --comfyui-dir "$PY_SPACING_COMFY" --python 3.12 >"$TMP_ROOT/env-import-python-spacing.out"
assert_file_exists "$PY_SPACING_WORK_DIR/state/plugins.json"
assert_file_exists "$PY_SPACING_COMFY/custom_nodes/demo-node/__init__.py"

cp "$BUNDLE_TAR" "$TMP_ROOT/bundle-platform-subset.tar"
extract_bundle_tar "$TMP_ROOT/bundle-platform-subset.tar" "$TMP_ROOT/bundle-platform-subset-edit"
python3 - "$TMP_ROOT/bundle-platform-subset-edit/bundle" <<'PY'
import pathlib
import re
import sys

bundle_root = pathlib.Path(sys.argv[1])
project_path = bundle_root / "pyproject.toml"
text = project_path.read_text(encoding="utf-8")
text = re.sub(
    r"(environments\s*=\s*\[\s*\")([^\"]+)(\",\s*\])",
    rf"\1sys_platform == '{sys.platform}'\3",
    text,
    count=1,
    flags=re.MULTILINE | re.DOTALL,
)
project_path.write_text(text, encoding="utf-8")
PY
update_bundle_manifest_checksum "$TMP_ROOT/bundle-platform-subset-edit/bundle" "pyproject.toml"
repack_bundle_tar "$TMP_ROOT/bundle-platform-subset-edit" "$TMP_ROOT/bundle-platform-subset.tar"
copy_fresh_workspace "$TMP_ROOT/import-platform-subset-work"
PLATFORM_SUBSET_WORK_DIR="$TMP_ROOT/import-platform-subset-work"
reset_local_state "$PLATFORM_SUBSET_WORK_DIR"
PLATFORM_SUBSET_COMFY="$TMP_ROOT/ComfyUI-import-platform-subset"
mkdir -p "$PLATFORM_SUBSET_COMFY"
PATH="$FAKE_BIN:$PATH" bash "$PLATFORM_SUBSET_WORK_DIR/bin/gov" env import "$TMP_ROOT/bundle-platform-subset.tar" --comfyui-dir "$PLATFORM_SUBSET_COMFY" --python 3.12 >"$TMP_ROOT/env-import-platform-subset.out"
assert_file_exists "$PLATFORM_SUBSET_WORK_DIR/state/plugins.json"
assert_file_exists "$PLATFORM_SUBSET_COMFY/custom_nodes/demo-node/__init__.py"

cp "$BUNDLE_TAR" "$TMP_ROOT/bundle-platform-mismatch.tar"
extract_bundle_tar "$TMP_ROOT/bundle-platform-mismatch.tar" "$TMP_ROOT/bundle-platform-mismatch-edit"
python3 - "$TMP_ROOT/bundle-platform-mismatch-edit/bundle" <<'PY'
import pathlib
import re
import sys

bundle_root = pathlib.Path(sys.argv[1])
project_path = bundle_root / "pyproject.toml"
text = project_path.read_text(encoding="utf-8")
text = re.sub(
    r"(environments\s*=\s*\[\s*\")([^\"]+)(\",\s*\])",
    r"\1sys_platform == 'darwin' and platform_machine == 'arm64'\3",
    text,
    count=1,
    flags=re.MULTILINE | re.DOTALL,
)
project_path.write_text(text, encoding="utf-8")
PY
update_bundle_manifest_checksum "$TMP_ROOT/bundle-platform-mismatch-edit/bundle" "pyproject.toml"
repack_bundle_tar "$TMP_ROOT/bundle-platform-mismatch-edit" "$TMP_ROOT/bundle-platform-mismatch.tar"
copy_fresh_workspace "$TMP_ROOT/import-platform-mismatch-work"
PLATFORM_MISMATCH_WORK_DIR="$TMP_ROOT/import-platform-mismatch-work"
reset_local_state "$PLATFORM_MISMATCH_WORK_DIR"
PLATFORM_MISMATCH_COMFY="$TMP_ROOT/ComfyUI-import-platform-mismatch"
mkdir -p "$PLATFORM_MISMATCH_COMFY"
set +e
PATH="$FAKE_BIN:$PATH" bash "$PLATFORM_MISMATCH_WORK_DIR/bin/gov" env import "$TMP_ROOT/bundle-platform-mismatch.tar" --comfyui-dir "$PLATFORM_MISMATCH_COMFY" --python 3.12 >"$TMP_ROOT/env-import-platform-mismatch.out" 2>"$TMP_ROOT/env-import-platform-mismatch.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected env import failure for platform mismatch" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/env-import-platform-mismatch.err" "bundle environments do not support this host"
assert_not_exists "$PLATFORM_MISMATCH_WORK_DIR/state/plugins.json"

cp "$BUNDLE_TAR" "$TMP_ROOT/bundle-corrupt.tar"
extract_bundle_tar "$TMP_ROOT/bundle-corrupt.tar" "$TMP_ROOT/bundle-corrupt-edit"
printf '\n# corrupt\n' >>"$TMP_ROOT/bundle-corrupt-edit/bundle/pyproject.toml"
repack_bundle_tar "$TMP_ROOT/bundle-corrupt-edit" "$TMP_ROOT/bundle-corrupt.tar"
copy_fresh_workspace "$TMP_ROOT/import-corrupt-work"
reset_local_state "$TMP_ROOT/import-corrupt-work"
mkdir -p "$TMP_ROOT/ComfyUI-import-corrupt"
set +e
PATH="$FAKE_BIN:$PATH" bash "$TMP_ROOT/import-corrupt-work/bin/gov" env import "$TMP_ROOT/bundle-corrupt.tar" --comfyui-dir "$TMP_ROOT/ComfyUI-import-corrupt" --python 3.12 >"$TMP_ROOT/env-import-corrupt.out" 2>"$TMP_ROOT/env-import-corrupt.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected env import failure for corrupt bundle" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/env-import-corrupt.err" "bundle checksum mismatch"

cp "$BUNDLE_TAR" "$TMP_ROOT/bundle-missing-file.tar"
extract_bundle_tar "$TMP_ROOT/bundle-missing-file.tar" "$TMP_ROOT/bundle-missing-file-edit"
rm -f "$TMP_ROOT/bundle-missing-file-edit/bundle/pyproject.toml"
repack_bundle_tar "$TMP_ROOT/bundle-missing-file-edit" "$TMP_ROOT/bundle-missing-file.tar"
copy_fresh_workspace "$TMP_ROOT/import-missing-file-work"
reset_local_state "$TMP_ROOT/import-missing-file-work"
mkdir -p "$TMP_ROOT/ComfyUI-import-missing-file"
set +e
PATH="$FAKE_BIN:$PATH" bash "$TMP_ROOT/import-missing-file-work/bin/gov" env import "$TMP_ROOT/bundle-missing-file.tar" --comfyui-dir "$TMP_ROOT/ComfyUI-import-missing-file" --python 3.12 >"$TMP_ROOT/env-import-missing-file.out" 2>"$TMP_ROOT/env-import-missing-file.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected env import failure for bundle missing key file" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/env-import-missing-file.err" "bundle file missing: pyproject.toml"

printf 'not a tar archive\n' >"$TMP_ROOT/not-a-bundle.txt"
copy_fresh_workspace "$TMP_ROOT/import-not-tar-work"
reset_local_state "$TMP_ROOT/import-not-tar-work"
mkdir -p "$TMP_ROOT/ComfyUI-import-not-tar"
set +e
PATH="$FAKE_BIN:$PATH" bash "$TMP_ROOT/import-not-tar-work/bin/gov" env import "$TMP_ROOT/not-a-bundle.txt" --comfyui-dir "$TMP_ROOT/ComfyUI-import-not-tar" --python 3.12 >"$TMP_ROOT/env-import-not-tar.out" 2>"$TMP_ROOT/env-import-not-tar.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected env import failure for non-tar input" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/env-import-not-tar.err" "bundle input must be a .tar file"

python3 - "$TMP_ROOT/bundle-no-top-level.tar" <<'PY'
import io
import pathlib
import tarfile
import sys

tar_path = pathlib.Path(sys.argv[1])
payload = b"demo"
with tarfile.open(tar_path, "w") as archive:
    info = tarfile.TarInfo("wrong")
    info.type = tarfile.DIRTYPE
    archive.addfile(info)
    file_info = tarfile.TarInfo("wrong/manifest.json")
    file_info.size = len(payload)
    archive.addfile(file_info, io.BytesIO(payload))
PY
copy_fresh_workspace "$TMP_ROOT/import-no-top-level-work"
reset_local_state "$TMP_ROOT/import-no-top-level-work"
mkdir -p "$TMP_ROOT/ComfyUI-import-no-top-level"
set +e
PATH="$FAKE_BIN:$PATH" bash "$TMP_ROOT/import-no-top-level-work/bin/gov" env import "$TMP_ROOT/bundle-no-top-level.tar" --comfyui-dir "$TMP_ROOT/ComfyUI-import-no-top-level" --python 3.12 >"$TMP_ROOT/env-import-no-top-level.out" 2>"$TMP_ROOT/env-import-no-top-level.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected env import failure for tar without top-level bundle dir" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/env-import-no-top-level.err" "top-level 'bundle/' directory"

python3 - "$TMP_ROOT/bundle-unsafe-path.tar" <<'PY'
import io
import pathlib
import tarfile
import sys

tar_path = pathlib.Path(sys.argv[1])
payload = b"demo"
with tarfile.open(tar_path, "w") as archive:
    info = tarfile.TarInfo("../escape.txt")
    info.size = len(payload)
    archive.addfile(info, io.BytesIO(payload))
PY
copy_fresh_workspace "$TMP_ROOT/import-unsafe-path-work"
reset_local_state "$TMP_ROOT/import-unsafe-path-work"
mkdir -p "$TMP_ROOT/ComfyUI-import-unsafe-path"
set +e
PATH="$FAKE_BIN:$PATH" bash "$TMP_ROOT/import-unsafe-path-work/bin/gov" env import "$TMP_ROOT/bundle-unsafe-path.tar" --comfyui-dir "$TMP_ROOT/ComfyUI-import-unsafe-path" --python 3.12 >"$TMP_ROOT/env-import-unsafe-path.out" 2>"$TMP_ROOT/env-import-unsafe-path.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected env import failure for unsafe tar path" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/env-import-unsafe-path.err" "bundle tar contains unsafe path"

copy_fresh_workspace "$TMP_ROOT/import-conflict-work"
EXACT_WORK_DIR="$TMP_ROOT/import-conflict-work"
EXACT_COMFY="$TMP_ROOT/ComfyUI-import-conflict"
mkdir -p "$EXACT_WORK_DIR/state" "$EXACT_WORK_DIR/.venv-prod" "$EXACT_COMFY/custom_nodes/demo-node" "$EXACT_COMFY/custom_nodes/legacy-node"
cat >"$EXACT_WORK_DIR/pyproject.toml" <<'EOF'
[project]
name = "original"
version = "0.0.1"
EOF
cat >"$EXACT_WORK_DIR/uv.lock" <<'EOF'
original-lock
EOF
cat >"$EXACT_WORK_DIR/state/plugins.json" <<'EOF'
[
  {
    "id": "legacy-node",
    "git_url": "https://example.invalid/legacy.git",
    "ref": "main",
    "install_relpath": "custom_nodes/legacy-node",
    "group": "node-legacy-node",
    "managed_deps": [],
    "enabled": true,
    "created_at": "2026-04-02T00:00:00Z",
    "updated_at": "2026-04-02T00:00:00Z"
  }
]
EOF
touch "$EXACT_WORK_DIR/.venv-prod/stale.txt"
cat >"$EXACT_COMFY/custom_nodes/demo-node/original.txt" <<'EOF'
old demo node
EOF
cat >"$EXACT_COMFY/custom_nodes/legacy-node/legacy.txt" <<'EOF'
legacy node
EOF
PATH="$FAKE_BIN:$PATH" bash "$EXACT_WORK_DIR/bin/gov" env import "$BUNDLE_TAR" --comfyui-dir "$EXACT_COMFY" --python 3.12 >"$TMP_ROOT/env-import-exact.out"
assert_file_exists "$EXACT_COMFY/custom_nodes/demo-node/__init__.py"
assert_not_exists "$EXACT_COMFY/custom_nodes/legacy-node"
assert_contains "$EXACT_WORK_DIR/state/plugins.json" "\"demo-node\""
assert_not_contains "$EXACT_WORK_DIR/state/plugins.json" "\"legacy-node\""
assert_not_contains "$EXACT_WORK_DIR/pyproject.toml" "name = \"original\""
assert_not_contains "$EXACT_WORK_DIR/uv.lock" "original-lock"

copy_fresh_workspace "$TMP_ROOT/import-relative-work"
RELATIVE_WORK_DIR="$TMP_ROOT/import-relative-work"
reset_local_state "$RELATIVE_WORK_DIR"
mkdir -p "$ROOT_DIR/../relative-comfy"
set +e
(cd "$ROOT_DIR" && PATH="$FAKE_BIN:$PATH" bash "$RELATIVE_WORK_DIR/bin/gov" env import "$BUNDLE_TAR" --comfyui-dir ../relative-comfy --python 3.12 >"$TMP_ROOT/env-import-relative.out" 2>"$TMP_ROOT/env-import-relative.err")
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected env import failure for relative comfyui dir" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/env-import-relative.err" "ComfyUI path must be absolute"

copy_fresh_workspace "$TMP_ROOT/import-lockfail-work"
LOCKFAIL_WORK_DIR="$TMP_ROOT/import-lockfail-work"
reset_local_state "$LOCKFAIL_WORK_DIR"
LOCKFAIL_COMFY="$TMP_ROOT/ComfyUI-import-lockfail"
mkdir -p "$LOCKFAIL_COMFY"
set +e
FAKE_UV_LOCK_CHECK_FAIL=1 PATH="$FAKE_BIN:$PATH" bash "$LOCKFAIL_WORK_DIR/bin/gov" env import "$BUNDLE_TAR" --comfyui-dir "$LOCKFAIL_COMFY" --python 3.12 >"$TMP_ROOT/env-import-lockfail.out" 2>"$TMP_ROOT/env-import-lockfail.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected env import failure when lock check fails" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/env-import-lockfail.err" "bundle lock check failed"
assert_not_exists "$LOCKFAIL_WORK_DIR/state/plugins.json"

copy_fresh_workspace "$TMP_ROOT/import-rollback-work"
ROLLBACK_WORK_DIR="$TMP_ROOT/import-rollback-work"
reset_local_state "$ROLLBACK_WORK_DIR"
ROLLBACK_COMFY="$TMP_ROOT/ComfyUI-import-rollback"
mkdir -p "$ROLLBACK_WORK_DIR/state" "$ROLLBACK_COMFY/custom_nodes/demo-node" "$ROLLBACK_COMFY/custom_nodes/legacy-node"
cat >"$ROLLBACK_WORK_DIR/pyproject.toml" <<'EOF'
[project]
name = "original"
version = "0.0.1"
EOF
cat >"$ROLLBACK_WORK_DIR/uv.lock" <<'EOF'
original-lock
EOF
cat >"$ROLLBACK_WORK_DIR/state/plugins.json" <<'EOF'
[
  {
    "id": "demo-node",
    "git_url": "https://example.invalid/original.git",
    "ref": "main",
    "install_relpath": "custom_nodes/demo-node",
    "group": "node-demo-node",
    "managed_deps": [],
    "enabled": true,
    "created_at": "2026-04-01T00:00:00Z",
    "updated_at": "2026-04-01T00:00:00Z"
  },
  {
    "id": "legacy-node",
    "git_url": "https://example.invalid/legacy.git",
    "ref": "main",
    "install_relpath": "custom_nodes/legacy-node",
    "group": "node-legacy-node",
    "managed_deps": [],
    "enabled": true,
    "created_at": "2026-04-01T00:00:00Z",
    "updated_at": "2026-04-01T00:00:00Z"
  }
]
EOF
cat >"$ROLLBACK_COMFY/custom_nodes/demo-node/original.txt" <<'EOF'
original node
EOF
cat >"$ROLLBACK_COMFY/custom_nodes/legacy-node/legacy.txt" <<'EOF'
legacy node
EOF
set +e
FAKE_PYTHON_EXIT_CODE=1 PATH="$FAKE_BIN:$PATH" bash "$ROLLBACK_WORK_DIR/bin/gov" env import "$BUNDLE_TAR" --comfyui-dir "$ROLLBACK_COMFY" --python 3.12 >"$TMP_ROOT/env-import-rollback.out" 2>"$TMP_ROOT/env-import-rollback.err"
rc=$?
set -e
if [ "$rc" -eq 0 ]; then
    echo "expected env import failure when smoke test fails" >&2
    exit 1
fi
assert_contains "$TMP_ROOT/env-import-rollback.err" "smoke test failed during env import"
assert_contains "$ROLLBACK_WORK_DIR/pyproject.toml" "name = \"original\""
assert_contains "$ROLLBACK_WORK_DIR/uv.lock" "original-lock"
assert_contains "$ROLLBACK_COMFY/custom_nodes/demo-node/original.txt" "original node"
assert_contains "$ROLLBACK_COMFY/custom_nodes/legacy-node/legacy.txt" "legacy node"
assert_contains "$ROLLBACK_WORK_DIR/state/plugins.json" "\"legacy-node\""

echo "test_gov_cli.sh: ok"
