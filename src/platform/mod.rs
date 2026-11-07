#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

pub struct PythonClient;

impl PythonClient {
    pub fn version_minor(interpreter: &Path) -> anyhow::Result<String> {
        let output = Command::new(interpreter)
            .args([
                "-B",
                "-c",
                "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}')",
            ])
            .output()?;
        let result = crate::domain::CmdResult::from_output(output);
        anyhow::ensure!(
            result.success,
            "python version lookup failed: {}",
            result.summary_line()
        );
        Ok(result.stdout)
    }

    pub fn version_full(interpreter: &Path) -> anyhow::Result<String> {
        let output = Command::new(interpreter)
            .args([
                "-B",
                "-c",
                "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}.{sys.version_info.micro}')",
            ])
            .output()?;
        let result = crate::domain::CmdResult::from_output(output);
        anyhow::ensure!(
            result.success,
            "python version lookup failed: {}",
            result.summary_line()
        );
        Ok(result.stdout)
    }
}

pub fn venv_python(venv_root: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        return venv_root.join("Scripts").join("python.exe");
    }

    #[cfg(not(windows))]
    {
        venv_root.join("bin").join("python")
    }
}

pub fn is_absolute(path: &Path) -> bool {
    path.is_absolute()
}

pub fn to_state_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub fn from_state_path(path: &str) -> PathBuf {
    #[cfg(windows)]
    {
        return PathBuf::from(path.replace('/', "\\"));
    }

    #[cfg(not(windows))]
    {
        PathBuf::from(path)
    }
}

#[cfg(windows)]
pub fn is_process_alive(pid: u32) -> bool {
    let filter = format!("PID eq {pid}");
    let Ok(output) = Command::new("tasklist")
        .args(["/FI", &filter, "/FO", "CSV", "/NH"])
        .output()
    else {
        return false;
    };

    if !output.status.success() {
        return false;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.contains(&format!("\"{pid}\""))
}

#[cfg(not(windows))]
pub fn is_process_alive(pid: u32) -> bool {
    PathBuf::from(format!("/proc/{pid}")).exists()
}

#[cfg(test)]
mod tests {
    use super::{from_state_path, to_state_path, venv_python};

    #[test]
    fn state_paths_use_forward_slashes() {
        let path = std::path::Path::new("state\\transactions\\example.json");
        assert_eq!(to_state_path(path), "state/transactions/example.json");
    }

    #[test]
    fn state_paths_round_trip() {
        let restored = from_state_path("custom_nodes/example-node");
        assert!(restored.ends_with("example-node"));
    }

    #[test]
    fn venv_python_path_matches_platform() {
        let path = venv_python(std::path::Path::new(".venv-prod"));
        let rendered = path.to_string_lossy();

        #[cfg(windows)]
        assert!(rendered.ends_with("Scripts\\python.exe"));

        #[cfg(not(windows))]
        assert!(rendered.ends_with("bin/python"));
    }
}
