//! SDK inputs and writable paths for one native compiler attempt. The attempt's own folder is
//! never reused by a retry; the compiler's work folder and Cargo's home can outlive it (see
//! [`SlotKey`]).
use super::{sdk_cache::Package, tool, Path};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs, io,
    path::{Component, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        LazyLock, Mutex, PoisonError,
    },
};
use sunrise_launcher_smod::{
    index::{library_identity::LibraryIdentity, sha256, sha256_file},
    manifest::Manifest,
};

/// An attempt's folder: this prefix and 6 random characters. Short, because Windows' path limit
/// counts every folder above the deepest file the SDK's build writes (#396).
pub(crate) const ATTEMPT_PREFIX: &str = ".nb-";
/// The folder launchers before 1.3.1 named attempts with, still found and counted on disk.
pub(crate) const OLD_ATTEMPT_PREFIX: &str = ".native-attempt-";
/// The SDK's work folder inside an attempt.
const WORK: &str = "w";
/// The deepest file a Windows build writes under the work folder, as #396 found it: rustc's
/// import definition for a raw-dylib DLL, in Cargo's target folder under the SDK's build key.
const DEEPEST: &str = r"prebuilt-build\94256510f74e-f618d2e4182eb19e\target\x86_64-pc-windows-gnu\release\deps\rustcgsxNAs\api-ms-win-core-backgroundtask-l1-1-0.dll_imports_indirect.def";
/// Windows' `MAX_PATH`, its terminating NUL included, which mingw's `dlltool` doesn't get around.
const MAX_PATH: usize = 260;

/// How long the deepest path of a build in `attempt` would be, when it would pass Windows' limit.
pub(super) fn too_deep(attempt: &Path) -> Option<usize> {
    too_long(&attempt.join(WORK))
}

/// The same for a build whose SDK work folder is `work`.
fn too_long(work: &Path) -> Option<usize> {
    let length = work.as_os_str().len() + 1 + DEEPEST.len();
    (length >= MAX_PATH).then_some(length)
}

/// How many hex characters of a game build's digest name its persistent work folder.
///
/// Derivation: SHA-256 over the four keying inputs joined by NUL bytes,
/// `<commit>\0<triple>\0<side>\0<rust version>`, written as lowercase hex, first 10 characters
/// (40 bits: collisions among the few builds one player keeps are not a concern, and a collision
/// would only share a folder whose contents Cargo's fingerprints already check).
///
/// Length arithmetic: the slot is `native\w\<10 hex>`, 6 + 1 + 1 + 1 + 10 = 19 characters below
/// the build folder, exactly the attempt's own `native\.nb-XXXXXX\w`, 6 + 1 + 10 + 1 + 1 = 19.
/// So the deepest path is 19 + 1 + `DEEPEST` (161) = 181 characters below the build folder, as
/// it is in an attempt's own folder, and the 35-character Windows user name that fits under
/// `MAX_PATH` (#396) still fits. A longer, readable slot name would shrink that budget.
const WORK_SLOT_HEX: usize = 10;

/// Game builds whose work folder was left with compiler workers not proved stopped, in this
/// launcher run only. Nothing is written to disk: the next start reuses the folder as usual.
static UNPROVED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Mutex::default);

/// Even another slot must not use the shared Cargo home after an unproved compiler exit.
static CARGO_HOME_UNPROVED: AtomicBool = AtomicBool::new(false);

pub(super) fn cargo_home_unproved() -> bool {
    CARGO_HOME_UNPROVED.load(Ordering::Relaxed)
}

/// Which game build a persistent work folder belongs to: its commit, the Rust target, the side
/// and the Rust version. Never a mod set or a profile: Cargo's own fingerprints decide what a
/// different set rebuilds, and every set on one game build shares the folder. The folder's name
/// is a short digest of those four inputs (see `WORK_SLOT_HEX`), so [`Self::describe`] says
/// what it stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SlotKey {
    dir: String,
    revision: String,
    triple: String,
    side: &'static str,
    rust: String,
}

impl SlotKey {
    pub(super) fn new(revision: &str, triple: &str, target: super::Target) -> Option<Self> {
        Self::keyed(revision, triple, target, crate::rust_toolchain::VERSION)
    }

