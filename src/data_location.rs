//! Offline, copy-first relocation. The fixed bootstrap is never resolved through Settings.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

use crate::i18n::{tr, trf};
#[cfg(windows)]
use crate::windows_runtime::wide;

type Result<T> = std::result::Result<T, String>;

mod root_leases;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Roots {
    pub launcher: PathBuf,
    pub game: PathBuf,
    pub build: PathBuf,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Location {
    version: u32,
    roots: Roots,
}

pub fn bootstrap_path() -> Result<PathBuf> {
    // Portable installs and tests can keep their location pointer separate from the user profile.
    if let Some(path) = std::env::var_os("SUNRISE_LAUNCHER_LOCATION_FILE").filter(|p| !p.is_empty())
    {
        return pointer_path(Path::new(&path));
    }
    if cfg!(test) {
        return Ok(test_home().join(crate::paths::LOCATION));
    }
    dirs::config_dir()
        .map(|p| {
            p.join(crate::identity::LOCATION_FOLDER)
                .join(crate::paths::LOCATION)
        })
        .ok_or_else(|| {
            tr("Cannot locate the platform configuration folder for the launcher location file")
                .into()
        })
}

/// Where the library's own tests keep the launcher's and the game's data (`launcher`, `game`,
/// `location.json` inside) unless `SUNRISE_LAUNCHER_HOME` or `SUNRISE_HOME` name another:
/// never a player's install. A fixed path, not an environment change, so it holds from the
/// first call on any thread, and a test that runs its own binary again shares it.
///
/// The temporary folder in its real spelling, as [`crate::backend::build_dir`] resolves it:
/// Windows can name it by its 8.3 short form (`C:\Users\RUNNER~1\…`), so every test path
/// compares under one spelling.
pub fn test_home() -> PathBuf {
    let temp = std::env::temp_dir();
    crate::backend::plain(temp.canonicalize().unwrap_or(temp)).join("sunrise-launcher-lib-tests")
}

fn overridden(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| !v.is_empty())
}

fn has_override() -> bool {
    overridden("SUNRISE_LAUNCHER_HOME") || overridden("SUNRISE_HOME")
}

/// Roots resolved once per process. A repaired pointer takes effect after restart.
static LOCATION: Mutex<Option<Result<Option<Roots>>>> = Mutex::new(None);
/// A startup failure falls back to the default folders, so a window can still open.
static FORCE_DEFAULT: AtomicBool = AtomicBool::new(false);
static STARTUP_ERROR: Mutex<Option<String>> = Mutex::new(None);
static STARTUP_LEASES: Mutex<Option<[MigrationLease; 3]>> = Mutex::new(None);
static ADMITTED_BUILD: OnceLock<PathBuf> = OnceLock::new();

pub(crate) fn admitted_build_root() -> Option<&'static Path> {
    ADMITTED_BUILD.get().map(PathBuf::as_path)
}

fn location() -> std::sync::MutexGuard<'static, Option<Result<Option<Roots>>>> {
    LOCATION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn open_error(error: &str) -> String {
    trf("Cannot open the data folder: {error}", &[("error", error)])
}

fn remember_startup_error(error: Option<String>) {
    *STARTUP_ERROR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = error;
}

/// Stable for the lifetime of this process, including errors. Environment overrides are
/// applied per root by the caller; fully isolated callers need not read the real bootstrap.
pub fn configured_roots() -> Result<Option<Roots>> {
    if FORCE_DEFAULT.load(Ordering::Acquire) {
        return Ok(None);
    }
    // A unit test never reaches the player's real folders: unless it points at a location file
    // of its own, its roots are this test process's, under the temporary folder.
    if cfg!(test) && !overridden("SUNRISE_LAUNCHER_LOCATION_FILE") {
        let temp = std::env::temp_dir();
        let home = plain_drive(temp.canonicalize().unwrap_or(temp))
            .join(format!("sunrise-launcher-test-{}", std::process::id()));
        return Ok(Some(Roots {
            launcher: home.join("launcher"),
            game: home.join("game"),
            build: home.join("launcher"),
        }));
    }
    let mut slot = location();
    if slot.is_none() {
        let resolved = if overridden("SUNRISE_LAUNCHER_HOME") && overridden("SUNRISE_HOME") {
            Ok(None)
        } else {
            match bootstrap_path() {
                Ok(bootstrap) => resolve_at(&bootstrap),
                Err(error) => Err(error),
            }
        };
        *slot = Some(resolved);
    }
    slot.as_ref().expect("roots are resolved above").clone()
}

