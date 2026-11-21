//! Registry and collection requests produce one admitted, fully proved same-base generation.
use super::{
    fetch, fetch_with_official, fits_game, index, installed, location, project_of, trust_label,
    version_of, OfficialRegistry,
};
use crate::{
    backend,
    browser::{mod_list, Kind, SourceId},
    i18n::{tr, trf},
    instances::{Instance, Kind as InstanceKind},
    managed_play::{self, FirstBuild, Profile, ScannedPackage},
    native_mods,
    profile_txn::Identity,
    worlds::switch::{Fence, Question},
};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    sync::{Arc, Mutex},
};
use sunrise_launcher_smod::{
    index::{browser_state, resolve},
    manifest::{Manifest, Side},
};

const MAX_TRIALS: usize = 64;
mod conflicts;
mod source;
mod switch;
pub use conflicts::ResolveQuestion;
use source::MixedSources;
pub use source::{
    apply_sources, prepare_pasted_sources, prepare_source, read_pasted_sources,
    update_registry_and_sources, CombinedUpdate, PastedSource, PreparedSource, SourceError,
    SourceHooks, SourceRequest,
};
pub use switch::{checked_switch_fit, switch_build, SwitchHooks, SwitchQuestion, Switched};

// Advisory only. Never use these in place of a fresh, admitted worker proof. Key by the
// base build and verified archive hash, not the Minecraft version or catalogue version number.
static UNFIT_UPDATES: Mutex<BTreeMap<String, BTreeSet<String>>> = Mutex::new(BTreeMap::new());

pub fn known_unfit_updates(base: &str) -> BTreeSet<String> {
    UNFIT_UPDATES
        .lock()
        .unwrap()
        .get(base)
        .cloned()
        .unwrap_or_default()
}

fn remember_unfit(base: &str, release: &index::Version) {
    UNFIT_UPDATES
        .lock()
        .unwrap()
        .entry(base.into())
        .or_default()
        .insert(release.file.sha256.to_ascii_lowercase());
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InstallProgress {
    Checking,
    Downloading(f32),
    Preparing,
    Tools { done: u64, total: u64 },
    Rust(f32),
    Building(Option<f32>),
    Publishing,
    Reviewing,
}

#[derive(Debug)]
pub enum InstallError {
    Refused(String),
    Host(String),
    Unproved(String),
    Download(String),
    NotForVersion(String),
    GithubBuild(String),
    Source(SourceError),
    Compile(BuildNotice),
    Cancelled,
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(text)
            | Self::Host(text)
            | Self::Unproved(text)
            | Self::NotForVersion(text)
            | Self::GithubBuild(text)
            | Self::Download(text) => f.write_str(text),
            Self::Compile(failure) => f.write_str(&failure.detail),
            Self::Source(error) => error.fmt(f),
            Self::Cancelled => f.write_str(tr("Install cancelled.")),
        }
    }
}

impl From<String> for InstallError {
    fn from(error: String) -> Self {
        Self::Unproved(error)
    }
}

#[derive(Clone, Debug)]
pub struct NativeQuestion {
    pub names: Vec<String>,
    pub reviewed: bool,
    /// A proved no-fit changed the named closure after this install's first approval.
    pub retry: bool,
}

#[derive(Clone, Debug)]
pub struct BuildNotice {
    pub changes: Vec<BuildChange>,
    /// Raw compiler/proof error for the headless route, not the visible sheet.
    pub detail: String,
    pub log: Option<Arc<str>>,
}

#[derive(Clone, Debug)]
pub struct BuildChange {
    pub project: String,
    pub title: String,
    pub reason: installed::InstallHold,
    pub predecessor: Option<String>,
}

