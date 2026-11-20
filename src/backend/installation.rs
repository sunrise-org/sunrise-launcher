//! Installation publication excludes concurrent capability snapshots.
use super::*;
use sunrise_launcher_smod::{
    locks::{LockMode, Release as LockRelease},
    runtime_leases::Registry,
};

const REPAIR: &str = ".repair-";

/// The old tree's move to `previous` is the recovery marker. The old tree stays intact until
/// the target's own completion marker commits its replacement.
fn recover_previous(repair: &Path, target: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(repair) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Err(std::io::ErrorKind::InvalidData.into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    }
    let previous = repair.join("previous");
    if !target.join(COMPLETE).is_file() {
        match fs::symlink_metadata(&previous) {
            Ok(meta) if meta.is_dir() && previous.join(COMPLETE).is_file() => {
                if target.try_exists()? {
                    fs::rename(target, repair.join("incomplete"))?;
                }
                fs::rename(&previous, target)?;
            }
            Ok(_) => return Err(std::io::ErrorKind::InvalidData.into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    fs::remove_dir_all(repair)
}

pub(super) fn recover_replacement(tag: &str) -> Result<(), String> {
    let repair = versions_dir().join(format!("{REPAIR}{tag}"));
    if !repair.try_exists().map_err(|e| e.to_string())? {
        return Ok(());
    }
    let registry = Registry::with_release(&data_dir(), LockRelease::Close)
        .map_err(|e| crate::native_mods::cache_error(e.into()))?;
    let _publication = registry
        .publication(LockMode::Exclusive)
        .map_err(|e| crate::native_mods::cache_error(e.into()))?;
    recover_previous(&repair, &version_dir(tag)).map_err(|e| e.to_string())
}

/// Restore complete installations before either the CLI or window resolves profile pins.
pub fn recover_game_installations() {
    let Ok(entries) = fs::read_dir(versions_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(tag) = name.to_str().and_then(|n| n.strip_prefix(REPAIR)) else {
            continue;
        };
        if Path::new(tag).file_name() != Some(std::ffi::OsStr::new(tag)) {
            continue;
        }
        if let Err(error) = recover_replacement(tag) {
            eprintln!("Could not recover game installation {tag}: {error}");
        }
    }
}

pub(crate) fn install_as(
    release: &Release,
    tag: &str,
    sha256: &str,
    requested_architecture: Option<GameArchitecture>,
    progress: impl Fn(f32),
) -> Result<(), String> {
    let staging = crate::run_leases::installation().map_err(crate::native_mods::cache_error)?;
    let (url, size) = release
        .asset
        .clone()
        .ok_or_else(|| format!("{tag} has no build for this platform."))?;
    let root = versions_dir();
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let work = tempfile::Builder::new()
        .prefix(&format!(".{tag}.partial-"))
        .tempdir_in(&root)
        .map_err(|e| e.to_string())?;
    let partial = work.path();
    let archive = partial.join("download");
    download(&url, size, sha256, &archive, &progress)?;
    let unpacked = partial.join("unpacked");
    unpack(&archive, &url, &unpacked)?;
    progress(0.97);
    // The archives hold one top folder (sunrise-linux-x86_64/...); the version folder is its contents.
    let entries: Vec<_> = fs::read_dir(&unpacked)
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();
    let top = match entries.as_slice() {
        [only] if only.path().is_dir() => only.path(),
        _ => unpacked.clone(),
    };
    if !top.join(CLIENT).exists() {
        return Err(format!("The download has no {CLIENT}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(top.join(CLIENT), fs::Permissions::from_mode(0o755));
    }
    if let (Some(requested), Some(binary)) = (requested_architecture, architecture_for_asset(&url))
    {
        fs::write(
            top.join(GAME_BUILD_ARCHITECTURE),
            serde_json::to_vec(&InstalledGameArchitecture { requested, binary })
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }
    crate::nightly::record_install_time(&top, release).map_err(|e| e.to_string())?;
    // Use an exclusive publication only for the directory switch, not for the download. That
    // makes two installs of the same tag see either the old tree or the new one, never the gap.
    drop(staging);
    let registry = Registry::with_release(&data_dir(), LockRelease::Close)
        .map_err(|e| crate::native_mods::cache_error(e.into()))?;
    let publication = registry
        .publication(LockMode::Exclusive)
        .map_err(|e| crate::native_mods::cache_error(e.into()))?;
    let target = version_dir(tag);
    let repair = root.join(format!("{REPAIR}{tag}"));
    recover_previous(&repair, &target).map_err(|e| e.to_string())?;
    // Archive contents are fully staged, but only the launcher can publish completion.
    match fs::remove_file(top.join(COMPLETE)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    let mut previous = partial.join("incomplete");
    let old_claim = if target.try_exists().map_err(|e| e.to_string())? {
        let tree = cap_std::fs::Dir::open_ambient_dir(&target, cap_std::ambient_authority())
            .map_err(|e| e.to_string())?;
        let claim = registry
            .lock(&tree, LockMode::Exclusive)
            .map_err(|e| crate::native_mods::cache_error(e.into()))?;
        if in_use(tag) {
            return Err(crate::native_mods::cache_error(crate::storage::Error::Busy));
        }
        // Keep preparation and the automatic-install classification of the same build.
        let from = resources_at(&target);
        let to = resources_at(&top);
        if !to.exists()
            && read_json(&from.join(RESOURCES_MANIFEST))
                .is_some_and(|manifest| resources_compatible(&target, &top, &manifest))
        {
            copy_dir_except(&from, &to, Some(&from.join(crate::preparer::DATA_FILE)))
                .map_err(|e| e.to_string())?;
        }
        if target.join(AUTO_INSTALLED).is_file() {
            fs::write(top.join(AUTO_INSTALLED), "").map_err(|e| e.to_string())?;
        }
        drop(tree);
        if target.join(COMPLETE).is_file() {
            fs::create_dir(&repair).map_err(|e| e.to_string())?;
            previous = repair.join("previous");
        }
        // An unmanaged Windows game that started meanwhile can refuse this rename. Never
        // remove files from the old installation one by one in that case.
        fs::rename(&target, &previous).map_err(|e| e.to_string())?;
        Some(claim)
    } else {
        None
    };
    let publish = (|| {
        fs::rename(&top, &target).map_err(|e| e.to_string())?;
        crate::run_leases::publish(&target).map_err(crate::native_mods::cache_error)?;
        if release.nightly.as_deref() == Some(sunrise_launcher_release_verify::NIGHTLY) {
            crate::nightly::record_current(&root, tag).map_err(|e| e.to_string())?;
        }
        fs::write(target.join(COMPLETE), "").map_err(|e| e.to_string())
    })();
    if let Err(error) = publish {
        // Keep the marker if rollback itself is interrupted or refused. Both startup and the
        // next install retry the same recovery without needing the vanished process's stage.
        return match recover_previous(&repair, &target) {
            Ok(()) => Err(error),
            Err(restore) => Err(format!("{error}; {restore}")),
        };
    }
    if let Some(claim) = old_claim {
        let removed = if previous.starts_with(&repair) {
            recover_previous(&repair, &target)
        } else {
            fs::remove_dir_all(&previous)
        };
        if removed.is_ok() {
            let _ = claim.retire();
        }
    }
    drop(publication);
    reuse_prepared_for(tag);
    progress(1.0);
    Ok(())
}