pub fn validate() -> Result<()> {
    configured_roots().map(|_| ())
}

/// Acquire the shared location lease before reading the pointer or dispatching anything that
/// can write. A broken pointer can still open Settings on default roots while holding that lease.
pub fn startup() -> Result<()> {
    let pointer = acquire_lock_at(&bootstrap_path()?)?;
    let problem = configured_roots().err().map(|error| {
        FORCE_DEFAULT.store(true, Ordering::Release);
        open_error(&error)
    });
    let registry = root_leases::Registry::open()?;
    let data = registry.acquire(
        &[&crate::backend::data_dir(), &crate::backend::game_home()],
        false,
    )?;
    // A Windows cache fallback can create its directory; admit the candidate first.
    let (build, build_lease) =
        crate::backend::admit_build_dir(|path| registry.acquire(&[path], false))?;
    let _ = ADMITTED_BUILD.set(build);
    *STARTUP_LEASES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some([pointer, data, build_lease]);
    remember_startup_error(problem);
    Ok(())
}

pub fn startup_error() -> Option<String> {
    STARTUP_ERROR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Try the configured location again. Success requires a restart: paths already resolved to
/// the default folders are not switched under a running window.
pub fn retry() -> Result<()> {
    resolve_at(&bootstrap_path()?).map(|_| ())
}

/// The error `startup` would report for one bootstrap; the UI opens with it instead of
/// the launcher exiting.
pub fn startup_error_at(bootstrap: &Path) -> Option<String> {
    resolve_at(bootstrap).err().map(|error| open_error(&error))
}

/// Forget the pointer and keep the default folders. The old folders are untouched and the
/// pointer is kept beside it as `location.json.old`, so it can be restored by hand.
pub fn use_default_folder() -> Result<()> {
    use_default_at(&bootstrap_path()?)
}

fn use_default_at(bootstrap: &Path) -> Result<()> {
    if bootstrap.exists() {
        let backup = bootstrap.with_file_name("location.json.old");
        if backup.exists() {
            fs::remove_file(&backup).map_err(|e| problem(&backup, e))?;
        }
        fs::rename(bootstrap, &backup).map_err(|e| problem(bootstrap, e))?;
    }
    Ok(())
}

/// Explicit paths make tests independent of the real user's bootstrap and environment.
pub fn resolve_at(bootstrap: &Path) -> Result<Option<Roots>> {
    pointer_path(bootstrap)?;
    let bytes = match fs::read(bootstrap) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(problem(bootstrap, e)),
    };
    let location: Location = serde_json::from_slice(&bytes).map_err(|e| {
        trf(
            "Invalid {path}: {error}; restore this file or the previous data folder",
            &[
                ("path", &bootstrap.display().to_string()),
                ("error", &e.to_string()),
            ],
        )
    })?;
    if location.version != 1 {
        return Err(trf(
            "Unsupported data location version {version} in {path}",
            &[
                ("version", &location.version.to_string()),
                ("path", &bootstrap.display().to_string()),
            ],
        ));
    }
    let canonical = validate_roots(&location.roots)?;
    tool_safe(&canonical.build)?;
    for root in [
        &location.roots.launcher,
        &location.roots.game,
        &location.roots.build,
    ] {
        if !root.is_dir() {
            return Err(trf(
                "Data folder {path} is missing; reconnect its drive or restore location.json",
                &[("path", &root.display().to_string())],
            ));
        }
    }
    Ok(Some(canonical))
}

/// Keeps the bootstrap lock held until the lease is dropped.
pub struct MigrationLease(File);

