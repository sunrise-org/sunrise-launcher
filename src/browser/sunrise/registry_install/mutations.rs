//! Explicit same-base mutations. Approval is for the exact aggregate impact before publication.
use super::*;
use crate::worlds;
use sunrise_launcher_smod::{dependencies::parse_version, mod_list::ModList};

mod gui;

pub(super) fn enabled(home: &Path) -> Result<BTreeSet<String>, String> {
    let path = home.join(sunrise_launcher_smod::mod_list::FILE);
    match fs::read_to_string(&path) {
        Ok(text) => Ok(ModList::parse(&text).enabled),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeSet::new()),
        Err(error) => Err(at(&path, error)),
    }
}

pub(super) fn world_evidence(
    home: &Path,
    suspect: &worlds::SuspectMods,
) -> Result<Vec<worlds::WorldRisk>, String> {
    if suspect.is_empty() {
        return Ok(Vec::new());
    }
    worlds::review_risk(home, suspect)
}

pub(super) fn bind_worlds(
    home: &Path,
    suspect: &worlds::SuspectMods,
    reviewed: &[worlds::WorldRisk],
) -> Result<worlds::ReviewedWorldLocks, String> {
    let changed = || {
        tr("The affected worlds changed after review. Nothing was changed; try again.").to_owned()
    };
    let locks = match worlds::lock_reviewed(reviewed) {
        Ok(locks) => locks,
        Err(error) => {
            return Err(if world_evidence(home, suspect)? != reviewed {
                changed()
            } else {
                error
            });
        }
    };
    if world_evidence(home, suspect)? != reviewed {
        return Err(changed());
    }
    Ok(locks)
}

/// Inspect only replaced packages on Update. An opaque package warrants a note, not consent;
/// only a known drop contributes namespaces to the saved-world review and cutoff fence.
pub(super) fn update_worlds<'a>(
    new_home: &Path,
    old: impl IntoIterator<Item = &'a managed_play::ScannedPackage>,
) -> Result<(worlds::SuspectMods, bool), String> {
    let new: BTreeMap<_, _> = managed_play::scan_packages(new_home)?
        .into_iter()
        .map(|package| (package.manifest.id.clone(), package))
        .collect();
    let mut drops = worlds::SuspectMods::new();
    let mut unchecked = false;
    for previous in old {
        let id = &previous.manifest.id;
        match new
            .get(id)
            .and_then(|next| worlds::update_drops(previous, next))
        {
            Some(namespaces) if !namespaces.is_empty() => {
                drops.insert(id.clone(), namespaces);
            }
            Some(_) => {}
            None => unchecked = true,
        }
    }
    Ok((drops, unchecked))
}

#[derive(Debug)]
pub struct ChangeImpact {
    change: Change,
    evidence: Vec<worlds::WorldRisk>,
    unchecked_update: bool,
}

#[derive(Debug)]
pub(super) enum Change {
    Update(Vec<ReviewedUpdate>),
    GithubUpdate {
        target: ReviewedMod,
        file: String,
        releases: [ReviewedRelease; 2],
    },
    Remove {
        removed: Vec<ReviewedArchive>,
        disabled: Vec<ReviewedMod>,
    },
}

#[derive(Debug)]
pub(super) struct ReviewedMod {
    pub(super) name: String,
    pub(super) id: String,
}

impl ReviewedMod {
    fn label(&self) -> String {
        format!("{} ({})", self.name, self.id)
    }
}

#[derive(Debug)]
pub(super) struct ReviewedUpdate {
    pub(super) name: String,
    pub(super) installed: Option<String>,
    pub(super) replacement: String,
}

#[derive(Debug)]
pub(super) struct ReviewedRelease {
    label: String,
    sha256: String,
}

impl From<&crate::github_mods::Origin> for ReviewedRelease {
    fn from(origin: &crate::github_mods::Origin) -> Self {
        Self {
            label: format!(
                "{} {} ({})",
                origin.version,
                origin.label(),
                origin.asset.as_deref().unwrap_or_default(),
            ),
            sha256: origin.sha256.clone(),
        }
    }
}

