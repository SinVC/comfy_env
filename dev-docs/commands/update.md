# Core update transactions

[Implementation](../../src/application/update.rs)

## `gov update run [--requirements-file <path>] [--timeout <seconds>]`

Requires a configured project and a populated Torch group. Read the requested
file or ComfyUI's `requirements.txt`, omitting the Torch family. Record the
requirements file path and hash. Replace the entire staged core group, then
lock. A lock conflict produces `needs_resolution` with a conflict report.

The shared [transaction observation](tx.md) records production and stage hashes,
the production package baseline, pre- and post-probe candidate packages,
logs, and core impact. ComfyUI must reach its native `--quick-test-for-ci` exit.
Reported custom-node load failures make the observation fail and are listed by
inspect. Timeout, interruption, and missing or unsupported probe evidence also
fail. The configured timeout applies unless overridden.

## `gov update inspect <txid>`

Show the transaction evidence and artifact paths as in `tx inspect`, along with
the source requirements file and SHA-256.

## `gov update resolve <txid> [--pin <pkg==version>]... [--pins-file <path>] [--override-requirements]`

Requires a core update awaiting resolution and at least one pin. Restage from
current truth with the recorded core requirements. Merge pins by normalized name,
keeping the last value. A pins file contains one pin per line and may include
blank lines and comments.

Ordinary pins add constraints to the overrides dependency group. For the old
forced behavior of `gov update resolve ID --pin pkg==version`, use
`gov update resolve ID --pin pkg==version --override-requirements`. Force mode
also writes the selected pins to uv's `override-dependencies`. An ordinary lock
conflict retains the updated conflict report and prints the explicit-mode hint.
Any lock success starts a fresh observation. The same snapshot and core
approval checks apply to forced resolutions.

## `gov update abort <txid> [--confirm-stopped]`

Use the [shared abort contract](tx.md) for candidate cleanup, stale running
transactions and verified manual confirmation for old records.

## `gov update promote <txid> [--approve-core] [--reason <text>] [--allow-failed-run] [--keep-artifacts]`

Use the [shared promotion checks](tx.md) and the observed locked snapshot. Apply
the locked core group and run the production sync and ordinary smoke test. The
operation commits before the transaction link is written. An interrupted link
can be repaired by [op recover](ops.md) or by retrying promotion.
