//! Recover the in-game registry's pending publication before the launcher uses its manifest.
//! Mirrors `crates/sunrise-mod-package/src/index/browser_state.rs` in sunrise-org/sunrise.
use super::{MAX_FILE, MAX_INDEX, hex64, sha256_file, valid_id};
use crate::install_lock::lock_home;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::{self, ErrorKind, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

mod claims;
pub mod records;
pub use claims::{ClaimProblem, portable_ascii_component};

pub const MANIFEST: &str = "mod-browser.json";
pub const PENDING: &str = ".mod-browser.pending.json";
// Old and new can come from different indexes. Each row stores the version label twice,
// so allow two index budgets per row, plus space for fixed fields and the journal envelope.
const MAX_PENDING: u64 = 4 * MAX_INDEX + (4 << 10);

#[derive(Deserialize)]
struct Pending {
    id: String,
    file: String,
    old: Option<Value>,
    new: Value,
}

/// Authored reasons are translated by the launcher; system and parser diagnostics stay raw.
#[derive(Debug)]
pub enum DetailReason {
    Diagnostic(String),
    DuplicateRows(String),
    NotRegular,
    PendingTooLarge,
    MissingSha,
    InvalidSha,
    InvalidPending,
    EntriesNotArray,
    ChangedRow,
    NonPortableDestination,
    NotPlainDestination,
    NotDirectory,
    UnknownRoot,
}

impl From<io::Error> for DetailReason {
    fn from(error: io::Error) -> Self {
        Self::Diagnostic(error.to_string())
    }
}

impl DetailReason {
    pub fn render(&self, format: impl Fn(&'static str, &[(&str, &str)]) -> String) -> String {
        let message = match self {
            Self::Diagnostic(detail) => return detail.clone(),
            Self::DuplicateRows(id) => {
                return format(
                    "several entries in {manifest} name {id}",
                    &[("manifest", MANIFEST), ("id", id)],
                );
            }
            Self::NotRegular => "not a regular file",
            Self::PendingTooLarge => "pending entry too large",
            Self::MissingSha => "missing SHA-256",
            Self::InvalidSha => "not a lowercase hex SHA-256",
            Self::InvalidPending => "invalid pending entry",
            Self::EntriesNotArray => "entries must be an array",
            Self::ChangedRow => "the project's entry changed since the install began",
            Self::NonPortableDestination => "not a portable home-relative file path",
            Self::NotPlainDestination => "the destination is not a plain regular file",
            Self::NotDirectory => "a destination component is not a directory",
            Self::UnknownRoot => "not a path with a known root",
        };
        format(message, &[])
    }
}

// Only literal template pieces are expanded; a manifest title isn't a template.
fn render_english(message: &'static str, args: &[(&str, &str)]) -> String {
    let mut out = String::new();
    let mut tail = message;
    while let Some((before, rest)) = tail.split_once('{') {
        let (key, after) = rest.split_once('}').expect("closed literal placeholder");
        out.push_str(before);
        out.push_str(
            args.iter()
                .find(|(name, _)| *name == key)
                .expect("literal argument")
                .1,
        );
        tail = after;
    }
    out.push_str(tail);
    out
}

#[derive(Debug)]
pub enum RecoveryProblem {
    Diagnostic(String),
    Detail {
        path: PathBuf,
        reason: DetailReason,
    },
    Claim {
        journal: PathBuf,
        problem: ClaimProblem,
    },
    /// A write, sync or retirement failed. Manifest or journal changes may be visible.
    Durability {
        path: PathBuf,
        error: io::Error,
    },
}

impl From<String> for RecoveryProblem {
    fn from(detail: String) -> Self {
        Self::Diagnostic(detail)
    }
}

impl RecoveryProblem {
    fn at(path: &Path, reason: DetailReason) -> Self {
        Self::Detail {
            path: path.to_owned(),
            reason,
        }
    }

    pub fn details(&self, format: impl Fn(&'static str, &[(&str, &str)]) -> String) -> String {
        match self {
            Self::Diagnostic(detail) => detail.clone(),
            Self::Detail { path, reason } => {
                format!("{}: {}", path.display(), reason.render(format))
            }
            Self::Claim { journal, problem } => {
                format!("{}: {}", journal.display(), problem.render(format))
            }
            Self::Durability { path, error } => format!("{}: {error}", path.display()),
        }
    }
}

impl std::fmt::Display for RecoveryProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.details(render_english))
    }
}

