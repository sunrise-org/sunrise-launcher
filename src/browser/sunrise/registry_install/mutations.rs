//! Explicit same-base mutations. Approval is for the exact aggregate impact before publication.
use super::*;
use crate::worlds;
use sunrise_launcher_smod::{dependencies::parse_version, mod_list::ModList};

mod gui;

pub(super) fn update_floors(
    catalogue: &index::Index,
    before: &Inventory,
    game: &str,
    requested: &[String],
) -> Result<BTreeMap<String, semver::Version>, InstallError> {
    let ids = requested
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut above = BTreeMap::new();
    for id in ids {
        let installed_package = before.get(id);
        let Some(old) = installed_package.filter(|p| p.owned) else {
            let name = installed_package.map_or_else(
                || {
                    catalogue
                        .project(id)
                        .map_or(tr("This mod"), |p| p.name.as_str())
                },
                |p| p.package.manifest.name.as_str(),
            );
            return Err(InstallError::Refused(trf(
                "Sunrise couldn't confirm that {name} came from the Sunrise registry, so it can't be updated here.",
                &[("name", name)],
            )));
        };
        let installed =
            parse_version(&old.package.manifest.version).map_err(|_| unproved_mods())?;
        if catalogue.project(id).is_some_and(|p| {
            p.versions.iter().any(|v| {
                fits_game(v, game)
                    && catalogue.revoked(id, &v.file.sha256).is_none()
                    && parse_version(&v.version)
                        .is_ok_and(|new| new.cmp_precedence(&installed).is_gt())
            })
        }) {
            above.insert(id.to_owned(), installed);
        }
    }
    Ok(above)
}

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
    target: ReviewedMod,
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

    pub(super) fn matches_world_review(&self, reviewed: &[worlds::WorldRisk]) -> bool {
        self.evidence == reviewed
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
    let packages = inventory(home, None)?;
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

/// Explicit Remove approves the displayed path and digest, independent of archive origin.
/// The required dependent closure is turned off, not deleted.
pub fn remove_mods(
    instance: &Instance,
    identity: Identity,
    ids: &[String],
    choose: impl FnOnce() -> Result<FirstBuild, String>,
    hooks: InstallHooks<'_>,
    approve: &dyn Fn(&ChangeImpact) -> bool,
) -> Result<Vec<String>, InstallError> {
    if instance.kind == InstanceKind::Pack || ids.is_empty() {
        return Err(InstallError::Cancelled);
    }
    (hooks.progress)(InstallProgress::Checking);
    let mut profile =
        Profile::open_for(&instance.home, None, Some(identity)).map_err(String::from)?;
    let edit = profile.begin_edit(
        choose,
        |f| (hooks.progress)(InstallProgress::Downloading(f)),
        &|_| (hooks.progress)(InstallProgress::Preparing),
    )?;
    let old = edit.candidate()?;
    let next = edit.candidate()?;
    let home = next.path();
    let present = inventory(home, None)?;
    let ids = ids.iter().cloned().collect::<BTreeSet<_>>();
    let mut rows = Document::read(home)?;
    let enabled = enabled(home)?;
    let mut changed_rows = false;
    let mut retired = BTreeSet::new();
    let mut archives = Vec::new();
    let mut reviewed_leaves = Vec::new();
    let live_rows = Document::read(&instance.home)?;
    for id in &ids {
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
            .check_other_claims(&instance.home, id, &file)
            .map_err(|error| error.render(trf))?;
        if let Some(hold) = edit.held(id) {
            if !item.matches_hold(home, edit.base(), hold) {
                return Err(held_change_refusal(id));
            }
            retired.insert(id.clone());
        }
        if rows
            .row(id)?
            .is_some_and(|row| row["kind"] == "mod" && row["file"].as_str() == Some(file.as_str()))
        {
            rows.remove(id)?;
            changed_rows = true;
        }
        if item.github_origin(home).is_some() {
            let origin = item.package.path.with_extension("origin.json");
            let origin_name = origin.strip_prefix(home).map_err(|_| unverified())?;
            let origin_name = origin_name
                .to_str()
                .ok_or_else(unverified)?
                .replace(std::path::MAIN_SEPARATOR, "/");
            live_rows
                .rows
                .check_other_claims(&instance.home, id, &origin_name)
                .map_err(|error| error.render(trf))?;
            reviewed_leaves.push((id.as_str(), origin_name));
            fs::remove_file(&origin).map_err(|e| at(&origin, e))?;
        }
        reviewed_leaves.push((id.as_str(), file.clone()));
        fs::remove_file(&item.package.path).map_err(|e| at(&item.package.path, e))?;
        archives.push(ReviewedArchive {
            target: ReviewedMod {
                name: item.package.manifest.name.clone(),
                id: id.clone(),
            },
            file,
            digest: digest.clone(),
        });
    }
    if changed_rows {
        rows.write(home)?;
    }

    // Only propagate along actual, enabled, required edges. Keep every dependent path and
    // provenance byte; validate the effective enabled graph separately after changing its list.
    let mut disabled = BTreeSet::new();
    let mut unavailable = ids.clone();
    loop {
        let before = unavailable.len();
        for (id, item) in &present {
            if !unavailable.contains(id)
                && enabled.contains(id)
                && item
                    .package
                    .manifest
                    .dependencies
                    .iter()
                    .any(|(dep, rule)| !rule.details().optional && unavailable.contains(dep))
            {
                disabled.insert(id.clone());
                unavailable.insert(id.clone());
            }
        }
        if before == unavailable.len() {
            break;
        }
    }
    let disabled_ids = unavailable.iter().map(String::as_str).collect::<Vec<_>>();
    mod_list::set_all_enabled(home, &disabled_ids, false)?;
    if let Some(conflict) = manifest_conflict(home, None, None, |_| false)? {
        return Err(InstallError::Refused(conflict.reason));
    }
    let removed = archives
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
        &instance.home,
        Change::Remove {
            removed: archives,
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
    let reviewed_image = managed_play::inputs(home, None)?;
    (hooks.progress)(InstallProgress::Reviewing);
    if !approve(&impact) {
        return Err(InstallError::Cancelled);
    }
    let cancellation = native_mods::Cancellation::default();
    let proof = prove_edit(&edit, home, instance, &cancellation, &hooks)
        .map_err(|e| InstallError::Unproved(e.into()))?;
    (hooks.progress)(InstallProgress::Publishing);
    let reviewed = impact.into_evidence();
    edit.publish_install(proof, &cancellation, &retired, || {
        if managed_play::inputs(home, None)? != reviewed_image {
            return Err(tr(
                "The files prepared for removal changed. Nothing was removed; try again.",
            )
            .into());
        }
        // A changed alias can transfer another row's claim without changing any archive bytes.
        let rows = Document::read(&instance.home)?;
        for (id, file) in &reviewed_leaves {
            rows.rows
                .check_other_claims(&instance.home, id, file)
                .map_err(|error| error.render(trf))?;
        }
        bind_worlds(&instance.home, &suspect, &reviewed)
    })?;
    Ok(removed)
}
