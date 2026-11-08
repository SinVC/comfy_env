# Command orchestration

[Implementation](../../src/application/) · [CLI](../../src/cli.rs)

Commands own validation, staging, and user-facing output. Subprocess clients
own command construction. The state ledger owns record I/O. Platform and
filesystem modules handle process identity, atomic writes, and tree operations.

## Production mutations

The shared `production::commit` path creates a prepared operation after command
validation and staging. A command can prepare additional configuration or
source backups. Verify the backup before recording the applying phase. Then
run command-specific writes, locked production sync, and any required smoke
test. Sync and smoke commands persist their external process identity before
execution. Source movement is recorded before it occurs.

Only after all production writes succeed does the operation become committed.
Promotion and undo links follow that commit and can be retried independently.
Both promotion commands consume a previously observed stage. Pin, install,
node removal, and reinit stage and lock before the production checkpoint.
Import checks the bundle and prepares its node movement journal. Undo restores
from the existing operation's backup.

A failed apply restores only the resources that may have been touched. Use the
previous configuration to sync the previous environment when needed. Keep
apply and restore errors in the ledger. Pending recovery blocks later
production writes. The same restore path runs through `gov op recover`.

## Candidate and read-only commands

Plugin and core transactions share staging, snapshot verification, and
inspection. They save the owner before staging becomes interruptible, sync an
isolated candidate, run the startup probe, and record one observation.
Resolution starts a fresh attempt before locking. Abort verifies recorded
process identities and artifact ownership.

Status and lists can skip and warn about malformed historical records.
Single-record commands and recovery are strict. Configuration reads and uv
client creation are independent, so node registration and abort do not depend
on a configured Python.
