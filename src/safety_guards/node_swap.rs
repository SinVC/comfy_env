use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::domain::{OpId, ProjectRoot};
use crate::{fs_support, platform};

const PREFIX: &str = ".gov-node-import-";

pub struct NodePlan {
    target: PathBuf,
    parent: PathBuf,
    create_dirs: Vec<PathBuf>,
    old: Vec<OsString>,
    incoming: Vec<OsString>,
}

#[derive(Serialize, Deserialize)]
struct Journal {
    target: String,
    scratch: String,
    create_dirs: Vec<String>,
    pending: Vec<NodeMove>,
}

#[derive(Serialize, Deserialize)]
enum NodeMove {
    KeepOld(OsString),
    Publish(OsString),
}

impl NodePlan {
    pub fn inspect(target: &Path, incoming: impl Iterator<Item = OsString>) -> Result<Self> {
        let mut existing = target;
        let mut create_dirs = Vec::new();
        loop {
            if let Some(metadata) = fs_support::metadata_if_exists(existing)? {
                ensure!(
                    metadata.is_dir() && !fs_support::is_link(&metadata),
                    "custom_nodes path component is not an ordinary directory: {}",
                    existing.display()
                );
                break;
            }
            create_dirs.push(existing.to_owned());
            existing = existing
                .parent()
                .context("custom_nodes has no existing parent")?;
        }
        let parent = target.parent().context("custom_nodes has no parent")?;
        let parent = if create_dirs.is_empty() {
            parent
        } else {
            existing
        };
        #[cfg(unix)]
        if create_dirs.is_empty() {
            use std::os::unix::fs::MetadataExt;
            ensure!(
                fs::metadata(target)?.dev() == fs::metadata(parent)?.dev(),
                "cannot stage a same-volume node recovery directory outside {}",
                target.display()
            );
        }
        let mut old = Vec::new();
        let mut files = BTreeSet::new();
        let mut links = BTreeSet::new();
        if create_dirs.is_empty() {
            for entry in fs::read_dir(target)? {
                let entry = entry?;
                let metadata = fs::symlink_metadata(entry.path())?;
                if fs_support::is_link(&metadata) {
                    links.insert(name_key(&entry.file_name()));
                    println!(
                        "Preserving linked custom_nodes entry: {}",
                        entry.path().display()
                    );
                } else if metadata.is_dir() {
                    old.push(entry.file_name());
                } else {
                    files.insert(name_key(&entry.file_name()));
                    println!("Preserving custom_nodes entry: {}", entry.path().display());
                }
            }
        }
        old.sort();
        let mut selected = Vec::new();
        let mut selected_names = BTreeSet::new();
        for name in incoming {
            let key = name_key(&name);
            ensure!(
                !files.contains(&key),
                "bundle node collides with a retained file: {}",
                target.join(&name).display()
            );
            if links.contains(&key) {
                eprintln!(
                    "warning: preserving link {}; bundle source was not installed",
                    target.join(&name).display()
                );
                continue;
            }
            ensure!(
                selected_names.insert(key),
                "duplicate bundle node destination: {}",
                target.join(&name).display()
            );
            selected.push(name);
        }
        selected.sort();
        for name in &old {
            let action = if selected_names.contains(&name_key(name)) {
                "Replacing"
            } else {
                "Removing"
            };
            println!(
                "{action} custom_nodes directory: {}",
                target.join(name).display()
            );
        }
        Ok(Self {
            target: target.to_owned(),
            parent: parent.to_owned(),
            create_dirs,
            old,
            incoming: selected,
        })
    }

    pub fn prepare(
        &self,
        bundle_nodes: &Path,
        root: &ProjectRoot,
        op: &OpId,
        verify: impl Fn(&OsStr, &Path) -> Result<()>,
    ) -> Result<()> {
        let scratch = tempfile::Builder::new()
            .prefix(PREFIX)
            .tempdir_in(&self.parent)?;
        fs::create_dir(scratch.path().join("old"))?;
        fs::create_dir(scratch.path().join("new"))?;
        for name in &self.incoming {
            let destination = scratch.path().join("new").join(name);
            fs_support::copy_tree(&bundle_nodes.join(name), &destination, &[])?;
            verify(name, &destination)?;
        }
        let journal = Journal {
            target: state_path(&self.target)?,
            scratch: state_path(scratch.path())?,
            create_dirs: self
                .create_dirs
                .iter()
                .map(|dir| state_path(dir))
                .collect::<Result<_>>()?,
            pending: Vec::new(),
        };
        save(root, op, &journal)?;
        let _ = scratch.keep();
        Ok(())
    }

    pub fn publish(&self, root: &ProjectRoot, op: &OpId) -> Result<()> {
        let mut journal = load(root, op)?.context("missing node move journal")?;
        for dir in self.create_dirs.iter().rev() {
            fs::create_dir(dir).with_context(|| format!("failed to create {}", dir.display()))?;
        }
        let actions = self
            .old
            .iter()
            .cloned()
            .map(NodeMove::KeepOld)
            .chain(self.incoming.iter().cloned().map(NodeMove::Publish));
        for action in actions {
            let (from, to) = journal.paths(&action)?;
            let source = fs::symlink_metadata(&from)?;
            ensure!(
                source.is_dir() && !fs_support::is_link(&source),
                "node move source changed: {}",
                from.display()
            );
            ensure!(
                fs_support::metadata_if_exists(&to)?.is_none(),
                "node move destination changed: {}",
                to.display()
            );
            journal.pending.push(action);
            save(root, op, &journal)?;
            fs_support::move_entry(&from, &to)?;
        }
        Ok(())
    }
}

