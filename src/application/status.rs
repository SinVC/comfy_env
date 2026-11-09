use std::collections::BTreeMap;

use crate::dependency_sync::UvClient;
use crate::domain::{AppResult, CmdResult, ProjectRoot, TORCH_FAMILY, TxKind, TxStatus};
use crate::source_integration::GitClient;
use crate::state_ledger::{self, TransactionRecord};
use crate::toml_support::{self, RuntimeConfig};

pub fn cmd_status(root: &ProjectRoot) -> AppResult<()> {
    let config = toml_support::read_config(&root.config_toml())?;
    let project = toml_support::read_project(&root.pyproject_toml())?;
    let torch: Vec<_> = toml_support::dependency_group(&project, "torch")?
        .iter()
        .map(|spec| toml_support::normalize_package_name(spec))
        .collect();
    let torch_ready = TORCH_FAMILY
        .iter()
        .all(|name| torch.iter().any(|pkg| pkg == name));
    let core_ready = !toml_support::dependency_group(&project, "core")?.is_empty();
    let config_ready = config.as_ref().is_some_and(|cfg| {
        cfg.paths
            .comfyui_dir
            .as_ref()
            .is_some_and(|p| !p.as_os_str().is_empty())
            && cfg
                .runtime
                .python
                .as_deref()
                .is_some_and(|py| !py.trim().is_empty())
    });
    let plugins = state_ledger::load_plugins(root)?;
    let transactions = state_ledger::list_transactions(root)?;
    let pending = state_ledger::list_operations(root)?
        .into_iter()
        .filter(|op| op.needs_recovery())
        .map(|op| op.op_id)
        .collect::<Vec<_>>();
    let runtime = match crate::runtime_executor::runtime_status(&root.pid_file()) {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("warning: could not verify runtime identity: {error:#}");
            None
        }
    };
    let runtime_pid = runtime.map(|(pid, _)| pid);
    let runtime_running = runtime.is_some_and(|(_, alive)| alive);

    println!("project_root: {}", root.as_path().display());
    println!("config_ready: {config_ready}");
    if !pending.is_empty() {
        println!("operations_needing_recovery: {}", pending.join(", "));
    }
    println!("comfyui_dir: {}", config_comfyui_dir(config.as_ref()));
    println!("python: {}", config_python(config.as_ref()));
    println!(
        "prod_env_exists: {}",
        prod_env_exists(root, config.as_ref())
    );
    println!("torch_ready: {torch_ready}");
    println!("core_ready: {core_ready}");
    println!("lock_exists: {}", root.uv_lock().is_file());
    println!("plugins_registered: {}", plugins.len());
    println!(
        "plugins_enabled: {}",
        plugins.iter().filter(|plugin| plugin.enabled).count()
    );
    let pending = transactions.iter().filter(|tx| {
        matches!(tx.status,
        Some(status) if !matches!(status, TxStatus::Aborted | TxStatus::Promoted))
    });
    println!("transactions_total: {}", transactions.len());
    println!("transactions_pending: {}", pending.clone().count());
    println!(
        "update_transactions_pending: {}",
        pending.filter(|tx| tx.kind == TxKind::CoreUpdate).count()
    );
    println!("transactions_recent: {}", transactions.len().min(8));
    println!("runtime_running: {}", runtime_running);
    println!("tool_uv: {}", describe_probe(UvClient::version()));
    println!("tool_git: {}", describe_probe(GitClient::version()));
    println!("tool_python: {}", python_probe(config.as_ref()));

    print_transaction_counts(&transactions);
    print_recent_transactions(&transactions);
    print_notes(
        root,
        config.as_ref(),
        root.pyproject_toml().exists(),
        runtime_pid,
        runtime_running,
    );

    Ok(())
}

fn config_comfyui_dir(config: Option<&RuntimeConfig>) -> String {
    config
        .and_then(|value| value.paths.comfyui_dir.as_ref())
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn config_python(config: Option<&RuntimeConfig>) -> String {
    config
        .and_then(|value| value.runtime.python.as_ref())
        .filter(|selector| !selector.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| "-".to_string())
}

fn prod_env_exists(root: &ProjectRoot, config: Option<&RuntimeConfig>) -> bool {
    let relative = config
        .map(|value| root.as_path().join(&value.runtime.prod_env))
        .unwrap_or_else(|| root.prod_env_dir());

    relative.is_dir()
}

fn describe_probe(result: anyhow::Result<CmdResult>) -> String {
    match result {
        Ok(output) if output.success => output.summary_line(),
        Ok(output) => format!(
            "unavailable (exit {}: {})",
            output.exit_code,
            output.summary_line()
        ),
        Err(error) => format!("unavailable ({})", error.root_cause()),
    }
}

fn python_probe(config: Option<&RuntimeConfig>) -> String {
    let Some(selector) = config
        .and_then(|value| value.runtime.python.as_ref())
        .filter(|selector| !selector.trim().is_empty())
    else {
        if config.is_some() {
            return "runtime.python is not configured".to_string();
        }
        return "managed by uv during init/run (deferred until config.toml exists)".to_string();
    };

    match UvClient::find_python(selector) {
        Ok(output) if output.success => {
            format!("uv resolved '{selector}' -> {}", output.summary_line())
        }
        Ok(output) => format!(
            "uv could not resolve '{selector}' (exit {}: {})",
            output.exit_code,
            output.summary_line()
        ),
        Err(error) => format!("uv probe failed for '{selector}' ({})", error.root_cause()),
    }
}

fn print_transaction_counts(transactions: &[TransactionRecord]) {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();

    for record in transactions {
        let key = record
            .status
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "unknown".to_string());
        *counts.entry(key).or_default() += 1;
    }

    if counts.is_empty() {
        println!("transaction_counts: none");
        return;
    }

    let summary = counts
        .into_iter()
        .map(|(status, count)| format!("{status}={count}"))
        .collect::<Vec<_>>()
        .join(", ");
    println!("transaction_counts: {summary}");
}

fn print_recent_transactions(transactions: &[TransactionRecord]) {
    if transactions.is_empty() {
        println!("recent_transactions: none");
        return;
    }

    println!("recent_transactions:");
    for record in transactions.iter().take(8) {
        println!(
            "  - {} | {} | {} | {} | {}",
            display_value(&record.id),
            display_value(&record.kind.to_string()),
            record
                .status
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "unknown".to_string()),
            display_value(&record.subject),
            display_value(record.sort_key().as_deref().unwrap_or(""))
        );
    }
}

fn print_notes(
    root: &ProjectRoot,
    config: Option<&RuntimeConfig>,
    pyproject_exists: bool,
    runtime_pid: Option<u32>,
    runtime_running: bool,
) {
    let mut notes = Vec::new();

    if config.is_none() {
        notes.push(
            "config.toml is missing; python resolution stays deferred until init".to_string(),
        );
    }
    if !pyproject_exists {
        notes.push("pyproject.toml is missing; dependency truth is not initialized".to_string());
    }
    if !root.plugins_registry().exists() {
        notes.push("state/plugins.json is missing; plugin registry is not initialized".to_string());
    }
    if runtime_pid.is_some() && !runtime_running {
        notes.push("state/comfyui.pid exists but the process is not alive".to_string());
    }

    if notes.is_empty() {
        println!("notes: none");
        return;
    }

    println!("notes:");
    for note in notes {
        println!("  - {note}");
    }
}

fn display_value(value: &str) -> String {
    if value.trim().is_empty() {
        "-".to_string()
    } else {
        value.to_string()
    }
}
