//! Installation publication excludes concurrent capability snapshots.
use super::{
    architecture_for_asset, copy_dir_except, data_dir, download_cancellable, game_version_at,
    in_use, keep_release_file, read_json, resources_for_game, unpack, version_dir, version_storage,
    versions_dir, GameArchitecture, InstalledGameArchitecture, Release, CLIENT, COMPLETE,
    GAME_BUILD_ARCHITECTURE, RESOURCES_MANIFEST,
};
use crate::{managed_play::Artifact, profile_txn::durable};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use sunrise_launcher_smod::{
    locks::{LockMode, Release as LockRelease},
    runtime_leases::Registry,
};

mod bound;
pub(super) use bound::sdk_target;
pub use bound::NightlyReceiptWarning;
pub(crate) use bound::{prove_release_files, BoundVariant, GameDownload, GameProof};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InstallPhase {
    Checking,
    Downloading(f32),
    DownloadingUnknownLength,
    Unpacking,
    Finishing,
    Preparing,
}

fn logged<T>(what: String, run: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let step = crate::launcher_log::step(what);
    let result = run();
    match &result {
        Ok(_) => step.done(""),
        Err(error) => step.failed(error),
    }
    result
}

const STAGE_PREFIX: &str = ".build-stage-";
const REPAIR: &str = ".repair-";

pub(crate) fn private_stage(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| name.starts_with(STAGE_PREFIX) || name.starts_with(REPAIR))
}

