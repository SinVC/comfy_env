# Data Sovereignty And Integration Spec

## Metadata

- Status: Active
- Last Reviewed: 2026-04-07

## Source Of Truth By Domain

- Dependency truth:
  - `pyproject.toml`
  - `uv.lock`
- Plugin registry truth:
  - `state/plugins.json`
- Transaction truth:
  - `state/transactions/<txid>.json`
- Operation truth:
  - `state/ops/<op_id>/meta.json`
  - `state/ops/<op_id>/backup/*`
- Conflict truth:
  - `state/conflicts/<txid>.json`
- Runtime liveness hint:
  - `state/comfyui.pid`
- Transfer artifact:
  - `env export` bundle tar (`bundle/manifest.json`, `bundle/pyproject.toml`, `bundle/uv.lock`, `bundle/pylock.toml`, `bundle/state/plugins.json`, `bundle/custom_nodes/*`, `bundle/audit/*`)

## Ownership Rules

- Dependency truth is authoritative for what prod should contain.
- Bundle artifacts are transport payloads only; after import, dependency truth remains `pyproject.toml` plus `uv.lock`.
- Plugin registry is authoritative for known plugin metadata, but not for actual dependency removal content.
- `env import` is an exact restore path for bundle-managed `custom_nodes/*`; bundle 外节点目录不应在导入后继续残留。
- Bundle `custom_nodes/*` snapshots reflect runtime working trees, preserving modified/untracked files while excluding VCS admin metadata.
- `paths.comfyui_dir` is target-local configuration and must come from `env import --comfyui-dir`, not from bundle payload.
- `managed_deps` is a cache field and can be rebuilt from dependency groups.
- Transaction and operation records are the audit trail; they should not be edited manually.

## Integration Boundaries

- `uv` reads and rewrites dependency truth and materializes envs from it.
- `env export` copies truth files and runtime source snapshots into a transport bundle but does not redefine data ownership.
- `env import` must validate manifest integrity and runtime compatibility before overwriting root truth.
- `git` touches plugin source trees only.
- ComfyUI runtime reads source trees and active environments but does not own governance records.

## Consistency Model

- There is no distributed consensus; consistency is local and sequential by convention.
- Workdirs are temporary staging copies used to prevent partially written root truth during lock generation.
- Operation backups are the recovery anchor when root truth mutation fails.

## Data Risks

- Concurrent local commands can race because the system has no lockfile protocol.
- Manual edits to local truth can invalidate undo assumptions and intentionally trigger the hash guard.