/// Settle an interrupted game install. The usual case has no pending file and does not wait
/// for a concurrent download. Writers still hold the home lock across read, modify and write.
pub fn reconcile(home: &Path) -> Result<(), RecoveryProblem> {
    let path = home.join(PENDING);
    // The common case does not need to take the shared game/launcher lock.
    if path
        .symlink_metadata()
        .is_err_and(|e| e.kind() == ErrorKind::NotFound)
    {
        return Ok(());
    }
    let _lock = lock_home(home).map_err(|e| format!("{}: {e}", home.display()))?;
    reconcile_locked(home, &path, sha256_file, sync_parent)
}

fn reconcile_locked(
    home: &Path,
    path: &Path,
    hash: impl FnOnce(&Path) -> Result<String, String>,
    sync: impl Fn(&Path) -> io::Result<()>,
) -> Result<(), RecoveryProblem> {
    let pending_problem = |reason| RecoveryProblem::at(path, reason);
    match path.symlink_metadata() {
        Ok(info) if !info.is_file() => return Err(pending_problem(DetailReason::NotRegular)),
        Ok(info) if info.len() > MAX_PENDING => {
            return Err(pending_problem(DetailReason::PendingTooLarge));
        }
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(pending_problem(e.into())),
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(MAX_PENDING + 1).read_to_end(&mut bytes))
        .map_err(|e| pending_problem(e.into()))?;
    if bytes.len() as u64 > MAX_PENDING {
        return Err(pending_problem(DetailReason::PendingTooLarge));
    }
    let pending: Pending = serde_json::from_slice(&bytes)
        .map_err(|e| pending_problem(DetailReason::Diagnostic(e.to_string())))?;
    let sha = pending.new["checksum"]
        .as_str()
        .and_then(|hash| hash.strip_prefix("SHA-256:"))
        .ok_or_else(|| pending_problem(DetailReason::MissingSha))?;
    let valid_row = |row: &Value| owned_by(row, &pending.id) && row["file"] == pending.file;
    if !valid_id(&pending.id)
        || pending.file != format!("{}/{}.smod", crate::mod_list::DIR, pending.id)
        || !valid_row(&pending.new)
        || !pending.new["version"].is_string()
        || !pending.old.as_ref().is_none_or(valid_row)
    {
        return Err(pending_problem(DetailReason::InvalidPending));
    }
    hex64(sha).map_err(|_| pending_problem(DetailReason::InvalidSha))?;

    // Read every raw row before reading the archive. Even an unpublished intent or an
    // already-new manifest needs a clean ownership check before retiring the sidecar.
    let manifest = home.join(MANIFEST);
    let mut document = manifest_document(&manifest)?;
    let manifest_problem = |reason| RecoveryProblem::at(&manifest, reason);
    let entries = document
        .get_mut("entries")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| manifest_problem(DetailReason::EntriesNotArray))?;
    let mut list = Installed {
        entries: std::mem::take(entries),
    };
    list.check_other_claims(home, &pending.id, &pending.file)
        .map_err(|problem| RecoveryProblem::Claim {
            journal: path.to_owned(),
            problem,
        })?;

    let target = home.join(&pending.file);
    let published = match target.symlink_metadata() {
        Ok(info) => info.len() <= MAX_FILE && hash(&target)? == sha,
        Err(e) if e.kind() == ErrorKind::NotFound => false,
        Err(e) => return Err(format!("{}: {e}", target.display()).into()),
    };
    let durability = |path: &Path, error| RecoveryProblem::Durability {
        path: path.to_owned(),
        error,
    };
    if published {
        let current = list
            .single(&pending.id)
            .map_err(manifest_problem)?
            .map(|i| &list.entries[i]);
        if current == Some(&pending.new) {
            // The row may be visible before its rename is durable.
            sync(&manifest).map_err(|e| durability(&manifest, e))?;
        } else if current == pending.old.as_ref() {
            sync(&target).map_err(|e| durability(&target, e))?;
            list.record(pending.new);
            *entries = list.entries;
            let bytes = serde_json::to_vec_pretty(&document)
                .map_err(|e| manifest_problem(DetailReason::Diagnostic(e.to_string())))?;
            save(&manifest, &bytes).map_err(|e| durability(&manifest, e))?;
        } else {
            return Err(pending_problem(DetailReason::ChangedRow));
        }
    }
    // Old or unrelated bytes keep the old row. Don't discard recovery data after an I/O error.
    match fs::remove_file(path) {
        Ok(()) => sync(path).map_err(|e| durability(path, e)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(durability(path, e)),
    }
}

