use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::path::{Component, Path};

fn unsafe_entry() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "unsafe filesystem entry")
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
        let fd = unsafe { libc::openat(dir.as_raw_fd(), value.as_ptr(), flags, mode) };
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }

    impl Dir {
        pub fn open(path: &Path, create: bool) -> io::Result<Self> {
            if !path.is_absolute() {
                return Err(unsafe_entry());
            }
            let mut dir = Self(File::open("/")?);
            for component in path.components() {
                match component {
                    Component::RootDir => {}
                    Component::Normal(value) => dir = dir.child(value, create)?,
                    _ => return Err(unsafe_entry()),
                }
            }
            Ok(dir)
        }

        pub fn child(&self, value: &OsStr, create: bool) -> io::Result<Self> {
            let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW;
            match open_at(&self.0, value, flags, 0) {
                Ok(file) => Ok(Self(file)),
                Err(error) if create && error.kind() == io::ErrorKind::NotFound => {
                    let value_c = name(value)?;
                    let result =
                        unsafe { libc::mkdirat(self.0.as_raw_fd(), value_c.as_ptr(), 0o700) };
                    if result != 0 {
                        let error = io::Error::last_os_error();
                        if error.kind() != io::ErrorKind::AlreadyExists {
                            return Err(error);
                        }
                    }
                    Ok(Self(open_at(&self.0, value, flags, 0)?))
                }
                Err(error) => Err(error),
            }
        }

        pub fn read(&self, value: &OsStr) -> io::Result<Option<File>> {
            let flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK;
            match open_at(&self.0, value, flags, 0) {
                Ok(file) if file.metadata()?.is_file() => Ok(Some(file)),
                Ok(_) => Err(unsafe_entry()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
            }
        }

        pub fn create(&self, value: &OsStr) -> io::Result<File> {
            open_at(
                &self.0,
                value,
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                0o600,
            )
        }

        pub fn lock_file(&self, value: &OsStr) -> io::Result<File> {
            let file = open_at(
                &self.0,
                value,
                libc::O_RDWR | libc::O_CREAT | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                0o600,
            )?;
            if !file.metadata()?.is_file() {
                return Err(unsafe_entry());
            }
            Ok(file)
        }

        pub fn remove(&self, value: &OsStr) -> io::Result<()> {
            if self.read(value)?.is_none() {
                return Ok(());
            }
            let value = name(value)?;
            if unsafe { libc::unlinkat(self.0.as_raw_fd(), value.as_ptr(), 0) } == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }

        pub fn link_new(&self, from: &OsStr, to_dir: &Self, to: &OsStr) -> io::Result<()> {
            let source = self.read(from)?.ok_or(io::ErrorKind::NotFound)?;
            let from_name = name(from)?;
            let to_name = name(to)?;
            if unsafe {
                libc::linkat(
                    self.0.as_raw_fd(),
                    from_name.as_ptr(),
                    to_dir.0.as_raw_fd(),
                    to_name.as_ptr(),
                    0,
                )
            } != 0
            {
                return Err(io::Error::last_os_error());
            }
            let matches = to_dir
                .read(to)
                .and_then(|linked| linked.ok_or_else(unsafe_entry))
                .and_then(|linked| same_file(&source, &linked));
            if matches!(matches, Ok(true)) {
                return Ok(());
            }
            unsafe { libc::unlinkat(to_dir.0.as_raw_fd(), to_name.as_ptr(), 0) };
            Err(unsafe_entry())
        }

        pub fn rename_new(&self, from: &OsStr, to: &OsStr) -> io::Result<()> {
            #[cfg(any(target_os = "linux", target_os = "android"))]
            {
                let _source = self.read(from)?.ok_or(io::ErrorKind::NotFound)?;
                let from = name(from)?;
                let to = name(to)?;
                if unsafe {
                    libc::renameat2(
                        self.0.as_raw_fd(),
                        from.as_ptr(),
                        self.0.as_raw_fd(),
                        to.as_ptr(),
                        libc::RENAME_NOREPLACE,
                    )
                } == 0
                {
                    Ok(())
                } else {
                    Err(io::Error::last_os_error())
                }
            }
            #[cfg(not(any(target_os = "linux", target_os = "android")))]
            {
                self.link_new(from, self, to)?;
                self.remove(from)
            }
        }

        pub fn sync(&self) -> io::Result<()> {
            self.0.sync_all()
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::fs::{self, OpenOptions};
    use std::os::windows::ffi::OsStrExt;
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
        pub fn open(path: &Path, create: bool) -> io::Result<Self> {
            if !path.is_absolute() {
                return Err(unsafe_entry());
            }
            let mut components = path.components();
            let Some(Component::Prefix(prefix)) = components.next() else {
                return Err(unsafe_entry());
            };
            if !matches!(components.next(), Some(Component::RootDir)) {
                return Err(unsafe_entry());
            }
            let mut root = PathBuf::from(prefix.as_os_str());
            root.push("\\");
            let mut dir = Self {
                handles: vec![open_dir(&root)?],
                path: root,
            };
            for component in components {
                match component {
                    Component::Normal(value) => dir = dir.child(value, create)?,
                    _ => return Err(unsafe_entry()),
                }
            }
            Ok(dir)
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
            let mut handles = self
                .handles
                .iter()
                .map(File::try_clone)
                .collect::<io::Result<Vec<_>>>()?;
            handles.push(file);
            Ok(Self { path, handles })
        }

        pub fn read(&self, value: &OsStr) -> io::Result<Option<File>> {
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
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .share_mode(SHARE_READ_WRITE)
                .custom_flags(OPEN_REPARSE_POINT)
                .open(self.path.join(value))
        }

        pub fn lock_file(&self, value: &OsStr) -> io::Result<File> {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .share_mode(SHARE_READ_WRITE)
                .custom_flags(OPEN_REPARSE_POINT)
                .open(self.path.join(value))?;
            check(&file, false)?;
            Ok(file)
        }

        pub fn remove(&self, value: &OsStr) -> io::Result<()> {
            if self.read(value)?.is_none() {
                return Ok(());
            }
            fs::remove_file(self.path.join(value))
        }

        pub fn link_new(&self, from: &OsStr, to_dir: &Self, to: &OsStr) -> io::Result<()> {
            let _source = self.read(from)?.ok_or(io::ErrorKind::NotFound)?;
            fs::hard_link(self.path.join(from), to_dir.path.join(to))
        }

        pub fn rename_new(&self, from: &OsStr, to: &OsStr) -> io::Result<()> {
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn MoveFileW(existing: *const u16, new: *const u16) -> i32;
            }

            drop(self.read(from)?.ok_or(io::ErrorKind::NotFound)?);
            let from: Vec<u16> = self
                .path
                .join(from)
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect();
            let to: Vec<u16> = self
                .path
                .join(to)
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect();
            if unsafe { MoveFileW(from.as_ptr(), to.as_ptr()) } == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }

        pub fn sync(&self) -> io::Result<()> {
            Ok(())
        }
    }
}

pub use platform::Dir;

#[cfg(unix)]
pub fn same_file(first: &File, second: &File) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let first = first.metadata()?;
    let second = second.metadata()?;
    Ok(first.dev() == second.dev() && first.ino() == second.ino())
}

#[cfg(windows)]
pub fn same_file(first: &File, second: &File) -> io::Result<bool> {
    use std::os::windows::io::AsRawHandle;
    #[repr(C)]
    struct Information {
        attributes: u32,
        created: [u32; 2],
        accessed: [u32; 2],
        modified: [u32; 2],
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, info: *mut Information)
        -> i32;
    }
    fn identity(file: &File) -> io::Result<(u32, u32, u32)> {
        let mut info = std::mem::MaybeUninit::<Information>::uninit();
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let info = unsafe { info.assume_init() };
        Ok((info.volume, info.index_high, info.index_low))
    }
    Ok(identity(first)? == identity(second)?)
}