    /// None unless every part is fit to be one folder name.
    fn keyed(revision: &str, triple: &str, target: super::Target, rust: &str) -> Option<Self> {
        let name = |text: &str| {
            !text.is_empty()
                && text
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        };
        (super::is_revision(revision) && name(triple) && name(rust)).then(|| {
            let side = target.name();
            let digest = sha256(format!("{revision}\0{triple}\0{side}\0{rust}").as_bytes());
            Self {
                dir: digest[..WORK_SLOT_HEX].to_owned(),
                revision: revision.to_owned(),
                triple: triple.to_owned(),
                side,
                rust: rust.to_owned(),
            }
        })
    }

    /// The slot only a build the launcher can trust to be left alone uses: not a developer's own
    /// SDK, whose unpinned checkout would mix into a signed build's folder, and not a game build
    /// whose last compiler tree was not proved gone in this run.
    pub(super) fn usable(self, local_sdk: bool) -> Option<Self> {
        (!local_sdk && !self.is_unproved()).then_some(self)
    }

    /// The slot's folder name, `native/w/<name>`.
    pub(super) fn as_str(&self) -> &str {
        &self.dir
    }

    /// The slot and the four inputs it stands for, for the build log. No path.
    pub(super) fn describe(&self) -> String {
        format!(
            "{} (commit={} triple={} side={} rust={})",
            self.dir, self.revision, self.triple, self.side, self.rust
        )
    }

    fn is_unproved(&self) -> bool {
        UNPROVED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&self.dir)
    }

    /// What the next launcher start does: the slot is used again.
    #[cfg(test)]
    fn forget_unproved(&self) {
        UNPROVED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.dir);
    }
}

/// The persistent folders a build compiles in, in place of the attempt's own.
struct Slot {
    key: SlotKey,
    work: PathBuf,
    shared_cargo: Option<PathBuf>,
    /// Something was in the work folder before this build: a failure there may be its leftovers.
    warm: bool,
}

pub(super) struct Workspace {
    dir: tempfile::TempDir,
    slot: Option<Slot>,
    retry_tmp: Option<PathBuf>,
    /// No compiler worker of this attempt may still be running: true until one starts, and
    /// again once an OS proof says the tree is gone.
    workers_stopped: bool,
}

/// One SDK library, with the exact basename it is installed under and the verified
/// bytes. A registry symlink alias is copied as a regular file under its original name.
#[derive(Debug)]
pub(super) struct Library {
    pub(super) name: String,
    pub(super) source: std::path::PathBuf,
    pub(super) sha256: String,
}

impl Library {
    pub(super) fn copy_into(&self, destination: &Path) -> io::Result<()> {
        let output = destination.join(&self.name);
        fs::copy(&self.source, &output)?;
        if sha256_file(&output).is_ok_and(|digest| digest == self.sha256) {
            Ok(())
        } else {
            Err(invalid())
        }
    }
}

impl Workspace {
    pub(super) fn new(parent: &Path) -> io::Result<Self> {
        let parent = crate::backend::plain(parent.canonicalize()?);
        tempfile::Builder::new()
            .prefix(ATTEMPT_PREFIX)
            .rand_bytes(6)
            .tempdir_in(parent)
            .map(|dir| Self {
                dir,
                slot: None,
                retry_tmp: None,
                workers_stopped: true,
            })
    }

    pub(super) fn path(&self) -> &Path {
        self.dir.path()
    }

    /// The folder the SDK builds in: the game build's persistent one, else the attempt's own.
    pub(super) fn work(&self) -> PathBuf {
        self.slot
            .as_ref()
            .map_or_else(|| self.path().join(WORK), |slot| slot.work.clone())
    }

    /// Cargo's home: the launcher's own `cargo/` for a persistent build, else the attempt's.
    pub(super) fn cargo_home(&self) -> PathBuf {
        self.slot
            .as_ref()
            .and_then(|slot| slot.shared_cargo.clone())
            .unwrap_or_else(|| self.path().join("cargo"))
    }

