# Exemptions

## Metadata

- Status: Active
- Last Reviewed: 2026-03-01

## Legacy Docs Boundary

- Existing `docs/` files remain source material and operational references.
- `dev-docs/architecture-haiku.md` is now the canonical architecture entrypoint for architecture navigation.
- If content conflicts, code and `dev-docs/` should be treated as the authoritative pair to maintain going forward.

## Structural Exemptions

- The implementation is currently a single shell script, so subsystem boundaries in `dev-docs/` are logical architecture slices, not separate code packages.
- No module-specific `key-flows.md` files are created yet because current module flow volume stays manageable inside each `spec.md`.
- No ADR documents exist yet; `dev-docs/adr/` is reserved for future architectural decisions that require durable records.

## Future Tightening

- If `bin/gov` is split into multiple files, update `code-map.md` and rebalance subsystem anchors from logical to physical ownership.
- If automation starts consuming these docs, add link checking and anchor validation as part of repository quality gates.
