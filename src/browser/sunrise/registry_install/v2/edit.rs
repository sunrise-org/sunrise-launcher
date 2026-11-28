//! Installing and updating v2 projects: the public entry points and the one edit they share.

use super::*;

/// The index a change plans against: the signed one, with this computer's fit checks filling
/// only what it says nothing of.
pub(super) fn local_fit_filled(index: &Index) -> Arc<Index> {
    Arc::new(crate::native_mods::local_fit::filled(index))
}

/// Install one or more schema-2 registry projects (by permanent 8-char project ID) into `instance`.
pub fn install_registry_v2(
    target: &Target,
    identity: Identity,
    requested_projects: Vec<String>,
    options: V2InstallOptions,
    choose: impl FnOnce() -> Result<FirstBuild, String>,
    hooks: V2Hooks<'_>,
) -> Result<InstalledBatch, InstallError> {
    let catalog =
        catalogue_v2::change_index().map_err(|error| InstallError::Unproved(error.to_string()))?;
    let stop = hooks.install.cancellation;
    let fetch = |url: &str, max: u64| fetch_release_bytes_cancellable(url, max, stop);
    install_registry_v2_with(
        V2EditRequest {
            target,
            identity,
            index: local_fit_filled(&catalog.index),
            options,
            transfer: V2Transfer {
                fetch_bytes: &fetch,
                verify_release: &attestation::verify_release,
            },
        },
        requested_projects,
        choose,
        hooks,
    )
}

/// Update installed schema-2 registry mods in `instance`, always preserving each mod's chosen
/// project and respecting `chosen-versions.json` pins during bulk updates.
pub fn update_registry_v2(
    target: &Target,
    identity: Identity,
    requested: UpdateRequest,
    options: V2InstallOptions,
    choose: impl FnOnce() -> Result<FirstBuild, String>,
    hooks: V2Hooks<'_>,
    approve: &dyn Fn(&ChangeImpact) -> bool,
) -> Result<InstalledBatch, InstallError> {
    let catalog =
        catalogue_v2::change_index().map_err(|error| InstallError::Unproved(error.to_string()))?;
    let stop = hooks.install.cancellation;
    let fetch = |url: &str, max: u64| fetch_release_bytes_cancellable(url, max, stop);
    update_registry_v2_with(
        V2EditRequest {
            target,
            identity,
            index: local_fit_filled(&catalog.index),
            options,
            transfer: V2Transfer {
                fetch_bytes: &fetch,
                verify_release: &attestation::verify_release,
            },
        },
        requested,
        choose,
        hooks,
        approve,
    )
}

pub(crate) fn install_registry_v2_with(
    request: V2EditRequest<'_>,
    requested_projects: Vec<String>,
    choose: impl FnOnce() -> Result<FirstBuild, String>,
    hooks: V2Hooks<'_>,
) -> Result<InstalledBatch, InstallError> {
    v2_edit_with(
        request,
        V2Action::Install(requested_projects),
        choose,
        hooks,
    )
}

pub(crate) fn update_registry_v2_with(
    request: V2EditRequest<'_>,
    requested: UpdateRequest,
    choose: impl FnOnce() -> Result<FirstBuild, String>,
    hooks: V2Hooks<'_>,
    approve: &dyn Fn(&ChangeImpact) -> bool,
) -> Result<InstalledBatch, InstallError> {
    v2_edit_with(request, V2Action::Update(requested, approve), choose, hooks)
}

enum V2Action<'a> {
    Install(Vec<String>),
    Update(UpdateRequest, &'a dyn Fn(&ChangeImpact) -> bool),
}

