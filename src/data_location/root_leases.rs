//! One registry for the OS account, independent of which location file or data roots a launcher uses.
use super::{canonical_data_path, open_lock_file, overlaps, problem, MigrationLease, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, TryLockError},
    path::{Path, PathBuf},
};

pub(super) struct Registry(PathBuf);

impl Registry {
    pub(super) fn open() -> Result<Self> {
        #[cfg(unix)]
        let path = {
            use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _};
            // SAFETY: geteuid has no arguments and returns the effective OS account ID.
            let uid = unsafe { libc::geteuid() };
            // HOME, TMPDIR and XDG may differ between portable launches. Read the OS
            // account directly and keep the registry out of temporary-file cleanup.
            let home = account_home(uid)?;
            let base = if cfg!(target_os = "macos") {
                home.join("Library/Application Support")
            } else {
                home.join(".local/state")
            };
            let path = base.join("sunrise-launcher-root-leases");
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true).mode(0o700);
            match builder.create(&path) {
                Ok(()) => (),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(error) => return Err(problem(&path, error)),
            }
            let meta = fs::symlink_metadata(&path).map_err(|error| problem(&path, error))?;
            if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
                return Err(not_private(&path));
            }
            path
        };
        #[cfg(windows)]
        let path = {
            // LocalAppData is the OS known folder, not SUNRISE_LAUNCHER_HOME or a bootstrap override.
            let path = dirs::data_local_dir()
                .ok_or_else(|| {
                    crate::i18n::tr("Cannot locate the launcher lock folder").to_owned()
                })?
                .join("sunrise-launcher-root-leases");
            fs::create_dir_all(&path).map_err(|error| problem(&path, error))?;
            let meta = fs::symlink_metadata(&path).map_err(|error| problem(&path, error))?;
            if !meta.is_dir() || crate::native_cache::linked(&meta) {
                return Err(not_private(&path));
            }
            path
        };
        Ok(Self(canonical_data_path(&path)?))
    }

    pub(super) fn path(&self) -> &Path {
        &self.0
    }

    pub(super) fn acquire(&self, roots: &[&Path], exclusive: bool) -> Result<MigrationLease> {
        let gate = self.0.join("gate.lock");
        let file = open_lock_file(&gate)?;
        file.lock().map_err(|error| problem(&gate, error))?;
        let _gate = MigrationLease(file);
        let roots = roots
            .iter()
            .map(|root| claim_key(root).map(|key| (*root, key)))
            .collect::<Result<Vec<_>>>()?;

        for entry in fs::read_dir(&self.0).map_err(|error| problem(&self.0, error))? {
            let entry = entry.map_err(|error| problem(&self.0, error))?;
            if !entry.file_name().to_string_lossy().starts_with("claim-") {
                continue;
            }
            let path = entry.path();
            let owner = path.join("owner.lock");
            // The gate protects both registration and reaping. Only the kernel lock,
            // not a PID or a timeout, distinguishes a live claim from a crashed one.
            match fs::symlink_metadata(&owner) {
                Ok(_) => {
                    let file = open_lock_file(&owner)?;
                    match file.try_lock() {
                        Ok(()) => drop(MigrationLease(file)),
                        Err(TryLockError::WouldBlock) => {
                            let record = path.join("claim.json");
                            let bytes =
                                fs::read(&record).map_err(|error| problem(&record, error))?;
                            let active: Claim = serde_json::from_slice(&bytes)
                                .map_err(|error| problem(&record, error))?;
                            if exclusive || active.exclusive {
                                let overlap = match active.exact {
                                    Some(exact) => {
                                        if exact.len() != active.roots.len()
                                            || exact.iter().any(|root| root.directories.is_empty())
                                        {
                                            return Err(problem(
                                                &record,
                                                std::io::ErrorKind::InvalidData,
                                            ));
                                        }
                                        roots.iter().find(|(_, key)| {
                                            exact.iter().any(|root| key.exact.overlaps(root))
                                        })
                                    }
                                    None => {
                                        let legacy: Vec<_> =
                                            active.roots.into_iter().map(decode_path).collect();
                                        roots.iter().find(|(_, key)| {
                                            legacy.iter().any(|root| overlaps(&key.legacy, root))
                                        })
                                    }
                                };
                                if let Some((root, _)) = overlap {
                                    return Err(crate::i18n::trf(
                                        "Cannot lock {path}: another launcher is using or moving this data. Close it and try again.",
                                        &[("path", &root.to_string_lossy())],
                                    ));
                                }
                            }
                            continue;
                        }
                        Err(error) => return Err(problem(&owner, error)),
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Err(error) => return Err(problem(&owner, error)),
            }
            fs::remove_dir_all(&path).map_err(|error| problem(&path, error))?;
        }

        // Publish the metadata while the gate excludes readers. Keep it readable even on
        // Windows, where an exclusive byte-range lock prevents other handles from reading.
        let entry = tempfile::Builder::new()
            .prefix("claim-")
            .tempdir_in(&self.0)
            .map_err(|error| problem(&self.0, error))?;
        let lease = super::lock_file(&entry.path().join("owner.lock"), true)?;
        let record = entry.path().join("claim.json");
        let claim = Claim {
            roots: roots
                .iter()
                .map(|(_, key)| encode_path(key.legacy.as_os_str()))
                .collect(),
            exact: Some(roots.into_iter().map(|(_, key)| key.exact).collect()),
            exclusive,
        };
        let bytes = serde_json::to_vec(&claim).map_err(|error| problem(&record, error))?;
        fs::write(&record, bytes).map_err(|error| problem(&record, error))?;
        let _ = entry.keep();
        Ok(lease)
    }
}

struct ClaimKey {
    legacy: PathBuf,
    exact: CheckedRoot,
}

fn claim_key(path: &Path) -> Result<ClaimKey> {
    let (prefix, missing) = match fs::canonicalize(path) {
        Ok(real) => (super::plain_drive(real), Vec::new()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let absolute = std::path::absolute(path).map_err(|error| problem(path, error))?;
            let (prefix, missing) = super::resolved_prefix(&absolute)?;
            use sunrise_launcher_smod::index::browser_state::portable_ascii_component;
            if missing
                .iter()
                .any(|part| !part.to_str().is_some_and(portable_ascii_component))
            {
                return Err(problem(
                    path,
                    crate::i18n::tr(
                        "Choose a portable folder name for a data folder that does not exist yet.",
                    ),
                ));
            }
            (prefix, missing)
        }
        Err(error) => return Err(problem(path, error)),
    };
    let mut directories = prefix
        .ancestors()
        .map(|dir| {
            Ok(ExistingDirectory {
                name: encode_path(dir.file_name().unwrap_or(dir.as_os_str())),
                identity: directory_identity(dir).map_err(|error| problem(dir, error))?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    directories.reverse();
    let legacy = missing
        .iter()
        .rev()
        .fold(prefix, |path, part| path.join(part));
    Ok(ClaimKey {
        legacy: PathBuf::from(legacy.as_os_str().to_ascii_lowercase()),
        exact: CheckedRoot {
            directories,
            missing: missing
                .iter()
                .rev()
                .map(|part| encode_path(&part.to_ascii_lowercase()))
                .collect(),
        },
    })
}

#[derive(Serialize, Deserialize)]
struct CheckedRoot {
    /// Exact native spelling and file identity through the deepest existing directory.
    directories: Vec<ExistingDirectory>,
    /// A reservation keeps these components unresolved for its lifetime, even after mkdir.
    missing: Vec<Vec<PathUnit>>,
}

#[derive(Serialize, Deserialize)]
struct ExistingDirectory {
    name: Vec<PathUnit>,
    identity: (u64, u64),
}

impl CheckedRoot {
    fn overlaps(&self, other: &Self) -> bool {
        self.overlaps_from_existing_prefix(other) || other.overlaps_from_existing_prefix(self)
    }

    fn overlaps_from_existing_prefix(&self, other: &Self) -> bool {
        let parent = self.directories.last().expect("checked existing prefix");
        other.directories.iter().enumerate().any(|(i, dir)| {
            // Identity also recognizes differently cased aliases on insensitive volumes.
            // Preserve exact path containment if a real directory was replaced meanwhile.
            let same_parent = dir.identity == parent.identity
                || i + 1 == self.directories.len()
                    && self
                        .directories
                        .iter()
                        .zip(&other.directories)
                        .all(|(a, b)| a.name == b.name);
            same_parent
                && self
                    .missing
                    .iter()
                    .zip(
                        other.directories[i + 1..]
                            .iter()
                            .map(|dir| &dir.name)
                            .chain(&other.missing),
                    )
                    .all(|(a, b)| {
                        a.len() == b.len() && a.iter().zip(b).all(|(&a, &b)| a == fold_ascii(b))
                    })
        })
    }
}

fn fold_ascii(unit: PathUnit) -> PathUnit {
    if (PathUnit::from(b'A')..=PathUnit::from(b'Z')).contains(&unit) {
        unit + PathUnit::from(b'a' - b'A')
    } else {
        unit
    }
}

fn directory_identity(path: &Path) -> std::io::Result<(u64, u64)> {
    #[cfg(unix)]
    let (volume, inode, directory) = {
        use std::os::unix::fs::MetadataExt as _;
        let meta = fs::metadata(path)?;
        (meta.dev(), meta.ino(), meta.is_dir())
    };
    #[cfg(windows)]
    let (volume, inode, directory) = {
        use cap_fs_ext::MetadataExt as _;
        use std::os::windows::fs::OpenOptionsExt as _;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES,
        };
        // Metadata only. A searchable ancestor need not be readable or writable.
        let file = fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)?;
        let meta = cap_std::fs::File::from_std(file).metadata()?;
        (meta.dev(), meta.ino(), meta.is_dir())
    };
    if !directory {
        return Err(std::io::ErrorKind::NotADirectory.into());
    }
    Ok((volume, inode))
}

#[cfg(unix)]
fn account_home(uid: libc::uid_t) -> Result<PathBuf> {
    use std::os::unix::ffi::OsStrExt as _;
    let missing = || crate::i18n::tr("Cannot locate the launcher lock folder").to_owned();
    let mut buffer = vec![0u8; 16 * 1024];
    loop {
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        // SAFETY: both return pointers are initialized by getpwuid_r on success;
        // the string storage is in the writable buffer for the duration of the lookup.
        let code = unsafe {
            libc::getpwuid_r(
                uid,
                entry.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if code == libc::ERANGE && buffer.len() < 1024 * 1024 {
            buffer.resize(buffer.len() * 2, 0);
            continue;
        }
        if code != 0 {
            return Err(format!(
                "{}: {}",
                missing(),
                std::io::Error::from_raw_os_error(code)
            ));
        }
        if result.is_null() {
            return Err(missing());
        }
        // SAFETY: a non-null successful result points at the initialized entry. pw_dir
        // is a NUL-terminated string in the live buffer returned by getpwuid_r.
        let directory = unsafe { (*result).pw_dir };
        if directory.is_null() {
            return Err(missing());
        }
        let home = Path::new(std::ffi::OsStr::from_bytes(unsafe {
            std::ffi::CStr::from_ptr(directory).to_bytes()
        }));
        return home
            .is_absolute()
            .then(|| home.to_owned())
            .ok_or_else(missing);
    }
}

fn not_private(path: &Path) -> String {
    problem(
        path,
        crate::i18n::tr("The launcher lock folder is not private"),
    )
}

// JSON paths lose non-UTF-8 Unix names and unpaired Windows surrogates. These records
// are local to one OS; native path units round trip without changing the selected root.
#[cfg(unix)]
type PathUnit = u8;
#[cfg(windows)]
type PathUnit = u16;

#[derive(Serialize, Deserialize)]
struct Claim {
    // Older launchers still use this conservative spelling for mixed-version leases.
    roots: Vec<Vec<PathUnit>>,
    #[serde(default)]
    exact: Option<Vec<CheckedRoot>>,
    exclusive: bool,
}

fn encode_path(path: &std::ffi::OsStr) -> Vec<PathUnit> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        path.as_bytes().to_vec()
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;
        path.encode_wide().collect()
    }
}

fn decode_path(units: Vec<PathUnit>) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt as _;
        std::ffi::OsString::from_vec(units).into()
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt as _;
        std::ffi::OsString::from_wide(&units).into()
    }
}
