//! Installed game builds, as distinct from the launcher's disposable download caches.
use super::{in_use, installed, is_game_tag, version_dir, Release, Settings};
use crate::browser::sunrise::txn;
use crate::cache_cleanup::{self, Error};
use crate::i18n::{tr, trf};
use crate::{instances, native_mods, nightly};
use std::{fs, path::Path};

pub(super) const KEEP_NEWEST: usize = 2;
const REQUESTED: &str = ".downloaded-build";
const AUTOMATIC: &str = ".automatic-download";
// The old marker was also written by an explicit CLI update. It is not proof of an
// automatic download, but remains valid download provenance for manual cleanup.
const LEGACY_DOWNLOADED: &str = ".auto-installed";

#[derive(Clone, Copy)]
pub(crate) enum Download {
    Requested,
    Automatic,
}

impl Download {
    pub(super) fn mark(self, dir: &Path) -> Result<(), String> {
        let path = dir.join(match self {
            Self::Requested => REQUESTED,
            Self::Automatic => AUTOMATIC,
        });
        match fs::File::create_new(&path) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && path.is_file() => Ok(()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }
}

fn downloaded(tag: &str) -> bool {
    let dir = version_dir(tag);
    [REQUESTED, AUTOMATIC, LEGACY_DOWNLOADED]
        .into_iter()
        .any(|name| dir.join(name).is_file())
}

fn auto_downloaded(dir: &Path) -> bool {
    dir.join(AUTOMATIC).is_file()
        && dir
            .join(REQUESTED)
            .symlink_metadata()
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
}

fn old_downloads(
    installed: &[String],
    keep: usize,
    downloaded: impl Fn(&str) -> bool,
) -> Vec<String> {
    let (mut releases, mut nightlies) = (0, 0);
    installed
        .iter()
        .filter(|tag| {
            // Versioned releases predate the download marker. Local builds have separate tags.
            if is_game_tag(tag) {
                releases += 1;
                releases > keep
            } else if nightly::date_of(tag).is_some() && downloaded(tag) {
                nightlies += 1;
                nightlies > keep
            } else {
                false
            }
        })
        .cloned()
        .collect()
}

/// The row's reason to hide Remove. This is advisory; deletion rechecks under the publication lock.
pub enum InUse {
    Profile(String),
    Current,
    Running,
    NativeBuild,
    ProfileReference,
}

pub struct Build {
    pub tag: String,
    pub bytes: u64,
    pub in_use: Option<InUse>,
    pub old_download: bool,
}

fn profile_user<'a>(
    profiles: &'a [instances::Instance],
    installed: &[String],
    selected: Option<&String>,
    releases: &[Release],
    tag: &str,
) -> Option<&'a instances::Instance> {
    profiles.iter().find(|profile| {
        instances::profile_target(
            profile.version.as_ref(),
            profile.channel,
            releases,
            installed,
            selected,
        )
        .is_some_and(|wanted| wanted == tag)
    })
}

pub fn inventory(releases: &[Release]) -> Vec<Build> {
    let installed = installed();
    let settings = Settings::load();
    let profiles = instances::list_for_storage();
    let old = old_downloads(&installed, settings.keep_newest(), downloaded);
    installed
        .iter()
        .map(|tag| {
            let in_use = if let Some(profile) = profile_user(
                &profiles,
                &installed,
                settings.selected.as_ref(),
                releases,
                tag,
            ) {
                Some(InUse::Profile(profile.name.clone()))
            } else if settings.selected.as_ref() == Some(tag) {
                Some(InUse::Current)
            } else if in_use(tag) {
                Some(InUse::Running)
            } else if native_mods::resource_dependents(tag) {
                Some(InUse::NativeBuild)
            } else if txn::references(tag).unwrap_or(true) {
                Some(InUse::ProfileReference)
            } else {
                None
            };
            Build {
                tag: tag.clone(),
                bytes: disk_use(&version_dir(tag)),
                in_use,
                old_download: old.contains(tag),
            }
        })
        .collect()
}

fn disk_use(path: &Path) -> u64 {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| disk_use(&entry.path()))
            .sum(),
        Ok(meta) if meta.is_file() => meta.len(),
        // Prepared resources can be linked from elsewhere. Count only files inside this build.
        _ => 0,
    }
}

