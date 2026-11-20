//! Stage the complete replacement set, hold save evidence, then publish exactly that set.
use super::{build, unsettled, Active, Lease, SwitchEffects};
use crate::{
    backend,
    browser::{self, installed, sunrise},
    i18n::tr,
    native_mods, worlds,
};
use semver::Version;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};
use sunrise_launcher_smod::{
    index,
    manifest::{self, Manifest},
    mod_list,
};

const MAX_TRIALS: usize = 20_000;

#[derive(Clone)]
pub(super) struct Package {
    pub(super) manifest: Manifest,
    pub(super) digest: String,
    pub(super) relative: PathBuf,
    file: PathBuf,
    pub(super) row: Option<installed::Entry>,
    pub(super) on: bool,
}

pub(super) struct Inventory {
    pub(super) packages: BTreeMap<String, Package>,
    pub(super) browser: serde_json::Value,
    selection: Option<String>,
}

/// A Switch keeps the gate and every affected world locked from the plan through cutoff.
/// The only caller permission it accepts is committing this very value, or dropping it.
pub struct Switch {
    owner: Arc<Lease>,
    basis: Basis,
    next: Active,
    initial_inputs: [u8; 32],
    index: String,
    cancellation: native_mods::Cancellation,
    fence: worlds::switch::Fence,
    question: worlds::SwitchQuestion,
    effects: SwitchEffects,
    live: bool,
}

enum Basis {
    Committed(Active),
    First(build::Bootstrap),
}

impl Switch {
    /// A same-base selection isn't a migration. Otherwise reserve before downloads or builds.
    pub fn stage(
        owner: Arc<Lease>,
        tag: &str,
        profile: native_mods::Profile,
        progress: impl Fn(native_mods::Step),
        registry_consent: &dyn Fn(&native_mods::Consent) -> bool,
    ) -> Result<Option<Self>, String> {
        build::validate_tag(tag)?;
        let prior = owner.active_build()?;
        if prior.as_ref().is_some_and(|prior| prior.base() == tag) {
            return Ok(None);
        }
        if native_mods::is_native(tag) {
            return Err(unsettled());
        }
        backend::need_prepared(tag)?;
        let (root, basis) = match prior {
            Some(prior) => {
                prior.verify_artifacts()?;
                (owner.reserve_switch(&prior, tag)?, Basis::Committed(prior))
            }
            None => {
                let target =
                    super::Target::first_play(tag, |_| Err(backend::no_version_installed()))?;
                let (root, binding) = owner.reserve_first_switch(&target)?;
                (root, Basis::First(binding))
            }
        };
        let compiler = CandidateBuild {
            profile,
            cancellation: native_mods::Cancellation::for_profile(owner.clone()),
            registry_consent: Some(registry_consent),
        };
        match Self::plan(owner.clone(), basis, tag, compiler, &root, progress) {
            Ok(plan) => Ok(Some(plan)),
            Err(error) => {
                owner.recover_abandoned()?;
                Err(error)
            }
        }
    }

