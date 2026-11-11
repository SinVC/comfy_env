# Operation Ledger, Crash Recovery & Undo (`op` & `undo`)

Every production mutation is recorded in `state/ops/` with an `<OP_ID>` formatted as `<UTC_TIMESTAMP>-op-<8_HEX>` (e.g., `20260419T120000Z-op-e5f6a7b8`).

> **When to use `op recover` vs. `undo`**:
> - Use `gov op recover <OP_ID>` when a previous command **crashed or was interrupted mid-execution** (`needs_recovery` is flagged in `gov status` or `gov op list`, blocking all new writes and `gov run`).
> - Use `gov undo <OP_ID>` when a previous command **completed (`success`)** and you want to revert its changes back to the pre-operation snapshot.

---

## 1. Auditing Operations (`op list` & `op inspect`)

### `gov op list`
Lists recorded production operations, newest first.

```text
gov op list
```

- Outputs tab-separated columns: `op_id`, `kind`, `status`, `reference`, `started_at`, and `needs_recovery` (when recovery is required). Warns and skips damaged historical records.

### `gov op inspect`
Displays detailed metadata, pre/post SHA-256 hashes, backup directory path, and recovery/undo status for one operation.

```text
gov op inspect <OP_ID>
```

---

## 2. Crash Recovery (`gov op recover`)

Resolves an interrupted production operation and lifts the Recovery Gate.

```text
gov op recover <OP_ID>
```

- **Preconditions**: ComfyUI must be stopped (`gov stop`), and the original operation owner and external subprocesses must have exited.
- **Behavior by Interruption Point**:
  - **Interrupted before commit** (`prepared` / `applying` / failed restore): Restores backed-up truth (`pyproject.toml`, `uv.lock`), `config.toml`, `state/plugins.json`, and any moved node sources, then re-syncs (or removes a newly created) `.venv-prod`.
  - **Committed, missing ledger link**: Finalizes the ledger link (marks the linked transaction `promoted` or the target operation `undone`) without re-running `uv sync`.
- **Idempotency**: Safe to repeat after recovery finishes. Legacy `running` records lacking phase metadata preserve backups and require manual verification.

---

## 3. Reverting Completed Changes (`gov undo`)

Restores the pre-operation backup of a completed (`success`) operation and re-syncs `.venv-prod`.

```text
gov undo <OP_ID>
```

- **Drift Guard**: Verifies that current SHA-256 hashes of `pyproject.toml`, `uv.lock`, and `state/plugins.json` match the operation's recorded `post_sha256`. If any subsequent modification occurred after `<OP_ID>`, `gov undo` refuses to run (you must undo newer operations in reverse chronological order first).
- **Restore Scope**: Restores `pyproject.toml`, `uv.lock`, `state/plugins.json`, and custom-node source files (if the operation modified or `--purge-code` deleted them), re-syncs `.venv-prod`, and marks `<OP_ID>` as `undone`.
- **Undo Eligibility Matrix**:

| Undoable (`undoable: true`) | NOT Undoable (Excluded from `gov undo`) | Alternative for Non-Undoable |
| :--- | :--- | :--- |
| `gov install torch` | `gov init` / re-init | Re-run `gov init` with desired settings |
| `gov install` | `gov node add` | `gov node remove <NODE_ID> --purge-code` |
| `gov pin add` / `gov pin remove` | `gov env import` | Import a previous bundle or re-configure |
| `gov node remove` (with or without `--purge-code`) | `gov run --sync` (`runtime_sync`) | No truth files changed |
| `gov tx promote` / `gov update promote` | | |
