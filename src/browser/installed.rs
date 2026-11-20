//! What the browser installed into the game folder, and installing, updating and removing it.
//!
//! `mod-browser.json` beside `options.txt` lists each file the browser put in `resourcepacks/`,
//! and each Sunrise mod it put in `sunrise-mods/`: where it came from, its version and its
//! checksum. Files the player put there themselves are never touched: the browser only replaces
//! or removes what it lists.
use super::{pick, safe_name, Kind, Project, Source, SourceId, Support, Version};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub(crate) const MANIFEST: &str = "mod-browser.json";
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
        fs::read_to_string(home.join(MANIFEST))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
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
        let res = fs::write(&part, text).and_then(|()| fs::rename(&part, &path));
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

/// An installed registry mod an install removes with it, as the player approved it: this exact
/// file (its checksum), and whether they also agreed to what saved worlds that use it lose.
/// Checked again under the install's lock, since the approval may have waited in a queue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Replace {
    pub id: String,
    pub checksum: String,
    pub worlds_ok: bool,
}

/// What the player approved for an install, checked again under the install's lock.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Approval {
    /// Removing this installed mod, as shown.
    Remove(Replace),
    /// Losing what saved worlds use from this mod: its new version's package no longer has
    /// content its old one had.
    DropContent { id: String },
}

/// Why an install didn't happen: native code the player hasn't allowed yet (the plan may not
/// have known it was there), which the launcher answers by asking and trying again, or any
/// other reason, in words.
#[derive(Clone, Debug, PartialEq)]
pub enum InstallError {
    /// Installing needs `native`, which runs native code, before the player has allowed it.
    NeedsNativeConsent {
        native: String,
    },
    /// A mod the install needs has no version for the game played (or none in the range
    /// asked): `dependency` by its name, with the planner's `reason` for the details.
    Unavailable {
        dependency: String,
        reason: String,
    },
    /// Installed mods pin what the install needs: they are named, with the planner's `reason`.
    Conflict {
        mods: Vec<String>,
        reason: String,
    },
    /// A saved world uses what the new version of these mods no longer has: the world question
    /// comes before anything is published.
    NeedsWorlds {
        mods: Vec<String>,
    },
    /// The planner refused for another reason, in its words.
    Refused(String),
    Message(String),
}

impl InstallError {
    pub(super) fn for_build(error: crate::native_mods::Failed, native: &str) -> Self {
        match error {
            crate::native_mods::Failed::NoConsent => Self::NeedsNativeConsent {
                native: native.into(),
            },
            other => Self::Message(other.to_string()),
        }
    }
}

#[cfg(test)]
impl InstallError {
    /// For tests that read the words of a refusal.
    pub(crate) fn contains(&self, text: &str) -> bool {
        self.to_string().contains(text)
    }
}

impl From<String> for InstallError {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}

impl From<&str> for InstallError {
    fn from(message: &str) -> Self {
        Self::Message(message.to_owned())
    }
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NeedsNativeConsent { native } => f.write_str(&crate::i18n::trf(
                "{native} runs native code: allow native mods first.",
                &[("native", native)],
            )),
            Self::Unavailable { reason, .. }
            | Self::Conflict { reason, .. }
            | Self::Refused(reason) => f.write_str(reason),
            Self::NeedsWorlds { mods } => f.write_str(&crate::i18n::trf(
                "A saved world uses {mods}: install again to be asked about it.",
                &[("mods", &mods.join(", "))],
            )),
            Self::Message(message) => f.write_str(message),
        }
    }
}

impl From<InstallError> for String {
    fn from(error: InstallError) -> Self {
        error.to_string()
    }
}

/// Installs a browser pack and any dependencies. Registry mods use the exact-profile
/// install plan instead. `progress` receives a line per file and its fraction.
pub fn install(
    source: &dyn Source,
    project: &Project,
    everything: bool,
    home: &Path,
    game: &str,
    progress: &dyn Fn(&str, f32),
) -> Result<Vec<Entry>, InstallError> {
    let lease = super::sunrise::txn::Lease::admit(home)?;
    lease.install(source, project, everything, game, progress)
}

impl super::sunrise::txn::Lease {
    /// Installs a browser pack into the leased profile.
    pub fn install(
        &self,
        source: &dyn Source,
        project: &Project,
        everything: bool,
        game: &str,
        progress: &dyn Fn(&str, f32),
    ) -> Result<Vec<Entry>, InstallError> {
        let home = self.home();
        let _lock = super::lock_home(home).map_err(|e| e.to_string())?;
        if super::sunrise::txn::has_build(home)? {
            return Err(super::sunrise::txn::unsettled().into());
        }
        if project.source == SourceId::Sunrise {
            return super::sunrise::install(project, home, game, progress);
        }
        let steps = plan(source, project, everything, game)?;
        Ok(put(
            source,
            steps,
            home,
            &|kind| kind.folder().expect("plan keeps folder kinds"),
            progress,
        )?)
    }
}

/// Downloads each step's file into `home`, in the folder `folder` names for its kind, and lists
/// it: what was already there at that version is kept, an older version replaced.
pub(super) fn put(
    source: &dyn Source,
    steps: Vec<Step>,
    home: &Path,
    folder: &dyn Fn(Kind) -> &'static str,
    progress: &dyn Fn(&str, f32),
) -> Result<Vec<Entry>, String> {
    let _lock = super::lock_home(home).map_err(|e| e.to_string())?;
    let mut installed = Installed::load(home);
    let mut done = Vec::new();
    for step in steps {
        let p = &step.project;
        let old = installed.find(p.source, &p.id).cloned();
        if old.as_ref().is_some_and(|o| o.version == step.version.id) {
            done.push(old.expect("checked"));
            continue;
        }
        let file = step
            .version
            .file()
            .expect("pick keeps versions with a file");
        let folder_name = folder(p.kind);
        let name = safe_name(&file.name)
            .ok_or_else(|| format!("{:?} isn't a plain file name.", file.name))?;
        let relative = format!("{folder_name}/{name}");
        let owner = installed.entries.iter().find(|e| e.file == relative);
        let ours = owner.is_some_and(|e| e.source == p.source && e.project == p.id);
        if home.join(&relative).exists() && !ours {
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
        let path = source.fetch(file, &home.join(folder_name), &|f| progress(&line, f))?;
        let (algo, hash) = file.checksum().expect("fetch checks for one");
        if let Some(old) = &old {
            if let Some(file) = old.path(home).filter(|_| old.file != relative) {
                let _ = fs::remove_file(file);
            }
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
        };
        debug_assert!(path.ends_with(name));
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
    let mut installed = Installed::load(home);
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

/// The newer version `entry` can be updated to on the Minecraft version `game`, if any.
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