impl Drop for MigrationLease {
    fn drop(&mut self) {
        // A concurrent Unix fork can hold the same file description until exec, even
        // with close-on-exec set. Release the lock without waiting for that copy to close.
        let _ = self.0.unlock();
    }
}

pub fn acquire_lock_at(bootstrap: &Path) -> Result<MigrationLease> {
    lock_at(bootstrap, false)
}

fn lock_at(bootstrap: &Path, exclusive: bool) -> Result<MigrationLease> {
    let bootstrap = &pointer_path(bootstrap)?;
    let parent = bootstrap
        .parent()
        .ok_or_else(|| tr("Bootstrap needs a parent folder").to_string())?;
    fs::create_dir_all(parent).map_err(|e| problem(parent, e))?;
    lock_file(&parent.join("migration.lock"), exclusive)
}

fn open_lock_file(path: &Path) -> Result<File> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !crate::native_cache::linked(&meta) => (),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(problem(path, e)),
        _ => return Err(problem(path, crate::i18n::tr("Not a regular lock file"))),
    }
    let mut open = OpenOptions::new();
    open.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        open.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
        open.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
    }
    open.open(path).map_err(|e| problem(path, e))
}

fn lock_file(path: &Path, exclusive: bool) -> Result<MigrationLease> {
    let file = open_lock_file(path)?;
    let result = if exclusive {
        file.try_lock()
    } else {
        file.try_lock_shared()
    };
    result.map_err(|error| {
        crate::i18n::trf(
            "Cannot lock {path}: {error}; close other launchers and stop games/builds before retrying",
            &[("path", &path.to_string_lossy()), ("error", &error.to_string())],
        )
    })?;
    Ok(MigrationLease(file))
}

/// Acquire an exclusive blocking OS lock on `path` using the same no-follow lock file rules.
pub(crate) fn lock_exclusive_blocking(path: &Path) -> Result<MigrationLease> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| problem(parent, e))?;
    }
    let file = open_lock_file(path)?;
    file.lock().map_err(|error| problem(path, error))?;
    Ok(MigrationLease(file))
}

pub fn migrate_current(source: &Roots, target: &Path, progress: impl FnMut(&str)) -> Result<Roots> {
    migrate_current_with_prepare(source, target, progress, |_, _| Ok(()))
}

/// Called only by the offline helper, after the GUI requests exit. Wait briefly for its lease.
pub fn migrate_current_with_prepare(
    source: &Roots,
    target: &Path,
    progress: impl FnMut(&str),
    prepare: impl FnOnce(&Roots, &Roots) -> Result<()>,
) -> Result<Roots> {
    if has_override() {
        return Err(tr("Unset SUNRISE_LAUNCHER_HOME and SUNRISE_HOME before moving data").into());
    }
    let bootstrap = bootstrap_path()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let lease = loop {
        match lock_at(&bootstrap, true) {
            Ok(lease) => break lease,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e),
        }
    };
    migrate_locked(source, target, &bootstrap, progress, prepare, &lease)
}

pub fn migrate(
    source: &Roots,
    target: &Path,
    bootstrap: &Path,
    progress: impl FnMut(&str),
) -> Result<Roots> {
    migrate_with_prepare(source, target, bootstrap, progress, |_, _| Ok(()))
}

/// prepare receives (staging roots, final roots), after byte verification and before commit.
/// It may rewrite managed absolute paths in structured files, and must sync those writes.
/// This explicit API deliberately ignores environment variables; production uses migrate_current.
pub fn migrate_with_prepare(
    source: &Roots,
    target: &Path,
    bootstrap: &Path,
    progress: impl FnMut(&str),
    prepare: impl FnOnce(&Roots, &Roots) -> Result<()>,
) -> Result<Roots> {
    let lease = lock_at(bootstrap, true)?;
    migrate_locked(source, target, bootstrap, progress, prepare, &lease)
}