    /// Moves the SDK's work folder and Cargo's home out of the attempt into `key`'s slot and the
    /// launcher's `cargo/`, both checked link-free below `lease`. False, with nothing moved,
    /// when the slot's deepest build path would pass Windows' limit (#396).
    pub(super) fn attach(
        &mut self,
        lease: &crate::native_cache::Cache,
        key: SlotKey,
    ) -> Result<bool, crate::native_cache::Error> {
        // Checked before the folder is made, so an unusable slot leaves no empty one behind.
        if cfg!(windows) && too_long(&lease.work_slot_path(key.as_str())).is_some() {
            return Ok(false);
        }
        let work = lease.work_slot(key.as_str())?;
        let shared_cargo = (!CARGO_HOME_UNPROVED.load(Ordering::Relaxed))
            .then(|| lease.cargo_home())
            .transpose()?;
        self.slot = Some(Slot {
            key,
            work: work.path,
            shared_cargo,
            warm: !work.cold,
        });
        Ok(true)
    }

    /// Empties the slot a failed build ran in, for one cold build: only after a proved stop of
    /// the compiler, only when the slot held something before, and so at most once. False when
    /// any of that doesn't hold; nothing is removed then.
    pub(super) fn retry_cold(
        &mut self,
        lease: &crate::native_cache::Cache,
    ) -> Result<bool, crate::native_cache::Error> {
        let stopped = self.workers_stopped;
        let parent = self.dir.path();
        let Some(slot) = self.slot.as_mut().filter(|slot| slot.warm && stopped) else {
            return Ok(false);
        };
        slot.work = lease.fresh_work_slot(slot.key.as_str())?.path;
        self.retry_tmp = Some(
            tempfile::Builder::new()
                .prefix("tmp-")
                .tempdir_in(parent)?
                .keep(),
        );
        slot.warm = false;
        Ok(true)
    }

    /// After an attempt: a persistent folder whose compiler tree was not proved gone is not
    /// used again in this launcher run (builds take their own folders), and nothing is written.
    pub(super) fn note_unproved(&self) {
        if self.workers_stopped {
            return;
        }
        CARGO_HOME_UNPROVED.store(true, Ordering::Relaxed);
        if let Some(slot) = &self.slot {
            UNPROVED
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(slot.key.dir.clone());
        }
    }

    pub(super) fn isolate(&self, command: &mut std::process::Command) -> io::Result<()> {
        if self.slot.is_some() {
            remove_slot_log(&self.work())?;
        }
        let path = self.path();
        let tmp = self.retry_tmp.clone().unwrap_or_else(|| path.join("tmp"));
        fs::create_dir_all(&tmp)?;
        let cargo_home = self.cargo_home();
        fs::create_dir_all(&cargo_home)?;
        command
            .env("CARGO_HOME", cargo_home)
            .env("TMPDIR", &tmp)
            .env("TMP", &tmp)
            .env("TEMP", &tmp);
        if self.slot.is_some() {
            // The SDK gives Cargo its own target folder inside the work folder.
            command.env_remove("CARGO_TARGET_DIR");
        } else {
            fs::create_dir_all(path.join("target"))?;
            command.env("CARGO_TARGET_DIR", path.join("target"));
        }
        Ok(())
    }

    /// From here on an escaped descendant can outlive the launcher or SDK. It can keep its
    /// private paths; ordinary retry and recovery must not unlink or reuse them.
    pub(super) fn allow_worker(&mut self) {
        self.workers_stopped = false;
        self.dir.disable_cleanup(true);
    }

    /// A terminal OS proof, not successful compilation by itself, permits cleanup.
    pub(super) fn worker_finished(&mut self) {
        self.workers_stopped = true;
        self.dir.disable_cleanup(false);
    }

