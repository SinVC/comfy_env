use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, ensure};
use toml_edit::{DocumentMut, Item, TableLike, Value};

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

    pub fn resolve_python_minor(selector: &str) -> Result<String> {
        let selector = selector.trim();
        ensure!(!selector.is_empty(), "Python selector must not be empty");
        if let Some((major, minor)) = selector.split_once('.')
            && [major, minor]
                .into_iter()
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return Ok(format!(
                "{}.{}",
                major.parse::<u32>()?,
                minor.parse::<u32>()?
            ));
        }
        let found = Self::find_python(selector)?;
        ensure!(
            found.success,
            "target Python lookup failed: {}",
            found.summary_line()
        );
        crate::platform::PythonClient::version_minor(Path::new(&found.stdout))
    }

    pub fn lock(&self, project_dir: &Path, root: &ProjectRoot) -> Result<CmdResult> {
        run_uv(&mut self.lock_command(project_dir, root)?)
    }

    pub fn lock_tracked(
        &self,
        project_dir: &Path,
        root: &ProjectRoot,
        record: impl FnMut(Option<crate::platform::ProcessIdentity>) -> Result<()>,
    ) -> Result<CmdResult> {
        crate::runtime_executor::run_tracked_command(
            &self.lock_command(project_dir, root)?,
            None,
            record,
        )
    }

    fn lock_command(&self, project_dir: &Path, root: &ProjectRoot) -> Result<Command> {
        validate_relocatable_project(&crate::toml_support::read_project(
            &project_dir.join("pyproject.toml"),
        )?)?;
        let mut command = uv_command();
        command
            .current_dir(project_dir)
            .args(["lock", "--python", &self.python]);
        normalize_find_links(&mut command, root.as_path());
        Ok(command)
    }

    fn sync_command(&self, project_dir: &Path, env_path: &Path) -> Result<Command> {
        let mut command = uv_command();
        command
            .current_dir(project_dir)
            .env("UV_PROJECT_ENVIRONMENT", env_path)
            .args(["sync", "--python", &self.python, "--locked", "--all-groups"]);
        normalize_find_links(&mut command, &std::env::current_dir()?);
        Ok(command)
    }

    pub fn sync_tracked(
        &self,
        project_dir: &Path,
        env_path: &Path,
        record: impl FnMut(Option<crate::platform::ProcessIdentity>) -> Result<()>,
    ) -> Result<CmdResult> {
        crate::runtime_executor::run_tracked_command(
            &self.sync_command(project_dir, env_path)?,
            None,
            record,
        )
    }

    pub fn sync_prod_tracked(
        &self,
        root: &ProjectRoot,
        config: &RuntimeConfig,
        record: impl FnMut(Option<crate::platform::ProcessIdentity>) -> Result<()>,
    ) -> Result<()> {
        let env_path = root.as_path().join(&config.runtime.prod_env);
        check_sync(self.sync_tracked(root.as_path(), &env_path, record)?)
    }

    pub fn sync_prod(&self, root: &ProjectRoot, config: &RuntimeConfig) -> Result<()> {
        let env_path = root.as_path().join(&config.runtime.prod_env);
        check_sync(run_uv(&mut self.sync_command(root.as_path(), &env_path)?)?)
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
        let mut command = uv_command();
        command
            .current_dir(project_dir)
            .args(["lock", "--check", "--python", &self.python]);
        normalize_find_links(&mut command, &std::env::current_dir()?);
        run_uv(&mut command)
    }

    pub fn check_target(&self, project_dir: &Path, scratch_env: &Path) -> Result<CmdResult> {
        let check = |locked| {
            let mut command = uv_command();
            command
                .current_dir(project_dir)
                .env("UV_PROJECT_ENVIRONMENT", scratch_env)
                .args([
                    "sync",
                    "--dry-run",
                    "--all-groups",
                    "--no-python-downloads",
                    "--python",
                    &self.python,
                ]);
            normalize_find_links(&mut command, &std::env::current_dir()?);
            if locked {
                command.arg("--locked");
            }
            run_uv(&mut command)
        };
        let result = check(true)?;
        // Older uv versions forbid combining --dry-run and --locked. The
        // caller has already checked the unchanged lock with this interpreter.
        if result.exit_code == 2
            && result.stderr.contains("cannot be used with")
            && result.stderr.contains("--dry-run")
            && result.stderr.contains("--locked")
        {
            return check(false);
        }
        Ok(result)
    }
}

