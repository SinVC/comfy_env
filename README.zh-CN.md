# ComfyUI 环境治理系统 (`gov`)

<p align="center">
  <b>确定性、事务化、面向 AI Agent 原生设计的 ComfyUI 侧车依赖治理引擎。</b>
</p>

<p align="center">
  <a href="README.md"><b>English</b></a> •
  <a href="README.zh-CN.md"><b>简体中文</b></a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/平台-Linux%20%7C%20Windows-blue.svg" alt="Platform: Linux | Windows" />
  <img src="https://img.shields.io/badge/语言-Rust%20单二进制-orange.svg" alt="Rust" />
  <img src="https://img.shields.io/badge/引擎-uv-purple.svg" alt="uv" />
  <img src="https://img.shields.io/badge/安全-事务化回滚-green.svg" alt="Safety" />
</p>

---

## 项目背景与优势

### 核心痛点：ComfyUI 插件的“依赖地狱”
几乎每一位深度使用 ComfyUI 的用户或开发者都经历过这种绝望：安装自定义节点（custom nodes）时，各节点自带的脚本常会无序执行 `pip install`，随意覆盖 PyTorch/CUDA 版本、降级底层依赖库，导致现有节点大面积报错甚至彻底瘫痪。一旦 Python 虚拟环境损坏，排查极为困难；卸载节点后又会遗留大量冗余垃圾包；最终往往只能推倒重来，耗费数小时重建虚拟环境。