pub fn restore(root: &ProjectRoot, op: &OpId) -> Result<bool> {
    let Some(mut journal) = load(root, op)? else {
        return Ok(false);
    };
    while let Some(action) = journal.pending.last() {
        let (from, to) = journal.paths(action)?;
        match (
            fs_support::metadata_if_exists(&from)?,
            fs_support::metadata_if_exists(&to)?,
        ) {
            (Some(source), None) if source.is_dir() && !fs_support::is_link(&source) => {}
            (None, Some(source)) if source.is_dir() && !fs_support::is_link(&source) => {
                fs_support::move_entry(&to, &from)?;
            }
            _ => bail!(
                "cannot establish node move state: {} and {}",
                from.display(),
                to.display()
            ),
        }
        journal.pending.pop();
        save(root, op, &journal)?;
    }
    for directory in &journal.create_dirs {
        let directory = platform::from_state_path(directory);
        if let Some(metadata) = fs_support::metadata_if_exists(&directory)? {
            ensure!(
                metadata.is_dir() && !fs_support::is_link(&metadata),
                "created directory was replaced: {}",
                directory.display()
            );
            fs::remove_dir(&directory).with_context(|| {
                format!("failed to remove created directory {}", directory.display())
            })?;
        }
    }
    Ok(true)
}

pub fn clean(root: &ProjectRoot, op: &OpId) -> Result<()> {
    if let Some(journal) = load(root, op)? {
        let path = platform::from_state_path(&journal.scratch);
        fs_support::remove_tree(&path)
            .with_context(|| format!("node recovery directory retained at {}", path.display()))?;
    }
    Ok(())
}

pub(super) fn verify_journal(root: &ProjectRoot, op: &OpId) -> Result<()> {
    let journal = load(root, op)?.context("missing node move journal")?;
    for action in &journal.pending {
        journal.paths(action)?;
    }
    Ok(())
}

fn name_key(name: &OsStr) -> OsString {
    if cfg!(windows) {
        name.to_string_lossy().to_lowercase().into()
    } else {
        name.to_owned()
    }
}

fn state_path(path: &Path) -> Result<String> {
    ensure!(
        path.to_str().is_some(),
        "path is not valid UTF-8: {}",
        path.display()
    );
    Ok(platform::to_state_path(path))
}

impl Journal {
    fn paths(&self, action: &NodeMove) -> Result<(PathBuf, PathBuf)> {
        let target = platform::from_state_path(&self.target);
        let scratch = platform::from_state_path(&self.scratch);
        let name = match action {
            NodeMove::KeepOld(name) | NodeMove::Publish(name) => name,
        };
        ensure!(
            Path::new(name).components().eq([Component::Normal(name)]),
            "invalid node move name"
        );
        Ok(match action {
            NodeMove::KeepOld(_) => (target.join(name), scratch.join("old").join(name)),
            NodeMove::Publish(_) => (scratch.join("new").join(name), target.join(name)),
        })
    }
}

fn journal_path(root: &ProjectRoot, op: &OpId) -> PathBuf {
    root.ops_dir().join(&op.0).join("backup/node-moves.json")
}

fn save(root: &ProjectRoot, op: &OpId, journal: &Journal) -> Result<()> {
    fs_support::atomic_write(&journal_path(root, op), &serde_json::to_vec(journal)?)
}

fn load(root: &ProjectRoot, op: &OpId) -> Result<Option<Journal>> {
    let Some(text) = fs_support::read_text_if_exists(&journal_path(root, op))? else {
        return Ok(None);
    };
    let journal: Journal = serde_json::from_str(&text)?;
    let target = platform::from_state_path(&journal.target);
    let scratch = platform::from_state_path(&journal.scratch);
    let parent = scratch
        .parent()
        .context("node recovery directory has no parent")?;
    ensure!(
        target.is_absolute()
            && target.ends_with("custom_nodes")
            && target.starts_with(parent)
            && !target.starts_with(&scratch)
            && scratch
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name.starts_with(PREFIX)),
        "invalid node recovery directory: {}",
        scratch.display()
    );
    let mut next = target.as_path();
    for dir in &journal.create_dirs {
        ensure!(
            next == platform::from_state_path(dir),
            "invalid node recovery directory chain"
        );
        next = next
            .parent()
            .context("invalid node recovery directory parent")?;
    }
    ensure!(
        next.parent() == Some(parent) || next == parent,
        "invalid node recovery parent"
    );
    for path in [
        &target,
        &scratch,
        &scratch.join("new"),
        &scratch.join("old"),
    ] {
        if let Some(metadata) = fs_support::metadata_if_exists(path)? {
            ensure!(
                metadata.is_dir() && !fs_support::is_link(&metadata),
                "node recovery path was replaced: {}",
                path.display()
            );
        }
    }
    Ok(Some(journal))
}