    fn plan(
        owner: Arc<Lease>,
        basis: Basis,
        tag: &str,
        compiler: CandidateBuild<'_>,
        root: &Path,
        progress: impl Fn(native_mods::Step),
    ) -> Result<Self, String> {
        let home = owner.home();
        let initial_inputs = build::inputs(home)?;
        match &basis {
            Basis::Committed(prior) if initial_inputs != prior.inputs => return Err(tr("The profile's mods have changed since it last played. Play it before switching builds.").into()),
            Basis::First(binding) => binding.verify(home)?,
            Basis::Committed(_) => {}
        }
        let source = Inventory::read(home)?;
        let game = backend::game_of_build(tag).ok_or_else(unsettled)?;
        let (catalogue, index) = sunrise::authoritative_index()?;
        let slots = candidates(&source, &catalogue, &game, root)?;
        let mut native_refusals = BTreeMap::new();
        let mut solved = None;
        best_survivor_set(&slots, &source, |chosen| {
            let prepared = tempfile::Builder::new()
                .prefix("candidate-")
                .tempdir_in(root)
                .map_err(|e| bad(root, e))?;
            stage_set(home, prepared.path(), &source, &chosen)?;
            match build_candidate(tag, prepared.path(), &compiler, &progress, &chosen) {
                Ok(image) => {
                    solved = Some((chosen, prepared, image));
                    return Ok(true);
                }
                Err(native_mods::Failed::Mods { left, error, .. })
                    if !left.is_empty()
                        && left
                            .iter()
                            .all(|mod_| chosen.contains_key(mod_.id.as_str())) =>
                {
                    for mod_ in left {
                        native_refusals.insert(mod_.id, error.clone());
                    }
                }
                Err(error) => return Err(error.to_string()),
            }
            Ok(false)
        })?;
        let (mut chosen, prepared, image) = solved.ok_or_else(unsettled)?;
        let prior_waiting = build::waiting_packages(home)?;
        let (disabled, waiting, effects) =
            outcomes(&source, &chosen, &catalogue, &prior_waiting, |id| {
                if let Some(error) = native_refusals.get(id) {
                    error.clone()
                } else if slots[id].is_empty() {
                    tr("No compatible version was found for this game build.").into()
                } else {
                    tr("It cannot stay with the other mods on this build.").into()
                }
            })?;
        // The build trial proved the surviving enabled set. Keep incompatible exact packages
        // disabled in the final candidate; they must never enter an activation trial.
        if !disabled.is_empty() {
            chosen.extend(disabled.iter().map(|(id, package)| (id.as_str(), package)));
            stage_set(home, prepared.path(), &source, &chosen)?;
        }
        let changed: Vec<_> = source
            .packages
            .iter()
            .filter(|(id, pkg)| {
                pkg.on
                    && chosen.get(id.as_str()).is_none_or(|next| {
                        !next.on
                            || next.digest != pkg.digest
                            || pkg.manifest.native.is_some()
                            || !pkg.manifest.dependencies.is_empty()
                    })
            })
            .map(|(id, old)| worlds::switch::Changed {
                id,
                title: &old.manifest.name,
                namespaces: &old.manifest.namespaces,
            })
            .collect();
        let next = match &basis {
            Basis::Committed(prior) => {
                prior.switched(tag, &image, prepared.path(), effects.clone(), waiting)?
            }
            Basis::First(binding) => binding
                .first_generation(&image, prepared.path(), build::inputs(prepared.path())?)?
                .with_switch_effects(prepared.path(), effects.clone(), waiting)?,
        };
        let mut order = backend::WorldBuildOrder::for_target(tag);
        let (fence, question) = worlds::switch::Fence::hold(
            home,
            &changed,
            Some(worlds::switch::TargetAge {
                data_version: backend::prepared_world_version(&next.resources()),
                compare_saved: &mut |commit| order.compare_saved(commit),
            }),
        )?;
        if build::inputs(home)? != initial_inputs {
            return Err(unsettled());
        }
        let files = changes(home, prepared.path(), &source, &chosen)?;
        owner.seal_staged_set(&next, &files)?;
        Ok(Self {
            owner,
            basis,
            next,
            initial_inputs,
            index,
            cancellation: compiler.cancellation,
            fence,
            question,
            effects,
            live: true,
        })
    }

    pub fn home(&self) -> &Path {
        self.owner.home()
    }

    pub fn target(&self) -> &str {
        self.next.base()
    }
    pub fn prior(&self) -> Option<&str> {
        match &self.basis {
            Basis::Committed(prior) => Some(prior.base()),
            Basis::First(_) => None,
        }
    }
    pub fn question(&self) -> &worlds::SwitchQuestion {
        &self.question
    }
    pub fn effects(&self) -> &SwitchEffects {
        &self.effects
    }

    pub fn commit(
        mut self,
        cutoff: impl FnOnce() -> Result<(), String>,
    ) -> Result<Option<super::Receipt>, String> {
        let home = self.owner.home();
        let (_, index) = sunrise::authoritative_index()?;
        if index != self.index {
            return Err(tr("The mod registry changed while the switch was being prepared. Nothing was switched.").into());
        }
        self.fence.revalidate(home)?;
        if build::inputs(home)? != self.initial_inputs {
            return Err(unsettled());
        }
        self.next.verify_artifacts()?;
        self.owner.publish_staged_set(|| {
            match &self.basis {
                Basis::Committed(prior) => {
                    prior.verify_artifacts()?;
                    prior.verify_resources()?;
                }
                Basis::First(binding) => binding.verify_artifact()?,
            }
            self.next.verify_resources()?;
            cutoff()?;
            self.cancellation.publication_guard(self.next.executable())
        })?;
        self.live = false;
        Ok((!self.effects.is_empty()).then(|| {
            self.next
                .receipts()
                .last()
                .expect("new switch receipt")
                .clone()
        }))
    }
}