fn migrate_locked(
    source: &Roots,
    target: &Path,
    bootstrap: &Path,
    mut progress: impl FnMut(&str),
    prepare: impl FnOnce(&Roots, &Roots) -> Result<()>,
    _lease: &MigrationLease,
) -> Result<Roots> {
    let source = validate_roots(source)?;
    if let Some(current) = resolve_at(bootstrap)? {
        if current != source {
            return Err(tr(
                "Data location changed while waiting; restart the launcher and retry the move",
            )
            .into());
        }
    }
    let target = checked_path(target)?;
    for root in [
        &source.launcher,
        &source.game,
        &source.build,
        &pointer_path(bootstrap)?,
    ] {
        if overlaps(root, &target) {
            return Err(trf(
                "Destination {target} overlaps existing data or its bootstrap {bootstrap}",
                &[
                    ("target", &target.display().to_string()),
                    ("bootstrap", &root.display().to_string()),
                ],
            ));
        }
    }
    let destination = layout(&source, &target);
    tool_safe(&destination.build)?;
    let registry = root_leases::Registry::open()?;
    // Reserve both sides by containment before checking the target or settling the source.
    // No independent launcher may publish into our destination while it is staged.
    let _roots = registry.acquire(
        &[&source.launcher, &source.game, &source.build, &target],
        true,
    )?;
    if target.exists()
        && (!target.is_dir()
            || fs::read_dir(&target)
                .map_err(|e| problem(&target, e))?
                .next()
                .is_some())
    {
        return Err(trf(
            "Destination {path} must be an empty folder",
            &[("path", &target.display().to_string())],
        ));
    }
    // Canonical spellings are only for excluding controls from the copied tree. Commit
    // to the caller's bootstrap, and report that same path if replacement is refused.
    let mut controls = [
        bootstrap.to_owned(),
        bootstrap.with_file_name("migration.lock"),
        bootstrap.with_file_name("location.json.new"),
    ]
    .iter()
    .map(|path| pointer_path(path))
    .collect::<Result<Vec<_>>>()?;
    controls.push(registry.path().to_owned());
    // Both pointer and source leases cover recovery and the entire snapshot/commit.
    crate::relocation::admit_idle_profiles(&source)?;
    progress(tr("Copying and verifying launcher data"));
    fs::create_dir_all(&target).map_err(|e| problem(&target, e))?;
    let stage = target.join(".relocation-staging");
    let staged = layout(&source, &stage);
    let result = (|| {
        private_dir(&stage)?;
        copy_tree(&source.launcher, &staged.launcher, &controls, &mut progress)?;
        progress(tr("Copying and verifying game data"));
        copy_tree(&source.game, &staged.game, &controls, &mut progress)?;
        if !within(&source.build, &source.launcher) {
            progress(tr("Copying and verifying build data"));
            copy_tree(&source.build, &staged.build, &controls, &mut progress)?;
        } else if !staged.build.exists() {
            fs::create_dir_all(&staged.build).map_err(|e| problem(&staged.build, e))?;
        }
        prepare(&staged, &destination)?;
        crate::managed_play::remap_relocated_root(
            &staged.launcher,
            &source.game,
            &staged.game,
            &destination.game,
        )?;
        for name in ["launcher", "game", "build"] {
            let from = stage.join(name);
            if from.exists() {
                fs::rename(&from, target.join(name)).map_err(|e| problem(&from, e))?;
            }
        }
        fs::remove_dir(&stage).map_err(|e| problem(&stage, e))?;
        sync_dir(&target)?;
        progress(tr("Committing data location"));
        commit(bootstrap, &destination)?;
        Ok(destination)
    })();
    result.map_err(|e: String| {
        trf(
            "{error}. Original data was kept. Partial copies may remain in {path}; choose an empty folder to retry",
            &[("error", &e), ("path", &target.display().to_string())],
        )
    })
}

fn layout(source: &Roots, target: &Path) -> Roots {
    let launcher = target.join("launcher");
    let build = if within(&source.build, &source.launcher) {
        // Windows containment is case-insensitive, while strip_prefix is not.
        launcher.join(
            source
                .build
                .components()
                .skip(source.launcher.components().count())
                .collect::<PathBuf>(),
        )
    } else {
        target.join("build")
    };
    Roots {
        launcher,
        game: target.join("game"),
        build,
    }
}

