//! Persistence of the transaction record itself, not of the installed mods it refers to.
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

/// Permission evidence belongs to each file image, not just its bytes. If the stage
/// cannot reproduce the source's access, seal refuses before changing any live names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Access {
    #[cfg(unix)]
    mode: u32,
    #[cfg(unix)]
    owner: u32,
    #[cfg(unix)]
    group: u32,
    #[cfg(windows)]
    descriptor: [u8; 32],
}

impl Access {
    #[cfg(unix)]
    pub(super) fn of(_path: &Path, file: &File) -> io::Result<Self> {
        use std::os::unix::fs::MetadataExt;
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            // A mode is not equivalent to an extended ACL. This journal does not
            // clone extended access metadata; refuse it while everything is private.
            for attr in [c"system.posix_acl_access", c"security.capability"] {
                // SAFETY: valid open file and constant C string; null plus zero queries size.
                let found = unsafe {
                    libc::fgetxattr(file.as_raw_fd(), attr.as_ptr(), std::ptr::null_mut(), 0)
                };
                if found >= 0 {
                    return Err(io::ErrorKind::Unsupported.into());
                }
                let error = io::Error::last_os_error();
                if !matches!(error.raw_os_error(), Some(libc::ENODATA | libc::EOPNOTSUPP)) {
                    return Err(error);
                }
            }
        }
        #[cfg(target_os = "macos")]
        {
            use std::os::unix::ffi::OsStrExt;
            let source = std::ffi::CString::new(_path.as_os_str().as_bytes())?;
            // SAFETY: COPYFILE_CHECK is a read-only source metadata check; no destination
            // or callback state is used. Reject an ACL that mode/owner alone cannot prove.
            let metadata = unsafe {
                libc::copyfile(
                    source.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    libc::COPYFILE_CHECK | libc::COPYFILE_ACL | libc::COPYFILE_NOFOLLOW_SRC,
                )
            };
            if metadata < 0 {
                return Err(io::Error::last_os_error());
            }
            if metadata as u32 & libc::COPYFILE_ACL != 0 {
                return Err(io::ErrorKind::Unsupported.into());
            }
        }
        let meta = file.metadata()?;
        Ok(Self {
            mode: meta.mode() & 0o7777,
            owner: meta.uid(),
            group: meta.gid(),
        })
    }

    #[cfg(windows)]
    pub(super) fn of(path: &Path, file: &File) -> io::Result<Self> {
        use sha2::{Digest, Sha256};
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::{
            Foundation::ERROR_INSUFFICIENT_BUFFER,
            Security::{
                GetFileSecurityW, DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION,
                OWNER_SECURITY_INFORMATION,
            },
        };
        if file.metadata()?.permissions().readonly() {
            // A read-only path cannot be atomically replaced and flushed by this journal.
            return Err(io::ErrorKind::Unsupported.into());
        }
        let path: Vec<_> = path.as_os_str().encode_wide().chain([0]).collect();
        require_unnamed_data_only(&path)?;
        let pieces =
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
        let mut size = 0;
        // SAFETY: a nul buffer with size 0 queries the size of a self-relative descriptor.
        let probed =
            unsafe { GetFileSecurityW(path.as_ptr(), pieces, std::ptr::null_mut(), 0, &mut size) };
        let error = io::Error::last_os_error();
        if probed != 0
            || error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32)
            || size == 0
        {
            return Err(error);
        }
        let mut buffer = vec![0u32; (size as usize).div_ceil(4)];
        // SAFETY: the aligned owned buffer has at least size writable bytes.
        if unsafe {
            GetFileSecurityW(
                path.as_ptr(),
                pieces,
                buffer.as_mut_ptr().cast(),
                size,
                &mut size,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: all stored bytes (including alignment padding) are initialized u32s.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                buffer.as_ptr().cast::<u8>(),
                std::mem::size_of_val(buffer.as_slice()),
            )
        };
        let descriptor = bytes
            .get(..size as usize)
            .ok_or(io::ErrorKind::InvalidData)?;
        Ok(Self {
            descriptor: Sha256::digest(descriptor).into(),
        })
    }

    pub(super) fn apply(self, file: &File) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(self.mode))?;
        }
        #[cfg(windows)]
        let _ = file; // Do not broaden or synthesize a different ACL. Verify the inherited one.
        Ok(())
    }
}