impl Drop for Switch {
    fn drop(&mut self) {
        if self.live {
            if let Err(error) = self.owner.recover_abandoned() {
                eprintln!("{error}");
            }
        }
    }
}

impl Inventory {
    pub(super) fn read(home: &Path) -> Result<Self, String> {
        build::verify_packages(home)?;
        let browser = match build::optional_bytes(&home.join(installed::MANIFEST))? {
            None => serde_json::json!({ "entries": [] }),
            Some(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| bad(&home.join(installed::MANIFEST), e))?,
        };
        let rows = browser
            .get("entries")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(unsettled)?;
        let mut tracked = BTreeMap::new();
        for raw in rows {
            let Some(file) = raw.get("file").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let path = PathBuf::from(file);
            if !(path.starts_with(mod_list::DIR) || path.starts_with(mod_list::NATIVE_DIR)) {
                continue;
            }
            let row: installed::Entry =
                serde_json::from_value(raw.clone()).map_err(|e| e.to_string())?;
            if !installed::in_folders(&row.file)
                || row.kind != browser::Kind::Mod
                || tracked.insert(path, row).is_some()
            {
                return Err(unsettled());
            }
        }
        let mut packages = BTreeMap::new();
        for folder in [mod_list::DIR, mod_list::NATIVE_DIR] {
            let dir = home.join(folder);
            let entries = match fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(bad(&dir, e)),
            };
            for entry in entries {
                let file = entry.map_err(|e| bad(&dir, e))?.path();
                if file.extension().is_none_or(|ext| ext != "smod") {
                    continue;
                }
                let relative = file
                    .strip_prefix(home)
                    .map_err(|e| e.to_string())?
                    .to_owned();
                let row = tracked.remove(&relative);
                let manifest = manifest::Package::read(&file)
                    .map_err(|e| bad(&file, e))?
                    .manifest;
                let id = manifest.id.clone();
                if row.as_ref().is_some_and(|row| {
                    row.source != browser::SourceId::Sunrise
                        || sunrise::mod_id(row) != Some(&id)
                        || row.version != manifest.version
                }) {
                    return Err(unsettled());
                }
                let on = browser::mod_list::is_enabled(home, &id);
                let digest = index::sha256_file(&file)?;
                let pkg = Package {
                    manifest,
                    digest,
                    relative,
                    file,
                    row,
                    on,
                };
                if packages.insert(id, pkg).is_some() {
                    return Err(unsettled());
                }
            }
        }
        if !tracked.is_empty() {
            return Err(unsettled());
        }
        let selection = build::optional_bytes(&home.join(mod_list::FILE))?
            .map(String::from_utf8)
            .transpose()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            packages,
            browser,
            selection,
        })
    }
}

type Outcomes = (
    BTreeMap<String, Package>,
    BTreeMap<String, build::WaitingMod>,
    SwitchEffects,
);

fn outcomes(
    source: &Inventory,
    chosen: &BTreeMap<&str, &Package>,
    catalogue: &index::Index,
    prior_waiting: &BTreeMap<String, build::WaitingMod>,
    reason: impl Fn(&str) -> String,
) -> Result<Outcomes, String> {
    let mut disabled = BTreeMap::new();
    let mut waiting = BTreeMap::new();
    let mut effects = SwitchEffects::default();
    for (id, old) in &source.packages {
        if chosen.contains_key(id.as_str()) {
            continue;
        }
        let origin = old.row.as_ref().map_or(id.as_str(), |row| &row.project);
        if let Some(revoked) = catalogue.revoked(origin, &old.digest) {
            effects
                .removed
                .push((old.manifest.name.clone(), revoked.reason.clone()));
            continue;
        }
        let reason = reason(id);
        let super::Image::Sha256(digest) = super::Image::at(&old.file)? else {
            return Err(unsettled());
        };
        waiting.insert(
            id.clone(),
            build::WaitingMod {
                file: old.relative.clone(),
                digest,
                reason: reason.clone(),
                was_on: old.on || prior_waiting.get(id).is_some_and(|old| old.was_on),
            },
        );
        effects
            .waiting_for_update
            .push((old.manifest.name.clone(), reason));
        let mut keep = old.clone();
        keep.on = false;
        disabled.insert(id.clone(), keep);
    }
    Ok((disabled, waiting, effects))
}

