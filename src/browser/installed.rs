//! What the browser installed into the game folder, and installing, updating and removing it.
//!
//! `mod-browser.json` beside `options.txt` lists each file the browser put in `resourcepacks/`,
//! and each Sunrise mod it put in `sunrise-mods/`: where it came from, its version and its
//! checksum. Files the player put there themselves are never touched: the browser only replaces
//! or removes what it lists.
use super::{pick, safe_name, Kind, Project, Source, SourceId, Support, Version};
use crate::i18n::{tr, trf};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub(crate) use sunrise_launcher_smod::index::browser_state::MANIFEST;
use sunrise_launcher_smod::index::browser_state::{self, RecoveryProblem};

/// A game-owned install not safe to finish. The full diagnostic belongs in Details, not in a banner.
#[derive(Debug)]
pub struct GameInstallProblem {
    detail: String,
    may_have_changed: bool,
}

impl GameInstallProblem {
    fn from_recovery(home: &Path, error: RecoveryProblem) -> Self {
        let may_have_changed = matches!(error, RecoveryProblem::Durability { .. });
        let error = error.details(crate::i18n::trf);
        // Relative file names and the cause are useful; the host's local path is not.
        let folder = home.display().to_string();
        let prefix = format!("{folder}{}", std::path::MAIN_SEPARATOR);
        let detail = error.strip_prefix(&prefix).map_or_else(
            || error.strip_prefix(&format!("{folder}: ")).unwrap_or(&error),
            |relative| relative,
        );
        Self {
            detail: detail.to_owned(),
            may_have_changed,
        }
    }

    pub fn summary(&self) -> &'static str {
        if self.may_have_changed {
            crate::i18n::tr(
                "A mod install the game started was interrupted and couldn't be finished.",
            )
        } else {
            crate::i18n::tr("A mod install the game started was interrupted and couldn't be finished. Your mods and files were left as they are.")
        }
    }

    pub fn details(&self) -> String {
        format!("{}\n{}", self.summary(), self.detail)
    }
}

impl From<GameInstallProblem> for String {
    fn from(value: GameInstallProblem) -> Self {
        value.details()
    }
}

/// Settle the game's sidecar using its own home lock. Callers that mutate the profile
/// hold the same, reentrant lock through their changes. No launcher journal is involved.
pub fn recover_game_install(home: &Path) -> Result<(), GameInstallProblem> {
    browser_state::reconcile(home).map_err(|error| GameInstallProblem::from_recovery(home, error))
}

/// The folders the browser installs into: an entry's `file` is always inside one of them.
const FOLDERS: [&str; 5] = [
    "mods",
    "resourcepacks",
    "shaderpacks",
    super::mod_list::MODS_DIR,
    crate::native_mods::INSTALLED,
];
/// How deep required dependencies are followed.
pub(super) const MAX_DEPTH: usize = 4;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub source: SourceId,
    pub project: String,
    pub slug: String,
    pub title: String,
    pub kind: Kind,
    pub version: String,
    pub number: String,
    /// Relative to the game folder: `resourcepacks/<name>`.
    pub file: String,
    /// The checksum the download was checked against: `SHA-512:<hex>`.
    pub checksum: String,
    pub page: String,
    /// Installed because another pack needs it.
    #[serde(default)]
    pub dependency: bool,
    /// What a Sunrise mod's trust was when it was installed (`trust_label`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<String>,
    /// A retained install's verified source stays off until the player can address this failure.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "launcher_install_hold"
    )]
    pub install_hold: Option<InstallHold>,
}

/// Why verified bytes are intentionally installed but off after a partial native install.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstallHold {
    Compile,
    Needs { name: String },
}

impl InstallHold {
    pub fn label(&self) -> String {
        match self {
            Self::Compile => tr("Installed, couldn't build").into(),
            Self::Needs { name } => trf("Needs {mod}", &[("mod", name)]),
        }
    }

    pub fn short_reason(&self) -> String {
        match self {
            Self::Compile => tr("Couldn't build").into(),
            Self::Needs { .. } => self.label(),
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Self::Compile => tr("Its code couldn't build on your computer.").into(),
            Self::Needs { name } => {
                trf("It needs {mod} to be on and compatible.", &[("mod", name)])
            }
        }
    }
}

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

