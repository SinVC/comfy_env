# 10 给接手 LLM 的快速上手

## 1. 先建立心智模型

1. 依赖真相是本地文件：`pyproject.toml` 和 `uv.lock`。
2. `config.toml` 现在还承载运行前置条件：`paths.comfyui_dir` 与 `runtime.python`。
3. `dependency-groups.torch`、`dependency-groups.core`、`dependency-groups.overrides` 分别治理 torch、ComfyUI 基础依赖、以及全局精确 pin。
4. 插件事务和核心依赖升级事务共用 `state/transactions/`，但通过 `kind` 区分。
5. 破坏性变更仍然依赖 `operation backup + undo`。
6. 环境交付 bundle 只是运输载体；导入后的本地依赖真相仍然是 `pyproject.toml + uv.lock`。

## 2. 首次落地（新的最小顺序）

```bash
./bin/gov init --comfyui-dir /abs/path/to/ComfyUI --python 3.12
./bin/gov install torch --index-url https://download.pytorch.org/whl/cu130
./bin/gov install
```

说明：

1. `init` 首次执行时必须显式传 `--comfyui-dir` 和 `--python`。
2. 推荐把 `--python` 直接写成 minor 线，如 `3.12`；若传入 `3.12.8` 或解释器选择器，CLI 会先用 `uv python find --no-python-downloads` 解析到本机解释器，再回写为 `3.12`。
3. `install torch` 必须先于 `install`。
4. `install` 默认读取 `${comfyui_dir}/requirements.txt`。

## 3. 核心目录与用途

1. `bin/gov`：主 CLI。
2. `state/transactions/`：插件事务和 `kind=core_update` 核心升级事务。
3. `state/ops/`：`install/install torch/promote/remove/update promote/undo` 的操作元数据与备份。
4. `state/conflicts/`：lock 冲突报告。
5. `state/work/`：staged workdir。

## 4. 命令速查

### 4.1 环境与首装

1. `./bin/gov init --comfyui-dir <abs-path> --python <python-spec>`
2. `./bin/gov install torch --index-url <url> [--torch <torch==version>] [--torchvision <torchvision==version>] [--torchaudio <torchaudio==version>]`
3. `./bin/gov install [--requirements-file <path>]`

### 4.2 核心依赖升级事务

1. `./bin/gov update run [--requirements-file <path>] [--timeout <seconds>]`
2. `./bin/gov update inspect <txid>`
3. `./bin/gov update resolve <txid> [--pin <pkg==version>]... [--pins-file <path>]`
4. `./bin/gov update promote <txid> [--approve-core --reason "..."] [--allow-failed-run] [--keep-artifacts]`
5. `./bin/gov update abort <txid>`

### 4.3 全局 Pin 管理

1. `./bin/gov pin add <pkg==version>...`
2. `./bin/gov pin list`
3. `./bin/gov pin remove <pkg>...`

说明：

1. `pin add` 只接受精确版本，不支持范围约束。
2. `pin` 不接受 `torch`、`torchvision`、`torchaudio`；torch-family 的版本与 source/index 由 `install torch` 单独治理。
3. `pin` 写入的是共享的 `dependency-groups.overrides`，会影响后续 lock、prod sync，以及事务 resolve 的共同求解结果。
4. `pin add` 在 staged workdir 中先按包名移除当前目标 override，再用 `uv add --group overrides --frozen` 写回 exact specs；`pin remove` 直接委托给 `uv remove --group overrides --frozen`。
5. 成功标准是 `lock + prod sync + smoke test`；失败会回滚 root truth，并把 prod 环境同步回恢复后的状态。

### 4.4 插件治理（保持原有路径）

1. `./bin/gov node add <git_url> [--id <node_id>]`
2. `./bin/gov tx run <node_id>`
3. `./bin/gov tx inspect <txid>`
4. `./bin/gov tx promote <txid> [--approve-core --reason "..."] [--allow-failed-run] [--keep-artifacts]`
5. `./bin/gov resolve <txid>`

### 4.5 环境交付

1. `./bin/gov env export <output_tar>`
2. `./bin/gov env import <bundle_tar> --comfyui-dir <abs-path> --python <python-spec>`

### 4.6 审计与运行

1. `./bin/gov status`
2. `./bin/gov op list`
3. `./bin/gov op inspect <op_id>`
4. `./bin/gov undo <op_id>`
5. `./bin/gov run [--sync] [-- <args...>]`
6. `./bin/gov stop`

## 5. 推荐工作流

### 5.1 首次安装

```bash
./bin/gov init --comfyui-dir /abs/path/to/ComfyUI --python 3.12
./bin/gov install torch --index-url https://download.pytorch.org/whl/cu130
./bin/gov install
```

### 5.2 核心依赖升级

```bash
./bin/gov update run
./bin/gov update inspect <txid>
./bin/gov update resolve <txid> --pin pkg==1.2.3
./bin/gov update promote <txid>
```

说明：

1. `update run` 会过滤 `torch`、`torchvision`、`torchaudio`，并继续事务。
2. candidate 运行到达 timeout 时会收束进程，但事务仍记为 `completed`，并保留真实 `run_exit_code`。
3. 成功 `update promote` 后默认清理 candidate env 与 staged workdir；如需保留可传 `--keep-artifacts`。

### 5.3 全局兼容 Pin

```bash
./bin/gov pin list
./bin/gov pin add numpy==1.26.4
./bin/gov pin add transformers==4.44.0
./bin/gov pin remove numpy
```

### 5.4 Torch 版本治理

```bash
./bin/gov install torch --index-url https://download.pytorch.org/whl/cu130
./bin/gov install torch --index-url https://download.pytorch.org/whl/cu130 --torch torch==2.11.1
./bin/gov install torch --index-url https://download.pytorch.org/whl/cu130 --torchvision torchvision==0.26.1
```

### 5.5 插件接入

```bash
./bin/gov node add <git_url> [--id <node_id>]
./bin/gov tx run <node_id>
./bin/gov tx promote <txid>
```

说明：

1. `tx run` 的 candidate 输出会同时进入终端和 `state/logs/`。
2. candidate 运行到达 timeout 时会收束进程，但事务仍记为 `completed`，并保留真实 `run_exit_code`。
3. 成功 `tx promote` 后默认清理 candidate env 与 staged workdir；如需保留可传 `--keep-artifacts`。

### 5.6 环境交付

```bash
./bin/gov env export /abs/path/to/bundle.tar
./bin/gov env import /abs/path/to/bundle.tar --comfyui-dir /abs/path/to/ComfyUI --python 3.12
```

说明：

1. `env export` 导出的是 `.tar` bundle，内部包含 `custom_nodes` 运行态源码快照，不依赖导入端重新拉 Git。
2. 快照会保留工作树中的已修改文件和未跟踪文件，但不会把 `.git` 元数据带进 bundle。
3. `env import` 默认是 exact restore，会清理 bundle 外的 `custom_nodes/*` 目录。

## 6. 接手时最小核查清单

1. `./bin/gov help` 包含 `install` 和 `update` 命令族。
2. `./bin/gov help` 包含 `pin add/list/remove`。
3. `./bin/gov status` 能输出 `config_ready/python/torch_ready/core_ready`。
4. `pyproject.toml.template` 包含 `core`、`torch`、`overrides` 三个固定组。
5. 若要做环境迁移，优先核对 `docs/04_cli_reference.md` 与 `docs/11_env_export_import_handoff.md` 中的 bundle/exact restore 语义。