fn candidates(
    source: &Inventory,
    catalogue: &index::Index,
    game: &str,
    root: &Path,
) -> Result<BTreeMap<String, Vec<Package>>, String> {
    let mut pending: BTreeSet<String> = source.packages.keys().cloned().collect();
    let mut slots = BTreeMap::new();
    let mut downloads = 0;
    while let Some(id) = pending.pop_first() {
        if slots.contains_key(&id) {
            continue;
        }
        let mut choices = Vec::new();
        let old = source.packages.get(&id);
        if let Some(old) = old.filter(|old| old.row.is_none()) {
            if catalogue.revoked(&id, &old.digest).is_some() {
                slots.insert(id, choices);
                continue;
            }
            if old.manifest.api != manifest::API {
                return Err(unsettled());
            }
            choices.push(Package {
                manifest: old.manifest.clone(),
                digest: old.digest.clone(),
                relative: old.relative.clone(),
                file: old.file.clone(),
                row: None,
                on: old.on,
            });
        } else if let Some(project) = old
            .and_then(|p| p.row.as_ref())
            .map(|p| p.project.as_str())
            .or_else(|| old.is_none().then_some(id.as_str()))
            .and_then(|id| catalogue.project(id))
        {
            if !matches!(project.kind, index::Kind::Mod | index::Kind::NativeMod) {
                return Err(unsettled());
            }
            for version in &project.versions {
                if !sunrise::fits_game(version, game)
                    || catalogue
                        .revoked(&project.id, &version.file.sha256)
                        .is_some()
                {
                    continue;
                }
                downloads += 1;
                if downloads > MAX_TRIALS {
                    return Err(too_many());
                }
                let d = &version.file;
                let file = if let Some(pkg) =
                    old.filter(|old| old.digest.eq_ignore_ascii_case(&d.sha256))
                {
                    pkg.file.clone()
                } else {
                    browser::download::fetch(
                        &browser::FileRef {
                            url: d.url.clone(),
                            name: d.name.clone(),
                            size: d.size,
                            sha256: Some(d.sha256.clone()),
                            sha512: None,
                            sha1: None,
                            primary: true,
                        },
                        &root.join("registry").join(downloads.to_string()),
                        &|_| {},
                    )?
                };
                index::check_file(catalogue, project, version, &file)?;
                let manifest = manifest::Package::read(&file)
                    .map_err(|e| bad(&file, e))?
                    .manifest;
                if manifest.id != id {
                    return Err(unsettled());
                }
                let folder = if manifest.native.is_some() {
                    mod_list::NATIVE_DIR
                } else {
                    mod_list::DIR
                };
                let relative = Path::new(folder).join(format!("{id}.smod"));
                let row = installed::Entry {
                    source: browser::SourceId::Sunrise,
                    project: project.id.clone(),
                    slug: project.id.clone(),
                    title: project.name.clone(),
                    kind: browser::Kind::Mod,
                    version: version.version.clone(),
                    number: version.version.clone(),
                    file: relative.to_string_lossy().replace('\\', "/"),
                    checksum: format!("SHA-256:{}", d.sha256),
                    page: format!("{}/{}", sunrise::SITE, project.id),
                    dependency: old
                        .and_then(|pkg| pkg.row.as_ref())
                        .is_none_or(|entry| entry.dependency),
                    verdict: None,
                };
                choices.push(Package {
                    manifest,
                    digest: d.sha256.to_ascii_lowercase(),
                    relative,
                    file,
                    row: Some(row),
                    on: old.is_none_or(|p| p.on),
                });
            }
        }
        choices.sort_by(|a, b| {
            Version::parse(&b.manifest.version)
                .ok()
                .cmp(&Version::parse(&a.manifest.version).ok())
                .then_with(|| a.digest.cmp(&b.digest))
        });
        for choice in &choices {
            pending.extend(
                choice
                    .manifest
                    .dependencies
                    .iter()
                    .filter(|(_, need)| {
                        let need = need.details();
                        !need.optional
                    })
                    .map(|(id, _)| id.clone()),
            );
        }
        slots.insert(id, choices);
    }
    Ok(slots)
}

