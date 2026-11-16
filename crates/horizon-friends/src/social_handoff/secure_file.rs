use super::{invalid, private_regular_file};
use std::{
    fs::{File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

#[cfg(any(windows, debug_assertions, test))]
fn options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000).share_mode(1);
    }
    options
}

#[cfg(any(debug_assertions, test))]
pub(super) fn open_private(path: &Path) -> io::Result<File> {
    let file = options().open(path).map_err(|_| invalid())?;
    if !private_regular_file(&file) {
        return Err(invalid());
    }
    Ok(file)
}

pub(super) fn open_handoff(path: &Path, expected: &Path) -> io::Result<(File, File, PathBuf)> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(invalid)?;
    let nonce = name
        .strip_prefix("handoff-")
        .and_then(|name| name.strip_suffix(".json"))
        .ok_or_else(invalid)?;
    if nonce.len() != 32 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid());
    }
    let parent = path
        .parent()
        .ok_or_else(invalid)?
        .canonicalize()
        .map_err(|_| invalid())?;
    if parent != expected.canonicalize().map_err(|_| invalid())? {
        return Err(invalid());
    }
    let mut directory_options = OpenOptions::new();
    directory_options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        directory_options.custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        directory_options.custom_flags(0x0200_0000).share_mode(3);
    }
    let directory = directory_options.open(&parent).map_err(|_| invalid())?;
    #[cfg(unix)]
    let file = {
        use std::{
            ffi::CString,
            os::fd::{AsRawFd, FromRawFd},
        };
        let name = CString::new(name).map_err(|_| invalid())?;
        let descriptor = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if descriptor < 0 {
            return Err(invalid());
        }
        unsafe { File::from_raw_fd(descriptor) }
    };
    #[cfg(windows)]
    let file = {
        use std::os::windows::fs::OpenOptionsExt;
        options()
            .access_mode(0x8001_0000)
            .share_mode(4)
            .open(parent.join(name))
            .map_err(|_| invalid())?
    };
    #[cfg(not(any(unix, windows)))]
    return Err(invalid());
    #[cfg(any(unix, windows))]
    {
        if !private_regular_file(&file) {
            return Err(invalid());
        }
        Ok((file, directory, PathBuf::from(name)))
    }
}

pub(super) fn consume(file: &File, directory: &File, name: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        let _ = (directory, name);
        super::windows_acl::delete_open_file(file).map_err(|_| invalid())
    }
    #[cfg(unix)]
    {
        use std::{
            ffi::CString,
            os::{
                fd::AsRawFd,
                unix::{ffi::OsStrExt, fs::MetadataExt},
            },
        };
        let name = CString::new(name.as_os_str().as_bytes()).map_err(|_| invalid())?;
        let metadata = file.metadata().map_err(|_| invalid())?;
        let mut current = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                directory.as_raw_fd(),
                name.as_ptr(),
                current.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(invalid());
        }
        let current = unsafe { current.assume_init() };
        if current.st_dev != metadata.dev() || current.st_ino != metadata.ino() {
            return Err(invalid());
        }
        if unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            return Err(invalid());
        }
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (file, directory, name);
        Err(invalid())
    }
}
