# state_ledger/

**Implementation target:** [src/state_ledger/](../../src/state_ledger/)

## Responsibility

CRUD operations for all durable state files. Owns the JSON schemas and lifecycle rules for transactions, operations, plugins, and conflict artifacts. No other module reads or writes these files directly.

## Capabilities

### 1. Transaction Records

```rust
fn create_transaction(tx: &TransactionRecord) -> Result<()>
fn load_transaction(txid: &TxId) -> Result<TransactionRecord>
fn update_transaction_status(txid: &TxId, status: TxStatus) -> Result<()>
fn update_transaction_field(txid: &TxId, field: TxField, value: JsonValue) -> Result<()>
fn list_transactions() -> Result<Vec<TransactionRecord>>
```

**Schema** (`state/transactions/<txid>.json`):

```json
{
  "txid": "20260419T120000Z-a1b2c3d4",
  "kind": "plugin | core_update",
  "subject": "node_id or core",
  "node_id": "optional, for plugin kind",
  "status": "running|completed|failed|needs_resolution|resolved|promoted|promote_failed|aborted",
  "started_at": "ISO8601",
  "ended_at": "ISO8601 or null",
  "candidate_env": "/path/to/.venv-candidate/txid",
  "staged_workdir": "/path/to/state/work/...",
  "pre_freeze": ["pkg==1.0", ...],
  "post_freeze": ["pkg==1.0", ...],
  "diff": { "added": [...], "removed": [...] },
  "core_impact": ["torch", "numpy"],
  "logs": { "stdout": "path", "stderr": "path", "run_exit_code": 0 },
  "base_truth_sha256": { "pyproject.toml": "...", "uv.lock": "..." },
  "source_requirements_path": "optional, core_update only",
  "source_requirements_sha256": "optional",
  "conflict_report": "optional path",
  "resolution_pins": ["pkg==1.2.3"],
  "promotion": {
    "status": "promoted|promote_failed",
    "reason": "optional",
    "op_id": "op_id",
    "error": "optional"
  }
}
```

**Invariants**:
- `txid` is immutable once created
- Core updates record `base_truth_sha256` when staging. For an older transaction with no recorded baseline, start a fresh update before promotion.
- Observation fields are populated after candidate execution and refreshed after resolution. The recorded production baseline stays fixed.
- `candidate_env` and `staged_workdir` retain their original path strings even if the referenced directories are later cleaned after `promote` or `abort`
- Read path must tolerate missing optional fields (schema evolution)

**Minimal fields at creation** (all others are null/empty until populated):
- `txid`, `kind`, `subject`, `status` ("running"), `started_at`, `candidate_env`, `staged_workdir`
- For plugin kind: also `node_id`

**Fields populated after run completes**:
- `pre_freeze`, `post_freeze`, `diff`, `core_impact`, `logs`, `ended_at`, `status` update

**Fields populated during promote/resolve**:
- `resolution_pins` (resolve), `promotion` (promote), `conflict_report` (lock failure)
- Inspect/read layers may annotate missing artifact paths as `(cleaned)` for promoted/aborted transactions or `(missing)` for other states, but they do not rewrite the stored JSON fields

### 2. Operation Records

```rust
fn create_operation(op: &OperationRecord) -> Result<OpId>
fn load_operation(op_id: &OpId) -> Result<OperationRecord>
fn update_operation(op_id: &OpId, update: OpUpdate) -> Result<()>
fn list_operations() -> Result<Vec<OperationRecord>>
fn prune_operations(retention_count: u32) -> Result<u32>
```

**Schema** (`state/ops/<op_id>/meta.json`):

```json
{
  "op_id": "20260419T120000Z-op-e5f6a7b8",
  "kind": "install_torch|install_core|pin_add|pin_remove|node_remove|plugin_promote|core_promote|update_promote|env_import|manual_undo",
  "reference": "txid or node_id or description",
  "status": "running|success|failed|undone",
  "started_at": "ISO8601",
  "ended_at": "ISO8601 or null",
  "files": {
    "pyproject.toml": { "pre_sha256": "...", "post_sha256": "..." },
    "uv.lock": { "pre_sha256": "...", "post_sha256": "..." },
    "plugins.json": { "pre_sha256": "...", "post_sha256": "..." }
  },
  "backup_dir": "/path/to/ops/op_id/backup",
  "undoable": true,
  "note": "optional"
}
```

**Invariants**:
- `op_id` is immutable
- `status` transitions: `running` → `success`|`failed`, `success` → `undone`
- Backup directory is co-located: `state/ops/<op_id>/backup/`
- Following a successful mutation or undo, prune the oldest finished operations by start time beyond retention_count (default 100, minimum 1). Keep running records and report pruning errors as warnings.

### 3. Plugin Registry

```rust
fn load_plugins() -> Result<Vec<PluginRecord>>
fn add_plugin(plugin: &PluginRecord) -> Result<()>
fn remove_plugin(node_id: &NodeId) -> Result<()>
fn update_plugin(node_id: &NodeId, update: PluginUpdate) -> Result<()>
```

**Schema** (`state/plugins.json`):

```json
[
  {
    "id": "comfyui-manager",
    "git_url": "https://github.com/...",
    "ref": "main",
    "install_relpath": "custom_nodes/comfyui-manager",
    "group": "node-comfyui-manager",
    "enabled": true,
    "managed_deps": ["dep1", "dep2"],
    "created_at": "ISO8601",
    "updated_at": "ISO8601"
  }
]
```

**Invariants**:
- `id` is unique within the array
- `install_relpath` uses forward slashes (platform-normalized via `platform/`)
- `managed_deps` is informational only — it records which packages were added to the plugin's dependency group during promote. It is NOT authoritative for dependencies; `pyproject.toml` dependency-groups is the source of truth. `managed_deps` exists for display purposes (`status`, `op inspect`) and to assist `node remove` in knowing which packages to clean up.

### 4. Conflict Artifacts

```rust
fn write_conflict(txid: &TxId, info: &ConflictInfo) -> Result<PathBuf>
fn load_conflict(txid: &TxId) -> Result<Option<ConflictReport>>
```

**Schema** (`state/conflicts/<txid>.json`):

```json
{
  "txid": "...",
  "node_id": "node_id or core",
  "created_at": "ISO8601",
  "raw_log": "state/conflicts/<txid>.lock.log",
  "summary": "first 40 lines of lock output",
  "detected_packages": ["pkg1", "pkg2"],
  "input_hint": "Use: gov resolve <txid> --pin pkg==version"
}
```

## Schema Evolution Rules

- New fields: append-only, optional, with sensible defaults on read
- No field renames
- No field type changes
- Reader must tolerate: missing optional fields, unknown extra fields
- Writer outputs canonical format (all known fields, null for unset optionals)

## Dependencies

- `serde` + `serde_json` (JSON serialization)
- `fs_support/` (atomic writes for JSON files)
- `platform/` (path normalization in records)

## Used By

- `application/` (all commands that create/read/update state)
- `safety_guards/` (reads operations for undo, reads transactions for gating)