/// Whether `entry` is the registry's row for mod or project `id`.
fn owned_by(entry: &Value, id: &str) -> bool {
    entry["source"] == "sunrise" && (entry["mod_id"].as_str() == Some(id) || entry["project"] == id)
}

fn manifest_document(path: &Path) -> Result<Value, RecoveryProblem> {
    let fail = |reason| RecoveryProblem::at(path, reason);
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => {
            let bytes = fs::read(path).map_err(|e| fail(e.into()))?;
            serde_json::from_slice(&bytes)
                .map_err(|e| fail(DetailReason::Diagnostic(e.to_string())))
        }
        Ok(_) => Err(fail(DetailReason::NotRegular)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(serde_json::json!({"entries": []})),
        Err(error) => Err(fail(error.into())),
    }
}

/// `mod-browser.json`, kept as JSON values so the launcher's entries this
/// game doesn't model (Modrinth resource packs, ported Java mods) are written
/// back as they were.
#[derive(Debug, Default)]
pub struct Installed {
    pub entries: Vec<Value>,
}

impl Installed {
    /// Read raw manifest claims after recovery. The caller holds the shared home lock.
    pub fn read(home: &Path) -> Result<Self, RecoveryProblem> {
        let path = home.join(MANIFEST);
        let value = manifest_document(&path)?;
        let entries = value["entries"]
            .as_array()
            .ok_or_else(|| RecoveryProblem::at(&path, DetailReason::EntriesNotArray))?
            .clone();
        Ok(Self { entries })
    }

    /// The registry's one entry for project `id`, if any. Several are refused: which one holds
    /// later edits is unknown, and replacing them would lose the others.
    pub fn single(&self, id: &str) -> Result<Option<usize>, DetailReason> {
        let mut rows = (0..self.entries.len()).filter(|&i| owned_by(&self.entries[i], id));
        match (rows.next(), rows.next()) {
            (_, Some(_)) => Err(DetailReason::DuplicateRows(id.to_owned())),
            (first, None) => Ok(first),
        }
    }

    fn record(&mut self, entry: Value) {
        let project = entry["project"].as_str().map(str::to_owned);
        let mod_id = entry["mod_id"].as_str().map(str::to_owned);
        self.entries.retain(|e| {
            if e["source"] != "sunrise" {
                return true;
            }
            if project.as_deref().is_some_and(|p| e["project"] == p) {
                return false;
            }
            if mod_id
                .as_deref()
                .is_some_and(|m| e["mod_id"].as_str() == Some(m) || e["project"] == m)
            {
                return false;
            }
            true
        });
        self.entries.push(entry);
    }

    /// Check the destination and other rows. The caller holds the shared home lock, and
    /// checks uniqueness of the target project's row separately.
    pub fn check_other_claims(
        &self,
        home: &Path,
        id: &str,
        file: &str,
    ) -> Result<(), ClaimProblem> {
        claims::check_destination(
            home,
            file,
            self.entries.iter().filter(|entry| !owned_by(entry, id)),
        )
    }
}

