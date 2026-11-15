use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::path::{Component, Path};

fn unsafe_entry() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "unsafe storage filesystem entry")
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

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
        pub fn open(path: &Path, create: bool) -> io::Result<Self> {
            if !path.is_absolute() {
                return Err(unsafe_entry());
            }
            let mut directory = Self(File::open("/")?);
            for component in path.components() {
                match component {
                    Component::RootDir => {}
                    Component::Normal(value) => directory = directory.child(value, create)?,
                    _ => return Err(unsafe_entry()),
                }
            }
            Ok(directory)
        }

        pub fn child(&self, value: &OsStr, create: bool) -> io::Result<Self> {
            let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW;
            match open_at(&self.0, value, flags, 0) {
                Ok(file) => Ok(Self(file)),
                Err(error) if create && error.kind() == io::ErrorKind::NotFound => {
                    let value = name(value)?;
                    let result = unsafe { libc::mkdirat(self.0.as_raw_fd(), value.as_ptr(), 0o700) };
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
            match open_at(&self.0, value, libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK, 0) {
                Ok(file) if file.metadata()?.is_file() => Ok(Some(file)),
                Ok(_) => Err(unsafe_entry()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
            }
        }

        pub fn create(&self, value: &OsStr) -> io::Result<File> {
            open_at(&self.0, value, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW, 0o600)
        }

        pub fn replace(&self, source: &OsStr, destination: &OsStr) -> io::Result<()> {
            let _existing = self.read(destination)?;
            let source = name(source)?;
            let destination = name(destination)?;
            if unsafe { libc::renameat(self.0.as_raw_fd(), source.as_ptr(), self.0.as_raw_fd(), destination.as_ptr()) } == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }

        pub fn remove(&self, value: &OsStr) -> io::Result<()> {
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

    pub struct Dir {
        path: PathBuf,
        handles: Vec<File>,
    }

    fn check(file: &File, directory: bool) -> io::Result<()> {
        let metadata = file.metadata()?;
        if metadata.file_attributes() & REPARSE_POINT != 0
            || if directory { !metadata.is_dir() } else { !metadata.is_file() }
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
        pub fn open(path: &Path, create: bool) -> io::Result<Self> {
            if !path.is_absolute() {
                return Err(unsafe_entry());
            }
            let mut components = path.components();
            let Some(Component::Prefix(prefix)) = components.next() else { return Err(unsafe_entry()) };
            if !matches!(components.next(), Some(Component::RootDir)) {
                return Err(unsafe_entry());
            }
            let mut root = PathBuf::from(prefix.as_os_str());
            root.push(std::path::MAIN_SEPARATOR.to_string());
            let mut directory = Self { handles: vec![open_dir(&root)?], path: root };
            for component in components {
                match component {
                    Component::Normal(value) => directory = directory.child(value, create)?,
                    _ => return Err(unsafe_entry()),
                }
            }
            Ok(directory)
        }

        pub fn child(&self, value: &OsStr, create: bool) -> io::Result<Self> {
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
            let mut handles = self.handles.iter().map(File::try_clone).collect::<io::Result<Vec<_>>>()?;
            handles.push(file);
            Ok(Self { path, handles })
        }

        pub fn read(&self, value: &OsStr) -> io::Result<Option<File>> {
            match OpenOptions::new().read(true).share_mode(SHARE_READ_WRITE)
                .custom_flags(OPEN_REPARSE_POINT).open(self.path.join(value))
            {
                Ok(file) => { check(&file, false)?; Ok(Some(file)) }
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
            }
        }

        pub fn create(&self, value: &OsStr) -> io::Result<File> {
            let file = OpenOptions::new().write(true).create_new(true).share_mode(SHARE_READ_WRITE)
                .custom_flags(OPEN_REPARSE_POINT).open(self.path.join(value))?;
            check(&file, false)?;
            Ok(file)
        }

        pub fn replace(&self, source: &OsStr, destination: &OsStr) -> io::Result<()> {
            drop(self.read(destination)?);
            fs::rename(self.path.join(source), self.path.join(destination))
        }

        pub fn remove(&self, value: &OsStr) -> io::Result<()> {
            fs::remove_file(self.path.join(value))
        }
    }
}

#[cfg(any(unix, windows))]
pub(super) use platform::Dir;