    /// The installed SDK inputs a compiler gets. A persistent build is pointed straight at the
    /// kept slices, read-only by agreement with the tool, once each is a link-free folder. Any
    /// other copies them, never giving a compiler links into the shared cache.
    pub(super) fn package(&self, package: &Package) -> io::Result<Package> {
        if self.slot.is_some() {
            for slice in [&package.runtime, &package.api, &package.engine] {
                let unit = slice.parent().ok_or_else(invalid)?;
                for folder in [unit, slice.as_path()] {
                    crate::native_cache::checked(folder, true).map_err(|_| invalid())?;
                }
            }
            return Ok(package.clone());
        }
        let root = self.path().join("package");
        fs::create_dir(&root)?;
        let private = Package::at(&root);
        for (from, to) in [
            (&package.runtime, &private.runtime),
            (&package.api, &private.api),
            (&package.engine, &private.engine),
        ] {
            match fs::symlink_metadata(from) {
                Ok(meta) if meta.is_dir() && !crate::native_cache::linked(&meta) => {
                    copy_slice(from, to)?;
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(err),
                Ok(_) => return Err(invalid()),
            }
        }
        Ok(private)
    }

    pub(super) fn import_log(&self) -> io::Result<String> {
        let work = self.work();
        let log = work.join("build.log");
        match regular_under(&work, &log) {
            Ok(()) => Ok(tool::tail(fs::File::open(log)?)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(String::new()),
            Err(err) => Err(err),
        }
    }

    /// Bind the exact SDK path and bytes of every published library, including ordinary unstamped
    /// dependencies. Only the registry route can add Cargo output or Cargo-home libraries. Each
    /// kind of file is checked against the one root it can belong to, never a folder above them
    /// all: the SDK runtime slice, the work folder, Cargo's home, or the toolchain's own folders.
    /// Nothing here may run before the compiler tree is proved gone.
    pub(super) fn verify_outputs<'a>(
        &self,
        built: &tool::Built,
        package: &Package,
        rust: &crate::rust_toolchain::Toolchain,
        expected_route: Option<tool::CheckRoute>,
        revision: &str,
        mods: impl Iterator<Item = &'a Manifest>,
    ) -> io::Result<Vec<Library>> {
        let work = self.work();
        if expected_route.is_some_and(|route| route != built.route) {
            return Err(invalid());
        }
        // Both SDK runtime slices use this filename; installation renames the server image.
        let binary = crate::backend::plain(built.binary.clone());
        let whole = work.join("prebuilt-package");
        let runtime_binary = binary == package.runtime.join("bin").join(crate::backend::CLIENT)
            && regular_under(&package.runtime, &binary).is_ok();
        let joined_binary = binary == whole.join("bin").join(crate::backend::CLIENT)
            && regular_under(&work, &binary).is_ok();
        match built.route {
            tool::CheckRoute::PrebuiltApi if !runtime_binary => return Err(invalid()),
            tool::CheckRoute::Prebuilt if !runtime_binary && !joined_binary => {
                return Err(invalid())
            }
            _ => (),
        }
        let wanted: BTreeMap<_, _> = mods.map(|m| (m.id.as_str(), m.version.as_str())).collect();
        let used: BTreeSet<_> = built.mods.iter().map(String::as_str).collect();
        if used.len() != built.mods.len()
            || !used.iter().copied().eq(wanted.keys().copied())
            || built.libraries.is_empty()
        {
            return Err(invalid());
        }
        let std_lib = rust.target_libdir();
        let mingw = rust.mingw_bin();
        // Registry compilation returns the merged package's executable and places the
        // stamped patch in <target>/<triple>/release. The latter binds the one allowed
        // Cargo target root without recalculating the SDK's private build key.
        let registry_target = if built.route == tool::CheckRoute::Prebuilt && joined_binary {
            let patch = built
                .records
                .iter()
                .find(|row| matches!(row.kind, tool::LibraryKind::Patch))
                .ok_or_else(invalid)?;
            let patch = built
                .libraries
                .iter()
                .find(|file| {
                    file.file_name()
                        .is_some_and(|name| name == patch.file.as_str())
                })
                .ok_or_else(invalid)?;
            let patch = crate::backend::plain(patch.clone());
            let builds = work.join("prebuilt-build");
            let release = patch
                .strip_prefix(&builds)
                .ok()
                .and_then(Path::parent)
                .ok_or_else(invalid)?;
            let folders: Vec<_> = release.components().collect();
            match folders.as_slice() {
                [Component::Normal(key), Component::Normal(target), Component::Normal(triple), Component::Normal(release)]
                    if *target == "target"
                        && *triple == rust.host.target
                        && *release == "release" =>
                {
                    Some(builds.join(key).join("target"))
                }
                _ => return Err(invalid()),
            }
        } else {
            None
        };
        let cargo_home = self.cargo_home();
        let joined_runtime = whole.join("deps");
        let stamped_names: BTreeSet<_> =
            built.records.iter().map(|row| row.file.as_str()).collect();
        let mut files = BTreeMap::new();
        for supplied in &built.libraries {
            // SDK directory scans can return canonical-prefixed Windows paths. The plan
            // otherwise uses the launcher's plain form; this does not resolve any links.
            let file = crate::backend::plain(supplied.clone());
            let name = file
                .file_name()
                .and_then(|s| s.to_str())
                .and_then(crate::browser::safe_name)
                .ok_or_else(invalid)?;
            let dynamic = [".so", ".dll", ".dylib"]
                .iter()
                .any(|suffix| name.ends_with(suffix));
            let versioned_so = name
                .rsplit_once(".so.")
                .is_some_and(|(base, version)| !base.is_empty() && !version.is_empty());
            let stamped = stamped_names.contains(name);
            let dependency = !stamped && registry_target.is_some();
            if !dynamic && !(dependency && versioned_so) {
                return Err(invalid());
            }
            let runtime = || {
                file.starts_with(&package.runtime) && regular_under(&package.runtime, &file).is_ok()
            };
            let standard = || {
                (name.starts_with("libstd-") || name.starts_with("std-"))
                    && std_lib.as_ref().is_some_and(|dir| {
                        file == dir.join(name) && regular_under(dir, &file).is_ok()
                    })
            };
            let gnu = || {
                matches!(name, "libgcc_s_seh-1.dll" | "libwinpthread-1.dll")
                    && file == mingw.join(name)
                    && regular_under(&mingw, &file).is_ok()
            };
            let source = if stamped {
                regular_under(&work, &file)?;
                file.clone()
            } else if dynamic
                && ((joined_binary
                    && file.starts_with(&joined_runtime)
                    && regular_under(&work, &file).is_ok())
                    || runtime()
                    || standard()
                    || gnu())
            {
                file.clone()
            } else if let Some(target) = registry_target.as_ref().filter(|_| dependency) {
                registry_library(&work, target, &cargo_home, &file)?
            } else {
                return Err(invalid());
            };
            let sha256 = sha256_file(&source).map_err(io::Error::other)?;
            let library = Library {
                name: name.to_owned(),
                source,
                sha256,
            };
            if files.insert(name.to_ascii_lowercase(), library).is_some() {
                return Err(invalid());
            }
        }
        let mut stamped = BTreeSet::new();
        let mut recorded = BTreeSet::new();
        let mut patch = false;
        // Each library row's identity, by mod id, and the native mods each one says it links.
        let mut identities = BTreeMap::new();
        let mut linked = Vec::new();
        for row in &built.records {
            let Some(file) = files
                .get(&row.file.to_ascii_lowercase())
                .filter(|file| file.name == row.file)
            else {
                return Err(invalid());
            };
            let id = row.mod_id.as_str();
            let version = match row.kind {
                tool::LibraryKind::Mod if recorded.insert(id) => {
                    wanted.get(id).copied().ok_or_else(invalid)?
                }
                tool::LibraryKind::Patch
                    if built.route == tool::CheckRoute::Prebuilt
                        && id == "sunrise_patch"
                        && !patch =>
                {
                    patch = true;
                    "0"
                }
                _ => return Err(invalid()),
            };
            let rust = crate::rust_toolchain::VERSION;
            // Since sunrise#5136 the identity also names the mod's files and the identity of each
            // native mod the library links; SDKs from before it stamp only the five fields.
            let identity_matches = match LibraryIdentity::parse(&row.identity) {
                Some(identity) => {
                    linked.push(identity.dependencies().ok_or_else(invalid)?);
                    identity.build == revision
                        && identity.id == id
                        && identity.version == version
                        && identity.rust == rust
                        && identity.panic == "unwind"
                }
                None => row.identity == format!("{revision} {id} {version} {rust} unwind"),
            };
            if !stamped.insert(&row.file) || !identity_matches || file.sha256 != row.sha256 {
                return Err(invalid());
            }
            if matches!(row.kind, tool::LibraryKind::Mod) {
                identities.insert(id, row.identity.as_str());
            }
        }
        if recorded != used {
            return Err(invalid());
        }
        // A library links only mods of this build, each as the identity this build recorded for it.
        for (id, digest) in linked.into_iter().flatten() {
            if identities
                .get(id)
                .is_none_or(|identity| sha256(identity.as_bytes()) != digest)
            {
                return Err(invalid());
            }
        }
        Ok(files.into_values().collect())
    }
}

fn copy_slice(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let meta = fs::symlink_metadata(entry.path())?;
        if crate::native_cache::linked(&meta) {
            return Err(invalid());
        }
        let to = to.join(entry.file_name());
        if meta.is_dir() {
            copy_slice(&entry.path(), &to)?;
        } else if meta.is_file() {
            crate::preparer::clone_or_copy(&entry.path(), &to)?;
            // The tool names its build folder from each slice file's path, length and modified
            // time (`kept_build`). A copy stamped now (Linux's copy and clone) would name another
            // folder every attempt, and no kept library would ever be found there.
            times_handle(&to)?.set_modified(meta.modified()?)?;
        } else {
            return Err(invalid());
        }
    }
    Ok(())
}

