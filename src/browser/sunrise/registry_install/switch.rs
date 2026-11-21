//! Switch plans against an exact prepared build and keeps the result private through approval.
use super::*;
use crate::{
    managed_play::{Active, CandidateImage, Held, HeldReason, HeldRoot, HeldSource},
    worlds::switch::{Fence, Question, TargetAge},
};
use std::{io, path::PathBuf};
use sunrise_launcher_smod::dependencies::parse_version;

pub struct SwitchHooks<'a> {
    pub cancellation: &'a native_mods::Cancellation,
    pub progress: &'a dyn Fn(InstallProgress),
    pub registry_crates: Option<&'a dyn Fn(&native_mods::Consent) -> bool>,
    pub approve: &'a dyn Fn(&SwitchQuestion) -> Result<bool, String>,
    pub at_cutoff: &'a dyn Fn() -> Result<(), String>,
}

/// The complete decision for one prepared candidate, never a request to update saves.
#[derive(Clone, Debug)]
pub struct SwitchQuestion {
    pub build: String,
    pub worlds: Question,
    pub held: Vec<String>,
    pub unproved: Vec<String>,
    pub removed: Vec<(String, String)>,
}

impl SwitchQuestion {
    pub fn hold_lines(&self) -> impl Iterator<Item = String> + '_ {
        self.held
            .iter()
            .map(|name| {
                trf(
                    "{mod} will be turned off. Needs an update from its author.",
                    &[("mod", name)],
                )
            })
            .chain(self.unproved.iter().map(|name| {
                trf(
                    "{mod} will be turned off. Fit could not be verified.",
                    &[("mod", name)],
                )
            }))
    }
}

pub enum Switched {
    AlreadySelected,
    Stayed,
    Changed(Box<Active>),
}

/// A positive picker note is a read-only check of the currently installed content
/// artifacts. Native SDK fit, possible replacements and ambiguous input stay unknown.
pub fn checked_switch_fit(home: &Path, tag: &str) -> bool {
    let prove = || -> Option<()> {
        let source = location();
        let local = super::super::local();
        if source != super::super::INDEX_URL && !(cfg!(debug_assertions) && local) {
            return None;
        }
        let (catalogue, _, official) = super::super::complete_index(&source, local).ok()?;
        let active = managed_play::display_active(home).ok()??;
        let game = backend::switch_game_version(tag).ok()?;
        let exact = Originals::read(home, Some(&active), &catalogue, official.as_ref()).ok()?;
        if !exact.removed.is_empty() {
            return None;
        }
        for (id, old) in &exact.mods {
            if old.held
                || old.withheld
                || old.seen.package.manifest.native.is_some()
                || !old.seen.registry_claim
            {
                return None;
            }
            let digest = old.seen.digest.as_deref()?;
            let project = catalogue.project(id)?;
            let version = project.versions.iter().find(|v| {
                v.file.sha256 == digest
                    && v.game == game
                    && v.side != Some(Side::Server)
                    && v.capabilities.unknown().is_empty()
            })?;
            if project.kind != index::Kind::Mod || catalogue.revoked(id, digest).is_some() {
                return None;
            }
            index::check_file_with_facts(&catalogue, project, version, &old.seen.package.path)
                .ok()?;
        }
        validate_enabled(home, &catalogue, official.as_ref()).ok()?;
        if !active.matches_inputs(home)
            || managed_play::display_active(home).ok()?.as_ref() != Some(&active)
        {
            return None;
        }
        Some(())
    };
    prove().is_some()
}