fn best_survivor_set<'a>(
    slots: &'a BTreeMap<String, Vec<Package>>,
    source: &Inventory,
    mut trial: impl FnMut(BTreeMap<&'a str, &'a Package>) -> Result<bool, String>,
) -> Result<(), String> {
    struct Search<'a> {
        slots: Vec<(&'a str, &'a [Package], bool)>,
        remaining: Vec<usize>,
        examined: usize,
    }
    impl<'a> Search<'a> {
        fn try_rank(
            &mut self,
            at: usize,
            retained: usize,
            chosen: &mut BTreeMap<&'a str, &'a Package>,
            trial: &mut impl FnMut(BTreeMap<&'a str, &'a Package>) -> Result<bool, String>,
        ) -> Result<bool, String> {
            if retained > self.remaining[at] {
                return Ok(false);
            }
            self.examined += 1;
            if self.examined > MAX_TRIALS {
                return Err(too_many());
            }
            let Some(&(id, choices, original)) = self.slots.get(at) else {
                let mut reachable = BTreeSet::new();
                let mut todo: Vec<_> = self
                    .slots
                    .iter()
                    .filter(|(id, _, original)| *original && chosen.contains_key(id))
                    .map(|(id, _, _)| *id)
                    .collect();
                while let Some(id) = todo.pop() {
                    if !reachable.insert(id) {
                        continue;
                    }
                    todo.extend(
                        chosen[id]
                            .manifest
                            .dependencies
                            .iter()
                            .filter(|(id, need)| {
                                let need = need.details();
                                !need.optional && chosen.contains_key(id.as_str())
                            })
                            .map(|(id, _)| id.as_str()),
                    );
                }
                if reachable.len() == chosen.len() && fits(chosen) && trial(chosen.clone())? {
                    return Ok(true);
                }
                return Ok(false);
            };
            // For a fixed retained count the first satisfying vector is the newest,
            // in stable ID order. Search as needed; don't enumerate lower-ranked sets.
            if let Some(left) = retained.checked_sub(usize::from(original)) {
                for choice in choices {
                    chosen.insert(id, choice);
                    if self.try_rank(at + 1, left, chosen, trial)? {
                        return Ok(true);
                    }
                }
                chosen.remove(id);
            }
            self.try_rank(at + 1, retained, chosen, trial)
        }
    }
    if slots.len() > 128 {
        return Err(too_many());
    }
    let slots: Vec<_> = slots
        .iter()
        .map(|(id, choices)| {
            (
                id.as_str(),
                choices.as_slice(),
                source.packages.contains_key(id),
            )
        })
        .collect();
    let mut remaining = vec![0; slots.len() + 1];
    for (i, (_, choices, original)) in slots.iter().enumerate().rev() {
        remaining[i] = remaining[i + 1] + usize::from(*original && !choices.is_empty());
    }
    let mut search = Search {
        slots,
        remaining,
        examined: 0,
    };
    let mut chosen = BTreeMap::new();
    for retained in (0..=search.remaining[0]).rev() {
        if search.try_rank(0, retained, &mut chosen, &mut trial)? {
            break;
        }
    }
    Ok(())
}

pub(super) fn fits(chosen: &BTreeMap<&str, &Package>) -> bool {
    let mut namespaces = BTreeSet::new();
    for pkg in chosen.values() {
        if pkg.on
            && std::iter::once(&pkg.manifest.id)
                .chain(&pkg.manifest.namespaces)
                .any(|ns| !namespaces.insert(ns))
        {
            return false;
        }
        for (id, need) in &pkg.manifest.dependencies {
            let need = need.details();
            if need.optional {
                continue;
            }
            if chosen.get(id.as_str()).is_none_or(|other| {
                pkg.on && !other.on
                    || !Version::parse(&other.manifest.version)
                        .is_ok_and(|v| need.version.matches(&v))
            }) {
                return false;
            }
        }
        for (id, rule) in &pkg.manifest.incompatible {
            let rule = rule.details();
            if pkg.on
                && chosen.get(id.as_str()).is_some_and(|other| {
                    other.on
                        && Version::parse(&other.manifest.version)
                            .is_ok_and(|v| rule.version.matches(&v))
                })
            {
                return false;
            }
        }
    }
    true
}

