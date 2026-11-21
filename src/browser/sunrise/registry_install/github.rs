//! GitHub Add and checked Update use the same admitted candidate and publication as registry installs.
use super::*;
use crate::github_mods;
use cap_fs_ext::MetadataExt;
use std::io::ErrorKind;

/// Explicitly replace an existing GitHub release archive. The reviewed path and digest
/// are the installed predecessor, not metadata from the update check. No fresh Add here.
pub fn update_github(
    instance: &Instance,
    identity: Identity,
    found: &github_mods::Resolved,
    packed: &github_mods::Packed,
    choose: impl FnOnce() -> Result<FirstBuild, String>,
    hooks: InstallHooks<'_>,
    approve: &dyn Fn(&ChangeImpact) -> bool,
) -> Result<github_mods::Origin, InstallError> {
    let replacement = packed.release_origin(found).ok_or_else(|| {
        InstallError::Refused(tr("GitHub Update needs a checked release asset.").into())
    })?;
    edit_github(
        instance,
        identity,
        &[Entry {
            package: packed,
            origin: replacement,
            prior: Prior::Release,
        }],
        choose,
        hooks,
        Some(approve),
        Check::Current,
    )?;
    Ok(replacement.clone())
}

/// Install all selected packages together. Fresh Add never silently upgrades or claims an
/// existing file, even when it came from the same repository; use checked Update for that.
pub fn add_github(
    instance: &Instance,
    identity: Identity,
    found: &github_mods::Resolved,
    selected: &[github_mods::Packed],
    base: Option<&str>,
    choose: impl FnOnce() -> Result<FirstBuild, String>,
    hooks: InstallHooks<'_>,
) -> Result<Vec<github_mods::Origin>, InstallError> {
    let packages = selected
        .iter()
        .map(|package| {
            let origin = package.checked_origin(found).ok_or_else(|| {
                InstallError::Refused(tr("Couldn't verify the selected GitHub package.").into())
            })?;
            Ok(Entry {
                package,
                origin,
                prior: Prior::Fresh,
            })
        })
        .collect::<Result<Vec<_>, InstallError>>()?;
    edit_github(
        instance,
        identity,
        &packages,
        choose,
        hooks,
        None,
        base.map_or(Check::Current, Check::Base),
    )?;
    Ok(packages
        .into_iter()
        .map(|entry| entry.origin.clone())
        .collect())
}

/// A single checked package and the precise reason it may occupy the live path.
pub(super) struct Entry<'a> {
    pub(super) package: &'a github_mods::Packed,
    pub(super) origin: &'a github_mods::Origin,
    pub(super) prior: Prior<'a>,
}

pub(super) enum Prior<'a> {
    Fresh,
    Release,
    Source(&'a github_mods::Origin),
}

pub(super) enum Check<'a> {
    Current,
    Base(&'a str),
    Source(SourceBatch<'a>),
}

pub(super) struct SourceBatch<'a> {
    pub(super) before: [u8; 32],
    pub(super) base: &'a str,
    pub(super) active: Option<&'a managed_play::Active>,
    pub(super) cancellation: &'a native_mods::Cancellation,
    pub(super) authorize: &'a dyn Fn() -> Result<bool, String>,
}

