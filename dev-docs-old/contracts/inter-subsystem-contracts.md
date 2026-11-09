# Inter-Subsystem Contracts

## Metadata

- Status: Active
- Last Reviewed: 2026-03-01

## Purpose

This file is a compatibility index only. It does not own contract source-of-truth content.

## Contract Index

| Contract | Source Document | Summary | Link |
|---|---|---|---|
| CLI command surface | `application-core/contracts.md` | User-facing command grammar and failure classes | [application-core/contracts.md](../application-core/contracts.md) |
| Transaction and operation records | `subsystems/state-ledger/contracts.md` | Durable local artifact shapes consumed across flows | [state-ledger/contracts.md](../subsystems/state-ledger/contracts.md) |
| Shared identifiers and status enums | `contracts/shared-types.md` | Common naming and state vocab | [shared-types.md](./shared-types.md) |

## Jump Table

- Application Core -> State Ledger: persists and reads transaction/op/plugin artifacts
- Application Core -> Dependency Sync: turns approved plans into lock/sync side effects
- Application Core -> Source Integration: acquires and maps plugin source trees
- Application Core -> Runtime Executor: runs candidate/prod ComfyUI processes
- Safety Guards -> State Ledger: validates hashes and writes conflict links
