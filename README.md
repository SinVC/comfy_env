# Comfy Environment Governance (`gov`)

<p align="center">
  <b>Deterministic, transactional, and agent-native sidecar dependency governance for ComfyUI.</b>
</p>

<p align="center">
  <a href="README.md"><b>English</b></a> •
  <a href="README.zh-CN.md"><b>简体中文</b></a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Platform-Linux%20%7C%20Windows-blue.svg" alt="Platform: Linux | Windows" />
  <img src="https://img.shields.io/badge/Rust-Single%20Binary-orange.svg" alt="Rust" />
  <img src="https://img.shields.io/badge/Engine-uv-purple.svg" alt="uv" />
  <img src="https://img.shields.io/badge/Safety-Transactional%20Rollback-green.svg" alt="Safety" />
</p>

---

## Overview

### The Problem: Custom-Node Dependency Chaos
Anyone running ComfyUI has experienced the nightmare: installing custom nodes often triggers arbitrary `pip install` commands that silently clobber PyTorch/CUDA builds, downgrade critical libraries, or inject conflicting dependencies. Once the Python virtual environment breaks, diagnosing root causes is difficult, uninstallation leaves toxic package residue, and recovery typically requires wiping and reinstalling the entire environment from scratch.

### The Solution: Sidecar Transactional Governance
`comfy_env` introduces **`gov`**, a high-reliability sidecar governance engine written in Rust. Powered by Astral's [`uv`](https://github.com/astral-sh/uv), `gov` isolates, stages, audits, and locks dependency changes:
- **Sandbox Observation**: Custom node dependencies are staged and verified in an isolated candidate environment (`.venv-candidate`) before touching production (`.venv-prod`).
- **Verifiable Rollbacks**: Every mutating operation creates a pre-operation cryptographic snapshot, allowing instant and deterministic rollbacks via `gov undo`.
- **Core Drift Protection**: Custom nodes cannot silently alter PyTorch or ComfyUI core dependencies without explicit policy approval (`--approve-core`).

### Why a CLI? Built for Autonomous AI Agents
Modern generative AI workflows increasingly rely on autonomous AI agents (such as Claude Code, AutoGPT, or IDE coding assistants) to install nodes, configure workflows, and orchestrate environments. GUIs and interactive prompts are notoriously brittle and stall automated agent loops.

`gov` is designed **agent-first**:
- **Deterministic & Scriptable**: Structured CLI flags, predictable exit codes, and non-interactive workflows provide seamless automation without prompt traps.
- **Machine-Inspectable State**: All transactions, operations, and lock conflicts produce structured JSON and lockfile artifacts (`state/transactions/`, `state/conflicts/`) that agents can easily parse and reason about.
- **Autonomous Self-Healing**: When dependency conflicts emerge, agents can inspect the conflict reports, programmatically resolve pins via `gov resolve`, or instantly roll back failing operations via `gov undo`.

---

## Architecture at a Glance

`comfy_env` runs alongside your ComfyUI installation using a clean **sidecar model**, leaving upstream ComfyUI untouched:

```text
Workspace/
├── ComfyUI/              # Upstream ComfyUI (unpolluted code repository)
│   ├── main.py
│   ├── requirements.txt
│   └── custom_nodes/     # Managed custom node installations
│
└── comfy_env/            # Sidecar Governance Directory
    ├── gov / gov.exe     # Standalone Rust binary (sole entry point)
    ├── config.toml       # Local path & Python runtime configuration
    ├── pyproject.toml    # Declarative dependency groups (uv)
    ├── uv.lock           # Locked, reproducible dependency tree
    ├── .venv-prod/       # Production Python virtual environment
    ├── .venv-candidate/  # Isolated sandbox for candidate transactions
    └── state/            # Transaction ledgers, operation backups & conflict logs
```

---

## Install a workspace

Install Git, uv, and Python 3.12 first. The installer uses the CPU PyTorch index by default. To use a GPU-specific index, supply `--torch-index-url` on Linux or set `GOV_TORCH_INDEX_URL` on Windows.

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