impl BuildChange {
    pub fn disposition(&self) -> String {
        match &self.predecessor {
            Some(version) => trf("Keep installed version {version}", &[("version", version)]),
            None => tr("Keep installed, but off").into(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct InstalledBatch {
    pub added: Vec<installed::Entry>,
    pub updated: Vec<(String, installed::Entry)>,
    pub failure: Option<BuildNotice>,
}

#[derive(Clone, Copy)]
pub struct InstallHooks<'a> {
    pub progress: &'a dyn Fn(InstallProgress),
    pub native: &'a dyn Fn(&NativeQuestion) -> Result<bool, String>,
    pub registry_crates: Option<&'a dyn Fn(&native_mods::Consent) -> bool>,
    pub continue_after_failure: &'a dyn Fn(&BuildNotice) -> bool,
    /// Interactive approval of a fully prepared same-base conflict plan; absent in automation.
    pub resolve: Option<&'a dyn Fn(&ResolveQuestion) -> bool>,
}

/// Capture the Install target before it queues. The initial default profile may not have
/// a game directory until its first operation; never recreate a named profile here.
pub fn install_target(instance: &Instance) -> Result<Identity, String> {
    if instance.kind == InstanceKind::Default && instance.incarnation.is_none() {
        fs::create_dir_all(&instance.home).map_err(|e| at(&instance.home, e))?;
    }
    let identity = Identity::of(&instance.home)?;
    match instance.incarnation {
        Some(expected) => expected.check(instance, identity)?,
        None if instance.kind == InstanceKind::Default => (),
        None => return Err(crate::instances::lifecycle::changed()),
    }
    Ok(identity)
}

/// Profile selection is captured at the click. A saved active/pending build wins over the
/// candidate in `choose`, just as in Play. This callback is never called again at cutoff.
pub fn install_registry(
    instance: &Instance,
    identity: Identity,
    request: impl FnOnce() -> Vec<String>,
    choose: impl FnOnce() -> Result<FirstBuild, String>,
    hooks: InstallHooks<'_>,
) -> Result<InstalledBatch, InstallError> {
    registry_edit(
        instance,
        identity,
        || Action::Install(request()),
        choose,
        hooks,
        fetch_with_official,
        None,
    )
}

pub enum UpdateRequest {
    /// Every named root must advance, even if it is the only one requested.
    Strict(Vec<String>),
    /// The bulk action can retain roots with no usable newer release proved by this attempt.
    Bulk(Vec<String>),
}

/// Explicit update uses the same candidate and proof as Install, but never opts a world in.
pub fn update_registry(
    instance: &Instance,
    identity: Identity,
    requested: UpdateRequest,
    choose: impl FnOnce() -> Result<FirstBuild, String>,
    hooks: InstallHooks<'_>,
    approve: &dyn Fn(&ChangeImpact) -> bool,
) -> Result<InstalledBatch, InstallError> {
    registry_edit(
        instance,
        identity,
        || Action::Update(requested, approve),
        choose,
        hooks,
        fetch_with_official,
        None,
    )
}

enum Action<'a> {
    Install(Vec<String>),
    Update(UpdateRequest, &'a dyn Fn(&ChangeImpact) -> bool),
}

fn registry_edit<'a>(
    instance: &Instance,
    identity: Identity,
    request: impl FnOnce() -> Action<'a>,
    choose: impl FnOnce() -> Result<FirstBuild, String>,
    hooks: InstallHooks<'_>,
    fetch_registry: impl Fn(
        &str,
        bool,
    ) -> Result<(index::Index, String, Option<OfficialRegistry>), String>,
    mut mixed: Option<&mut MixedSources<'_>>,
) -> Result<InstalledBatch, InstallError> {
    if instance.kind == InstanceKind::Pack {
        return Err(InstallError::Refused(
            tr("No registry mods to install in this profile.").into(),
        ));
    }
    (hooks.progress)(InstallProgress::Checking);
    let source = location();
    let local = super::local();
    if source != super::INDEX_URL && !(cfg!(debug_assertions) && local) {
        return Err(InstallError::Unproved(
            tr("Connect to the Sunrise registry before installing a mod and its dependencies.")
                .into(),
        ));
    }
    let (catalogue, source_bytes, authority) = fetch_registry(&source, local)?;
    let authority = authority.as_ref();
    if !catalogue.left_out.is_empty() {
        return Err(InstallError::Unproved(
            tr("The mod registry didn't send a complete index. Nothing was installed.").into(),
        ));
    }
    // Freeze the visible batch before starting any private planning. A later click must queue.
    let action = request();
    if matches!(&action, Action::Install(ids) if ids.is_empty()) {
        return Err(InstallError::Cancelled);
    }
    // Reject a replaced home before recovery or any live edit.
    let mut profile =
        Profile::open_for(&instance.home, None, Some(identity)).map_err(String::from)?;
    let edit = profile.begin_edit(
        choose,
        |f| (hooks.progress)(InstallProgress::Downloading(f)),
        &|_| (hooks.progress)(InstallProgress::Preparing),
    )?;
    let game = backend::game_version(edit.base());
    let cancelled = native_mods::Cancellation::default();
    let old = edit.candidate()?;
    let before = inventory(old.path(), authority)?;
    let (ids, above) = match &action {
        Action::Install(ids) => (ids.clone(), BTreeMap::new()),
        Action::Update(requested, _) => {
            let ids = match requested {
                UpdateRequest::Strict(ids) | UpdateRequest::Bulk(ids) => ids.as_slice(),
            };
            let above = mutations::update_floors(&catalogue, &before, &game, ids)?;
            if above.is_empty() {
                return Ok(InstalledBatch::default());
            }
            (above.keys().cloned().collect(), above)
        }
    };
    let roots: Vec<_> = ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|id| resolve::Request { id, version: None })
        .collect();
    for need in &roots {
        if catalogue
            .project(&need.id)
            .is_none_or(|p| !matches!(p.kind, index::Kind::Mod | index::Kind::NativeMod))
        {
            return Err(InstallError::Refused(trf(
                "{mod} is not a mod in the Sunrise registry.",
                &[("mod", &need.id)],
            )));
        }
    }
    for root in &roots {
        if let Some(package) = before.get(&root.id).filter(|package| !package.owned) {
            return Err(InstallError::Refused(trf(
                "{mod} is already installed from another source or location. Nothing was installed.",
                &[("mod", &package.package.manifest.name)],
            )));
        }
    }
    let was_enabled = mutations::enabled(old.path())?;
    let held = before
        .iter()
        .filter(|(id, _)| was_enabled.contains(*id))
        .map(|(id, p)| {
            let managed = p.owned.then(|| p.digest.clone()).flatten();
            resolve::InstalledMod {
                id: id.clone(),
                version: p.package.manifest.version.clone(),
                sha256: managed,
            }
        })
        .collect::<Vec<_>>();
    let mut minimum = above.clone();
    if !above.is_empty() {
        for (id, installed) in before.iter().filter(|(_, installed)| installed.owned) {
            if let std::collections::btree_map::Entry::Vacant(slot) = minimum.entry(id.clone()) {
                slot.insert(
                    sunrise_launcher_smod::dependencies::parse_version(
                        &installed.package.manifest.version,
                    )
                    .map_err(|_| unproved_mods())?,
                );
            }
        }
    }
    let retain_unfit = matches!(action, Action::Update(UpdateRequest::Bulk(_), _));
    let mut unfit = BTreeSet::new();
    let mut conflicts = Vec::new();
    let mut last_sdk_refusal = None;
    let mut named_native: Option<BTreeSet<String>> = None;
    'trials: for _ in 0..MAX_TRIALS {
        let options = resolve::WholeOptions {
            conflicts: &conflicts,
            native_fits: |p: &index::Project, v: &index::Version| {
                !unfit.contains(&(p.id.clone(), v.version.clone()))
            },
            messages: trf,
        };
        let plan = if above.is_empty() {
            resolve::install_whole(&catalogue, &roots, &held, &game, Side::Both, options)
        } else {
            resolve::update_whole(
                &catalogue,
                resolve::UpdateTargets {
                    above: &above,
                    minimum: &minimum,
                    retain_unfit,
                },
                &held,
                &game,
                Side::Both,
                options,
            )
        };
        let (plan, disabled) = match plan {
            Ok(plan) => (plan, Vec::new()),
            Err(resolve::WholeRefusal::Unproved(reason)) => {
                return Err(InstallError::Unproved(reason))
            }
            Err(resolve::WholeRefusal::Inconsistent(reason)) => {
                if !matches!(action, Action::Install(_))
                    || hooks.resolve.is_none()
                    || edit.current().is_none()
                {
                    return Err(InstallError::Refused(if retain_unfit {
                        reason
                    } else {
                        last_sdk_refusal.unwrap_or(reason)
                    }));
                }
                // Negative evidence must come from a complete index, not Discover or cached rows.
                let (_, complete, proof) = super::complete_index(&source, local)?;
                if complete != source_bytes || proof.is_some() != authority.is_some() {
                    return Err(registry_changed().into());
                }
                validate_enabled(old.path(), &catalogue, authority)?;
                conflicts::minimal(&catalogue, &roots, &held, &game, &conflicts, |p, v| {
                    !unfit.contains(&(p.id.clone(), v.version.clone()))
                })?
                .ok_or_else(|| InstallError::Refused(last_sdk_refusal.take().unwrap_or(reason)))?
            }
        };
        let versions = plan
            .install
            .iter()
            .map(|row| (row.id.as_str(), row.version.as_str()))
            .chain(
                plan.update
                    .iter()
                    .map(|row| (row.id.as_str(), row.to.as_str())),
            )
            .map(|(id, version)| {
                let project = catalogue.project(id).ok_or_else(unproved_mods)?;
                let version = project
                    .versions
                    .iter()
                    .find(|v| v.version == version && fits_game(v, &game))
                    .ok_or_else(unproved_mods)?;
                Ok((project, version))
            })
            .collect::<Result<Vec<_>, String>>()?;
        if versions.is_empty() {
            return Ok(InstalledBatch::default());
        }
        let next = edit.candidate()?;
        let mut entries = BTreeMap::new();
        for (n, (project, version)) in versions.iter().enumerate() {
            let entry = stage_package(
                &catalogue,
                local,
                next.path(),
                &before,
                project,
                version,
                |f| {
                    (hooks.progress)(InstallProgress::Downloading(
                        (n as f32 + f) / versions.len() as f32,
                    ));
                },
            )?;
            entries.insert(entry.project.clone(), entry);
        }
        let mut doc = Document::read(next.path())?;
        for entry in entries.values_mut() {
            entry.dependency = !ids.contains(&entry.project);
            if let Some(old) = doc.row(&entry.project)? {
                let dependency = old["dependency"].as_bool().unwrap_or(false);
                entry.dependency = (entry.dependency || !above.is_empty()) && dependency;
            }
            doc.replace(entry)?;
        }
        doc.write(next.path())?;
        let mut restored_holds = BTreeSet::new();
        let mut retired_off = BTreeSet::new();
        for (id, entry) in &entries {
            let Some(held) = edit.held(id) else { continue };
            // Only an explicit Update may consume an exact predecessor. An implicit
            // dependency or Install cannot replace bytes while the hold still names them.
            let Some(previous) = before.get(id).filter(|p| {
                p.owned
                    && matches!(held.source(), managed_play::HeldSource::Registry { .. })
                    && p.matches_hold(old.path(), edit.base(), held)
            }) else {
                return Err(held_change_refusal(id));
            };
            if previous.digest.as_ref().is_some_and(|digest| {
                entry
                    .checksum
                    .eq_ignore_ascii_case(&format!("SHA-256:{digest}"))
            }) {
                continue;
            }
            if !above.contains_key(id) {
                return Err(held_change_refusal(id));
            }
            if held.enabled_before() {
                restored_holds.insert(id.clone());
            } else {
                retired_off.insert(id.clone());
            }
        }
        let staged = inventory(next.path(), authority)?;
        let activate: Vec<_> =
            if matches!(action, Action::Install(_)) {
                entries
                    .keys()
                    .filter(|id| {
                        edit.held(id).is_none()
                            && (!before.contains_key(*id) || was_enabled.contains(*id))
                    })
                    .map(String::as_str)
                    .collect()
            } else {
                // Updating an off row does not turn it (or its new dependencies) on. Walk only
                // the intended enabled closure; an existing off dependency remains explicitly off.
                let mut activate = BTreeSet::new();
                let mut visited = BTreeSet::new();
                let mut needed: Vec<_> = was_enabled
                    .iter()
                    .chain(&restored_holds)
                    .map(String::as_str)
                    .collect();
                while let Some(id) = needed.pop() {
                    if !visited.insert(id) {
                        continue;
                    }
                    let Some(package) = staged.get(id) else {
                        continue;
                    };
                    if !restored_holds.contains(id)
                        && (edit.held(id).is_some()
                            || (!was_enabled.contains(id) && before.contains_key(id)))
                    {
                        continue;
                    }
                    if !was_enabled.contains(id) {
                        activate.insert(id);
                    }
                    needed.extend(package.package.manifest.dependencies.iter().filter_map(
                        |(id, rule)| (!rule.details().optional).then_some(id.as_str()),
                    ));
                }
                activate.into_iter().collect()
            };
        if !activate.is_empty() {
            mod_list::set_all_enabled(next.path(), &activate, true)?;
        }
        // An inconsistent on-list cannot bypass a typed hold while updating that package.
        let keep_off: Vec<_> = entries
            .keys()
            .filter(|id| {
                edit.held(id).is_some()
                    && !restored_holds.contains(*id)
                    && was_enabled.contains(*id)
            })
            .map(String::as_str)
            .collect();
        if !keep_off.is_empty() {
            mod_list::set_all_enabled(next.path(), &keep_off, false)?;
        }
        if !disabled.is_empty() {
            if disabled.iter().any(|id| entries.contains_key(*id)) {
                return Err(unproved_mods().into());
            }
            mod_list::set_all_enabled(next.path(), &disabled, false)?;
        }
        if let Some(conflict) = manifest_conflict(next.path(), Some(&catalogue), authority, |id| {
            entries.contains_key(id)
        })? {
            if conflicts.contains(&conflict) {
                return Err(InstallError::Unproved(conflict.reason));
            }
            conflicts.push(conflict);
            continue;
        }
        (hooks.progress)(InstallProgress::Preparing);
        let staged = inventory(next.path(), authority)?;
        let build_packages: Vec<_> = staged
            .values()
            .filter(|p| {
                p.package.manifest.native.is_some()
                    && (mod_list::is_enabled(next.path(), &p.package.manifest.id)
                        || retired_off.contains(&p.package.manifest.id))
            })
            .collect();
        if !build_packages.is_empty() {
            if let Some(error) = native_mods::known_host_block(Some(edit.base())) {
                return Err(InstallError::Host(error));
            }
            let retry = named_native.is_some();
            if retry || backend::NativeGrant::read()?.is_none() {
                let newly_named: Vec<_> = build_packages
                    .iter()
                    .copied()
                    .filter(|p| {
                        named_native
                            .as_ref()
                            .is_none_or(|ids| !ids.contains(&p.package.manifest.id))
                    })
                    .collect();
                if !newly_named.is_empty() {
                    if !(hooks.native)(&NativeQuestion {
                        names: newly_named
                            .iter()
                            .map(|p| p.package.manifest.name.clone())
                            .collect(),
                        reviewed: newly_named.iter().all(|p| p.owned),
                        retry,
                    })? {
                        return Err(InstallError::Cancelled);
                    }
                    named_native
                        .get_or_insert_with(BTreeSet::new)
                        .extend(newly_named.iter().map(|p| p.package.manifest.id.clone()));
                }
            }
            // This comes from the strict saved grant, never from the dialog reply alone.
            cancelled
                .admit_native_work()
                .map_err(|e| InstallError::Unproved(e.into()))?;
        }
        for id in &retired_off {
            // Prove a deliberately off replacement in the private candidate, without
            // making activation or this build visible in the actual postimage.
            let list = next.path().join(sunrise_launcher_smod::mod_list::FILE);
            let selection = match fs::read(&list) {
                Ok(selection) => Some(selection),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(at(&list, e).into()),
            };
            mod_list::set_enabled(next.path(), id, true)?;
            let trial = prove_edit(&edit, next.path(), instance, &cancelled, &hooks);
            match selection {
                Some(selection) => fs::write(&list, selection),
                None => fs::remove_file(&list),
            }
            .map_err(|e| at(&list, e))?;
            match trial {
                Ok(_) => (),
                Err(native_mods::Failed::Mods {
                    left,
                    error,
                    compiling: false,
                    ..
                }) if left.iter().any(|f| {
                    &f.id == id
                        && f.version == entries[id].version
                        && matches!(f.why, native_mods::Why::Unfit(_))
                }) && left.iter().all(|f| {
                    matches!(
                        f.why,
                        native_mods::Why::Unfit(_) | native_mods::Why::Needs(_)
                    )
                }) =>
                {
                    let candidate = &entries[id].version;
                    let (_, release) = versions
                        .iter()
                        .find(|(project, release)| {
                            &project.id == id && &release.version == candidate
                        })
                        .expect("the prior-off trial proved a checked staged release");
                    remember_unfit(edit.base(), release);
                    if !unfit.insert((id.clone(), candidate.clone())) {
                        return Err(InstallError::Unproved(error));
                    }
                    last_sdk_refusal = Some(error);
                    continue 'trials;
                }
                Err(error) => return Err(InstallError::Unproved(error.into())),
            }
        }
        let prove = || prove_edit(&edit, next.path(), instance, &cancelled, &hooks);
        let mut result = InstalledBatch::default();
        let proof = match prove() {
            Ok(proof) => proof,
            Err(native_mods::Failed::Mods {
                left,
                error,
                compiling: false,
                ..
            }) if !left.is_empty()
                && left.iter().all(|failure| match &failure.why {
                    native_mods::Why::Needs(_) => true,
                    native_mods::Why::Unfit(_) => entries
                        .get(&failure.id)
                        .is_some_and(|entry| entry.version == failure.version),
                    _ => false,
                }) =>
            {
                let first = unfit.len();
                for failed in left
                    .into_iter()
                    .filter(|f| matches!(f.why, native_mods::Why::Unfit(_)))
                {
                    let (_, version) = versions
                        .iter()
                        .find(|(p, v)| p.id == failed.id && v.version == failed.version)
                        .expect("the no-fit names a checked staged release");
                    remember_unfit(edit.base(), version);
                    unfit.insert((failed.id, failed.version));
                }
                if first == unfit.len() {
                    return Err(InstallError::Unproved(error));
                }
                last_sdk_refusal = Some(error);
                continue;
            }
            Err(native_mods::Failed::Mods {
                left,
                error,
                compiling: true,
                log,
            }) => {
                let mut failed = BTreeMap::new();
                for failure in left {
                    match failure.why {
                        native_mods::Why::Needs(_) => (),
                        native_mods::Why::DidNotBuild
                            if entries
                                .get(&failure.id)
                                .is_some_and(|entry| entry.version == failure.version) =>
                        {
                            failed.insert(failure.id, installed::InstallHold::Compile);
                        }
                        _ => return Err(InstallError::Unproved(error)),
                    }
                }
                if failed.is_empty() {
                    return Err(InstallError::Unproved(error));
                }
                let changes = restore_or_disable(
                    old.path(),
                    next.path(),
                    &before,
                    &mut entries,
                    &mut doc,
                    failed,
                )?;
                if let Some(conflict) =
                    manifest_conflict(next.path(), Some(&catalogue), authority, |id| {
                        entries.contains_key(id)
                    })?
                {
                    return Err(InstallError::Refused(conflict.reason));
                }
                let notice = BuildNotice {
                    changes,
                    detail: error,
                    log,
                };
                if !disabled.is_empty() || !(hooks.continue_after_failure)(&notice) {
                    return Err(InstallError::Compile(notice));
                }
                result.failure = Some(notice);
                let proof = prove().map_err(|e| InstallError::Unproved(e.into()))?;
                if entries.is_empty() {
                    // Only the already-live predecessors survived. No pointer needs advancing.
                    return Ok(result);
                }
                proof
            }
            Err(error) => return Err(InstallError::Unproved(error.into())),
        };
        let resolve_fence = if let Some(approve) = hooks.resolve.filter(|_| !disabled.is_empty()) {
            let question =
                conflicts::question(&catalogue, &roots, &before, next.path(), &disabled)?;
            let affected = crate::worlds::unproved_mod_effects(
                old.path(),
                edit.current(),
                next.path(),
                edit.current(),
                disabled
                    .iter()
                    .copied()
                    .chain(entries.keys().map(String::as_str)),
            )?;
            let (fence, worlds) = Fence::hold(
                &instance.home,
                affected,
                |id| &before[id].package.manifest.name,
                None,
            )?;
            if !approve(&ResolveQuestion { worlds, ..question }) {
                return Err(InstallError::Cancelled);
            }
            Some(fence)
        } else {
            None
        };
        let mut replaced = entries.keys().filter_map(|id| before.get(id)).peekable();
        let (suspect, unchecked_update) = match &action {
            Action::Update(..) => {
                mutations::update_worlds(next.path(), replaced.map(|old| &old.package))?
            }
            Action::Install(_) if replaced.peek().is_some() => (
                crate::worlds::unproved_mod_effects(
                    old.path(),
                    edit.current(),
                    next.path(),
                    edit.current(),
                    replaced.map(|old| old.package.manifest.id.as_str()),
                )?,
                false,
            ),
            Action::Install(_) => (crate::worlds::SuspectMods::new(), false),
        };
        for id in restored_holds.iter().filter(|id| entries.contains_key(*id)) {
            if !mod_list::is_enabled(next.path(), id) {
                return Err(held_change_refusal(id));
            }
        }
        let committed_inputs = proof.inputs();
        let reviewed_worlds = match &action {
            Action::Install(_) if resolve_fence.is_some() => Vec::new(),
            Action::Install(_) => guard_replacements(
                &instance.home,
                &suspect,
                entries
                    .keys()
                    .filter_map(|id| before.get(id.as_str()))
                    .map(|old| old.package.manifest.name.as_str()),
            )?,
            Action::Update(_, approve) => {
                let reviewed = entries
                    .values()
                    .map(|entry| mutations::ReviewedUpdate {
                        name: entry.title.clone(),
                        installed: before
                            .get(&entry.project)
                            .map(|old| old.package.manifest.version.clone()),
                        replacement: entry.version.clone(),
                    })
                    .collect();
                let impact = ChangeImpact::read(
                    &instance.home,
                    mutations::Change::Update(reviewed),
                    &suspect,
                )?
                .with_unchecked_update(unchecked_update);
                (hooks.progress)(InstallProgress::Reviewing);
                let allowed = if let Some(mixed) = &mut mixed {
                    mixed.combine(&impact, &edit, old.path(), next.path(), committed_inputs)
                } else {
                    approve(&impact)
                };
                if !allowed {
                    return Err(InstallError::Cancelled);
                }
                impact.into_evidence()
            }
        };
        let retired_holds = restored_holds
            .into_iter()
            .chain(retired_off)
            .filter(|id| entries.contains_key(id))
            .collect();
        (hooks.progress)(InstallProgress::Publishing);
        let published = edit.publish_install(proof, &cancelled, &retired_holds, || {
            let (_, current, current_authority) = if resolve_fence.is_some() {
                super::complete_index(&source, local)?
            } else {
                fetch_registry(&source, local)?
            };
            if source_bytes != current || current_authority.is_some() != authority.is_some() {
                return Err(registry_changed());
            }
            if let Some(fence) = &resolve_fence {
                fence.revalidate(&instance.home)?;
                Ok(None)
            } else {
                mutations::bind_worlds(&instance.home, &suspect, &reviewed_worlds).map(Some)
            }
        })?;
        if let Some(mixed) = &mut mixed {
            mixed.published(published, committed_inputs);
        }
        for (id, entry) in entries {
            if let Some(previous) = before.get(&id) {
                result
                    .updated
                    .push((previous.package.manifest.version.clone(), entry));
            } else {
                result.added.push(entry);
            }
        }
        return Ok(result);
    }
    Err(InstallError::Unproved(
        tr("Not every compatible version could be checked. Nothing was installed.").into(),
    ))
}