#[derive(Debug)]
pub(super) struct ReviewedArchive {
    pub(super) target: ReviewedMod,
    file: String,
    digest: String,
}

impl ChangeImpact {
    pub(super) fn read(
        home: &Path,
        change: Change,
        suspect: &worlds::SuspectMods,
    ) -> Result<Self, String> {
        Ok(Self {
            change,
            evidence: world_evidence(home, suspect)?,
            unchecked_update: false,
        })
    }

    pub(super) fn with_unchecked_update(mut self, unchecked: bool) -> Self {
        self.unchecked_update = unchecked;
        self
    }

    pub(super) fn into_evidence(self) -> Vec<worlds::WorldRisk> {
        self.evidence
    }

    fn affected(&self) -> impl Iterator<Item = &worlds::WorldRisk> {
        self.evidence.iter().filter(|world| !world.mods.is_empty())
    }

    pub fn risks_worlds(&self) -> bool {
        self.affected().next().is_some()
    }

    pub fn is_removal(&self) -> bool {
        matches!(self.change, Change::Remove { .. })
    }

    /// The heading and warning shown by the GUI. Remove uses labels from its Installed view;
    /// a dependent with no visible row uses its manifest name. Audit details are separate.
    pub fn gui_review(&self, displayed_names: &BTreeMap<String, String>) -> (String, String) {
        self.change.gui_review(
            self.affected().count(),
            self.unchecked_update,
            displayed_names,
        )
    }

    pub fn detail(&self) -> String {
        let mut lines = Vec::new();
        match &self.change {
            Change::Update(mods) => {
                let labels = mods
                    .iter()
                    .map(|update| match &update.installed {
                        Some(old) => format!("{} {old} → {}", update.name, update.replacement),
                        None => trf(
                            "Install {mod} {version}",
                            &[("mod", &update.name), ("version", &update.replacement)],
                        ),
                    })
                    .collect::<Vec<_>>();
                lines.push(trf("Update: {mods}.", &[("mods", &labels.join(", "))]));
            }
            Change::GithubUpdate {
                target,
                file,
                releases,
            } => {
                lines.push(trf("Update: {mods}.", &[("mods", &target.label())]));
                for (label, release) in [tr("Installed archive"), tr("Replacement archive")]
                    .into_iter()
                    .zip(releases)
                {
                    lines.push(format!(
                        "{label}: {}\n{file}\nSHA-256: {}",
                        release.label, release.sha256
                    ));
                }
            }
            Change::Remove {
                removed, disabled, ..
            } => {
                let names = removed
                    .iter()
                    .map(|archive| archive.target.label())
                    .collect::<Vec<_>>();
                lines.push(trf("Remove: {mods}.", &[("mods", &names.join(", "))]));
                lines.extend(
                    removed
                        .iter()
                        .map(|archive| format!("{}\nSHA-256: {}", archive.file, archive.digest)),
                );
                if !disabled.is_empty() {
                    lines.push(trf(
                        "These mods need the ones you are removing. They will be turned off, but their files will stay installed: {mods}.",
                        &[("mods", &disabled.iter().map(ReviewedMod::label).collect::<Vec<_>>().join(", "))],
                    ));
                }
            }
        }
        match self.affected().count() {
            0 if self.unchecked_update => lines.push(
                tr("Sunrise can't check whether this update removes blocks or items your worlds use.")
                    .into(),
            ),
            0 => (),
            1 => lines.push(tr(
                "You may lose blocks and items from the affected mods when you open 1 saved world without their definitions.",
            ).into()),
            n => lines.push(trf(
                "You may lose blocks and items from the affected mods when you open {count} saved worlds without their definitions.",
                &[("count", &n.to_string())],
            )),
        }
        if self.affected().next().is_some() {
            lines.push(
                tr("Sunrise can't tell which affected blocks and items are still in those worlds.")
                    .into(),
            );
        }
        lines.join("\n\n")
    }
}

