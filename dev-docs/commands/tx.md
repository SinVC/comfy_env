# tx (Plugin Transactions)

**Implementation target:** [src/application/tx.rs](../../src/application/tx.rs)

## `tx run`

### Synopsis

```
gov tx run <node_id> [--timeout <seconds>]
```

### Purpose

Create a candidate transaction that observes the dependency impact of a plugin. Installs the plugin's requirements into a candidate environment and runs ComfyUI to verify compatibility.

### Arguments

| Flag | Required | Description |
|------|----------|-------------|
| `<node_id>` | Yes | Plugin identifier (must be registered) |
| `--timeout` | No | Override timeout in seconds (default: config tx.timeout_seconds) |

### Preconditions

- Plugin must exist in `plugins.json`
- Plugin source directory must exist (`custom_nodes/<node_id>/`)
- A missing plugin requirements.txt contributes an empty dependency set.
- `config.toml` must exist
- No existing `running` transaction for this node (prevents duplicates)

### Reads

- `config.toml` — python, candidate_root, comfyui_dir, timeout
- `state/plugins.json` — plugin record (install path, group name)
- Plugin's `requirements.txt` — dependency specs
- `pyproject.toml`, `uv.lock` — current truth for staging

### Writes

- `state/transactions/<txid>.json` — new transaction record
- `.venv-candidate/<txid>/` — candidate environment
- `state/work/<workdir>/` — staged workdir for dependency resolution
- `state/logs/<txid>.stdout.log`, `state/logs/<txid>.stderr.log` — run output

### Success Path

```
1. Generate txid
2. Load plugin record from plugins.json
3. Read plugin requirements (from plugin source dir)
4. Create transaction record (status="running", kind="plugin")
5. Copy truth files to staged workdir
6. Add plugin requirements to workdir pyproject.toml:
   - Create/update dependency-groups.<group_name>
   - Write the whole dependency group once.
7. Lock workdir: `uv lock --python <py>`
8. Create candidate env: sync workdir to .venv-candidate/<txid>/
9. Capture pre_freeze from the current production environment as the comparison baseline.
10. Run ComfyUI in candidate env:
    - Command: venv_python <comfyui_dir>/main.py
    - With timeout
    - Mirror stdout/stderr to the terminal in real time
    - Capture the same stdout/stderr to log files
    - Record exit code
11. Capture post-freeze: `uv pip freeze` in candidate env
12. Compute diff (added/removed packages between pre and post freeze)
13. Compute core_impact: diff ∩ policy.core_packages
14. Update transaction record:
    - status = "completed" (if exit 0 or observation timeout)
    - status = "failed" (if non-zero runtime failure or candidate sync failure)
    - pre_freeze, post_freeze, diff, core_impact, logs, run_exit_code
```

### Failure Path

```
IF plugin not found: exit with precondition error
IF lock fails during workdir staging:
  - Write conflict artifact via [Lock Conflict Protocol]
  - Update tx status → "needs_resolution"
  - Return (not a fatal error — user resolves then re-promotes)
IF candidate sync fails: mark failed, retain diagnostics and require a new successful candidate setup before promotion.
IF ComfyUI run times out: record RunOutcome::TimedOut, preserve run_exit_code, status → "completed"
IF ComfyUI run exits non-zero: record exit code, status → "failed"
```

### State Transitions

- Creates: transaction `running` → `completed` | `failed` | `needs_resolution`
- `running` → `needs_resolution`: lock fails during workdir staging (before ComfyUI runs)
- `running` → `completed`: ComfyUI exits 0 or reaches the observation timeout and is cleanly terminated
- `running` → `failed`: ComfyUI exits non-zero or candidate sync/run setup fails

### Cleanup Ownership

- **Candidate env** (`.venv-candidate/<txid>/`): cleaned up by `tx abort` or by successful `tx promote` by default. `tx promote --keep-artifacts` preserves it. Not cleaned on `tx run` failure — preserved for inspection.
- **Staged workdir** (`state/work/...`): cleaned up by `tx abort` or by successful `tx promote` by default. `tx promote --keep-artifacts` preserves it. Not cleaned on `needs_resolution` — needed for resolve retry.

### Platform Notes

- Candidate env Python located via `venv_python()`
- Timeout uses platform-native child process timeout (not Unix `timeout` command)
- Log capture uses stdout/stderr pipes (cross-platform)
- Candidate output is a dual channel: terminal visibility for the operator and durable tx log files for audit

---

## `tx inspect`

### Synopsis

```
gov tx inspect <txid>
```