fn prove_edit(
    edit: &managed_play::Edit<'_>,
    candidate: &Path,
    instance: &Instance,
    cancellation: &native_mods::Cancellation,
    hooks: &InstallHooks<'_>,
) -> Result<managed_play::CandidateImage, native_mods::Failed> {
    edit.prove(
        candidate,
        instance.native_profile,
        cancellation,
        |step| build_progress(step, hooks.progress),
        hooks.registry_crates,
    )
}

fn build_progress(step: native_mods::Step, progress: &dyn Fn(InstallProgress)) {
    progress(match step {
        native_mods::Step::Rust(fraction) => InstallProgress::Rust(fraction),
        native_mods::Step::Tools { done, total, .. } => InstallProgress::Tools { done, total },
        native_mods::Step::Install | native_mods::Step::Done { .. } => {
            InstallProgress::Building(Some(1.))
        }
        _ => match step.progress() {
            native_mods::Progress {
                stage: native_mods::Stage::Compile,
                fraction,
            } => InstallProgress::Building(fraction),
            _ => InstallProgress::Preparing,
        },
    });
}

struct SeenPackage {
    package: ScannedPackage,
    digest: Option<String>,
    registry_claim: bool,
    owned: bool,
}

impl SeenPackage {
    fn matches_hold(&self, home: &Path, base: &str, hold: &managed_play::Held) -> bool {
        if hold.reason().build() != base
            || self.package.path != home.join(hold.path())
            || !self.digest.as_ref().is_some_and(|hash| {
                let digest = sha2::digest::Output::<sha2::Sha256>::from(*hold.digest());
                hash.eq_ignore_ascii_case(&format!("{digest:x}"))
            })
        {
            return false;
        }
        match hold.source() {
            managed_play::HeldSource::Registry { version, .. }
            | managed_play::HeldSource::Local { version } => {
                version == &self.package.manifest.version
            }
            managed_play::HeldSource::Github {
                owner,
                repo,
                commit,
                version,
            } => self.github_origin(home).is_some_and(|row| {
                &row.owner == owner
                    && &row.repo == repo
                    && &row.commit == commit
                    && &row.version == version
            }),
        }
    }

