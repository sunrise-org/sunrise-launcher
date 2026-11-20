//! Installation publication excludes concurrent capability snapshots.
use super::*;

mod bound;
pub(crate) use bound::{GameDownload, GameProof};

pub(crate) const ARCHIVE_ID: &str = ".sunrise-archive-sha256";

pub(crate) fn install_as(
    release: &Release,
    tag: &str,
    sha256: &str,
    requested_architecture: Option<GameArchitecture>,
    release_facts: Option<(&[u8], &str)>,
    provenance: version_storage::Download,
    progress: impl Fn(f32),
) -> Result<(), String> {
    let _publication =
        crate::run_leases::installation().map_err(crate::native_mods::cache_error)?;
    let (url, size) = release
        .asset
        .clone()
        .ok_or_else(|| format!("{tag} has no build for this platform."))?;
    let root = versions_dir();
    let target = version_dir(tag);
    let matches_archive = |meta: &fs::Metadata| {
        meta.is_dir()
            && !crate::native_cache::linked(meta)
            && target.join(COMPLETE).is_file()
            && fs::read_to_string(target.join(ARCHIVE_ID)).is_ok_and(|id| id == sha256)
    };
    match fs::symlink_metadata(&target) {
        Ok(meta) if matches_archive(&meta) => {
            if matches!(provenance, version_storage::Download::Requested) {
                provenance.mark(&target)?;
            }
            progress(1.0);
            return Ok(());
        }
        Ok(_) => return Err(existing_build(tag)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let stage = tempfile::Builder::new()
        .prefix(".build-stage-")
        .tempdir_in(root)
        .map_err(|e| e.to_string())?;
    let archive = stage.path().join("download");
    download(&url, size, sha256, &archive, &progress)?;
    let unpacked = stage.path().join("unpacked");
    unpack(&archive, &url, &unpacked)?;
    progress(0.97);
    // The archives hold one top folder (sunrise-linux-x86_64/...); the version folder is its contents.
    let entries: Vec<_> = fs::read_dir(&unpacked)
        .and_then(|entries| entries.collect::<std::io::Result<_>>())
        .map_err(|e| e.to_string())?;
    let top = match entries.as_slice() {
        [only] if only.path().is_dir() => only.path(),
        _ => unpacked.clone(),
    };
    if !fs::symlink_metadata(&top)
        .is_ok_and(|meta| meta.is_dir() && !crate::native_cache::linked(&meta))
        || !fs::symlink_metadata(top.join(CLIENT))
            .is_ok_and(|meta| meta.is_file() && !crate::native_cache::linked(&meta))
    {
        return Err(format!("The download has no {CLIENT}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(top.join(CLIENT), fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    if let (Some(requested), Some(binary)) = (requested_architecture, architecture_for_asset(&url))
    {
        let mut marker =
            fs::File::create_new(top.join(GAME_BUILD_ARCHITECTURE)).map_err(|e| e.to_string())?;
        serde_json::to_writer(
            &mut marker,
            &InstalledGameArchitecture { requested, binary },
        )
        .map_err(|e| e.to_string())?;
    }
    if let Some((bytes, sha256)) = release_facts {
        keep_release_file(&top, bytes, sha256)?;
    }
    fs::File::create_new(top.join(ARCHIVE_ID))
        .and_then(|mut file| file.write_all(sha256.as_bytes()))
        .map_err(|e| e.to_string())?;
    provenance.mark(&top)?;
    crate::nightly::record_install_time(&top, release).map_err(|e| e.to_string())?;
    match crate::run_leases::publish_build(&top, &target) {
        Ok(()) => {}
        Err(crate::native_cache::Error::Io(ref e))
            if e.kind() == std::io::ErrorKind::AlreadyExists =>
        {
            if !fs::symlink_metadata(&target)
                .as_ref()
                .is_ok_and(matches_archive)
            {
                return Err(existing_build(tag));
            }
            if matches!(provenance, version_storage::Download::Requested) {
                provenance.mark(&target)?;
            }
            progress(1.0);
            return Ok(());
        }
        Err(e) => return Err(crate::native_mods::cache_error(e)),
    }
    reuse_prepared_for(tag);
    progress(1.0);
    Ok(())
}

fn existing_build(tag: &str) -> String {
    crate::i18n::trf(
        "A different installation already occupies {build}. It was not replaced.",
        &[("build", tag)],
    )
}