pub(super) fn copy_tree(src: &Path, dst: &Path, top: bool) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| bad(dst, e))?;
    for entry in fs::read_dir(src).map_err(|e| bad(src, e))? {
        let entry = entry.map_err(|e| bad(src, e))?;
        let file = entry.path();
        let target = dst.join(entry.file_name());
        let meta = fs::symlink_metadata(&file).map_err(|e| bad(&file, e))?;
        if !build::regular(&meta) {
            return Err(unsettled());
        }
        if meta.is_dir() {
            copy_tree(&file, &target, false)?;
        } else if meta.is_file() {
            if !(top && file.extension().is_some_and(|ext| ext == "smod")) {
                fs::copy(&file, &target).map_err(|e| bad(&file, e))?;
            }
        } else {
            return Err(unsettled());
        }
    }
    Ok(())
}
fn stage_set(
    home: &Path,
    stage: &Path,
    source: &Inventory,
    chosen: &BTreeMap<&str, &Package>,
) -> Result<(), String> {
    for folder in [mod_list::DIR, mod_list::NATIVE_DIR] {
        match fs::symlink_metadata(home.join(folder)) {
            Ok(_) => copy_tree(&home.join(folder), &stage.join(folder), true)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(bad(&home.join(folder), e)),
        }
    }
    let mut browser = source.browser.clone();
    let rows = browser
        .get_mut("entries")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(unsettled)?;
    rows.retain(|row| {
        row.get("file")
            .and_then(serde_json::Value::as_str)
            .is_none_or(|file| {
                !source
                    .packages
                    .values()
                    .any(|pkg| pkg.row.as_ref().is_some_and(|entry| entry.file == file))
            })
    });
    let mut selection: String = source
        .selection
        .as_deref()
        .unwrap_or("")
        .split_inclusive('\n')
        .filter(|line| {
            line.split_once(':')
                .and_then(|(key, _)| key.trim().strip_prefix("mod."))
                .is_none_or(|id| !source.packages.contains_key(id) && !chosen.contains_key(id))
        })
        .collect();
    if !selection.is_empty() && !selection.ends_with('\n') {
        selection.push('\n');
    }
    let mut paths = BTreeSet::new();
    for (id, pkg) in chosen {
        if !paths.insert(&pkg.relative) {
            return Err(unsettled());
        }
        let target = stage.join(&pkg.relative);
        let parent = target.parent().ok_or_else(unsettled)?;
        fs::create_dir_all(parent).map_err(|e| bad(parent, e))?;
        fs::copy(&pkg.file, &target).map_err(|e| bad(&target, e))?;
        if index::sha256_file(&target)? != pkg.digest {
            return Err(unsettled());
        }
        if let Some(row) = &pkg.row {
            rows.push(serde_json::to_value(row).map_err(|e| e.to_string())?);
        }
        selection.push_str(&format!("mod.{id}:{}\n", pkg.on));
    }
    if source.selection.is_some() || !selection.is_empty() {
        save(&stage.join(mod_list::FILE), selection.as_bytes())?;
    }
    if build::optional_bytes(&home.join(installed::MANIFEST))?.is_some() || !rows.is_empty() {
        save(
            &stage.join(installed::MANIFEST),
            &serde_json::to_vec(&browser).map_err(|e| e.to_string())?,
        )?;
    }
    Ok(())
}

pub(super) struct CandidateBuild<'a> {
    pub(super) profile: native_mods::Profile,
    pub(super) cancellation: native_mods::Cancellation,
    pub(super) registry_consent: Option<&'a dyn Fn(&native_mods::Consent) -> bool>,
}

