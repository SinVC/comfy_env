use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::domain::CmdResult;

pub struct GitClient;

impl GitClient {
    pub fn version() -> Result<CmdResult> {
        let output = Command::new("git")
            .arg("--version")
            .output()
            .context("failed to run `git`")?;

        Ok(CmdResult::from_output(output))
    }

    pub fn clone(url: &str, target: &Path) -> Result<CmdResult> {
        let output = Command::new("git")
            .arg("clone")
            .arg("--")
            .arg(url)
            .arg(target)
            .output()
            .context("failed to run `git clone`")?;
        Ok(CmdResult::from_output(output))
    }

    pub fn checkout(repo: &Path, reference: &str) -> Result<CmdResult> {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .arg("checkout")
            .arg(reference)
            .output()
            .context("failed to run `git checkout`")?;
        Ok(CmdResult::from_output(output))
    }
}

pub fn node_id_from_url(url: &str) -> String {
    let basename = url
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\', ':'])
        .next()
        .unwrap_or("");
    basename
        .strip_suffix(".git")
        .unwrap_or(basename)
        .to_string()
}

pub fn plugin_install_path(comfyui_dir: &Path, node_id: &str) -> PathBuf {
    comfyui_dir.join("custom_nodes").join(node_id)
}

pub fn remove_plugin_source(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    for entry in walkdir(path)? {
        let metadata = fs::metadata(&entry)?;
        if metadata.is_file() && metadata.permissions().readonly() {
            let mut permissions = metadata.permissions();
            permissions.set_readonly(false);
            fs::set_permissions(&entry, permissions)?;
        }
    }
    fs::remove_dir_all(path).with_context(|| format!("failed to remove {}", path.display()))
}

pub fn copy_plugin_source(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_plugin_source(&entry.path(), &target)?;
        } else {
            crate::fs_support::copy_file(&entry.path(), &target)?;
        }
    }
    Ok(())
}

fn walkdir(path: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            paths.extend(walkdir(&entry.path())?);
        } else {
            paths.push(entry.path());
        }
    }
    Ok(paths)
}
