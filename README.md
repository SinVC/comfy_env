# Comfy Environment Governance (`comfy_env`)

ComfyUI 依赖治理 CLI 已迁移到 Rust。在侧车项目根目录运行 `gov help` 查看命令，使用 `gov init` 创建本地配置和生产环境。

编译使用 `cargo build --release`。Linux 的二进制文件为 `target/release/gov`，Windows 为 `target/release/gov.exe`。开发与审查从 [Rust 架构说明](dev-docs/architecture.md) 进入。[命令契约](dev-docs/commands/contracts.md) 给出每个命令的设计。

## 完整工作流

以下章节描述命令和目录结构。`bin/gov` 是保留的 Bash 行为参考。

## 1. 项目目标

1. 可复现：本地 `pyproject.toml + uv.lock` 可重建依赖环境。
2. 可审计：插件事务、核心依赖升级、操作备份都有状态与日志。
3. 可恢复：`install/promote/remove/update promote` 失败自动回退；成功操作可 `undo`。
4. 可治理：核心包变更需要显式批准。

## 2. 命令总览

1. `gov init --comfyui-dir <abs-path> --python <python-spec>`
2. `gov install torch --index-url <url>`
3. `gov install [--requirements-file <path>]`
4. `gov update run [--requirements-file <path>] [--timeout <seconds>]`
5. `gov update inspect <txid>`
6. `gov update resolve <txid> [--pin <pkg==version>]... [--pins-file <path>]`
7. `gov update promote <txid> [--approve-core --reason "..."] [--allow-failed-run]`
8. `gov update abort <txid>`
9. `gov env export <output_dir>`
10. `gov env import <bundle_dir> --comfyui-dir <abs-path> --python <python-spec>`
11. `gov status`
12. `gov node add <git_url> [--ref <sha/tag>] [--id <node_id>]`
13. `gov node remove <node_id> [--purge-code]`
14. `gov tx run <node_id> [--timeout <seconds>]`
15. `gov tx inspect <txid>`
16. `gov tx abort <txid>`
17. `gov tx promote <txid> [--approve-core --reason "..."] [--allow-failed-run]`
18. `gov resolve <txid>`
19. `gov op list`
20. `gov op inspect <op_id>`
21. `gov undo <op_id>`
22. `gov run [--sync] [-- <args...>]`
23. `gov stop`

## 3. 目录结构（侧车模式）

```text
TopDir/
├── ComfyUI/
│   ├── main.py
│   ├── requirements.txt
│   └── custom_nodes/
└── comfy_env/
    ├── gov                     # Windows 为 gov.exe
    ├── config.toml.template
    ├── pyproject.toml.template
    ├── config.toml
    ├── pyproject.toml
    ├── uv.lock
    ├── .venv-prod/
    ├── .venv-candidate/
    ├── state/
    │   ├── plugins.json
    │   ├── ops/
    │   ├── transactions/
    │   ├── conflicts/
    │   ├── logs/
    │   └── work/
    ├── dev-docs/
    └── docs/
```

## 4. 首次安装（新顺序）

```bash
gov init --comfyui-dir /abs/path/to/ComfyUI --python 3.12
gov install torch --index-url https://download.pytorch.org/whl/cu130
gov install
```

说明：

1. `init` 负责写入 `config.toml` 的 `paths.comfyui_dir` 与 `runtime.python`，并初始化最小 prod 环境；若 `--python` 传入纯 `major.minor`（如 `3.12`）则直接采用该 minor 线，若传入 patch 版本或解释器选择器（如 `3.12.8`、`python3.12`）则先用 `uv python find --no-python-downloads` 解析到本机可执行解释器，再回写成 minor 线。
2. `install torch` 把 `torch/torchvision/torchaudio` 收敛到 `dependency-groups.torch`，并记录可撤销的 operation。
3. `install` 从 `${comfyui_dir}/requirements.txt` 导入基础依赖到 `dependency-groups.core`。
4. `install` 在 torch 未先安装时会阻断。

## 5. 后续升级（核心依赖事务）

```bash
gov update run
gov update inspect <txid>
gov update resolve <txid> --pin pkg==1.2.3
gov update promote <txid>
```

说明：

1. `update run` 读取 `requirements.txt`，在 staged workdir 构造候选真相，再同步到 candidate 环境观察。
2. `update promote` 只晋升该事务已经固定下来的 staged snapshot，不会重新读取当前 `requirements.txt`。
3. promote 失败会回滚，成功的 `update_promote` operation 可用 `undo` 撤销。

## 6. 插件事务（保持不变）

```bash
gov node add <git_url> [--id <node_id>]
gov tx run <node_id>
gov tx inspect <txid>
gov tx promote <txid>
```

冲突时：

```bash
gov resolve <txid>
gov tx promote <txid>
```

## 7. 撤销与状态

```bash
gov status
gov op list
gov undo <op_id>
```

`status` 现在会额外显示：

1. `config_ready`
2. `comfyui_dir`
3. `python`
4. `torch_ready`
5. `core_ready`
6. `update_transactions_pending`

## 8. 环境交付

```bash
gov env export /abs/path/to/bundle-dir
gov env import /abs/path/to/bundle-dir --comfyui-dir /abs/path/to/ComfyUI --python 3.12
```

说明：

1. `env export` 导出目录型 bundle，包含 `pyproject.toml`、`uv.lock`、`pylock.toml`、`state/plugins.json`、过滤 Git 元数据后的 `custom_nodes` 运行时源码快照和审计文件；快照保留工作树中的已修改文件和未跟踪文件。
2. `env import` 只接受目录 bundle，不支持 tarball。
3. `env import --python` 和 `init --python` 采用同一套规则：纯 `major.minor` 直接使用，其他 selector 先解析到目标机已安装解释器，再规范化为 minor 线。
4. `comfyui_dir` 是目标机本地配置，不属于 bundle 真相；导入时必须显式传入。
5. 导入后的本地依赖真相仍然是 `pyproject.toml + uv.lock`；`pylock.toml` 是标准化交付物，不替代本地真相。
6. `env import` 默认执行精确恢复：覆盖目标 root truth、prod env、插件注册，并清理 bundle 外的 `custom_nodes/*` 目录，使结果与 bundle 一致。
7. v1 不支持 partial import、nodes-only import 或按节点依赖增量导入。

## 9. 文档导航

1. [Rust 架构说明](dev-docs/architecture.md)
2. `docs/04_cli_reference.md`
3. `docs/10_quick_start_for_llm.md`