pub(super) fn build_candidate(
    tag: &str,
    stage: &Path,
    compiler: &CandidateBuild<'_>,
    progress: &impl Fn(native_mods::Step),
    chosen: &BTreeMap<&str, &Package>,
) -> Result<String, native_mods::Failed> {
    let image = if crate::native_status::exact_enabled(stage)?.is_empty() {
        tag.to_owned()
    } else {
        native_mods::build_exact_client(
            tag,
            stage,
            compiler.profile,
            &compiler.cancellation,
            progress,
            compiler.registry_consent,
        )?
    };
    build::verify_included(tag, &image, stage)?;
    let list = stage.join(mod_list::FILE);
    let selection = build::optional_bytes(&list)?;
    for (id, pkg) in chosen {
        if pkg.on || pkg.manifest.native.is_none() {
            continue;
        }
        let mut active: BTreeSet<&str> = chosen
            .iter()
            .filter(|(_, pkg)| pkg.on)
            .map(|(id, _)| *id)
            .collect();
        let mut todo = vec![*id];
        while let Some(id) = todo.pop() {
            if !active.insert(id) {
                continue;
            }
            todo.extend(
                chosen[id]
                    .manifest
                    .dependencies
                    .iter()
                    .filter(|(_, need)| {
                        let need = need.details();
                        !need.optional
                    })
                    .map(|(id, _)| id.as_str()),
            );
        }
        let trial: String = chosen
            .keys()
            .map(|id| format!("mod.{id}:{}\n", active.contains(id)))
            .collect();
        save(&list, trial.as_bytes())?;
        let candidate = native_mods::build_exact_client(
            tag,
            stage,
            compiler.profile,
            &compiler.cancellation,
            progress,
            compiler.registry_consent,
        );
        let verified = candidate.and_then(|candidate| {
            build::verify_included(tag, &candidate, stage).map_err(Into::into)
        });
        if let Some(text) = &selection {
            save(&list, text)?;
        } else {
            fs::remove_file(&list).map_err(|e| bad(&list, e))?;
        }
        verified?;
    }
    Ok(image)
}

pub(super) fn changes(
    home: &Path,
    stage: &Path,
    old: &Inventory,
    chosen: &BTreeMap<&str, &Package>,
) -> Result<Vec<(PathBuf, Option<PathBuf>)>, String> {
    let mut paths: BTreeSet<_> = old
        .packages
        .values()
        .chain(chosen.values().copied())
        .map(|pkg| pkg.relative.clone())
        .collect();
    paths.extend([
        PathBuf::from(mod_list::FILE),
        PathBuf::from(installed::MANIFEST),
    ]);
    let mut changed = Vec::new();
    for path in paths {
        let before = super::Image::at(&home.join(&path))?;
        let source = stage.join(&path);
        let after = super::Image::at(&source)?;
        if before != after {
            changed.push((path, (after != super::Image::Missing).then_some(source)));
        }
    }
    Ok(changed)
}

