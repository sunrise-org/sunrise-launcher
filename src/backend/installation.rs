//! Installation publication excludes concurrent capability snapshots.
use super::*;

mod bound;
pub(crate) use bound::{BoundVariant, GameDownload, GameProof};

use crate::managed_play::Artifact;

const STAGE_PREFIX: &str = ".build-stage-";

pub(crate) fn private_stage(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| name.starts_with(STAGE_PREFIX))
}

pub(crate) fn valid_archive_digest(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn install_as(
    release: &Release,
    tag: &str,
    sha256: &str,
    requested_architecture: Option<GameArchitecture>,
    release_facts: Option<(&[u8], &str)>,
    progress: impl Fn(f32),
) -> Result<(), String> {
    let (url, size) = release
        .asset
        .clone()
        .ok_or_else(|| format!("{tag} has no build for this platform."))?;
    let root = versions_dir();
    let target = version_dir(tag);
    let matches_archive = || Artifact::matches_archive(&target, tag, sha256);
    match fs::symlink_metadata(&target) {
        Ok(_) if matches_archive() => {
            progress(1.0);
            return Ok(());
        }
        Ok(_) => return Err(existing_build(tag)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let stage = tempfile::Builder::new()
        .prefix(STAGE_PREFIX)
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
    Artifact::record_archive(&top, tag, sha256)?;
    crate::nightly::record_install_time(&top, release).map_err(|e| e.to_string())?;
    // Transfer and unpack privately. Cache readers wait only at the visibility boundary.
    let _publication =
        crate::run_leases::installation().map_err(crate::native_mods::cache_error)?;
    match crate::run_leases::publish_build(&top, &target) {
        Ok(()) => {}
        Err(crate::native_cache::Error::Io(ref e))
            if e.kind() == std::io::ErrorKind::AlreadyExists =>
        {
            if !matches_archive() {
                return Err(existing_build(tag));
            }
            progress(1.0);
            return Ok(());
        }
        Err(e) => return Err(crate::native_mods::cache_error(e)),
    }
    // First Play prepares through the profile's staged handoff, not the cache-only
    // import hook (whose detached preparer could otherwise outlive that admission).
    progress(1.0);
    Ok(())
}

fn existing_build(tag: &str) -> String {
    crate::i18n::trf(
        "A different installation already occupies {build}. It was not replaced.",
        &[("build", tag)],
    )
}
