//! Managed Play reads the exact prepared tree saved with its generation. The ordinary
//! preparation cache remains available for a later Prepare, without changing an active game.
use super::{artifact, digest_file, optional_bytes, plain_file, pool, require_dir, unsettled};
use crate::{
    backend, launcher_log, native_mods::Cancellation, preparer, process::leases::ProfileHandoff,
    profile_txn::durable,
};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
};

const ROOT: &str = "launcher-prepared";
const EVIDENCE: &str = "data-file.json";
const CURRENT: &str = "current.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Resources {
    primary: [u8; 32],
    accelerator: Option<[u8; 32]>,
}

/// Prepared resources selected for Play. `proven_by_seal` is true only when this call
/// just sealed a brand-new snapshot into place (the seal itself already hashed the source
/// tree under the preparation lock), so Play skips a redundant verify pass over that tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PreparedResources {
    pub(super) resources: Resources,
    pub(super) proven_by_seal: bool,
}

impl PreparedResources {
    pub(super) fn reused(resources: Resources) -> Self {
        Self {
            resources,
            proven_by_seal: false,
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Full prepared-file verifications requested on this thread. Tests take it with `Cell::take`.
    pub(crate) static FULL_VERIFICATIONS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

impl Resources {
    pub(super) fn capture(
        base: &str,
        stage: &crate::profile_txn::Staging<'_>,
    ) -> Result<Self, String> {
        Self::capture_cancellable(base, stage, None)
    }

    /// [`Self::capture`], whose preparation and sealing stop at the next step once `stop` is
    /// set: a cancelled switch leaves no preparer running.
    pub(super) fn capture_cancellable(
        base: &str,
        stage: &crate::profile_txn::Staging<'_>,
        stop: Option<&crate::native_mods::Cancellation>,
    ) -> Result<Self, String> {
        let root = backend::version_dir(base);
        let preparation = backend::prepare_lock(base)?;
        match Self::current(&root)? {
            Some(resources) => Ok(resources),
            None => Self::prepare_cancellable(base, &preparation, Some(stage.handoff()), stop),
        }
    }

    fn current(root: &Path) -> Result<Option<Self>, String> {
        let Some(resources) = optional_bytes(&receipt(root)?)?
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
        else {
            return Ok(None);
        };
        match backend::check_play_output(root, &resources.path_at(root)) {
            Ok(()) => Ok(Some(resources)),
            Err(backend::Unprepared::Incomplete | backend::Unprepared::ServerOnly) => Ok(None),
            Err(issue) => Err(issue.message(|| unsettled!())),
        }
    }

    /// Select current resources for first Play or advance an old profile on the same base.
    /// The version lock and the admitted profile are inherited by any preparer children.
    pub(super) fn for_play(
        base: &str,
        profile: ProfileHandoff<'_>,
        log: &dyn Fn(String),
    ) -> Result<PreparedResources, String> {
        Self::for_play_with(base, profile, log, log)
    }

    /// [`Self::for_play`], routing raw preparer child output to `diagnostics` only so arbitrary
    /// stdout/stderr never reaches `launcher.log`.
    pub(super) fn for_play_with(
        base: &str,
        profile: ProfileHandoff<'_>,
        log: &dyn Fn(String),
        diagnostics: &dyn Fn(String),
    ) -> Result<PreparedResources, String> {
        let preparation = backend::prepare_lock(base)?;
        // A prepared profile uses its receipt here, without sealing or hashing again.
        // The admitted Play handoff checks the saved snapshot digest.
        if let Some(resources) = Self::current(&backend::version_dir(base))? {
            return Ok(PreparedResources::reused(resources));
        }
        let handoff = preparation.handoff(Some(profile));
        if !backend::check_prepared(base) {
            backend::reuse_prepared(&backend::versions_dir(), base);
            if !backend::check_prepared(base) {
                let (command, minecraft) = backend::preparer_staged(base, None, log, &handoff)?;
                backend::sounds_staged(base, &minecraft, diagnostics, &handoff);
                backend::set_prepare_step(backend::PrepareStep::Prepare);
                launcher_log::play_detail(crate::i18n::tr("Preparing the game"));
                let step = launcher_log::step("Preparing game files");
                match preparer::run_staged(command, &handoff, diagnostics) {
                    Ok(()) => step.done(""),
                    Err(error) => {
                        step.failed(&error);
                        return Err(error);
                    }
                }
            }
        }
        Self::seal_prepared(base, &preparation, &handoff, None, None, false)
    }

    /// Finish the shared preparation under its existing lock, before a profile needs it.
    #[cfg(test)]
    pub(crate) fn prepare(
        base: &str,
        preparation: &preparer::Preparation,
        profile: Option<ProfileHandoff<'_>>,
    ) -> Result<Self, String> {
        Self::seal_prepared(
            base,
            preparation,
            &preparation.handoff(profile),
            None,
            None,
            true,
        )
        .map(|prepared| prepared.resources)
    }

    /// [`Self::prepare`] when the caller already probed `missing-extras` unlocked (startup).
    pub(crate) fn prepare_with_missing(
        base: &str,
        preparation: &preparer::Preparation,
        profile: Option<ProfileHandoff<'_>>,
        missing: Option<preparer::Extras>,
    ) -> Result<Self, String> {
        Self::seal_prepared(
            base,
            preparation,
            &preparation.handoff(profile),
            None,
            Some(missing),
            true,
        )
        .map(|prepared| prepared.resources)
    }

    /// [`Self::prepare`], stopping at the next step once `stop` is set.
    pub(crate) fn prepare_cancellable(
        base: &str,
        preparation: &preparer::Preparation,
        profile: Option<ProfileHandoff<'_>>,
        stop: Option<&crate::native_mods::Cancellation>,
    ) -> Result<Self, String> {
        let handoff = match stop {
            Some(stop) => preparation.handoff_cancellable(profile, stop),
            None => preparation.handoff(profile),
        };
        Self::seal_prepared(base, preparation, &handoff, stop, None, true)
            .map(|prepared| prepared.resources)
    }

    fn seal_prepared(
        base: &str,
        preparation: &preparer::Preparation,
        handoff: &preparer::Handoff<'_>,
        stop: Option<&crate::native_mods::Cancellation>,
        probed_missing: Option<Option<preparer::Extras>>,
        allow_repair: bool,
    ) -> Result<PreparedResources, String> {
        let check = || stop.map_or(Ok(()), crate::native_mods::Cancellation::check);
        check()?;
        backend::need_base_prepared(base, Some(preparation))?;
        match probed_missing {
            None => backend::complete_assets_staged(base, handoff),
            Some(Some(lacking)) if !lacking.is_empty() => {
                backend::complete_assets_lacking_staged(base, handoff, lacking);
            }
            Some(_) => {}
        }
        check()?;
        let root = backend::version_dir(base);
        Self::seal_at(
            &root,
            &backend::resources(base),
            stop,
            allow_repair,
            |resources| {
                check()?;
                let Some(binary) = preparer::binary(&root) else {
                    return Ok(None);
                };
                launcher_log::play_detail(crate::i18n::tr("Preparing the game"));
                pack_data_cache(&binary, &root, resources, Some(handoff), stop)
            },
        )
    }

    pub(crate) fn invalidate(root: &Path) -> io::Result<()> {
        durable::retire(&receipt(root).map_err(io::Error::other)?)
    }

    pub(super) fn path(&self, base: &str) -> PathBuf {
        self.path_at(&backend::version_dir(base))
    }

    fn path_at(&self, root: &Path) -> PathBuf {
        parent(root).join(key(self.primary)).join("resources")
    }

    pub(super) fn check_play_output(&self, base: &str) -> Result<(), backend::Unprepared> {
        let root = backend::version_dir(base);
        backend::check_play_output(&root, &self.path_at(&root))
    }

    pub(super) fn verify(&self, base: &str) -> Result<(), String> {
        #[cfg(test)]
        FULL_VERIFICATIONS.with(|count| count.set(count.get() + 1));
        let root = backend::version_dir(base);
        let path = self.path(base);
        for dir in [
            &root,
            &root.join(".local"),
            &parent(&root),
            path.parent().expect("snapshot parent"),
        ] {
            require_dir(dir).map_err(|_| setup_error())?;
        }
        launcher_log::play_detail(crate::i18n::tr("Checking the game files"));
        let meter = launcher_log::PlayMeter::new(|done, total| {
            crate::i18n::trf(
                "Checking the game files: {done} of {total}",
                &[("done", &done.to_string()), ("total", &total.to_string())],
            )
        });
        let on_done = |done, total| meter.tick(done, total);
        let actual = logged("Verifying game files", || {
            Ok(
                match artifact::tree_with_progress(
                    &path,
                    true,
                    pool::workers(),
                    None,
                    Some(&on_done),
                )
                .map_err(|_| setup_error())?
                {
                    Some((digest, files)) => (Some(digest), files),
                    None => (None, 0),
                },
            )
        })?;
        if actual != Some(self.primary) {
            return Err(setup_error());
        }
        Ok(())
    }

    pub(super) fn allow_accelerator(&self, base: &str) -> bool {
        self.accelerator.is_some_and(|digest| {
            digest_file(&self.path(base).join(preparer::DATA_FILE))
                .is_ok_and(|found| found == Some(digest))
        })
    }

    // Flush a private primary tree before publishing it or replacing a damaged snapshot.
    // Build the optional cache against the final path: the game caches
    // the canonical resource root inside it. A kill after exposure but before its evidence
    // write leaves a usable primary tree and an untrusted accelerator, never the reverse.
    #[cfg(test)]
    fn seal(
        root: &Path,
        source: &Path,
        stop: Option<&Cancellation>,
        accelerator: impl FnOnce(&Path) -> Result<Option<[u8; 32]>, String>,
    ) -> Result<Self, String> {
        Self::seal_at(root, source, stop, true, accelerator).map(|prepared| prepared.resources)
    }

    fn seal_at(
        root: &Path,
        source: &Path,
        stop: Option<&Cancellation>,
        allow_repair: bool,
        accelerator: impl FnOnce(&Path) -> Result<Option<[u8; 32]>, String>,
    ) -> Result<PreparedResources, String> {
        Self::seal_with(
            root,
            source,
            pool::workers(),
            stop,
            allow_repair,
            accelerator,
        )
    }

    // Each pass (hash, copy, flush) runs on `workers` threads and joins them before the next.
    fn seal_with(
        root: &Path,
        source: &Path,
        workers: usize,
        stop: Option<&Cancellation>,
        allow_repair: bool,
        accelerator: impl FnOnce(&Path) -> Result<Option<[u8; 32]>, String>,
    ) -> Result<PreparedResources, String> {
        let relative = source.strip_prefix(root).map_err(|_| setup_error())?;
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            return Err(setup_error());
        }
        for ancestor in source.ancestors().take_while(|path| *path != root) {
            require_dir(ancestor)?;
        }
        require_dir(root)?;
        launcher_log::play_detail(crate::i18n::tr("Sealing the game files"));
        // One monotonic count across hashing, copying and flushing so the count never restarts.
        let seal_meter = launcher_log::PlayMeter::new(|done, total| {
            crate::i18n::trf(
                "Sealing the game files: {done} of {total}",
                &[("done", &done.to_string()), ("total", &total.to_string())],
            )
        });
        let on_hash = |done, total: usize| seal_meter.tick(done, total.saturating_mul(3));
        let (primary, hashed_files) = logged("Sealing game files: hashing", || {
            Ok(
                match artifact::tree_with_progress(source, true, workers, stop, Some(&on_hash))? {
                    Some((digest, files)) => ((Some(digest), files), files),
                    None => ((None, 0), 0),
                },
            )
        })?;
        let Some(primary) = primary else {
            return Err(setup_error());
        };
        let parent = parent(root);
        fs::create_dir_all(&parent).map_err(io_failure)?;
        require_dir(&parent)?;
        let snapshot = parent.join(key(primary));
        let resources = snapshot.join("resources");
        let evidence = snapshot.join(EVIDENCE);
        let exists = snapshot.exists();
        if exists {
            require_dir(&snapshot)?;
        }
        if let Some(stop) = stop {
            stop.check()?;
        }
        // Only explicit preparation may verify-and-repair an existing snapshot here.
        // On Play's receipt-miss path, leave an already-existing snapshot untouched and
        // unhashed for the single counted cutoff, and do not publish a receipt for it yet.
        if exists && !allow_repair {
            let accelerator =
                optional_bytes(&evidence)?.and_then(|bytes| serde_json::from_slice(&bytes).ok());
            return Ok(PreparedResources::reused(Self {
                primary,
                accelerator,
            }));
        }
        // Only preparation may repair a snapshot. Check reuse here, not on Play
        // for a profile with a prepared snapshot.
        let reusable = exists
            && artifact::tree_with_workers(&resources, true, workers, stop)
                .is_ok_and(|tree| tree.is_some_and(|(digest, _)| digest == primary));
        if let Some(stop) = stop {
            stop.check()?;
        }
        let accelerator = if reusable {
            optional_bytes(&evidence)?.and_then(|bytes| serde_json::from_slice(&bytes).ok())
        } else {
            // A move from .local flushes both directories, including the new store's entry.
            let stage = tempfile::Builder::new()
                .prefix(".prepared-")
                .tempdir_in(root.join(".local"))
                .map_err(io_failure)?;
            let candidate = stage.path().join("resources");
            let on_copy = |done, total: usize| {
                seal_meter.tick(
                    hashed_files.saturating_add(done),
                    hashed_files.saturating_add(total.saturating_mul(2)),
                )
            };
            let copied = logged("Sealing game files: copying", || {
                let copied =
                    copy_tree_with(source, &candidate, true, 0, workers, stop, Some(&on_copy))?;
                let files = copied.files.len();
                Ok((copied, files))
            })?;
            let copied_files = copied.files.len();
            let on_flush = |done, total: usize| {
                seal_meter.tick(
                    hashed_files
                        .saturating_add(copied_files)
                        .saturating_add(done),
                    hashed_files
                        .saturating_add(copied_files)
                        .saturating_add(total),
                )
            };
            logged("Sealing game files: flushing", || {
                flush_copied(&copied, workers, stop, Some(&on_flush))?;
                Ok(((), copied_files))
            })?;
            if let Some(stop) = stop {
                stop.check()?;
            }
            // Move the entire damaged snapshot aside, including stale accelerator evidence.
            // Publish only the already-flushed replacement while the preparation lock is held.
            publish_snapshot(
                root,
                stage.path(),
                &snapshot,
                exists,
                durable::move_new_directory,
            )?;
            let digest = accelerator(&resources)?;
            if let Some(digest) = digest {
                write_receipt(&evidence, &digest)?;
            }
            digest
        };
        let prepared = Self {
            primary,
            accelerator,
        };
        write_receipt(&parent.join(CURRENT), &prepared)?;
        Ok(PreparedResources {
            resources: prepared,
            proven_by_seal: true,
        })
    }
}

pub(super) fn generated_cache(name: &OsStr) -> bool {
    name == preparer::DATA_FILE
        || name.to_str().is_some_and(|name| {
            name.strip_prefix("sunrise-data.bin.")
                .and_then(|name| name.strip_suffix(".tmp"))
                .is_some_and(|pid| !pid.is_empty() && pid.bytes().all(|c| c.is_ascii_digit()))
        })
}

fn publish_snapshot(
    root: &Path,
    staged: &Path,
    snapshot: &Path,
    exists: bool,
    publish: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> Result<(), String> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static RETIRED: AtomicUsize = AtomicUsize::new(0);

    let local = root.join(".local");
    let retired = if exists {
        require_dir(&local)?;
        let name = format!(
            ".prepared-damaged-{}-{}",
            std::process::id(),
            RETIRED.fetch_add(1, Ordering::Relaxed)
        );
        let path = local.join(&name);
        durable::move_new_directory(snapshot, &path).map_err(io_failure)?;
        Some((name, path))
    } else {
        None
    };
    if let Err(publish_err) = publish(staged, snapshot) {
        if let Some((_, retired_path)) = retired {
            durable::move_new_directory(&retired_path, snapshot).map_err(io_failure)?;
        }
        return Err(io_failure(publish_err));
    }
    if let Some((retired_name, _)) = retired {
        if let Ok(local_dir) =
            cap_std::fs::Dir::open_ambient_dir(&local, cap_std::ambient_authority())
        {
            let _ = crate::native_cache::remove_entry(&local_dir, Path::new(&retired_name));
        }
    }
    Ok(())
}

fn parent(root: &Path) -> PathBuf {
    root.join(".local").join(ROOT)
}
fn receipt(root: &Path) -> Result<PathBuf, String> {
    let parent = parent(root);
    if parent.exists() {
        for dir in [root, &root.join(".local"), &parent] {
            require_dir(dir)?;
        }
    }
    Ok(parent.join(CURRENT))
}
// These receipts only cache selection or accelerator evidence. A crash can lose them; the
// primary tree is already durable and a profile's independent digest is checked at cutoff.
fn write_receipt(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let mut staged = tempfile::NamedTempFile::new_in(path.parent().expect("snapshot directory"))
        .map_err(io_failure)?;
    serde_json::to_writer(&mut staged, value).map_err(|e| e.to_string())?;
    staged
        .persist(path)
        .map(|_| ())
        .map_err(|e| io_failure(e.error))
}
fn key(digest: [u8; 32]) -> String {
    format!("{:x}", sha2::digest::Output::<Sha256>::from(digest))
}

/// Start and end lines in the launcher log for one pass over a snapshot's files, with how many.
fn logged<T>(what: &str, run: impl FnOnce() -> Result<(T, usize), String>) -> Result<T, String> {
    let step = launcher_log::step(what);
    match run() {
        Ok((value, files)) => {
            step.done(format!("{files} files"));
            Ok(value)
        }
        Err(error) => {
            step.failed(&error);
            Err(error)
        }
    }
}

pub(super) fn copy_tree(
    source: &Path,
    dest: &Path,
    skip_accelerator: bool,
    depth: usize,
) -> Result<(), String> {
    copy_tree_with(
        source,
        dest,
        skip_accelerator,
        depth,
        pool::workers(),
        None,
        None,
    )
    .map(drop)
}

/// What a copy made: every directory in creation order, then every file.
struct Copied {
    dirs: Vec<PathBuf>,
    files: Vec<(PathBuf, PathBuf)>,
}

/// Create every directory first, in order, then copy the files on `workers` threads. The
/// workers are joined before this returns, so a caller may remove `dest` after an error.
fn copy_tree_with(
    source: &Path,
    dest: &Path,
    skip_accelerator: bool,
    depth: usize,
    workers: usize,
    stop: Option<&Cancellation>,
    on_done: Option<&(dyn Fn(usize, usize) + Sync)>,
) -> Result<Copied, String> {
    let mut copied = Copied {
        dirs: Vec::new(),
        files: Vec::new(),
    };
    lay_out(source, dest, skip_accelerator, depth, stop, &mut copied)?;
    pool::run_with_progress(
        &copied.files,
        workers,
        stop,
        on_done,
        || (),
        |_, (src, dst)| {
            let meta = fs::symlink_metadata(src).map_err(io_failure)?;
            if !plain_file(&meta) || !meta.is_file() {
                return Err(setup_error());
            }
            preparer::clone_or_copy(src, dst).map_err(io_failure)
        },
    )?;
    Ok(copied)
}

fn lay_out(
    source: &Path,
    dest: &Path,
    skip_accelerator: bool,
    depth: usize,
    stop: Option<&Cancellation>,
    copied: &mut Copied,
) -> Result<(), String> {
    if depth > 64 {
        return Err(setup_error());
    }
    if let Some(stop) = stop {
        stop.check()?;
    }
    require_dir(source)?;
    fs::create_dir(dest).map_err(io_failure)?;
    copied.dirs.push(dest.to_owned());
    for entry in fs::read_dir(source).map_err(io_failure)? {
        let entry = entry.map_err(io_failure)?;
        let name = entry.file_name();
        if skip_accelerator && generated_cache(&name) {
            continue;
        }
        let src = entry.path();
        let dst = dest.join(name);
        let meta = fs::symlink_metadata(&src).map_err(io_failure)?;
        if !plain_file(&meta) {
            return Err(setup_error());
        }
        if meta.is_dir() {
            lay_out(&src, &dst, false, depth + 1, stop, copied)?;
        } else if meta.is_file() {
            copied.files.push((src, dst));
        } else {
            return Err(setup_error());
        }
    }
    Ok(())
}

/// The flush guarantee of `durable::flush_tree`, as its own pass over the copy: every file
/// first, then every directory, all before the snapshot is renamed into place.
fn flush_copied(
    copied: &Copied,
    workers: usize,
    stop: Option<&Cancellation>,
    on_done: Option<&(dyn Fn(usize, usize) + Sync)>,
) -> Result<(), String> {
    pool::run_with_progress(
        &copied.files,
        workers,
        stop,
        on_done,
        || (),
        |_, (_, dst)| durable::flush_tree(dst).map_err(io_failure),
    )?;
    pool::run(
        &copied.dirs,
        workers,
        stop,
        || (),
        |_, dir| durable::flush_directory(dir).map_err(io_failure),
    )
    .map(drop)
}

fn setup_error() -> String {
    unsettled!()
}
fn io_failure(error: io::Error) -> String {
    unsettled!(error.kind())
}

/// The optional data cache a seal packs, as a `launcher.log` step: its digest when written.
/// A failure is logged only as the step's kind of failure, as the preparer's own message can name
/// a local path (review of #615), and the seal goes on without the cache unless it was cancelled.
fn pack_data_cache(
    binary: &Path,
    root: &Path,
    resources: &Path,
    handoff: Option<&preparer::Handoff<'_>>,
    stop: Option<&crate::native_mods::Cancellation>,
) -> Result<Option<[u8; 32]>, String> {
    let check = || stop.map_or(Ok(()), crate::native_mods::Cancellation::check);
    let step = launcher_log::step("Packing game data cache");
    match preparer::data_file(binary, root, resources, handoff) {
        Ok(Some(true)) => {
            check()?;
            let digest = digest_file(&resources.join(preparer::DATA_FILE))
                .ok()
                .flatten();
            step.done("");
            Ok(digest)
        }
        Ok(_) => {
            step.done("already present");
            Ok(None)
        }
        Err(error) if stop.is_some_and(crate::native_mods::Cancellation::is_stopped) => {
            step.failed(&error);
            Err(error)
        }
        Err(error) => {
            step.failed(&error);
            eprintln!(
                "Packing game data cache: {}",
                launcher_log::safe_error(&error)
            );
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A data-file reply whose error message names a local path: the seal's step logs only the
    /// kind of failure, and nothing of the message reaches the log (review of #615).
    #[cfg(unix)]
    #[test]
    fn a_failed_data_cache_logs_none_of_the_preparers_message() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("version");
        let resources = root.join(".local/sunrise-client/26.3");
        fs::create_dir_all(&resources).unwrap();
        let binary = root.join("sunrise-prepare");
        fs::write(
            &binary,
            "#!/bin/sh\ncase \"$1\" in\n  data-file) echo '{\"format\":1,\"error\":{\"message\":\"/home/ada/Private Client Backup/sunrise/data.bin: Permission denied (os error 13)\"}}';;\n  *) exit 2;;\nesac\n",
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let before = launcher_log::since(0).0;
        assert_eq!(
            pack_data_cache(&binary, &root, &resources, None, None),
            Ok(None),
            "the seal goes on without the cache"
        );
        let (_, lines) = launcher_log::since(before);
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Packing game data cache: failed after")),
            "{lines:?}"
        );
        for word in ["Private Client Backup", "/home/ada"] {
            assert!(
                lines.iter().all(|line| !line.contains(word)),
                "{word}: {lines:?}"
            );
        }
    }

    #[test]
    fn cannot_seal_a_prepared_tree_outside_the_install() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("version");
        let source = temp.path().join("prepared");
        fs::create_dir_all(root.join(".local")).unwrap();
        fs::create_dir(&source).unwrap();
        fs::write(source.join("sunrise-resources.json"), "{}").unwrap();
        for source in [source, root.join("../prepared")] {
            assert!(Resources::seal(&root, &source, None, |_| Ok(None)).is_err());
        }
        assert!(!parent(&root).exists());
    }

    #[test]
    fn missing_snapshot_uses_the_same_seal_path_as_first_preparation() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let source = root.join(".local/prepared");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("sunrise-resources.json"), "{}").unwrap();
        let prepared = Resources::seal(root, &source, None, |_| Ok(None)).unwrap();
        let path = prepared.path_at(root);
        for missing in [&path, path.parent().unwrap()] {
            fs::remove_dir_all(missing).unwrap();
            assert_eq!(Resources::current(root).unwrap(), None);
            assert_eq!(
                Resources::seal(root, &source, None, |_| Ok(None)).unwrap(),
                prepared
            );
            assert_eq!(Resources::current(root).unwrap().as_ref(), Some(&prepared));
        }
    }