/// Remove exactly the rows confirmed by the player. Neither a newer download nor a changed
/// profile may cause a different row to be removed. Automatic cleanup uses this same path.
pub fn remove_builds(
    tags: &[String],
    releases: &[Release],
    old_only: bool,
) -> Vec<(String, String)> {
    tags.iter()
        .filter_map(|tag| {
            delete(tag, Some(releases), old_only, false)
                .err()
                .map(|e| (tag.clone(), e))
        })
        .collect()
}

/// The picker can fetch a candidate without selecting it or deleting anything.
pub fn prune_old_versions(
    releases: &[Release],
    just_installed: &str,
    cache_only: bool,
) -> Vec<(String, String)> {
    if cache_only {
        return Vec::new();
    }
    let settings = Settings::load();
    let mut errors = Vec::new();
    if settings.auto_prune() {
        for tag in old_downloads(&installed(), settings.keep_newest(), downloaded) {
            if tag == just_installed || !auto_downloaded(&version_dir(&tag)) {
                continue;
            }
            if let Err(error) = delete(&tag, Some(releases), true, true) {
                errors.push((tag, error));
            }
        }
    }
    errors
}

/// The CLI's explicit Remove may change the launcher selection, but not a profile pointer.
pub fn remove_version(tag: &str) -> Result<(), String> {
    delete(tag, None, false, false)?;
    let mut settings = Settings::load();
    if settings.selected.as_deref() == Some(tag) {
        settings.selected = installed().first().cloned();
        settings.save()?;
    }
    Ok(())
}