    fn github_origin(&self, home: &Path) -> Option<crate::github_mods::Origin> {
        let id = &self.package.manifest.id;
        let target = home.join(native_mods::INSTALLED).join(format!("{id}.smod"));
        if self.package.path != target || self.package.manifest.native.is_none() {
            return None;
        }
        crate::github_mods::origin(home, id).filter(|row| {
            self.digest.as_ref().is_some_and(|digest| {
                row.matches_archive(id, &self.package.manifest.version, digest)
            })
        })
    }
}

type Inventory = BTreeMap<String, SeenPackage>;

fn inventory(home: &Path, authority: Option<&OfficialRegistry>) -> Result<Inventory, String> {
    let doc = Document::read(home)?;
    managed_play::scan_packages(home)?
        .into_iter()
        .map(|p| {
            let digest = (!p.directory)
                .then(|| index::sha256_file(&p.path))
                .transpose()?;
            let id = p.manifest.id.clone();
            let row: Option<installed::Entry> = doc
                .row(&id)?
                .and_then(|row| serde_json::from_value(row.clone()).ok());
            let claim = row.as_ref().zip(digest.as_deref()).filter(|(row, _)| {
                row.source == SourceId::Sunrise
                    && row.kind == Kind::Mod
                    && row.path(home).as_ref() == Some(&p.path)
            });
            let registry_claim = claim.is_some();
            let owned = if let Some((row, digest)) = claim {
                doc.rows
                    .check_other_claims(home, &id, &row.file)
                    .map_err(|error| error.render(trf))?;
                authority.is_some_and(|proof| proof.proves_archive(&id, digest))
            } else {
                false
            };
            Ok((
                id,
                SeenPackage {
                    package: p,
                    digest,
                    registry_claim,
                    owned,
                },
            ))
        })
        .collect()
}

