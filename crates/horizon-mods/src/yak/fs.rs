use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::path::Path;

fn check_name(value: &OsStr) -> io::Result<()> {
    let value = value.to_str().ok_or_else(unsafe_entry)?;
    if value.is_empty()
        || value == "."
        || value == ".."
        || value
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\' | ':'))
    {
        return Err(unsafe_entry());
    }
    Ok(())
}

fn unsafe_entry() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "unsafe storage filesystem entry",
    )
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::ffi::{CStr, CString};
    use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    #[cfg(target_os = "linux")]
    const DIRECTORY_ACCESS: i32 = libc::O_PATH;
    #[cfg(not(target_os = "linux"))]
    const DIRECTORY_ACCESS: i32 = libc::O_SEARCH;

    #[derive(Debug)]
    pub struct Dir(File);

    fn name(value: &OsStr) -> io::Result<CString> {
        CString::new(value.as_bytes()).map_err(|_| unsafe_entry())
    }

    fn open_at(dir: &File, value: &OsStr, flags: i32, mode: u32) -> io::Result<File> {
        let value = name(value)?;
        let descriptor = unsafe { libc::openat(dir.as_raw_fd(), value.as_ptr(), flags, mode) };
        if descriptor < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(descriptor) })
        }
    }

    impl Dir {
        pub fn open(path: &Path, _create: bool) -> io::Result<Self> {
            if !path.is_absolute() {
                return Err(unsafe_entry());
            }
            let path = name(path.as_os_str())?;
            let descriptor = unsafe {
                libc::open(
                    path.as_ptr(),
                    DIRECTORY_ACCESS | libc::O_DIRECTORY | libc::O_CLOEXEC,
                )
            };
            if descriptor < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(Self(unsafe { File::from_raw_fd(descriptor) }))
            }
        }

        pub fn child(&self, value: &OsStr, create: bool) -> io::Result<Self> {
            check_name(value)?;
            let flags = DIRECTORY_ACCESS | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW;
            match open_at(&self.0, value, flags, 0) {
                Ok(file) => Ok(Self(file)),
                Err(error) if create && error.kind() == io::ErrorKind::NotFound => {
                    let value = name(value)?;
                    let result =
                        unsafe { libc::mkdirat(self.0.as_raw_fd(), value.as_ptr(), 0o700) };
                    if result != 0 {
                        let error = io::Error::last_os_error();
                        if error.kind() != io::ErrorKind::AlreadyExists {
                            return Err(error);
                        }
                    }
                    open_at(&self.0, OsStr::from_bytes(value.as_bytes()), flags, 0).map(Self)
                }
                Err(error) => Err(error),
            }
        }

        pub fn read(&self, value: &OsStr) -> io::Result<Option<File>> {
            check_name(value)?;
            match open_at(
                &self.0,
                value,
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
                0,
            ) {
                Ok(file) if file.metadata()?.is_file() => Ok(Some(file)),
                Ok(_) => Err(unsafe_entry()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
            }
        }

        pub fn create(&self, value: &OsStr) -> io::Result<File> {
            check_name(value)?;
            open_at(
                &self.0,
                value,
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                0o600,
            )
        }

        pub fn lock(&self, value: &OsStr) -> io::Result<File> {
            check_name(value)?;
            let file = open_at(
                &self.0,
                value,
                libc::O_RDWR
                    | libc::O_CREAT
                    | libc::O_CLOEXEC
                    | libc::O_NOFOLLOW
                    | libc::O_NONBLOCK,
                0o600,
            )?;
            if !file.metadata()?.is_file() {
                return Err(unsafe_entry());
            }
            file.try_lock().map_err(io::Error::from)?;
            Ok(file)
        }

        pub fn entries(&self) -> io::Result<Vec<OsString>> {
            let file = open_at(
                &self.0,
                OsStr::new("."),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
                0,
            )?;
            let descriptor = file.into_raw_fd();
            let directory = unsafe { libc::fdopendir(descriptor) };
            if directory.is_null() {
                unsafe {
                    libc::close(descriptor);
                }
                return Err(io::Error::last_os_error());
            }
            let mut names = Vec::new();
            loop {
                let entry = unsafe { libc::readdir(directory) };
                if entry.is_null() {
                    break;
                }
                let value = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
                if value != b"." && value != b".." {
                    names.push(OsString::from_vec(value.to_vec()));
                }
            }
            unsafe {
                libc::closedir(directory);
            }
            Ok(names)
        }

        pub fn replace(&self, source: &OsStr, destination: &OsStr) -> io::Result<()> {
            check_name(source)?;
            check_name(destination)?;
            let _existing = self.read(destination)?;
            let source = name(source)?;
            let destination = name(destination)?;
            let directory = open_at(
                &self.0,
                OsStr::new("."),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
                0,
            )?;
            if unsafe {
                libc::renameat(
                    self.0.as_raw_fd(),
                    source.as_ptr(),
                    self.0.as_raw_fd(),
                    destination.as_ptr(),
                )
            } == 0
            {
                if let Err(error) = directory.sync_all() {
                    eprintln!("[yoake] renamed storage file, but directory sync failed: {error}");
                }
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }

        pub fn remove(&self, value: &OsStr) -> io::Result<()> {
            check_name(value)?;
            let value = name(value)?;
            if unsafe { libc::unlinkat(self.0.as_raw_fd(), value.as_ptr(), 0) } == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::fs::{self, OpenOptions};
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::path::PathBuf;

    const SHARE_READ_WRITE: u32 = 0x0000_0003;
    const BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const REPARSE_POINT: u32 = 0x0000_0400;

    #[derive(Debug)]
    pub struct Dir {
        path: PathBuf,
        handles: Vec<File>,
    }

    fn check(file: &File, directory: bool) -> io::Result<()> {
        let metadata = file.metadata()?;
        if metadata.file_attributes() & REPARSE_POINT != 0
            || if directory {
                !metadata.is_dir()
            } else {
                !metadata.is_file()
            }
        {
            Err(unsafe_entry())
        } else {
            Ok(())
        }
    }

    fn open_dir(path: &Path) -> io::Result<File> {
        let file = OpenOptions::new()
            .read(true)
            .share_mode(SHARE_READ_WRITE)
            .custom_flags(BACKUP_SEMANTICS | OPEN_REPARSE_POINT)
            .open(path)?;
        check(&file, true)?;
        Ok(file)
    }

    impl Dir {
        pub fn open(path: &Path, _create: bool) -> io::Result<Self> {
            if !path.is_absolute() {
                return Err(unsafe_entry());
            }
            Ok(Self {
                handles: vec![open_dir(path)?],
                path: path.to_owned(),
            })
        }

        pub fn child(&self, value: &OsStr, create: bool) -> io::Result<Self> {
            check_name(value)?;
            let path = self.path.join(value);
            let file = match open_dir(&path) {
                Ok(file) => file,
                Err(error) if create && error.kind() == io::ErrorKind::NotFound => {
                    match fs::create_dir(&path) {
                        Ok(()) => {}
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                        Err(error) => return Err(error),
                    }
                    open_dir(&path)?
                }
                Err(error) => return Err(error),
            };
            let mut handles = self
                .handles
                .iter()
                .map(File::try_clone)
                .collect::<io::Result<Vec<_>>>()?;
            handles.push(file);
            Ok(Self { path, handles })
        }

        pub fn read(&self, value: &OsStr) -> io::Result<Option<File>> {
            check_name(value)?;
            match OpenOptions::new()
                .read(true)
                .share_mode(SHARE_READ_WRITE)
                .custom_flags(OPEN_REPARSE_POINT)
                .open(self.path.join(value))
            {
                Ok(file) => {
                    check(&file, false)?;
                    Ok(Some(file))
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
            }
        }

        pub fn create(&self, value: &OsStr) -> io::Result<File> {
            check_name(value)?;
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .share_mode(SHARE_READ_WRITE)
                .custom_flags(OPEN_REPARSE_POINT)
                .open(self.path.join(value))?;
            check(&file, false)?;
            Ok(file)
        }

        pub fn lock(&self, value: &OsStr) -> io::Result<File> {
            check_name(value)?;
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .share_mode(SHARE_READ_WRITE)
                .custom_flags(OPEN_REPARSE_POINT)
                .open(self.path.join(value))?;
            check(&file, false)?;
            file.try_lock().map_err(io::Error::from)?;
            Ok(file)
        }

        pub fn entries(&self) -> io::Result<Vec<OsString>> {
            fs::read_dir(&self.path)?
                .map(|entry| entry.map(|entry| entry.file_name()))
                .collect()
        }

        pub fn replace(&self, source: &OsStr, destination: &OsStr) -> io::Result<()> {
            check_name(source)?;
            check_name(destination)?;
            drop(self.read(destination)?);
            fs::rename(self.path.join(source), self.path.join(destination))
        }

        pub fn remove(&self, value: &OsStr) -> io::Result<()> {
            check_name(value)?;
            fs::remove_file(self.path.join(value))
        }
    }
}

#[cfg(any(unix, windows))]
pub use platform::Dir;