pub fn switch_build(
    instance: &Instance,
    identity: Identity,
    tag: &str,
    hooks: SwitchHooks<'_>,
) -> Result<Switched, String> {
    if instance.kind == InstanceKind::Pack {
        return Err(tr("Choose an ordinary profile to switch builds.").into());
    }
    (hooks.progress)(InstallProgress::Checking);
    let mut profile =
        Profile::open_for(&instance.home, None, Some(identity)).map_err(String::from)?;
    let Some(edit) = profile.begin_switch(tag)? else {
        return Ok(Switched::AlreadySelected);
    };
    let game = backend::switch_game_version(edit.base())?;
    let source = location();
    let local = super::super::local();
    if source != super::super::INDEX_URL && !(cfg!(debug_assertions) && local) {
        return Err(tr("Connect to the Sunrise registry before switching builds.").into());
    }
    let (catalogue, evidence, official) = super::super::complete_index(&source, local)?;
    let old = edit.candidate()?;
    let original = Originals::read(old.path(), edit.current(), &catalogue, official.as_ref())?;
    let download = tempfile::Builder::new()
        .prefix("switch-download-")
        .tempdir_in(old.path())
        .map_err(|e| at(old.path(), e))?;
    let choices = choices(
        &original,
        &catalogue,
        local,
        &game,
        download.path(),
        hooks.progress,
    )?;
    let mut selected = None;
    let mut unproved = BTreeSet::new();
    best_survivor_set(&choices, &original.mods, |set| {
        (hooks.progress)(InstallProgress::Preparing);
        let Some(enabled) = enabled_set(&set, &original.mods) else {
            return Ok(false);
        };
        let next = edit.candidate()?;
        apply(next.path(), &original, &set, &enabled)?;
        validate_enabled(next.path(), &catalogue, official.as_ref()).map_err(|e| e.to_string())?;
        let prove = |home: &Path| {
            edit.prove(
                home,
                instance.native_profile,
                hooks.cancellation,
                |step| {
                    super::build_progress(step, hooks.progress);
                },
                hooks.registry_crates,
            )
        };
        let Some(proof) = exact_image(
            prove(next.path()),
            &set,
            &enabled,
            hooks.cancellation,
            &mut unproved,
        )?
        else {
            return Ok(false);
        };
        // An intentionally off native choice still needs proof for its legitimate activation.
        // It may activate its required off dependencies, but never evict another enabled mod.
        for (&id, package) in &set {
            if package.manifest.native.is_none() || enabled.contains(id) {
                continue;
            }
            let mut activated = enabled.clone();
            if !activate(id, &set, &mut activated) || !fits(&set, &activated) {
                return Ok(false);
            }
            let trial = edit.candidate()?;
            apply(trial.path(), &original, &set, &activated)?;
            if exact_image(
                prove(trial.path()),
                &set,
                &activated,
                hooks.cancellation,
                &mut unproved,
            )?
            .is_none()
            {
                return Ok(false);
            }
        }
        selected = Some((next, proof, set));
        Ok(true)
    })?;
    let (next, proof, set) = selected.ok_or_else(unproved_switch)?;
    let mut holds = Vec::new();
    let mut held = Vec::new();
    let mut unknown = Vec::new();
    for (id, old) in &original.mods {
        let manifest = &old.seen.package.manifest;
        let chosen = set.get(id.as_str());
        if chosen.is_none() && !original.removed.contains_key(id) && !old.held {
            let path = &old.seen.package.path;
            let directory = path
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str());
            let root = [HeldRoot::Native, HeldRoot::Content]
                .into_iter()
                .find(|root| Some(root.directory()) == directory)
                .ok_or_else(unproved_switch)?;
            let leaf = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(unproved_switch)?;
            let digest = old.seen.digest.as_deref().ok_or_else(|| {
                trf(
                    "Pack {mod} as an .smod before switching so its original bytes can be kept.",
                    &[("mod", &manifest.name)],
                )
            })?;
            let reason = if unproved.contains(id) {
                unknown.push(manifest.name.clone());
                HeldReason::Unproved { build: tag.into() }
            } else {
                held.push(manifest.name.clone());
                HeldReason::NoFit { build: tag.into() }
            };
            holds.push(Held::new(
                id.clone(),
                manifest.name.replace(char::is_control, " "),
                sha256_bytes(digest)?,
                (root, leaf.into()),
                old.source.clone(),
                old.on,
                reason,
            )?);
        }
    }
    let affected = crate::worlds::unproved_mod_effects(
        old.path(),
        edit.current(),
        next.path(),
        // The staged image proves compatibility with the target, not that the old
        // game and the new one interpret saved definitions in the same way.
        None,
        original.removed.keys().map(String::as_str).chain(
            set.iter()
                .filter(|(_, package)| package.replacement.is_some())
                .map(|(&id, _)| id),
        ),
    )?;
    let resources = edit.switch_resources()?;
    let mut order = None;
    let mut compare = |commit: &str| {
        order
            .get_or_insert_with(|| backend::WorldBuildOrder::for_target(tag))
            .compare_saved(commit)
    };
    let (fence, worlds) = Fence::hold(
        &instance.home,
        affected,
        |id| &original.mods[id].seen.package.manifest.name,
        Some(TargetAge {
            data_version: backend::prepared_world_version(&resources),
            compare_saved: &mut compare,
        }),
    )?;
    let question = SwitchQuestion {
        build: tag.into(),
        worlds,
        held,
        unproved: unknown,
        removed: original.removed.values().cloned().collect(),
    };
    if question.worlds.needed() && !(hooks.approve)(&question)? {
        return Ok(Switched::Stayed);
    }
    let active = edit.publish_switch(proof, hooks.cancellation, holds, original.removed, || {
        let (_, current, proof) = super::super::complete_index(&source, local)?;
        if current != evidence || proof.is_some() != official.is_some() {
            return Err(tr("The mod registry changed while the switch was being prepared. Nothing was switched.").into());
        }
        fence.revalidate(&instance.home)
    }, || {
        (hooks.at_cutoff)()?;
        (hooks.progress)(InstallProgress::Publishing);
        Ok(())
    })?;
    Ok(Switched::Changed(Box::new(active)))
}