fn validate_roots(roots: &Roots) -> Result<Roots> {
    let roots = Roots {
        launcher: checked_path(&roots.launcher)?,
        game: checked_path(&roots.game)?,
        build: checked_path(&roots.build)?,
    };
    for root in [&roots.launcher, &roots.game, &roots.build] {
        match fs::metadata(root) {
            Ok(meta) if !meta.is_dir() => {
                return Err(trf(
                    "Data root must be a folder: {path}",
                    &[("path", &root.display().to_string())],
                ))
            }
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(problem(root, e)),
            _ => {}
        }
    }
    if overlaps(&roots.launcher, &roots.game)
        || overlaps(&roots.build, &roots.game)
        || (overlaps(&roots.launcher, &roots.build) && !within(&roots.build, &roots.launcher))
    {
        return Err(tr(
            "Source data roots overlap; only a build folder inside/equal to launcher is supported",
        )
        .into());
    }
    Ok(roots)
}

fn within(path: &Path, parent: &Path) -> bool {
    #[cfg(windows)]
    {
        Path::new(&path.to_string_lossy().to_lowercase())
            .starts_with(parent.to_string_lossy().to_lowercase())
    }
    #[cfg(not(windows))]
    {
        path.starts_with(parent)
    }
}
fn overlaps(a: &Path, b: &Path) -> bool {
    within(a, b) || within(b, a)
}

fn same_path(a: &Path, b: &Path) -> bool {
    within(a, b) && within(b, a)
}

/// Canonicalize existing ancestors, reject links/junctions and ambiguous relative components.
fn checked_path(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(trf(
            "Use an absolute path without '..': {path}",
            &[("path", &path.display().to_string())],
        ));
    }
    let mut current = PathBuf::new();
    let last = path.components().count().saturating_sub(1);
    for (i, part) in path.components().enumerate() {
        current.push(part);
        // A Windows drive/UNC prefix alone is not a filesystem root. Inspect it
        // only after RootDir completes it, including canonical verbatim paths.
        if matches!(part, Component::Prefix(_)) {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(meta) => {
                // Only the root itself may not be a link: the folders above it are never copied
                // (a ~/.local/share linked to another disk, Fedora Atomic's /home).
                if i == last {
                    reject_link(&current, &meta)?;
                }
                current = fs::canonicalize(&current).map_err(|e| problem(&current, e))?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(problem(&current, e)),
        }
    }
    Ok(plain_drive(current))
}

/// `canonicalize`'s verbatim `\\?\C:\…` as the plain drive path the other roots use.
pub(crate) fn plain_drive(path: PathBuf) -> PathBuf {
    if let Some(rest) = path.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        if rest.as_bytes().get(1) == Some(&b':') {
            return PathBuf::from(rest);
        }
    }
    path
}

/// Resolve a real data path without making a virgin folder. Legacy environment overrides
/// also accept relative paths and existing links; their claim must follow the resolved root.
pub(crate) fn canonical_data_path(path: &Path) -> Result<PathBuf> {
    fs::canonicalize(path).map(plain_drive).or_else(|_| {
        pointer_path(&std::path::absolute(path).map_err(|error| problem(path, error))?)
    })
}

/// The pointer and its lock sit in the platform config folder, which may be a link (a
/// dotfiles-managed `~/.config`, Silverblue's `/home`): unlike the data roots, they are never
/// copied, so a link on the way is followed, not refused. Refusing it would report a broken
/// data folder at every start to players who never moved their data.
fn pointer_path(path: &Path) -> Result<PathBuf> {
    let (existing, missing) = resolved_prefix(path)?;
    Ok(missing
        .iter()
        .rev()
        .fold(existing, |path, part| path.join(part)))
}

