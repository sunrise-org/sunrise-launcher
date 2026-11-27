//! Installed game builds, separate from the launcher's disposable downloads.
use super::{
    in_use, installed, is_game_tag, play_installations, play_installations_until, version_dir,
    Release, Settings,
};
use crate::cache_cleanup::{self, Error};
use crate::i18n::{tr, trf};
use crate::storage::Cancel;
use crate::{instances, native_mods, nightly};
use std::{fs, path::Path};

pub(super) const KEEP_NEWEST: usize = 2;
const REQUESTED: &str = ".downloaded-build";
const AUTOMATIC: &str = ".automatic-download";
// An explicit CLI update could write the old marker. It is not proof of an automatic download.
const LEGACY_DOWNLOADED: &str = ".auto-installed";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Download {
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
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && plain_file(&path) => Ok(()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }
}

fn plain_file(path: &Path) -> bool {
    path.symlink_metadata()
        .is_ok_and(|m| m.is_file() && !crate::native_cache::linked(&m))
}

fn downloaded(tag: &str) -> bool {
    let dir = version_dir(tag);
    [REQUESTED, AUTOMATIC, LEGACY_DOWNLOADED]
        .into_iter()
        .any(|name| plain_file(&dir.join(name)))
}

fn auto_downloaded(dir: &Path) -> bool {
    plain_file(&dir.join(AUTOMATIC))
        && dir
            .join(REQUESTED)
            .symlink_metadata()
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
}

fn old_downloads<'a>(
    installed: &'a [String],
    keep: usize,
    downloaded: impl Fn(&str) -> bool + 'a,
) -> impl Iterator<Item = &'a String> + 'a {
    let (mut stable, mut nightlies) = (0, 0);
    installed.iter().filter(move |tag| {
        if is_game_tag(tag) {
            stable += 1;
            stable > keep
        } else if nightly::date_of(tag).is_some() && downloaded(tag) {
            nightlies += 1;
            nightlies > keep
        } else {
            false
        }
    })
}

/// The row's reason to hide Remove. Deletion rechecks under the publication fence.
#[derive(Clone)]
pub enum InUse {
    Profile(String),
    Current,
    Running,
    NativeBuild,
    ProfileReference,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SizeUnavailable {
    Vanished,
    Unreadable,
    Cancelled,
}

/// An installed build. Both measurement fields are `None` until counting reaches it.
#[derive(Clone)]
pub struct Build {
    pub tag: String,
    pub bytes: Option<u64>,
    pub size_unavailable: Option<SizeUnavailable>,
    pub in_use: Option<InUse>,
    pub old_download: bool,
}

fn profile_user<'a>(
    profiles: &'a [instances::Instance],
    installed: &[String],
    selected: Option<&String>,
    nightly: Option<&String>,
    releases: &[Release],
    tag: &str,
) -> Option<&'a instances::Instance> {
    let selected = selected.filter(|tag| installed.contains(tag));
    profiles.iter().find(|profile| {
        profile
            .version
            .as_ref()
            .or_else(|| {
                profile
                    .channel
                    .play_target(releases, installed, nightly, selected)
            })
            .is_some_and(|wanted| wanted.eq_ignore_ascii_case(tag))
    })
}

/// Under the exclusive publication fence, verify no ordinary profile or launcher default
/// selection pins or resolves to `tag`. Fails closed if `instances.json` cannot be read.
pub(crate) fn require_unpinned_by_profiles(tag: &str) -> Result<(), Error> {
    let settings = Settings::load();
    let selected = settings.selected.as_ref();
    let (installed, nightly) = play_installations(selected.map(String::as_str));
    let profiles = instances::list_for_storage().map_err(|_| Error::UnprovenReferences)?;
    if selected.is_some_and(|v| v.eq_ignore_ascii_case(tag))
        || profile_user(&profiles, &installed, selected, nightly.as_ref(), &[], tag).is_some()
    {
        return Err(Error::ReferencedProfile);
    }
    Ok(())
}

pub fn inventory(releases: &[Release]) -> Vec<Build> {
    inventory_until(releases, &Cancel::default()).expect("an uncancellable inventory finishes")
}