pub(crate) fn valid_archive_digest(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn completion_marker(tree: &Path) -> std::io::Result<bool> {
    use std::io::ErrorKind;
    match fs::symlink_metadata(tree.join(COMPLETE)) {
        Ok(meta) if meta.is_file() && !crate::native_cache::linked(&meta) => Ok(true),
        Ok(_) => Err(ErrorKind::InvalidData.into()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// The old tree's move to `previous` is the recovery marker. The replacement commits
/// when the complete, flushed tree becomes visible under its version name.
fn recover_previous(repair: &Path, target: &Path) -> std::io::Result<()> {
    use std::io::ErrorKind;
    let plain = |meta: &fs::Metadata| meta.is_dir() && !crate::native_cache::linked(meta);
    match fs::symlink_metadata(repair) {
        Ok(meta) if plain(&meta) => {}
        Ok(_) => return Err(ErrorKind::InvalidData.into()),
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    }
    let visible = match fs::symlink_metadata(target) {
        Ok(meta) if plain(&meta) => Some(completion_marker(target)?),
        Ok(_) => return Err(ErrorKind::InvalidData.into()),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    if visible == Some(true) {
        return fs::remove_dir_all(repair);
    }
    let previous = repair.join("previous");
    match fs::symlink_metadata(&previous) {
        Ok(meta) if plain(&meta) => {
            completion_marker(&previous)?;
        }
        Ok(_) => return Err(ErrorKind::InvalidData.into()),
        // The launcher can stop after creating the marker, before moving a plain
        // incomplete predecessor. An empty marker with its visible tree is harmless.
        Err(e) if e.kind() == ErrorKind::NotFound && visible.is_some() => {
            return fs::remove_dir(repair);
        }
        Err(e) => return Err(e),
    }
    // A repair from an earlier launcher can leave a visible uncommitted tree.
    let discarded = repair.join("incomplete");
    if visible.is_some() {
        durable::move_new_directory(target, &discarded)?;
    }
    // Clear the discarded tree before restoring the predecessor. A stop after the
    // restore then leaves just the empty marker, even when the predecessor is incomplete.
    match fs::symlink_metadata(&discarded) {
        Ok(meta) if plain(&meta) => fs::remove_dir_all(&discarded)?,
        Ok(_) => return Err(ErrorKind::InvalidData.into()),
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    durable::move_new_directory(&previous, target)?;
    fs::remove_dir(repair)
}

pub(super) fn recover_replacement(tag: &str) -> Result<(), String> {
    if Path::new(tag).file_name() != Some(std::ffi::OsStr::new(tag)) {
        return Err(existing_build(tag));
    }
    let repair = versions_dir().join(format!("{REPAIR}{tag}"));
    match fs::symlink_metadata(&repair) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
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
        if let Err(error) = recover_replacement(tag) {
            crate::elog!(
                "Could not recover game installation {tag}: {}",
                crate::launcher_log::safe_error(&error.to_string())
            );
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum InstallIntent {
    /// First Play fetches its saved binding automatically. A different installation stays untouched.
    Exact,
    /// An explicit release install may repair this exact tag, never a saved binding.
    Repair,
}

impl InstallIntent {
    fn download(self) -> version_storage::Download {
        match self {
            Self::Exact => version_storage::Download::Automatic,
            Self::Repair => version_storage::Download::Requested,
        }
    }
}

/// Success means the image is usable, even when the separate rolling Nightly receipt failed.
#[derive(Debug)]
pub enum InstallOutcome {
    Installed(Option<Arc<NightlyReceiptWarning>>),
    AlreadyInstalled(Option<Arc<NightlyReceiptWarning>>),
    ReceiptOnly(Option<Arc<NightlyReceiptWarning>>),
}

impl InstallOutcome {
    pub fn receipt_warning(&self) -> Option<&Arc<NightlyReceiptWarning>> {
        match self {
            Self::Installed(warning)
            | Self::AlreadyInstalled(warning)
            | Self::ReceiptOnly(warning) => warning.as_ref(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct ReceiptFailure {
    error: std::io::Error,
    previous: Option<String>,
}

fn save_nightly_receipt(root: &Path, tag: &str) -> Option<ReceiptFailure> {
    crate::nightly::record_current(root, tag)
        .err()
        .map(|error| {
            crate::nightly::remember_install_receipt_failure(root, tag);
            ReceiptFailure {
                error,
                previous: crate::nightly::current_receipt(root),
            }
        })
}

/// Cancellation and progress callbacks shared across archive and SDK downloads.
pub(crate) struct Transfer<'a, F> {
    pub(crate) stop: Option<&'a crate::native_mods::Cancellation>,
    pub(crate) progress: F,
}

#[cfg(test)]
pub(crate) fn install_as(
    release: &Release,
    tag: &str,
    sha256: &str,
    requested_architecture: Option<GameArchitecture>,
    release_facts: Option<(&[u8], &str)>,
    intent: InstallIntent,
    progress: impl Fn(InstallPhase),
) -> Result<Option<ReceiptFailure>, String> {
    install_as_cancellable(
        release,
        tag,
        sha256,
        requested_architecture,
        release_facts,
        intent,
        Transfer {
            stop: None,
            progress,
        },
    )
}

pub(crate) fn install_as_cancellable(
    release: &Release,
    tag: &str,
    sha256: &str,
    requested_architecture: Option<GameArchitecture>,
    release_facts: Option<(&[u8], &str)>,
    intent: InstallIntent,
    transfer: Transfer<'_, impl Fn(InstallPhase)>,
) -> Result<Option<ReceiptFailure>, String> {
    let Transfer { stop, progress } = transfer;
    let (url, size) = release.asset.as_ref().ok_or_else(|| {
        crate::i18n::trf("{tag} has no build for this platform.", &[("tag", tag)])
    })?;
    install_image(
        release,
        tag,
        intent,
        stop,
        |target| Artifact::matches_archive(target, tag, sha256),
        |stage| {
            let archive = stage.join("download");
            let reported_unknown_length = std::cell::Cell::new(false);
            logged(format!("Downloading {tag} ({size} bytes)"), || {
                download_cancellable(url, *size, sha256, &archive, stop, &|done, total| {
                    if total > 0 {
                        progress(InstallPhase::Downloading(
                            (done as f32 / total as f32).min(1.0),
                        ));
                    } else if !reported_unknown_length.replace(true) {
                        progress(InstallPhase::DownloadingUnknownLength);
                    }
                })
            })?;
            let unpacked = stage.join("unpacked");
            progress(InstallPhase::Unpacking);
            logged(format!("Unpacking {tag}"), || {
                unpack(&archive, url, &unpacked, stop)
            })?;
            // The archives usually hold one top folder; install its contents.
            let entries: Vec<_> = fs::read_dir(&unpacked)
                .and_then(|entries| entries.collect::<std::io::Result<_>>())
                .map_err(|e| e.to_string())?;
            let top = match entries.as_slice() {
                [only] if only.path().is_dir() => only.path(),
                _ => unpacked,
            };
            let architecture = requested_architecture
                .zip(architecture_for_asset(url))
                .map(|(requested, binary)| InstalledGameArchitecture { requested, binary });
            progress(InstallPhase::Finishing);
            finish_image(&top, architecture, release_facts)?;
            Artifact::record_archive(&top, tag, sha256)?;
            Ok(top)
        },
    )
}

fn finish_image(
    top: &Path,
    architecture: Option<InstalledGameArchitecture>,
    release_facts: Option<(&[u8], &str)>,
) -> Result<(), String> {
    if !fs::symlink_metadata(top)
        .is_ok_and(|meta| meta.is_dir() && !crate::native_cache::linked(&meta))
        || !fs::symlink_metadata(top.join(CLIENT))
            .is_ok_and(|meta| meta.is_file() && !crate::native_cache::linked(&meta))
    {
        return Err(crate::i18n::trf(
            "The download has no {client}",
            &[("client", CLIENT)],
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(top.join(CLIENT), fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    if let Some(architecture) = architecture {
        let mut marker =
            fs::File::create_new(top.join(GAME_BUILD_ARCHITECTURE)).map_err(|e| e.to_string())?;
        serde_json::to_writer(&mut marker, &architecture).map_err(|e| e.to_string())?;
    }
    if let Some((bytes, sha256)) = release_facts {
        keep_release_file(top, bytes, sha256)?;
    }
    Ok(())
}

fn install_image(
    release: &Release,
    tag: &str,
    intent: InstallIntent,
    stop: Option<&crate::native_mods::Cancellation>,
    matches: impl Fn(&Path) -> bool,
    stage_tree: impl FnOnce(&Path) -> Result<PathBuf, String>,
) -> Result<Option<ReceiptFailure>, String> {
    if let Some(stop) = stop {
        stop.check()?;
    }
    let root = versions_dir();
    let target = version_dir(tag);
    if intent == InstallIntent::Exact {
        match fs::symlink_metadata(&target) {
            Ok(_) if matches(&target) => return Ok(None),
            Ok(_) => return Err(existing_build(tag)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let stage = tempfile::Builder::new()
        .prefix(STAGE_PREFIX)
        .tempdir_in(&root)
        .map_err(|e| e.to_string())?;
    let top = stage_tree(stage.path())?;
    if let Some(stop) = stop {
        stop.check()?;
    }
    // Its own step, so a failure here is logged by name with its error (#607).
    let publishing = crate::launcher_log::step(format!("Putting {tag} in place"));
    let published = (|| -> Result<(Option<ReceiptFailure>, &'static str), String> {
        intent.download().mark(&top)?;
        crate::nightly::record_install_time(&top, release).map_err(|e| e.to_string())?;
        // Transfer and unpack privately. Only the visibility boundary fences readers.
        let registry = Registry::with_release(&data_dir(), LockRelease::Close)
            .map_err(|e| crate::native_mods::cache_error(e.into()))?;
        let mode = if intent == InstallIntent::Repair {
            LockMode::Exclusive
        } else {
            LockMode::Shared
        };
        let _publication = registry
            .publication(mode)
            .map_err(|e| crate::native_mods::cache_error(e.into()))?;
        if let Some(stop) = stop {
            stop.check()?;
        }
        let replaced = if intent == InstallIntent::Repair {
            // A pending First Play can reserve this name before its tree exists.
            // Only the exact managed path may publish its frozen game image in that case.
            crate::managed_play::require_unreferenced_build(tag)
                .map_err(crate::native_mods::cache_error)?;
            replace_installed(
                &registry,
                tag,
                &top,
                &target,
                &root.join(format!("{REPAIR}{tag}")),
            )?
        } else {
            false
        };
        if !replaced {
            match crate::run_leases::publish_build(&top, &target) {
                Ok(()) => {}
                Err(crate::native_cache::Error::Io(ref e))
                    if e.kind() == std::io::ErrorKind::AlreadyExists =>
                {
                    if !matches(&target) {
                        return Err(existing_build(tag));
                    }
                    if intent == InstallIntent::Repair {
                        intent.download().mark(&target)?;
                    }
                    return Ok((None, "already in place"));
                }
                Err(e) => return Err(crate::native_mods::cache_error(e)),
            }
        }
        // The tree is already committed. Failure of this independent receipt must not
        // undo it, stop someone from playing it, or turn success into an install error.
        let warning =
            if release.nightly.as_deref() == Some(sunrise_launcher_release_verify::NIGHTLY) {
                save_nightly_receipt(&root, tag)
            } else {
                None
            };
        Ok((warning, ""))
    })();
    match &published {
        Ok((_, detail)) => publishing.done(detail),
        Err(error) => publishing.failed(error),
    }
    let (warning, _) = published?;
    Ok(warning)
}

/// Reconcile an explicit release install with a plain existing version, even if it never
/// completed. The caller holds the exclusive publication fence and ruled out references.
fn replace_installed(
    registry: &std::sync::Arc<Registry>,
    tag: &str,
    top: &Path,
    target: &Path,
    repair: &Path,
) -> Result<bool, String> {
    use crate::native_mods::cache_error;
    recover_previous(repair, target).map_err(|e| e.to_string())?;
    match fs::symlink_metadata(target) {
        Ok(meta) if meta.is_dir() && !crate::native_cache::linked(&meta) => {}
        Ok(_) => return Err(existing_build(tag)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.to_string()),
    }
    let tree = cap_std::fs::Dir::open_ambient_dir(target, cap_std::ambient_authority())
        .map_err(|e| e.to_string())?;
    let old = registry
        .lock(&tree, LockMode::Exclusive)
        .map_err(|e| cache_error(e.into()))?;
    if in_use(tag) {
        return Err(cache_error(crate::storage::Error::Busy));
    }
    // Same-tag repair preserves local preparation regardless of what can be selected for
    // Play. Keep its Minecraft folder name even if the replacement carries a different pin.
    if completion_marker(target).map_err(|e| e.to_string())? {
        let game = game_version_at(target);
        let from = resources_for_game(target, &game);
        let to = resources_for_game(top, &game);
        if !to.exists() && read_json(&from.join(RESOURCES_MANIFEST)).is_some() {
            copy_dir_except(&from, &to, Some(&from.join(crate::preparer::DATA_FILE)))
                .map_err(|e| e.to_string())?;
        }
    }
    drop(tree);
    fs::create_dir(repair).map_err(|e| e.to_string())?;
    let publish = durable::move_new_directory(target, &repair.join("previous"))
        .map_err(|e| e.to_string())
        .and_then(|()| crate::run_leases::publish_build(top, target).map_err(cache_error));
    if let Err(error) = publish {
        return match recover_previous(repair, target) {
            Ok(()) => Err(error),
            Err(restore) => Err(format!("{error}; {restore}")),
        };
    }
    if recover_previous(repair, target).is_ok() {
        let _ = old.retire();
    }
    Ok(true)
}

fn existing_build(tag: &str) -> String {
    crate::i18n::trf(
        "A different installation already occupies {build}. It was not replaced.",
        &[("build", tag)],
    )
}
