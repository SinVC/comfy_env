# Runtime

[Implementation](../../src/application/runtime.rs)

## `gov run [--sync] [-- <args...>]`

Run ComfyUI with the production venv Python and return its exit status. Requires
a configured ComfyUI directory containing `main.py` and an existing production
venv. Configured extra arguments precede the passthrough arguments. The
`run.extra_args` setting accepts an array for exact arguments and a legacy
whitespace-separated string.

Under the production lock, check any previous process record and remove it if stale.
If requested by the flag or `run.sync_before_run`, sync the locked production
environment before starting ComfyUI. The sync uses the operation recovery
ledger and is excluded from ordinary undo. An operation awaiting recovery
blocks startup. Record the launcher PID and host creation identity in
`state/comfyui.pid` before allowing it to start ComfyUI. Release the lock and
wait. On exit, reacquire the lock and remove the matching record after the
managed group or job has emptied. Output goes to the terminal.

## `gov stop`

Read the process record under the same lock. Missing records and identities
that have exited are successful no-op stops. A recycled PID is treated as stale.
A living process from a legacy numeric record is unverified. Stop it manually
after checking its identity. A dead legacy record can be cleaned up normally.

For a verified process, attempt graceful termination, wait up to 30 seconds,
and force termination if needed. The Linux supervisor signals and waits for
members of the production process group. A process that creates its own group
falls outside that scope. Windows launches the production tree inside a Job Object,
attempts Ctrl+Break, and uses the same verified process handle and the Job
Object to force termination if needed. Remove the record after the tree exits.

Commands that write production configuration, truth, source, or the venv hold
the same lock through commit or rollback. Recovery uses it as well. A concurrent
command receives a busy error. When the tracked instance is live, run
`gov stop` before retrying. The gate covers instances started by gov.