fn stage_package(
    catalogue: &index::Index,
    local: bool,
    home: &Path,
    before: &Inventory,
    p: &index::Project,
    v: &index::Version,
    progress: impl Fn(f32),
) -> Result<installed::Entry, InstallError> {
    let shown = project_of(catalogue, p, &v.game).ok_or_else(unproved_mods)?;
    let native = p.kind == index::Kind::NativeMod;
    let dir = if native {
        native_mods::INSTALLED
    } else {
        mod_list::MODS_DIR
    };
    let relative = format!("{dir}/{}.smod", p.id);
    let target = home.join(&relative);
    let claims = browser_state::Installed::read(home).map_err(|error| error.details(trf))?;
    let existing = before.get(&p.id);
    if existing.is_some_and(|old| !old.owned || !old.package.path.ends_with(&relative)) {
        return Err(InstallError::Refused(trf(
            "{mod} is already installed from another source or location. Nothing was installed.",
            &[("mod", &p.name)],
        )));
    }
    let row = claims.single(&p.id).map_err(|error| error.render(trf))?;
    if row.is_some() != existing.is_some()
        || existing.is_none() && target.try_exists().map_err(|e| at(&target, e))?
    {
        return Err(unproved_mods().into());
    }
    claims
        .check_other_claims(home, &p.id, &relative)
        .map_err(|error| error.render(trf))?;
    let staging = tempfile::Builder::new()
        .prefix("package-")
        .tempdir_in(home)
        .map_err(|e| at(home, e))?;
    let file = &version_of(catalogue, p, v).files[0];
    let fetched = fetch(file, staging.path(), local, &progress).map_err(InstallError::Download)?;
    let (checked, facts) =
        index::check_file_with_facts(catalogue, p, v, &fetched).map_err(InstallError::Download)?;
    if v.dependencies.is_none()
        && facts
            .dependencies
            .values()
            .any(|entry| !entry.details().optional)
    {
        return Err(InstallError::Unproved(trf(
            "The registry didn't list the required dependencies for {mod}. Nothing was installed.",
            &[("mod", &p.name)],
        )));
    }
    let dest = target.parent().expect("managed mod parent");
    fs::create_dir_all(dest).map_err(|e| at(dest, e))?;
    fs::rename(&fetched, &target).map_err(|e| at(&target, e))?;
    Ok(installed::Entry {
        source: SourceId::Sunrise,
        project: p.id.clone(),
        slug: p.id.clone(),
        title: p.name.clone(),
        kind: Kind::Mod,
        version: v.version.clone(),
        number: v.version.clone(),
        file: relative,
        checksum: format!("SHA-256:{}", v.file.sha256),
        page: shown.page,
        dependency: false,
        verdict: Some(trust_label(checked).into()),
        install_hold: None,
    })
}