fn check_sync(result: CmdResult) -> Result<()> {
    ensure!(
        result.success,
        "uv sync exited {}: {}",
        result.exit_code,
        result.summary_line()
    );
    Ok(())
}

fn normalize_find_links(command: &mut Command, cwd: &Path) {
    // uv 0.6 accepts comma-separated paths or URLs for UV_FIND_LINKS.
    if let Ok(links) = std::env::var("UV_FIND_LINKS") {
        let locations = links
            .split(',')
            .map(|link| {
                if link.is_empty() || link.contains("://") || Path::new(link).is_absolute() {
                    link.to_owned()
                } else {
                    cwd.join(link).to_string_lossy().into_owned()
                }
            })
            .collect::<Vec<_>>();
        command.env("UV_FIND_LINKS", locations.join(","));
    }
}

pub fn create_staged_workdir(root: &ProjectRoot) -> Result<StagedWorkdir> {
    validate_relocatable_project(&crate::toml_support::read_project(&root.pyproject_toml())?)?;
    let workdir = crate::fs_support::create_workdir(&root.work_dir())?;
    crate::fs_support::copy_file(
        &root.pyproject_toml(),
        &workdir.path().join("pyproject.toml"),
    )?;
    copy_lock(root.as_path(), workdir.path())?;
    Ok(StagedWorkdir { dir: workdir })
}