fn resolved_prefix(path: &Path) -> Result<(PathBuf, Vec<std::ffi::OsString>)> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(trf(
            "Use an absolute path without '..': {path}",
            &[("path", &path.display().to_string())],
        ));
    }
    let mut existing = path;
    let mut rest = Vec::new();
    let mut looked_again = false;
    loop {
        match fs::canonicalize(existing) {
            Ok(real) => return Ok((plain_drive(real), rest)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                match fs::symlink_metadata(existing) {
                    // Made since `canonicalize` looked (another launcher starting): look again.
                    Ok(meta) if !meta.file_type().is_symlink() && !looked_again => {
                        looked_again = true;
                        continue;
                    }
                    Ok(_) => return Err(problem(existing, e)), // Not missing: a dangling link.
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                    Err(error) => return Err(problem(existing, error)),
                }
            }
            Err(e) => return Err(problem(existing, e)),
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                existing = parent;
            }
            _ => return Err(problem(existing, std::io::ErrorKind::NotFound)),
        }
    }
}

fn reject_link(path: &Path, meta: &fs::Metadata) -> Result<()> {
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let reparse = false;
    if meta.file_type().is_symlink() || reparse {
        return Err(trf(
            "Symbolic links and reparse points cannot be relocated: {path}",
            &[("path", &path.display().to_string())],
        ));
    }
    Ok(())
}

/// `path` spelled only in ASCII: what Windows' MinGW-w64 tools can open.
pub(crate) fn ascii(path: &Path) -> bool {
    path.as_os_str().to_str().is_some_and(str::is_ascii)
}

fn tool_safe(path: &Path) -> Result<()> {
    if cfg!(windows)
        && !(ascii(path)
            && !path
                .to_str()
                .is_some_and(|s| s.bytes().any(|b| b.is_ascii_whitespace())))
    {
        return Err(
            "Choose a destination with only ASCII characters and no spaces for Windows build tools"
                .into(),
        );
    }
    Ok(())
}

fn private_dir(path: &Path) -> Result<()> {
    let builder = fs::DirBuilder::new();
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = builder;
        builder.mode(0o700);
        builder
    };
    builder.create(path).map_err(|e| problem(path, e))
}

fn new_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|e| problem(path, e))
}

pub(crate) fn copy_tree(
    source: &Path,
    dest: &Path,
    controls: &[PathBuf],
    progress: &mut impl FnMut(&str),
) -> Result<()> {
    copy_tree_except(source, dest, controls, &[], progress)
}

/// [`copy_tree`], leaving out the entries directly in `source` named in `skip`. The root is
/// refused when it is a link, made private and given the source's access controls first, as in
/// [`copy_tree`]; deeper entries with a name in `skip` are copied.
pub(crate) fn copy_tree_except(
    source: &Path,
    dest: &Path,
    controls: &[PathBuf],
    skip: &[&str],
    progress: &mut impl FnMut(&str),
) -> Result<()> {
    let meta = match fs::symlink_metadata(source) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return private_dir(dest),
        Err(e) => return Err(problem(source, e)),
    };
    reject_link(source, &meta)?;
    if meta.is_dir() {
        private_dir(dest)?;
        copy_acl(source, dest)?;
        for entry in fs::read_dir(source).map_err(|e| problem(source, e))? {
            let entry = entry.map_err(|e| problem(source, e))?;
            if skip.iter().any(|name| entry.file_name() == *name) {
                continue;
            }
            let path = entry.path();
            if controls.iter().any(|p| same_path(p, &path)) {
                continue;
            }
            copy_tree(&path, &dest.join(entry.file_name()), controls, progress)?;
        }
        fs::set_permissions(dest, meta.permissions()).map_err(|e| problem(dest, e))?;
        sync_dir(dest)?;
    } else if meta.is_file() {
        let mut input = File::open(source).map_err(|e| problem(source, e))?;
        let mut output = new_file(dest)?;
        copy_acl(source, dest)?; // Apply the source DACL before writing credentials or other bytes.
        std::io::copy(&mut input, &mut output).map_err(|e| problem(dest, e))?;
        output.sync_all().map_err(|e| problem(dest, e))?;
        if hash(source)? != hash(dest)? {
            return Err(trf(
                "Copy verification failed for {path}",
                &[("path", &source.display().to_string())],
            ));
        }
        let after = fs::symlink_metadata(source).map_err(|e| problem(source, e))?;
        reject_link(source, &after)?;
        if meta.len() != after.len() || meta.modified().ok() != after.modified().ok() {
            return Err(trf(
                "Source changed during copying: {path}; stop all writers and retry",
                &[("path", &source.display().to_string())],
            ));
        }
        fs::set_permissions(dest, meta.permissions()).map_err(|e| problem(dest, e))?;
        progress("Verified file");
    } else {
        return Err(trf(
            "Unsupported special file: {path}",
            &[("path", &source.display().to_string())],
        ));
    }
    Ok(())
}

