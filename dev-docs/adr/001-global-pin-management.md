# ADR 001 — Global pin management

**Status** Accepted

Exact global pins give operators a way to keep dependencies such as NumPy or
Transformers on an approved version across subsequent locks and production
syncs. Torch-family packages use the dedicated installation flow.

Ordinary pins add constraints to `dependency-groups.overrides`. Incompatible
requirements stay in conflict. Explicit `--override-requirements` writes the
exact pin to the dependency group and to `tool.uv.override-dependencies`, so uv
can replace incompatible constraints while the group continues to install the
package. Plugin and core resolution share this contract. Previous core resolve
calls that relied on implicit force must add the explicit flag.

The two entries are staged and locked as one change. Production writes use
the operation backup and can be undone. A matching unscoped pair can be removed
together. Historical orphaned or mismatched exact entries require the explicit
mode for removal. Marker-qualified, scoped, and other hand-written overrides
remain available to uv. Node removal leaves global pins intact.

See [pin](../commands/pin.md), [resolve](../commands/resolve.md), and
[update](../commands/update.md).
