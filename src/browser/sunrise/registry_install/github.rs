//! Replacing a GitHub release uses an exact admission, review and same-base publication.
use super::*;
use crate::{github_mods, worlds};
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
    if instance.kind == InstanceKind::Pack {
        return Err(InstallError::Refused(
            tr("This pack controls which mods it installs.").into(),
        ));
    }
    let replacement = packed.release_origin(found).ok_or_else(|| {
        InstallError::Refused(tr("GitHub Update needs a checked release asset.").into())
    })?;
    let id = &replacement.id;
    let file = format!("{}/{id}.smod", native_mods::INSTALLED);
    (hooks.progress)(InstallProgress::Checking);
    let mut profile =
        Profile::open_for(&instance.home, None, Some(identity)).map_err(String::from)?;
    let predecessor = {
        let _home = crate::browser::lock_home(&instance.home).map_err(|e| at(&instance.home, e))?;
        installed_release(&instance.home, id, &file)?
    };
    if !predecessor.owner.eq_ignore_ascii_case(&replacement.owner)
        || !predecessor.repo.eq_ignore_ascii_case(&replacement.repo)
    {
        return Err(InstallError::Refused(
            tr("Choose a release from the same GitHub repository as the installed mod.").into(),
        ));
    }
    if predecessor.sha256.eq_ignore_ascii_case(&replacement.sha256) {
        return Err(InstallError::Refused(
            tr("That GitHub archive is already installed. Nothing was changed.").into(),
        ));
    }

    let edit = profile.begin_edit(
        choose,
        |f| (hooks.progress)(InstallProgress::Downloading(f)),
        &|_| (hooks.progress)(InstallProgress::Preparing),
    )?;
    let old = edit.candidate()?;
    let before = inventory(old.path(), None)?;
    let previous = before.get(id).ok_or_else(|| changed(id))?;
    if previous.github_origin(old.path()).as_ref() != Some(&predecessor) {
        return Err(changed(id).into());
    }
    let held = edit.held(id);
    if held.is_some_and(|held| {
        !matches!(held.source(), managed_play::HeldSource::Github { .. })
            || !previous.matches_hold(old.path(), edit.base(), held)
    }) {
        return Err(held_change_refusal(id));
    }
    let next = edit.candidate()?;
    let home = next.path();
    packed.copy_verified(&home.join(&file))?;
    let sidecar = github_mods::origin_path(home, id);
    let bytes = serde_json::to_vec_pretty(replacement).map_err(|e| at(&sidecar, e))?;
    fs::write(&sidecar, bytes).map_err(|e| at(&sidecar, e))?;
    let was_enabled = mutations::enabled(old.path())?.contains(id);
    let enabled = held.map_or(was_enabled, managed_play::Held::enabled_before);
    if enabled != was_enabled {
        mod_list::set_enabled(home, id, enabled)?;
    }
    let catalogue = super::super::fetched_index();
    let check_graph = || match manifest_conflict(home, catalogue.as_deref(), None)? {
        Some(conflict) => Err(InstallError::Refused(conflict.reason)),
        None => Ok(()),
    };
    check_graph()?;
    let reviewed_inputs = managed_play::inputs(home, None)?;
    let suspect = worlds::unproved_mod_effects(
        old.path(),
        edit.active_basis(),
        home,
        edit.active_basis(),
        [id.as_str()],
    )?;
    let title = &previous.package.manifest.name;
    let impact = ChangeImpact::read(
        &instance.home,
        mutations::Change::GithubUpdate {
            name: format!("{title} ({id})"),
            file: file.clone(),
            releases: [(&predecessor).into(), replacement.into()],
        },
        &suspect,
    )?;
    // Approval is required even when the current world roster proves there is no save risk.
    // Ask before proving the replacement: native builds may run the author's build scripts.
    (hooks.progress)(InstallProgress::Reviewing);
    if !approve(&impact) {
        return Err(InstallError::Cancelled);
    }
    let unchanged_candidate = || {
        if managed_play::inputs(home, None)? != reviewed_inputs {
            return Err(tr(
                "The staged GitHub update changed after review. Nothing was changed; try again.",
            )
            .to_owned());
        }
        Ok(())
    };
    unchanged_candidate()?;
    let cancellation = native_mods::Cancellation::default();
    if backend::NativeGrant::read()?.is_none()
        && !(hooks.native)(&NativeQuestion {
            names: vec![title.clone()],
            reviewed: false,
            retry: false,
        })?
    {
        return Err(InstallError::Cancelled);
    }
    cancellation
        .admit_native_work()
        .map_err(|e| InstallError::Unproved(e.into()))?;
    let prove = || {
        prove_edit(&edit, home, instance, &cancellation, &hooks)
            .map_err(|e| InstallError::Unproved(e.into()))
    };
    if held.is_some() && !enabled {
        // Prove the off replacement would build, but do not publish a different switch or image.
        let list = home.join(sunrise_launcher_smod::mod_list::FILE);
        let selection = match fs::read(&list) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == ErrorKind::NotFound => None,
            Err(e) => return Err(at(&list, e).into()),
        };
        unchanged_candidate()?;
        mod_list::set_enabled(home, id, true)?;
        check_graph()?;
        prove()?;
        match selection {
            Some(bytes) => fs::write(&list, bytes),
            None => fs::remove_file(&list),
        }
        .map_err(|e| at(&list, e))?;
    }
    unchanged_candidate()?;
    let proof = prove()?;
    let retired = held.into_iter().map(|_| id.clone()).collect();
    let reviewed = impact.into_evidence();
    (hooks.progress)(InstallProgress::Publishing);
    edit.publish(proof, &cancellation, &retired, || {
        unchanged_candidate()?;
        // The journal checks both input digests and holds the home lock. Also recheck claims
        // and aliases here: they can change without changing any bytes in the copied mod tree.
        if installed_release(&instance.home, id, &file).map_err(|e| e.to_string())? != predecessor {
            return Err(changed(id));
        }
        mutations::bind_worlds(&instance.home, &suspect, &reviewed)
    })?;
    Ok(replacement.clone())
}