### 解决方案：侧车化事务隔离与治理
`comfy_env` 提供了名为 **`gov`** 的 Rust 单二进制侧车治理引擎。它基于现代高速 Python 包管理器 [`uv`](https://github.com/astral-sh/uv)，构建了严密的保护机制：
- **沙盒隔离预演**：所有插件安装与依赖变更均先在候选沙盒环境（`.venv-candidate`）中演算并验证兼容性，决不直接污染生产环境（`.venv-prod`）。
- **密码学级操作回滚**：每次环境变更前自动记录不可篡改的快照；一旦发现异常，可通过 `gov undo` 100% 确定性回滚至操作前状态。
- **核心包漂移防护**：严格管控 PyTorch、CUDA 等核心依赖，防止插件静默篡改核心版本（需要显式 `--approve-core` 批准）。

### 为什么是 CLI？专为 AI Agent 与自动化而生
现代生成式 AI 工作流正迅速走向自主化——由各类 AI Agent（如 Claude Code、AutoGPT、本地自动化运维 Agent 等）自主寻找并安装节点、生成工作流、维护运行环境。然而，GUI 界面或交互式命令行提示极易让 Agent 陷入停滞或报错死循环。

`gov` 采用 **Agent 优先（Agent-First）** 的设计哲学：
- **高确定性与纯粹的脚本化**：全命令驱动、明确的退出状态码、无需交互式打断，让 Agent 能够极为可靠、安全地编排环境。
- **机器可读的结构化状态**：所有事务演进、锁冲突报告及操作记录均输出清晰的 JSON 与工件（`state/transactions/`、`state/conflicts/`），天然适配 LLM 上下文解析。
- **自主闭环自愈**：当遇到依赖冲突时，Agent 可以自动化读取冲突报告、调用 `gov resolve` 执行 Pin 约束解算，或通过 `gov undo` 一键安全回滚，实现完全自主的环境自愈。

---

## 架构一览（侧车模式）

`comfy_env` 与您的 ComfyUI 主目录保持平行，采用完全解耦的**侧车（Sidecar）结构**，对 ComfyUI 源码零侵入：

```text
工作区/
├── ComfyUI/              # 原始 ComfyUI 代码仓库（保持纯净）
│   ├── main.py
│   ├── requirements.txt
│   └── custom_nodes/     # 统一托管的自定义节点
│
└── comfy_env/            # 侧车治理环境目录
    ├── gov / gov.exe     # 单一 Rust 二进制文件（系统唯一入口）
    ├── config.toml       # 本地路径与 Python 解释器配置
    ├── pyproject.toml    # 声明式依赖组管理 (uv)
    ├── uv.lock           # 锁定的确定性依赖树
    ├── .venv-prod/       # 生产环境专用 Python 虚拟环境
    ├── .venv-candidate/  # 候选事务专用的隔离观察沙盒
    └── state/            # 事务记录、操作备份与冲突审计日志
```

---

## 安装工作区

先安装 Git、uv 和 Python 3.12。安装器默认使用 CPU 版 PyTorch。使用 GPU 时，在 Linux 传入 `--torch-index-url`，在 Windows 设置 `GOV_TORCH_INDEX_URL`，指定对应的 PyTorch 索引。

**Linux x86-64**

```sh
curl -fsSL https://github.com/SinVC/comfy_env/releases/latest/download/install.sh |
  sh -s -- --workspace "$PWD/ComfyUIWorkspace"
```

**Windows x64**

```powershell
$env:GOV_WORKSPACE = 'D:\AI\ComfyUIWorkspace'
irm https://github.com/SinVC/comfy_env/releases/latest/download/install.ps1 | iex
```

复用已有 ComfyUI 时，先自行将 Git checkout 放到工作区的 `ComfyUI/`。Linux 加 `--mode migrate`，Windows 设置 `$env:GOV_SETUP_MODE = 'migrate'`。安装器根据该源码的依赖声明重新建立治理环境。模型和路径配置由用户在 ComfyUI 中管理。

安装后在工作区运行 `./bin/gov run` 或 `.\bin\gov.exe run`。可选的 Agent Skill 位于 [`skills/gov-cli/`](skills/gov-cli/SKILL.md)，可从 CLI 对应的 Git tag 安装。

## 极速上手

### 1. 初始化与生产引导
绑定 ComfyUI 目录与 Python 解释器，配置 PyTorch 及 ComfyUI 基础依赖：

```bash
# 1. 初始化配置与生产虚拟环境
gov init --comfyui-dir /abs/path/to/ComfyUI --python 3.12

# 2. 安装并锁定对应 CUDA 版本的 PyTorch
gov install torch --index-url https://download.pytorch.org/whl/cu124

# 3. 锁定并安装 ComfyUI 基础依赖到 core 组
gov install
```

### 2. 核心工作流：安全的插件安装事务
告别盲目的 `pip install`，通过事务生命周期安全演进环境：

```bash
# 1. 注册并拉取自定义节点源码
gov node add https://github.com/example/custom-node.git

# 2. 在沙盒 (.venv-candidate) 中运行事务并观察依赖变动
gov tx run custom-node

# 3. 检查依赖变动与锁差异
gov tx inspect <txid>

# 4. 安全晋升至生产环境 (.venv-prod)
gov tx promote <txid>
```

*若发生依赖冲突，可审计生成的冲突报告并通过 `gov resolve <txid>` 求解后再晋升。*

### 3. 状态巡检与一键撤销
随时查看环境状态，或安全回滚任何破坏性操作：

```bash
# 查看完整系统健康状况与待办事务
gov status

# 查看历史变更操作列表
gov op list

# 依据操作备份一键安全回滚
gov undo <op_id>
```

---

## 完整文档导航

本项目拥有完备的架构规范与实现文档，无需在 README 中冗余复述，请直接查阅：

| 文档分类 | 文档链接 | 说明 |
| :--- | :--- | :--- |
| **系统架构** | [架构总览](dev-docs/architecture.zh-CN.md) ([英文版](dev-docs/architecture.md)) | 系统不变式、模块边界与跨平台设计规则 |
| **项目目标** | [项目目标与原则](docs/01_project_objectives.md) | 核心痛点分析、设计原则与成功验收标准 |
| **AI Agent 指南** | [LLM / Agent 快速上手指南](docs/10_quick_start_for_llm.md) | 心智模型、命令速查表与 Agent 自动化落地策略 |
| **命令契约** | [命令契约总览](dev-docs/commands/contracts.md) | 所有 CLI 命令的输入输出契约与执行步骤 |
| **CLI 手册** | [CLI 参考手册](docs/cli-reference.zh-CN.md) ([英文版](docs/cli-reference.md)) | 完整的命令参数、选项与行为规范 |
| **节点生命周期** | [节点命令](dev-docs/commands/node.md) 与 [事务命令](dev-docs/commands/tx.md) | 节点增删与候选环境事务执行管理 |
| **核心依赖升级** | [Update 升级命令](dev-docs/commands/update.md) | ComfyUI 核心依赖升级的沙盒演进与晋升机制 |
| **依赖锁定 Pin** | [Pin 全局治理](docs/12_gov_pin_handoff.md) | 全局包版本精确锁定与冲突覆盖管理 (`gov pin`) |
| **运行与恢复** | [运行时与运维指南](docs/03_runtime_and_operations.md) | 生产锁机制、ComfyUI 进程托管 (`gov run`) 与故障恢复 |
| **环境跨机交付** | [环境导出与导入交付](docs/11_env_export_import_handoff.md) | 离线环境 Bundle 导出与导入规范 (`gov env`) |

---

## 源码编译

`gov` 采用纯 Rust 编写，无第三方 C 运行库外部依赖：

```bash
# 依赖环境：当前稳定版 Rust 与 Cargo
cargo build --release

# 编译产物位置：
# Linux:   target/release/gov
# Windows: target/release/gov.exe
```

---

## 开源协议

本项目基于 Apache 2.0 / MIT 双重协议开源。
