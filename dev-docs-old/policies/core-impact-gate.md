# Core Impact Gate Policy

## Metadata

- Status: Active
- Last Reviewed: 2026-03-01

## Scope

This policy governs promotion when a transaction diff touches packages considered core to the local runtime baseline.

## Rules

- Core impact is computed by normalizing added and removed package names against `policy.core_packages`.
- If the computed impact set is non-empty, `tx promote` must fail closed unless `--approve-core` is present.
- Operators may attach `--reason` to document why the risky promotion is accepted.
- A failed candidate run does not bypass the gate; `--allow-failed-run` and `--approve-core` are independent controls.

## Control Points

- `cmd_tx_promote` preflight before any operation backup is consumed for mutation.
- `write_tx_file` captures `core_impact` during transaction persistence.

## Exceptions

- No automatic exception list exists beyond changing `policy.core_packages` in config.
- Empty impact set proceeds without extra approval.

## Verification

- Run `tx inspect <txid>` and confirm `core_impact`.
- Attempt `tx promote <txid>` without `--approve-core`; it must block when impact is non-empty.
- Re-run with `--approve-core` and optionally `--reason` to allow the guarded path to continue.
