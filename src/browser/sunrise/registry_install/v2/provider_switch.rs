//! Switching an installed mod slot to another project that provides it, with its dependents and the staged result checked first.

use super::*;

/// Explicitly switch an installed mod slot (`mod_id`) to a different v2 project (`to_project_id`),
/// previewing affected dependents and saved-world risks before committing.
pub fn switch_provider_v2(
    target: &Target,
    identity: Identity,
    mod_id: &str,
    to_project_id: &str,
    options: V2InstallOptions,
    choose: impl FnOnce() -> Result<FirstBuild, String>,
    hooks: V2ProviderSwitchHooks<'_>,
) -> Result<InstalledBatch, InstallError> {
    let catalog =
        catalogue_v2::change_index().map_err(|error| InstallError::Unproved(error.to_string()))?;
    let stop = hooks.install.install.cancellation;
    let fetch = |url: &str, max: u64| fetch_release_bytes_cancellable(url, max, stop);
    switch_provider_v2_with(
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
        mod_id,
        to_project_id,
        choose,
        hooks,
    )
}

/// Internal testable entrypoint for explicit provider switching.
pub(crate) fn switch_provider_v2_with(
    request: V2EditRequest<'_>,
    mod_id: &str,
    to_project_id: &str,
    choose: impl FnOnce() -> Result<FirstBuild, String>,
    hooks: V2ProviderSwitchHooks<'_>,
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
    let to_project = index.project(to_project_id).ok_or_else(|| {
        InstallError::Refused(trf(
            "Project {project} is not in the Sunrise registry.",
            &[("project", to_project_id)],
        ))
    })?;
    if to_project.mod_id != mod_id {
        return Err(InstallError::Refused(trf(
            "{project} provides {actual}, not {expected}.",
            &[
                ("project", &to_project.name),
                ("actual", &to_project.mod_id),
                ("expected", mod_id),
            ],
        )));
    }
    if !matches!(to_project.kind, V2Kind::Mod | V2Kind::NativeMod) {
        return Err(InstallError::Refused(trf(
            "{mod} is not a mod package.",
            &[("mod", &to_project.name)],
        )));
    }

    (hooks.install.install.progress)(InstallProgress::Checking);
    let mut profile =
        Profile::open_for(target.home(), None, Some(identity)).map_err(String::from)?;
    let default_cancelled = native_mods::Cancellation::default();
    let cancelled = hooks
        .install
        .install
        .cancellation
        .unwrap_or(&default_cancelled);
    let edit = profile
        .begin_edit_cancellable(
            choose,
            |f| (hooks.install.install.progress)(InstallProgress::Downloading(f)),
            Some(cancelled),
        )
        .map_err(|error| stopped_or(Some(cancelled), error))?;
    // The game build is in: say the checks that follow, not its last percentage.
    (hooks.install.install.progress)(InstallProgress::Checking);
    let base_build = edit.base().to_owned();
    let old = edit.candidate()?;
    let before = inventory_v2(old.path(), &index)?;

    let current = before.get(mod_id).ok_or_else(|| {
        InstallError::Refused(trf(
            "{mod} is not installed in this profile.",
            &[("mod", mod_id)],
        ))
    })?;
    if current.ownership.project_id() == Some(to_project.id.as_str()) {
        return Err(InstallError::Refused(trf(
            "{mod} is already using {project}.",
            &[("mod", mod_id), ("project", &to_project.name)],
        )));
    }

    let unfit_hashes = known_unfit_updates(&base_build);
    let jar_cache = options.jar_cache.as_deref();
    let version_override = options.versions.get(&to_project.id).map(String::as_str);
    let release = select_project_release(
        &index,
        to_project,
        &base_build,
        version_override,
        None,
        &unfit_hashes,
        jar_cache,
    )?;
    let step = planned_from(to_project, release, false);

    // An enabled dependent pinned to the old project, or whose requirement the new release can't
    // meet, refuses the switch before anything downloads.
    if let Some(incompatible) =
        find_affected_dependents(&index, &before, mod_id, &to_project.id, &release.version).first()
    {
        return Err(InstallError::Refused(trf(
            "Cannot switch {mod} to {project}: {dependent} ({reason}).",
            &[
                ("mod", mod_id),
                ("project", &to_project.name),
                ("dependent", &incompatible.name),
                ("reason", &incompatible.reason),
            ],
        )));
    }

    // Plan any new required dependencies of the incoming provider.
    let mut dep_queue: VecDeque<Queued> = VecDeque::new();
    for dep in &release.dependencies {
        if !dep.required || dep.mod_id == mod_id {
            continue;
        }
        let range = Some(dep.version.clone());
        let installed = before.get(&dep.mod_id).filter(|m| m.enabled);
        let in_range_now =
            installed.is_some_and(|m| in_range(Some(&dep.version), &m.package.manifest.version));
        if let Some(dep_pid) = dep.project.as_deref() {
            if installed.is_none_or(|m| m.ownership.project_id() != Some(dep_pid)) || !in_range_now
            {
                dep_queue.push_back((dep_pid.to_owned(), true, to_project.name.clone(), range));
            }
        } else if let Some(owner) = installed
            .filter(|_| !in_range_now)
            .and_then(|m| m.ownership.project_id())
        {
            // Keep the profile's provider, at a release in the range.
            dep_queue.push_back((owner.to_owned(), true, to_project.name.clone(), range));
        } else if installed.is_none() {
            if let Some(picked) = options.provider_choices.get(&dep.mod_id) {
                dep_queue.push_back((picked.clone(), true, to_project.name.clone(), range));
            } else {
                let mut candidates: Vec<ProviderCandidate> = index
                    .projects_for_mod(&dep.mod_id)
                    .filter(|p| {
                        matches!(p.kind, V2Kind::Mod | V2Kind::NativeMod)
                            && index.revoked(&p.id, None).is_none()
                    })
                    .filter_map(|p| {
                        let best = p
                            .releases
                            .iter()
                            .filter(|r| {
                                r.fit_verdict(&base_build) == FitVerdict::Fits
                                    && in_range(Some(&dep.version), &r.version)
                                    && index.revoked(&p.id, Some(&r.file.sha256)).is_none()
                            })
                            .max_by(|a, b| compare_versions(&a.version, &b.version))?;
                        Some(ProviderCandidate {
                            project_id: p.id.clone(),
                            slug: p.slug.clone(),
                            name: p.name.clone(),
                            repository_name: p.repository.name.clone(),
                            version: best.version.clone(),
                        })
                    })
                    .collect();
                candidates.sort_by(|a, b| a.project_id.cmp(&b.project_id));
                match candidates.as_slice() {
                    [] => {
                        return Err(InstallError::NotForVersion(trf(
                            "{mod} needs {dep} {range}, which has no release fitting Sunrise {build}.",
                            &[
                                ("mod", &to_project.name),
                                ("dep", &dep.mod_id),
                                ("range", &dep.version.to_string()),
                                ("build", &base_build),
                            ],
                        )));
                    }
                    [only] => dep_queue.push_back((
                        only.project_id.clone(),
                        true,
                        to_project.name.clone(),
                        range,
                    )),
                    _ => {
                        let need = ProviderChoiceNeeded {
                            mod_id: dep.mod_id.clone(),
                            required_by: to_project.name.clone(),
                            candidates,
                        };
                        if let Some(picked) =
                            hooks.install.choose_provider.and_then(|ask| ask(&need))
                        {
                            if need.candidates.iter().any(|c| c.project_id == picked) {
                                dep_queue.push_back((picked, true, to_project.name.clone(), range));
                                continue;
                            }
                        }
                        return Err(InstallError::AmbiguousProvider(Box::new(need)));
                    }
                }
            }
        }
    }
    let dep_steps = if dep_queue.is_empty() {
        Vec::new()
    } else {
        let chosen_pins = chosen(target.home());
        resolve_v2_queue(
            &ResolveContext {
                index: &index,
                build: &base_build,
                installed: &before,
                chosen_pins: &chosen_pins,
                options: &options,
                unfit_hashes: &unfit_hashes,
                choose_provider: hooks.install.choose_provider,
            },
            dep_queue,
        )?
        .steps
    };

    let pin_dir = options.pin_dir.clone().unwrap_or_else(backend::data_dir);
    let next = edit.candidate()?;
    let mut doc = Document::read(next.path())?;
    let mut entries: BTreeMap<String, installed::Entry> = BTreeMap::new();
    let mut pending_pins: Vec<PendingPin> = Vec::new();
    let mut native = false;

    for planned_step in std::iter::once(&step).chain(dep_steps.iter()) {
        cancelled.check().map_err(|_| InstallError::Cancelled)?;
        let (entry, pin) = stage_and_verify_v2_release(
            &index,
            next.path(),
            planned_step,
            &pin_dir,
            &options.accepted_publisher_changes,
            hooks.install.confirm_publisher_change,
            transfer,
        )
        .map_err(|error| {
            if matches!(error, InstallError::Download(_)) {
                stopped_or(Some(cancelled), error)
            } else {
                error
            }
        })?;
        doc.replace(&entry)?;
        mod_list::set_enabled(next.path(), &planned_step.mod_id, true)?;
        native |= planned_step.kind == V2Kind::NativeMod;
        entries.insert(planned_step.mod_id.clone(), entry);
        pending_pins.push(pin);
    }
    doc.write(next.path())?;

    check_staged_v2_candidate(next.path(), &index, |id| entries.contains_key(id))?;

    // A reviewed registry release asks nothing before its build: its attestation and the
    // signed index were verified while it was fetched and staged above.
    if native {
        if let Some(error) = native_mods::known_host_block(Some(&base_build)) {
            return Err(InstallError::Host(error));
        }
        cancelled.check().map_err(|_| InstallError::Cancelled)?;
    }

    (hooks.install.install.progress)(InstallProgress::Preparing);
    edit.prepare_first_cancellable(
        &super::preparation_line(hooks.install.install.progress),
        Some(cancelled),
    )
    .map_err(|error| stopped_or(Some(cancelled), InstallError::Unproved(error)))?;
    let proof = prove_edit(
        &edit,
        next.path(),
        target,
        cancelled,
        &hooks.install.install,
    )
    .map_err(|error| {
        if cancelled.is_stopped() {
            InstallError::Cancelled
        } else {
            InstallError::Unproved(error.to_string())
        }
    })?;

    let (suspect, _) = mutations::update_worlds(next.path(), std::iter::once(&current.package))?;
    let worlds = mutations::world_evidence(target.home(), &suspect)?;
    let preview = ProviderSwitchPreview {
        mod_id: mod_id.to_owned(),
        from_project: current.ownership.project_id().map(str::to_owned),
        from_name: current.package.manifest.name.clone(),
        to_project: to_project.id.clone(),
        to_name: to_project.name.clone(),
        to_version: release.version.clone(),
        worlds: worlds.clone(),
    };
    if !(hooks.approve_switch)(&preview) {
        return Err(InstallError::Cancelled);
    }

    (hooks.install.install.progress)(InstallProgress::Publishing);
    cancelled.check().map_err(|_| InstallError::Cancelled)?;
    commit_pending_pins(&pin_dir, &pending_pins).map_err(InstallError::Unproved)?;
    let authorize = || mutations::bind_worlds(target.home(), &suspect, &worlds);
    edit.publish_install(proof, cancelled, &BTreeSet::new(), authorize)?;

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

pub(super) fn find_affected_dependents(
    index: &Index,
    installed: &BTreeMap<String, InstalledV2Mod>,
    slot_mod_id: &str,
    to_project_id: &str,
    to_version: &str,
) -> Vec<V2AffectedDependent> {
    let parsed_new = parse_version(to_version).ok();
    let mut affected = Vec::new();
    for (id, item) in installed {
        if id == slot_mod_id || !item.enabled {
            continue;
        }
        // Check v2 release-level pinned-project dependency.
        if let Some(dep_pid) = item.ownership.project_id() {
            if let Some(project) = index.project(dep_pid) {
                if let Some(rel) = project
                    .releases
                    .iter()
                    .find(|r| r.file.sha256.eq_ignore_ascii_case(&item.digest))
                {
                    for dep in &rel.dependencies {
                        if let Some(req) = dep.project.as_deref() {
                            if dep.required && dep.mod_id == slot_mod_id && req != to_project_id {
                                affected.push(V2AffectedDependent {
                                    mod_id: id.clone(),
                                    name: item.package.manifest.name.clone(),
                                    reason: trf(
                                        "requires project {project} specifically",
                                        &[("project", req)],
                                    ),
                                });
                            }
                        }
                    }
                }
            }
        }
        // Check manifest semver constraint against the new provider's version.
        if let Some(dep) = item.package.manifest.dependencies.get(slot_mod_id) {
            let details = dep.details();
            if !details.optional {
                let fits = parsed_new
                    .as_ref()
                    .is_none_or(|ver| details.version.matches(ver));
                if !fits {
                    let req = details.version.to_string();
                    affected.push(V2AffectedDependent {
                        mod_id: id.clone(),
                        name: item.package.manifest.name.clone(),
                        reason: trf(
                            "needs {mod} {req}, not {version}",
                            &[("mod", slot_mod_id), ("req", &req), ("version", to_version)],
                        ),
                    });
                }
            }
        }
    }
    affected
}

/// Validate that all enabled packages in staged candidate `home` have no manifest conflicts and no
/// active v2 revocations.
pub(crate) fn check_staged_v2_candidate(
    home: &Path,
    index: &Index,
    staged: impl Fn(&str) -> bool,
) -> Result<(), InstallError> {
    let current = inventory_v2(home, index)?;
    let enabled_manifests: BTreeMap<&str, &sunrise_launcher_smod::manifest::Manifest> = current
        .iter()
        .filter(|(_, m)| m.enabled)
        .map(|(k, m)| (k.as_str(), &m.package.manifest))
        .collect();
    for item in current.values() {
        if !item.enabled || item.package.directory {
            continue;
        }
        if let Some(revoked) = revoked_for_package(index, &item.ownership, &item.digest) {
            return Err(InstallError::Refused(trf(
                "{mod} was taken down: {reason}",
                &[
                    ("mod", &item.package.manifest.name),
                    ("reason", &revoked.reason),
                ],
            )));
        }
        if let Some(missing) = missing_required(&item.package.manifest, &enabled_manifests) {
            return Err(InstallError::Refused(trf(
                "{mod} needs {dep} to be installed and enabled.",
                &[("mod", &item.package.manifest.name), ("dep", missing)],
            )));
        }
    }
    if let Some(conflict) = manifest_conflict(home, Side::Both, staged)? {
        return Err(InstallError::Refused(conflict.reason));
    }
    Ok(())
}