To use an existing ComfyUI Git checkout, place it at `ComfyUIWorkspace/ComfyUI/` before running the installer. Pass `--mode migrate` on Linux or set `$env:GOV_SETUP_MODE = 'migrate'` on Windows. The installer builds a fresh managed Python environment from the checkout's requirements. Manage models and model paths through ComfyUI.

From the workspace, start ComfyUI with `./bin/gov run` or `.\bin\gov.exe run`. The optional Agent Skill is in [`skills/gov-cli/`](skills/gov-cli/SKILL.md). Install it from the same Git tag as the CLI.

## Quick Start

### 1. Initialize & Bootstrap
Initialize the governance configuration, bind to Python, and configure PyTorch and core requirements:

```bash
# 1. Initialize configuration and production environment
gov init --comfyui-dir /path/to/ComfyUI --python 3.12

# 2. Install and pin PyTorch with matching CUDA/CPU index
gov install torch --index-url https://download.pytorch.org/whl/cu124

# 3. Import baseline ComfyUI requirements into dependency-groups.core
gov install
```

### 2. The Hero Flow: Safe Custom Node Installation
Never run unmanaged `pip install` again. Safely stage, observe, and promote node changes:

```bash
# 1. Register and clone custom node
gov node add https://github.com/example/custom-node.git

# 2. Run candidate transaction in sandbox (.venv-candidate)
gov tx run custom-node

# 3. Inspect proposed lock diff and dependency impacts
gov tx inspect <txid>

# 4. Promote safely into production (.venv-prod)
gov tx promote <txid>
```

*If dependency conflicts occur, inspect the conflict report and resolve via `gov resolve <txid>` before promotion.*

### 3. Inspection & Instant Rollback
Audit system health or roll back any change safely:

```bash
# View comprehensive system status & health
gov status

# List mutation operations
gov op list

# Roll back any mutation using its pre-op snapshot
gov undo <op_id>
```

---

## Documentation Navigation

The repository contains extensive, detailed specifications and implementation guides. Follow the links below for deep dives:

| Topic | Document Link | Description |
| :--- | :--- | :--- |
| **System Architecture** | [Architecture Overview](dev-docs/architecture.md) ([中文版](dev-docs/architecture.zh-CN.md)) | System invariants, module boundaries, and cross-platform design |
| **Design Objectives** | [Project Objectives](docs/01_project_objectives.md) | Problem analysis, design philosophy, and success metrics |
| **Agent / LLM Guide** | [Quick Start for LLMs & Agents](docs/10_quick_start_for_llm.md) | Mental models, command quick-reference, and agent execution recipes |
| **Command Contracts** | [Command Contracts Index](dev-docs/commands/contracts.md) | Exhaustive behavioral specifications for every CLI command |
| **CLI Reference** | [CLI Reference Manual](docs/cli-reference.md) ([中文版](docs/cli-reference.zh-CN.md)) | Complete CLI options, flags, and parameter reference |
| **Node Management** | [Node Commands](dev-docs/commands/node.md) & [Tx Commands](dev-docs/commands/tx.md) | Node lifecycle and candidate transaction management |
| **Core Updates** | [Update Commands](dev-docs/commands/update.md) | Staging and promoting ComfyUI core dependency upgrades |
| **Dependency Overrides** | [Pin Governance](docs/12_gov_pin_handoff.md) | Global package pins and version override management (`gov pin`) |
| **Runtime & Operations** | [Runtime & Operations Guide](docs/03_runtime_and_operations.md) | Production locking, process supervision (`gov run`), and recovery |
| **Environment Portability** | [Export & Import Handoff](docs/11_env_export_import_handoff.md) | Exporting and importing reproducible environment bundles (`gov env`) |

---

## Building from Source

`gov` is written in Rust and compiles to a single native binary on both Linux and Windows:

```bash
# Build release binary
cargo build --release

# Output binary location:
# Linux:   target/release/gov
# Windows: target/release/gov.exe
```

---

## License

This project is licensed under the Apache 2.0 / MIT Dual License.
