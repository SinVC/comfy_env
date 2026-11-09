# Application Core Contracts

## Metadata

- Status: Active
- Last Reviewed: 2026-04-17

## Contract List

| Name | Caller | Callee/Implementer | Stability |
|---|---|---|---|
| CLI Command Contract | Local operator | Application Core | Stable for current command set |
| Transaction Control Contract | Application Core | State Ledger + Adapters | Stable, additive fields only |
| Promotion Approval Contract | Local operator + Application Core | Safety Guards | Stable policy surface |
| Runtime Control Contract | Local operator | Application Core + Runtime Executor | Stable |
| Environment Handoff Contract | Local operator | Application Core + Dependency Sync + Source Integration | Stable for v1 bundle shape |

## Input / Output Semantics

- CLI Command Contract
  - Input: top-level verbs and subcommands parsed from `argv`, including exact pin specs for `pin add`, package identifiers for `pin remove`, optional exact torch-family specs on `install torch`, and transactional resolution pins for conflict repair flows.
  - Output: stdout summary for success paths, stderr plus non-zero exit on invalid usage or failed operations.
- Transaction Control Contract
  - Input: `node_id`, optional timeout, `txid`, optional resolution pins, approval flags.
  - Output: durable transaction record updates plus human-readable summaries.
- Promotion Approval Contract
  - Input: computed `core_impact`, `--approve-core`, optional `--reason`, and `--allow-failed-run`.
  - Output: either a blocked promote, a conflict state, or a finalized promote result.
- Runtime Control Contract
  - Input: optional `--sync`, passthrough ComfyUI args, PID file presence.
  - Output: foreground exec into ComfyUI or explicit stop result.
- Environment Handoff Contract
  - Input: `env export <output_tar>` or `env import <bundle_tar> --comfyui-dir --python`, where `--python` can be a canonical minor line or a selector resolvable on the target machine.
  - Output: a verified `.tar` bundle for export, or a staged-and-committed exact restore for import.

## Error Taxonomy

- Usage errors:
  - Missing required identifiers or unknown flags.
- State precondition errors:
  - Missing transaction, missing operation, invalid transaction status, missing plugin metadata, missing PID file.
  - Missing bundle manifest, bundle checksum mismatch, invalid bundle registry, invalid absolute target path, unresolvable Python selector.
  - Removing a package that is not currently pinned in `dependency-groups.overrides`.
- Policy errors:
  - Core package impact without explicit approval.
  - Undo target is not `success && undoable`.
  - Undo hash mismatch against current local truth.
- Validation errors:
  - Non-exact pin specs for `pin add`.
  - Invalid package names for `pin remove`.
  - Torch-family packages rejected from generic `pin add/remove`.
  - Mismatched or non-exact package specs on `install torch --torch*`.
- Adapter-propagated errors:
  - `git` failure, lock conflict, prod sync failure, smoke test failure, missing ComfyUI entrypoint.

## Versioning & Compatibility

- Command names and primary flags are the stable compatibility surface.
- New flags or output fields should be additive.
- Transaction and operation JSON fields can grow, but existing field names should not be repurposed without migration notes.
- Human-readable stdout may grow, but destructive-path guarantees must remain backward compatible with existing operator expectations.

## Event Semantics

- Idempotency Key:
  - Command invocation is not globally idempotent; logical idempotency is approximated by `txid` and `op_id` per persisted artifact.
- Version Field:
  - None at the CLI envelope; versioning relies on stable command grammar plus additive JSON payloads downstream.
- Replay Strategy:
  - Operators may replay read-only commands safely.
  - Mutating commands must be replayed only after checking current state via `status`, `tx inspect`, or `op inspect`.