/// The ordinary enable switch must not reactivate an exact hold or a mod missing a required
/// dependency. Call under the game's manifest lock and write the list under the same lock.
pub(crate) fn check_activation(
    home: &Path,
    active: Option<&managed_play::Active>,
    activate: &[&str],
) -> Result<(), String> {
    let packages = inventory(home)?;
    let mut enabled = enabled(home)?;
    enabled.extend(activate.iter().map(|&id| id.to_owned()));
    let manifests = packages
        .iter()
        .filter(|(id, _)| enabled.contains(*id))
        .map(|(id, p)| (id.as_str(), &p.package.manifest))
        .collect::<BTreeMap<_, _>>();
    let mut visited = BTreeSet::new();
    let mut remaining = activate.to_vec();
    while let Some(id) = remaining.pop() {
        if !visited.insert(id) {
            continue;
        }
        if let Some(hold) = active.and_then(|active| active.holds().find(|hold| hold.id() == id)) {
            return Err(trf(
                "{mod}: {reason}.",
                &[("mod", id), ("reason", hold.reason().label())],
            ));
        }
        let target = manifests.get(id).ok_or_else(unproved_mods)?;
        if let Some(host) = target.served_by.as_deref() {
            let serving = manifests.get(host);
            if !serving.is_some_and(|host| host.native.as_ref().is_some_and(|n| n.serves_content)) {
                let name = serving.map_or(host, |h| h.name.as_str());
                return Err(trf(
                    "{mod} is served by {host}. Install and turn on {host} before trying again.",
                    &[("mod", &target.name), ("host", name)],
                ));
            }
            // The game's `side_covers`: the host runs everywhere the package it serves must.
            if let Some(serving) = serving.filter(|h| h.side != Side::Both && h.side != target.side)
            {
                return Err(trf(
                    "{host} can't serve {mod}: it doesn't run everywhere {mod} must.",
                    &[("mod", &target.name), ("host", &serving.name)],
                ));
            }
            remaining.push(host);
        }
        for (name, dep) in &target.dependencies {
            let requirement = dep.details();
            if requirement.optional {
                continue;
            }
            if !manifests.get(name.as_str()).is_some_and(|other| {
                parse_version(&other.version)
                    .is_ok_and(|version| requirement.version.matches(&version))
            }) {
                return Err(trf("{mod} needs {required} {version}. Install or turn on a version that fits before trying again.", &[
                    ("mod", &target.name), ("required", name), ("version", &requirement.version.to_string()),
                ]));
            }
            remaining.push(name.as_str());
        }
    }
    Ok(())
}

/// `removed` and every enabled package that needs one of them: through a required dependency, or
/// as the content a removed native host serves. Only actual, enabled edges propagate; the
/// effective enabled graph is validated separately after its list changes.
pub(super) fn unavailable_with(
    present: &Inventory,
    enabled: &BTreeSet<String>,
    removed: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut unavailable = removed.clone();
    loop {
        let before = unavailable.len();
        for (id, item) in present {
            let manifest = &item.package.manifest;
            if !unavailable.contains(id)
                && enabled.contains(id)
                && (manifest
                    .dependencies
                    .iter()
                    .any(|(dep, rule)| !rule.details().optional && unavailable.contains(dep))
                    || manifest
                        .served_by
                        .as_ref()
                        .is_some_and(|host| unavailable.contains(host)))
            {
                unavailable.insert(id.clone());
            }
        }
        if before == unavailable.len() {
            return unavailable;
        }
    }
}

/// Exact archive leaves selected for removal in a private profile edit.
pub(super) struct StagedRemoval {
    pub(super) archives: Vec<ReviewedArchive>,
    retired: BTreeSet<String>,
    reviewed_leaves: Vec<(String, String)>,
}

impl StagedRemoval {
    pub(super) fn revalidate(&self, live: &Path) -> Result<(), String> {
        // A changed alias can transfer another row's claim without changing any archive bytes.
        let rows = Document::read(live)?;
        for (id, file) in &self.reviewed_leaves {
            rows.rows
                .check_other_claims(live, id, file)
                .map_err(|error| error.render(trf))?;
        }
        Ok(())
    }
}

