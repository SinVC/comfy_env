# Runtime

[Implementation](../../src/application/runtime.rs)

## `gov run [--sync] [-- <args...>]`

Run ComfyUI with the production venv Python and return its exit status. Requires
a configured ComfyUI directory containing `main.py` and an existing production
venv. Configured extra arguments precede the passthrough arguments. The
`run.extra_args` setting accepts an array for exact arguments and a legacy
whitespace-separated string.

Under the PID lock, check any previous process record and remove it if stale.
If requested by the flag or `run.sync_before_run`, sync the locked production
environment before starting ComfyUI. An operation awaiting recovery blocks
startup. Record the new process PID and host creation identity in
`state/comfyui.pid`, release the lock, and wait. On exit, reacquire the lock
and remove the matching record. Output goes to the terminal.

## `gov stop`

Read the process record under the same lock. Missing records and identities
that have exited are successful no-op stops. A recycled PID is treated as stale.
A living process from a legacy numeric record is unverified. Stop it manually
after checking its identity. A dead legacy record can be cleaned up normally.

For a verified process, attempt graceful termination, wait up to 30 seconds,
and force termination if needed. Linux uses SIGTERM and SIGKILL through a PID
file descriptor. Windows launches the production tree inside a Job Object,
attempts Ctrl+Break, and uses the same verified process handle and the Job
Object to force termination if needed. Remove the record after the tree exits.

Commands that write production configuration, truth, source, or the venv check
the process record before writing. When the tracked instance is live, run
`gov stop` before retrying. The gate covers instances started by gov.
