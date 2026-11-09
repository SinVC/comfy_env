# Operations Spec

## Metadata

- Status: Active
- Last Reviewed: 2026-03-01

## Operational Model

- `init` establishes layout and a synchronized prod environment.
- `status` is the lightweight summary command for local health.
- `tx run` is the primary observation path for candidate experimentation.
- `tx promote`, `node remove`, and `undo` are the primary guarded mutation paths.
- `run` and `stop` are runtime control commands for the managed ComfyUI instance.

## Runtime Artifacts

- `state/logs/*.stdout.log`
- `state/logs/*.stderr.log`
- `state/conflicts/*.json`
- `state/comfyui.pid`

## Failure Handling

- Lock conflicts become conflict reports, not silent retries.
- Failed prod sync or smoke attempts restore from operation backups.
- Stop first attempts `SIGTERM`, then `SIGKILL` after 30 seconds.

## Retention And Cleanup

- Operation retention defaults to 100 directories unless config overrides it.
- `ops_prune` removes older operation directories beyond retention.
- Candidate envs remain until manually aborted or otherwise cleaned by the operator.

## Operator Checks

- Use `status` for high-level state.
- Use `tx inspect` for transaction-level diagnostics.
- Use `op list` and `op inspect` for rollback evidence.
