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

#[repr(C)]
struct FileTime {
    low: u32,
    high: u32,
}

#[repr(C)]
struct ByHandleFileInformation {
    attributes: u32,
    creation: FileTime,
    access: FileTime,
    write: FileTime,
    volume_serial: u32,
    size_high: u32,
    size_low: u32,
    number_links: u32,
    index_high: u32,
    index_low: u32,
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
    fn GetFileInformationByHandle(handle: Handle, info: *mut ByHandleFileInformation) -> i32;
    fn SetFileInformationByHandle(
        handle: Handle,
        class: u32,
        info: *const c_void,
        size: u32,
    ) -> i32;
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

pub fn delete_open_file(file: &File) -> io::Result<()> {
    let delete_file: i32 = 1;
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle().cast(),
            4,
            (&delete_file as *const i32).cast(),
            size_of::<i32>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
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

pub fn is_user_only(file: &File) -> bool {
    unsafe {
        let mut info = std::mem::MaybeUninit::<ByHandleFileInformation>::uninit();
        if GetFileInformationByHandle(file.as_raw_handle().cast(), info.as_mut_ptr()) == 0
            || info.assume_init().number_links != 1
        {
            return false;
        }
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
        let mut safe =
            !owner.is_null() && !dacl.is_null() && !user.is_null() && EqualSid(owner, user) != 0;
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
