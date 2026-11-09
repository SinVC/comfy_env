# Shared Types

## Metadata

- Status: Active
- Last Reviewed: 2026-03-01

## Identifier Set

- `node_id`: logical plugin identifier derived from explicit `--id` or Git repo basename.
- `txid`: UTC timestamp plus short UUID suffix, used as the stable key for one transaction.
- `op_id`: UTC timestamp plus `-op-` plus short UUID suffix, used as the stable key for one operation.

## Transaction Status Vocabulary

- `running`
- `completed`
- `failed`
- `aborted`
- `needs_resolution`
- `resolved`
- `promoted`
- `promote_failed`

## Operation Status Vocabulary

- `running`
- `success`
- `failed`
- `undone`

## Shared Record Fields

- timestamps are UTC strings from `timestamp_utc()`
- file integrity uses SHA-256 hashes
- path references inside records are local filesystem paths, not remote URIs

## Compatibility Notes

- Readers should tolerate additive fields in JSON records.
- Consumers should treat empty strings as "not set yet" for mutable lifecycle fields such as `ended_at`, `conflict_report`, and `promotion.error`.
