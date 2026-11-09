# Plugin conflict resolution

[Implementation](../../src/application/resolve.rs)

```
gov resolve <txid> [--pin <pkg==version>]... [--pins-file <path>] [--override-requirements]
```

Requires a registered plugin transaction in `needs_resolution`. Provide one or
more exact pins through repeatable flags or a file with one pin per line.
Blank lines and comments in the file are ignored. Without pins, gov prints the
conflict summary and prompts for one pin per line until a blank line or EOF.

Merge pins by normalized package name with the last value winning. Stage from
current production truth, then apply the node's recorded requirements and the
resolution pins. A normal pin adds a constraint in
`dependency-groups.overrides`. Use `--override-requirements` to also write the
pins from this invocation to `tool.uv.override-dependencies`. The Torch family
is managed by `gov install torch`.

Apply and validate the staged edit before replacing the old record or stage.
A rejected pin leaves the existing conflict evidence available for retry.
Before locking the new stage, invalidate the earlier observation and record
this invocation as owner. If the lock conflicts, keep the updated report in
`needs_resolution` and offer the explicit force flag when appropriate. When
the lock succeeds, sync and observe the candidate again. A passed observation
becomes `resolved`. Failed observations retain their own outcome and logs.
Forced overrides still require the same snapshot checks and core approval at
[promotion](tx.md). Production truth is unchanged until promotion.