/// A handle that may set `file`'s times even when the file is read-only: on Windows it asks
/// for attribute writes only, elsewhere setting times needs ownership, not write permission.
fn times_handle(file: &Path) -> io::Result<fs::File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_WRITE_ATTRIBUTES
        fs::File::options().access_mode(0x100).open(file)
    }
    #[cfg(not(windows))]
    {
        fs::File::open(file)
    }
}

fn remove_slot_log(work: &Path) -> io::Result<()> {
    let dir = match sunrise_launcher_smod::runtime_leases::open_directory(work) {
        Ok(dir) => dir,
        Err(sunrise_launcher_smod::runtime_leases::Error::Io(err))
            if err.kind() == io::ErrorKind::NotFound =>
        {
            return Ok(());
        }
        Err(sunrise_launcher_smod::runtime_leases::Error::Io(err)) => return Err(err),
        Err(_) => return Err(invalid()),
    };
    let meta = match dir.symlink_metadata("build.log") {
        Ok(meta) => meta,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    if crate::native_cache::cap_linked(&meta) || !meta.is_file() {
        return Err(invalid());
    }
    dir.remove_file("build.log")
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid native SDK output")
}

// The SDK preserves linked library basenames (e.g. libx.so -> libx.so.1). Only a
// final file symlink within the registry's exact roots is allowed: Cargo's target folder
// (checked from the work folder down) or Cargo's home. Keep the resolved checked source, not
// the symlink, and verify the published copy's digest again.
fn registry_library(
    work: &Path,
    target: &Path,
    cargo_home: &Path,
    path: &Path,
) -> io::Result<PathBuf> {
    // The folder each place is checked from, component by component.
    let root = |file: &Path| {
        if file.starts_with(target) {
            Some(work)
        } else if file.starts_with(cargo_home) {
            Some(cargo_home)
        } else {
            None
        }
    };
    let parent = path.parent().ok_or_else(invalid)?;
    let from = root(path).ok_or_else(invalid)?;
    if parent != from {
        checked_under(from, parent, true)?;
    }
    let source = if fs::symlink_metadata(path)?.file_type().is_symlink() {
        crate::backend::plain(path.canonicalize()?)
    } else {
        path.to_owned()
    };
    regular_under(root(&source).ok_or_else(invalid)?, &source)?;
    Ok(source)
}

// Refuse reparse points and links at every component, including the root. The compiler never
// receives the installation destination; the launcher copies these bytes out of the work tree.
fn regular_under(root: &Path, file: &Path) -> io::Result<()> {
    checked_under(root, file, false)
}

fn checked_under(root: &Path, file: &Path, directory: bool) -> io::Result<()> {
    let tail = file.strip_prefix(root).map_err(|_| invalid())?;
    let mut path = root.to_path_buf();
    let mut parts = tail.components().peekable();
    if parts.peek().is_none() {
        return Err(invalid());
    }
    let meta = fs::symlink_metadata(root)?;
    if !meta.is_dir() || crate::native_cache::linked(&meta) {
        return Err(invalid());
    }
    while let Some(std::path::Component::Normal(part)) = parts.next() {
        path.push(part);
        let meta = fs::symlink_metadata(&path)?;
        if crate::native_cache::linked(&meta) {
            return Err(invalid());
        }
        if parts.peek().is_none() {
            return if (directory && meta.is_dir()) || (!directory && meta.is_file()) {
                Ok(())
            } else {
                Err(invalid())
            };
        }
        if !meta.is_dir() {
            return Err(invalid());
        }
    }
    Err(invalid())
}

#[cfg(test)]
mod tests;
