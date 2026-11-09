# Safety Guards Spec

## Metadata

- Status: Active
- Last Reviewed: 2026-03-01

## Scope & Boundary

This subsystem owns the policies and helper mechanics that reduce the chance of corrupting local truth:

- gating core package impact before promotion
- taking backups before destructive changes
- restoring pre-op truth on lock/sync/smoke failure
- blocking undo when current state has drifted

It does not own lock solving or state schema; it consumes both.

## Domain Model

- Core Impact Set: normalized package names considered safety-sensitive.
- Guarded Mutation: a flow that must have backup then finalize semantics.
- Drift Check: comparison between current file hashes and recorded post hashes.
- Conflict Fence: the boundary where lock failure becomes explicit `needs_resolution`.

## Use-Case Catalog

- `SG-UC-001` Reject core-impact promote unless explicitly approved.
- `SG-UC-002` Wrap destructive flows in backup/restore/finalize.
- `SG-UC-003` Prevent undo from overwriting drifted local truth.

## Key Flows & Failure Recovery

- `safety-guards#KF-001` Core impact gate
  - Trigger: preflight inside `cmd_tx_promote`.
  - Success: promote continues only when impact is empty or approved.
  - Failure: command exits before any mutation.
- `safety-guards#KF-002` Backup-first mutation
  - Trigger: `op_begin` before `promote`, `remove`, or `undo`.
  - Success: post hashes are recorded only after the flow settles.
  - Failure: callers restore backups and finalize failed ops.
- `safety-guards#KF-003` Undo drift guard
  - Trigger: `cmd_undo`.
  - Success: undo proceeds only when current hashes match the target op's post hashes.
  - Failure: undo blocks before restore, preserving current drifted state.

## Internal Components / Collaboration

- Sensitive package catalog from config (`policy.core_packages`)
- backup/restore helpers built on State Ledger op records
- conflict report emission when workdir lock cannot complete
- smoke-test validation after prod sync

## State & Lifecycle

- Guard decisions are ephemeral, but they read durable transaction/op state.
- Backup directories remain durable until pruned by retention policy.
- Failed guarded mutations are expected to end either in restored pre-op state or an explicit conflict artifact.

## Error Boundary

- Domain guard failures:
  - missing approval
  - non-undoable target
  - hash divergence
- Recovery failures:
  - missing backup
  - failed re-sync after restore
- Translation rule:
  - if guard preconditions fail, stop before mutation
  - if mutation fails after backup, restore first, then surface failure

## Dependencies

- Allowed:
  - Transaction record inspection
  - Operation backups and hash helpers
  - Smoke command lookup
- Forbidden:
  - Treating restore as optional on destructive-path failures
  - Silent acceptance of hash drift

## Code Anchors

| Doc ID | path | symbol | line |
|---|---|---|---|
| SG-001 | `bin/gov` | `core_packages_csv` | 155 |
| SG-002 | `bin/gov` | `op_begin` | 256 |
| SG-003 | `bin/gov` | `op_restore_backup` | 305 |
| SG-004 | `bin/gov` | `op_finalize` | 318 |
| SG-005 | `bin/gov` | `write_conflict_report` | 993 |
| SG-006 | `bin/gov` | `cmd_undo` | 1079 |
| SG-007 | `bin/gov` | `cmd_tx_promote` | 1772 |

## Internal Contracts

Safety rules are documented in policy files and consume the shared State Ledger contract; they do not expose a separate versioned contract file at this time.
