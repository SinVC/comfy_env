use std::fs;
use std::io::{self, Read};
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};

use crate::cli::{
    Commands, InitArgs, InstallArgs, InstallSubcommand, InstallTorchArgs, SetupArgs, SetupMode,
};
use crate::dependency_sync::UvClient;
use crate::domain::{AppResult, CmdResult, ProjectRoot};
use crate::runtime_executor::RuntimeState;
use crate::source_integration::GitClient;
use crate::{fs_support, platform, runtime_executor, safety_guards, toml_support};

const COMFY_URL: &str = "https://github.com/Comfy-Org/ComfyUI.git";
const COMFY_TAG: &str = "v0.45.0";
const COMFY_REVISION: &str = "989d93d0d968e704c589ce0146bb18c77f6b50cc";
const SOURCE_RECEIPT: &str = ".git/gov-workspace-source";
const TEMPLATES: [&str; 2] = ["config.toml.template", "pyproject.toml.template"];
const CPU_INDEX: &str = "https://download.pytorch.org/whl/cpu";

pub(super) fn cmd_setup(args: SetupArgs) -> AppResult<i32> {
    setup(args).map_err(Into::into)
}

fn setup(args: SetupArgs) -> Result<i32> {
    checked(GitClient::version()?, "git --version")?;
    checked(UvClient::version()?, "uv --version")?;
    ensure!(
        !args.torch_index_url.is_empty() && !args.torch_index_url.chars().any(char::is_whitespace),
        "--torch-index-url must be a non-empty URL without whitespace"
    );

    let executable = std::env::current_exe()?;
    let package = executable
        .parent()
        .context("cannot locate the release package")?;
    let templates = TEMPLATES.map(|name| package.join(name));
    for template in &templates {
        ensure!(
            template.is_file(),
            "release template missing: {}",
            template.display()
        );
    }

    let workspace = std::path::absolute(&args.workspace)?;
    let comfy = workspace.join("ComfyUI");
    let sidecar = workspace.join("ComfyUI.gov");
    let bin = workspace.join("bin");
    ordinary_directory_if_present(&sidecar)?;
    ordinary_directory_if_present(&bin)?;
    let root = ProjectRoot::at(sidecar.clone());
    let existing = preflight_sidecar(&root, &comfy)?;
    preflight_source(&workspace, &comfy, args.mode, existing)?;
    if !existing {
        UvClient::resolve_python_minor(&args.python)?;
    }
    if existing {
        check_stopped(&root)?;
    }

    fs::create_dir_all(&bin)?;
    let install_lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(bin.join(".gov-setup.lock"))?;
    install_lock
        .try_lock()
        .context("another installer is using this workspace")?;

    // Check again after acquiring the setup lock. Other processes may have finished preflight.
    let existing = preflight_sidecar(&root, &comfy)?;
    let source_exists = preflight_source(&workspace, &comfy, args.mode, existing)?;
    if !source_exists {
        stage_comfy(&workspace, &comfy)?;
    }
    fs::create_dir_all(&sidecar)?;
    {
        let _guard = runtime_executor::try_lock_production(&root.pid_file())?;
        check_stopped(&root)?;
        safety_guards::ensure_production_recoverable(&root)?;
        for template in &templates {
            let name = template
                .file_name()
                .context("release template has no filename")?;
            let destination = sidecar.join(name);
            if fs_support::metadata_if_exists(&destination)?.is_none() {
                fs_support::atomic_write(&destination, &fs::read(template)?)?;
            }
        }
        install_binary(
            &executable,
            &bin.join(if cfg!(windows) { "gov.exe" } else { "gov" }),
        )?;
    }

    let cpu = args.torch_index_url.trim_end_matches('/') == CPU_INDEX;
    if !existing {
        super::dispatch_at(
            Commands::Init(InitArgs {
                comfyui_dir: Some(comfy.clone()),
                python: Some(args.python),
                cpu,
            }),
            &root,
        )
        .context("initializing the governance project")?;
    }
    let (torch_ready, core_ready) = super::status::dependency_readiness(&root)?;
    if !torch_ready {
        super::dispatch_at(
            Commands::Install(InstallArgs {
                command: Some(InstallSubcommand::Torch(InstallTorchArgs {
                    index_url: args.torch_index_url,
                    torch: None,
                    torchvision: None,
                    torchaudio: None,
                })),
                requirements_file: None,
            }),
            &root,
        )
        .context("installing PyTorch")?;
    }
    if !core_ready {
        super::dispatch_at(
            Commands::Install(InstallArgs {
                command: None,
                requirements_file: None,
            }),
            &root,
        )
        .context("installing ComfyUI core dependencies")?;
    }

    if args.mode == SetupMode::Migrate {
        report_unregistered_nodes(&root, &comfy)?;
    }
    match super::runtime::cmd_verify(&root, false).context("verifying ComfyUI startup")? {
        0 => {}
        3 if args.mode == SetupMode::Migrate => {
            println!("Installation complete. Verify startup manually with the command below.");
        }
        _ => bail!("The pinned ComfyUI revision must pass the startup probe"),
    }
    println!("Workspace: {}", workspace.display());
    println!(
        "From the workspace, run: {}",
        if cfg!(windows) {
            r".\bin\gov.exe run"
        } else {
            "./bin/gov run"
        }
    );
    Ok(0)
}

