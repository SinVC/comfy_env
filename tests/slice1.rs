use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

struct TestWorkspace {
    _temp: TempDir,
    root: PathBuf,
    fake_bin: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().to_path_buf();
        let fake_bin = root.join("fake-bin");
        fs::create_dir_all(&fake_bin).expect("fake-bin");
        fs::create_dir_all(root.join("state").join("ops")).expect("state/ops");
        fs::create_dir_all(root.join("state").join("work")).expect("state/work");
        fs::write(root.join("state").join("plugins.json"), "[]\n").expect("plugins");
        fs::write(root.join("uv.lock"), "base-lock\n").expect("uv.lock");
        fs::write(
            root.join("pyproject.toml"),
            "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependency-groups]\ncore = []\ntorch = []\noverrides = []\n",
        )
        .expect("pyproject");

        write_fake_uv(&fake_bin);
        write_fake_smoke(&fake_bin);
        let smoke_program = fake_smoke_path(&fake_bin)
            .to_string_lossy()
            .replace('\\', "\\\\");
        fs::write(
            root.join("config.toml"),
            format!(
                "[runtime]\npython = \"3.12\"\nprod_env = \".venv-prod\"\n\n[tx]\ntimeout_seconds = 30\n\n[tx.smoke_test]\nprogram = \"{smoke_program}\"\nargs = []\n"
            ),
        )
        .expect("config");

        Self {
            _temp: temp,
            root,
            fake_bin,
        }
    }

    fn write_pyproject(&self, contents: &str) {
        fs::write(self.root.join("pyproject.toml"), contents).expect("write pyproject");
    }

    fn read_pyproject(&self) -> String {
        fs::read_to_string(self.root.join("pyproject.toml")).expect("read pyproject")
    }

    fn read_uv_lock(&self) -> String {
        fs::read_to_string(self.root.join("uv.lock")).expect("read uv.lock")
    }

    fn op_ids(&self) -> Vec<String> {
        let mut ids = fs::read_dir(self.root.join("state").join("ops"))
            .expect("read ops")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    fn gov(&self) -> Command {
        let mut command = Command::cargo_bin("gov").expect("gov binary should build");
        command.current_dir(&self.root);
        command.env("PATH", prefixed_path(&self.fake_bin));
        command.env("GOV_UV_BIN", fake_uv_ok_path(&self.fake_bin));
        command
    }
}