    #[test]
    fn sealing_replaces_a_damaged_snapshot_and_its_accelerator() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let source = nested_source(root);
        let prepared = Resources::seal(root, &source, None, |dir| {
            let data = dir.join(preparer::DATA_FILE);
            fs::write(&data, b"old cache").unwrap();
            digest_file(&data)
        })
        .unwrap();
        let snapshot = prepared.path_at(root);
        fs::write(snapshot.join("assets/deep/er/blob.bin"), b"damage").unwrap();
        let repaired = Resources::seal(root, &source, None, |dir| {
            let data = dir.join(preparer::DATA_FILE);
            assert!(!data.exists());
            fs::write(&data, b"rebuilt cache").unwrap();
            digest_file(&data)
        })
        .unwrap();
        assert_eq!(repaired.primary, prepared.primary);
        assert_eq!(
            artifact::tree(&snapshot, true).unwrap(),
            Some(prepared.primary)
        );
        assert_ne!(repaired.accelerator, prepared.accelerator);
        assert_eq!(Resources::current(root).unwrap(), Some(repaired));
    }

    #[test]
    fn a_receipt_cannot_select_stale_or_server_preparation() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let source = root.join(".local/prepared");
        fs::create_dir_all(&source).unwrap();
        let lib = root.join("crates/sunrise-prepare/src/lib.rs");
        fs::create_dir_all(lib.parent().unwrap()).unwrap();
        fs::write(lib, "pub const OUTPUT_VERSION: u32 = 8;\n").unwrap();
        let manifest = source.join("sunrise-resources.json");
        let stamp = source.join("sunrise-prepare.json");
        fs::write(&manifest, "{}").unwrap();
        fs::write(&stamp, r#"{"output_version":7}"#).unwrap();
        Resources::seal(root, &source, None, |_| Ok(None)).unwrap();
        assert_eq!(Resources::current(root).unwrap(), None);

        fs::write(&stamp, r#"{"output_version":8}"#).unwrap();
        fs::write(&manifest, r#"{"profile":"server"}"#).unwrap();
        Resources::seal(root, &source, None, |_| Ok(None)).unwrap();
        assert_eq!(Resources::current(root).unwrap(), None);

        fs::write(&manifest, "{}").unwrap();
        let full = Resources::seal(root, &source, None, |_| Ok(None)).unwrap();
        assert_eq!(Resources::current(root).unwrap(), Some(full));
    }

    #[test]
    fn prepared_snapshot_does_not_read_a_cache_from_a_different_tree() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path();
        let source = root.join(".local/prepared");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("sunrise-resources.json"), "{}").unwrap();
        fs::write(source.join("blocks.json"), "{\"old\": true}").unwrap();
        fs::write(source.join(preparer::DATA_FILE), b"untrusted").unwrap();
        let prepared = Resources::seal(root, &source, None, |dir| {
            let data = dir.join(preparer::DATA_FILE);
            assert!(!data.exists());
            fs::write(&data, b"generated from the snapshot").unwrap();
            Ok(digest_file(&data).unwrap())
        })
        .unwrap();
        let snapshot = parent(root).join(key(prepared.primary)).join("resources");
        assert_eq!(
            fs::read_to_string(snapshot.join("blocks.json")).unwrap(),
            "{\"old\": true}"
        );
        assert_eq!(
            Resources::seal(root, &source, None, |_| panic!("snapshot reused")).unwrap(),
            prepared
        );
        fs::write(source.join("blocks.json"), "{\"new\": true}").unwrap();
        assert_eq!(Resources::current(root).unwrap().as_ref(), Some(&prepared));
        fs::write(receipt(root).unwrap(), b"partial").unwrap();
        assert_eq!(Resources::current(root).unwrap(), None);
        Resources::invalidate(root).unwrap();
        assert_eq!(Resources::current(root).unwrap(), None);
        let changed = Resources::seal(root, &source, None, |_| Ok(None)).unwrap();
        assert_ne!(prepared.primary, changed.primary);
        assert_eq!(Resources::current(root).unwrap(), Some(changed));
        fs::write(snapshot.join(preparer::DATA_FILE), b"different cache").unwrap();
        fs::write(snapshot.join("sunrise-data.bin.123.tmp"), b"pending cache").unwrap();
        let primary = || artifact::tree(&snapshot, true).unwrap();
        assert_eq!(primary(), Some(prepared.primary));
        fs::write(snapshot.join("blocks.json"), b"different primary bytes").unwrap();
        assert_ne!(primary(), Some(prepared.primary));
    }

    fn nested_source(root: &Path) -> PathBuf {
        let source = root.join(".local/prepared");
        fs::create_dir_all(source.join("assets/empty")).unwrap();
        fs::create_dir_all(source.join("assets/deep/er")).unwrap();
        fs::write(source.join("sunrise-resources.json"), "{}").unwrap();
        fs::write(source.join("assets/deep/er/blob.bin"), vec![7u8; 150_000]).unwrap();
        fs::write(source.join("assets/empty.txt"), b"").unwrap();
        fs::write(source.join(preparer::DATA_FILE), b"untrusted").unwrap();
        source
    }

    #[test]
    fn one_and_two_workers_seal_the_same_snapshot() {
        let mut sealed = Vec::new();
        for workers in [1, 2] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            let source = nested_source(root);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let path = source.join("assets/empty.txt");
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            }
            let prepared = Resources::seal_with(root, &source, workers, None, true, |_| Ok(None))
                .unwrap()
                .resources;
            let snapshot = prepared.path_at(root);
            assert_eq!(
                artifact::tree(&snapshot, true).unwrap(),
                Some(prepared.primary)
            );
            assert!(snapshot.join("assets/empty").is_dir());
            assert!(!snapshot.join(preparer::DATA_FILE).exists());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = fs::metadata(snapshot.join("assets/empty.txt"))
                    .unwrap()
                    .permissions()
                    .mode();
                assert_eq!(mode & 0o111, 0o111);
            }
            assert_eq!(Resources::current(root).unwrap().as_ref(), Some(&prepared));
            sealed.push(prepared);
        }
        assert_eq!(sealed[0], sealed[1]);
    }

    #[test]
    fn a_stopped_seal_returns_the_cancellation_and_leaves_no_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let source = nested_source(root);
        let stop = Cancellation::default();
        stop.stop();
        for workers in [1, 2] {
            let error = Resources::seal_with(root, &source, workers, Some(&stop), true, |_| {
                panic!("sealed after a stop")
            })
            .unwrap_err();
            assert_eq!(Some(error), stop.check().err());
            assert!(!parent(root).exists());
        }
        let copying = copy_tree_with(&source, &root.join("copy"), true, 0, 2, Some(&stop), None);
        assert!(copying.is_err());
        let leftovers: Vec<_> = fs::read_dir(root.join(".local"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, ["prepared"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_found_while_copying_is_refused_and_every_worker_is_joined() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let source = nested_source(root);
        let link = source.join("assets/deep/link");
        std::os::unix::fs::symlink(source.join("assets/empty.txt"), &link).unwrap();
        for workers in [1, 2] {
            let error = copy_tree_with(&source, &root.join("copy"), true, 0, workers, None, None)
                .err()
                .unwrap();
            assert_eq!(error, setup_error());
            fs::remove_dir_all(root.join("copy")).unwrap();
            let error =
                Resources::seal_with(root, &source, workers, None, true, |_| Ok(None)).unwrap_err();
            assert!(error.contains("invalid data"), "{error}");
            assert!(!parent(root).exists());
        }
    }

    #[test]
    fn a_home_sealed_by_v1_3_4_is_selected_and_kept_without_sealing_again() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let source = nested_source(root);
        // v1.3.4's layout: `<key>/resources` beside `current.json`, written by hand here.
        let primary = artifact::tree(&source, true).unwrap().unwrap();
        let sealed = Resources {
            primary,
            accelerator: None,
        };
        let snapshot = sealed.path_at(root);
        fs::create_dir_all(snapshot.parent().unwrap()).unwrap();
        copy_tree_with(&source, &snapshot, true, 0, 1, None, None).unwrap();
        write_receipt(&parent(root).join(CURRENT), &sealed).unwrap();
        assert_eq!(Resources::current(root).unwrap().as_ref(), Some(&sealed));
        assert_eq!(artifact::tree(&snapshot, true).unwrap(), Some(primary));
        let again = Resources::seal(root, &source, None, |_| panic!("sealed again")).unwrap();
        assert_eq!(again, sealed);
    }

    #[test]
    fn play_receipt_miss_seals_new_snapshot_but_never_repairs_an_existing_one() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let source = nested_source(root);
        let sealed = Resources::seal_with(root, &source, 1, None, false, |_| Ok(None)).unwrap();
        assert!(sealed.proven_by_seal);
        assert_eq!(
            Resources::current(root).unwrap().as_ref(),
            Some(&sealed.resources)
        );
        let snapshot = sealed.resources.path_at(root);
        let blob = snapshot.join("assets/deep/er/blob.bin");
        Resources::invalidate(root).unwrap();
        fs::write(&blob, b"damaged on disk").unwrap();
        let reused = Resources::seal_with(root, &source, 1, None, false, |_| {
            panic!("Play receipt miss must not seal over an existing snapshot")
        })
        .unwrap();
        assert!(!reused.proven_by_seal);
        assert_eq!(reused.resources, sealed.resources);
        assert_eq!(fs::read(&blob).unwrap(), b"damaged on disk");
        assert_eq!(Resources::current(root).unwrap(), None);
        let repaired = Resources::seal_with(root, &source, 1, None, true, |_| Ok(None)).unwrap();
        assert!(repaired.proven_by_seal);
        assert_eq!(
            artifact::tree(&snapshot, true).unwrap(),
            Some(sealed.resources.primary)
        );
        assert_eq!(
            Resources::current(root).unwrap().as_ref(),
            Some(&repaired.resources)
        );
    }

    #[test]
    fn publish_snapshot_restores_previous_on_failure_and_removes_retired_sibling_on_success() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let local = root.join(".local");
        let snapshot = parent(root).join("snapshot-a");
        let staged = local.join(".prepared-stage");
        fs::create_dir_all(snapshot.join("resources")).unwrap();
        fs::create_dir_all(staged.join("resources")).unwrap();
        fs::write(snapshot.join("resources/blob.bin"), b"original snapshot").unwrap();
        fs::write(snapshot.join("data-file.json"), b"old evidence").unwrap();
        fs::write(staged.join("resources/blob.bin"), b"replacement snapshot").unwrap();

        let damaged_siblings = || {
            fs::read_dir(&local)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with(".prepared-damaged-"))
                .collect::<Vec<_>>()
        };

        let err = publish_snapshot(root, &staged, &snapshot, true, |_, _| {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        })
        .expect_err("failing publish must return an error");
        assert!(!err.is_empty());
        assert_eq!(
            fs::read(snapshot.join("resources/blob.bin")).unwrap(),
            b"original snapshot"
        );
        assert_eq!(
            fs::read(snapshot.join("data-file.json")).unwrap(),
            b"old evidence"
        );
        assert!(
            damaged_siblings().is_empty(),
            "retired sibling must be restored, not left behind"
        );
        assert_eq!(
            fs::read(staged.join("resources/blob.bin")).unwrap(),
            b"replacement snapshot"
        );

        publish_snapshot(root, &staged, &snapshot, true, durable::move_new_directory).unwrap();
        assert_eq!(
            fs::read(snapshot.join("resources/blob.bin")).unwrap(),
            b"replacement snapshot"
        );
        assert!(!snapshot.join("data-file.json").exists());
        assert!(!staged.exists());
        assert!(
            damaged_siblings().is_empty(),
            "retired sibling must be removed after publish succeeds"
        );
    }
}
