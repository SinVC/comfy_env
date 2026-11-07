use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

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
        copy_file(src, backup)?;
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
    if !path.try_exists()? {
        return Ok(());
    }

    #[cfg(windows)]
    {
        let mut permissions = fs::metadata(path)?.permissions();
        if permissions.readonly() {
            permissions.set_readonly(false);
            fs::set_permissions(path, permissions)?;
        }
    }
    fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))
}

pub fn copy_tree(src: &Path, dst: &Path, exclude: &[&str]) -> Result<()> {
    let kind = fs::symlink_metadata(src)?.file_type();
    ensure_regular_tree_entry(src, &kind)?;
    if kind.is_file() {
        return copy_file(src, dst);
    }
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if exclude
            .iter()
            .any(|skip| name == std::ffi::OsStr::new(skip))
        {
            continue;
        }
        copy_tree(&entry.path(), &dst.join(name), exclude)?;
    }
    Ok(())
}

pub fn copy_tree_preserving_links(src: &Path, dst: &Path) -> Result<()> {
    let kind = fs::symlink_metadata(src)?.file_type();
    if kind.is_symlink() {
        let link_target = fs::read_link(src)?;
        let parent = dst.parent().context("linked entry has no parent")?;
        fs::create_dir_all(parent)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&link_target, dst)?;
        #[cfg(windows)]
        if fs::metadata(src)?.is_dir() {
            std::os::windows::fs::symlink_dir(&link_target, dst)?;
        } else {
            std::os::windows::fs::symlink_file(&link_target, dst)?;
        }
        return Ok(());
    }
    ensure_regular_tree_entry(src, &kind)?;
    if kind.is_file() {
        return copy_file(src, dst);
    }
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        copy_tree_preserving_links(&entry.path(), &dst.join(entry.file_name()))?;
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
    let kind = fs::symlink_metadata(path)?.file_type();
    if kind.is_symlink() {
        if fs::metadata(path).is_ok_and(|metadata| metadata.is_dir()) {
            return fs::remove_dir(path)
                .with_context(|| format!("failed to remove link {}", path.display()));
        }
        return fs::remove_file(path)
            .with_context(|| format!("failed to remove link {}", path.display()));
    }
    ensure_regular_tree_entry(path, &kind)?;
    if kind.is_file() {
        return remove_file_if_exists(path);
    }
    for entry in fs::read_dir(path)? {
        remove_tree(&entry?.path())?;
    }
    fs::remove_dir(path).with_context(|| format!("failed to remove {}", path.display()))
}

fn sync_parent_dir(parent: &Path) {
    let _ = OpenOptions::new()
        .read(true)
        .open(parent)
        .and_then(|file| file.sync_all());
}