fn delete(
    tag: &str,
    releases: Option<&[Release]>,
    old_only: bool,
    automatic: bool,
) -> Result<(), String> {
    // Only installed folder names may enter the deletion fence.
    if !installed().iter().any(|v| v == tag) {
        return Err(trf("{tag} is not installed.", &[("tag", tag)]));
    }
    let mut dependent = None;
    let deletion = cache_cleanup::delete_version(&version_dir(tag), || {
        let installed = installed();
        let settings = Settings::load();
        if automatic && !auto_downloaded(&version_dir(tag)) {
            return Err(Error::IncompleteUnit);
        }
        if old_only
            && !old_downloads(&installed, settings.keep_newest(), downloaded)
                .iter()
                .any(|t| t == tag)
        {
            return Err(Error::IncompleteUnit);
        }
        dependent = native_mods::resource_dependent(tag);
        if in_use(tag)
            || dependent.is_some()
            || releases.is_some_and(|releases| {
                settings.selected.as_deref() == Some(tag)
                    || profile_user(
                        &instances::list_for_storage(),
                        &installed,
                        settings.selected.as_ref(),
                        releases,
                        tag,
                    )
                    .is_some()
            })
        {
            return Err(Error::IncompleteUnit);
        }
        // The fence also checks durable profile pointers and runtime leases before detaching.
        Ok(())
    });
    if automatic && matches!(deletion, Err(Error::IncompleteUnit | Error::Busy)) {
        // A profile or runtime can claim a build between inventory and deletion.
        return Ok(());
    }
    deletion.map_err(|error| match (error, dependent) {
        (Error::IncompleteUnit, Some(build)) => trf(
            "This version supplies resources to native build {build}. Remove that build first.",
            &[("build", &build)],
        ),
        (Error::IncompleteUnit | Error::Busy, _) => {
            tr("This build is in use or could not be safely removed.").into()
        }
        (error, _) => native_mods::cache_error(error),
    })?;
    super::forget_prepared();
    crate::storage::eviction::request_cleanup();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulk_cleanup_keeps_local_and_unknown_builds_and_counts_each_channel_separately() {
        let installed = [
            "developer-2026-11-22-abcd",
            "imported-2026-11-22-abcd",
            "native-local-build",
            "nightly-2026-11-21-aaaaaaa",
            "nightly-2026-11-20-bbbbbbb",
            "nightly-2026-11-19-ccccccc",
            "nightly-2026-11-18-ddddddd",
            "v0.2.91",
            "v0.2.90",
            "v0.2.89",
        ]
        .map(String::from);
        // An older manually installed Nightly of unknown origin doesn't use a slot.
        let downloaded = |tag: &str| tag != installed[4];
        assert_eq!(
            old_downloads(&installed, 2, downloaded),
            ["nightly-2026-11-18-ddddddd", "v0.2.89"]
        );
        assert_eq!(
            old_downloads(&installed, 1, downloaded),
            [
                "nightly-2026-11-19-ccccccc",
                "nightly-2026-11-18-ddddddd",
                "v0.2.90",
                "v0.2.89"
            ]
        );
    }

    #[test]
    fn pruning_preserves_manual_and_legacy_nightlies() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        assert!(!auto_downloaded(dir), "unknown origin stays");
        fs::write(dir.join(LEGACY_DOWNLOADED), "").unwrap();
        assert!(
            !auto_downloaded(dir),
            "the legacy mark may be from CLI update"
        );
        Download::Automatic.mark(dir).unwrap();
        assert!(auto_downloaded(dir));
        Download::Requested.mark(dir).unwrap();
        assert!(
            !auto_downloaded(dir),
            "a later explicit download protects a cached archive"
        );
    }

    #[test]
    fn confirmation_does_not_remove_a_build_a_profile_has_since_pinned() {
        const ACTOR: &str = "SUNRISE_TEST_STORAGE_CONFIRMATION";
        let scratch = tempfile::tempdir().unwrap();
        if std::env::var_os(ACTOR).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "backend::version_storage::tests::confirmation_does_not_remove_a_build_a_profile_has_since_pinned"])
                .env(ACTOR, "1")
                .env("SUNRISE_LAUNCHER_HOME", scratch.path().join("launcher"))
                .env("SUNRISE_HOME", scratch.path().join("game"))
                .output().unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        let recent = "v0.2.92";
        let newly_pinned = "v0.2.91";
        let spare = "v0.2.90";
        for tag in [recent, newly_pinned, spare] {
            let dir = version_dir(tag);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(super::super::COMPLETE), "").unwrap();
            fs::write(dir.join(super::super::CLIENT), "fixture").unwrap();
        }
        Settings {
            selected: Some(recent.into()),
            keep_versions: Some(1),
            ..Default::default()
        }
        .save()
        .unwrap();
        Download::Automatic.mark(&version_dir(spare)).unwrap();
        assert!(prune_old_versions(&[], recent, true).is_empty());
        assert_eq!(
            installed(),
            [recent, newly_pinned, spare],
            "picker Download cannot prune"
        );
        assert!(prune_old_versions(&[], recent, false).is_empty());
        assert_eq!(
            installed(),
            [recent, newly_pinned],
            "only the automatic download may be pruned"
        );
        fs::create_dir_all(version_dir(spare)).unwrap();
        fs::write(version_dir(spare).join(super::super::COMPLETE), "").unwrap();
        fs::write(version_dir(spare).join(super::super::CLIENT), "fixture").unwrap();
        let profile = instances::create("Survival", None).unwrap();
        let stable = instances::create("Offline Stable", None).unwrap();
        let stable = instances::edit(&stable.id, |meta| {
            meta.channel = Some(instances::Channel::Stable)
        })
        .unwrap();
        assert!(profile_user(&[stable], &installed(), None, &[], recent).is_some());
        let worlds = profile.home.join("saves");
        fs::create_dir_all(&worlds).unwrap();
        fs::write(worlds.join("world"), "keep me").unwrap();
        let confirmed: Vec<_> = inventory(&[])
            .into_iter()
            .filter(|row| row.old_download && row.in_use.is_none())
            .map(|row| row.tag)
            .collect();
        assert_eq!(confirmed, [newly_pinned, spare]);
        instances::edit(&profile.id, |meta| meta.version = Some(newly_pinned.into())).unwrap();
        let errors = remove_builds(&confirmed, &[], true);
        assert_eq!(
            errors
                .iter()
                .map(|(tag, _)| tag.as_str())
                .collect::<Vec<_>>(),
            [newly_pinned]
        );
        assert_eq!(installed(), [recent, newly_pinned]);
        assert_eq!(fs::read_to_string(worlds.join("world")).unwrap(), "keep me");
    }
}
