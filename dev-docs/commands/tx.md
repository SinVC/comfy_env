# Plugin transactions

[Implementation](../../src/application/tx.rs)

## `gov tx run <node_id> [--timeout <seconds>]`

Requires a registered node with source in `custom_nodes/<node_id>`, a configured
project, and no running transaction for that node. A missing `requirements.txt`
contributes an empty dependency group. Copy production truth to a temporary
stage and retain the stage only after the running transaction and its owner
identity have been saved. Write the declared requirements to the node group,
then lock. A resolver conflict records `needs_resolution` and a conflict report
for [resolve](resolve.md).

Sync a candidate venv from the locked stage. Record both truth hashes and
production's actual package set. Compare the candidate package set immediately
before and after the startup probe. Keep stdout and stderr visible live and
save both streams in `state/logs`. Each retry has its own observation and logs.
The candidate must reach ComfyUI's native `--quick-test-for-ci` exit and report
successful loading of the exact target node path.

An observed pass is `completed`. Sync failure, infrastructure errors,
unsupported or incomplete probes, target load failure, a nonzero exit, timeout,
interruption, and candidate package drift are distinct failed outcomes. Timeout
is the execution limit. A candidate that installs additional packages must be
observed again with those dependencies explicitly declared. Transitive
dependencies remain in `uv.lock`; freeze differences are audit data.

## `gov tx inspect <txid>`

Show timestamps, status, package and core diffs, exit evidence, node load and
package drift results, logs and artifact locations. Inspect can show the
associated operation and whether it has since been undone. Historical records
remain readable. Missing artifacts are annotated as cleaned after promotion or
abort and as missing in other states.

## `gov tx abort <txid> [--confirm-stopped]`

Clean the candidate and stage after validating their recorded path ownership.
Retain the transaction ID and logs. An already aborted transaction can be
retried. Promoted transactions and transactions whose promotion requires
operation recovery are refused.

A stale running transaction can be aborted when its owner and candidate
processes have ended. For old running records without identity evidence,
verify both have stopped and then pass `--confirm-stopped`. This flag cannot
override evidence of a live owner or candidate.

## `gov tx promote <txid> [--approve-core] [--reason <text>] [--allow-failed-run] [--keep-artifacts]`

Verify the current production truth, the staged truth, the current production
package set, and the node's group against the recorded observation. Recompute
the core impact using the current `policy.core_packages`. Approval is required
for a nonempty intersection. `--reason` is saved with the promotion.

`--allow-failed-run` applies only to an actual nonzero runtime exit after a
successful candidate sync and valid observation. Missing evidence or drift
requires a fresh run or resolution. Use the observed lock and its equivalent
local wheel paths directly. Promotion runs one locked production sync, the
ordinary smoke command, and the node registry update. The production operation
commits before the transaction is marked promoted. If its ledger link needs
repair, use [op recover](ops.md) or retry promotion. Successful promotion cleans
the candidate and stage unless `--keep-artifacts` is set.
