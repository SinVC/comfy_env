# Environment bundles

[Implementation](../../src/application/env.rs)

## `gov env export <output_dir>`

Requires configuration, both truth files, the registry, and the source directory
of each registered node. The output directory must be new and outside the
node source trees. The bundle includes `pyproject.toml`, `uv.lock`, the
registry, `pylock.toml` from `uv export`, and the production freeze and export
summary under `audit/`. Node snapshots exclude `.git` and reject links.
The v1 manifest includes SHA-256 hashes for all payload files, requirements,
platform, architecture and node IDs. An export error cleans its partial output.

## `gov env import <bundle_dir> --comfyui-dir <absolute-path> --python <selector>`

Bundle v1 restores the complete locked project on the same platform and
architecture. It uses the bundle's lock and registry with the chosen target
interpreter. Validate the manifest, file hashes, node IDs and normalized group
ownership, and path separation before writing the target. Every bundled
`node-*` dependency group must have a registered owner.

Resolve the actual target interpreter with uv and run `uv lock --check`. A uv
sync dry run evaluates the bundled Python version specifiers and environment
markers using that interpreter. On older uv versions that reject combining
`--dry-run` and `--locked`, the dry run follows the independent lock check.
Import saves the resolved local interpreter. The manifest's platform and
architecture must match even if the lock could otherwise be used.

List the ordinary node directories to replace or remove. Preserve top-level
regular files, symlinks, and junctions. A bundle node that collides with a
regular file blocks the import. A colliding link is preserved with a warning
that the bundle's source for that node was skipped. Reject overlaps that would
let import delete or copy its own bundle, project, venv or recovery data.

Stage verified node copies in a sibling recovery area on the target volume
outside the scanned `custom_nodes` tree. Back up configuration, truth and
registry and record the operation. Write the bundle truth and sync production
before moving any node entries. Journal each move, move old ordinary
directories into recovery storage, then publish the staged directories. Run
the smoke test and commit. Cleanup failures warn with the retained path.
Import skips the core approval gate and ordinary undo.

On error, restore the backup and the original venv. Reverse only source moves
that were attempted. A sync failure before source replacement leaves the node
tree intact. An interrupted or failed rollback uses [op recover](ops.md).

Local wheel registry paths are adjusted when copying locks to preserve their
meaning. Relative `UV_FIND_LINKS` paths resolve from the invocation directory.
Sources that cannot be moved reliably are rejected with their location.
