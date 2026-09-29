//! Replacing a small file whole, for Pane's own records such as
//! `installed.json` and `settings.json`.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Who may read a file Pane writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Readers {
    /// Whoever the folder and the process's defaults allow (the umask on
    /// Unix).
    Default,
    /// Only the user Pane runs as: on Unix the file is created with mode
    /// 0600, whatever the umask. On Windows it is created with a protected
    /// DACL, inheriting nothing from its folder, that gives full control to
    /// the user Pane runs as and to SYSTEM only
    /// (`D:P(A;;FA;;;SY)(A;;FA;;;<user's SID>)`).
    OwnerOnly,
}

/// Distinguishes the temporary files of one process's writes.
static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

/// Replaces the file at `path` with `contents`, readable by `readers`,
/// creating its folder if needed. The new file never has wider permissions
/// than `readers`, even for a moment, and it replaces any earlier file's
/// permissions.
///
/// The contents go to a new temporary file in the same folder, which is
/// flushed to disk and then renamed over `path`; on Unix the folder is
/// flushed too, so the rename itself survives a power loss. A crash or power
/// loss therefore leaves either the old file or the new one, never a torn
/// one. (Windows offers no way to flush a folder, so there a power loss just
/// after the rename can still leave the old file.)
///
/// Each write uses its own temporary name, including the process id, so
/// writers in two processes or threads never share one. The file is not
/// locked, though: when two writers each read, change and write it back, the
/// later rename wins and the other change is lost.
pub(crate) fn write_atomically(path: &Path, contents: &[u8], readers: Readers) -> io::Result<()> {
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other(format!("{} names no file", path.display())))?;
    let temporary = dir.join(format!(
        ".{}.{}-{}.tmp",
        name.to_string_lossy(),
        std::process::id(),
        NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed)
    ));
    let written = (|| {
        let mut file = create_new(&temporary, readers)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written?;
    sync_dir(dir)
}

/// Creates the file at `path`, which must not exist yet, for writing,
/// readable by `readers` from the moment it exists.
#[cfg(unix)]
fn create_new(path: &Path, readers: Readers) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    if readers == Readers::OwnerOnly {
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(windows)]
fn create_new(path: &Path, readers: Readers) -> io::Result<fs::File> {
    match readers {
        Readers::Default => OpenOptions::new().write(true).create_new(true).open(path),
        Readers::OwnerOnly => owner_only::create_new(path),
    }
}

#[cfg(not(any(unix, windows)))]
fn create_new(path: &Path, _readers: Readers) -> io::Result<fs::File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

/// Files only the user Pane runs as (and SYSTEM) can open, on Windows.
#[cfg(windows)]
mod owner_only {
    use std::fs::File;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use std::path::Path;

    use ::windows::Win32::Foundation::{CloseHandle, GENERIC_WRITE, HANDLE, HLOCAL, LocalFree};
    use ::windows::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use ::windows::Win32::Security::{
        GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
        TokenUser,
    };
    use ::windows::Win32::Storage::FileSystem::{
        CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_NONE,
    };
    use ::windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use ::windows::core::{PCWSTR, PWSTR};

    fn failed(error: ::windows::core::Error) -> io::Error {
        io::Error::from_raw_os_error(error.code().0 & 0xFFFF)
    }

    /// The SID of the user this process runs as, in its string form.
    pub(super) fn user_sid() -> io::Result<String> {
        let mut token = HANDLE::default();
        // SAFETY: the current process's pseudo handle; `token` is writable
        // and closed below.
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
            .map_err(failed)?;
        let sid = (|| {
            let mut length = 0u32;
            // SAFETY: asks for the size only; this call fails by design.
            let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut length) };
            // u64s, so the buffer is aligned for TOKEN_USER.
            let mut buffer = vec![0u64; (length as usize).div_ceil(8)];
            // SAFETY: `buffer` is writable for `length` bytes.
            unsafe {
                GetTokenInformation(
                    token,
                    TokenUser,
                    Some(buffer.as_mut_ptr().cast()),
                    length,
                    &mut length,
                )
            }
            .map_err(failed)?;
            // SAFETY: the buffer now holds a TOKEN_USER, whose SID points
            // into the same buffer.
            let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
            let mut text = PWSTR::null();
            // SAFETY: a valid SID; `text` is freed below.
            unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) }.map_err(failed)?;
            // SAFETY: a NUL-terminated string the call allocated.
            let sid = unsafe { text.to_string() };
            // SAFETY: allocated by ConvertSidToStringSidW with LocalAlloc.
            unsafe { LocalFree(Some(HLOCAL(text.0.cast()))) };
            sid.map_err(|_| io::Error::other("the user's SID is not valid UTF-16"))
        })();
        // SAFETY: opened above.
        let _ = unsafe { CloseHandle(token) };
        sid
    }

    /// Creates the file at `path`, which must not exist yet, for writing,
    /// with a protected DACL giving full control to this user and SYSTEM
    /// only.
    pub(super) fn create_new(path: &Path) -> io::Result<File> {
        let sddl = format!("D:P(A;;FA;;;SY)(A;;FA;;;{})", user_sid()?);
        let sddl: Vec<u16> = sddl.encode_utf16().chain([0]).collect();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: `sddl` is NUL-terminated; the descriptor is freed below.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }
        .map_err(failed)?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: false.into(),
        };
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: `wide` is NUL-terminated and `attributes` valid for the
        // call; the handle is owned by the returned file.
        let created = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                GENERIC_WRITE.0,
                FILE_SHARE_NONE,
                Some(&attributes),
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        };
        // SAFETY: allocated by the conversion above with LocalAlloc.
        unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
        let handle = created.map_err(failed)?;
        // SAFETY: a new, valid handle nothing else owns.
        Ok(unsafe { File::from_raw_handle(handle.0) })
    }
}