#[cfg(windows)]
fn require_unnamed_data_only(path: &[u16]) -> io::Result<()> {
    use windows_sys::Win32::{
        Foundation::{ERROR_HANDLE_EOF, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            FindClose, FindFirstStreamW, FindNextStreamW, FindStreamInfoStandard,
            WIN32_FIND_STREAM_DATA,
        },
    };
    let mut data = WIN32_FIND_STREAM_DATA::default();
    // SAFETY: path is nul-terminated and data has the requested layout and size.
    let handle = unsafe {
        FindFirstStreamW(
            path.as_ptr(),
            FindStreamInfoStandard,
            (&raw mut data).cast(),
            0,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // The snapshot copies only the unnamed stream, even if another stream is empty.
    // Require positive evidence that this is the only stream on the file.
    let unnamed = data
        .cStreamName
        .iter()
        .copied()
        .take_while(|&unit| unit != 0)
        .eq("::$DATA".encode_utf16());
    // SAFETY: handle is open and data remains writable.
    let result = if !unnamed || unsafe { FindNextStreamW(handle, (&raw mut data).cast()) } != 0 {
        Err(io::ErrorKind::Unsupported.into())
    } else {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_HANDLE_EOF as i32) {
            Ok(())
        } else {
            Err(error)
        }
    };
    // SAFETY: this is the valid search handle obtained above, closed exactly once.
    if unsafe { FindClose(handle) } == 0 && result.is_ok() {
        return Err(io::Error::last_os_error());
    }
    result
}

pub(super) fn replace(home: &Path, record: &Path, bytes: &[u8]) -> io::Result<()> {
    replace_at(home, record, bytes, || {})
}

pub(super) fn replace_at(
    home: &Path,
    record: &Path,
    bytes: &[u8],
    renamed: impl FnOnce(),
) -> io::Result<()> {
    let dir = record.parent().expect("inside the profile");
    fs::create_dir_all(dir)?;
    let mut next = tempfile::Builder::new().prefix(".txn-").tempfile_in(dir)?;
    next.write_all(bytes)?;
    flush(next.as_file())?;
    #[cfg(windows)]
    {
        let staged = next.into_temp_path();
        let _ = home;
        move_through(&staged, record, true)?;
        renamed();
        Ok(())
    }
    #[cfg(not(windows))]
    {
        fs::rename(next.path(), record)?;
        renamed();
        sync_directory(dir)?;
        // `mod-browser` may have just been created, and its own name must survive too.
        sync_directory(home)?;
        #[cfg(target_os = "macos")]
        flush(next.as_file())?;
        Ok(())
    }
}

pub(super) fn retire(record: &Path) -> io::Result<()> {
    #[cfg(not(windows))]
    {
        // On macOS the device flush after the directory sync uses the still-open file.
        #[cfg(target_os = "macos")]
        let old = match File::options().write(true).open(record) {
            Ok(old) => old,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        match fs::remove_file(record) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        }
        sync_directory(record.parent().expect("inside the profile"))?;
        #[cfg(target_os = "macos")]
        flush(&old)?;
        Ok(())
    }
    #[cfg(windows)]
    {
        // No unprivileged directory flush. Retire with the same write-through move as
        // replacement; only the live filename is read, so tombstone cleanup needn't be durable.
        let retired = record.with_extension("json.retired");
        match move_through(record, &retired, true) {
            Ok(()) => {
                let _ = fs::remove_file(retired);
                Ok(())
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}

/// Retire a live record without dropping its only recovery evidence until the rename is durable.
pub(super) fn mark_retired(home: &Path, from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(not(windows))]
    {
        fs::rename(from, to)?;
        establish_retirement(home, to)
    }
    #[cfg(windows)]
    {
        let _ = home;
        move_through(from, to, false)?;
        #[cfg(test)]
        fail_barrier_if_injected(to)?;
        Ok(())
    }
}

/// Re-establish the barrier on the *visible retired record*. In particular, absence of
/// the live name does not prove a rename whose flush failed.
pub(super) fn establish_retirement(home: &Path, retired: &Path) -> io::Result<()> {
    #[cfg(test)]
    fail_barrier_if_injected(retired)?;
    #[cfg(not(windows))]
    {
        let _ = home;
        sync_directory(retired.parent().expect("inside the profile"))?;
        flush(&File::open(retired)?)
    }
    #[cfg(windows)]
    {
        // Re-issue an atomic write-through replacement of the same evidence. File
        // FlushFileBuffers alone is not an unprivileged directory rename barrier.
        replace(home, retired, &fs::read(retired)?)
    }
}

#[cfg(test)]
thread_local! {
    static FAIL_RETIREMENT: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn fail_next_retirement_barrier(record: &Path) {
    FAIL_RETIREMENT.with(|fault| *fault.borrow_mut() = Some(record.to_owned()));
}

#[cfg(test)]
fn fail_barrier_if_injected(record: &Path) -> io::Result<()> {
    FAIL_RETIREMENT.with(|fault| {
        if fault.borrow().as_deref() == Some(record) {
            fault.take();
            return Err(io::Error::other("injected retirement barrier failure"));
        }
        Ok(())
    })
}

/// Repeat the restoration barrier when the target already matches the recorded preimage.
/// A pending addition may never have created its target directory at all.
pub(super) fn sync_restored(to: &Path, backup: &Path, missing: bool) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    let file = if missing { None } else { Some(File::open(to)?) };
    #[cfg(not(windows))]
    for path in [to, backup] {
        let dir = path.parent().expect("inside the profile");
        match sync_directory(dir) {
            Err(e) if (missing || path == backup) && e.kind() == io::ErrorKind::NotFound => {}
            result => result?,
        }
    }
    #[cfg(target_os = "macos")]
    if let Some(file) = file {
        flush(&file)?;
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    if !missing {
        flush(&File::open(to)?)?;
    }
    #[cfg(windows)]
    {
        // Windows' write-through move already supplied the namespace barrier.
        let _ = backup;
        if !missing {
            flush(&File::options().write(true).open(to)?)?;
        }
    }
    Ok(())
}

/// A restore is also a same-filesystem rename. Flush its data and both changed directories
/// before the journal can advance to an older step that may touch the same target again.
pub(super) fn restore_move(from: &Path, to: &Path) -> io::Result<()> {
    restore_move_at(from, to, || {})
}

pub(super) fn restore_move_at(from: &Path, to: &Path, renamed: impl FnOnce()) -> io::Result<()> {
    #[cfg(windows)]
    let file = File::options().write(true).open(from)?;
    #[cfg(not(windows))]
    let file = File::open(from)?;
    flush(&file)?;
    #[cfg(windows)]
    {
        drop(file);
        move_through(from, to, true)?;
        renamed();
        Ok(())
    }
    #[cfg(not(windows))]
    {
        fs::rename(from, to)?;
        renamed();
        for path in [from, to] {
            sync_directory(path.parent().expect("inside the profile"))?;
        }
        #[cfg(target_os = "macos")]
        flush(&file)?;
        Ok(())
    }
}

/// Rename a prepared directory into an absent destination, or undo that move.
pub(super) fn move_new_directory(from: &Path, to: &Path) -> io::Result<()> {
    move_new_directory_at(from, to, || {})
}

pub(super) fn move_new_directory_at(
    from: &Path,
    to: &Path,
    renamed: impl FnOnce(),
) -> io::Result<()> {
    #[cfg(not(windows))]
    sync_directory(from)?;
    #[cfg(not(windows))]
    crate::instances::publish_new_folder(from, to)?;
    #[cfg(windows)]
    move_through(from, to, false)?;
    renamed();
    #[cfg(not(windows))]
    for path in [from, to] {
        sync_directory(path.parent().expect("inside the profile"))?;
    }
    Ok(())
}

/// Unlike the record file, an APFS directory only needs its metadata fsynced. The device
/// barrier runs immediately afterwards on the (regular) record file.
#[cfg(not(windows))]
fn sync_directory(dir: &Path) -> io::Result<()> {
    let directory = File::open(dir)?;
    #[cfg(not(target_os = "macos"))]
    {
        directory.sync_all()
    }
    #[cfg(target_os = "macos")]
    loop {
        use std::os::fd::AsRawFd;
        // SAFETY: this descriptor is valid until `directory` goes out of scope.
        if unsafe { libc::fsync(directory.as_raw_fd()) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

#[cfg(not(windows))]
pub(super) fn flush(file: &File) -> io::Result<()> {
    // Rust's File::sync_all uses F_FULLFSYNC on macOS and fsync on Linux.
    file.sync_all()
}

#[cfg(windows)]
pub(super) fn flush(file: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::FlushFileBuffers;
    // SAFETY: the handle is for the open staging file, not a directory or volume.
    if unsafe { FlushFileBuffers(file.as_raw_handle()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(windows)]
fn move_through(from: &Path, to: &Path, replace: bool) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let wide = |path: &Path| -> io::Result<Vec<u16>> {
        Ok(path
            .parent()
            .expect("inside the profile")
            .canonicalize()?
            .join(path.file_name().expect("has a filename"))
            .as_os_str()
            .encode_wide()
            .chain([0])
            .collect())
    };
    let (from, to) = (wide(from)?, wide(to)?);
    let mut flags = MOVEFILE_WRITE_THROUGH;
    if replace {
        flags |= MOVEFILE_REPLACE_EXISTING;
    }
    // SAFETY: both buffers are NUL-terminated; cross-volume copies are not enabled.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), flags) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_is_replaced_whole_and_retired_even_if_an_older_tombstone_remains() {
        let home = tempfile::tempdir().unwrap();
        let record = home.path().join(crate::profile_txn::RECORD);
        replace(home.path(), &record, b"first").unwrap();
        assert_eq!(fs::read(&record).unwrap(), b"first");
        replace(home.path(), &record, b"the next record").unwrap();
        assert_eq!(fs::read(&record).unwrap(), b"the next record");
        fs::write(record.with_extension("json.retired"), b"an older record").unwrap();
        retire(&record).unwrap();
        assert!(!record.exists());
        retire(&record).unwrap();
    }

    #[test]
    fn replacement_reports_an_io_error_without_touching_an_unexpected_destination() {
        let home = tempfile::tempdir().unwrap();
        let record = home.path().join(crate::profile_txn::RECORD);
        fs::create_dir_all(&record).unwrap();
        fs::write(record.join("keep"), b"not a record").unwrap();
        assert!(replace(home.path(), &record, b"replacement").is_err());
        assert_eq!(fs::read(record.join("keep")).unwrap(), b"not a record");
        assert_eq!(fs::read_dir(record.parent().unwrap()).unwrap().count(), 1);
    }
}