fn missing_required<'a>(
    manifest: &'a Manifest,
    enabled: &BTreeMap<&str, &Manifest>,
) -> Option<&'a str> {
    manifest.dependencies.iter().find_map(|(id, dependency)| {
        let need = dependency.details();
        (!need.optional
            && need.side != Side::Server
            && !enabled.get(id.as_str()).is_some_and(|other| {
                sunrise_launcher_smod::dependencies::parse_version(&other.version)
                    .is_ok_and(|v| need.version.matches(&v))
            }))
        .then_some(id.as_str())
    })
}

fn manifest_conflict(
    home: &Path,
    catalogue: Option<&index::Index>,
    authority: Option<&OfficialRegistry>,
    staged: impl Fn(&str) -> bool,
) -> Result<Option<resolve::ManifestConflict>, InstallError> {
    let packages = inventory(home, authority)?;
    let enabled: BTreeMap<_, _> = packages
        .iter()
        .filter(|(id, _)| mod_list::is_enabled(home, id))
        .map(|(id, p)| (id.as_str(), &p.package.manifest))
        .collect();
    let file_for = |id: &str| {
        let package = &packages[id];
        resolve::InstalledMod {
            id: id.to_owned(),
            version: package.package.manifest.version.clone(),
            // A freshly checked download names a resolver choice, not replacement authority.
            sha256: (package.owned || staged(id))
                .then(|| package.digest.clone())
                .flatten(),
        }
    };
    let mut namespaces = BTreeMap::new();
    for (&id, &manifest) in &enabled {
        let package = &packages[id];
        let revocation = catalogue.and_then(|catalogue| {
            package.digest.as_ref().and_then(|hash| {
                if package.owned {
                    catalogue.revoked(id, hash)
                } else if manifest.native.is_some() {
                    catalogue.revoked.iter().find(|row| {
                        row.id == id
                            && row
                                .sha256
                                .as_ref()
                                .is_some_and(|listed| listed.eq_ignore_ascii_case(hash))
                    })
                } else {
                    None
                }
            })
        });
        if let Some(revocation) = revocation {
            return Err(InstallError::Refused(trf(
                "{mod} was taken down: {reason}",
                &[("mod", &manifest.name), ("reason", &revocation.reason)],
            )));
        }
        if missing_required(manifest, &enabled).is_some() {
            return Err(InstallError::Refused(trf(
                "The proposed mods don't work with {mod}. Nothing was installed.",
                &[("mod", &manifest.name)],
            )));
        }
        for namespace in manifest.owned_namespaces() {
            if let Some(peer) = namespaces.insert(namespace, id) {
                return Ok(Some(resolve::ManifestConflict {
                    files: [file_for(peer), file_for(id)],
                    reason: trf(
                        "{first} and {second} both own the {namespace} namespace.",
                        &[
                            ("first", &enabled[peer].name),
                            ("second", &manifest.name),
                            ("namespace", namespace),
                        ],
                    ),
                }));
            }
        }
        for (peer, rule) in &manifest.incompatible {
            if let Some(other) = enabled.get(peer.as_str()) {
                let version = sunrise_launcher_smod::dependencies::parse_version(&other.version)
                    .map_err(|error| InstallError::Unproved(error.to_string()))?;
                if rule.details().version.matches(&version) {
                    return Ok(Some(resolve::ManifestConflict {
                        files: [file_for(id), file_for(peer)],
                        reason: trf(
                            "{first} {version} doesn't work with {second} {other_version}.",
                            &[
                                ("first", &manifest.name),
                                ("version", &manifest.version),
                                ("second", &other.name),
                                ("other_version", &other.version),
                            ],
                        ),
                    }));
                }
            }
        }
    }
    Ok(None)
}