struct Original {
    seen: SeenPackage,
    source: HeldSource,
    on: bool,
    held: bool,
    withheld: bool,
}

struct Originals<'a> {
    home: &'a Path,
    mods: BTreeMap<String, Original>,
    removed: BTreeMap<String, (String, String)>,
}

impl<'a> Originals<'a> {
    fn read(
        home: &'a Path,
        active: Option<&Active>,
        catalogue: &index::Index,
        official: Option<&super::super::OfficialRegistry>,
    ) -> Result<Self, String> {
        let selection = home.join(sunrise_launcher_smod::mod_list::FILE);
        match fs::read_to_string(&selection) {
            Ok(_) => (),
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(at(&selection, e)),
        }
        let doc = Document::read(home)?;
        let mut mods = BTreeMap::new();
        let mut removed = BTreeMap::new();
        for (id, seen) in inventory(home, official)? {
            let manifest = &seen.package.manifest;
            let origin_at = crate::github_mods::origin_path(home, &id);
            let github = match fs::symlink_metadata(&origin_at) {
                Ok(meta) if meta.is_file() => {
                    Some(crate::github_mods::origin(home, &id).ok_or_else(unproved_switch)?)
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                _ => return Err(unproved_switch()),
            };
            let has_row = doc.row(&id)?.is_some();
            if has_row != seen.registry_claim || (has_row && github.is_some()) {
                return Err(unproved_switch());
            }
            // The registry row selects a checked destination, not its publisher. Only the
            // actual digest in the official catalogue can authorize destroying old bytes.
            let source = if let Some(origin) = github {
                if origin.id != id
                    || origin.version != manifest.version
                    || seen.digest.as_deref() != Some(&origin.sha256)
                {
                    return Err(unproved_switch());
                }
                HeldSource::Github {
                    owner: origin.owner,
                    repo: origin.repo,
                    commit: origin.commit,
                    version: origin.version,
                }
            } else if seen.owned {
                HeldSource::Registry {
                    project: id.clone(),
                    version: manifest.version.clone(),
                }
            } else {
                HeldSource::Local {
                    version: manifest.version.clone(),
                }
            };
            let on = mod_list::is_enabled(home, &id);
            let held = active.and_then(|active| active.holds().find(|held| held.id() == id));
            if let Some(hold) = held {
                // Registry authorization can disappear or arrive on a later request. It
                // doesn't change the immutable identity of an already held exact archive.
                let same_source = &source == hold.source()
                    || matches!(
                        (&source, hold.source()),
                        (HeldSource::Registry { version: a, .. }, HeldSource::Local { version: b })
                            | (HeldSource::Local { version: a }, HeldSource::Registry { version: b, .. }) if a == b
                    );
                if on
                    || !same_source
                    || home.join(hold.path()) != seen.package.path
                    || seen
                        .digest
                        .as_deref()
                        .map(sha256_bytes)
                        .transpose()?
                        .as_ref()
                        != Some(hold.digest())
                {
                    return Err(unproved_switch());
                }
            }
            let revoked = seen.digest.as_deref().and_then(|digest| {
                let listed = catalogue.revoked(&id, digest)?;
                let authorized = seen.owned
                    || (!seen.registry_claim
                        && official.is_some_and(|proof| proof.proves_archive(&id, digest)));
                (authorized || seen.registry_claim || listed.sha256.is_some())
                    .then_some((listed, authorized))
            });
            if let Some((revoked, true)) = revoked {
                removed.insert(id.clone(), (manifest.name.clone(), revoked.reason.clone()));
            }
            mods.insert(
                id,
                Original {
                    seen,
                    source,
                    on,
                    held: held.is_some(),
                    withheld: revoked.is_some_and(|(_, authorized)| !authorized),
                },
            );
        }
        if active.is_some_and(|active| active.holds().any(|hold| !mods.contains_key(hold.id()))) {
            return Err(unproved_switch());
        }
        Ok(Self {
            home,
            mods,
            removed,
        })
    }
}

struct Package {
    manifest: Manifest,
    digest: Option<String>,
    replacement: Option<(PathBuf, installed::Entry)>,
    unlisted: bool,
}

type Choices = BTreeMap<String, Vec<Package>>;
type Set<'a> = BTreeMap<&'a str, &'a Package>;

fn choices(
    original: &Originals<'_>,
    catalogue: &index::Index,
    local: bool,
    game: &str,
    root: &Path,
    progress: &dyn Fn(InstallProgress),
) -> Result<Choices, String> {
    let mut todo: BTreeSet<_> = original.mods.keys().cloned().collect();
    let mut slots = BTreeMap::new();
    let mut fetched = 0;
    let doc = Document::read(original.home)?;
    while let Some(id) = todo.pop_first() {
        if slots.contains_key(&id) {
            continue;
        }
        if slots.len() == 128 {
            return Err(too_many());
        }
        let old = original.mods.get(&id);
        let mut available = Vec::new();
        if original.removed.contains_key(&id) || old.is_some_and(|old| old.held || old.withheld) {
            // Preserve unproved takedown bytes, off. Only an explicit Update on the
            // current base can restore a prior hold.
        } else if let Some(old) = old.filter(|old| !old.seen.owned) {
            let manifest = &old.seen.package.manifest;
            // A complete index can prove fit for unchanged listed content. Absence of these
            // exact bytes from a present project instead means they're a pinned local input.
            let mut pinned = true;
            if old.seen.registry_claim {
                let project = catalogue.project(&id).ok_or_else(unproved_switch)?;
                let mut exact = project
                    .versions
                    .iter()
                    .filter(|version| old.seen.digest.as_deref() == Some(&version.file.sha256))
                    .peekable();
                pinned = exact.peek().is_none();
                if !pinned {
                    let release = exact.find(|version| {
                        version.side != Some(Side::Server)
                            && (manifest.native.is_some() || version.game == game)
                    });
                    let Some(version) = release else {
                        slots.insert(id, available);
                        continue;
                    };
                    index::check_file_with_facts(
                        catalogue,
                        project,
                        version,
                        &old.seen.package.path,
                    )?;
                }
            }
            if manifest.side != Side::Server && manifest.api == sunrise_launcher_smod::manifest::API
            {
                available.push(Package {
                    manifest: manifest.clone(),
                    digest: old.seen.digest.clone(),
                    replacement: None,
                    unlisted: pinned,
                });
            }
        } else if let Some(project) = catalogue.project(&id) {
            if !matches!(project.kind, index::Kind::Mod | index::Kind::NativeMod) {
                return Err(unproved_switch());
            }
            let root_name = if project.kind == index::Kind::NativeMod {
                HeldRoot::Native
            } else {
                HeldRoot::Content
            }
            .directory();
            for version in &project.versions {
                if version.side == Some(Side::Server)
                    || project.kind != index::Kind::NativeMod && version.game != game
                    || catalogue.revoked(&id, &version.file.sha256).is_some()
                {
                    continue;
                }
                if !version.capabilities.unknown().is_empty() {
                    return Err(unproved_switch());
                }
                let same =
                    old.filter(|old| old.seen.digest.as_deref() == Some(&version.file.sha256));
                let (file, replacement) = if let Some(old) = same {
                    (old.seen.package.path.clone(), None)
                } else {
                    fetched += 1;
                    if fetched > MAX_TRIALS {
                        return Err(too_many());
                    }
                    let dir = root.join(fetched.to_string());
                    let path = old
                        .map(|old| relative(&old.seen.package.path, original.home))
                        .transpose()?
                        .unwrap_or_else(|| Path::new(root_name).join(format!("{id}.smod")));
                    let filename = path
                        .to_str()
                        .ok_or_else(unproved_switch)?
                        .replace('\\', "/");
                    if !path.starts_with(root_name) || old.is_none() && doc.row(&id)?.is_some() {
                        return Err(unproved_switch());
                    }
                    doc.rows
                        .check_other_claims(original.home, &id, &filename)
                        .map_err(|e| e.render(trf))?;
                    let file = fetch(
                        &version_of(catalogue, project, version).files[0],
                        &dir,
                        local,
                        &|f| progress(InstallProgress::Downloading(f)),
                    )?;
                    let entry = installed::Entry {
                        source: SourceId::Sunrise,
                        project: id.clone(),
                        slug: id.clone(),
                        title: project.name.clone(),
                        kind: Kind::Mod,
                        version: version.version.clone(),
                        number: version.version.clone(),
                        file: filename,
                        checksum: format!("SHA-256:{}", version.file.sha256),
                        page: format!("{}/{}", super::super::SITE, id),
                        dependency: old.is_none()
                            || doc
                                .row(&id)?
                                .is_some_and(|row| row["dependency"].as_bool() == Some(true)),
                        verdict: None,
                        install_hold: None,
                    };
                    (file.clone(), Some((file, entry)))
                };
                let (trust, facts) =
                    index::check_file_with_facts(catalogue, project, version, &file)?;
                if version.dependencies.is_none()
                    && facts
                        .dependencies
                        .values()
                        .any(|need| !need.details().optional)
                {
                    return Err(unproved_switch());
                }
                let manifest = sunrise_launcher_smod::manifest::Package::read(&file)
                    .map_err(|e| at(&file, e))?
                    .manifest;
                let replacement = replacement.map(|(file, mut entry)| {
                    entry.verdict = Some(trust_label(trust).into());
                    (file, entry)
                });
                available.push(Package {
                    manifest,
                    digest: Some(version.file.sha256.clone()),
                    replacement,
                    unlisted: false,
                });
            }
        } else if old.is_some() {
            // A complete response can still omit a project. Omission doesn't prove
            // an installed archive incompatible (and must not turn it off).
            return Err(unproved_switch());
        }
        available.sort_by(|a, b| {
            parse_version(&b.manifest.version)
                .ok()
                .cmp(&parse_version(&a.manifest.version).ok())
                .then_with(|| a.digest.cmp(&b.digest))
        });
        for choice in &available {
            todo.extend(required(&choice.manifest).map(|(id, _)| id.clone()));
        }
        slots.insert(id, available);
    }
    Ok(slots)
}

fn relative(path: &Path, home: &Path) -> Result<PathBuf, String> {
    path.strip_prefix(home)
        .map(Path::to_owned)
        .map_err(|_| unproved_switch())
}

pub(super) fn sha256_bytes(hex: &str) -> Result<[u8; 32], String> {
    let mut bytes = [0; 32];
    if hex.len() != 64 || !hex.is_ascii() {
        return Err(unproved_switch());
    }
    for (i, out) in bytes.iter_mut().enumerate() {
        *out = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| unproved_switch())?;
    }
    Ok(bytes)
}

