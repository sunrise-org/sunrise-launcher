//! Where a build's resources are, whether they are prepared (and the short-lived cache of that
//! answer), and the step a preparation is at.
use super::{check_play_resources, preparer_source_error, server_only_resources};
use crate::backend::{game_version_at, read_json, version_dir};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The version whose folder holds `tag`'s prepared resources: a native build shares its base's
/// (`copy_base` leaves `.local` out), any other version keeps its own.
pub fn resources_owner(tag: &str) -> String {
    resources_owner_in(&crate::backend::versions_dir(), tag)
}

pub(in crate::backend) fn resources_owner_in(versions: &Path, tag: &str) -> String {
    crate::native_mods::is_native(tag)
        .then(|| crate::native_mods::about_in(&versions.join(tag)))
        .flatten()
        .map_or_else(|| tag.to_owned(), |about| about.base)
}

pub fn resources(tag: &str) -> PathBuf {
    resources_at(&version_dir(&resources_owner(tag)))
}

pub(in crate::backend) fn resources_at(dir: &Path) -> PathBuf {
    resources_for_game(dir, &game_version_at(dir))
}

pub(crate) fn resources_for_game(dir: &Path, game_version: &str) -> PathBuf {
    dir.join(".local/sunrise-client").join(game_version)
}

pub(in crate::backend) const RESOURCES_MANIFEST: &str = "sunrise-resources.json";
pub(in crate::backend) const PREPARATION_STAMP: &str = "sunrise-prepare.json";

/// The target preparation records the world format of the verified client JAR it used.
pub(crate) fn prepared_world_version(resources: &Path) -> Option<i32> {
    let manifest = read_json(&resources.join(RESOURCES_MANIFEST))?;
    i32::try_from(manifest.get("data_version")?.as_i64()?).ok()
}

/// How long a drawn page believes an answer of [`is_prepared`] without looking at the disk again.
/// What the launcher itself changes (a preparation, an install, a removal) drops it at once
/// ([`forget_prepared`]); this only bounds how late a change made by another process shows.
pub(in crate::backend) const PREPARED_TTL: std::time::Duration = std::time::Duration::from_secs(5);

/// The answers of [`is_prepared`]: version folder, whether it was prepared, when that was checked.
#[derive(Default)]
pub(in crate::backend) struct PreparedCache(
    pub(in crate::backend) Vec<(PathBuf, bool, std::time::Instant)>,
);

impl PreparedCache {
    fn get(&self, dir: &Path, now: std::time::Instant) -> Option<bool> {
        let (_, prepared, at) = self.0.iter().find(|(d, ..)| d == dir)?;
        (now.saturating_duration_since(*at) < PREPARED_TTL).then_some(*prepared)
    }

    fn put(&mut self, dir: &Path, prepared: bool, now: std::time::Instant) {
        self.0.retain(|(d, ..)| d != dir);
        self.0.push((dir.to_path_buf(), prepared, now));
    }
}

static PREPARED: Mutex<PreparedCache> = Mutex::new(PreparedCache(Vec::new()));

fn lock_prepared(cache: &Mutex<PreparedCache>) -> std::sync::MutexGuard<'_, PreparedCache> {
    cache.lock().unwrap_or_else(|e| e.into_inner())
}

/// The answer kept for `dir`, else `check()` (run without the lock held), kept.
pub(in crate::backend) fn cached_prepared(
    cache: &Mutex<PreparedCache>,
    dir: &Path,
    now: std::time::Instant,
    check: impl FnOnce() -> bool,
) -> bool {
    if let Some(known) = lock_prepared(cache).get(dir, now) {
        return known;
    }
    let prepared = check();
    lock_prepared(cache).put(dir, prepared, now);
    prepared
}

/// Whether `tag` is prepared, as a page being drawn asks it (every frame, for each version it
/// shows): an answer stands for [`PREPARED_TTL`], so a frame does no file reads. Anything that
/// must not act on a stale answer (starting the game or a server) calls [`check_prepared`].
pub fn is_prepared(tag: &str) -> bool {
    cached_prepared(
        &PREPARED,
        &version_dir(tag),
        std::time::Instant::now(),
        || {
            let root = version_dir(&resources_owner(tag));
            check_play_resources(&root, &resources_at(&root)).is_ok()
        },
    )
}

/// Whether `tag` is prepared, checked on the disk now. The answer replaces the one
/// [`is_prepared`] keeps.
pub fn check_prepared(tag: &str) -> bool {
    check_build_preparation(tag).is_ok()
}

pub(in crate::backend) fn check_build_preparation(tag: &str) -> Result<(), Unprepared> {
    checked_prepared(
        &PREPARED,
        &version_dir(tag),
        &version_dir(&resources_owner(tag)),
        std::time::Instant::now(),
    )
}

pub(in crate::backend) fn checked_prepared(
    cache: &Mutex<PreparedCache>,
    key: &Path,
    root: &Path,
    now: std::time::Instant,
) -> Result<(), Unprepared> {
    let prepared = check_play_resources(root, &resources_at(root));
    lock_prepared(cache).put(key, prepared.is_ok(), now);
    prepared
}

/// Drops what [`is_prepared`] remembers: a preparation, install or removal changed the disk.
pub fn forget_prepared() {
    lock_prepared(&PREPARED).0.clear();
}

#[derive(Debug)]
pub(crate) enum Unprepared {
    Incomplete,
    ServerOnly,
    PreparerSource(std::io::Error),
}

impl Unprepared {
    pub(crate) fn message(self, incomplete: impl FnOnce() -> String) -> String {
        match self {
            Self::Incomplete => incomplete(),
            Self::ServerOnly => server_only_resources(),
            Self::PreparerSource(error) => preparer_source_error(&error),
        }
    }
}

/// A preparation's steps, in the order they run, as Home names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PrepareStep {
    /// Minecraft's files: checked in Sunrise's store, copied from matching installs or downloaded.
    Files,
    Java,
    Sounds,
    /// sunrise-prepare's own work on those files.
    Prepare,
}

impl PrepareStep {
    pub const ALL: [PrepareStep; 4] = [Self::Files, Self::Java, Self::Sounds, Self::Prepare];

    pub fn name(self) -> &'static str {
        match self {
            Self::Files => crate::i18n::tr_key("Game files"),
            Self::Java => crate::i18n::tr_key("Java"),
            Self::Sounds => crate::i18n::tr_key("Sounds"),
            Self::Prepare => crate::i18n::tr_key("Prepare"),
        }
    }
}

/// The step the preparation under way is at and when it began, kept together so readers never
/// pair a new step with the previous step's start time.
static PREPARE_STATE: std::sync::Mutex<(PrepareStep, Option<std::time::Instant>)> =
    std::sync::Mutex::new((PrepareStep::Files, None));

pub fn prepare_step() -> PrepareStep {
    prepare_step_state().0
}

pub fn prepare_step_state() -> (PrepareStep, Option<std::time::Instant>) {
    *PREPARE_STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn set_prepare_step(step: PrepareStep) {
    *PREPARE_STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = (step, Some(std::time::Instant::now()));
}