impl Entry {
    /// Where the entry's file is in the game folder `home`, when it is inside one the browser
    /// installs into ([`in_folders`]).
    pub fn path(&self, home: &Path) -> Option<PathBuf> {
        in_folders(&self.file).then(|| home.join(&self.file))
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Installed {
    /// An entry this launcher can't read (a shader pack, which older launchers installed) is
    /// left out, and its file left where it is; so is one whose file is outside the folders the
    /// browser installs into ([`in_folders`]).
    #[serde(deserialize_with = "readable_entries")]
    pub entries: Vec<Entry>,
}

fn readable_entries<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Entry>, D::Error> {
    let entries = Vec::<serde_json::Value>::deserialize(d)?;
    Ok(entries
        .into_iter()
        .filter_map(|e| serde_json::from_value::<Entry>(e).ok())
        .filter(|e| {
            let kept = in_folders(&e.file);
            if !kept {
                eprintln!(
                    "{MANIFEST}: left out {:?}, whose file {:?} isn't in a pack or mod folder",
                    e.title, e.file
                );
            }
            kept
        })
        .collect())
}

impl Installed {
    pub fn load(home: &Path) -> Self {
        Self::read(home).unwrap_or_default()
    }

    pub fn read(home: &Path) -> Result<Self, String> {
        let path = home.join(MANIFEST);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => return Err(format!("{MANIFEST}: {error}")),
        };
        serde_json::from_str(&text).map_err(|error| format!("{MANIFEST}: {error}"))
    }

    /// Atomically publishes the list. Mutating callers must hold `lock_home`
    /// from before `load` through this save, not just while writing the file.
    pub fn save(&self, home: &Path) -> Result<(), String> {
        let _lock = super::lock_home(home).map_err(|e| e.to_string())?;
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = home.join(MANIFEST);
        let part = home.join(format!(".{MANIFEST}.{}-{id}.part", std::process::id()));
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::create_dir_all(home).map_err(|e| format!("{}: {e}", home.display()))?;
        let mut file =
            fs::File::create_new(&part).map_err(|e| format!("{}: {e}", part.display()))?;
        let written = std::io::Write::write_all(&mut file, text.as_bytes());
        drop(file);
        let res = written.and_then(|()| fs::rename(&part, &path));
        if let Err(e) = res {
            let _ = fs::remove_file(&part);
            return Err(format!("{}: {e}", path.display()));
        }
        Ok(())
    }

    pub fn find(&self, source: SourceId, project: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.source == source && e.project == project)
    }

    fn single(&self, source: SourceId, project: &str) -> Result<Option<&Entry>, String> {
        let mut rows = self
            .entries
            .iter()
            .filter(|e| e.source == source && e.project == project);
        let first = rows.next();
        if rows.next().is_some() {
            return Err(format!("Several browser entries claim {project}."));
        }
        Ok(first)
    }

    /// By project id or slug, as the CLI takes them.
    pub fn lookup(&self, name: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.project == name || e.slug == name)
    }
}

/// One file to install: the project, the version chosen and why it was pulled in.
#[derive(Clone, Debug)]
pub struct Step {
    pub project: Project,
    pub version: Version,
    pub dependency: bool,
}

/// What installing `project` takes, dependencies first, checked before any byte is downloaded:
/// every file runs in Sunrise and has a version to install.
pub fn plan(
    source: &dyn Source,
    project: &Project,
    everything: bool,
    game: &str,
) -> Result<Vec<Step>, String> {
    if project.kind == Kind::Mod {
        return Err(crate::i18n::trf(
            "{title} is a Java mod. Sunrise doesn't install Java mods; it installs resource packs, data packs and shader packs from Modrinth.",
            &[("title", &project.title)],
        ));
    }
    if project.kind.folder().is_none() {
        return Err(format!(
            "{} isn't a pack Sunrise installs as a file.",
            project.title
        ));
    }
    let mut steps = Vec::new();
    add(
        source,
        project.clone(),
        everything,
        game,
        false,
        0,
        &mut steps,
    )?;
    Ok(steps)
}