fn required(
    manifest: &Manifest,
) -> impl Iterator<Item = (&String, &sunrise_launcher_smod::manifest::Dependency)> {
    manifest.dependencies.iter().filter(|(_, need)| {
        let need = need.details();
        !need.optional && need.side != Side::Server
    })
}

fn best_survivor_set<'a>(
    slots: &'a Choices,
    original: &BTreeMap<String, Original>,
    mut trial: impl FnMut(Set<'a>) -> Result<bool, String>,
) -> Result<(), String> {
    struct Search<'a> {
        slots: Vec<(&'a str, &'a [Package], bool)>,
        remaining: Vec<usize>,
        checked: usize,
    }
    impl<'a> Search<'a> {
        fn rank(
            &mut self,
            at: usize,
            retained: usize,
            chosen: &mut Set<'a>,
            trial: &mut impl FnMut(Set<'a>) -> Result<bool, String>,
        ) -> Result<bool, String> {
            if retained > self.remaining[at] {
                return Ok(false);
            }
            self.checked += 1;
            if self.checked > MAX_TRIALS * 128 {
                return Err(too_many());
            }
            let Some(&(id, choices, original)) = self.slots.get(at) else {
                let mut reachable = BTreeSet::new();
                for &(id, _, was) in &self.slots {
                    if was && chosen.contains_key(id) && !activate(id, chosen, &mut reachable) {
                        return Ok(false);
                    }
                }
                return if reachable.len() == chosen.len() {
                    trial(chosen.clone())
                } else {
                    Ok(false)
                };
            };
            if let Some(left) = retained.checked_sub(usize::from(original)) {
                for choice in choices {
                    chosen.insert(id, choice);
                    if self.rank(at + 1, left, chosen, trial)? {
                        return Ok(true);
                    }
                }
                chosen.remove(id);
            }
            self.rank(at + 1, retained, chosen, trial)
        }
    }
    let slots: Vec<_> = slots
        .iter()
        .map(|(id, choices)| (id.as_str(), choices.as_slice(), original.contains_key(id)))
        .collect();
    let mut remaining = vec![0; slots.len() + 1];
    for (i, (_, choices, was)) in slots.iter().enumerate().rev() {
        remaining[i] = remaining[i + 1] + usize::from(*was && !choices.is_empty());
    }
    let mut search = Search {
        slots,
        remaining,
        checked: 0,
    };
    let mut tried = 0;
    let mut checked_trial = |chosen| {
        tried += 1;
        if tried > MAX_TRIALS {
            return Err(too_many());
        }
        trial(chosen)
    };
    for retained in (0..=search.remaining[0]).rev() {
        if search.rank(0, retained, &mut BTreeMap::new(), &mut checked_trial)? {
            return Ok(());
        }
    }
    Err(unproved_switch())
}

