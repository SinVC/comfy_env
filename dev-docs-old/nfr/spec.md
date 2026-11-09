# NFR Spec

## Metadata

- Status: Active
- Last Reviewed: 2026-03-01

## Reliability

- Destructive flows prefer rollback to partial success.
- Transaction and operation records provide replayable evidence after failures.
- Missing critical state should fail closed instead of silently continuing.

## Performance

- The system optimizes for correctness and auditability over speed.
- Locking and syncing are expected to dominate runtime cost.
- Candidate runs can be bounded by timeout to avoid indefinite hangs.

## Security

- Scope is local workstation governance, not multi-tenant isolation.
- Trust is limited to locally installed tools, local filesystem state, and operator intent.
- The system does not validate plugin code safety beyond controlled installation flow.

## Observability

- Transaction stdout/stderr logs are durable files.
- Operation metadata stores pre/post hashes and status.
- Conflict reports persist summarized lock failure context.
- `status`, `tx inspect`, and `op inspect` expose operator-facing visibility.

## Operability Constraints

- Concurrent writers are not coordinated by file locks.
- Recovery assumes backup directories and local disk remain available.
- `gov run` is foreground-oriented and not a service manager.
