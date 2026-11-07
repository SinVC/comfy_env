# resolve (Plugin Conflict Resolution)

**Implementation target:** [src/application/resolve.rs](../../src/application/resolve.rs)

## Synopsis

```
gov resolve <txid> [--pin <pkg==version>]...
```

## Purpose

Resolve a lock conflict in a plugin transaction by providing additional pins. Unlike `update resolve`, this command also supports interactive input (prompts for pins if none provided via flags).

## Arguments

| Flag | Required | Description |
|------|----------|-------------|
| `<txid>` | Yes | Transaction to resolve |
| `--pin` | No | Pin specs (repeatable) |

If no `--pin` flags provided, enters interactive mode: displays conflict summary and prompts for `pkg==version` entries (one per line, empty line to finish).

## Preconditions

- Transaction must exist with status `needs_resolution`
- Transaction kind must be `plugin`

## Reads

- `state/transactions/<txid>.json`
- `state/conflicts/<txid>.json` — conflict report (for display)
- `pyproject.toml`, `uv.lock` — current truth

## Writes

- `state/transactions/<txid>.json` — resolution_pins merged, status may change
- `state/work/<workdir>/` — re-staged with resolution pins

## Success Path

```
1. Load transaction record
2. Verify status == "needs_resolution" and kind == "plugin"
3. Collect pins:
   a. From --pin flags if provided
   b. ELSE: display conflict summary, prompt user for pins interactively
4. Merge new pins into transaction's resolution_pins (last-wins dedup)
5. Copy truth to staged workdir
6. Apply original plugin deps + all resolution pins to workdir:
   - Re-add plugin group entries
   - Add/update resolution pins in overrides group
7. Lock workdir: `uv lock --python <py>`
8. IF lock succeeds:
   - Sync and observe the resolved candidate and refresh diff/core_impact before marking it resolved. A candidate setup failure stays failed.
9. IF lock fails:
   - Write updated conflict artifact
   - tx remains "needs_resolution"
   - Display updated conflict info
```

## Failure Path

No backup/restore needed — `resolve` only mutates the staged workdir and transaction record, not production truth.

## Compatibility Notes

- Bash era: interactive mode reads from stdin line-by-line
- Rust era: same interactive behavior preserved; non-interactive mode uses `--pin` flags
- Future consideration: `--pins-file` flag for consistency with `update resolve`