fn save(path: &Path, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|e| bad(path, e))
}
fn bad(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}
fn too_many() -> String {
    tr("The launcher couldn't verify every alternative. Nothing was switched.").into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_fit_keeps_the_exact_native_package_off_but_a_takedown_removes_it() {
        let profile = tempfile::tempdir().unwrap();
        let home = profile.path();
        let root = home.join(mod_list::NATIVE_DIR);
        fs::create_dir(&root).unwrap();
        let mut rows = Vec::new();
        for (id, name) in [("lantern", "Lantern"), ("revoked", "Revoked")] {
            let bytes = sunrise_launcher_smod::Package::write_files(
                &format!(
                    "id='{id}'\nname='{name}'\nversion='2.0.0'\napi='{}'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{}'\nmixins='{}'\n",
                    manifest::API, "a".repeat(40), "b".repeat(40),
                ),
                &[("src/lib.rs", b""), ("Cargo.toml", b"[package]\nname='fixture'\nversion='2.0.0'\n")],
            ).unwrap();
            let file = format!("{}/{id}.smod", mod_list::NATIVE_DIR);
            fs::write(home.join(&file), &bytes).unwrap();
            rows.push(serde_json::json!({
                "source":"sunrise", "project":id, "slug":id, "title":name, "kind":"mod",
                "version":"2.0.0", "number":"2.0.0", "file":file,
                "checksum":format!("SHA-256:{}", index::sha256(&bytes)), "page":"",
            }));
        }
        fs::write(
            home.join(mod_list::FILE),
            "mod.lantern:true\nmod.revoked:true\n",
        )
        .unwrap();
        fs::write(
            home.join(installed::MANIFEST),
            serde_json::to_vec(&serde_json::json!({"entries":rows})).unwrap(),
        )
        .unwrap();
        let source = Inventory::read(home).unwrap();
        let index: index::Index = serde_json::from_value(serde_json::json!({
            "schema": 1, "projects": [], "revoked": [{"id":"revoked","reason":"taken down"}],
        }))
        .unwrap();
        let stage = tempfile::tempdir().unwrap();
        let slots = candidates(&source, &index, "26.4", stage.path()).unwrap();
        assert!(slots.values().all(Vec::is_empty));
        let (retained, waiting, effects) =
            outcomes(&source, &BTreeMap::new(), &index, &BTreeMap::new(), |_| {
                "no fit".into()
            })
            .unwrap();
        assert_eq!(
            effects.waiting_for_update,
            [("Lantern".to_owned(), "no fit".to_owned())]
        );
        assert_eq!(
            effects.removed,
            [("Revoked".to_owned(), "taken down".to_owned())]
        );
        assert!(waiting["lantern"].was_on);
        let chosen = retained.iter().map(|(id, p)| (id.as_str(), p)).collect();
        stage_set(home, stage.path(), &source, &chosen).unwrap();
        let after = Inventory::read(stage.path()).unwrap();
        let kept = &after.packages["lantern"];
        assert_eq!(kept.relative, source.packages["lantern"].relative);
        assert_eq!(kept.digest, source.packages["lantern"].digest);
        assert!(!kept.on);
        assert!(crate::native_status::enabled(stage.path()).is_empty());
        assert!(!after.packages.contains_key("revoked"));
        assert_eq!(
            installed::Installed::load(stage.path()).entries[0].file,
            rows[0]["file"].as_str().unwrap()
        );

        // An older compatible patch is an eligible update, but not a new label for the same bytes.
        let registry: index::Index = serde_json::from_value(serde_json::json!({
            "schema":1, "projects":[{
                "id":"lantern", "kind":"native-mod", "name":"Lantern", "summary":"Fixture", "authors":["Someone"], "license":"MIT", "source":"https://github.com/example/lantern", "versions":[
                    {"version":"2.0.0", "published":"2026-11-19", "api":manifest::API, "game":"26.4", "file":{"name":"lantern.smod", "url":"https://example.test/same", "size":128, "sha256":kept.digest}},
                    {"version":"1.9.5", "published":"2026-11-20", "api":manifest::API, "game":"26.4", "file":{"name":"lantern.smod", "url":"https://example.test/new", "size":128, "sha256":"a".repeat(64)}}
                ]
            }], "revoked":[],
        })).unwrap();
        let project = registry.project("lantern").unwrap();
        assert_eq!(
            waiting["lantern"]
                .replacement(&registry, project, "26.4")
                .unwrap()
                .version,
            "1.9.5"
        );
        assert!(waiting["lantern"]
            .replacement(&registry, project, "26.5")
            .is_none());
    }

    #[test]
    fn an_all_compatible_set_does_not_enumerate_every_lower_rank() {
        let mut packages = BTreeMap::new();
        let mut slots = BTreeMap::new();
        for n in 0..20 {
            let id = format!("id_{n:02}");
            let make = |version: &str| {
                Package {
                manifest: Manifest::parse(&format!("id = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\napi = \"{}\"\ncapabilities = [\"registries\"]\n", manifest::API)).unwrap(),
                digest: String::new(), file: PathBuf::new(), relative: PathBuf::new(), row: None, on: true,
            }
            };
            packages.insert(id.clone(), make("1.0.0"));
            slots.insert(id.clone(), vec![make("2.0.0"), make("1.0.0")]);
        }
        let source = Inventory {
            packages,
            browser: serde_json::Value::Null,
            selection: None,
        };
        let mut attempted = 0;
        best_survivor_set(&slots, &source, |set| {
            attempted += 1;
            assert_eq!(set.len(), 20);
            assert!(set
                .iter()
                .take(19)
                .all(|(_, pkg)| pkg.manifest.version == "2.0.0"));
            let last = set["id_19"].manifest.version.as_str();
            assert_eq!(last, if attempted == 1 { "2.0.0" } else { "1.0.0" });
            Ok(attempted == 2)
        })
        .unwrap();
        assert_eq!(attempted, 2);
    }
}
