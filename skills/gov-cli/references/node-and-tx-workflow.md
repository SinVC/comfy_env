# Custom Nodes & Candidate Transactions (`tx` & `update`)

All plugin dependency changes and core updates must be staged, locked, and observed in `.venv-candidate/<TX_ID>` before promoting to `.venv-prod`.

---

## 1. End-to-End Custom Node Installation Flow

```bash
# Step 1: Clone & register node source (does NOT install dependencies)
gov node add https://github.com/example/my-node.git --id my-node

# Step 2: Stage requirements, lock, and run candidate startup probe
gov tx run my-node

# Step 3: Inspect results or conflicts
gov tx inspect <TX_ID>

# Step 4 (Only if status == needs_resolution): Supply pins and re-test
gov resolve <TX_ID> --pin numpy==1.26.4 [--override-requirements]

# Step 5: Promote verified candidate lock to .venv-prod (or abort)
gov tx promote <TX_ID> [--approve-core]
```

---

## 2. Transaction State Machine (`<TX_ID>`)

`<TX_ID>` format: `<UTC_TIMESTAMP>-<8_HEX>` (e.g., `20260419T120000Z-a1b2c3d4`).

| Status | Meaning & Next Action |
| :--- | :--- |
| `running` | Lock or candidate observation is executing (or crashed mid-run). Wait, or `abort` once process exits. |
| `needs_resolution` | `uv lock` failed due to conflicts (`state/conflicts/<TX_ID>.json`). Run `gov resolve` / `gov update resolve`. |
| `completed` | Initial `run` passed candidate sync and `--quick-test-for-ci` probe with zero package drift. Ready for `promote`. |
| `resolved` | Conflict resolution locked cleanly and passed candidate observation. Ready for `promote`. |
| `failed` | Sync error, timeout, interruption, probe/node load failure, or runtime `pip install` drift. Inspect logs via `inspect`; abort or fix. |
| `promoted` | Applied to `.venv-prod` (terminal; cannot be aborted, undoable via `gov undo <OP_ID>`). |
| `promote_failed` | Promotion failed and rolled back automatically. Can be inspected or aborted. |
| `aborted` | Candidate venv and staged workdir deleted; JSON record and logs retained. |

---

## 3. Custom Node Source Management

### `gov node add`
Clones a Git repo into `<comfyui_dir>/custom_nodes/<node_id>` and registers it in `state/plugins.json`.

```text
gov node add <GIT_URL> [--ref <REF_NAME>] [--id <ID>]
```

- `--id`: Allowed chars: ASCII letters, digits, `.`, `_`, `-` (not `.` or `..`). Defaults to repo basename without `.git`.
- **Guards**: Rejects duplicate IDs, existing `custom_nodes/<node_id>` directories, unowned `node-*` groups in `pyproject.toml`, and normalized group collisions (e.g., `foo_bar` vs `foo-bar` both map to `node-foo-bar`).
- **Important**: Does **not** install dependencies (follow with `gov tx run <node_id>`). **Not undoable via `gov undo`** (use `gov node remove <id> --purge-code`).

### `gov node remove`
Unregisters a node, removes `dependency-groups.node-<normalized_id>`, re-locks, and syncs `.venv-prod`.

```text
gov node remove <NODE_ID> [--purge-code]
```

- `--purge-code`: Deletes `<comfyui_dir>/custom_nodes/<NODE_ID>` (backed up first; restored if sync/smoke fails). Without it, source files remain on disk.
- Preserves global pins in `dependency-groups.overrides`. Refuses if another registered node shares the dependency group. **Undoable via `gov undo`**.

---

## 4. Candidate Transaction Commands (`Plugin tx` vs `Core update`)

Plugin transactions (`gov tx ...`) and ComfyUI core updates (`gov update ...`) share identical staging, observation, promotion, and abort mechanics:

| Step | Plugin Transaction (`node-<id>`) | Core Update (`dependency-groups.core`) | Key Behavioral Differences |
| :--- | :--- | :--- | :--- |
| **1. Stage & Test** | `gov tx run <NODE_ID> [--timeout <SEC>]` | `gov update run [--requirements-file <PATH>] [--timeout <SEC>]` | `tx run` reads `custom_nodes/<NODE_ID>/requirements.txt` (missing file = empty list) and blocks if `<NODE_ID>` already has a `running` tx. `update run` defaults to `<comfyui_dir>/requirements.txt`, strips Torch-family packages, and fails if any custom node fails to load during the probe. Default timeout: `tx.timeout_seconds` (`120`). |
| **2. Inspect** | `gov tx inspect <TX_ID>` | `gov update inspect <TX_ID>` | Prints diffs, `Core impact`, exit code, logs, node load failures, package drift, and conflict hints. `update inspect` also shows `Source requirements` and `Source SHA256`. |
| **3. Resolve Conflicts** | `gov resolve <TX_ID> [--pin <PKG==VER>]... [--pins-file <FILE>] [--override-requirements]` | `gov update resolve <TX_ID> [--pin <PKG==VER>]... [--pins-file <FILE>] [--override-requirements]` | Both reject Torch-family packages, re-stage from current production truth, write pins to `dependency-groups.overrides` (plus `tool.uv.override-dependencies` only when `--override-requirements` is passed), re-lock, and re-observe. **Difference**: `gov resolve` reads pins interactively from stdin if no `--pin`/`--pins-file` is passed; `gov update resolve` is strictly non-interactive and requires at least one pin flag. |
| **4. Promote to Prod** | `gov tx promote <TX_ID> [OPTIONS]` | `gov update promote <TX_ID> [OPTIONS]` | Identical options and rules (see Promotion Rules below). |
| **5. Abort / Clean** | `gov tx abort <TX_ID> [--confirm-stopped]` | `gov update abort <TX_ID> [--confirm-stopped]` | Deletes `.venv-candidate/<TX_ID>` and staged workdir (idempotent). Refused if `promoted` or pending `gov op recover`. For `running` records, verifies candidate processes exited (`--confirm-stopped` only applies to legacy records missing process identity metadata and cannot override a live process). |

---

## 5. Promotion Rules (`tx promote` & `update promote`)

```text
gov tx promote <TX_ID> [--approve-core] [--reason <REASON>] [--allow-failed-run] [--keep-artifacts]
gov update promote <TX_ID> [--approve-core] [--reason <REASON>] [--allow-failed-run] [--keep-artifacts]
```

- **Baseline Drift Check**: Refuses if root `pyproject.toml`, `uv.lock`, `.venv-prod` packages, resolved Python interpreter, or target group drifted since candidate observation. Re-run `tx run`/`update run` or `resolve` if drift occurred.
- `--approve-core`: **Required** if the transaction adds, upgrades, downgrades, or removes any package in `policy.core_packages` (default: `torch`, `torchvision`, `torchaudio`, `xformers`, `triton`, `onnxruntime`, `onnxruntime-gpu`, `numpy`).
- `--allow-failed-run`: Permits promoting a `failed` transaction **only** when candidate sync succeeded cleanly and the sole failure was a nonzero runtime exit code. Cannot bypass sync failures, timeouts, interruptions, missing `--quick-test-for-ci` probe completion, node load failures, or runtime package drift.
- `--keep-artifacts`: Retains `.venv-candidate/<TX_ID>` and staged workdir after promotion (deleted by default).
- **Undo**: Recorded as an undoable operation (`gov undo <OP_ID>`).
