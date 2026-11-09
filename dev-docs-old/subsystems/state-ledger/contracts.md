# State Ledger Contracts

## Metadata

- Status: Active
- Last Reviewed: 2026-03-01

## Contract List

| Name | Caller | Callee/Implementer | Stability |
|---|---|---|---|
| Plugin Registry Contract | Application Core, Source Integration | State Ledger | Stable, additive fields only |
| Transaction Record Contract | Application Core, Safety Guards, Dependency Sync | State Ledger | Stable, additive fields only |
| Operation Record Contract | Application Core, Safety Guards | State Ledger | Stable, additive fields only |
| Conflict Report Contract | Application Core, Resolve flow | State Ledger | Stable |

## Input / Output Semantics

- Plugin Registry Contract
  - Input: plugin identity, Git source metadata, install path, normalized group, managed dependency snapshot.
  - Output: one array element per plugin in `state/plugins.json`.
- Transaction Record Contract
  - Input:
    - `txid`
    - `kind` (`plugin` | `core_update`)
    - `subject`
    - `node_id`
    - candidate env path
    - pre/post freeze snapshots
    - diff
    - logs
    - status
    - resolution pins
    - promotion payload
    - optional `source_requirements_path`
    - optional `source_requirements_sha256`
    - optional `staged_workdir`
    - optional `target_core_deps`
  - Output: `state/transactions/<txid>.json`.
- Operation Record Contract
  - Input: `op_id`, kind, reference, backup directory, pre/post hashes, status, note, undoable flag.
  - Output: `state/ops/<op_id>/meta.json` plus backup file copies.
- Conflict Report Contract
  - Input: `txid`, `node_id` (plugin id or `core`), lock log path, created timestamp.
  - Output: `state/conflicts/<txid>.json` containing a summary, detected packages, and input hint.

## Error Taxonomy

- Lookup miss:
  - transaction not found
  - operation not found
  - plugin metadata missing
- Backup integrity errors:
  - target backup directory missing during restore
- Parse tolerance:
  - some reads default malformed JSON/TOML to safe empty values
  - critical lookups still fail closed when exact record presence is required

## Versioning & Compatibility

- Record schemas are local but should preserve additive compatibility.
- Unknown fields must be tolerated by readers.
- Existing field names should not be removed or redefined without migration logic.
- Historical transactions that do not carry `kind` must be treated as `plugin`.
- `managed_deps` is explicitly secondary to dependency-group truth in `pyproject.toml`; consumers must treat it as cache, not authority.

## Event Semantics

- Idempotency Key:
  - `txid` for transaction records
  - `op_id` for operation records
  - plugin `id` for registry upserts
- Version Field:
  - No explicit schema version field; compatibility is maintained structurally by additive change only.
- Replay Strategy:
  - Rewriting the same `txid` or `op_id` is allowed only as lifecycle progression inside the owning flow.
  - Callers must not fabricate records outside the documented helper functions.