fn hash(path: &Path) -> Result<[u8; 32]> {
    let mut file = File::open(path).map_err(|e| problem(path, e))?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(|e| problem(path, e))?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(hash.finalize().into())
}

fn commit(bootstrap: &Path, roots: &Roots) -> Result<()> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let temp = bootstrap.with_file_name(format!("location.{}.{nonce}.new", std::process::id()));
    let bytes = serde_json::to_vec_pretty(&Location {
        version: 1,
        roots: roots.clone(),
    })
    .map_err(|e| e.to_string())?;
    let mut file = new_file(&temp)?;
    let written = file
        .write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| problem(&temp, e));
    drop(file);
    let replaced = written.and_then(|_| atomic_replace(&temp, bootstrap));
    if replaced.is_err() {
        let _ = fs::remove_file(&temp);
    }
    replaced?;
    // The pointer is now committed: failure to sync the directory must not report rollback.
    let _ = sync_dir(
        bootstrap
            .parent()
            .ok_or_else(|| tr("Bootstrap needs a parent folder").to_string())?,
    );
    Ok(())
}

pub(crate) fn sync_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        File::open(path)
            .and_then(|f| f.sync_all())
            .map_err(|e| problem(path, e))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace(from: &Path, to: &Path) -> Result<()> {
    fs::rename(from, to).map_err(|e| problem(to, e))
}

#[cfg(windows)]
fn atomic_replace(from: &Path, to: &Path) -> Result<()> {
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
    }
    if unsafe { MoveFileExW(wide(from).as_ptr(), wide(to).as_ptr(), 1 | 8) } == 0 {
        return Err(problem(to, std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(not(windows))]
fn copy_acl(_source: &Path, _dest: &Path) -> Result<()> {
    Ok(())
}

#[cfg(windows)]
fn copy_acl(source: &Path, dest: &Path) -> Result<()> {
    use std::ffi::c_void;
    #[link(name = "advapi32")]
    extern "system" {
        fn GetFileSecurityW(
            path: *const u16,
            info: u32,
            descriptor: *mut c_void,
            length: u32,
            needed: *mut u32,
        ) -> i32;
        fn SetFileSecurityW(path: *const u16, info: u32, descriptor: *const c_void) -> i32;
    }
    let mut needed = 0;
    let source_w = wide(source);
    unsafe {
        GetFileSecurityW(source_w.as_ptr(), 4, std::ptr::null_mut(), 0, &mut needed);
    }
    if needed == 0 {
        return Err(problem(source, std::io::Error::last_os_error()));
    }
    // Aligned storage for the self-relative SECURITY_DESCRIPTOR; protect against target inheritance.
    let mut descriptor = vec![0u32; (needed as usize).div_ceil(4)];
    if unsafe {
        GetFileSecurityW(
            source_w.as_ptr(),
            4,
            descriptor.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    } == 0
    {
        return Err(problem(source, std::io::Error::last_os_error()));
    }
    if unsafe {
        SetFileSecurityW(
            wide(dest).as_ptr(),
            4 | 0x8000_0000,
            descriptor.as_ptr().cast(),
        )
    } == 0
    {
        return Err(trf(
            "Cannot preserve access permissions on {path}: {error}",
            &[
                ("path", &dest.display().to_string()),
                ("error", &std::io::Error::last_os_error().to_string()),
            ],
        ));
    }
    Ok(())
}

fn problem(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}

#[cfg(test)]
mod tests;
