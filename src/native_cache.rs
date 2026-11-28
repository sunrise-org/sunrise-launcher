//! Disposable native compilation outputs only. Installed games and player data are never targets.
//! Keep Cargo's profile directories and lock files in place: unlinking a locked inode would let
//! another Cargo acquire a replacement lock while cleanup was still deleting its outputs.
use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use std::fs::{self, File, Metadata};
use std::io;
use std::path::{Path, PathBuf};

pub(crate) const PROFILES: [&str; 2] = ["fast", "release"];

/// The folder under `native/` holding one persistent compiler work folder per game build. One
/// letter, because Windows' path limit counts it (see `WORK_SLOT_HEX` in the native workspace).
pub(crate) const WORK: &str = "w";

#[derive(Debug)]
pub enum Error {
    Busy,
    UnprovenExclusion,
    IncompleteUnit,
    UnpublishedGame,
    ReferencedProfile,
    ResourceDependent(String),
    UnprovenReferences,
    UnsupportedFilesystem,
    Unsafe(PathBuf),
    Io(io::Error),
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub(crate) struct Cache {
    root: PathBuf,
    build_dir: Dir,
    directory: Dir,
    _lease: Locked,
}

pub(crate) use sunrise_launcher_smod::locks::{LockMode, Locked, Release};

pub(crate) use sunrise_launcher_smod::runtime_leases::linked;
pub(crate) fn checked(path: &Path, directory: bool) -> Result<Option<Metadata>, Error> {
    sunrise_launcher_smod::runtime_leases::checked(path, directory).map_err(Into::into)
}
impl From<sunrise_launcher_smod::runtime_leases::Error> for Error {
    fn from(error: sunrise_launcher_smod::runtime_leases::Error) -> Self {
        use sunrise_launcher_smod::runtime_leases::Error as Shared;
        match error {
            Shared::Busy => Self::Busy,
            Shared::UnsupportedFilesystem => Self::UnsupportedFilesystem,
            Shared::Unsafe(path) => Self::Unsafe(path),
            Shared::Io(error) => Self::Io(error),
        }
    }
}

/// Whether `target` is where a link the game's prepare step leaves in a cache points: into an
/// installed build under the launcher's own `versions/`. Such a link is counted as nothing and
/// removed as the link itself. Its target is never opened, so the test is on the written path
/// alone, and every other link is refused as before.
pub(crate) fn known_link(target: &Path) -> bool {
    if !target.is_absolute()
        || target
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return false;
    }
    let versions = crate::backend::data_dir().join("versions");
    target.starts_with(&versions)
        || versions.canonicalize().is_ok_and(|canonical| {
            target.starts_with(&canonical) || target.starts_with(crate::backend::plain(canonical))
        })
}

fn lock(path: &Path) -> Result<Locked, Error> {
    checked(path, false)?;
    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    locked(file, LockMode::Exclusive)
}

pub(crate) fn locked(file: File, mode: LockMode) -> Result<Locked, Error> {
    sunrise_launcher_smod::locks::locked(file, mode).map_err(|error| match error {
        fs::TryLockError::WouldBlock => Error::Busy,
        fs::TryLockError::Error(error) => Error::Io(error),
    })
}

/// A persistent build folder as [`Cache::work_slot`] left it.
pub(crate) struct Provisioned {
    pub(crate) path: PathBuf,
    /// Nothing is in it: the build that runs there compiles everything.
    pub(crate) cold: bool,
}

/// `name` as one folder name, never a path.
fn single(name: &str) -> Result<&Path, Error> {
    let path = Path::new(name);
    let mut parts = path.components();
    match (parts.next(), parts.next()) {
        (Some(std::path::Component::Normal(_)), None) => Ok(path),
        _ => Err(Error::Unsafe(path.to_owned())),
    }
}