fn add(
    source: &dyn Source,
    project: Project,
    everything: bool,
    game: &str,
    dependency: bool,
    depth: usize,
    steps: &mut Vec<Step>,
) -> Result<(), String> {
    if steps.iter().any(|s| s.project.id == project.id) {
        return Ok(());
    }
    if depth > MAX_DEPTH {
        return Err(format!("{}'s dependencies go too deep.", project.title));
    }
    if let Support::No(why) = project.support(game) {
        return Err(format!("{} doesn't run in Sunrise: {why}", project.title));
    }
    let versions = source.versions(&project)?;
    let Some(version) = pick(project.kind, &versions, everything, game).cloned() else {
        return Err(match versions.iter().find_map(|v| v.blocked.clone()) {
            Some(why) => format!("{} can't be installed from here: {why}", project.title),
            None => format!("{} has no version Sunrise runs.", project.title),
        });
    };
    for id in &version.required {
        let needed = source.project(id).map_err(|_| {
            format!(
                "{} needs {id} on {}, which isn't a pack Sunrise loads.",
                project.title,
                source.id().label()
            )
        })?;
        if needed.kind.folder().is_none() {
            return Err(format!("{} needs {}.", project.title, needed.title));
        }
        add(source, needed, everything, game, true, depth + 1, steps)?;
    }
    steps.push(Step {
        project,
        version,
        dependency,
    });
    Ok(())
}

/// Installs `project` (and what it needs) into the game folder `home`. A project already
/// installed is updated to the version chosen. `progress` gets a line per file and the fraction.
pub fn install(
    source: &dyn Source,
    project: &Project,
    everything: bool,
    home: &Path,
    game: &str,
    progress: &dyn Fn(&str, f32),
) -> Result<Vec<Entry>, String> {
    let _lock = super::lock_home(home).map_err(|e| e.to_string())?;
    if project.source == SourceId::Sunrise {
        return super::sunrise::install_pack(project, home, game, progress);
    }
    recover_game_install(home)?;
    let installed = Installed::read(home)?;
    let steps = plan(source, project, everything, game)?;
    put_packs(source, steps, home, installed, progress)
}

/// Checks a legacy destination under the home lock. Only a plain file directly in its own
/// kind's pack folder is admissible, without symlinks or Windows junctions in the way.
pub(super) fn pack_path(home: &Path, kind: Kind, file: &str) -> Result<PathBuf, String> {
    let (folder, name) = file
        .split_once('/')
        .filter(|(folder, name)| kind.folder() == Some(*folder) && safe_name(name).is_some())
        .ok_or_else(|| format!("{file} isn't a pack file in its expected folder."))?;
    let dir = home.join(folder);
    let path = dir.join(name);
    for (part, is_dir) in [(&dir, true), (&path, false)] {
        match fs::symlink_metadata(part) {
            Ok(meta) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
                    if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                        return Err(format!("{file} goes through a filesystem link."));
                    }
                }
                let plain = if is_dir {
                    meta.is_dir()
                } else {
                    meta.is_file()
                };
                if !plain {
                    return Err(format!("{file} isn't a pack file in a plain pack folder."));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(format!("{}: {e}", part.display())),
        }
    }
    Ok(path)
}

/// A live row must still match the pack kind selected by the caller or resolved from the source.
pub(super) fn old_pack_path(home: &Path, entry: &Entry, kind: Kind) -> Result<PathBuf, String> {
    if entry.kind != kind {
        return Err(format!("{} has a different installed kind.", entry.title));
    }
    pack_path(home, kind, &entry.file)
}

/// Publishes a planned Modrinth pack set. The caller holds the home lock from before the read.
fn put_packs(
    source: &dyn Source,
    steps: Vec<Step>,
    home: &Path,
    mut installed: Installed,
    progress: &dyn Fn(&str, f32),
) -> Result<Vec<Entry>, String> {
    let mut done = Vec::new();
    for step in steps {
        let p = &step.project;
        let old = installed.single(p.source, &p.id)?.cloned();
        let previous = old
            .as_ref()
            .map(|row| old_pack_path(home, row, p.kind))
            .transpose()?;
        if old.as_ref().is_some_and(|o| o.version == step.version.id) {
            done.push(old.expect("checked"));
            continue;
        }
        let file = step
            .version
            .file()
            .expect("pick keeps versions with a file");
        let folder_name = p.kind.folder().expect("plan keeps folder kinds");
        let name = safe_name(&file.name)
            .ok_or_else(|| format!("{:?} isn't a plain file name.", file.name))?;
        let relative = format!("{folder_name}/{name}");
        let target = pack_path(home, p.kind, &relative)?;
        let owner = installed.entries.iter().find(|e| e.file == relative);
        let ours = owner.is_some_and(|e| e.source == p.source && e.project == p.id);
        if target.exists() && !ours {
            return Err(match owner {
                Some(e) => format!(
                    "{relative} belongs to {}, installed from the browser.",
                    e.title
                ),
                None => format!(
                    "{relative} is already there and wasn't installed by the browser: move it \
                     away first, or keep it."
                ),
            });
        }
        let line = format!("{} {}", p.title, step.version.number);
        // Stage on the destination filesystem without trusting a pre-existing partial file.
        let dir = home.join(folder_name);
        fs::create_dir_all(&dir).map_err(|e| format!("{folder_name}: {e}"))?;
        let stage = tempfile::tempdir_in(dir).map_err(|e| e.to_string())?;
        let path = source.fetch(file, stage.path(), &|f| progress(&line, f))?;
        let (algo, hash) = file.checksum().expect("fetch checks for one");
        fs::rename(path, &target).map_err(|e| format!("{relative}: {e}"))?;
        if let Some(old) = previous.filter(|path| path != &target) {
            let _ = fs::remove_file(old);
        }
        let entry = Entry {
            source: p.source,
            project: p.id.clone(),
            slug: p.slug.clone(),
            title: p.title.clone(),
            kind: p.kind,
            version: step.version.id.clone(),
            number: step.version.number.clone(),
            file: relative,
            checksum: format!("{algo}:{hash}"),
            page: p.page.clone(),
            // Asked for by the player once: it stays theirs.
            dependency: step.dependency && old.as_ref().is_none_or(|o| o.dependency),
            verdict: None,
            install_hold: None,
        };
        installed
            .entries
            .retain(|e| !(e.source == p.source && e.project == p.id));
        installed.entries.push(entry.clone());
        installed.save(home)?;
        done.push(entry);
    }
    Ok(done)
}