fn v2_edit_with(
    request: V2EditRequest<'_>,
    action: V2Action<'_>,
    choose: impl FnOnce() -> Result<FirstBuild, String>,
    hooks: V2Hooks<'_>,
) -> Result<InstalledBatch, InstallError> {
    let V2EditRequest {
        target,
        identity,
        index,
        mut options,
        transfer,
    } = request;
    options
        .jar_cache
        .get_or_insert_with(|| target.home().join("jars"));
    if target.is_pack() {
        return Err(InstallError::Refused(
            tr("No registry mods to install in this profile.").into(),
        ));
    }
    (hooks.install.progress)(InstallProgress::Checking);
    if matches!(&action, V2Action::Install(ids) if ids.is_empty()) {
        return Err(InstallError::Cancelled);
    }

    let mut profile =
        Profile::open_for(target.home(), None, Some(identity)).map_err(String::from)?;
    let default_cancelled = native_mods::Cancellation::default();
    let cancelled = hooks.install.cancellation.unwrap_or(&default_cancelled);
    let edit = profile
        .begin_edit_cancellable(
            choose,
            |f| (hooks.install.progress)(InstallProgress::Downloading(f)),
            Some(cancelled),
        )
        .map_err(|error| stopped_or(Some(cancelled), error))?;
    // The game build is in: say the checks that follow, not its last percentage.
    (hooks.install.progress)(InstallProgress::Checking);
    let base_build = edit.base().to_owned();
    let old = edit.candidate()?;
    let before = inventory_v2(old.path(), &index)?;
    let chosen_pins = chosen(target.home());

    let plan = match &action {
        V2Action::Install(projects) => plan_v2_install(
            &index,
            &base_build,
            projects,
            &before,
            &chosen_pins,
            &options,
            hooks.choose_provider,
        )?,
        V2Action::Update(request, _) => plan_v2_update(
            &index,
            &base_build,
            request,
            &before,
            &chosen_pins,
            &options,
            hooks.choose_provider,
        )?,
    };
    if plan.steps.is_empty() {
        return Ok(InstalledBatch::default());
    }

    let pin_dir = options.pin_dir.clone().unwrap_or_else(backend::data_dir);
    let next = edit.candidate()?;
    let mut doc = Document::read(next.path())?;
    let mut entries: BTreeMap<String, installed::Entry> = BTreeMap::new();
    let mut pending_pins: Vec<PendingPin> = Vec::new();
    let mut native = false;
    let mut replacements: Vec<(String, String)> = Vec::new();
    let mut retired_holds: BTreeSet<String> = BTreeSet::new();

    for (slot_idx, step) in plan.steps.iter().enumerate() {
        // Stop between releases: nothing staged so far is published, and the candidate is
        // dropped with the edit.
        cancelled.check().map_err(|_| InstallError::Cancelled)?;
        (hooks.install.progress)(InstallProgress::Downloading(
            slot_idx as f32 / plan.steps.len().max(1) as f32,
        ));
        let (mut entry, pin) = stage_and_verify_v2_release(
            &index,
            next.path(),
            step,
            &pin_dir,
            &options.accepted_publisher_changes,
            hooks.confirm_publisher_change,
            transfer,
        )
        .map_err(|error| {
            if matches!(error, InstallError::Download(_)) {
                stopped_or(Some(cancelled), error)
            } else {
                error
            }
        })?;
        // Preserve player-owned dependency flag if previously explicitly installed.
        if let Ok(Some(prev_row)) = doc.row(&step.mod_id) {
            if prev_row["dependency"].as_bool() == Some(false) {
                entry.dependency = false;
            }
        }
        // Check active Switch hold authority before replacing a held archive.
        if let Some(held) = edit.held(&step.mod_id) {
            let can_replace = matches!(action, V2Action::Update(..))
                && before.get(&step.mod_id).is_some_and(|prev| {
                    prev.ownership.is_owned()
                        && matches!(held.source(), managed_play::HeldSource::Registry { .. })
                        && prev.digest.eq_ignore_ascii_case(&{
                            let d = sha2::digest::Output::<sha2::Sha256>::from(*held.digest());
                            format!("{d:x}")
                        })
                });
            if !can_replace {
                return Err(held_change_refusal(&step.mod_id));
            }
            retired_holds.insert(step.mod_id.clone());
        }
        doc.replace(&entry)?;
        mod_list::set_enabled(next.path(), &step.mod_id, true)?;
        native |= step.kind == V2Kind::NativeMod;
        if let Some(prev) = before.get(&step.mod_id) {
            replacements.push((prev.package.manifest.name.clone(), step.mod_id.clone()));
        }
        entries.insert(step.mod_id.clone(), entry);
        pending_pins.push(pin);
    }
    doc.write(next.path())?;
    // The releases are in: what follows checks them and fetches their original jars, so it says
    // so rather than staying on the last download's percentage (sunrise-org/sunrise-launcher#376).
    (hooks.install.progress)(InstallProgress::Checking);

    // Check enabled manifests and v2 revocations on the staged candidate before any build.
    check_staged_v2_candidate(next.path(), &index, |id| entries.contains_key(id))?;

    // A reviewed registry release asks nothing before its build: its attestation and the
    // signed index were verified while it was fetched and staged above.
    if native {
        if let Some(error) = native_mods::known_host_block(Some(&base_build)) {
            return Err(InstallError::Host(error));
        }
        cancelled.check().map_err(|_| InstallError::Cancelled)?;
    }

    // Every original jar the new set of registry mods lists is in this profile's cache (the game
    // reads `<home>/jars`) before anything is published; a jar Modrinth no longer has fails the
    // edit here.
    let staged = inventory_v2(next.path(), &index)?;
    let listed = collect_jar_requirements(
        staged
            .values()
            .filter(|item| item.enabled && item.ownership.is_owned())
            .map(|item| {
                (
                    item.package.manifest.id.as_str(),
                    item.package.manifest.jar.as_slice(),
                )
            }),
    )
    .map_err(|error| InstallError::Refused(jars::LaunchError::Conflict(error).to_string()))?;
    cancelled.check().map_err(|_| InstallError::Cancelled)?;
    jars::fetch_listed_cancellable(target.home(), &listed, Some(cancelled))
        .map_err(|error| stopped_or(Some(cancelled), InstallError::Download(error.to_string())))?;
    cancelled.check().map_err(|_| InstallError::Cancelled)?;

    (hooks.install.progress)(InstallProgress::Preparing);
    edit.prepare_first_cancellable(
        &super::preparation_line(hooks.install.progress),
        Some(cancelled),
    )
    .map_err(|error| stopped_or(Some(cancelled), InstallError::Unproved(error)))?;
    let proof = match prove_edit(&edit, next.path(), target, cancelled, &hooks.install) {
        Ok(proof) => proof,
        Err(native_mods::Failed::Mods {
            left,
            error,
            compiling: false,
            ..
        }) => {
            for failed in &left {
                if matches!(failed.why, native_mods::Why::Unfit(_)) {
                    if let Some(step) = plan.steps.iter().find(|step| {
                        step.kind == V2Kind::NativeMod
                            && step.mod_id == failed.id
                            && step.version == failed.version
                    }) {
                        remember_unfit_sha256(&base_build, &step.sha256);
                    }
                }
            }
            return Err(InstallError::Unproved(error));
        }
        Err(_) if cancelled.is_stopped() => return Err(InstallError::Cancelled),
        Err(error) => return Err(InstallError::Unproved(error.to_string())),
    };

    // World impact review for updates (or guard against unexpected replacements on Install).
    let (suspect, reviewed_worlds) = match &action {
        V2Action::Install(_) => {
            // A dependency that replaces an installed mod may drop what saved worlds use.
            let suspect = if replacements.is_empty() {
                crate::worlds::SuspectMods::new()
            } else {
                crate::worlds::unproved_mod_effects(
                    old.path(),
                    edit.current(),
                    next.path(),
                    edit.current(),
                    replacements.iter().map(|(_, id)| id.as_str()),
                )?
            };
            let reviewed = guard_replacements(
                target.home(),
                &suspect,
                replacements.iter().map(|(name, _)| name.as_str()),
            )?;
            (suspect, reviewed)
        }
        V2Action::Update(_, approve) => {
            let old_packages: Vec<&ScannedPackage> = entries
                .keys()
                .filter_map(|id| before.get(id).map(|m| &m.package))
                .collect();
            let (suspect, unchecked) = mutations::update_worlds(next.path(), old_packages)?;
            let reviewed_updates: Vec<mutations::ReviewedUpdate> = entries
                .iter()
                .map(|(id, entry)| mutations::ReviewedUpdate {
                    name: entry.title.clone(),
                    installed: before.get(id).map(|p| p.package.manifest.version.clone()),
                    replacement: entry.version.clone(),
                })
                .collect();
            let impact = ChangeImpact::read(
                target.home(),
                mutations::Change::Update(reviewed_updates),
                &suspect,
            )?
            .with_unchecked_update(unchecked);
            if !approve(&impact) {
                return Err(InstallError::Cancelled);
            }
            (suspect, impact.into_evidence())
        }
    };

    (hooks.install.progress)(InstallProgress::Publishing);
    cancelled.check().map_err(|_| InstallError::Cancelled)?;
    // Persist verified publisher pins before publishing so a pin-store failure aborts the
    // install cleanly instead of failing after the mods are already live.
    commit_pending_pins(&pin_dir, &pending_pins).map_err(InstallError::Unproved)?;
    let authorize = || mutations::bind_worlds(target.home(), &suspect, &reviewed_worlds);
    edit.publish_install(proof, cancelled, &retired_holds, authorize)?;

    let mut result = InstalledBatch::default();
    for (id, entry) in entries {
        if let Some(previous) = before.get(&id) {
            result
                .updated
                .push((previous.package.manifest.version.clone(), entry));
        } else {
            result.added.push(entry);
        }
    }
    Ok(result)
}