#[cfg(unix)]
fn sync_dir(dir: &Path) -> io::Result<()> {
    fs::File::open(dir)?.sync_all()
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Readers, write_atomically};
    use std::fs;

    #[test]
    fn writers_that_overlap_each_leave_a_whole_file_and_no_temporary_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("records").join("installed.json");
        let payloads: Vec<String> = (0..8)
            .map(|writer| {
                format!(
                    "{{\"writer\": {writer}, \"padding\": \"{}\"}}",
                    "x".repeat(4096)
                )
            })
            .collect();
        std::thread::scope(|scope| {
            for payload in &payloads {
                let path = &path;
                scope.spawn(move || {
                    for _ in 0..50 {
                        match write_atomically(path, payload.as_bytes(), Readers::Default) {
                            Ok(()) => {}
                            // Windows can refuse a rename onto a file that
                            // another rename is replacing at that moment; the
                            // write fails whole, which is still not torn.
                            Err(error)
                                if cfg!(windows)
                                    && error.kind() == std::io::ErrorKind::PermissionDenied => {}
                            Err(error) => panic!("{error}"),
                        }
                    }
                });
            }
        });
        let written = fs::read_to_string(&path).unwrap();
        assert!(payloads.contains(&written), "a torn file: {written:.80}");
        let names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["installed.json"]);
    }

    #[cfg(unix)]
    #[test]
    fn an_owner_only_file_has_mode_0600_whatever_it_replaced() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        write_atomically(&path, b"{}", Readers::Default).unwrap();
        write_atomically(&path, b"{\"a\":1}", Readers::OwnerOnly).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    /// The file's DACL, in SDDL.
    #[cfg(windows)]
    fn dacl(path: &std::path::Path) -> String {
        use ::windows::Win32::Foundation::{HLOCAL, LocalFree};
        use ::windows::Win32::Security::Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, GetNamedSecurityInfoW,
            SDDL_REVISION_1, SE_FILE_OBJECT,
        };
        use ::windows::Win32::Security::{DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};
        use ::windows::core::{PCWSTR, PWSTR};
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: `wide` is NUL-terminated; the descriptor is freed below.
        let status = unsafe {
            GetNamedSecurityInfoW(
                PCWSTR(wide.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                None,
                None,
                &mut descriptor,
            )
        };
        assert!(status.is_ok(), "{status:?}");
        let mut text = PWSTR::null();
        // SAFETY: a valid descriptor; `text` is freed below.
        unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &mut text,
                None,
            )
        }
        .unwrap();
        // SAFETY: a NUL-terminated string the call allocated.
        let sddl = unsafe { text.to_string() }.unwrap();
        // SAFETY: both were allocated with LocalAlloc by the calls above.
        unsafe {
            LocalFree(Some(HLOCAL(text.0.cast())));
            LocalFree(Some(HLOCAL(descriptor.0)));
        }
        sddl
    }

    #[cfg(windows)]
    #[test]
    fn an_owner_only_file_is_open_to_this_user_and_system_only_whatever_it_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        write_atomically(&path, b"{}", Readers::Default).unwrap();
        // A file of the folder's permissions inherits them.
        assert!(!dacl(&path).starts_with("D:P"), "{}", dacl(&path));
        write_atomically(&path, b"{\"a\":1}", Readers::OwnerOnly).unwrap();
        let sddl = dacl(&path);
        let mut user = super::owner_only::user_sid().unwrap();
        // SDDL names the built-in Administrator account by its alias.
        if user.starts_with("S-1-5-21-") && user.ends_with("-500") {
            user = "LA".into();
        }
        assert!(sddl.starts_with("D:P"), "{sddl}");
        let mut entries: Vec<&str> = sddl["D:P".len()..]
            .trim_matches(['(', ')'])
            .split(")(")
            .collect();
        entries.sort_unstable();
        let mut expected = ["A;;FA;;;SY".to_string(), format!("A;;FA;;;{user}")];
        expected.sort_unstable();
        assert_eq!(entries, expected, "{sddl}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"a\":1}");
    }
}
