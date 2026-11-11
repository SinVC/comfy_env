use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::domain::{CmdResult, NodeId};

pub struct GitClient;

impl GitClient {
    pub fn checkout_release(url: &str, target: &Path, tag: &str) -> Result<CmdResult> {
        let output = Command::new("git")
            .args(["clone", "--depth", "1", "--branch", tag, "--", url])
            .arg(target)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "Never")
            .output()
            .context("failed to clone the ComfyUI release")?;
        Ok(CmdResult::from_output(output))
    }

    pub fn revision(repo: &Path, reference: &str) -> Result<CmdResult> {
        Self::rev_parse(repo, &["--verify", reference])
    }

    pub fn worktree_root(repo: &Path) -> Result<CmdResult> {
        Self::rev_parse(repo, &["--show-toplevel"])
    }

    fn rev_parse(repo: &Path, args: &[&str]) -> Result<CmdResult> {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .arg("rev-parse")
            .args(args)
            .output()
            .context("failed to inspect the ComfyUI checkout")?;
        Ok(CmdResult::from_output(output))
    }

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