/// Whether `file` is a folder directly in `sunrise-mods/`, which is itself a real folder of
/// `home` (a link in the way is not followed): where an earlier launcher put a ported mod.
fn legacy_mod_folder(home: &Path, file: &str) -> bool {
    let mut parts = Path::new(file).components();
    let (Some(Component::Normal(top)), Some(Component::Normal(_)), None) =
        (parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    top == "sunrise-mods" && fs::symlink_metadata(home.join(top)).is_ok_and(|m| m.is_dir())
}

/// Removes what the browser installed for `name` (a project id or slug): its file and its entry.
pub fn remove(home: &Path, name: &str) -> Result<Entry, String> {
    let _lock = super::lock_home(home).map_err(|e| e.to_string())?;
    recover_game_install(home)?;
    let mut installed = Installed::read(home)?;
    let entry = installed
        .lookup(name)
        .cloned()
        .ok_or_else(|| format!("{name} wasn't installed by the browser."))?;
    // A mod an earlier launcher ported from Modrinth is a folder in `sunrise-mods/`, and only
    // that is removed recursively; a pack is a file, and a folder where a pack's file was is the
    // player's own and stays.
    let removed = match entry.path(home) {
        None => Ok(()),
        Some(p) => match fs::symlink_metadata(&p) {
            Ok(m) if m.is_dir() => {
                if legacy_mod_folder(home, &entry.file) {
                    fs::remove_dir_all(&p)
                } else {
                    return Err(crate::i18n::trf(
                        "{file} is a folder, not a file the browser installed. Nothing was removed: move it away yourself.",
                        &[("file", &entry.file)],
                    ));
                }
            }
            Ok(_) => fs::remove_file(&p),
            Err(e) => Err(e),
        },
    };
    match removed {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("{}: {e}", entry.file)),
    }
    let mod_id = match entry.source {
        SourceId::Sunrise => super::sunrise::mod_id(&entry),
        SourceId::Modrinth => entry.file.strip_prefix("sunrise-mods/"),
    };
    if let Some(id) = mod_id {
        super::mod_list::set_enabled(home, id, false)?;
    }
    installed
        .entries
        .retain(|e| !(e.source == entry.source && e.project == entry.project));
    installed.save(home)?;
    Ok(entry)
}

/// Removes a CLI-selected pack, never a mod. The unlocked CLI selection is only a lookup key.
pub fn remove_pack(home: &Path, selected: &Entry) -> Result<Entry, String> {
    let _lock = super::lock_home(home).map_err(|e| e.to_string())?;
    recover_game_install(home)?;
    let mut installed = Installed::read(home)?;
    let entry = installed
        .single(selected.source, &selected.project)?
        .cloned()
        .ok_or_else(|| format!("{} wasn't installed by the browser.", selected.title))?;
    let path = old_pack_path(home, &entry, selected.kind)?;
    match fs::remove_file(path) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(format!("{}: {e}", entry.file)),
    }
    installed
        .entries
        .retain(|e| !(e.source == entry.source && e.project == entry.project));
    installed.save(home)?;
    Ok(entry)
}