fn activate<'a>(id: &'a str, set: &Set<'a>, selected: &mut BTreeSet<&'a str>) -> bool {
    if !selected.insert(id) {
        return true;
    }
    required(&set[id].manifest).all(|(dep, need)| {
        set.get_key_value(dep.as_str()).is_some_and(|(&key, pkg)| {
            parse_version(&pkg.manifest.version).is_ok_and(|v| need.details().version.matches(&v))
                && activate(key, set, selected)
        })
    })
}

fn enabled_set<'a>(
    set: &Set<'a>,
    original: &BTreeMap<String, Original>,
) -> Option<BTreeSet<&'a str>> {
    let mut enabled = BTreeSet::new();
    for &id in set.keys() {
        if original.get(id).is_some_and(|old| old.on) && !activate(id, set, &mut enabled) {
            return None;
        }
    }
    (enabled
        .iter()
        .all(|id| original.get(*id).is_none_or(|old| old.on))
        && fits(set, &enabled))
    .then_some(enabled)
}

fn fits(set: &Set<'_>, enabled: &BTreeSet<&str>) -> bool {
    let mut namespaces = BTreeSet::new();
    enabled.iter().all(|id| {
        let manifest = &set[id].manifest;
        manifest.owned_namespaces().all(|ns| namespaces.insert(ns))
            && manifest.incompatible.iter().all(|(other, rule)| {
                !enabled.contains(other.as_str())
                    || !parse_version(&set[other.as_str()].manifest.version)
                        .is_ok_and(|v| rule.details().version.matches(&v))
            })
    })
}

