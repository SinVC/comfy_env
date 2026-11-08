use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(unix)]
#[path = "process_unix.rs"]
mod process;
#[cfg(windows)]
#[path = "process_windows.rs"]
mod process;

#[cfg(windows)]
pub use process::keep_launcher_alive_on_break;
pub use process::{
    CandidateProcess, RunInterrupts, check_process_alive, exit_code, process_creation,
    terminate_process,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub creation: String,
}

impl ProcessIdentity {
    pub fn capture(pid: u32) -> anyhow::Result<Self> {
        let creation = process_creation(pid)?.ok_or_else(|| {
            anyhow::anyhow!("process {pid} exited before its identity was recorded")
        })?;
        Ok(Self { pid, creation })
    }

    pub fn is_alive(&self) -> anyhow::Result<bool> {
        Ok(check_process_alive(self.pid)?
            && process_creation(self.pid)?.as_ref() == Some(&self.creation))
    }

    pub fn candidate_is_alive(&self) -> anyhow::Result<bool> {
        process::candidate_is_alive(self)
    }
}

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
                "import platform; print(platform.python_version())",
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

pub fn sys_platform() -> &'static str {
    match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        os => os,
    }
}

pub fn platform_machine() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "AMD64",
        ("windows", "aarch64") => "ARM64",
        ("macos", "aarch64") => "arm64",
        (_, arch) => arch,
    }
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