pub fn stage_init(root: &ProjectRoot, project: &str) -> Result<StagedWorkdir> {
    validate_relocatable_project(&project.parse::<DocumentMut>()?)?;
    let dir = crate::fs_support::create_workdir(&root.work_dir())?;
    crate::fs_support::atomic_write(&dir.path().join("pyproject.toml"), project.as_bytes())?;
    if root.uv_lock().try_exists()? {
        copy_lock(root.as_path(), dir.path())?;
    }
    Ok(StagedWorkdir { dir })
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
pub fn copy_lock(from: &Path, to: &Path) -> Result<()> {
    let content = relocated_lock(from, to)?;
    crate::fs_support::atomic_write(&to.join("uv.lock"), content.as_bytes())
}

fn relocated_lock(from: &Path, to: &Path) -> Result<String> {
    let source = from.join("uv.lock");
    let text = std::fs::read_to_string(&source)
        .with_context(|| format!("failed to read {}", source.display()))?;
    relocate_lock_contents(from, to, text)
}

pub fn relocate_lock_contents(from: &Path, to: &Path, text: String) -> Result<String> {
    if !text.contains("source") {
        return Ok(text);
    }
    let mut lock = text.parse::<toml_edit::DocumentMut>()?;
    let mut changed = false;
    if let Some(packages) = lock
        .get_mut("package")
        .and_then(|item| item.as_array_of_tables_mut())
    {
        for package in packages.iter_mut() {
            let Some(source) = package
                .get_mut("source")
                .and_then(|item| item.as_inline_table_mut())
            else {
                continue;
            };
            for key in ["directory", "editable", "path", "url"] {
                if let Some(path) = source.get(key).and_then(Value::as_str) {
                    if key != "url" || path.starts_with("file:") {
                        ensure_portable_location(path, &format!("uv.lock source.{key}"))?;
                    }
                }
            }
            let Some(registry) = source.get_mut("registry") else {
                continue;
            };
            let Some(path) = registry.as_str() else {
                continue;
            };
            if path.contains("://") || Path::new(path).is_absolute() {
                continue;
            }
            changed = true;
            let path = relative_index_path(from, to, Path::new(path))?;
            *registry = toml_edit::Value::from(crate::platform::to_state_path(&path));
        }
    }
    Ok(if changed { lock.to_string() } else { text })
}

pub fn validate_relocatable_project(project: &DocumentMut) -> Result<()> {
    let uv = project.get("tool").and_then(|item| item.get("uv"));
    if let Some(uv) = uv {
        ensure!(
            uv.get("workspace").is_none(),
            "cannot relocate tool.uv.workspace with a single-project snapshot"
        );
        if let Some(sources) = uv.get("sources").and_then(Item::as_table_like) {
            for (name, source) in sources.iter() {
                let sources = source
                    .as_array()
                    .map(|array| array.iter().collect::<Vec<_>>())
                    .unwrap_or_else(|| source.as_value().into_iter().collect());
                for source in sources {
                    if let Some(table) = source.as_inline_table() {
                        validate_source_table(table, name)?;
                    }
                }
                if let Some(table) = source.as_table() {
                    validate_source_table(table, name)?;
                }
            }
        }
        if let Some(indexes) = uv.get("index").and_then(Item::as_array_of_tables) {
            for index in indexes {
                if let Some(url) = index.get("url").and_then(Item::as_str) {
                    ensure_portable_location(url, "tool.uv.index.url")?;
                }
            }
        }
        if let Some(links) = uv.get("find-links").and_then(Item::as_array) {
            for location in links.iter().filter_map(Value::as_str) {
                ensure_portable_location(location, "tool.uv.find-links")?;
            }
        }
    }
    let metadata = project.get("project");
    let core = metadata.and_then(|item| item.get("dependencies"));
    let optional = metadata.and_then(|item| item.get("optional-dependencies"));
    let groups = project.get("dependency-groups");
    for requirements in core.into_iter().chain(
        [optional, groups]
            .into_iter()
            .flatten()
            .filter_map(Item::as_table_like)
            .flat_map(|table| table.iter().map(|(_, item)| item)),
    ) {
        if let Some(array) = requirements.as_array() {
            for spec in array.iter().filter_map(Value::as_str) {
                if let Some((_, reference)) = spec.split_once('@') {
                    let location = reference.split(';').next().unwrap_or(reference).trim();
                    ensure_portable_location(location, "direct dependency")?;
                }
            }
        }
    }
    Ok(())
}

fn validate_source_table(table: &dyn TableLike, name: &str) -> Result<()> {
    ensure!(
        !table
            .get("workspace")
            .and_then(Item::as_bool)
            .unwrap_or(false),
        "cannot relocate workspace source for {name}"
    );
    for key in ["path", "url"] {
        if let Some(location) = table.get(key).and_then(Item::as_str) {
            ensure_portable_location(location, &format!("tool.uv.sources.{name}.{key}"))?;
        }
    }
    Ok(())
}

fn ensure_portable_location(location: &str, context: &str) -> Result<()> {
    let absolute = Path::new(location).is_absolute()
        || location.contains("://")
        || location
            .strip_prefix("file:")
            .is_some_and(|path| Path::new(path).is_absolute());
    ensure!(
        absolute,
        "cannot relocate relative source in {context}: {location}"
    );
    Ok(())
}

fn relative_index_path(from: &Path, to: &Path, index: &Path) -> Result<PathBuf> {
    use std::path::Component;
    let mut absolute = from.canonicalize()?;
    let to = to.canonicalize()?;
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
    ensure!(
        shared > 0,
        "relative wheel registry cannot move across volumes: {}",
        index.display()
    );
    let mut relative: PathBuf = to.components().skip(shared).map(|_| "..").collect();
    relative.extend(absolute.components().skip(shared));
    if relative.as_os_str().is_empty() {
        relative.push(".");
    }
    Ok(relative)
}

fn uv_command() -> Command {
    Command::new(std::env::var_os("GOV_UV_BIN").unwrap_or_else(|| "uv".into()))
}

fn run_uv(command: &mut Command) -> Result<CmdResult> {
    let output = command.output().context("failed to run uv")?;
    Ok(CmdResult::from_output(output))
}