fn apply(
    home: &Path,
    original: &Originals<'_>,
    set: &Set<'_>,
    enabled: &BTreeSet<&str>,
) -> Result<(), String> {
    let mut doc = Document::read(home)?;
    for id in original.removed.keys() {
        let old = &original.mods[id];
        let target = home.join(relative(&old.seen.package.path, original.home)?);
        fs::remove_file(&target).map_err(|e| at(&target, e))?;
        if let HeldSource::Github { .. } = old.source {
            let path = crate::github_mods::origin_path(home, id);
            fs::remove_file(&path).map_err(|e| at(&path, e))?;
        }
        doc.remove(id)?;
    }
    for pkg in set.values() {
        if let Some((file, entry)) = &pkg.replacement {
            let target = home.join(&entry.file);
            let parent = target
                .parent()
                .expect("managed package destination has a root");
            fs::create_dir_all(parent).map_err(|e| at(parent, e))?;
            if !original.mods.contains_key(&entry.project)
                && target.try_exists().map_err(|e| at(&target, e))?
            {
                return Err(unproved_switch());
            }
            fs::copy(file, &target).map_err(|e| at(&target, e))?;
            doc.replace(entry)?;
        }
    }
    if !original.removed.is_empty() || set.values().any(|pkg| pkg.replacement.is_some()) {
        doc.write(home)?;
    }
    let off: Vec<_> = original
        .mods
        .iter()
        .filter(|(id, old)| old.on && !enabled.contains(id.as_str()))
        .map(|(id, _)| id.as_str())
        .collect();
    let on: Vec<_> = enabled
        .iter()
        .copied()
        .filter(|id| original.mods.get(*id).is_none_or(|old| !old.on))
        .collect();
    if !off.is_empty() {
        mod_list::set_all_enabled(home, &off, false)?
    }
    if !on.is_empty() {
        mod_list::set_all_enabled(home, &on, true)?
    }
    Ok(())
}