/// Cancels discovery at the next version or process-probe boundary.
pub fn inventory_until(releases: &[Release], cancel: &Cancel) -> Option<Vec<Build>> {
    if cancel.is_cancelled() {
        return None;
    }
    let settings = Settings::load();
    let selected = settings.selected.as_ref();
    let (installed, nightly) = play_installations_until(selected.map(String::as_str), cancel)?;
    let profiles = match instances::list_for_storage_until(cancel) {
        Ok(Some(profiles)) => Ok(profiles),
        Ok(None) => return None,
        Err(error) => Err(error),
    };
    let old: Vec<_> = old_downloads(&installed, settings.keep_newest(), |tag| {
        !cancel.is_cancelled() && downloaded(tag)
    })
    .take_while(|_| !cancel.is_cancelled())
    .collect();
    let mut builds = Vec::new();
    for tag in &installed {
        if cancel.is_cancelled() {
            return None;
        }
        let in_use = match &profiles {
            Ok(profiles) => profile_user(
                profiles,
                &installed,
                selected,
                nightly.as_ref(),
                releases,
                tag,
            )
            .map(|profile| InUse::Profile(profile.name.clone())),
            Err(_) => Some(InUse::ProfileReference),
        }
        .or_else(|| {
            if cancel.is_cancelled() {
                None
            } else if selected.is_some_and(|v| v.eq_ignore_ascii_case(tag)) {
                Some(InUse::Current)
            } else if in_use(tag) {
                Some(InUse::Running)
            } else if cancel.is_cancelled() {
                None
            } else if native_mods::resource_dependent(tag).is_some() {
                Some(InUse::NativeBuild)
            } else if cancel.is_cancelled() {
                None
            } else if crate::managed_play::require_unreferenced_build(tag).is_err() {
                Some(InUse::ProfileReference)
            } else {
                None
            }
        });
        builds.push(Build {
            tag: tag.clone(),
            bytes: None,
            size_unavailable: None,
            in_use,
            old_download: old.contains(&tag),
        });
    }
    (!cancel.is_cancelled()).then_some(builds)
}

/// Fills in the sizes of `builds` one at a time, handing the rows to `filled` after each, and
/// stops at once when `cancel` is set. A build cut short keeps no size.
pub fn measure(builds: &mut [Build], cancel: &Cancel, mut filled: impl FnMut(&[Build])) {
    for index in 0..builds.len() {
        if cancel.is_cancelled() {
            return;
        }
        let row = &mut builds[index];
        let size = disk_use(&version_dir(&row.tag), cancel, true);
        if cancel.is_cancelled() || size == Err(SizeUnavailable::Cancelled) {
            return;
        }
        row.bytes = size.ok();
        row.size_unavailable = size.err();
        filled(builds);
    }
}

impl From<std::io::Error> for SizeUnavailable {
    fn from(error: std::io::Error) -> Self {
        if error.kind() == std::io::ErrorKind::NotFound {
            Self::Vanished
        } else {
            Self::Unreadable
        }
    }
}

#[cfg(test)]
type ListingHook = Box<dyn Fn(&Path)>;
#[cfg(test)]
thread_local! {
    static BEFORE_LISTING: std::cell::RefCell<Option<ListingHook>> = const { std::cell::RefCell::new(None) };
}