### Purpose

Display the full details of a transaction record.

### Preconditions

- Transaction must exist in `state/transactions/`

### Reads

- `state/transactions/<txid>.json`
- Optionally: `state/conflicts/<txid>.json` (if status is needs_resolution)

### Writes

Nothing.

### Success Path

```
1. Load transaction record
2. Display: txid, kind, subject, status, timestamps
3. Display: diff (added/removed packages)
4. Display: core_impact (if non-empty)
5. Display: run logs summary (exit code, timeout status)
6. Display artifact paths (`candidate_env`, `staged_workdir`) with presence annotation:
   - `(cleaned)` when the path is absent after a promoted/aborted transaction
   - `(missing)` when the path is absent in any other state
7. IF needs_resolution: display conflict summary and resolution hint
8. IF promoted: display promotion details (op_id, reason)
```

---

## `tx abort`

### Synopsis

```
gov tx abort <txid>
```

### Purpose

Cancel a transaction and clean up its candidate environment.

### Preconditions

- Transaction must exist
- Transaction status must be one of: `completed`, `failed`, `needs_resolution`, `resolved`, `promote_failed`
- Cannot abort an already `promoted` or `aborted` transaction

### Reads

- `state/transactions/<txid>.json`

### Writes

- `state/transactions/<txid>.json` — status updated to "aborted"
- `.venv-candidate/<txid>/` — deleted
- `state/work/<workdir>/` — deleted (if exists)

### Success Path

```
1. Load transaction record
2. Verify status is abortable
3. Delete candidate environment directory
4. Delete staged workdir (if exists)
5. Update transaction status → "aborted"
```

---

## `tx promote`

### Synopsis

```
gov tx promote <txid> [--approve-core --reason "<text>"] [--allow-failed-run] [--keep-artifacts]
```

### Purpose

Promote a completed transaction's dependency changes into production truth and sync the prod env.

### Arguments

| Flag | Required | Description |
|------|----------|-------------|
| `<txid>` | Yes | Transaction to promote |
| `--approve-core` | Conditional | Required if core_impact is non-empty |
| `--reason` | No | Documentation for core approval |
| `--allow-failed-run` | No | Allow promoting even if run status was "failed" |
| `--keep-artifacts` | No | Preserve `candidate_env` and `staged_workdir` after a successful promote |

### Preconditions

- Transaction must exist with status `completed`, `resolved`, or `failed`
- `--allow-failed-run` applies only to a recorded nonzero ComfyUI run after candidate setup succeeded. A failed sync has no runtime observation to approve.
- Core impact gate: if core_impact non-empty, requires `--approve-core`

### Reads

- `state/transactions/<txid>.json` — diff, core_impact, staged workdir path
- `config.toml` — python, prod env, core_packages
- `pyproject.toml`, `uv.lock` — current production truth

### Writes

- `pyproject.toml` — adds plugin dependency group entries
- `uv.lock` — re-locked with new dependencies
- `.venv-prod/` — synced
- `state/transactions/<txid>.json` — status → "promoted", promotion payload
- `state/ops/<op_id>/` — new operation record

### Success Path

```
1. Load transaction record
2. Verify status is promotable
3. IF run was failed AND no --allow-failed-run: exit with policy error
4. Build the promotion plan from the plugin requirements, observed additions and resolution pins.
5. Apply that plan once to a fresh staging copy of the current production truth.
6. Lock the workdir. On conflict, store the conflict and mark the transaction needs_resolution.
7. Compare the resolved changes to current prod and apply the core impact gate to the plan being promoted.
8. op_begin(kind="plugin_promote", reference=txid)
9. Copy workdir truth → root (pyproject.toml, uv.lock)
10. Sync prod via [Prod Sync Protocol] and run the smoke command.
11. Update managed_deps in plugins.json, finalize the operation and record the promotion op_id on the transaction.
12. IF `--keep-artifacts` is not set:
    - Clean up candidate env and staged workdir
    - If cleanup fails, emit warning only; do not change promote success
```

### Failure Path

```
IF policy gate blocks: exit before mutation
IF staging lock conflicts: set needs_resolution and return before the production checkpoint
IF sync fails: op_restore → re-sync prod → op_finalize(failed)
    → tx status → "promote_failed"
IF smoke fails: op_restore → re-sync prod → op_finalize(failed)
    → tx status → "promote_failed"
```

### State Transitions

- Successful, resolved and explicitly approved failed runtime observations may transition to `promoted`, `needs_resolution` or `promote_failed`.
- Operation: created `running` → `success` | `failed`