fn installed_release(
    home: &Path,
    id: &str,
    file: &str,
) -> Result<github_mods::Origin, InstallError> {
    let refusal = || {
        InstallError::Refused(trf(
            "Couldn't verify an installed GitHub release asset for {mod}. Nothing was changed.",
            &[("mod", id)],
        ))
    };
    let claims = browser_state::Installed::read(home).map_err(|e| e.details(trf))?;
    if claims.single(id).map_err(|e| e.render(trf))?.is_some() {
        return Err(refusal());
    }
    let sidecar = format!("{}/{id}.origin.json", native_mods::INSTALLED);
    for relative in [file, sidecar.as_str()] {
        claims
            .check_other_claims(home, id, relative)
            .map_err(|e| e.render(trf))?;
    }
    let packages = inventory(home, None)?;
    let origin = packages
        .get(id)
        .and_then(|p| p.github_origin(home))
        .filter(github_mods::Origin::is_release_asset)
        .ok_or_else(refusal)?;
    // The candidate copies lose hard-link identity, and case-insensitive filesystems can
    // open a differently spelled sidecar through the canonical path. Require exact leaves.
    for relative in [file, sidecar.as_str()] {
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
        assert!(installed_release(home, "zoom", &file).is_err());
        let mut origin = github_mods::Origin {
            id: "zoom".into(),
            version: "1.2.0".into(),
            owner: "owner".into(),
            repo: "zoom".into(),
            tag: Some("v1.2.0".into()),
            commit,
            asset: Some("zoom-1.2.0.smod".into()),
            sha256: "0".repeat(64),
        };
        let write_origin = |origin: &github_mods::Origin| {
            fs::write(&receipt, serde_json::to_vec(origin).unwrap()).unwrap();
        };
        write_origin(&origin);
        assert!(installed_release(home, "zoom", &file).is_err());
        origin.sha256 = index::sha256(&bytes);
        write_origin(&origin);
        assert_eq!(installed_release(home, "zoom", &file).unwrap(), origin);
        let alias = home.join("alias");
        for path in [&package, &receipt] {
            fs::hard_link(path, &alias).unwrap();
            assert!(installed_release(home, "zoom", &file).is_err());
            fs::remove_file(&alias).unwrap();
        }
        let mut doc = Document::read(home).unwrap();
        doc.rows
            .entries
            .push(serde_json::json!({"project": "other", "file": file}));
        doc.write(home).unwrap();
        assert!(installed_release(home, "zoom", &file).is_err());
    }
}
