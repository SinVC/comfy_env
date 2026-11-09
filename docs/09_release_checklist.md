# 09 发布检查清单

## 1. 功能回归

1. `init`、`status`、`node add/remove` 可用。
2. `tx run/inspect/abort/promote/resolve` 链路可用。
3. `op list/inspect` 与 `undo` 可用。

## 2. 关键恢复能力

1. promote/remove 失败可自动恢复 pre-op。
2. undo 哈希校验与阻断逻辑正确。
3. `--purge-code` 场景说明清晰且行为符合不可逆约束。

## 3. 契约同步

1. `docs/04_cli_reference.md` 与实现一致。
2. `docs/05_data_contracts.md` 与 JSON 字段一致。
3. `.gitignore` 不跟踪本地真相（`config.toml`、`pyproject.toml`、`uv.lock`）和运行状态。

## 4. Rust 发布门禁

在 Windows 和 Linux 各运行以下检查。两个 Python 路径应指向不同的本机解释器。黑盒测试使用临时项目、真实 uv 和小型本地 wheel。

```text
cargo test --locked
cargo build --locked
<python> tests/slice3_black_box.py <gov>
<python> tests/release_black_box.py <gov> <alternate-python>
```

另在 Linux 从待发布提交的 LF 归档运行 `bash tests/test_gov_cli.sh`，单独记录旧 Bash 基线。实际 ComfyUI 启动冒烟单独记录平台、版本与结果。
