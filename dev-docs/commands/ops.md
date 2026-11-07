# ops (Operations & Undo)

**Implementation targets:** [src/application/ops.rs](../../src/application/ops.rs) for `op list` and `op inspect`; [src/application/undo.rs](../../src/application/undo.rs) for `undo`

## `op list`

### Synopsis

```
gov op list
```

### Purpose

List all operations with their status, kind, reference, and timestamps.

### Reads

- `state/ops/*/meta.json` — all operation records

### Writes

Nothing.

### Success Path

```
1. Scan state/ops/ for subdirectories
2. Load meta.json from each
3. Sort by timestamp (newest first)
4. Display table: op_id, kind, status, reference, started_at
5. If no operations: print informational message
```

---

## `op inspect`

### Synopsis

```
gov op inspect <op_id>
```

### Purpose

Display full details of an operation record.

### Reads

- `state/ops/<op_id>/meta.json`

### Writes

Nothing.

### Success Path

```
1. Load operation metadata
2. Display: op_id, kind, status, reference, timestamps
3. Display: file hashes (pre/post for pyproject.toml, uv.lock, plugins.json)
4. Display: backup directory path
5. Display: undoable flag
6. IF status == "failed": display note/error
7. IF status == "undone": display undo reference
```

---

## `undo`

### Synopsis

```
gov undo <op_id>
```

### Purpose

Revert a successful operation by restoring from its backup. Creates a new operation record tracking the undo itself.

### Arguments

| Flag | Required | Description |
|------|----------|-------------|
| `<op_id>` | Yes | Operation to undo |

### Preconditions

- Operation must exist
- Operation status must be `success`
- Operation must have `undoable == true`
- [Undo Drift Guard Protocol] must pass (current file hashes match operation's post_hashes)

### Reads

- `state/ops/<op_id>/meta.json` — operation record
- `state/ops/<op_id>/backup/` — backup files
- `pyproject.toml`, `uv.lock`, `state/plugins.json` — current truth (for hash check)

### Writes

- `pyproject.toml` — restored from backup
- `uv.lock` — restored from backup
- `state/plugins.json` — restored from backup (if backed up)
- `custom_nodes/*/` — restored if backed up by original operation
- `.venv-prod/` — re-synced to match restored truth
- `state/ops/<op_id>/meta.json` — status updated to "undone"
- `state/ops/<new_op_id>/` — new undo operation record

### Success Path

```
1. Load target operation record
2. Verify status == "success" and undoable == true
3. Execute [Undo Drift Guard Protocol]:
   - Compute current hashes of truth files
   - Compare with target operation's post_hashes
   - IF mismatch: exit with "hash drift detected" error
4. op_begin(kind="manual_undo", reference="undo:<target_op_id>")
5. Restore truth files from target operation's backup:
   - pyproject.toml (respecting existence markers)
   - uv.lock (respecting existence markers)
   - plugins.json (respecting existence markers)
6. Sync prod env via [Prod Sync Protocol]
7. IF original operation backed up custom_nodes:
   - Restore custom_nodes from backup
8. Finalize the new undo operation, then mark the target undone, clear its undoable flag and store the undo reference.
```

### Failure Path

```
IF op not found: exit with error
IF op not undoable or not success: exit with error
IF hash drift detected: exit with error (list mismatched files)
IF restore, prod sync or ledger update fails: restore the undo operation's backup, re-sync prod and finalize the undo operation as failed. Include any recovery failure in its note.
```

### State Transitions

- Target operation: `success` → `undone`
- New operation: `running` → `success` | `failed`

### Platform Notes

- Existence markers are platform-agnostic (simple flag files)
- custom_nodes restoration must handle read-only files on Windows
