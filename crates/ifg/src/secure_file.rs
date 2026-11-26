//! Owner-only secret files (design doc §11): the storage behind collaboration
//! session files and agent token files.
//!
//! Unix creates the file with mode `0600`. Windows has no POSIX mode bits, so
//! the equivalent is a *protected* DACL that grants the file's owner full
//! control and nobody else, applied atomically at creation (no inherited ACEs,
//! no window where the file is briefly world-readable). Loading verifies the
//! same property on the already-open handle, so a swapped or loosened file is
//! refused before its secret is read.
//!
//! The Win32 calls are this workspace's only `unsafe` exception. `unsafe_code`
//! is `deny` everywhere (see the workspace `Cargo.toml`); the `allow` below is
//! scoped to the Windows implementation module, every `unsafe` block carries a
//! safety note, and the Unix path uses no `unsafe` at all.

#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use std::io;
#[cfg(unix)]
use std::path::Path;

/// Create `path` as a new owner-only file. Fails if it already exists, so a
/// pre-planted file is never overwritten or truncated.
#[cfg(unix)]
pub fn create_private(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

/// Refuse a file that group or others can access (the `0600` guarantee).
#[cfg(unix)]
pub fn assert_private_open_file(file: &File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if file.metadata()?.permissions().mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "file is accessible by group or others",
        ));
    }
    Ok(())
}

#[cfg(windows)]
pub use win::{assert_private_open_file, create_private};

#[cfg(windows)]
#[allow(unsafe_code)] // audited Win32 ACL exception; see the module docs above
mod win {
    use std::ffi::c_void;
    use std::fs::File;
    use std::io;
    use std::iter::once;
    use std::mem::size_of;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use std::path::Path;
    use std::ptr::null_mut;

    use windows_sys::core::PWSTR;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, LocalFree, ERROR_SUCCESS, GENERIC_READ, GENERIC_WRITE, HANDLE,
        INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        GetSecurityInfo, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        AclSizeInformation, EqualSid, GetAce, GetAclInformation, GetSecurityDescriptorControl,
        GetTokenInformation, TokenUser, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION,
        DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
        SECURITY_ATTRIBUTES, SE_DACL_PRESENT, SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// Win32 `ACCESS_ALLOWED_ACE_TYPE`: the ACE header type of an allowed ACE.
    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;

    fn last_error() -> io::Error {
        // SAFETY: GetLastError reads this thread's last error; no preconditions.
        io::Error::from_raw_os_error(unsafe { GetLastError() } as i32)
    }

    fn not_private(reason: &str) -> io::Error {
        io::Error::new(io::ErrorKind::PermissionDenied, reason)
    }

    /// Read a NUL-terminated wide string into an owned `String`.
    ///
    /// SAFETY: `ptr` must point to a NUL-terminated wide string that stays
    /// valid for the call (the caller owns it until `LocalFree`).
    unsafe fn read_wide(ptr: PWSTR) -> String {
        let mut len = 0usize;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
    }