fn validate_enabled(
    home: &Path,
    catalogue: &index::Index,
    authority: Option<&OfficialRegistry>,
) -> Result<(), InstallError> {
    match manifest_conflict(home, Some(catalogue), authority, |_| false)? {
        Some(conflict) => Err(InstallError::Refused(conflict.reason)),
        None => Ok(()),
    }
}

fn restore_or_disable(
    original: &Path,
    candidate: &Path,
    before: &Inventory,
    entries: &mut BTreeMap<String, installed::Entry>,
    doc: &mut Document,
    mut remaining: BTreeMap<String, installed::InstallHold>,
) -> Result<Vec<BuildChange>, String> {
    let old = Document::read(original)?;
    let mut changes = Vec::new();
    while !remaining.is_empty() {
        for (id, reason) in std::mem::take(&mut remaining) {
            let entry = entries.get_mut(&id).ok_or_else(unproved_mods)?;
            let title = entry.title.clone();
            let predecessor = if let Some(previous) = before.get(&id) {
                let target = candidate.join(&entry.file);
                fs::copy(&previous.package.path, &target).map_err(|e| at(&target, e))?;
                doc.set(&id, old.row(&id)?.ok_or_else(unproved_mods)?.clone())?;
                mod_list::set_enabled(candidate, &id, mod_list::is_enabled(original, &id))?;
                entries.remove(&id);
                Some(previous.package.manifest.version.clone())
            } else {
                mod_list::set_enabled(candidate, &id, false)?;
                entry.install_hold = Some(reason.clone());
                doc.replace(entry)?;
                None
            };
            changes.push(BuildChange {
                project: id,
                title,
                reason,
                predecessor,
            });
        }
        let packages = inventory(candidate, None)?;
        let enabled: BTreeMap<_, _> = packages
            .iter()
            .filter(|(id, _)| mod_list::is_enabled(candidate, id))
            .map(|(id, p)| (id.as_str(), &p.package.manifest))
            .collect();
        for (&id, &manifest) in &enabled {
            if let Some(missing) = missing_required(manifest, &enabled) {
                let name = packages
                    .get(missing)
                    .map_or(missing, |p| p.package.manifest.name.as_str());
                remaining.insert(
                    id.to_owned(),
                    installed::InstallHold::Needs {
                        name: name.to_owned(),
                    },
                );
            }
        }
    }
    doc.write(candidate)?;
    Ok(changes)
}