/// Whether a `cap_std` metadata record represents a symlink or Windows reparse point.
pub(crate) fn cap_linked(meta: &cap_std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

/// Removes `name` under `parent` without opening what a link there points to: a symlink or a
/// junction goes as the link, and a folder with links inside goes with each of them unlinked
/// through the held `Dir` handle.
pub(crate) fn remove_entry(parent: &Dir, name: &Path) -> io::Result<()> {
    let meta = match parent.symlink_metadata(name) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if cap_linked(&meta) {
        // On Windows a junction or folder symlink goes by `remove_dir`; a file symlink goes by
        // `remove_file`.
        #[cfg(windows)]
        {
            use cap_std::fs::MetadataExt;
            use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY;
            if meta.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0 {
                return parent.remove_dir(name);
            }
        }
        parent.remove_file(name)
    } else if meta.is_dir() {
        parent.remove_dir_all(name)
    } else {
        parent.remove_file(name)
    }
}

/// The folders of a work slot the SDK's tool keeps and reuses, checked for links one level at a
/// time before any of them is handed to a compiler: the slot itself, `prebuilt-build` and
/// `prebuilt-package`, each build folder in `prebuilt-build`, and in it `target` and `stage-*`.
fn sdk_owned(slot: &Path) -> Result<(), Error> {
    if checked(slot, true)?.is_none() {
        return Ok(());
    }
    checked(&slot.join("prebuilt-package"), true)?;
    let builds = slot.join("prebuilt-build");
    if checked(&builds, true)?.is_none() {
        return Ok(());
    }
    for build in fs::read_dir(&builds)? {
        let build = build?.path();
        if checked(&build, true)?.is_none() {
            continue;
        }
        for entry in fs::read_dir(&build)? {
            let entry = entry?;
            let name = entry.file_name();
            if name == "target" || name.to_string_lossy().starts_with("stage-") {
                checked(&entry.path(), true)?;
            }
        }
    }
    Ok(())
}

fn size(path: &Path) -> Result<u64, Error> {
    let meta = fs::symlink_metadata(path)?;
    if linked(&meta) {
        return Err(Error::Unsafe(path.to_owned()));
    }
    if meta.is_file() {
        return Ok(meta.len());
    }
    if !meta.is_dir() {
        return Err(Error::Unsafe(path.to_owned()));
    }
    let mut bytes = 0u64;
    for entry in fs::read_dir(path)? {
        bytes = bytes.saturating_add(size(&entry?.path())?);
    }
    Ok(bytes)
}

impl Cache {
    /// One cross-process lease covers staging, Cargo, installation/export and manual clear.
    /// OS locks release on exit, including a crash; a leftover lock file is not a busy marker.
    pub(crate) fn acquire(build_dir: &Path) -> Result<Self, Error> {
        checked(build_dir, true)?;
        fs::create_dir_all(build_dir)?;
        let build_path = build_dir.canonicalize()?;
        let root = build_path.join("native");
        checked(&root, true)?;
        fs::create_dir_all(&root)?;
        let anchor = build_path
            .ancestors()
            .last()
            .expect("canonical build root is absolute");
        let mut build_handle = Dir::open_ambient_dir(anchor, cap_std::ambient_authority())?;
        for component in build_path
            .strip_prefix(anchor)
            .expect("build root anchor")
            .components()
        {
            build_handle = build_handle.open_dir_nofollow(component.as_os_str())?;
        }
        let directory = build_handle.open_dir_nofollow("native")?;
        let lease = lock(&root.join(".build.lock"))?;
        Ok(Self {
            root,
            build_dir: build_handle,
            directory,
            _lease: lease,
        })
    }

    #[cfg(test)]
    pub(crate) fn bytes(&self) -> Result<u64, Error> {
        self.process(None, false)
    }

    /// `name` under `parent` as a link-free folder of this cache, made when it is absent. Anything
    /// else there (a link, a junction, a file) is removed through `parent` as itself, never
    /// followed, and replaced by an empty folder. The lease must be held; `parent` is a folder
    /// this cache already owns.
    fn own_in(&self, parent: &Dir, name: &Path, path: &Path) -> Result<Dir, Error> {
        match parent.symlink_metadata(name) {
            Ok(meta) if meta.is_dir() && !cap_linked(&meta) => (),
            Ok(_) => {
                remove_entry(parent, name)?;
                parent.create_dir(name)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                parent.create_dir(name)?;
            }
            Err(error) => return Err(error.into()),
        }
        let dir = parent.open_dir_nofollow(name)?;
        self.unchanged_path(path)?;
        Ok(dir)
    }

    /// Where [`Self::work_slot`] puts `slot`, before anything is made there.
    pub(crate) fn work_slot_path(&self, slot: &str) -> PathBuf {
        crate::backend::plain(self.root.join(WORK).join(slot))
    }

    /// The folder the compiler's own build folders persist in for the game build `slot` names,
    /// `native/w/<slot>`, checked link-free below the lease. A slot holding a link where the
    /// SDK keeps its own folders is removed whole (never followed) and made again empty: it
    /// builds cold once, with no repair of what was there.
    pub(crate) fn work_slot(&self, slot: &str) -> Result<Provisioned, Error> {
        let work = self.root.join(WORK);
        let leaf = single(slot)?;
        let path = work.join(leaf);
        let work_dir = self.own_in(&self.directory, Path::new(WORK), &work)?;
        match sdk_owned(&path) {
            Ok(()) => (),
            Err(Error::Unsafe(_)) => remove_entry(&work_dir, leaf)?,
            Err(error) => return Err(error),
        }
        let slot_dir = self.own_in(&work_dir, leaf, &path)?;
        let cold = slot_dir.entries()?.next().is_none();
        Ok(Provisioned {
            path: crate::backend::plain(path),
            cold,
        })
    }

    /// [`Self::work_slot`] after removing what the slot held: the one cold build a slot that
    /// failed gets.
    pub(crate) fn fresh_work_slot(&self, slot: &str) -> Result<Provisioned, Error> {
        let work = self.root.join(WORK);
        let leaf = single(slot)?;
        // A completed compiler could have changed this parent since the slot was attached.
        // Check and remove strictly through the held `native/w` directory handle.
        let parent = self.directory.open_dir_nofollow(WORK)?;
        self.unchanged_path(&work)?;
        remove_entry(&parent, leaf)?;
        drop(parent);
        self.work_slot(slot)
    }

    /// Cargo's home, `<build folder>/cargo`, the same folder Storage counts as build files,
    /// checked link-free below the lease. A link there is removed as itself and the folder made
    /// again.
    pub(crate) fn cargo_home(&self) -> Result<PathBuf, Error> {
        let build = self
            .root
            .parent()
            .expect("the native folder is inside the build folder");
        let path = build.join("cargo");
        let _dir = self.own_in(&self.build_dir, Path::new("cargo"), &path)?;
        Ok(crate::backend::plain(path))
    }
    pub(crate) fn clear(&self) -> Result<u64, Error> {
        crate::cache_cleanup::Access::IncompleteGame.check()?;
        self.process(None, true)
    }
    #[cfg(test)]
    pub(crate) fn trim(&self, limit: u64) -> Result<u64, Error> {
        if limit == 0 {
            return Ok(0);
        }
        if self.bytes()? <= limit {
            return Ok(0);
        }
        crate::cache_cleanup::Access::IncompleteGame.check()?;
        self.process(Some(limit), true)
    }

    // Detect a managed directory redirected through an ancestor link after taking the lease.
    fn unchanged_path(&self, path: &Path) -> Result<(), Error> {
        if path.canonicalize()? != path {
            return Err(Error::Unsafe(path.to_owned()));
        }
        Ok(())
    }

    fn process(&self, limit: Option<u64>, clear: bool) -> Result<u64, Error> {
        checked(&self.root, true)?;
        self.unchanged_path(&self.root)?;
        let target = self.root.join("target");
        if checked(&target, true)?.is_none() {
            return Ok(0);
        }
        self.unchanged_path(&target)?;
        let target_dir = self.directory.open_dir_nofollow("target")?;
        let mut locks = Vec::new();
        let mut folders = Vec::new();
        for profile in PROFILES {
            let folder = target.join(profile);
            if checked(&folder, true)?.is_some() {
                // Also protects an older launcher's Cargo or one started outside our lease.
                self.unchanged_path(&folder)?;
                let profile_dir = target_dir.open_dir_nofollow(profile)?;
                locks.push(lock(&folder.join(".cargo-lock"))?);
                folders.push((folder, profile_dir));
            }
        }
        // Validate ALL profiles and descendants before deleting ANY output. Never traverse links.
        let mut batches = Vec::new();
        let mut bytes = 0u64;
        for (folder, profile_dir) in folders {
            let mut names = Vec::new();
            for entry in profile_dir.entries()? {
                let entry = entry?;
                let name = entry.file_name();
                if name == ".cargo-lock" {
                    continue;
                }
                let path = folder.join(&name);
                bytes = bytes.saturating_add(size(&path)?);
                names.push((name, path));
            }
            batches.push((profile_dir, names));
        }
        if !clear {
            return Ok(bytes);
        }
        if limit.is_some_and(|limit| bytes <= limit) {
            return Ok(0);
        }
        let records = self.root.join("built.json");
        let has_records = checked(&records, false)?.is_some();
        for (profile_dir, names) in batches {
            for (name, path) in names {
                self.unchanged_path(&path)?;
                // The launcher lease and all Cargo locks remain held through removal. Remove
                // strictly through the held profile `Dir` handle so an ambient swap cannot
                // redirect deletion.
                let meta = profile_dir.symlink_metadata(&name)?;
                if cap_linked(&meta) {
                    return Err(Error::Unsafe(path));
                }
                if meta.is_dir() {
                    profile_dir.remove_dir_all(&name)?;
                } else {
                    profile_dir.remove_file(&name)?;
                }
            }
        }
        if has_records {
            self.unchanged_path(&records)?;
            let meta = self.directory.symlink_metadata("built.json")?;
            if cap_linked(&meta) || !meta.is_file() {
                return Err(Error::Unsafe(records));
            }
            self.directory.remove_file("built.json")?;
        }
        drop(locks);
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests;