pub(super) fn edit_github(
    instance: &Instance,
    identity: Identity,
    packages: &[Entry<'_>],
    choose: impl FnOnce() -> Result<FirstBuild, String>,
    hooks: InstallHooks<'_>,
    approve: Option<&dyn Fn(&ChangeImpact) -> bool>,
    check: Check<'_>,
) -> Result<(), InstallError> {
    let source = match &check {
        Check::Source(source) => Some(source),
        _ => None,
    };
    if instance.kind == InstanceKind::Pack {
        return Err(InstallError::Refused(
            tr("This pack controls which mods it installs.").into(),
        ));
    }
    if packages.is_empty() {
        return Err(InstallError::Cancelled);
    }
    let legacy_cancellation = native_mods::Cancellation::default();
    let cancellation = source.map_or(&legacy_cancellation, |source| source.cancellation);
    cancellation.check()?;
    (hooks.progress)(InstallProgress::Checking);
    let mut profile =
        Profile::open_for(&instance.home, None, Some(identity)).map_err(String::from)?;
    let previous = {
        let _home = profile.lock_inputs()?;
        if let Some(source) = source {
            if managed_play::inputs(&instance.home, None)? != source.before
                || profile.active()?.as_ref() != source.active
            {
                return Err(InstallError::Source(super::source::changed_source_target()));
            }
            // A previously accepted first Play is not ours to retarget or resume.
            if source.active.is_none() && profile.has_accepted_bootstrap()? {
                return Err(InstallError::Source(super::source::changed_source_target()));
            }
        }
        predecessors(&instance.home, packages)?
    };
    let edit = profile.begin_edit(
        choose,
        |f| (hooks.progress)(InstallProgress::Downloading(f)),
        &|_| (hooks.progress)(InstallProgress::Preparing),
    )?;
    if matches!(check, Check::Base(base) if edit.base() != base) {
        return Err(InstallError::Refused(
            super::source::changed_source_target().to_string(),
        ));
    }
    let old = edit.candidate()?;
    let next = edit.candidate()?;
    if let Some(source) = source {
        if edit.base() != source.base
            || edit.current() != source.active
            || managed_play::inputs(old.path(), None)? != source.before
        {
            return Err(InstallError::Source(super::source::changed_source_target()));
        }
    }
    let home = next.path();
    let staged = stage_entries(home, packages, &previous, edit.current(), edit.base())?;
    let catalogue = super::super::fetched_index();
    let check_graph = || match manifest_conflict(home, catalogue.as_deref(), None, |_| false)? {
        Some(conflict) => Err(InstallError::Refused(conflict.reason)),
        None => Ok(()),
    };
    if source.is_some() {
        super::source::check_source_set(home, &staged.off, edit.base(), cancellation)
            .map_err(InstallError::Source)?;
    } else {
        check_graph()?;
    }
    let reviewed_inputs = managed_play::inputs(home, None)?;
    let review = if previous.is_empty() {
        None
    } else {
        let approve = approve.ok_or_else(|| {
            InstallError::Refused(tr("Review the update before building.").into())
        })?;
        let (impact, suspect) = if source.is_some() {
            source_review(&instance.home, old.path(), home, &previous, packages)?
        } else {
            let before = inventory(old.path(), None)?;
            let id = &packages[0].origin.id;
            let prior = before.get(id).ok_or_else(|| changed(id))?;
            let (suspect, unchecked) = mutations::update_worlds(home, [&prior.package])?;
            let impact = ChangeImpact::read(
                &instance.home,
                mutations::Change::GithubUpdate {
                    target: mutations::ReviewedMod {
                        name: prior.package.manifest.name.clone(),
                        id: id.clone(),
                    },
                    file: archive_path(id),
                    releases: [(&previous[id]).into(), packages[0].origin.into()],
                },
                &suspect,
            )?
            .with_unchecked_update(unchecked);
            (impact, suspect)
        };
        (hooks.progress)(InstallProgress::Reviewing);
        if !approve(&impact) {
            return Err(InstallError::Cancelled);
        }
        Some((suspect, impact.into_evidence()))
    };
    let unchanged_candidate = || {
        if managed_play::inputs(home, None)? != reviewed_inputs {
            return Err(
                tr("The staged GitHub packages changed. Nothing was changed; try again.")
                    .to_owned(),
            );
        }
        Ok(())
    };
    cancellation.check()?;
    if let Some(source) = source {
        if !(source.authorize)()? {
            return Err(InstallError::Cancelled);
        }
    } else if backend::NativeGrant::read()?.is_none()
        && !(hooks.native)(&NativeQuestion {
            names: packages.iter().map(|p| p.package.name.clone()).collect(),
            reviewed: false,
            retry: false,
        })?
    {
        return Err(InstallError::Cancelled);
    }
    cancellation
        .admit_native_work()
        .map_err(|e| InstallError::Unproved(e.into()))?;
    let tools = std::cell::Cell::new(false);
    let progress = |progress: InstallProgress| {
        tools.set(matches!(
            progress,
            InstallProgress::Tools { .. } | InstallProgress::Rust(_)
        ));
        (hooks.progress)(progress);
    };
    let prove = || {
        let proof_hooks = InstallHooks {
            progress: &progress,
            ..hooks
        };
        prove_edit(&edit, home, instance, cancellation, &proof_hooks).map_err(|e| match &e {
            native_mods::Failed::Mods { left, .. }
                if left.iter().any(|item| {
                    matches!(item.why, native_mods::Why::Unfit(_))
                        && packages.iter().any(|entry| {
                            item.id == entry.origin.id && item.version == entry.origin.version
                        })
                }) =>
            {
                InstallError::NotForVersion(e.into())
            }
            native_mods::Failed::Mods {
                compiling: true,
                error,
                log,
                ..
            } if source.is_some() => InstallError::Source(SourceError::Build {
                detail: error.clone(),
                log: log.clone(),
            }),
            native_mods::Failed::Tools(_) if source.is_some() => {
                InstallError::Source(SourceError::Tools(e.into()))
            }
            _ if source.is_some() && tools.get() => {
                InstallError::Source(SourceError::Tools(e.into()))
            }
            native_mods::Failed::Tools(_) => InstallError::Download(e.into()),
            _ if tools.get() => InstallError::Download(e.into()),
            native_mods::Failed::Mods {
                compiling: true, ..
            } => InstallError::GithubBuild(e.into()),
            _ => InstallError::Unproved(e.into()),
        })
    };
    if !staged.off.is_empty() {
        // Prove the off replacement would build, but don't publish its trial activation or image.
        unchanged_candidate()?;
        with_trial_selection(home, &staged.off, || {
            check_graph()?;
            prove().map(|_| ())
        })?;
    }
    unchanged_candidate()?;
    let proof = prove()?;
    cancellation.check()?;
    edit.publish_install_at(
        proof,
        cancellation,
        &staged.retired,
        || {
            unchanged_candidate()?;
            if predecessors(&instance.home, packages).map_err(|e| e.to_string())? != previous {
                return Err(tr(
                    "An installed GitHub archive changed. Nothing was changed; try again.",
                )
                .into());
            }
            if let Some((suspect, evidence)) = &review {
                mutations::bind_worlds(&instance.home, suspect, evidence).map(Some)
            } else {
                Ok(None)
            }
        },
        || (hooks.progress)(InstallProgress::Publishing),
    )?;
    Ok(())
}

pub(super) fn source_review(
    live: &Path,
    original: &Path,
    staged: &Path,
    previous: &BTreeMap<String, github_mods::Origin>,
    entries: &[Entry<'_>],
) -> Result<(ChangeImpact, crate::worlds::SuspectMods), String> {
    let before = inventory(original, None)?;
    let replaced = previous
        .keys()
        .map(|id| {
            before
                .get(id)
                .map(|old| &old.package)
                .ok_or_else(|| changed(id))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (suspect, unchecked) = mutations::update_worlds(staged, replaced)?;
    let change = mutations::Change::Update(
        entries
            .iter()
            .map(|entry| mutations::ReviewedUpdate {
                name: entry.package.name.clone(),
                installed: previous
                    .get(&entry.origin.id)
                    .map(|old| old.version.clone()),
                replacement: entry.origin.version.clone(),
            })
            .collect(),
    );
    Ok((
        ChangeImpact::read(live, change, &suspect)?.with_unchecked_update(unchecked),
        suspect,
    ))
}

pub(super) fn with_trial_selection<T, E: From<String>>(
    home: &Path,
    ids: &[String],
    run: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    if ids.is_empty() {
        return run();
    }
    let list = home.join(sunrise_launcher_smod::mod_list::FILE);
    let selection = match fs::read(&list) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => return Err(at(&list, e).into()),
    };
    let ids = ids.iter().map(String::as_str).collect::<Vec<_>>();
    mod_list::set_all_enabled(home, &ids, true)?;
    let result = run();
    match selection {
        Some(bytes) => fs::write(&list, bytes),
        None => fs::remove_file(&list),
    }
    .map_err(|e| at(&list, e))?;
    result
}

fn archive_path(id: &str) -> String {
    format!("{}/{id}.smod", native_mods::INSTALLED)
}

pub(super) fn predecessors(
    home: &Path,
    entries: &[Entry<'_>],
) -> Result<BTreeMap<String, github_mods::Origin>, InstallError> {
    let mut ids = BTreeSet::new();
    let mut previous = BTreeMap::new();
    for entry in entries {
        let id = &entry.origin.id;
        if !ids.insert(id) {
            return Err(changed(id).into());
        }
        match entry.prior {
            Prior::Fresh => check_fresh(home, id)?,
            Prior::Release => {
                let old = installed_origin(home, id, github_mods::Origin::is_release_asset)?;
                if !old.owner.eq_ignore_ascii_case(&entry.origin.owner)
                    || !old.repo.eq_ignore_ascii_case(&entry.origin.repo)
                {
                    return Err(InstallError::Refused(tr("Choose a release from the same GitHub repository as the installed mod.").into()));
                }
                if old.sha256.eq_ignore_ascii_case(&entry.origin.sha256) {
                    return Err(InstallError::Refused(
                        tr("That GitHub archive is already installed. Nothing was changed.").into(),
                    ));
                }
                previous.insert(id.clone(), old);
            }
            Prior::Source(expected) => {
                let old = installed_origin(home, id, |origin| {
                    origin == expected && origin.source.is_some()
                })?;
                previous.insert(id.clone(), old);
            }
        }
    }
    Ok(previous)
}

pub(super) struct Staged {
    pub(super) off: Vec<String>,
    retired: BTreeSet<String>,
}

pub(super) fn stage_entries(
    home: &Path,
    entries: &[Entry<'_>],
    previous: &BTreeMap<String, github_mods::Origin>,
    active: Option<&managed_play::Active>,
    base: &str,
) -> Result<Staged, InstallError> {
    let before = inventory(home, None)?;
    let mut result = Staged {
        off: Vec::new(),
        retired: BTreeSet::new(),
    };
    fs::create_dir_all(home.join(native_mods::INSTALLED)).map_err(|e| at(home, e))?;
    for entry in entries {
        let id = &entry.origin.id;
        let prior = previous.get(id);
        let hold = active.and_then(|active| active.holds().find(|held| held.id() == id));
        if let Some(expected) = prior {
            let old = before.get(id).ok_or_else(|| changed(id))?;
            if old.github_origin(home).as_ref() != Some(expected) {
                return Err(changed(id).into());
            }
            if let Some(held) = hold {
                if !matches!(held.source(), managed_play::HeldSource::Github { .. })
                    || !old.matches_hold(home, base, held)
                {
                    return Err(held_change_refusal(id));
                }
                result.retired.insert(id.clone());
            }
        } else {
            check_fresh(home, id)?;
            if hold.is_some() {
                return Err(held_change_refusal(id));
            }
        }
        let was_enabled = mod_list::is_enabled(home, id);
        let enabled =
            prior.is_none() || hold.map_or(was_enabled, managed_play::Held::enabled_before);
        if !enabled && (hold.is_some() || entry.origin.source.is_some()) {
            result.off.push(id.clone());
        }
        entry.package.copy_verified(&home.join(archive_path(id)))?;
        let sidecar = github_mods::origin_path(home, id);
        let bytes = serde_json::to_vec_pretty(entry.origin).map_err(|e| at(&sidecar, e))?;
        fs::write(&sidecar, bytes).map_err(|e| at(&sidecar, e))?;
        if enabled != was_enabled {
            mod_list::set_enabled(home, id, enabled)?;
        }
    }
    Ok(result)
}

fn check_fresh(home: &Path, id: &str) -> Result<(), InstallError> {
    let claims = browser_state::Installed::read(home).map_err(|e| e.details(trf))?;
    let present = inventory(home, None)?;
    let collision = || {
        InstallError::Refused(trf(
        "{mod} is already installed. Use Update for a GitHub release or remove the existing mod first.",
        &[("mod", id)],
    ))
    };
    if present.contains_key(id) || claims.single(id).map_err(|e| e.render(trf))?.is_some() {
        return Err(collision());
    }
    for suffix in ["smod", "origin.json"] {
        let relative = format!("{}/{id}.{suffix}", native_mods::INSTALLED);
        claims
            .check_other_claims(home, id, &relative)
            .map_err(|e| e.render(trf))?;
        let path = home.join(&relative);
        match fs::symlink_metadata(&path) {
            Ok(_) => return Err(collision()),
            Err(e) if e.kind() == ErrorKind::NotFound => (),
            Err(e) => return Err(at(&path, e).into()),
        }
    }
    Ok(())
}

fn installed_origin(
    home: &Path,
    id: &str,
    matches: impl FnOnce(&github_mods::Origin) -> bool,
) -> Result<github_mods::Origin, InstallError> {
    let file = archive_path(id);
    let refusal = || {
        InstallError::Refused(trf(
            "Couldn't verify the installed GitHub package and origin for {mod}. Nothing was changed.",
            &[("mod", id)],
        ))
    };
    let claims = browser_state::Installed::read(home).map_err(|e| e.details(trf))?;
    if claims.single(id).map_err(|e| e.render(trf))?.is_some() {
        return Err(refusal());
    }
    let sidecar = format!("{}/{id}.origin.json", native_mods::INSTALLED);
    for relative in [file.as_str(), sidecar.as_str()] {
        claims
            .check_other_claims(home, id, relative)
            .map_err(|e| e.render(trf))?;
    }
    let packages = inventory(home, None)?;
    let origin = packages
        .get(id)
        .and_then(|p| p.github_origin(home))
        .filter(matches)
        .ok_or_else(refusal)?;
    // The candidate copies lose hard-link identity, and case-insensitive filesystems can
    // open a differently spelled sidecar through the canonical path. Require exact leaves.
    for relative in [file.as_str(), sidecar.as_str()] {
        let path = home.join(relative);
        let leaf = path.file_name().expect("an installed leaf");
        let parent = path.parent().expect("an installed directory");
        let mut exact = false;
        for entry in fs::read_dir(parent).map_err(|e| at(parent, e))? {
            let entry = entry.map_err(|e| at(parent, e))?;
            let name = entry.file_name();
            if name.eq_ignore_ascii_case(leaf) {
                if name != leaf {
                    return Err(refusal());
                }
                exact = true;
            }
        }
        let open = fs::File::open(&path).map_err(|e| at(&path, e))?;
        let meta = cap_std::fs::File::from_std(open)
            .metadata()
            .map_err(|e| at(&path, e))?;
        if !exact || meta.nlink() != 1 {
            return Err(refusal());
        }
    }
    Ok(origin)
}

fn changed(id: &str) -> String {
    trf(
        "The installed GitHub archive or origin for {mod} changed. Nothing was changed; try again.",
        &[("mod", id)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_sidecar_only_owns_its_exact_unaliased_archive() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let file = format!("{}/zoom.smod", native_mods::INSTALLED);
        let package = home.join(&file);
        let receipt = github_mods::origin_path(home, "zoom");
        let commit = "a".repeat(40);
        let bytes = sunrise_launcher_smod::Package::write_files(
            &format!("id = 'zoom'\nname = 'Zoom'\nversion = '1.2.0'\napi = '0.2'\ncapabilities = ['registries']\n[native]\ngame = '26.3'\nsunrise = '{commit}'\nmixins = '{commit}'\n"),
            &[("Cargo.toml", b"[package]\nname = 'zoom'\nversion = '1.2.0'\n"), ("src/lib.rs", b"pub fn mod_hooks() {}")],
        ).unwrap();
        fs::create_dir_all(package.parent().unwrap()).unwrap();
        fs::write(&package, &bytes).unwrap();
        assert!(installed_origin(home, "zoom", github_mods::Origin::is_release_asset).is_err());
        let mut origin = github_mods::Origin {
            id: "zoom".into(),
            version: "1.2.0".into(),
            owner: "owner".into(),
            repo: "zoom".into(),
            tag: Some("v1.2.0".into()),
            commit,
            asset: Some("zoom-1.2.0.smod".into()),
            source: None,
            sha256: "0".repeat(64),
        };
        let write_origin = |origin: &github_mods::Origin| {
            fs::write(&receipt, serde_json::to_vec(origin).unwrap()).unwrap();
        };
        write_origin(&origin);
        assert!(installed_origin(home, "zoom", github_mods::Origin::is_release_asset).is_err());
        origin.sha256 = index::sha256(&bytes);
        write_origin(&origin);
        assert_eq!(
            installed_origin(home, "zoom", github_mods::Origin::is_release_asset).unwrap(),
            origin
        );
        let alias = home.join("alias");
        for path in [&package, &receipt] {
            fs::hard_link(path, &alias).unwrap();
            assert!(installed_origin(home, "zoom", github_mods::Origin::is_release_asset).is_err());
            fs::remove_file(&alias).unwrap();
        }
        let mut doc = Document::read(home).unwrap();
        doc.rows
            .entries
            .push(serde_json::json!({"project": "other", "file": file}));
        doc.write(home).unwrap();
        assert!(installed_origin(home, "zoom", github_mods::Origin::is_release_asset).is_err());
    }
}