fn disk_use(path: &Path, cancel: &Cancel, root: bool) -> Result<u64, SizeUnavailable> {
    if cancel.is_cancelled() {
        return Err(SizeUnavailable::Cancelled);
    }
    let meta = path.symlink_metadata()?;
    // Prepared resources may be linked. Do not count files outside this build.
    if crate::native_cache::linked(&meta) {
        return if root {
            Err(SizeUnavailable::Unreadable)
        } else {
            Ok(0)
        };
    }
    if meta.is_dir() {
        let mut total = 0u64;
        #[cfg(test)]
        BEFORE_LISTING.with(|hook| {
            if let Some(hook) = &*hook.borrow() {
                hook(path);
            }
        });
        for entry in fs::read_dir(path)? {
            if cancel.is_cancelled() {
                return Err(SizeUnavailable::Cancelled);
            }
            let child = entry
                .map_err(SizeUnavailable::from)
                .and_then(|entry| disk_use(&entry.path(), cancel, false));
            match child {
                Ok(bytes) => total = total.saturating_add(bytes),
                Err(SizeUnavailable::Vanished) => (),
                Err(error) => return Err(error),
            }
        }
        // The directory iterator can finish successfully after the build is detached.
        if root {
            let current = path.symlink_metadata()?;
            if !current.is_dir() || crate::native_cache::linked(&current) {
                return Err(SizeUnavailable::Unreadable);
            }
        }
        Ok(total)
    } else if meta.is_file() && !root {
        Ok(meta.len())
    } else {
        Err(SizeUnavailable::Unreadable)
    }
}

/// Remove only the confirmed rows, rechecking protection and the bulk age filter on disk.
pub fn remove_builds(
    tags: &[String],
    releases: &[Release],
    old_only: bool,
) -> Vec<(String, String)> {
    tags.iter()
        .filter_map(|tag| {
            delete(tag, releases, false, old_only, false)
                .err()
                .map(|error| (tag.clone(), error))
        })
        .collect()
}

/// Requested and unmarked downloads are never eligible for automatic cleanup.
pub fn prune_old_versions(releases: &[Release], just_installed: &str) -> Vec<(String, String)> {
    let settings = Settings::load();
    if !settings.auto_prune() {
        return Vec::new();
    }
    old_downloads(&installed(), settings.keep_newest(), downloaded)
        .filter(|tag| tag.as_str() != just_installed && auto_downloaded(&version_dir(tag)))
        .filter_map(|tag| {
            delete(tag, releases, false, true, true)
                .err()
                .map(|error| (tag.clone(), error))
        })
        .collect()
}

/// CLI Remove may change the launcher selection, but cannot change a profile pointer.
pub fn remove_version(tag: &str) -> Result<(), String> {
    delete(tag, &[], true, false, false)?;
    let mut settings = Settings::load();
    if settings.selected.as_deref() == Some(tag) {
        settings.selected = installed().first().cloned();
        settings.save()?;
    }
    Ok(())
}

