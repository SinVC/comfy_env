# 架构总览

## 系统身份

`comfy_env`（`gov`）是一个侧车式治理 CLI，用于控制如何观察、提升、回滚以及运行 ComfyUI 自定义节点的依赖变更。

- **语言**: Rust，单一二进制，单一 crate
- **平台**: Linux + Windows
- **外部依赖**: `uv`、`git`、Python（全部通过子进程调用，绝不嵌入）

## AI 阅读协议

这套 `dev-docs` 主要是给 AI 辅助开发和审查使用的，因此文档被刻意拆分，用来控制读取范围并隔离无关上下文。

推荐按以下顺序读取：

1. 先读 `architecture.md`，只拿系统不变量、模块边界和跨平台规则
2. 再读当前任务对应的单个 `commands/*.md`
3. 仅继续读取该命令文档引用到的 `modules/*.md`
4. 最后顺着文档顶部的实现链接进入对应 Rust 文件

规则：

- 不要为了“补背景”预读无关命令或无关模块文档，除非当前任务确实跨越这些边界
- 文档已经给出实现入口时，优先沿着链接进入代码，而不是先做全仓库扫描
- 迁移期间，Rust 文件可能仍是脚手架；行为权威仍以文档契约为准
- 保持上下文隔离：命令文档回答“要做什么”，模块文档回答“这个构件如何工作”，无关切片不要混读

## 不变量

1. 所有生产依赖变更都必须先锚定到本地真相文件 `pyproject.toml` 和 `uv.lock`，之后才可应用到 `.venv-prod`。
2. 事务是插件依赖影响在提升前唯一受支持的观察单元。
3. 生产变更在首次写入前备份并记录恢复依据。同一项目的生产写入、恢复和运行时启动共用一把锁。仍活跃的纳管进程组或 Job 和待恢复操作都会阻止生产写入。
4. 锁文件冲突必须以显式冲突产物和可解析事务状态的形式暴露，而不是静默部分成功。
5. 核心包漂移受策略门控约束，未经显式批准不得提升。
6. `state/plugins.json` 是注册表元数据；`pyproject.toml` 中 dependency-group 的内容才是实际依赖的权威来源。
7. `env export` / `env import` 产生的是传输工件；导入完成后，本地真相仍然是 `pyproject.toml + uv.lock`。
8. Bundle 源快照不包含 VCS 管理元数据（`.git/`）。
9. Rust 二进制是唯一入口。产品中不允许有 shell wrapper 或 dispatcher。
10. 跨平台行为在产品语义层定义，而不是留作实现细节。

## 模块映射

```text
src/
├── main.rs                 → 二进制入口、panic 处理器
├── cli.rs                  → clap 定义、命令分发
├── application/            → 命令实现（每个命令组一个文件）
├── domain/                 → newtype、枚举、共享领域记录
├── state_ledger/           → 事务、操作、插件、冲突的 CRUD
├── safety_guards/          → 备份/恢复、漂移守卫、核心影响门控
├── dependency_sync/        → UvClient、暂存工作目录、freeze、锁检查
├── source_integration/     → GitClient、插件路径映射
├── runtime_executor/       → 进程执行、PID 跟踪、日志捕获
├── platform/               → venv Python 定位、进程终止、PID 检查
├── toml_support/           → 保格式编辑 `config.toml` 与 `pyproject.toml`
└── fs_support/             → 原子写入、哈希、路径规范化、目录操作
```

## 观察、晋升与恢复

候选必须完成 ComfyUI 的原生启动探针。插件事务还需要准确的目标节点加载证据。超时、导入失败和包集漂移会阻止晋升。只有本次同步成功后的真实非零退出可以显式批准。重新暂存前先作废上次观察。

两个晋升入口共用同一套检查。它们核对生产和暂存真相的哈希、实际生产包集及解析出的解释器，并以当前核心包策略判断审批。晋升直接使用已观察的锁，提交前再次核对实际解释器与包集。旧事务缺少这些证据时，先重跑或重新 resolve。

操作账本在生产同步、冒烟和元数据写入成功后先记录提交，再更新晋升或撤销的关联。`gov op recover <op_id>` 恢复中断的生产写入，或补记已提交操作的关联。恢复失败保留备份和错误。`tx abort` 和 `update abort` 会检查事务所有者与候选的身份后再清理。旧运行中事务需要人工核实并提供 `--confirm-stopped`。

## 数据主权

| 域 | 真相来源 | 位置 |
|----|----------|------|
| Dependencies (依赖) | `pyproject.toml` + `uv.lock` | 项目根目录 |
| Configuration (配置) | `config.toml` | 项目根目录 |
| 插件注册表 | `state/plugins.json` | state 目录 |
| 事务 | `state/transactions/*.json` | state 目录 |
| 操作 | `state/ops/<op_id>/meta.json` + `backup/` | state 目录 |
| 冲突 | `state/conflicts/*.json` | state 目录 |
| 运行时 PID 与宿主进程创建身份 | `state/comfyui.pid` | state 目录 |
| 日志 | `state/logs/*` | state 目录 |

## External Tool Contracts (外部工具契约)

| 工具 | 用途 | 调用入口 |
|------|------|----------|
| `uv` | lock、sync、add、remove、pip freeze、export、python find | `UvClient` |
| `git` | clone、checkout | `GitClient` |
| Python | smoke test、ComfyUI 运行时 | `PythonClient`、`RuntimeClient` |

所有工具调用都必须通过返回 `CmdResult` 的 client struct 执行。应用层代码中不允许直接使用原始 `Command::new()`。

## 跨平台规则

完整细节见 ADR-003 第 Platform Abstraction（平台抽象）节。摘要如下：

- Venv Python: Linux 为 `<venv>/bin/python`，Windows 为 `<venv>/Scripts/python.exe`
- Linux 通过 PID 文件描述符发送 SIGTERM，超时后发送 SIGKILL。Windows 尝试 Ctrl+Break，超时后使用同一已验证进程句柄强制终止。存活的旧数字 PID 需人工核实。
- 状态文件中的路径: 始终使用正斜杠
- 原子写入: 两个平台都基于 rename，配合平台特定 fsync
- Bundle 导入: v1 拒绝跨平台导入
- Smoke test: 使用结构化命令（`program` + `args`），而不是 shell 字符串

## 关键流程

1. **插件接入**: `node add` → `tx run` → `tx inspect` → `tx promote`
2. **锁冲突解决** — 暂存锁定失败 → 用 pin 执行 `resolve` → 重新观察 → 晋升
3. **插件移除与撤销**: `node remove` → 通过备份恢复执行 `undo`
4. **引导初始化**: `init` → `install torch` → `install` → `update run` → `update promote`
5. **环境迁移**: `env export` → `env import`（v1 仅支持同平台）
