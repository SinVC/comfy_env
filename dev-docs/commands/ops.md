# Operations and undo

[Ledger commands](../../src/application/ops.rs) ·
[Recovery](../../src/application/production.rs) ·
[Undo](../../src/application/undo.rs)

## `gov op list` and `gov op inspect <op_id>`

List operations newest first. Inspect shows the reference, timestamps, pre- and
post-write hashes, backup location, undoability, and any recovery evidence.
The list warns about damaged historical entries and continues. Inspect reads
the requested record strictly. `status` also shows pending operation IDs.

## `gov op recover <op_id>`

Stop the managed production runtime before recovering. Recovery checks that the
recorded owner and any external command have exited.

- A prepared operation has backups and has yet to write production. Close it as
  failed and clean any staged import recovery area.
- An interrupted or failed applying operation restores from the verified backup.
  Restore the recorded source actions if touched. If production sync was
  attempted, use the old configuration and lock to sync the previous venv, or
  remove a venv created by the failed operation.
- An already committed operation may still need its promotion or undo link.
  Finish the ledger link without running production sync again.
- If recovery fails, keep the original error, the recovery error, and the backup.
  Correct the cause and retry. Historical running records without phase evidence
  require manual inspection and retain their backups.

Ordinary apply failures attempt recovery in the same invocation. An operation
that still needs recovery blocks subsequent production writes, and pruning keeps
its backup. Repeating a completed recovery is safe.

## `gov undo <op_id>`

The target must be a successful undoable operation. Compare current truth and
registry hashes with its recorded post-write hashes. Drift blocks the undo.
Back up the current truth and affected source first. Restore the target backup
and sync the production venv. Commit the undo operation before marking the
original operation undone. If the link fails, retry undo or run `op recover`
for the new operation. A successful environment import and a reinitialization
are excluded from ordinary undo.
