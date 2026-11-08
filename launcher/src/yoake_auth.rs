use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const MAX_TOKEN_BYTES: u64 = 8192;
static CREDENTIAL_LOCK: Mutex<()> = Mutex::new(());
static CREDENTIAL_EPOCH: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthError {
    Unreadable,
    UnsafePermissions,
    InvalidToken,
}

pub trait YoakeAuth: Send + Sync {
    fn authorization(&self) -> Result<Option<String>, AuthError>;
}

pub struct TokenFileAuth {
    path: PathBuf,
}

impl TokenFileAuth {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join("yoake-token"),
        }
    }
}

impl YoakeAuth for TokenFileAuth {
    fn authorization(&self) -> Result<Option<String>, AuthError> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x0020_0000);
        }
        let file = match options.open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(AuthError::Unreadable),
        };
        check_private_file(&file, &self.path)?;
        let mut bytes = Vec::new();
        file.take(MAX_TOKEN_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| AuthError::Unreadable)?;
        if bytes.len() as u64 > MAX_TOKEN_BYTES {
            return Err(AuthError::InvalidToken);
        }
        let token = std::str::from_utf8(&bytes)
            .map_err(|_| AuthError::InvalidToken)?
            .trim_end_matches(['\r', '\n']);
        if !valid_token(token) {
            return Err(AuthError::InvalidToken);
        }
        Ok(Some(format!("Bearer {token}")))
    }
}

fn valid_token(token: &str) -> bool {
    let (body, padding) = match token.find('=') {
        Some(index) => (&token[..index], &token[index..]),
        None => (token, ""),
    };
    token.len() as u64 <= MAX_TOKEN_BYTES
        && !body.is_empty()
        && body
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._~+/".contains(&byte))
        && padding.bytes().all(|byte| byte == b'=')
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct YoakeAccount {
    pub kind: String,
    pub expires_at: serde_json::Value,
}

pub fn save_credentials(
    data_dir: &Path,
    token: &str,
    account: &YoakeAccount,
) -> Result<(), AuthError> {
    let _guard = CREDENTIAL_LOCK.lock().map_err(|_| AuthError::Unreadable)?;
    save_credentials_locked(data_dir, token, account)
}

pub(crate) fn credential_epoch() -> u64 {
    CREDENTIAL_EPOCH.load(Ordering::Acquire)
}

pub(crate) fn save_credentials_if_current(
    data_dir: &Path,
    token: &str,
    account: &YoakeAccount,
    epoch: u64,
    cancelled: &AtomicBool,
) -> Result<bool, AuthError> {
    let _guard = CREDENTIAL_LOCK.lock().map_err(|_| AuthError::Unreadable)?;
    if epoch != credential_epoch() || cancelled.load(Ordering::Acquire) {
        return Ok(false);
    }
    save_credentials_locked(data_dir, token, account)?;
    if cancelled.load(Ordering::Acquire) {
        remove_credentials(data_dir)?;
        return Ok(false);
    }
    Ok(true)
}

fn save_credentials_locked(
    data_dir: &Path,
    token: &str,
    account: &YoakeAccount,
) -> Result<(), AuthError> {
    let valid_expiry = match &account.expires_at {
        serde_json::Value::String(value) => {
            !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
        }
        serde_json::Value::Number(_) | serde_json::Value::Null => true,
        _ => false,
    };
    if !valid_token(token)
        || account.kind.is_empty()
        || account.kind.len() > 80
        || account.kind.chars().any(char::is_control)
        || account.kind == token
        || account.expires_at.as_str() == Some(token)
        || !valid_expiry
    {
        return Err(AuthError::InvalidToken);
    }
    std::fs::create_dir_all(data_dir).map_err(|_| AuthError::Unreadable)?;
    let account_bytes = serde_json::to_vec(account).map_err(|_| AuthError::Unreadable)?;
    write_private(data_dir, "yoake-account.json", &account_bytes)?;
    if let Err(error) = write_private(data_dir, "yoake-token", token.as_bytes()) {
        let _ = remove_credentials(data_dir);
        return Err(error);
    }
    Ok(())
}

pub fn logout(data_dir: &Path) -> Result<(), AuthError> {
    let _guard = CREDENTIAL_LOCK.lock().map_err(|_| AuthError::Unreadable)?;
    CREDENTIAL_EPOCH.fetch_add(1, Ordering::AcqRel);
    remove_credentials(data_dir)
}

fn remove_credentials(data_dir: &Path) -> Result<(), AuthError> {
    let mut failed = false;
    for name in ["yoake-token", "yoake-account.json"] {
        if let Err(error) = std::fs::remove_file(data_dir.join(name)) {
            failed |= error.kind() != io::ErrorKind::NotFound;
        }
    }
    if failed {
        Err(AuthError::Unreadable)
    } else {
        Ok(())
    }
}

fn write_private(data_dir: &Path, name: &str, bytes: &[u8]) -> Result<(), AuthError> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_| AuthError::Unreadable)?;
    let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let temporary = data_dir.join(format!(".{name}.{suffix}.tmp"));
    let result = (|| {
        let mut file = create_private_file(&temporary).map_err(|_| AuthError::Unreadable)?;
        check_private_file(&file, &temporary)?;
        file.write_all(bytes).map_err(|_| AuthError::Unreadable)?;
        file.sync_all().map_err(|_| AuthError::Unreadable)?;
        drop(file);
        std::fs::rename(&temporary, data_dir.join(name)).map_err(|_| AuthError::Unreadable)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

#[cfg(unix)]
fn create_private_file(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(windows)]
fn create_private_file(path: &Path) -> io::Result<File> {
    windows_acl::create_private_file(path)
}

#[cfg(not(any(unix, windows)))]
fn create_private_file(_path: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "private files unsupported",
    ))
}

