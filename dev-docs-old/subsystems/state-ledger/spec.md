# State Ledger Spec

## Metadata

- Status: Active
- Last Reviewed: 2026-03-01

## Scope & Boundary

This subsystem owns the durable local records under `state/`. It defines the persistent shapes and lifecycle expectations for:

- plugin registry (`state/plugins.json`)
- transactions (`state/transactions/*.json`)
- operations (`state/ops/<op_id>/meta.json` plus backup files)
- conflict reports (`state/conflicts/*.json`)

It does not decide whether a mutation is allowed. It provides the persistence contract consumed by the core and safety logic.

## Domain Model

- Plugin Record: metadata for one managed plugin source and its dependency group.
- Transaction Record: one candidate execution plus diff, logs, conflict state, and promotion outcome.
- Operation Record: one destructive change attempt with pre/post hashes and backup directory.
- Conflict Report: summarized lock failure payload to support interactive resolution.

## Use-Case Catalog

- `SL-UC-001` Seed or migrate plugin registry during layout creation.
- `SL-UC-002` Create and rewrite transaction records.
- `SL-UC-003` Create, finalize, inspect, and mark operation records.
- `SL-UC-004` Emit conflict reports and attach them to transactions.

## Key Flows & Failure Recovery

- `state-ledger#KF-001` Initialize plugin registry
  - Trigger: `ensure_layout`, `migrate_plugins_registry`.
  - Success: layout exists and legacy/missing registry is normalized into current shape.
  - Failure: callers stop before relying on missing metadata.
- `state-ledger#KF-002` Persist transaction lifecycle
  - Trigger: `write_tx_file`, `tx_update_status`, `tx_set_*`.
  - Success: transaction transitions remain explicit and queryable.
  - Failure: missing transaction file aborts the caller rather than silently inventing state.
- `state-ledger#KF-003` Persist operation lifecycle
  - Trigger: `op_begin`, `op_finalize`, `cmd_op_list`, `cmd_op_inspect`, `cmd_undo`.
  - Success: every destructive path has backup metadata and post hashes.
  - Failure: missing backups convert the parent flow into explicit failure.

## Internal Components / Collaboration

- Layout initializer: creates required directories and empty registry file.
- Registry normalizer: derives plugin group metadata and keeps `managed_deps` in sync after promote.
- Transaction helpers: write full snapshots and targeted field updates.
- Operation helpers: capture backup files, restore them, and finalize metadata.
- Conflict writer: serializes lock-failure summaries to a durable JSON report.

## State & Lifecycle

- Plugin registry remains a mutable array document, updated in place.
- Transactions are immutable by identity (`txid`) but mutable by status and promote metadata.
- Operations are immutable by identity (`op_id`) but mutable until finalized; a successful undo also mutates the original op.
- Conflict reports are append-only per write and referenced by path from transactions.

## Error Boundary

- Domain errors:
  - missing transaction/op identifiers
  - absent backup directory
- Persistence errors:
  - malformed JSON/TOML tolerated only where code explicitly defaults to safe empty state
- Translation rule:
  - malformed or absent state either normalizes to empty/default during non-critical reads or aborts the caller during critical lookup

## Dependencies

- Allowed:
  - Filesystem reads/writes
  - timestamp and hash helpers
  - embedded Python for JSON/TOML normalization
- Forbidden:
  - `uv` execution
  - `git` execution
  - ComfyUI process control

## Code Anchors

| Doc ID | path | symbol | line |
|---|---|---|---|
| SL-UC-001 | `bin/gov` | `ensure_layout` | 112 |
| SL-UC-001 | `bin/gov` | `migrate_plugins_registry` | 394 |
| SL-UC-002 | `bin/gov` | `write_tx_file` | 1165 |
| SL-UC-002 | `bin/gov` | `load_tx` | 497 |
| SL-UC-002 | `bin/gov` | `tx_update_status` | 734 |
| SL-UC-002 | `bin/gov` | `tx_set_conflict` | 808 |
| SL-UC-002 | `bin/gov` | `tx_set_promoted` | 837 |
| SL-UC-003 | `bin/gov` | `op_begin` | 256 |
| SL-UC-003 | `bin/gov` | `op_restore_backup` | 305 |
| SL-UC-003 | `bin/gov` | `op_finalize` | 318 |
| SL-UC-004 | `bin/gov` | `write_conflict_report` | 993 |

## Internal Contracts

Contracts are split into [contracts.md](./contracts.md) because the same record schemas are consumed by multiple logical modules and by future tooling that may inspect local artifacts without reading the full narrative spec.
