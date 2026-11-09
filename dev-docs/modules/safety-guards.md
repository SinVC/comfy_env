# safety_guards/

**Implementation target:** [src/safety_guards/](../../src/safety_guards/)

## Responsibility

Enforces all safety rules around destructive mutations. Owns backup creation, restoration, drift detection, policy gating, and smoke testing. Commands call into this module at critical checkpoints — they don't implement safety logic themselves.

## Capabilities

### 1. Backup/Restore/Finalize (Operation Lifecycle)

```rust
fn op_begin(kind: OpKind, reference: &str, root: &Path) -> Result<OpId>
```

1. Generate `OpId`
2. Create `state/ops/<op_id>/backup/`
3. Compute pre-hashes of truth files (pyproject.toml, uv.lock, plugins.json)
4. Copy truth files to backup with existence markers
5. Write initial `meta.json` (status = "running")
6. Return `OpId`

```rust
fn op_finalize(op_id: &OpId, success: bool, root: &Path) -> Result<()>
```

If success:
- Compute post-hashes
- Update meta.json: status = "success", undoable = true

If failure:
- Update meta.json: status = "failed", undoable = false

```rust
fn op_restore(op_id: &OpId, root: &Path) -> Result<()>
```

1. Restore truth files from backup (via existence markers)
2. If custom_nodes were backed up: restore them
3. Return a file restoration error to the caller if any restore step fails.

**Contract**: `op_restore` only restores files. It does NOT call prod sync. The **command layer** is responsible for calling `dependency_sync::sync_prod()` after restore. This avoids a circular dependency between `safety_guards` and `dependency_sync`.

See the [application recovery sequence](application.md#restore-sequence) for sync and failure reporting.

### 2. Undo Drift Guard

```rust
fn check_undo_drift(op_id: &OpId, root: &Path) -> Result<DriftCheck>
```

Returns:
- `DriftCheck::Clean` — current hashes match operation's post_hashes, safe to undo
- `DriftCheck::Drifted(mismatches)` — lists which files have changed since the operation

**Contract**: Undo is blocked if any truth file has been modified since the target operation completed. This prevents silent overwrites of subsequent work.

### 3. Core Impact Gate

```rust
fn check_core_impact(diff: &Diff, config: &RuntimeConfig, flags: &PromoteFlags) -> Result<GateResult>
```

1. Compute `impact_set = normalize(diff.added ∪ diff.removed) ∩ config.policy.core_packages`
2. If empty: return `GateResult::Pass`
3. If non-empty and `flags.approve_core == true`: return `GateResult::Approved(reason)`
4. If non-empty and `flags.approve_core == false`: return `GateResult::Blocked(impact_set)`

**Contract**: Checked before any mutation in promote flows. If blocked, no backup is created and no files are touched.

### 4. Smoke Test

```rust
fn run_smoke_test(env_path: &Path, config: &RuntimeConfig) -> Result<SmokeResult>
```

1. Determine test command:
   - If `config.tx.smoke_test` is set: use structured command (program + args)
   - Else: default to `{program: venv_python(env_path), args: ["-c", "import sys; print(sys.version)"]}`
2. Resolve `program = "python"` through `venv_python(env_path)`
3. Execute with timeout (`config.tx.timeout_seconds`)
4. Return `SmokeResult::Pass` or `SmokeResult::Fail(exit_code | timeout)`

**Contract**: Smoke test failure after prod sync triggers `op_restore`. The caller is responsible for calling restore — this module only reports the result.

### 5. Custom Nodes Backup/Restore

```rust
fn backup_custom_nodes(op_id: &OpId, nodes: &[PathBuf]) -> Result<()>
fn restore_custom_nodes(op_id: &OpId, target_root: &Path) -> Result<()>
```

Used by `node remove`, `env import`, and `undo` when operations affect plugin source directories.

## Invariants

- `op_begin` must always be called before any truth file mutation in destructive commands
- If mutation fails after `op_begin`, `op_restore` must be called before returning
- After `op_restore`, the command layer must call prod sync (restore only restores files)
- Drift guard runs before restore in undo (never blindly overwrite)
- Core impact gate runs before backup in promote (never create backup for a blocked operation)

## Dependencies

- `state_ledger/` — read/write operation records
- `fs_support/` — hashing, backup with markers, restore with markers
- `platform/` — venv Python for smoke test, process execution for smoke test

Note: `safety_guards` does NOT depend on `dependency_sync`. Prod sync after restore is the command layer's responsibility.

## Used By

- `application/pin.rs` — op_begin/finalize/restore, smoke test
- `application/install.rs` — op_begin/finalize/restore, smoke test
- `application/node.rs` — op_begin/finalize/restore, custom_nodes backup
- `application/tx.rs` — core impact gate, op_begin/finalize/restore, smoke test
- `application/update.rs` — core impact gate, op_begin/finalize/restore, smoke test
- `application/env.rs` — op_begin/finalize/restore, custom_nodes backup/restore
- `application/undo.rs` — drift guard, op_begin/finalize/restore