pub enum PackUpdate {
    Current,
    Available(String),
    Updated(Vec<Entry>),
}

/// Check or update an installed resource or shader pack. Data packs are managed within worlds.
pub fn update_pack(
    home: &Path,
    selected: &Entry,
    game: &str,
    check: bool,
    progress: &dyn Fn(&str, f32),
) -> Result<PackUpdate, String> {
    let _lock = super::lock_home(home).map_err(|e| e.to_string())?;
    if !check {
        recover_game_install(home)?;
    }
    let installed = Installed::read(home)?;
    let entry = installed
        .single(selected.source, &selected.project)?
        .ok_or_else(|| format!("{} wasn't installed by the browser.", selected.title))?;
    old_pack_path(home, entry, selected.kind)?;
    if entry.source == SourceId::Sunrise {
        return super::sunrise::update_pack(entry, home, game, check, progress);
    }
    let source = super::modrinth::Modrinth;
    let project = source.project(&entry.project)?;
    if project.kind != entry.kind {
        return Err(format!("{} has a different installed kind.", entry.title));
    }
    let versions = source.versions(&project)?;
    let Some(newer) = pick(project.kind, &versions, true, game).filter(|v| v.id != entry.version)
    else {
        return Ok(PackUpdate::Current);
    };
    if check {
        let folder = entry.kind.folder().expect("checked pack kind");
        let file = newer.file().expect("pick keeps versions with a file");
        pack_path(home, entry.kind, &format!("{folder}/{}", file.name))?;
        return Ok(PackUpdate::Available(newer.number.clone()));
    }
    let steps = plan(&source, &project, true, game)?;
    put_packs(&source, steps, home, installed, progress).map(PackUpdate::Updated)
}

/// Read-only version lookup. The CLI uses `update_pack` for checks on the pack exception.
pub fn update_for(
    source: &dyn Source,
    entry: &Entry,
    game: &str,
) -> Result<Option<Version>, String> {
    let project = source.project(&entry.project)?;
    let versions = source.versions(&project)?;
    Ok(pick(project.kind, &versions, true, game)
        .filter(|v| v.id != entry.version)
        .cloned())
}

/// A data pack installed into a world.
pub struct DataPackInstall {
    pub pack: crate::datapacks::DataPack,
    /// Its version's number.
    pub number: String,
    /// What of it runs in Sunrise.
    pub coverage: crate::datapacks::Coverage,
}

/// Downloads a project's data pack (checksum checked) into the world folder `world`, under a
/// free name. Refused when nothing in it runs in Sunrise yet. It isn't recorded with the
/// browser's installs: the world's data pack list (Home, `worlds datapack-list`) has it.
pub fn install_datapack(
    source: &dyn Source,
    project: &Project,
    everything: bool,
    world: &Path,
    game: &str,
    progress: &dyn Fn(f32),
) -> Result<DataPackInstall, String> {
    if project.kind != Kind::DataPack && !project.loaders.iter().any(|l| l == "datapack") {
        return Err(format!("{} isn't a data pack.", project.title));
    }
    let folder = std::env::temp_dir().join(format!(
        "sunrise-launcher-datapack-{}-{}",
        std::process::id(),
        project.id
    ));
    // The registry's are checked against its index (signature, `pack.mcmeta` and `data/`).
    let fetched = match project.source {
        SourceId::Sunrise => super::sunrise::fetch_datapack(project, &folder, game, progress),
        _ => {
            let versions = source.versions(project)?;
            let version = pick(Kind::DataPack, &versions, everything, game).ok_or_else(|| {
                format!(
                    "{} has no data pack for {}{}.",
                    project.title,
                    game,
                    if everything {
                        " or any other version"
                    } else {
                        ""
                    }
                )
            })?;
            let file = version.file().expect("pick keeps versions with a file");
            source
                .fetch(file, &folder, progress)
                .map(|zip| (zip, version.number.clone()))
        }
    };
    let result = fetched.and_then(|(zip, number)| {
        let coverage = crate::datapacks::coverage(&zip)?;
        if coverage.read.is_empty() {
            return Err(crate::i18n::trf(
                "{title} would change nothing in Sunrise yet: Sunrise doesn't read its {files}.",
                &[("title", &project.title), ("files", &coverage.not_read())],
            ));
        }
        let pack = crate::datapacks::add(world, &zip)?;
        Ok(DataPackInstall {
            pack,
            number,
            coverage,
        })
    });
    let _ = fs::remove_dir_all(&folder);
    result
}

#[cfg(test)]
mod tests;