fn sync_parent(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        let parent = path
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let dir = fs::File::open(parent)?;
        #[cfg(not(target_os = "macos"))]
        dir.sync_all()?;
        // A directory has no data for macOS's regular-file F_FULLFSYNC barrier.
        #[cfg(target_os = "macos")]
        loop {
            use std::os::fd::AsRawFd;
            // SAFETY: the descriptor is valid while `dir` is open.
            if unsafe { libc::fsync(dir.as_raw_fd()) } == 0 {
                break;
            }
            let error = io::Error::last_os_error();
            if error.kind() != ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
    // The game uses a write-through move on Windows, where dirs cannot be fsynced.
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn save(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    for _ in 0..16 {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let temporary =
            path.with_file_name(format!(".mod-browser-{}-{id}.part", std::process::id()));
        let mut file = match fs::File::create_new(&temporary) {
            Ok(file) => file,
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let written = file.write_all(bytes).and_then(|()| file.sync_all());
        drop(file);
        return written
            .and_then(|()| replace(&temporary, path))
            .inspect_err(|_| {
                let _ = fs::remove_file(&temporary);
            });
    }
    Err(ErrorKind::AlreadyExists.into())
}

fn replace(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let wide = |path: &Path| -> io::Result<Vec<u16>> {
            let parent = path.parent().expect("file in the game home");
            let name = path.file_name().expect("a filename");
            Ok(parent
                .canonicalize()?
                .join(name)
                .as_os_str()
                .encode_wide()
                .chain([0])
                .collect())
        };
        let (from, to) = (wide(from)?, wide(to)?);
        // SAFETY: both buffers are NUL-terminated and live through this synchronous call.
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
    #[cfg(not(windows))]
    {
        fs::rename(from, to)?;
        sync_parent(to)?;
        // Flush the device after the directory metadata barrier on APFS.
        #[cfg(target_os = "macos")]
        fs::File::open(to)?.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod claims_matrix_tests;
#[cfg(test)]
mod tests;

/// What a project is, as far as Sunrise is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    ResourcePack,
    Modpack,
    /// A Java mod: its content is ported.
    Mod,
    /// One of Sunrise's own shader packs (`.sshader`, docs/shaders.md), from Sunrise's registry
    /// only.
    Shader,
    /// A data pack: it goes into one world's `datapacks/`.
    DataPack,
}

impl Kind {
    pub const ALL: [Kind; 5] = [
        Kind::Mod,
        Kind::Modpack,
        Kind::ResourcePack,
        Kind::Shader,
        Kind::DataPack,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Mod => "Mods",
            Kind::ResourcePack => "Resource packs",
            Kind::Modpack => "Modpacks",
            Kind::Shader => "Shader packs",
            Kind::DataPack => "Data packs",
        }
    }

    /// The folder beside `options.txt` the file goes in; a modpack is imported, a mod ported
    /// and a data pack put in a world instead.
    pub fn folder(self) -> Option<&'static str> {
        match self {
            Kind::ResourcePack => Some("resourcepacks"),
            Kind::Shader => Some("shaderpacks"),
            Kind::Modpack | Kind::Mod | Kind::DataPack => None,
        }
    }
}

/// Where a project comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceId {
    Modrinth,
    /// Sunrise's own registry of reviewed mods, modpacks and resource packs.
    Sunrise,
}

impl SourceId {
    pub fn label(self) -> &'static str {
        match self {
            SourceId::Modrinth => "Modrinth",
            SourceId::Sunrise => "Sunrise",
        }
    }
}

/// The folders the browser installs into: an entry's `file` is always inside one of them.
const FOLDERS: [&str; 5] = [
    "mods",
    "resourcepacks",
    "shaderpacks",
    crate::mod_list::DIR,
    crate::mod_list::NATIVE_DIR,
];

/// Whether `file` names something inside one of the folders the browser installs into, made of
/// plain names only (no root, drive or `..`), so `home.join(file)` stays in the game folder. The
/// list is a file in that folder, which a modpack could have put there: an entry naming any other
/// place must never make the browser remove or rename it.
pub fn in_folders(file: &str) -> bool {
    let mut parts = Path::new(file).components();
    let top = parts.next();
    matches!(top, Some(Component::Normal(t)) if FOLDERS.iter().any(|f| t == *f))
        && parts.clone().next().is_some()
        && parts.all(|c| matches!(c, Component::Normal(_)))
}
