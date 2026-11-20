//! Persistence of the transaction record itself, not of the installed mods it refers to.
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

pub(crate) fn replace(home: &Path, record: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = record.parent().expect("inside the profile");
    fs::create_dir_all(dir)?;
    let mut next = tempfile::Builder::new().prefix(".txn-").tempfile_in(dir)?;
    next.write_all(bytes)?;
    flush(next.as_file())?;
    #[cfg(windows)]
    {
        let staged = next.into_temp_path();
        let _ = home;
        move_through(&staged, record)
    }
    #[cfg(not(windows))]
    {
        fs::rename(next.path(), record)?;
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
        match move_through(record, &retired) {
            Ok(()) => {
                let _ = fs::remove_file(retired);
                Ok(())
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
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
    #[cfg(windows)]
    let file = File::options().write(true).open(from)?;
    #[cfg(not(windows))]
    let file = File::open(from)?;
    flush(&file)?;
    #[cfg(windows)]
    {
        drop(file);
        move_through(from, to)
    }
    #[cfg(not(windows))]
    {
        fs::rename(from, to)?;
        for path in [from, to] {
            sync_directory(path.parent().expect("inside the profile"))?;
        }
        #[cfg(target_os = "macos")]
        flush(&file)?;
        Ok(())
    }
}

/// Flush a private complete profile before its directory is made visible in one step.
pub(crate) fn flush_tree(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if !super::build::regular(&meta) {
        return Err(io::ErrorKind::InvalidData.into());
    }
    if meta.is_dir() {
        for entry in fs::read_dir(path)? {
            flush_tree(&entry?.path())?;
        }
        #[cfg(not(windows))]
        sync_directory(path)?;
        Ok(())
    } else if meta.is_file() {
        #[cfg(windows)]
        let file = File::options().write(true).open(path)?;
        #[cfg(not(windows))]
        let file = File::open(path)?;
        flush(&file)
    } else {
        Err(io::ErrorKind::InvalidData.into())
    }
}

/// Rename a prepared directory into an absent destination, or undo that move.
pub(crate) fn move_new_directory(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(not(windows))]
    sync_directory(from)?;
    crate::instances::publish_new_folder(from, to)?;
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
fn flush(file: &File) -> io::Result<()> {
    // Rust's File::sync_all uses F_FULLFSYNC on macOS and fsync on Linux.
    file.sync_all()
}

#[cfg(windows)]
fn flush(file: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::FlushFileBuffers;
    // SAFETY: the handle is for the open staging file, not a directory or volume.
    if unsafe { FlushFileBuffers(file.as_raw_handle()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(windows)]
fn move_through(from: &Path, to: &Path) -> io::Result<()> {
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
    // SAFETY: both buffers are NUL-terminated; cross-volume copies are not enabled.
    if unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
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
        let record = home.path().join(super::super::FILE);
        replace(home.path(), &record, b"first").unwrap();
        assert_eq!(fs::read(&record).unwrap(), b"first");
        replace(home.path(), &record, b"the next record").unwrap();
        assert_eq!(fs::read(&record).unwrap(), b"the next record");
        fs::write(record.with_extension("json.retired"), b"an older record").unwrap();
        retire(&record).unwrap();
        assert!(!record.exists());
        assert!(super::super::read(home.path()).unwrap().is_none());
        retire(&record).unwrap();
    }

    #[test]
    fn replacement_reports_an_io_error_without_touching_an_unexpected_destination() {
        let home = tempfile::tempdir().unwrap();
        let record = home.path().join(super::super::FILE);
        fs::create_dir_all(&record).unwrap();
        fs::write(record.join("keep"), b"not a record").unwrap();
        assert!(replace(home.path(), &record, b"replacement").is_err());
        assert_eq!(fs::read(record.join("keep")).unwrap(), b"not a record");
        assert_eq!(fs::read_dir(record.parent().unwrap()).unwrap().count(), 1);
    }
}