fn check_private_file(file: &File, path: &Path) -> Result<(), AuthError> {
    let path_metadata = std::fs::symlink_metadata(path).map_err(|_| AuthError::Unreadable)?;
    let metadata = file.metadata().map_err(|_| AuthError::Unreadable)?;
    if path_metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AuthError::UnsafePermissions);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o077 != 0
            || metadata.dev() != path_metadata.dev()
            || metadata.ino() != path_metadata.ino()
        {
            return Err(AuthError::UnsafePermissions);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 || !windows_acl::is_user_only(file) {
            return Err(AuthError::UnsafePermissions);
        }
    }
    #[cfg(not(any(unix, windows)))]
    return Err(AuthError::UnsafePermissions);
    #[cfg(any(unix, windows))]
    Ok(())
}

#[cfg(windows)]
mod windows_acl {
    use std::ffi::c_void;
    use std::fs::File;
    use std::io;
    use std::mem::size_of;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use std::path::Path;
    use std::ptr::null_mut;

    type Handle = *mut c_void;

    #[repr(C)]
    struct SidAndAttributes {
        sid: *mut c_void,
        attributes: u32,
    }

    #[repr(C)]
    struct AclSizeInformation {
        ace_count: u32,
        bytes_in_use: u32,
        bytes_free: u32,
    }

    #[repr(C)]
    struct AceHeader {
        ace_type: u8,
        flags: u8,
        size: u16,
    }

    #[repr(C)]
    struct AllowedAce {
        header: AceHeader,
        mask: u32,
        sid_start: u32,
    }

