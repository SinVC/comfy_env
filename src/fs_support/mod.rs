use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

pub fn read_text_if_exists(path: &Path) -> std::io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub fn atomic_write(target: &Path, content: &[u8]) -> Result<()> {
    let parent = target
        .parent()
        .with_context(|| format!("{} has no parent directory", target.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;

    let mut temp = NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temp file in {}", parent.display()))?;
    temp.write_all(content)
        .with_context(|| format!("failed to write temp file for {}", target.display()))?;
    temp.as_file_mut()
        .sync_all()
        .with_context(|| format!("failed to flush temp file for {}", target.display()))?;

    temp.persist(target)
        .with_context(|| format!("failed to replace {}", target.display()))?;

    sync_parent_dir(parent);
    Ok(())
}

pub fn sha256(path: &Path) -> Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];

    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

pub fn sha256_if_exists(path: &Path) -> Result<Option<String>> {
    if !path.try_exists()? {
        return Ok(None);
    }

    sha256(path).map(Some)
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn create_workdir(base: &Path) -> Result<tempfile::TempDir> {
    fs::create_dir_all(base).with_context(|| format!("failed to create {}", base.display()))?;
    tempfile::Builder::new()
        .prefix("gov-")
        .tempdir_in(base)
        .with_context(|| format!("failed to create workdir in {}", base.display()))
}

pub fn copy_file(src: &Path, dst: &Path) -> Result<()> {
    let parent = dst
        .parent()
        .with_context(|| format!("{} has no parent directory", dst.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    fs::copy(src, dst)
        .with_context(|| format!("failed to copy {} to {}", src.display(), dst.display()))?;
    Ok(())
}

pub fn backup_with_marker(src: &Path, backup: &Path, marker: &Path) -> Result<()> {
    if src.try_exists()? {
        atomic_write(backup, &fs::read(src)?)?;
        atomic_write(marker, b"1")?;
    } else {
        atomic_write(marker, b"0")?;
    }
    Ok(())
}

pub fn restore_with_marker(backup: &Path, dst: &Path, marker: &Path) -> Result<()> {
    let marker_value = fs::read_to_string(marker)
        .with_context(|| format!("failed to read {}", marker.display()))?;

    match marker_value.trim() {
        "1" => {
            let bytes =
                fs::read(backup).with_context(|| format!("failed to read {}", backup.display()))?;
            atomic_write(dst, &bytes)?;
        }
        "0" => remove_file_if_exists(dst)?,
        other => bail!("invalid existence marker `{other}` in {}", marker.display()),
    }

    Ok(())
}

pub fn remove_file_if_exists(path: &Path) -> Result<()> {
    let Some(metadata) = metadata_if_exists(path)? else {
        return Ok(());
    };
    if is_link(&metadata) {
        return remove_link(path, &metadata);
    }

    #[cfg(windows)]
    {
        let mut permissions = metadata.permissions();
        if permissions.readonly() {
            permissions.set_readonly(false);
            fs::set_permissions(path, permissions)?;
        }
    }
    fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))
}

pub fn copy_tree(src: &Path, dst: &Path, exclude: &[&str]) -> Result<()> {
    copy_tree_with_links(src, dst, exclude, false)
}

pub fn copy_tree_preserving_links(src: &Path, dst: &Path) -> Result<()> {
    copy_tree_with_links(src, dst, &[], true)
}

fn copy_tree_with_links(src: &Path, dst: &Path, exclude: &[&str], preserve: bool) -> Result<()> {
    let metadata = fs::symlink_metadata(src)?;
    if is_link(&metadata) {
        anyhow::ensure!(
            preserve,
            "linked bundle entry is not supported: {}",
            src.display()
        );
        fs::create_dir_all(dst.parent().context("linked entry has no parent")?)?;
        return copy_link(src, dst, &metadata);
    }
    let kind = metadata.file_type();
    ensure_regular_tree_entry(src, &kind)?;
    if kind.is_file() {
        return copy_file(src, dst);
    }
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if !exclude
            .iter()
            .any(|skip| name == std::ffi::OsStr::new(skip))
        {
            copy_tree_with_links(&entry.path(), &dst.join(name), exclude, preserve)?;
        }
    }
    Ok(())
}

fn ensure_regular_tree_entry(path: &Path, kind: &fs::FileType) -> Result<()> {
    anyhow::ensure!(
        kind.is_file() || kind.is_dir(),
        "linked or special entry is not supported: {}",
        path.display()
    );
    Ok(())
}

pub fn remove_tree(path: &Path) -> Result<()> {
    let Some(metadata) = metadata_if_exists(path)? else {
        return Ok(());
    };
    if is_link(&metadata) {
        return remove_link(path, &metadata);
    }
    let kind = metadata.file_type();
    ensure_regular_tree_entry(path, &kind)?;
    if kind.is_file() {
        return remove_file_if_exists(path);
    }
    for entry in fs::read_dir(path)? {
        remove_tree(&entry?.path())?;
    }
    fs::remove_dir(path).with_context(|| format!("failed to remove {}", path.display()))
}

pub fn metadata_if_exists(path: &Path) -> Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to inspect {}", path.display())),
    }
}

pub fn resolved_path(path: &Path) -> Result<PathBuf> {
    let mut ancestor = path;
    let mut missing = Vec::new();
    loop {
        match ancestor.canonicalize() {
            Ok(mut resolved) => {
                for component in missing.into_iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                anyhow::ensure!(
                    metadata_if_exists(ancestor)?.is_none(),
                    "dangling path: {}",
                    ancestor.display()
                );
                missing.push(
                    ancestor
                        .file_name()
                        .context("path has no existing ancestor")?,
                );
                ancestor = ancestor.parent().context("path has no existing ancestor")?;
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to resolve {}", ancestor.display()));
            }
        }
    }
}

pub fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(unix)]
    metadata.file_type().is_symlink()
}

#[cfg(unix)]
fn copy_link(src: &Path, dst: &Path, _: &fs::Metadata) -> Result<()> {
    std::os::unix::fs::symlink(fs::read_link(src)?, dst)?;
    Ok(())
}

#[cfg(windows)]
mod windows_links;
#[cfg(windows)]
use windows_links::copy_link;

fn remove_link(path: &Path, _metadata: &fs::Metadata) -> Result<()> {
    #[cfg(windows)]
    if windows_links::is_directory(_metadata) {
        return fs::remove_dir(path)
            .with_context(|| format!("failed to remove link {}", path.display()));
    }
    fs::remove_file(path).with_context(|| format!("failed to remove link {}", path.display()))
}

pub fn move_entry(from: &Path, to: &Path) -> Result<()> {
    anyhow::ensure!(
        metadata_if_exists(to)?.is_none(),
        "move destination already exists: {}",
        to.display()
    );
    fs::rename(from, to)
        .with_context(|| format!("failed to move {} to {}", from.display(), to.display()))?;
    for parent in [from.parent(), to.parent()].into_iter().flatten() {
        sync_parent_dir(parent);
    }
    Ok(())
}

fn sync_parent_dir(parent: &Path) {
    let _ = OpenOptions::new()
        .read(true)
        .open(parent)
        .and_then(|file| file.sync_all());
}
