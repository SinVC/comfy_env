# Global pins

[Implementation](../../src/application/pin.rs)

## `gov pin add <pkg==version>... [--override-requirements]`

Validate exact pins and normalize package names. The last value wins for each
name. Torch-family packages use `gov install torch`. Other packages are accepted
with a warning outside the usual NumPy and Transformers pins.

A normal pin adds an exact constraint to `dependency-groups.overrides`. It can
fail the lock if a requirement is incompatible. With `--override-requirements`,
write the exact pin in both the group and `tool.uv.override-dependencies`, so
uv overrides conflicting requirements while the group still installs the
package. Updating an unmatched forced pin requires the explicit mode.

## `gov pin list [--details]`

Print one entry per line. The listing includes independent uv overrides.
Matching exact pins appear once. `--details` labels constraints, paired forced
pins, forced-only entries, inconsistent pairs, and manual overrides.

## `gov pin remove <pkg>... [--override-requirements]`

Remove the selected exact pins by normalized package name. A matching group pin
and unscoped forced override are removed together. An orphaned forced override
or one with a different version requires `--override-requirements`. Scoped,
marker-qualified, and other non-exact hand-written requirements remain in place.
A missing requested pin rejects the whole change. Node removal leaves global
pins alone.

For add and remove, apply the edit in a temporary project, lock, then back up
and commit the staged truth with a production sync and smoke test. A failed
lock leaves production untouched. An interrupted write uses
[operation recovery](ops.md), and a successful change can be undone.
