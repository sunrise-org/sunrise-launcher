//! What the browser installed into the game folder, and installing, updating and removing it.
//!
//! `mod-browser.json` beside `options.txt` lists each file the browser put in `resourcepacks/`,
//! and each Sunrise mod it put in `sunrise-mods/`: where it came from, its version and its
//! checksum. Files the player put there themselves are never touched: the browser only replaces
//! or removes what it lists.
use super::{pick, safe_name, Kind, Project, Source, SourceId, Support, Version};
use crate::i18n::{tr, trf};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) use sunrise_launcher_smod::index::browser_state::MANIFEST;

/// The profile's install journal owns this type and its recovery entry point.
pub use crate::managed_play::{recover_game_install, GameInstallProblem};
/// How deep required dependencies are followed.
pub(super) const MAX_DEPTH: usize = 4;

pub use sunrise_launcher_smod::index::browser_state::in_folders;
pub use sunrise_launcher_smod::index::browser_state::records::{
    Entry, InstallHold, Installed, VerifiedRepository,
};

/// Player-facing label for a retained install's hold.
pub fn hold_label(hold: &InstallHold) -> String {
    match hold {
        InstallHold::Compile => tr("Installed, couldn't build").into(),
        InstallHold::Needs { name } => trf("Needs {mod}", &[("mod", name)]),
    }
}

/// Short reason a retained install is off, for a status line.
pub fn hold_short_reason(hold: &InstallHold) -> String {
    match hold {
        InstallHold::Compile => tr("Couldn't build").into(),
        InstallHold::Needs { .. } => hold_label(hold),
    }
}

/// Full explanation for the Details pane.
pub fn hold_detail(hold: &InstallHold) -> String {
    match hold {
        InstallHold::Compile => tr("Its code couldn't build on your computer.").into(),
        InstallHold::Needs { name } => {
            trf("It needs {mod} to be on and compatible.", &[("mod", name)])
        }
    }
}

/// Display metadata, not proof that a package may be updated or removed.
#[derive(Clone, Copy)]
pub enum ModLabelSource {
    Registry,
    Github,
    Local,
}

pub struct ModLabel<'a> {
    /// The manifest ID, used only to order ordinals (never displayed).
    pub id: &'a str,
    pub name: &'a str,
    pub authors: &'a [String],
    pub version: &'a str,
    pub source: ModLabelSource,
}

impl ModLabel<'_> {
    fn with_author(&self) -> Option<String> {
        let mut authors: Vec<_> = self
            .authors
            .iter()
            .map(|a| a.trim())
            .filter(|a| !a.is_empty())
            .collect();
        authors.sort_unstable();
        authors.dedup();
        (!authors.is_empty()).then(|| {
            trf(
                "{name} · by {authors}",
                &[("name", self.name), ("authors", &authors.join(", "))],
            )
        })
    }

    fn with_version(&self, name: &str) -> Option<String> {
        (!self.version.is_empty()).then(|| {
            trf(
                "{name} · {version}",
                &[("version", self.version), ("name", name)],
            )
        })
    }

    fn with_source(&self, name: &str) -> String {
        let source = match self.source {
            ModLabelSource::Registry => tr("Sunrise registry"),
            ModLabelSource::Github => tr("GitHub"),
            ModLabelSource::Local => tr("Local file"),
        };
        trf("{name} · {source}", &[("source", source), ("name", name)])
    }
}

/// Player-facing labels for equal display names, returned in input order. A unique
/// name needs no replacement. For namesakes, try author, version and source alone,
/// then combine them. Break remaining ties with just the name and a number, ordered
/// by manifest ID, not display order.
pub fn colliding_mod_labels<'a>(
    mods: impl IntoIterator<Item = ModLabel<'a>>,
) -> Vec<Option<String>> {
    let mods: Vec<_> = mods.into_iter().collect();
    let mut names = HashMap::new();
    for item in &mods {
        *names.entry(item.name).or_insert(0) += 1;
    }
    let mut used = HashSet::new();
    let mut pending = Vec::new();
    let mut labels = vec![None; mods.len()];
    for (i, item) in mods.iter().enumerate() {
        if names[item.name] == 1 {
            used.insert(item.name.to_owned());
        } else {
            pending.push(i);
        }
    }

    // Compare rendered labels, not just the fields: e.g. one author named "A, B"
    // displays the same as two authors named "A" and "B".
    let mut assign = |candidate: &dyn Fn(&ModLabel<'_>) -> Option<String>| {
        let mut proposals = HashMap::new();
        for &i in &pending {
            if let Some(label) = candidate(&mods[i]) {
                proposals
                    .entry(label)
                    .and_modify(|slot| *slot = None)
                    .or_insert(Some(i));
            }
        }
        for (label, row) in proposals {
            if let Some(i) = row {
                if used.insert(label.clone()) {
                    labels[i] = Some(label);
                }
            }
        }
        pending.retain(|&i| labels[i].is_none());
    };
    assign(&|item| item.with_author());
    assign(&|item| item.with_version(item.name));
    assign(&|item| Some(item.with_source(item.name)));
    assign(&|item| {
        item.with_author()
            .and_then(|author| item.with_version(&author))
    });
    let all_details = |item: &ModLabel<'_>| {
        let author = item.with_author();
        let detail = author.as_deref().unwrap_or(item.name);
        let version = item.with_version(detail);
        item.with_source(version.as_deref().unwrap_or(detail))
    };
    assign(&|item| Some(all_details(item)));

    pending.sort_by_key(|&i| mods[i].id);
    let mut ordinals = HashMap::new();
    for i in pending {
        let name = mods[i].name;
        let ordinal = ordinals.entry(name).or_insert(0usize);
        loop {
            *ordinal += 1;
            let label = trf(
                "{label} · {n}",
                &[("n", &ordinal.to_string()), ("label", name)],
            );
            if used.insert(label.clone()) {
                labels[i] = Some(label);
                break;
            }
        }
    }
    labels
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
            mod_id: None,
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
            repository: None,
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

/// Why a pack installed from Sunrise's first registry can't be updated: that registry is gone,
/// and the signed one has no packs.
pub fn old_registry_pack(title: &str) -> String {
    crate::i18n::trf(
        "{title} came from Sunrise's old mod registry, which has closed, so it can't be updated. It keeps working as it is.",
        &[("title", title)],
    )
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
        return Err(old_registry_pack(&entry.title));
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
    let fetched = source
        .fetch(file, &folder, progress)
        .map(|zip| (zip, version.number.clone()));
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
