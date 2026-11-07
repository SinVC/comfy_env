use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, ensure};

use crate::domain::{CmdResult, ProjectRoot};
use crate::toml_support::RuntimeConfig;

pub struct UvClient {
    python: String,
}

pub struct StagedWorkdir {
    dir: tempfile::TempDir,
}

impl StagedWorkdir {
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn pyproject_toml(&self) -> PathBuf {
        self.path().join("pyproject.toml")
    }

    pub fn persist(self) -> PathBuf {
        self.dir.keep()
    }
}

impl UvClient {
    pub fn new(python: impl Into<String>) -> Self {
        Self {
            python: python.into(),
        }
    }

    pub fn version() -> Result<CmdResult> {
        run_uv(uv_command().arg("--version"))
    }

    pub fn find_python(selector: &str) -> Result<CmdResult> {
        run_uv(uv_command().args(["python", "find", "--no-python-downloads", selector]))
    }

    pub fn lock(&self, project_dir: &Path, root: &ProjectRoot) -> Result<CmdResult> {
        let mut command = uv_command();
        command
            .current_dir(project_dir)
            .args(["lock", "--python", &self.python]);
        // uv 0.6 accepts comma-separated paths or URLs for UV_FIND_LINKS.
        if let Ok(links) = std::env::var("UV_FIND_LINKS") {
            let locations = links
                .split(',')
                .map(|link| {
                    if link.is_empty() || link.contains("://") || Path::new(link).is_absolute() {
                        link.to_owned()
                    } else {
                        root.as_path().join(link).to_string_lossy().into_owned()
                    }
                })
                .collect::<Vec<_>>();
            command.env("UV_FIND_LINKS", locations.join(","));
        }
        run_uv(&mut command)
    }

    pub fn sync(&self, project_dir: &Path, env_path: &Path) -> Result<CmdResult> {
        run_uv(
            uv_command()
                .current_dir(project_dir)
                .env("UV_PROJECT_ENVIRONMENT", env_path)
                .args(["sync", "--python", &self.python, "--locked", "--all-groups"]),
        )
    }

    pub fn sync_prod(&self, root: &ProjectRoot, config: &RuntimeConfig) -> Result<()> {
        let env_path = root.as_path().join(&config.runtime.prod_env);
        let result = self.sync(root.as_path(), &env_path)?;
        ensure!(
            result.success,
            "uv sync exited {}: {}",
            result.exit_code,
            result.summary_line()
        );
        Ok(())
    }

    pub fn pip_freeze(&self, env_path: &Path) -> Result<Vec<String>> {
        if !env_path.exists() {
            return Ok(Vec::new());
        }
        let result = run_uv(
            uv_command()
                .args(["pip", "freeze", "--python"])
                .arg(crate::platform::venv_python(env_path)),
        )?;
        ensure!(
            result.success,
            "uv pip freeze failed: {}",
            result.summary_line()
        );
        Ok(result
            .stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect())
    }

    pub fn export_pylock(&self, project_dir: &Path) -> Result<CmdResult> {
        run_uv(uv_command().current_dir(project_dir).args([
            "export",
            "--format",
            "pylock.toml",
            "--locked",
            "--all-groups",
        ]))
    }

    pub fn lock_check(&self, project_dir: &Path) -> Result<CmdResult> {
        run_uv(uv_command().current_dir(project_dir).args([
            "lock",
            "--check",
            "--python",
            &self.python,
        ]))
    }
}

pub fn create_staged_workdir(root: &ProjectRoot) -> Result<StagedWorkdir> {
    let workdir = crate::fs_support::create_workdir(&root.work_dir())?;
    crate::fs_support::copy_file(
        &root.pyproject_toml(),
        &workdir.path().join("pyproject.toml"),
    )?;
    copy_lock(root.as_path(), workdir.path())?;
    Ok(StagedWorkdir { dir: workdir })
}

pub fn promote_workdir(workdir: &StagedWorkdir, root: &ProjectRoot) -> Result<()> {
    promote_staged_path(workdir.path(), root)
}

pub fn promote_staged_path(workdir: &Path, root: &ProjectRoot) -> Result<()> {
    let staged_project = workdir.join("pyproject.toml");
    let pyproject = std::fs::read(&staged_project)
        .with_context(|| format!("failed to read {}", staged_project.display()))?;
    crate::fs_support::atomic_write(&root.pyproject_toml(), &pyproject)?;

    copy_lock(workdir, root.as_path())
}

// uv records local flat-index directories relative to the lockfile. Keep the
// same directory when transferring a lock between the project and its stage.
fn copy_lock(from: &Path, to: &Path) -> Result<()> {
    let source = from.join("uv.lock");
    let text = std::fs::read_to_string(&source)
        .with_context(|| format!("failed to read {}", source.display()))?;
    let target = to.join("uv.lock");
    if !text.contains("registry") {
        return crate::fs_support::atomic_write(&target, text.as_bytes());
    }
    let mut lock = text.parse::<toml_edit::DocumentMut>()?;
    let mut changed = false;
    if let Some(packages) = lock
        .get_mut("package")
        .and_then(|item| item.as_array_of_tables_mut())
    {
        for package in packages.iter_mut() {
            let Some(registry) = package
                .get_mut("source")
                .and_then(|item| item.as_inline_table_mut())
                .and_then(|source| source.get_mut("registry"))
            else {
                continue;
            };
            let Some(path) = registry.as_str() else {
                continue;
            };
            if path.contains("://") || Path::new(path).is_absolute() {
                continue;
            }
            changed = true;
            let path = relative_index_path(from, to, Path::new(path));
            *registry = toml_edit::Value::from(crate::platform::to_state_path(&path));
        }
    }
    let content = if changed { lock.to_string() } else { text };
    crate::fs_support::atomic_write(&target, content.as_bytes())
}

fn relative_index_path(from: &Path, to: &Path, index: &Path) -> PathBuf {
    use std::path::Component;
    let mut absolute = from.to_owned();
    for component in index.components() {
        match component {
            Component::ParentDir => {
                absolute.pop();
            }
            Component::CurDir => {}
            other => absolute.push(other.as_os_str()),
        }
    }
    let shared = to
        .components()
        .zip(absolute.components())
        .take_while(|(a, b)| a == b)
        .count();
    let mut relative: PathBuf = to.components().skip(shared).map(|_| "..").collect();
    relative.extend(absolute.components().skip(shared));
    if relative.as_os_str().is_empty() {
        relative.push(".");
    }
    relative
}

fn uv_command() -> Command {
    Command::new(std::env::var_os("GOV_UV_BIN").unwrap_or_else(|| "uv".into()))
}

fn run_uv(command: &mut Command) -> Result<CmdResult> {
    let output = command.output().context("failed to run uv")?;
    Ok(CmdResult::from_output(output))
}