pub(super) fn stage_removal<'a>(
    live: &Path,
    home: &Path,
    active: Option<&managed_play::Active>,
    ids: impl IntoIterator<Item = &'a str>,
    present: &Inventory,
) -> Result<StagedRemoval, InstallError> {
    let mut rows = Document::read(home)?;
    let mut changed_rows = false;
    let mut retired = BTreeSet::new();
    let mut archives = Vec::new();
    let mut reviewed_leaves = Vec::new();
    let live_rows = Document::read(live)?;
    for id in ids {
        let unverified = || {
            trf(
                "Couldn't verify the installed file for {mod}. Nothing was removed.",
                &[("mod", id)],
            )
        };
        let item = present.get(id).ok_or_else(unverified)?;
        let digest = item.digest.as_ref().ok_or_else(unverified)?;
        let relative = item
            .package
            .path
            .strip_prefix(home)
            .map_err(|_| unverified())?;
        let file = relative
            .to_str()
            .ok_or_else(unverified)?
            .replace(std::path::MAIN_SEPARATOR, "/");
        // The live home retains filesystem aliases which a private copy can't reproduce.
        live_rows
            .rows
            .check_other_claims(live, id, &file)
            .map_err(|error| error.render(trf))?;
        if let Some((active, hold)) = active.and_then(|active| {
            active
                .holds()
                .find(|hold| hold.id() == id)
                .map(|hold| (active, hold))
        }) {
            if !item.matches_hold(home, active.base(), hold) {
                return Err(held_change_refusal(id));
            }
            retired.insert(id.to_owned());
        }
        if item.registry_claim || raw_row_matches_archive(home, &rows, id, &item.package.path)? {
            rows.remove(id)?;
            changed_rows = true;
        }
        if let Some((origin, _)) = item.verified_origin_sidecar(home).ok().flatten() {
            let origin_name = origin.strip_prefix(home).map_err(|_| unverified())?;
            let origin_name = origin_name
                .to_str()
                .ok_or_else(unverified)?
                .replace(std::path::MAIN_SEPARATOR, "/");
            live_rows
                .rows
                .check_other_claims(live, id, &origin_name)
                .map_err(|error| error.render(trf))?;
            reviewed_leaves.push((id.to_owned(), origin_name));
            fs::remove_file(&origin).map_err(|e| at(&origin, e))?;
        }
        reviewed_leaves.push((id.to_owned(), file.clone()));
        fs::remove_file(&item.package.path).map_err(|e| at(&item.package.path, e))?;
        archives.push(ReviewedArchive {
            target: ReviewedMod {
                name: item.package.manifest.name.clone(),
                id: id.to_owned(),
            },
            file,
            digest: digest.clone(),
        });
    }
    if changed_rows {
        rows.write(home)?;
    }

    Ok(StagedRemoval {
        archives,
        retired,
        reviewed_leaves,
    })
}

/// Match a Sunrise browser row by source, kind and portable archive path directly on the raw JSON
/// row so removal cleans up rows missing display fields (such as `page`) without loosening
/// `registry_claim` for replacements.
fn raw_row_matches_archive(
    home: &Path,
    rows: &Document,
    id: &str,
    archive_path: &Path,
) -> Result<bool, String> {
    let Some(row) = rows.row(id)? else {
        return Ok(false);
    };
    let source = serde_json::from_value::<SourceId>(row["source"].clone()).ok();
    let kind = serde_json::from_value::<Kind>(row["kind"].clone()).ok();
    let path_matches = row["file"]
        .as_str()
        .and_then(|file| portable_row_path(home, file))
        .is_some_and(|(_, path)| path == archive_path);
    Ok(source == Some(SourceId::Sunrise) && kind == Some(Kind::Mod) && path_matches)
}

