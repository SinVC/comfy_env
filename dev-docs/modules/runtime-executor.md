# Runtime executor

[Implementation](../../src/runtime_executor/)

The runtime executor starts and reaps ComfyUI with platform-specific process
control. Production launch, stop, writes and recovery share the production lock.
PID records contain the PID and host creation identity. Linux uses a supervisor
as process group leader and verifies its boot ID and start time before signaling
it through a PID file descriptor. The supervisor remains until the group empties.
Windows reads process creation time and
verifies and terminates through the same process handle. The Windows
production launcher waits for the PID record before starting the interpreter
inside a Job Object. Stop waits for that job to empty, including when venv
Python creates a separate interpreter process. Live numeric legacy records
require operator verification. See [run and stop](../commands/runtime.md) and
[Windows Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects).

A candidate starts through a gated launcher. Linux puts it in its own process
group. Windows assigns it to a kill-on-close Job Object. Before releasing the
launcher, persist the candidate identity in the transaction. The group or Job
Object is terminated and the launcher reaped after a normal exit, timeout,
Ctrl+C, or infrastructure failure. Log readers have a bounded finish window
after termination. A stale owner may leave a live Linux process group. Abort
checks the persisted identity and refuses while the group is active. Candidate
lock and sync use the same launch gate. Production sync, recovery sync, and
smoke tests record the scoped process in the operation ledger before execution.

The native quick-test probe recognizes ComfyUI's guarded zero exit from
`--quick-test-for-ci`. It distinguishes that exit from an early zero exit. Node
load evidence comes from the structured import summary and the exact canonical
node path. Unsupported probe variants fail with diagnostics. The transaction
adds candidate sync, exact package drift, snapshot integrity, and core policy
checks on top of the runtime outcome. See [ComfyUI's startup entry point](https://github.com/Comfy-Org/ComfyUI/blob/master/main.py)
and [custom-node import reporting](https://github.com/Comfy-Org/ComfyUI/blob/master/nodes.py).

Both stdout and stderr are relayed live and saved to transaction log files.
First attempts use `<txid>.stdout.log` and `<txid>.stderr.log`. Retries use an
attempt suffix. Production output goes directly to the terminal.