#[test]
fn pin_success_creates_operation_and_supports_undo() {
    let workspace = TestWorkspace::new();

    workspace
        .gov()
        .args(["pin", "add", "numpy==1.26.4"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Pins added."));

    workspace
        .gov()
        .args(["pin", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("numpy==1.26.4"));

    workspace
        .gov()
        .args(["op", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("pin_add"))
        .stdout(predicate::str::contains("success"));

    let original_op = workspace
        .op_ids()
        .into_iter()
        .find(|id| {
            let output = workspace
                .gov()
                .args(["op", "inspect", id])
                .output()
                .expect("op inspect");
            String::from_utf8_lossy(&output.stdout).contains("kind: pin_add")
        })
        .expect("pin_add op id");

    workspace
        .gov()
        .args(["undo", &original_op])
        .assert()
        .success()
        .stdout(predicate::str::contains("Undo completed"));

    workspace
        .gov()
        .args(["op", "inspect", &original_op])
        .assert()
        .success()
        .stdout(predicate::str::contains("status: undone"))
        .stdout(predicate::str::contains("undo_reference:"));

    let pyproject = workspace.read_pyproject();
    assert!(!pyproject.contains("numpy==1.26.4"));
}

#[test]
fn pin_validation_errors_use_usage_exit_code() {
    let workspace = TestWorkspace::new();

    workspace
        .gov()
        .args(["pin", "add", "numpy>=1.26"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("invalid pin format"));

    workspace
        .gov()
        .args(["pin", "add", "torch==2.1.0"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "torch-family packages are managed by 'gov install torch'",
        ));
}

#[test]
fn pin_remove_requires_existing_package_atomically() {
    let workspace = TestWorkspace::new();
    workspace.write_pyproject(
        "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependency-groups]\ncore = []\ntorch = []\noverrides = [\n  \"numpy==1.26.4\",\n  \"pillow==10.0.0\",\n]\n",
    );
    let before = workspace.read_pyproject();

    workspace
        .gov()
        .args(["pin", "remove", "numpy", "transformers"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("could not be found"));

    assert_eq!(workspace.read_pyproject(), before);
}

#[test]
fn pin_rolls_back_on_lock_sync_and_smoke_failures() {
    let workspace = TestWorkspace::new();

    let before_project = workspace.read_pyproject();
    let before_lock = workspace.read_uv_lock();
    workspace
        .gov()
        .env("GOV_UV_BIN", fake_uv_lock_fail_path(&workspace.fake_bin))
        .args(["pin", "add", "transformers==4.44.0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("pin add failed during lock"));
    assert_eq!(workspace.read_pyproject(), before_project);
    assert_eq!(workspace.read_uv_lock(), before_lock);

    workspace
        .gov()
        .env("GOV_UV_BIN", fake_uv_sync_fail_path(&workspace.fake_bin))
        .args(["pin", "add", "transformers==4.44.0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("prod sync failed during pin add"));
    assert_eq!(workspace.read_pyproject(), before_project);
    assert_eq!(workspace.read_uv_lock(), before_lock);

    fs::write(
        workspace.root.join("config.toml"),
        format!(
            "[runtime]\npython = \"3.12\"\nprod_env = \".venv-prod\"\n\n[tx]\ntimeout_seconds = 30\n\n[tx.smoke_test]\nprogram = \"{}\"\nargs = []\n",
            fake_smoke_fail_path(&workspace.fake_bin)
                .to_string_lossy()
                .replace('\\', "\\\\")
        ),
    )
    .expect("rewrite config");
    workspace
        .gov()
        .args(["pin", "add", "transformers==4.44.0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("smoke test failed during pin add"));
    assert_eq!(workspace.read_pyproject(), before_project);
    assert_eq!(workspace.read_uv_lock(), before_lock);
}

#[test]
fn undo_blocks_when_truth_has_drifted() {
    let workspace = TestWorkspace::new();
    workspace
        .gov()
        .args(["pin", "add", "numpy==1.26.4"])
        .assert()
        .success();

    let op_id = workspace.op_ids().pop().expect("op id");
    fs::write(
        workspace.root.join("pyproject.toml"),
        workspace.read_pyproject() + "\n# drift\n",
    )
    .expect("drift write");

    workspace
        .gov()
        .args(["undo", &op_id])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("hash drift detected"));
}

fn prefixed_path(fake_bin: &Path) -> OsString {
    let existing = std::env::var_os("PATH").unwrap_or_default();
    let mut parts = Vec::new();
    parts.push(fake_bin.as_os_str().to_owned());
    parts.extend(std::env::split_paths(&existing).map(|path| path.into_os_string()));
    std::env::join_paths(parts).expect("join PATH")
}

fn write_fake_uv(fake_bin: &Path) {
    #[cfg(windows)]
    {
        fs::write(
            fake_bin.join("uv-ok.cmd"),
            "@echo off\r\nset cmd=%1\r\nshift\r\nif \"%cmd%\"==\"lock\" (\r\n  >uv.lock echo locked\r\n  exit /b 0\r\n)\r\nif \"%cmd%\"==\"sync\" (\r\n  exit /b 0\r\n)\r\nif \"%cmd%\"==\"--version\" (\r\n  echo uv 0.fake\r\n  exit /b 0\r\n)\r\nif \"%cmd%\"==\"python\" (\r\n  echo C:\\\\Python\\\\python.exe\r\n  exit /b 0\r\n)\r\necho unsupported uv command 1>&2\r\nexit /b 1\r\n",
        )
        .expect("write uv-ok.cmd");
        fs::write(
            fake_bin.join("uv-lock-fail.cmd"),
            "@echo off\r\nif \"%1\"==\"lock\" (\r\n  echo lock failed 1>&2\r\n  exit /b 1\r\n)\r\nif \"%1\"==\"sync\" exit /b 0\r\nif \"%1\"==\"--version\" (\r\n  echo uv 0.fake\r\n  exit /b 0\r\n)\r\nif \"%1\"==\"python\" (\r\n  echo C:\\\\Python\\\\python.exe\r\n  exit /b 0\r\n)\r\nexit /b 0\r\n",
        )
        .expect("write uv-lock-fail.cmd");
        fs::write(
            fake_bin.join("uv-sync-fail.cmd"),
            "@echo off\r\nif \"%1\"==\"lock\" (\r\n  >uv.lock echo locked\r\n  exit /b 0\r\n)\r\nif \"%1\"==\"sync\" (\r\n  echo sync failed 1>&2\r\n  exit /b 1\r\n)\r\nif \"%1\"==\"--version\" (\r\n  echo uv 0.fake\r\n  exit /b 0\r\n)\r\nif \"%1\"==\"python\" (\r\n  echo C:\\\\Python\\\\python.exe\r\n  exit /b 0\r\n)\r\nexit /b 0\r\n",
        )
        .expect("write uv-sync-fail.cmd");
    }

    #[cfg(not(windows))]
    {
        let path = fake_bin.join("uv-ok");
        fs::write(
            &path,
            "#!/bin/sh\ncmd=\"$1\"\ncase \"$cmd\" in\n  lock)\n    printf 'locked\\n' > uv.lock\n    ;;\n  sync)\n    ;;\n  --version)\n    echo 'uv 0.fake'\n    ;;\n  python)\n    echo '/usr/bin/python3'\n    ;;\n  *)\n    echo 'unsupported uv command' >&2\n    exit 1\n    ;;\nesac\n",
        )
        .expect("write uv-ok");
        make_executable(&path);

        let lock_fail = fake_bin.join("uv-lock-fail");
        fs::write(
            &lock_fail,
            "#!/bin/sh\ncmd=\"$1\"\ncase \"$cmd\" in\n  lock)\n    echo 'lock failed' >&2\n    exit 1\n    ;;\n  sync)\n    ;;\n  --version)\n    echo 'uv 0.fake'\n    ;;\n  python)\n    echo '/usr/bin/python3'\n    ;;\nesac\n",
        )
        .expect("write uv-lock-fail");
        make_executable(&lock_fail);

        let sync_fail = fake_bin.join("uv-sync-fail");
        fs::write(
            &sync_fail,
            "#!/bin/sh\ncmd=\"$1\"\ncase \"$cmd\" in\n  lock)\n    printf 'locked\\n' > uv.lock\n    ;;\n  sync)\n    echo 'sync failed' >&2\n    exit 1\n    ;;\n  --version)\n    echo 'uv 0.fake'\n    ;;\n  python)\n    echo '/usr/bin/python3'\n    ;;\nesac\n",
        )
        .expect("write uv-sync-fail");
        make_executable(&sync_fail);
    }
}

fn write_fake_smoke(fake_bin: &Path) {
    #[cfg(windows)]
    {
        fs::write(
            fake_bin.join("smoke-pass.cmd"),
            "@echo off\r\nexit /b 0\r\n",
        )
        .expect("write smoke-pass.cmd");
        fs::write(fake_bin.join("smoke-fail.cmd"), "@echo off\r\nexit /b 1\r\n")
            .expect("write smoke-fail.cmd");
    }

    #[cfg(not(windows))]
    {
        let path = fake_bin.join("smoke-pass");
        fs::write(
            &path,
            "#!/bin/sh\nexit 0\n",
        )
        .expect("write smoke-pass");
        make_executable(&path);

        let fail = fake_bin.join("smoke-fail");
        fs::write(&fail, "#!/bin/sh\nexit 1\n").expect("write smoke-fail");
        make_executable(&fail);
    }
}

#[cfg(windows)]
fn fake_uv_ok_path(fake_bin: &Path) -> PathBuf {
    fake_bin.join("uv-ok.cmd")
}

#[cfg(not(windows))]
fn fake_uv_ok_path(fake_bin: &Path) -> PathBuf {
    fake_bin.join("uv-ok")
}

#[cfg(windows)]
fn fake_smoke_path(fake_bin: &Path) -> PathBuf {
    fake_bin.join("smoke-pass.cmd")
}

#[cfg(not(windows))]
fn fake_smoke_path(fake_bin: &Path) -> PathBuf {
    fake_bin.join("smoke-pass")
}

#[cfg(windows)]
fn fake_uv_lock_fail_path(fake_bin: &Path) -> PathBuf {
    fake_bin.join("uv-lock-fail.cmd")
}

#[cfg(not(windows))]
fn fake_uv_lock_fail_path(fake_bin: &Path) -> PathBuf {
    fake_bin.join("uv-lock-fail")
}

#[cfg(windows)]
fn fake_uv_sync_fail_path(fake_bin: &Path) -> PathBuf {
    fake_bin.join("uv-sync-fail.cmd")
}

#[cfg(not(windows))]
fn fake_uv_sync_fail_path(fake_bin: &Path) -> PathBuf {
    fake_bin.join("uv-sync-fail")
}

#[cfg(windows)]
fn fake_smoke_fail_path(fake_bin: &Path) -> PathBuf {
    fake_bin.join("smoke-fail.cmd")
}

#[cfg(not(windows))]
fn fake_smoke_fail_path(fake_bin: &Path) -> PathBuf {
    fake_bin.join("smoke-fail")
}

#[cfg(not(windows))]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::metadata(path).expect("metadata");
    let mut permissions = metadata.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("set permissions");
}