fn checked(result: CmdResult, command: &str) -> Result<String> {
    ensure!(
        result.success,
        "{command} failed: {}",
        result.summary_line()
    );
    Ok(result.stdout.trim_end_matches(['\r', '\n']).to_owned())
}

fn ordinary_directory_if_present(path: &Path) -> Result<()> {
    if let Some(metadata) = fs_support::metadata_if_exists(path)? {
        ensure!(
            metadata.is_dir() && !fs_support::is_link(&metadata),
            "expected an ordinary directory at {}",
            path.display()
        );
    }
    Ok(())
}

fn preflight_source(
    workspace: &Path,
    comfy: &Path,
    mode: SetupMode,
    existing: bool,
) -> Result<bool> {
    ordinary_directory_if_present(comfy)?;
    if !comfy.is_dir() {
        ensure!(
            mode == SetupMode::Fresh && !existing,
            "Place the ComfyUI Git checkout at {} and retry.",
            comfy.display()
        );
        return Ok(false);
    }
    let head = validate_checkout(comfy)?;
    if mode == SetupMode::Fresh {
        ensure!(
            head == COMFY_REVISION
                && fs_support::read_text_if_exists(&comfy.join(SOURCE_RECEIPT))?.as_deref()
                    == Some(source_receipt(workspace)?.as_str()),
            "Existing checkout at {} is unrecognized. Use --mode migrate or choose a new workspace.",
            comfy.display()
        );
    }
    Ok(true)
}

fn validate_checkout(comfy: &Path) -> Result<String> {
    let top = checked(
        GitClient::worktree_root(comfy)?,
        "git rev-parse --show-toplevel",
    )?;
    ensure!(
        fs::canonicalize(top)? == fs::canonicalize(comfy)?,
        "{} must itself be the root of a ComfyUI Git checkout",
        comfy.display()
    );
    let head = checked(
        GitClient::revision(comfy, "HEAD^{commit}")?,
        "git rev-parse --verify HEAD",
    )?;
    ensure!(
        comfy.join("main.py").is_file(),
        "missing {}",
        comfy.join("main.py").display()
    );
    let requirements = comfy.join("requirements.txt");
    ensure!(requirements.is_file(), "missing {}", requirements.display());
    fs::File::open(&requirements)
        .with_context(|| format!("cannot read {}", requirements.display()))?
        .read(&mut [0; 1])?;
    Ok(head)
}

fn source_receipt(workspace: &Path) -> Result<String> {
    Ok(format!(
        "{COMFY_REVISION}\n{}\n",
        fs::canonicalize(workspace)?.display()
    ))
}

fn stage_comfy(workspace: &Path, target: &Path) -> Result<()> {
    let stage = tempfile::Builder::new()
        .prefix(".comfyui-")
        .tempdir_in(workspace)?;
    let checkout = stage.path().join("ComfyUI");
    checked(
        GitClient::checkout_release(COMFY_URL, &checkout, COMFY_TAG)?,
        "clone ComfyUI",
    )?;
    ensure!(
        validate_checkout(&checkout)? == COMFY_REVISION,
        "unexpected ComfyUI release commit"
    );
    fs_support::atomic_write(
        &checkout.join(SOURCE_RECEIPT),
        source_receipt(workspace)?.as_bytes(),
    )?;
    fs_support::move_entry(&checkout, target)
        .context("publishing the validated ComfyUI checkout")?;
    Ok(())
}

