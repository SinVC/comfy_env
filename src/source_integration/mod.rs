use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::domain::{CmdResult, NodeId};

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

pub fn plugin_install_path(comfyui_dir: &Path, node_id: &NodeId) -> PathBuf {
    comfyui_dir.join("custom_nodes").join(&node_id.0)
}

pub fn remove_plugin_source(path: &Path) -> Result<()> {
    crate::fs_support::remove_tree(path)
}

pub fn copy_plugin_source(src: &Path, dst: &Path) -> Result<()> {
    crate::fs_support::copy_tree_preserving_links(src, dst)
}
