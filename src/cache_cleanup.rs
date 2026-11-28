//! Descriptor-bounded snapshot cleanup through held, no-follow directory capabilities.
//! Cargo has no launcher exclusion contract: target/profile and package-cache lock inodes and
//! their ancestry are retained. Locks are checked before enumeration, then reacquired through
//! each file's removal. A late Cargo either blocks deletion or starts after it has completed.
//! Only snapshotted files are selected. Same-user swaps inside caches are not a security boundary.
pub use crate::native_cache::Error;
use crate::native_cache::{known_link, locked, LockMode, Locked};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const LOCKS: [&str; 3] = [".cargo-lock", ".package-cache", ".package-cache-mutate"];

/// Classification belongs to the fixed-path inventory, never a process snapshot.
pub(crate) enum Access {
    BuildTools,
    UninstrumentedGame,
    LeasedGame,
    IncompleteGame,
    UnpublishedGame,
}
impl Access {
    /// Installed units trust only their own executable; shared units require all potentially
    /// referencing installed builds. Unknown ownership remains fail-closed.
    pub(crate) fn for_unit(path: &Path) -> Result<Self, Error> {
        if !path.try_exists()? {
            return Ok(Self::UninstrumentedGame);
        }
        let path = path.canonicalize()?;
        let path = path.as_path();
        let data = match crate::backend::data_dir().canonicalize() {
            Ok(data) => data,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::UninstrumentedGame)
            }
            Err(error) => return Err(error.into()),
        };
        if !path.starts_with(&data) {
            return Ok(Self::UninstrumentedGame);
        }
        let build = match crate::backend::build_dir().canonicalize() {
            Ok(build) => build,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                crate::backend::build_dir()
            }
            Err(error) => return Err(error.into()),
        }
        .join("native");
        if path
            .strip_prefix(&build)
            .is_ok_and(|relative| relative.starts_with("target"))
        {
            return Ok(Self::IncompleteGame);
        }
        let versions = data.join("versions");
        let servers = build.join("server-builds");
        for (root, binary) in [
            (&versions, crate::backend::CLIENT.to_owned()),
            (
                &servers,
                format!("sunrise-engine{}", std::env::consts::EXE_SUFFIX),
            ),
        ] {
            if let Ok(relative) = path.strip_prefix(root) {
                if let Some(unit) = relative.components().next() {
                    if root == &versions
                        && crate::backend::installation::private_stage(unit.as_os_str())
                    {
                        return Ok(Self::UnpublishedGame);
                    }
                    let image = root.join(unit).join(binary);
                    return Ok(if capable_image(&image, &data)? {
                        Self::LeasedGame
                    } else {
                        Self::UninstrumentedGame
                    });
                }
            }
        }
        let mut images = Vec::new();
        // Include incomplete/unused cached builds too: an old executable does not honor
        // completion markers. These shared units conservatively consider all local builds.
        for root in [
            &versions,
            &servers,
            &build.join("prebuilt"),
            &build.join("target"),
        ] {
            match std::fs::read_dir(root) {
                Ok(entries) => {
                    for entry in entries {
                        let entry = entry?;
                        let kind = entry.file_type()?;
                        if kind.is_symlink() {
                            return Ok(Self::UninstrumentedGame);
                        }
                        if !kind.is_dir() {
                            continue;
                        }
                        let unit = entry.path();
                        for binary in [
                            crate::backend::CLIENT.to_owned(),
                            format!("sunrise-engine{}", std::env::consts::EXE_SUFFIX),
                        ] {
                            if unit.join(&binary).is_file() {
                                images.push(unit.join(&binary));
                            }
                            // Cargo cross-target units contain target-triple/profile directories.
                            if root.ends_with("target") {
                                for profile in crate::native_cache::PROFILES {
                                    if unit.join(profile).join(&binary).is_file() {
                                        images.push(unit.join(profile).join(&binary));
                                    }
                                }
                            }
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        if path.is_dir() {
            for binary in [
                crate::backend::CLIENT.to_owned(),
                format!("sunrise-engine{}", std::env::consts::EXE_SUFFIX),
            ] {
                if path.join(&binary).is_file() {
                    images.push(path.join(binary));
                }
            }
        }
        if images.is_empty() {
            return Ok(Self::UninstrumentedGame);
        }
        for image in images {
            if !capable_image(&image, &data)? {
                return Ok(Self::UninstrumentedGame);
            }
        }
        Ok(Self::LeasedGame)
    }

    pub(crate) fn check(self) -> Result<(), Error> {
        match self {
            Self::BuildTools | Self::LeasedGame => Ok(()),
            Self::IncompleteGame => Err(Error::IncompleteUnit),
            Self::UnpublishedGame => Err(Error::UnpublishedGame),
            Self::UninstrumentedGame => Err(Error::UnprovenExclusion),
        }
    }
}

fn capable_image(image: &Path, data: &Path) -> Result<bool, Error> {
    let meta = match std::fs::symlink_metadata(image) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if !meta.is_file() || crate::native_cache::linked(&meta) {
        return Ok(false);
    }
    let image = image.canonicalize()?;
    if !image.starts_with(data) {
        return Ok(false);
    }
    sunrise_launcher_smod::holder::capable(&image).map_err(Into::into)
}

/// Operational locks are neither disposable outputs nor user reports/jar-cache entries.
pub(crate) fn reserved(name: &Path) -> bool {
    LOCKS.iter().any(|lock| name == Path::new(lock)) || name == Path::new(".build.lock")
}

/// One root handle and value-only output records; no handle is retained per file or directory.
pub struct Plan {
    directory: Dir,
    outputs: Vec<PathBuf>,
}

/// A link is kept in a plan only when it is one the game leaves here (`known_link`); clearing
/// removes the link itself and never opens what it points at.
fn known(directory: &Dir, name: &Path) -> bool {
    directory
        .read_link_contents(name)
        .is_ok_and(|target| known_link(&target))
}

fn exists(directory: &Dir, name: &str) -> Result<bool, Error> {
    match directory.symlink_metadata(name) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn guard(directory: &Dir, relative: &Path) -> Result<Vec<Locked>, Error> {
    let root = relative.as_os_str().is_empty();
    // Cargo locks target itself, target/<profile> and target/<triple>/<profile>.
    let target = relative
        .ancestors()
        .take(3)
        .any(|path| path.file_name().is_some_and(|name| name == "target"));
    let cargo = root || target || exists(directory, LOCKS[0])?;
    let packages = root || exists(directory, LOCKS[1])? || exists(directory, LOCKS[2])?;
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .follow(FollowSymlinks::No);
    let mut locks = Vec::new();
    for (name, needed) in [
        (LOCKS[0], cargo),
        (LOCKS[1], packages),
        (LOCKS[2], packages),
    ] {
        if needed {
            locks.push(locked(
                directory.open_with(name, &options)?.into_std(),
                LockMode::Exclusive,
            )?);
        }
    }
    Ok(locks)
}

impl Plan {
    /// Launcher Cargo output roots must be named target; existing locks anywhere are honored.
    /// Arbitrary late-created locks in other same-user directories are outside this contract.
    pub fn pin(directory: &Dir) -> Result<Self, Error> {
        Self::pin_observed(directory, &|| {})
    }

    fn pin_observed(directory: &Dir, observe: &impl Fn()) -> Result<Self, Error> {
        let mut plan = Self {
            directory: directory.try_clone()?,
            outputs: Vec::new(),
        };
        plan.visit(directory, Path::new(""), observe)?;
        Ok(plan)
    }

    pub fn pin_file(directory: &Dir, name: &Path) -> Result<Self, Error> {
        if name.components().count() != 1 || reserved(name) {
            return Err(Error::Unsafe(name.to_owned()));
        }
        let _locks = guard(directory, Path::new(""))?;
        let meta = directory.symlink_metadata(name)?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(Error::Unsafe(name.to_owned()));
        }
        Ok(Self {
            directory: directory.try_clone()?,
            outputs: vec![name.to_owned()],
        })
    }

    fn visit(
        &mut self,
        directory: &Dir,
        relative: &Path,
        observe: &impl Fn(),
    ) -> Result<(), Error> {
        let _locks = guard(directory, relative)?;
        observe();
        for entry in directory.entries()? {
            let name = PathBuf::from(entry?.file_name());
            observe();
            if reserved(&name) {
                continue;
            }
            let meta = directory.symlink_metadata(&name)?;
            let path = relative.join(&name);
            if meta.file_type().is_symlink() {
                if !known(directory, &name) {
                    return Err(Error::Unsafe(name));
                }
                self.outputs.push(path);
            } else if meta.is_dir() {
                self.visit(&directory.open_dir_nofollow(&name)?, &path, observe)?;
            } else if meta.is_file() {
                self.outputs.push(path);
            } else {
                return Err(Error::Unsafe(name));
            }
        }
        Ok(())
    }

    /// Trash into the caller's held root on the same volume; preserve lock inode ancestry.
    /// Busy/errors can leave a partially cleared tree; no freed-byte count is returned on error.
    pub fn clear(self, trash: &Dir) -> Result<(), Error> {
        self.clear_observed(trash, &|_| {}, &|| {})
    }

    fn clear_observed(
        self,
        trash: &Dir,
        before_lock: &impl Fn(&Path),
        observe: &impl Fn(),
    ) -> Result<(), Error> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for output in &self.outputs {
            before_lock(output);
            let mut parent = self.directory.try_clone()?;
            let mut relative = PathBuf::new();
            let mut locks = guard(&parent, &relative)?;
            for component in output.parent().unwrap_or(Path::new("")).components() {
                parent = parent.open_dir_nofollow(component.as_os_str())?;
                relative.push(component);
                locks.extend(guard(&parent, &relative)?);
                observe();
            }
            let file = Path::new(output.file_name().unwrap());
            let meta = match parent.symlink_metadata(file) {
                Ok(meta) => meta,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            let link = meta.file_type().is_symlink();
            // Pinned as a file or as a known link; whatever else stands here now is left alone.
            if !(meta.is_file() || link && known(&parent, file)) {
                continue;
            }
            let name = format!(
                ".trash-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            parent.rename(file, trash, &name)?;
            observe();
            #[cfg(windows)]
            if meta.permissions().readonly() {
                use cap_std::fs::OpenOptionsExt;
                use windows_sys::Win32::Storage::FileSystem::FILE_WRITE_ATTRIBUTES;
                let mut options = OpenOptions::new();
                options
                    .read(true)
                    .access_mode(FILE_WRITE_ATTRIBUTES)
                    .follow(FollowSymlinks::No);
                let mut permissions = meta.permissions();
                permissions.set_readonly(false);
                trash
                    .open_with(&name, &options)?
                    .set_permissions(permissions)?;
            }
            // A link to a folder is itself a folder entry on Windows.
            match trash.remove_file(&name) {
                Err(_) if link && cfg!(windows) => trash.remove_dir(&name)?,
                result => result?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

struct DeleteOptions<'a> {
    contexts: &'a [&'a Path],
    expected: Option<(u64, u64)>,
    idle: bool,
    explicit_storage_clear: bool,
    check_profile_pins: bool,
}

/// Game-visible units are detached whole while exclusively leased, never partially cleared.
pub(crate) fn delete_game_unit(path: &Path) -> Result<(), Error> {
    delete_game_unit_observed(path, || {})
}

pub(crate) fn delete_game_unit_with_contexts(
    path: &Path,
    contexts: &[&Path],
    expected: Option<(u64, u64)>,
) -> Result<(), Error> {
    delete_unit(
        path,
        DeleteOptions {
            contexts,
            expected,
            idle: false,
            explicit_storage_clear: false,
            check_profile_pins: true,
        },
        || {},
        || Ok(()),
    )
}

pub(crate) fn delete_game_unit_observed(
    path: &Path,
    after_classification: impl FnOnce(),
) -> Result<(), Error> {
    delete_unit(
        path,
        DeleteOptions {
            contexts: &[],
            expected: None,
            idle: false,
            explicit_storage_clear: false,
            check_profile_pins: true,
        },
        after_classification,
        || Ok(()),
    )
}

/// Explicit Storage Clear relies on the exclusive claim, not an executable marker.
pub(crate) fn clear_claimed_unit_with_contexts(
    path: &Path,
    contexts: &[&Path],
    expected: Option<(u64, u64)>,
) -> Result<(), Error> {
    delete_unit(
        path,
        DeleteOptions {
            contexts,
            expected,
            idle: true,
            explicit_storage_clear: true,
            check_profile_pins: true,
        },
        || {},
        || Ok(()),
    )
}

/// Remove an installed version after the caller's selection checks under the publication fence.
/// Legacy builds have no runtime lease, so `check` must also rule out a running program.
pub(crate) fn delete_version_checked(
    path: &Path,
    check: impl FnOnce() -> Result<(), Error>,
) -> Result<(), Error> {
    delete_unit(
        path,
        DeleteOptions {
            contexts: &[],
            expected: None,
            idle: true,
            explicit_storage_clear: false,
            check_profile_pins: false,
        },
        || {},
        check,
    )
}

/// Remove an installed version whose running processes have already been checked.
pub(crate) fn delete_version(path: &Path, idle: bool) -> Result<(), Error> {
    delete_unit(
        path,
        DeleteOptions {
            contexts: &[],
            expected: None,
            idle,
            explicit_storage_clear: false,
            check_profile_pins: true,
        },
        || {},
        || Ok(()),
    )
}

fn delete_unit(
    path: &Path,
    options: DeleteOptions<'_>,
    after_classification: impl FnOnce(),
    check: impl FnOnce() -> Result<(), Error>,
) -> Result<(), Error> {
    use sunrise_launcher_smod::{locks::Release, runtime_leases::Registry};
    let registry = Registry::with_release(&crate::backend::data_dir(), Release::Close)?;
    let is_version_unit = path.parent() == Some(crate::backend::versions_dir().as_path());
    registry.remove_unit_with_contexts(
        path,
        options.contexts,
        options.expected,
        |directory, is_directory| {
            if !options.explicit_storage_clear {
                match Access::for_unit(path)? {
                    Access::UninstrumentedGame if options.idle => (),
                    access => access.check()?,
                }
            } else if is_version_unit
                && path
                    .file_name()
                    .is_some_and(crate::backend::installation::private_stage)
            {
                return Err(Error::UnpublishedGame);
            }
            // remove_unit holds the same exclusive fence as journal publication and
            // roster changes. Candidate selection alone can never authorize detaching.
            if is_version_unit {
                let tag = path
                    .file_name()
                    .and_then(std::ffi::OsStr::to_str)
                    .ok_or_else(|| Error::Unsafe(path.to_owned()))?;
                // Explicit Storage Clear skips Access::for_unit and may select a build that a
                // standalone process started using after discovery; check under the fence while
                // keeping low-level delete_game_unit semantics unchanged.
                if options.explicit_storage_clear && crate::backend::in_use(tag) {
                    return Err(Error::Busy);
                }
                // Traditional native builds also keep the resources of their base build.
                if let Some(build) = crate::native_mods::resource_dependent(tag) {
                    return Err(Error::ResourceDependent(build));
                }
                crate::managed_play::require_unreferenced_build(tag)?;
                if options.check_profile_pins {
                    crate::backend::version_storage::require_unpinned_by_profiles(tag)?;
                }
            }
            check()?;
            after_classification();
            if is_directory && contains_operational_locks(directory)? {
                return Err(Error::IncompleteUnit);
            }
            Ok(())
        },
    )
}

fn contains_operational_locks(directory: &Dir) -> Result<bool, Error> {
    for entry in directory.entries()? {
        let name = PathBuf::from(entry?.file_name());
        if reserved(&name) {
            return Ok(true);
        }
        let meta = directory.symlink_metadata(&name)?;
        if meta.file_type().is_symlink() {
            if known(directory, &name) {
                continue;
            }
            return Err(Error::Unsafe(name));
        }
        if meta.is_dir() && contains_operational_locks(&directory.open_dir_nofollow(&name)?)? {
            return Ok(true);
        }
    }
    Ok(false)
}