fn preflight_sidecar(root: &ProjectRoot, comfy: &Path) -> Result<bool> {
    safety_guards::ensure_production_recoverable(root)?;
    let present = [root.config_toml(), root.pyproject_toml(), root.uv_lock()]
        .iter()
        .filter(|path| path.is_file())
        .count();
    if present == 0 {
        if root.as_path().is_dir() {
            for entry in fs::read_dir(root.as_path())? {
                let entry = entry?;
                let name = entry.file_name();
                if name == "state" {
                    ensure!(
                        entry.path().is_dir(),
                        "invalid state directory at {}",
                        entry.path().display()
                    );
                    for child in fs::read_dir(entry.path())? {
                        let child = child?;
                        ensure!(
                            matches!(
                                child.file_name().to_str(),
                                Some(
                                    "comfyui.pid.lock"
                                        | "ops"
                                        | "logs"
                                        | "work"
                                        | "transactions"
                                        | "conflicts"
                                        | "plugins.json"
                                )
                            ),
                            "unexpected state entry {}",
                            child.path().display()
                        );
                    }
                } else {
                    ensure!(
                        TEMPLATES.iter().any(|template| name == *template)
                            || name == ".venv-candidate",
                        "unexpected sidecar entry {}",
                        entry.path().display()
                    );
                }
            }
        }
        return Ok(false);
    }
    ensure!(
        present == 3,
        "incomplete governance configuration at {}",
        root.as_path().display()
    );
    let config = toml_support::read_config(&root.config_toml())?.context("missing config.toml")?;
    let bound = config
        .paths
        .comfyui_dir
        .context("missing paths.comfyui_dir")?;
    ensure!(
        fs::canonicalize(root.as_path().join(bound))? == fs::canonicalize(comfy)?,
        "sidecar is bound to a different ComfyUI directory"
    );
    let python = platform::venv_python(&root.as_path().join(&config.runtime.prod_env));
    ensure!(
        python.is_file(),
        "managed interpreter is missing at {}; repair the environment before setup",
        python.display()
    );
    Ok(true)
}

fn check_stopped(root: &ProjectRoot) -> Result<()> {
    match runtime_executor::verified_runtime_state(&root.pid_file())
        .context("verify that ComfyUI is stopped before retrying setup")?
    {
        RuntimeState::Stopped => Ok(()),
        RuntimeState::Running => bail!("ComfyUI is running; run gov stop before retrying setup"),
        RuntimeState::Unverified => bail!(
            "ComfyUI process identity is unverified; verify it manually before retrying setup"
        ),
    }
}

fn install_binary(source: &Path, destination: &Path) -> Result<()> {
    if destination.is_file() && fs_support::sha256(source)? == fs_support::sha256(destination)? {
        return Ok(());
    }
    let directory = destination
        .parent()
        .context("missing binary destination directory")?;
    let mut staged = tempfile::NamedTempFile::new_in(directory)?;
    let mut source = fs::File::open(source)?;
    io::copy(&mut source, staged.as_file_mut())?;
    staged
        .as_file()
        .set_permissions(source.metadata()?.permissions())?;
    staged.as_file().sync_all()?;
    staged.persist(destination).with_context(|| {
        format!(
            "could not replace {}; close running gov commands and retry",
            destination.display()
        )
    })?;
    Ok(())
}

fn report_unregistered_nodes(root: &ProjectRoot, comfy: &Path) -> Result<()> {
    let nodes = comfy.join("custom_nodes");
    if !nodes.is_dir() {
        return Ok(());
    }
    let plugins = crate::state_ledger::load_plugins(root)?;
    for entry in fs::read_dir(nodes)? {
        let entry = entry?;
        if entry.path().is_dir()
            && entry.file_name() != "__pycache__"
            && !plugins
                .iter()
                .any(|plugin| comfy.join(&plugin.install_relpath) == entry.path())
        {
            eprintln!("Unregistered custom node: {}", entry.path().display());
        }
    }
    Ok(())
}