    #[repr(C)]
    struct SecurityAttributes {
        length: u32,
        descriptor: *mut c_void,
        inherit_handle: i32,
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
        fn GetTokenInformation(
            token: Handle,
            class: u32,
            information: *mut c_void,
            length: u32,
            returned: *mut u32,
        ) -> i32;
        fn GetSecurityInfo(
            handle: Handle,
            object_type: u32,
            information: u32,
            owner: *mut *mut c_void,
            group: *mut *mut c_void,
            dacl: *mut *mut c_void,
            sacl: *mut *mut c_void,
            descriptor: *mut *mut c_void,
        ) -> u32;
        fn GetAclInformation(acl: *mut c_void, info: *mut c_void, size: u32, class: u32) -> i32;
        fn GetAce(acl: *mut c_void, index: u32, ace: *mut *mut c_void) -> i32;
        fn EqualSid(first: *const c_void, second: *const c_void) -> i32;
        fn ConvertSidToStringSidW(sid: *const c_void, string: *mut *mut u16) -> i32;
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            string: *const u16,
            revision: u32,
            descriptor: *mut *mut c_void,
            size: *mut u32,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> Handle;
        fn CloseHandle(handle: Handle) -> i32;
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *mut SecurityAttributes,
            creation: u32,
            flags: u32,
            template: Handle,
        ) -> Handle;
    }

    pub fn create_private_file(path: &Path) -> io::Result<File> {
        unsafe {
            let mut token = null_mut();
            if OpenProcessToken(GetCurrentProcess(), 0x0008, &mut token) == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut length = 0;
            GetTokenInformation(token, 1, null_mut(), 0, &mut length);
            if length == 0 || length > 65536 {
                CloseHandle(token);
                return Err(io::Error::other("cannot identify current user"));
            }
            let mut token_data = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
            let result = GetTokenInformation(
                token,
                1,
                token_data.as_mut_ptr().cast(),
                length,
                &mut length,
            );
            CloseHandle(token);
            if result == 0 {
                return Err(io::Error::last_os_error());
            }
            let user = (*(token_data.as_ptr().cast::<SidAndAttributes>())).sid;
            let mut sid_string = null_mut();
            if user.is_null() || ConvertSidToStringSidW(user, &mut sid_string) == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut sid_length = 0;
            while *sid_string.add(sid_length) != 0 {
                sid_length += 1;
            }
            let sid = String::from_utf16_lossy(std::slice::from_raw_parts(sid_string, sid_length));
            LocalFree(sid_string.cast());
            let sddl: Vec<u16> = format!("O:{sid}D:P(A;;FA;;;{sid})")
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let mut descriptor = null_mut();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                null_mut(),
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let mut security = SecurityAttributes {
                length: size_of::<SecurityAttributes>() as u32,
                descriptor,
                inherit_handle: 0,
            };
            let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let handle = CreateFileW(
                wide_path.as_ptr(),
                0xc000_0000,
                0x0000_0007,
                &mut security,
                1,
                0x0000_0080,
                null_mut(),
            );
            let error = io::Error::last_os_error();
            LocalFree(descriptor);
            if handle as isize == -1 {
                Err(error)
            } else {
                Ok(File::from_raw_handle(handle.cast()))
            }
        }
    }

    // CHECK: Exercise this fail-closed ACL check with real Windows token files and
    // portal-created ACLs.
    pub fn is_user_only(file: &File) -> bool {
        unsafe {
            let mut token = null_mut();
            if OpenProcessToken(GetCurrentProcess(), 0x0008, &mut token) == 0 {
                return false;
            }
            let mut length = 0;
            GetTokenInformation(token, 1, null_mut(), 0, &mut length);
            if length == 0 || length > 65536 {
                CloseHandle(token);
                return false;
            }
            let mut token_data = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
            let result = GetTokenInformation(
                token,
                1,
                token_data.as_mut_ptr().cast(),
                length,
                &mut length,
            );
            CloseHandle(token);
            if result == 0 {
                return false;
            }
            let user = (*(token_data.as_ptr().cast::<SidAndAttributes>())).sid;
            let mut owner = null_mut();
            let mut dacl = null_mut();
            let mut descriptor = null_mut();
            let status = GetSecurityInfo(
                file.as_raw_handle().cast(),
                1,
                0x0000_0005,
                &mut owner,
                null_mut(),
                &mut dacl,
                null_mut(),
                &mut descriptor,
            );
            if status != 0 || descriptor.is_null() {
                if !descriptor.is_null() {
                    LocalFree(descriptor);
                }
                return false;
            }
            let mut safe = !owner.is_null()
                && !dacl.is_null()
                && !user.is_null()
                && EqualSid(owner, user) != 0;
            let mut info = AclSizeInformation {
                ace_count: 0,
                bytes_in_use: 0,
                bytes_free: 0,
            };
            if safe
                && GetAclInformation(
                    dacl,
                    (&mut info as *mut AclSizeInformation).cast(),
                    size_of::<AclSizeInformation>() as u32,
                    2,
                ) == 0
            {
                safe = false;
            }
            for index in 0..info.ace_count {
                let mut ace = null_mut();
                if GetAce(dacl, index, &mut ace) == 0 || ace.is_null() {
                    safe = false;
                    break;
                }
                let header = &*(ace.cast::<AceHeader>());
                match header.ace_type {
                    0 if header.size as usize >= size_of::<AllowedAce>() => {
                        let allowed = &*(ace.cast::<AllowedAce>());
                        let sid = (&allowed.sid_start as *const u32).cast::<c_void>();
                        if EqualSid(sid, user) == 0 {
                            safe = false;
                            break;
                        }
                    }
                    1 => {}
                    _ => {
                        safe = false;
                        break;
                    }
                }
            }
            LocalFree(descriptor);
            safe
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_token_has_no_authorization_header() {
        let path = std::env::temp_dir().join(format!("yoake-missing-{}", std::process::id()));
        assert_eq!(TokenFileAuth::new(&path).authorization(), Ok(None));
    }

    #[cfg(unix)]
    #[test]
    fn token_must_be_private_and_valid() {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        use std::time::{SystemTime, UNIX_EPOCH};

        let directory = std::env::temp_dir().join(format!(
            "yoake-auth-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("yoake-token");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        file.write_all(b"test-token\n").unwrap();
        drop(file);
        let auth = TokenFileAuth::new(&directory);
        assert_eq!(
            auth.authorization(),
            Ok(Some("Bearer test-token".to_owned()))
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(auth.authorization(), Err(AuthError::UnsafePermissions));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&path, b"invalid token").unwrap();
        assert_eq!(auth.authorization(), Err(AuthError::InvalidToken));
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
