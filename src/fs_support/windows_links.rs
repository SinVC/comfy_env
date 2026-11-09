use std::fs::{self, File, OpenOptions};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::{io, ptr};

use anyhow::{Result, bail, ensure};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{FSCTL_GET_REPARSE_POINT, FSCTL_SET_REPARSE_POINT};

const MOUNT_POINT: u32 = 0xA0000003;
const SYMBOLIC_LINK: u32 = 0xA000000C;

pub(super) fn is_directory(metadata: &fs::Metadata) -> bool {
    metadata.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0
}

pub(super) fn copy_link(src: &Path, dst: &Path, metadata: &fs::Metadata) -> Result<()> {
    let source = open_link(src, false)?;
    let mut data = [0u8; 16 * 1024];
    let mut len = 0;
    if unsafe {
        DeviceIoControl(
            source.as_raw_handle(),
            FSCTL_GET_REPARSE_POINT,
            ptr::null(),
            0,
            data.as_mut_ptr().cast(),
            data.len() as u32,
            &mut len,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error().into());
    }
    ensure!(
        len >= 8 && len as usize <= data.len(),
        "invalid reparse buffer for {}",
        src.display()
    );
    match u32::from_le_bytes(data[..4].try_into()?) {
        SYMBOLIC_LINK if is_directory(metadata) => {
            std::os::windows::fs::symlink_dir(fs::read_link(src)?, dst)?
        }
        SYMBOLIC_LINK => std::os::windows::fs::symlink_file(fs::read_link(src)?, dst)?,
        MOUNT_POINT => {
            fs::create_dir(dst)?;
            let result = set_junction(dst, &data[..len as usize]);
            if result.is_err() {
                let _ = fs::remove_dir(dst);
            }
            result?;
        }
        _ => bail!("unsupported reparse point: {}", src.display()),
    }
    Ok(())
}

fn open_link(path: &Path, write: bool) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(write)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

fn set_junction(path: &Path, data: &[u8]) -> io::Result<()> {
    let destination = open_link(path, true)?;
    let mut written = 0;
    if unsafe {
        DeviceIoControl(
            destination.as_raw_handle(),
            FSCTL_SET_REPARSE_POINT,
            data.as_ptr().cast(),
            data.len() as u32,
            ptr::null_mut(),
            0,
            &mut written,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