    /// The current process token's user SID, as an SDDL string (`S-1-5-…`).
    fn current_user_sid() -> io::Result<String> {
        let mut token: HANDLE = null_mut();
        // SAFETY: GetCurrentProcess is a pseudo-handle valid for this call;
        // OpenProcessToken writes a real handle into `token`, closed below.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(last_error());
        }
        let mut len: u32 = 0;
        // SAFETY: a null buffer with length 0 asks GetTokenInformation for the
        // required size; it fails with ERROR_INSUFFICIENT_BUFFER and sets len.
        unsafe { GetTokenInformation(token, TokenUser, null_mut(), 0, &mut len) };
        if len == 0 {
            // SAFETY: token is a real handle from OpenProcessToken.
            unsafe { CloseHandle(token) };
            return Err(io::Error::other("could not size the token user buffer"));
        }
        // A usize-aligned buffer so the TOKEN_USER (which holds a pointer) is
        // correctly aligned; the SID bytes follow the struct inside the buffer.
        let words = (len as usize).div_ceil(size_of::<usize>());
        let mut buf = vec![0usize; words];
        // SAFETY: buf is at least `len` bytes and pointer-aligned, matching the
        // size GetTokenInformation reported for the TokenUser class.
        let ok = unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buf.as_mut_ptr().cast::<c_void>(),
                len,
                &mut len,
            )
        };
        // SAFETY: token is a real handle from OpenProcessToken.
        unsafe { CloseHandle(token) };
        if ok == 0 {
            return Err(last_error());
        }
        // SAFETY: GetTokenInformation wrote a TOKEN_USER at the aligned start of
        // buf; its User.Sid points to SID bytes inside the same live buffer.
        let user = unsafe { &*(buf.as_ptr().cast::<TOKEN_USER>()) };
        let mut sid_str: PWSTR = null_mut();
        // SAFETY: user.User.Sid is a valid SID; ConvertSidToStringSidW allocates
        // a wide string into sid_str that we read and free below.
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid_str) } == 0 {
            return Err(last_error());
        }
        // SAFETY: sid_str is the NUL-terminated string just allocated.
        let sid = unsafe { read_wide(sid_str) };
        // SAFETY: sid_str was allocated by ConvertSidToStringSidW (LocalAlloc).
        unsafe { LocalFree(sid_str.cast()) };
        Ok(sid)
    }

    pub fn create_private(path: &Path) -> io::Result<File> {
        let sid = current_user_sid()?;
        // O: sets the owner to the token user; D:P is a protected DACL (no
        // inheritance) with a single allowed ACE granting that user (GA) full
        // control. No other principal appears, so only the owner can access it.
        let sddl = format!("O:{sid}D:P(A;;GA;;;{sid})");
        let sddl_w: Vec<u16> = sddl.encode_utf16().chain(once(0)).collect();
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: sddl_w is a NUL-terminated wide SDDL string; the API allocates
        // a self-relative security descriptor into sd, freed below.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl_w.as_ptr(),
                1, // SDDL_REVISION_1
                &mut sd,
                null_mut(),
            )
        } == 0
        {
            return Err(last_error());
        }
        let sa = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd,
            bInheritHandle: 0,
        };
        let path_w: Vec<u16> = path.as_os_str().encode_wide().chain(once(0)).collect();
        // SAFETY: path_w is NUL-terminated; sa points to the live sd; share mode
        // 0 denies concurrent opens; CREATE_NEW fails if the file exists. The
        // returned handle is owned by us (closed via File or on error).
        let handle = unsafe {
            CreateFileW(
                path_w.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                &sa,
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            )
        };
        // Capture CreateFileW's error before LocalFree: the free call may
        // overwrite the thread's last-error code with its own result.
        let create_error = (handle == INVALID_HANDLE_VALUE).then(last_error);
        // SAFETY: sd was allocated by ConvertStringSecurityDescriptor… above.
        unsafe { LocalFree(sd) };
        if let Some(error) = create_error {
            return Err(error);
        }
        // SAFETY: handle is a fresh, exclusively-owned file handle; transferring
        // it to File means File will close it exactly once.
        Ok(unsafe { File::from_raw_handle(handle) })
    }

    pub fn assert_private_open_file(file: &File) -> io::Result<()> {
        let handle = file.as_raw_handle();
        let mut owner: PSID = null_mut();
        let mut dacl: *mut ACL = null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: handle is a live open file handle; GetSecurityInfo allocates a
        // security descriptor into sd and points owner/dacl into it, freed below.
        let err = unsafe {
            GetSecurityInfo(
                handle,
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                null_mut(),
                &mut dacl,
                null_mut(),
                &mut sd,
            )
        };
        if err != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(err as i32));
        }
        let mut control: u16 = 0;
        let mut revision: u32 = 0;
        // SAFETY: sd is the live descriptor from GetSecurityInfo.
        let ok = unsafe { GetSecurityDescriptorControl(sd, &mut control, &mut revision) };
        let result = if ok == 0 {
            Err(last_error())
        } else if control & SE_DACL_PRESENT == 0 || control & SE_DACL_PROTECTED == 0 {
            Err(not_private("file DACL is absent or inherits permissions"))
        } else if dacl.is_null() || owner.is_null() {
            Err(not_private("file has no DACL or owner"))
        } else {
            // SAFETY: dacl and owner point into the live sd until LocalFree.
            unsafe { dacl_owner_only(dacl, owner) }
        };
        // SAFETY: sd was allocated by GetSecurityInfo.
        unsafe { LocalFree(sd) };
        result
    }

    /// Every ACE must be an allowed ACE for the owner SID and no one else.
    ///
    /// SAFETY: `dacl` is a live DACL and `owner` a live SID, both valid for the
    /// call (they point into a security descriptor the caller frees afterwards).
    unsafe fn dacl_owner_only(dacl: *mut ACL, owner: PSID) -> io::Result<()> {
        let mut info = ACL_SIZE_INFORMATION {
            AceCount: 0,
            AclBytesInUse: 0,
            AclBytesFree: 0,
        };
        if GetAclInformation(
            dacl,
            (&mut info as *mut ACL_SIZE_INFORMATION).cast::<c_void>(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        ) == 0
        {
            return Err(last_error());
        }
        if info.AceCount == 0 {
            return Err(not_private("file DACL grants no access"));
        }
        for index in 0..info.AceCount {
            let mut ace_ptr: *mut c_void = null_mut();
            if GetAce(dacl, index, &mut ace_ptr) == 0 {
                return Err(last_error());
            }
            let header = &*(ace_ptr.cast::<ACE_HEADER>());
            if header.AceType != ACCESS_ALLOWED_ACE_TYPE {
                return Err(not_private("file DACL has a non-allowed ACE"));
            }
            let ace = &*(ace_ptr.cast::<ACCESS_ALLOWED_ACE>());
            // The SID begins at the SidStart field of the ACE.
            let ace_sid = (&ace.SidStart as *const u32) as *mut c_void;
            if EqualSid(ace_sid, owner) == 0 {
                return Err(not_private("file DACL grants access beyond the owner"));
            }
        }
        Ok(())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn created_file_is_private_and_create_new_refuses_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret.txt");
        let file = create_private(&path).unwrap();
        assert_private_open_file(&file).unwrap();
        drop(file);
        // CREATE_NEW must not overwrite a pre-planted file.
        assert!(create_private(&path).is_err());
    }

    #[test]
    fn a_plainly_created_file_is_not_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("loose.txt");
        std::fs::write(&path, b"secret").unwrap();
        let file = std::fs::File::open(&path).unwrap();
        // An inherited (non-protected) DACL must be refused.
        assert!(assert_private_open_file(&file).is_err());
    }
}