fn guard_replacements<'a>(
    home: &Path,
    suspect: &crate::worlds::SuspectMods,
    replacements: impl Iterator<Item = &'a str>,
) -> Result<Vec<crate::worlds::WorldRisk>, String> {
    let reviewed = mutations::world_evidence(home, suspect)?;
    if reviewed.iter().any(|world| !world.mods.is_empty()) {
        let names: Vec<_> = replacements.collect();
        return Err(match names.as_slice() {
            [name] => trf(
                "{mod} is already installed. Update it from Installed.",
                &[("mod", name)],
            ),
            _ => trf(
                "These mods are already installed: {mods}. Update them from Installed.",
                &[("mods", &names.join(", "))],
            ),
        });
    }
    Ok(reviewed)
}

struct Document {
    map: Map<String, Value>,
    rows: browser_state::Installed,
}
impl Document {
    fn read(home: &Path) -> Result<Self, String> {
        let path = home.join(installed::MANIFEST);
        let mut map: Map<String, Value> = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| at(&path, e))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Map::from_iter([("entries".into(), Value::Array(Vec::new()))])
            }
            Err(error) => return Err(at(&path, error)),
        };
        let Some(Value::Array(entries)) = map.remove("entries") else {
            return Err(unproved_mods());
        };
        Ok(Self {
            map,
            rows: browser_state::Installed { entries },
        })
    }
    fn position(&self, id: &str) -> Result<Option<usize>, String> {
        self.rows.single(id).map_err(|e| e.render(trf))
    }
    fn row(&self, id: &str) -> Result<Option<&Value>, String> {
        Ok(self.position(id)?.map(|i| &self.rows.entries[i]))
    }
    fn set(&mut self, id: &str, value: Value) -> Result<(), String> {
        if let Some(i) = self.position(id)? {
            self.rows.entries[i] = value;
        } else {
            self.rows.entries.push(value);
        }
        Ok(())
    }
    fn remove(&mut self, id: &str) -> Result<(), String> {
        if let Some(i) = self.position(id)? {
            self.rows.entries.remove(i);
        }
        Ok(())
    }
    fn replace(&mut self, entry: &installed::Entry) -> Result<(), String> {
        let encoded = serde_json::to_value(entry).map_err(|e| e.to_string())?;
        let mut fields = match self.row(&entry.project)? {
            Some(Value::Object(previous)) => previous.clone(),
            Some(_) => return Err(unproved_mods()),
            None => Map::new(),
        };
        fields.remove("launcher_install_hold");
        fields.remove("verdict");
        let Value::Object(encoded) = encoded else {
            unreachable!("Entry serializes as an object")
        };
        fields.extend(encoded);
        self.set(&entry.project, Value::Object(fields))
    }
    fn write(&self, home: &Path) -> Result<(), String> {
        let mut map = self.map.clone();
        map.insert("entries".into(), Value::Array(self.rows.entries.clone()));
        let bytes = serde_json::to_vec(&map).map_err(|e| e.to_string())?;
        let path = home.join(installed::MANIFEST);
        fs::write(&path, bytes).map_err(|e| at(&path, e))
    }
}

fn held_change_refusal(id: &str) -> InstallError {
    InstallError::Refused(trf(
        "Couldn't verify a usable replacement for the installed hold on {mod}. Nothing was changed.",
        &[("mod", id)],
    ))
}

fn registry_changed() -> String {
    tr("The mod registry changed while the install was being prepared. Nothing was installed.")
        .into()
}

fn unproved_mods() -> String {
    tr("Sunrise couldn't verify the installed mod set. Nothing was installed.").into()
}
fn at(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}

mod github;
mod mutations;
#[cfg(test)]
mod tests;
#[cfg(all(test, debug_assertions, unix))]
mod update_tests;
pub use github::{add_github, update_github};
pub(crate) use mutations::check_activation;
pub use mutations::{remove_mods, ChangeImpact};