fn exact_image(
    result: Result<CandidateImage, native_mods::Failed>,
    set: &Set<'_>,
    enabled: &BTreeSet<&str>,
    cancellation: &native_mods::Cancellation,
    unproved: &mut BTreeSet<String>,
) -> Result<Option<CandidateImage>, String> {
    match result {
        Ok(image) => Ok(Some(image)),
        Err(native_mods::Failed::Mods {
            left,
            compiling: false,
            ..
        }) => {
            cancellation.check()?;
            let mut unfit = BTreeSet::new();
            let mut unknown = BTreeSet::new();
            for omission in &left {
                let (id, _) = set
                    .get_key_value(omission.id.as_str())
                    .filter(|(id, p)| {
                        enabled.contains(*id) && p.manifest.version == omission.version
                    })
                    .ok_or_else(unproved_switch)?;
                match &omission.why {
                    native_mods::Why::Unfit(_) => unfit.insert(*id),
                    native_mods::Why::Unproved(_) => unknown.insert(*id),
                    native_mods::Why::Needs(_) => continue,
                    _ => return Err(unproved_switch()),
                };
            }
            if unfit.is_empty() && unknown.is_empty() {
                return Err(unproved_switch());
            }
            // The builder only sees native bundles and reports one cause per omitted
            // dependent. Use the entire trial's required client closure, including content
            // and all of its causes. A known incompatibility alone defeats that trial.
            spread_to_dependents(set, &mut unfit);
            spread_to_dependents(set, &mut unknown);
            unproved.extend(unknown.difference(&unfit).map(|&id| id.to_owned()));
            Ok(None)
        }
        Err(native_mods::Failed::NoConsent) => {
            cancellation.check()?;
            // The trial could not check any native input. Only a wholly pinned local
            // input set can be left off without attempting a listed mod's exact build.
            let mut unknown = BTreeSet::new();
            for (&id, p) in set {
                if enabled.contains(id) && p.manifest.native.is_some() {
                    if !p.unlisted {
                        return Err(native_mods::Failed::NoConsent.into());
                    }
                    unknown.insert(id);
                }
            }
            if unknown.is_empty() {
                return Err(unproved_switch());
            }
            spread_to_dependents(set, &mut unknown);
            unproved.extend(unknown.into_iter().map(str::to_owned));
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

fn spread_to_dependents<'a>(set: &Set<'a>, omitted: &mut BTreeSet<&'a str>) {
    let mut before = 0;
    while before != omitted.len() {
        before = omitted.len();
        for (&id, package) in set {
            if required(&package.manifest).any(|(dep, _)| omitted.contains(dep.as_str())) {
                omitted.insert(id);
            }
        }
    }
}

fn unproved_switch() -> String {
    tr("Sunrise couldn't verify the full mod set for this build. Nothing was switched.").into()
}

fn too_many() -> String {
    tr("Not every compatible mod set could be checked. Nothing was switched.").into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_official_historical_bytes_allow_a_staged_replacement_or_takedown() {
        let id = "coords_hud";
        for (takedown, official) in [(false, false), (false, true), (true, false), (true, true)] {
            let before = tempfile::tempdir().unwrap();
            let candidate = tempfile::tempdir().unwrap();
            let download = tempfile::tempdir().unwrap();
            let row =
                super::super::tests::archive(before.path(), id, "Coordinates HUD", "0.0.1", false);
            let archive = before.path().join(&row.file);
            let old = fs::read(&archive).unwrap();
            let mut catalogue = super::super::super::tests::sample();
            let project = catalogue.projects.iter_mut().find(|p| p.id == id).unwrap();
            let mut historical = project.versions[0].clone();
            historical.game = "26.2".into();
            historical.version = "0.0.1".into();
            historical.file.sha256 = index::sha256(&old);
            project.versions.push(historical);
            if takedown {
                catalogue.revoked.push(index::Revoked {
                    id: id.into(),
                    sha256: Some(index::sha256(&old)),
                    reason: "Fixture withdrawal".into(),
                    date: "2026-11-20".into(),
                });
            }
            // The claim locates the file; neither its recorded version nor its checksum proves
            // the archive. An untracked exact takedown is also allowed by the official proof.
            fs::create_dir_all(candidate.path().join(mod_list::MODS_DIR)).unwrap();
            fs::write(candidate.path().join(&row.file), &old).unwrap();
            mod_list::set_enabled(before.path(), id, true).unwrap();
            mod_list::set_enabled(candidate.path(), id, true).unwrap();
            if !takedown {
                let mut stale = row.clone();
                stale.version = "0.1.0".into();
                stale.checksum = format!("SHA-256:{}", "0".repeat(64));
                let mut doc = Document::read(before.path()).unwrap();
                doc.replace(&stale).unwrap();
                doc.write(before.path()).unwrap();
                doc.write(candidate.path()).unwrap();
            }
            let authority = official.then(|| OfficialRegistry::fixture(&catalogue));
            let originals =
                Originals::read(before.path(), None, &catalogue, authority.as_ref()).unwrap();
            let options = choices(
                &originals,
                &catalogue,
                true,
                "26.3",
                download.path(),
                &|_| {},
            )
            .unwrap();
            best_survivor_set(&options, &originals.mods, |set| {
                apply(
                    candidate.path(),
                    &originals,
                    &set,
                    &enabled_set(&set, &originals.mods).unwrap(),
                )?;
                Ok(true)
            })
            .unwrap();
            let changed = candidate.path().join(&row.file);
            match (takedown, official) {
                (true, true) => assert!(!changed.exists()),
                (false, true) => assert_eq!(
                    index::sha256_file(&changed).unwrap(),
                    catalogue.project(id).unwrap().versions[0].file.sha256
                ),
                (_, false) => {
                    assert_eq!(fs::read(&changed).unwrap(), old);
                    assert!(!mod_list::is_enabled(candidate.path(), id));
                }
            }
            assert_eq!(fs::read(archive).unwrap(), old);
        }
    }
}
