use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn status_reports_foundation_state() {
    let mut command = Command::cargo_bin("gov").expect("gov binary should build");
    command
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("config_ready: false"))
        .stdout(predicate::str::contains("tool_uv:"))
        .stdout(predicate::str::contains("tool_python: managed by uv"));
}

#[test]
fn node_add_requires_config() {
    let workspace = tempfile::tempdir().expect("temporary workspace should be available");
    let mut command = Command::cargo_bin("gov").expect("gov binary should build");
    command
        .current_dir(workspace.path())
        .args(["node", "add", "https://example.com/repo.git"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error: config.toml is required"));
}