fn delete(
    tag: &str,
    releases: &[Release],
    allow_selected: bool,
    old_only: bool,
    automatic: bool,
) -> Result<(), String> {
    if !installed().iter().any(|v| v == tag) {
        return Err(trf("{tag} is not installed.", &[("tag", tag)]));
    }
    let path = version_dir(tag);
    let deletion = cache_cleanup::delete_version_checked(&path, || {
        let settings = Settings::load();
        let selected = settings.selected.as_ref();
        let (installed, nightly) = play_installations(selected.map(String::as_str));
        if !installed.iter().any(|v| v == tag)
            || (automatic && (!settings.auto_prune() || !auto_downloaded(&path)))
            || (old_only
                && !old_downloads(&installed, settings.keep_newest(), downloaded).any(|v| v == tag))
        {
            return Err(Error::IncompleteUnit);
        }
        let profiles = instances::list_for_storage().map_err(|_| Error::UnprovenReferences)?;
        if in_use(tag) {
            return Err(Error::Busy);
        }
        if (!allow_selected && selected.is_some_and(|v| v.eq_ignore_ascii_case(tag)))
            || profile_user(
                &profiles,
                &installed,
                selected,
                nightly.as_ref(),
                releases,
                tag,
            )
            .is_some()
        {
            return Err(Error::ReferencedProfile);
        }
        // The same fence separately checks committed pointers, pending First Play and leases.
        Ok(())
    });
    if automatic
        && matches!(
            deletion,
            Err(Error::IncompleteUnit
                | Error::Busy
                | Error::ReferencedProfile
                | Error::ResourceDependent(_))
        )
    {
        return Ok(());
    }
    deletion.map_err(|error| match error {
        Error::IncompleteUnit | Error::Busy => {
            tr("This build is in use or could not be safely removed.").into()
        }
        error => crate::storage::error_message(error),
    })?;
    super::forget_prepared();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolated(name: &str, run: impl FnOnce()) {
        const ACTOR: &str = "SUNRISE_TEST_STORAGE_BUILDS";
        if std::env::var_os(ACTOR).is_some() {
            run();
            return;
        }
        let scratch = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("backend::version_storage::tests::{name}"),
            ])
            .env(ACTOR, "1")
            .env("SUNRISE_LAUNCHER_HOME", scratch.path().join("launcher"))
            .env("SUNRISE_HOME", scratch.path().join("game"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    }

    fn install(tag: &str) -> std::path::PathBuf {
        let dir = version_dir(tag);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(super::super::COMPLETE), "").unwrap();
        fs::write(dir.join(super::super::CLIENT), "fixture").unwrap();
        dir
    }

    #[test]
    fn old_builds_count_each_channel_but_not_local_or_unknown_nightlies() {
        let installed = [
            "imported-2026-11-21-abcd",
            "native-local-build",
            "nightly-2026-11-21-aaaaaaa",
            "nightly-2026-11-20-bbbbbbb",
            "nightly-2026-11-19-ccccccc",
            "26.3-0.1.9",
            "26.3-0.1.8",
        ]
        .map(String::from);
        let old: Vec<_> = old_downloads(&installed, 1, |tag| tag != installed[3])
            .map(String::as_str)
            .collect();
        assert_eq!(old, ["nightly-2026-11-19-ccccccc", "26.3-0.1.8"]);
    }

    #[test]
    fn automatic_cleanup_only_removes_old_automatic_downloads_not_claimed_by_the_player() {
        isolated(
            "automatic_cleanup_only_removes_old_automatic_downloads_not_claimed_by_the_player",
            || {
                let newest = "26.3-0.1.9";
                let automatic = "26.3-0.1.8";
                let requested = "26.3-0.1.7";
                let legacy = "26.3-0.1.6";
                let unmarked = "26.3-0.1.5";
                install(newest);
                Download::Automatic.mark(&install(automatic)).unwrap();
                let dir = install(requested);
                Download::Automatic.mark(&dir).unwrap();
                Download::Requested.mark(&dir).unwrap();
                fs::write(install(legacy).join(LEGACY_DOWNLOADED), "").unwrap();
                install(unmarked);
                Settings {
                    keep_versions: Some(1),
                    ..Settings::default()
                }
                .save()
                .unwrap();
                assert!(prune_old_versions(&[], newest).is_empty());
                assert_eq!(installed(), [newest, requested, legacy, unmarked]);
            },
        );
    }

    #[test]
    fn confirming_cleanup_rechecks_a_new_pin_and_a_pending_first_play() {
        use crate::{
            managed_play::ReferencePublication,
            profile_txn::{Admission, Bootstrap},
        };
        isolated(
            "confirming_cleanup_rechecks_a_new_pin_and_a_pending_first_play",
            || {
                let newest = "26.3-0.1.9";
                let newly_pinned = "26.3-0.1.8";
                let pending = "26.3-0.1.7";
                let spare = "26.3-0.1.6";
                for tag in [newest, newly_pinned, pending, spare] {
                    install(tag);
                }
                Settings {
                    keep_versions: Some(1),
                    ..Settings::default()
                }
                .save()
                .unwrap();
                let confirmed: Vec<_> = inventory(&[])
                    .into_iter()
                    .filter(|row| row.old_download && row.in_use.is_none())
                    .map(|row| row.tag)
                    .collect();
                assert_eq!(confirmed, [newly_pinned, pending, spare]);
                let pin = instances::create("Pinned", None).unwrap();
                instances::edit(&pin.id, |meta| meta.version = Some(newly_pinned.into())).unwrap();
                let first = instances::create("First Play", None).unwrap();
                let mut admission = Admission::open(&first.home, None).unwrap();
                admission
                    .bind_bootstrap(
                        || {
                            Ok(Bootstrap::new(
                                pending.into(),
                                pending.into(),
                                [1; 16],
                                [2; 32],
                                serde_json::json!({}),
                            ))
                        },
                        |_| ReferencePublication::begin(&first.home),
                    )
                    .unwrap();
                drop(admission);
                let errors = remove_builds(&confirmed, &[], true);
                assert_eq!(
                    errors
                        .iter()
                        .map(|(tag, _)| tag.as_str())
                        .collect::<Vec<_>>(),
                    [newly_pinned, pending]
                );
                assert_eq!(installed(), [newest, newly_pinned, pending]);
            },
        );
    }

    #[test]
    fn sizes_fill_in_one_build_at_a_time_and_stop_when_cancelled() {
        isolated(
            "sizes_fill_in_one_build_at_a_time_and_stop_when_cancelled",
            || {
                for tag in ["26.3-0.1.9", "26.3-0.1.8"] {
                    install(tag);
                }
                let mut builds = inventory(&[]);
                assert!(
                    builds.iter().all(|build| build.bytes.is_none()),
                    "listing measures nothing"
                );
                let cancel = Cancel::default();
                let mut handed_over = 0;
                measure(&mut builds, &cancel, |_| {
                    handed_over += 1;
                    cancel.cancel();
                });
                assert_eq!(handed_over, 1);
                assert_eq!(
                    builds.iter().filter(|build| build.bytes.is_some()).count(),
                    1
                );
                let mut builds = inventory(&[]);
                measure(&mut builds, &Cancel::default(), |_| {});
                assert!(builds
                    .iter()
                    .all(|build| build.bytes.is_some_and(|bytes| bytes >= 7)));
            },
        );
    }

    #[test]
    fn missing_and_unreadable_builds_are_not_zero_byte_measurements() {
        let scratch = tempfile::tempdir().unwrap();
        let file = scratch.path().join("not-a-build");
        fs::write(&file, []).unwrap();
        let cancel = Cancel::default();
        assert_eq!(
            disk_use(&scratch.path().join("missing"), &cancel, true),
            Err(SizeUnavailable::Vanished)
        );
        assert_eq!(
            disk_use(&file, &cancel, true),
            Err(SizeUnavailable::Unreadable)
        );
        let child = scratch.path().join("swapped");
        fs::create_dir(&child).unwrap();
        BEFORE_LISTING.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |path| {
                if path == child {
                    fs::remove_dir(path).unwrap();
                    fs::write(path, []).unwrap();
                }
            }));
        });
        let result = disk_use(scratch.path(), &cancel, true);
        BEFORE_LISTING.with(|hook| hook.borrow_mut().take());
        assert_eq!(result, Err(SizeUnavailable::Unreadable));
        cancel.cancel();
        assert!(inventory_until(&[], &cancel).is_none());
        assert_eq!(
            disk_use(scratch.path(), &cancel, true),
            Err(SizeUnavailable::Cancelled)
        );
    }

    #[test]
    fn installed_build_discovery_stops_when_cancelled() {
        let scratch = tempfile::tempdir().unwrap();
        let versions = scratch.path().join("versions");
        for index in 0..8 {
            let dir = versions.join(format!("26.3-0.1.{index}"));
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(super::super::COMPLETE), "").unwrap();
        }
        let checks = std::cell::Cell::new(0usize);
        let result = super::super::installed_in_until(&versions, || {
            let count = checks.get();
            checks.set(count + 1);
            count >= 2
        });
        assert!(result.is_none());
        assert_eq!(
            checks.get(),
            3,
            "must stop inside the version discovery loop"
        );
    }

    #[test]
    fn never_played_stable_profile_protects_its_installed_build_without_a_catalogue() {
        isolated(
            "never_played_stable_profile_protects_its_installed_build_without_a_catalogue",
            || {
                let stable = "26.3-0.1.0";
                install(stable);
                let profile = instances::create("Stable", None).unwrap();
                instances::edit(&profile.id, |meta| {
                    meta.channel = Some(instances::Channel::Stable)
                })
                .unwrap();
                assert!(
                    matches!(inventory(&[])[0].in_use.as_ref(), Some(InUse::Profile(name)) if name == &profile.name)
                );
                assert_eq!(remove_builds(&[stable.into()], &[], false).len(), 1);
                assert_eq!(installed(), [stable]);
            },
        );
    }
}