/// Explicit Remove approves the displayed path and digest, independent of archive origin.
/// The required dependent closure is turned off, not deleted.
pub fn remove_mods(
    target: &Target,
    identity: Identity,
    ids: &[String],
    choose: impl FnOnce() -> Result<FirstBuild, String>,
    hooks: InstallHooks<'_>,
    approve: &dyn Fn(&ChangeImpact) -> bool,
) -> Result<Vec<String>, InstallError> {
    if target.is_pack() || ids.is_empty() {
        return Err(InstallError::Cancelled);
    }
    (hooks.progress)(InstallProgress::Checking);
    let mut profile =
        Profile::open_for(target.home(), None, Some(identity)).map_err(String::from)?;
    let edit = profile
        .begin_edit_cancellable(
            choose,
            |f| (hooks.progress)(InstallProgress::Downloading(f)),
            hooks.cancellation,
        )
        .map_err(|error| super::stopped_or(hooks.cancellation, error))?;
    // The game build is in: say the checks that follow, not its last percentage.
    (hooks.progress)(InstallProgress::Checking);
    edit.prepare_first_cancellable(&super::preparation_line(hooks.progress), hooks.cancellation)
        .map_err(|error| super::stopped_or(hooks.cancellation, error))?;
    let old = edit.candidate()?;
    let next = edit.candidate()?;
    let home = next.path();
    let present = inventory(home)?;
    let ids = ids.iter().cloned().collect::<BTreeSet<_>>();
    let enabled = enabled(home)?;
    let mut staged = stage_removal(
        target.home(),
        home,
        edit.current(),
        ids.iter().map(String::as_str),
        &present,
    )?;

    let unavailable = unavailable_with(&present, &enabled, &ids);
    let disabled: BTreeSet<String> = unavailable.difference(&ids).cloned().collect();
    let disabled_ids = unavailable.iter().map(String::as_str).collect::<Vec<_>>();
    mod_list::set_all_enabled(home, &disabled_ids, false)?;
    if let Some(conflict) = manifest_conflict(home, Side::Both, |_| false)? {
        return Err(InstallError::Refused(conflict.reason));
    }
    let removed = staged
        .archives
        .iter()
        .map(|archive| archive.target.label())
        .collect::<Vec<_>>();
    let suspect = worlds::unproved_mod_effects(
        old.path(),
        edit.current(),
        home,
        edit.current(),
        unavailable.iter().map(String::as_str),
    )?;
    let impact = ChangeImpact::read(
        target.home(),
        Change::Remove {
            removed: std::mem::take(&mut staged.archives),
            disabled: disabled
                .into_iter()
                .map(|id| ReviewedMod {
                    name: present[&id].package.manifest.name.clone(),
                    id,
                })
                .collect(),
        },
        &suspect,
    )?;
    let reviewed_image = managed_play::inputs(home)?;
    (hooks.progress)(InstallProgress::Reviewing);
    if !approve(&impact) {
        return Err(InstallError::Cancelled);
    }
    // Stop reaches a Remove that waits for another build to finish.
    let unstoppable = native_mods::Cancellation::default();
    let cancellation = hooks.cancellation.unwrap_or(&unstoppable);
    let proof = prove_edit(&edit, home, target, cancellation, &hooks).map_err(|e| {
        if cancellation.is_stopped() {
            InstallError::Cancelled
        } else {
            InstallError::Unproved(e.into())
        }
    })?;
    let listed: Vec<_> = ids
        .iter()
        .filter_map(|id| present.get(id))
        .flat_map(|item| item.package.manifest.jar.iter().cloned())
        .collect();
    (hooks.progress)(InstallProgress::Publishing);
    let reviewed = impact.into_evidence();
    edit.publish_install(proof, cancellation, &staged.retired, || {
        if managed_play::inputs(home)? != reviewed_image {
            return Err(tr(
                "The files prepared for removal changed. Nothing was removed; try again.",
            )
            .into());
        }
        staged.revalidate(target.home())?;
        bind_worlds(target.home(), &suspect, &reviewed)
    })?;
    // The removal is published. A jar goes from this profile's cache (the game reads
    // `<home>/jars`) unless a mod still in the profile lists it; when the profile can't be read,
    // every jar stays.
    if let Ok(packages) = managed_play::scan_packages(target.home()) {
        let kept: Vec<_> = packages
            .into_iter()
            .flat_map(|package| package.manifest.jar)
            .collect();
        let _ = crate::jars::forget(target.home(), &listed, &kept);
    }
    Ok(removed)
}
