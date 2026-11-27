//! The builds a profile plays and can pick: its play target and the release to install for it,
//! the build choices its dialog offers, Home's update offer, and how a build is named.
use super::*;

use std::path::Path;
use sunrise_launcher::{
    instances::Channel,
    managed_play::{display_active, Active},
};

impl Launcher {
    /// The exact build the profile will play, including an explicit pin.
    pub(super) fn target_version<'a>(
        &'a self,
        pin: Option<&'a String>,
        channel: Channel,
    ) -> Option<&'a String> {
        profile_target(
            pin,
            channel,
            self.game_releases(),
            &self.installed,
            self.known_nightly.as_ref(),
            self.launcher_version(),
        )
    }

    pub(crate) fn game_releases(&self) -> &[Release] {
        match &self.releases {
            Load::Ready(releases) => releases,
            _ => &[],
        }
    }

    /// The Home offer and the picker opened by its Update action use the same decision.
    pub(crate) fn game_update(&self) -> Option<&Release> {
        let profile = self.selected_instance()?;
        home_update(profile, self.game_releases(), display_active)
    }

    /// Stable can follow an installed build even when its release feed is unavailable.
    pub(super) fn stable_available(&self) -> bool {
        instances::newest_installed(Channel::Stable, &self.installed).is_some()
            || Channel::Stable
                .builds(self.game_releases())
                .next()
                .is_some()
    }

    /// Exact choices include builds still on disk even after their releases are pruned.
    pub(super) fn profile_build_choices(&self, channel: Channel) -> Vec<String> {
        profile_build_choices(channel, self.game_releases(), &self.installed)
    }

    /// The release to download for the Sunrise version `tag`, when it isn't installed.
    pub(super) fn release_to_install(&self, tag: &str) -> Option<Release> {
        let Load::Ready(releases) = &self.releases else {
            return None;
        };
        (!self.installed.iter().any(|t| t == tag))
            .then(|| releases.iter().find(|r| r.tag == tag && r.asset.is_some()))
            .flatten()
            .cloned()
    }
}

fn home_update<'a>(
    profile: &instances::Instance,
    releases: &'a [Release],
    read_active: impl FnOnce(&Path) -> Result<Option<Active>, String>,
) -> Option<&'a Release> {
    home_update_with(
        profile,
        releases,
        read_active,
        sunrise_launcher::nightly::published_at,
    )
}

fn home_update_with<'a>(
    profile: &instances::Instance,
    releases: &'a [Release],
    read_active: impl FnOnce(&Path) -> Result<Option<Active>, String>,
    remembered_published: impl FnOnce(&str) -> Option<String>,
) -> Option<&'a Release> {
    if profile.kind == instances::Kind::Pack {
        return None;
    }
    // Compare only a settled base, never a legacy pin, derived executable, or cache guess.
    let active = read_active(&profile.home).ok()??;
    let base = active.base();
    let channel = profile.channel;
    if Channel::of_build(base) != Some(channel) {
        return None;
    }
    let release = channel.builds(releases).next()?;
    match backend::compare_game_versions(&release.tag, base) {
        std::cmp::Ordering::Greater => Some(release),
        std::cmp::Ordering::Equal if channel != Channel::Stable && release.tag != base => {
            // Different commits on the same day are not necessarily newer. Compare full
            // publication timestamps when available, including Nightly's saved timestamp for
            // the previous rolling build, falling back to HH:MM only when full timestamps
            // are absent.
            let old_published = releases
                .iter()
                .find(|row| row.tag == base && !row.published_at.is_empty())
                .map(|row| row.published_at.clone())
                .or_else(|| remembered_published(base));
            if let (Some(new_ts), Some(old_ts)) = (
                parse_published_at(&release.published_at),
                old_published.as_deref().and_then(parse_published_at),
            ) {
                return (new_ts > old_ts).then_some(release);
            }
            if release.time.is_empty() {
                return None;
            }
            let nightly_time = sunrise_launcher::nightly::same_day_time(base);
            let old_time = releases
                .iter()
                .find(|row| row.tag == base && !row.time.is_empty())
                .map(|row| row.time.as_str())
                .or(nightly_time.as_deref())?;
            (release.time.as_str() > old_time).then_some(release)
        }
        _ => None,
    }
}

fn parse_published_at(value: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    (!value.is_empty())
        .then(|| chrono::DateTime::parse_from_rfc3339(value).ok())
        .flatten()
}

/// Pick one tag for both the Home picker and Play. A pin is never replaced by another
/// build just because it isn't installed yet. Channel release lists are newest first.
fn profile_target<'a>(
    pin: Option<&'a String>,
    channel: Channel,
    releases: &'a [Release],
    installed: &'a [String],
    nightly: Option<&'a String>,
    launcher_version: Option<&'a String>,
) -> Option<&'a String> {
    pin.or_else(|| instances::play_target(channel, releases, installed, nightly, launcher_version))
}

/// The channel's published builds, plus installed ones. Nightly also offers legacy and imported
/// builds as before, but never Dev: using Dev starts with choosing that channel.
fn profile_build_choices(
    channel: Channel,
    releases: &[Release],
    installed: &[String],
) -> Vec<String> {
    let published = channel.builds(releases).map(|r| &r.tag);
    let games = installed.iter().filter(|tag| {
        let installed_channel = Channel::of_build(tag);
        if channel == Channel::Nightly {
            installed_channel != Some(Channel::Dev) && !native_mods::is_native(tag)
        } else {
            installed_channel == Some(channel)
        }
    });
    let mut out: Vec<String> = Vec::new();
    for tag in published.chain(games) {
        if !out.contains(tag) {
            out.push(tag.clone());
        }
    }
    if channel != Channel::Nightly {
        out.sort_by(|a, b| backend::compare_game_versions(b, a));
    }
    out
}

impl Launcher {
    /// The channel and version, or a local build’s name.
    pub(super) fn version_label(&self, tag: &str) -> String {
        if let Some(developer) = sunrise_launcher::developer_install::label(tag) {
            return developer;
        }
        // A game built with mods reads as what it was built on: "Nov 13 update, with your mods".
        if native_mods::is_native(tag) {
            return match native_base(tag) {
                Some(base) => trf(
                    "{version}, with your mods",
                    &[("version", &self.version_label(&base))],
                ),
                None => tr("A build with your mods").to_string(),
            };
        }
        build_import::label(tag).unwrap_or_else(|| crate::sheets::build_name(tag))
    }
}

/// The release a native build started from, read once per build: a new build has a new tag.
pub(crate) fn native_base(tag: &str) -> Option<String> {
    thread_local! {
        static BASES: std::cell::RefCell<std::collections::HashMap<String, Option<String>>> =
            Default::default();
    }
    BASES.with(|bases| {
        bases
            .borrow_mut()
            .entry(tag.to_string())
            .or_insert_with(|| native_mods::about(tag).map(|about| about.base))
            .clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_offers_the_update_to_the_committed_older_profile_even_after_it_is_cached() {
        let old = "26.3-0.1.0";
        let new = "26.3-0.1.1";
        let releases = [release(new, false, true), release(old, false, true)];
        // The legacy pin already names the new release, but the running base is still old.
        let mut older = home_profile("older", Channel::Stable, Some(new));
        let current = home_profile("current", Channel::Stable, None);
        let older_base = active(old, "native-aaaaaaa-0123456789abcdef");
        let current_base = active(new, new);
        let before = [old.to_owned()];
        let after = [new.to_owned(), old.to_owned()];
        let offer = |profile: &instances::Instance| {
            home_update(profile, &releases, |home| {
                assert_eq!(home, profile.home.as_path());
                Ok(Some(if home == current.home.as_path() {
                    current_base.clone()
                } else {
                    older_base.clone()
                }))
            })
            .map(|release| release.tag.clone())
        };
        let cache_target = |installed: &[String]| {
            instances::play_target(Channel::Stable, &releases, installed, None, None).cloned()
        };
        assert_eq!(cache_target(&before).as_deref(), Some(old));
        assert_eq!(offer(&older).as_deref(), Some(new));
        // The unbound cache target changes, not the committed profile's offer.
        assert_eq!(cache_target(&after).as_deref(), Some(new));
        assert_eq!(offer(&older).as_deref(), Some(new));
        older.version = None;
        assert_eq!(offer(&older).as_deref(), Some(new));
        assert_eq!(offer(&current), None);
    }

    #[test]
    fn home_does_not_offer_an_update_to_a_committed_stable_fallback() {
        let fallback = "nightly-2026-11-18-aaaaaaa";
        let stable = "26.3-0.1.0";
        let profile = home_profile("fallback", Channel::Stable, None);
        let releases = [
            release("nightly-2026-11-20-bbbbbbb", true, true),
            release("26.3-0.1.1", false, false),
            release(stable, false, true),
        ];
        let offer = |releases: &[Release], base: &str| {
            home_update(&profile, releases, |_| Ok(Some(active(base, base))))
                .map(|release| release.tag.clone())
        };
        assert_eq!(offer(&releases[..2], fallback), None);
        assert_eq!(offer(&releases, fallback), None);
        assert_eq!(offer(&releases, "imported-2026-11-18"), None);
    }

    #[test]
    fn home_offers_dev_only_within_an_active_dev_profile() {
        let stable = "26.3-0.1.0";
        let nightly = "nightly-2026-11-20-ccccccc";
        let old_dev = "dev-2026-11-20-aaaaaaa";
        let new_dev = "dev-2026-11-21-bbbbbbb";
        let releases = [
            Release {
                prerelease: true,
                nightly: Some(new_dev.into()),
                ..release(new_dev, false, true)
            },
            release(nightly, true, true),
            release(stable, false, true),
        ];
        let offer = |channel, base: &str, pin, with_stable| {
            let profile = home_profile("profile", channel, pin);
            let offers = if with_stable {
                &releases[..]
            } else {
                &releases[..2]
            };
            home_update(&profile, offers, |_| Ok(Some(active(base, base))))
                .map(|release| release.tag.as_str())
        };
        assert_eq!(offer(Channel::Stable, stable, None, true), None);
        assert_eq!(offer(Channel::Dev, stable, None, true), None);
        assert_eq!(offer(Channel::Dev, stable, Some(stable), true), None);
        assert_eq!(offer(Channel::Dev, old_dev, None, true), Some(new_dev));
        assert_eq!(offer(Channel::Stable, new_dev, None, true), None);
        assert_eq!(offer(Channel::Nightly, new_dev, None, true), None);
        assert_eq!(offer(Channel::Stable, new_dev, None, false), None);
        // A feed with only Dev still cannot offer an update to either ordinary channel.
        for (channel, base) in [(Channel::Stable, stable), (Channel::Nightly, nightly)] {
            let profile = home_profile("profile", channel, None);
            assert!(
                home_update(&profile, &releases[..1], |_| Ok(Some(active(base, base)))).is_none()
            );
        }
    }

    #[test]
    fn home_offers_a_same_day_build_only_when_it_was_published_later() {
        let old = "dev-2026-11-22-aaaaaaa";
        let other = "dev-2026-11-22-bbbbbbb";
        let profile = home_profile("dev", Channel::Dev, None);
        let dev = |tag: &str, time: &str, published_at: &str| Release {
            prerelease: true,
            nightly: Some(tag.into()),
            time: time.into(),
            published_at: published_at.into(),
            ..release(tag, false, true)
        };
        let offer = |releases: &[Release]| {
            home_update(&profile, releases, |_| Ok(Some(active(old, old))))
                .map(|release| release.tag.clone())
        };
        let previous = dev(old, "12:30", "2026-11-22T12:30:10Z");
        assert_eq!(
            offer(&[
                dev(other, "12:20", "2026-11-22T12:20:00Z"),
                previous.clone()
            ]),
            None
        );
        assert_eq!(
            offer(&[
                dev(other, "12:30", "2026-11-22T12:30:05Z"),
                previous.clone()
            ]),
            None
        );
        assert_eq!(
            offer(&[
                dev(other, "12:30", "2026-11-22T12:30:10Z"),
                previous.clone()
            ]),
            None
        );
        assert_eq!(offer(&[dev(other, "12:40", "2026-11-22T12:40:00Z")]), None);
        // Two Dev builds in the same minute still order by full publication timestamp.
        assert_eq!(
            offer(&[
                dev(other, "12:30", "2026-11-22T12:30:45Z"),
                previous.clone()
            ])
            .as_deref(),
            Some(other)
        );
        assert_eq!(
            offer(&[dev(other, "12:40", "2026-11-22T12:40:00Z"), previous]).as_deref(),
            Some(other)
        );
    }

    #[test]
    fn home_offers_a_same_minute_rolling_nightly_using_its_remembered_timestamp() {
        let old = "nightly-2035-08-09-aaaaaaa";
        let newer = "nightly-2035-08-09-bbbbbbb";
        let profile = home_profile("nightly", Channel::Nightly, None);
        let nightly = |tag: &str, published_at: &str| Release {
            time: "12:30".into(),
            published_at: published_at.into(),
            ..release(tag, true, true)
        };
        let first = nightly(old, "2035-08-09T12:30:10Z");
        let second = nightly(newer, "2035-08-09T12:30:45Z");
        let mut table = sunrise_launcher::nightly::PublishedTimestamps::default();
        table.note(&[first]);
        table.note(std::slice::from_ref(&second));
        assert_eq!(
            home_update_with(
                &profile,
                &[second],
                |_| Ok(Some(active(old, old))),
                |tag| table.get(tag),
            )
            .map(|release| release.tag.as_str()),
            Some(newer)
        );
    }

    #[test]
    fn home_update_requires_a_settled_ordinary_profile() {
        let releases = [release("26.3-0.1.1", false, true)];
        let mut profile = home_profile("unbound", Channel::Stable, None);
        assert!(home_update(&profile, &releases, |_| Ok(None)).is_none());
        assert!(home_update(&profile, &releases, |_| Err("Unsettled".into())).is_none());

        profile.version = Some("26.3-0.1.0".into());
        assert!(home_update(&profile, &releases, |_| Ok(None)).is_none());

        profile.kind = instances::Kind::Pack;
        assert!(home_update(&profile, &releases, |_| panic!("Pack is fenced")).is_none());
    }

    fn home_profile(id: &str, channel: Channel, pin: Option<&str>) -> instances::Instance {
        instances::Instance {
            id: id.into(),
            name: id.into(),
            kind: instances::Kind::Own,
            work: id.into(),
            home: id.into(),
            incarnation: None,
            icon: None,
            group: None,
            version: pin.map(String::from),
            channel,
            native_profile: Default::default(),
            played: Duration::ZERO,
            last_played: None,
            launch: Default::default(),
        }
    }

    // The display decision needs a parsed pointer, not an executable or prepared resources.
    fn active(base: &str, executable: &str) -> Active {
        let digest = [0_u8; 32];
        let profile = [1_u8; 16];
        let artifact =
            |tag: &str| serde_json::json!({ "tag": tag, "executable": digest, "runtime": digest });
        let pointer = serde_json::json!({
            "version": 1,
            "profile": profile,
            "generation": 1,
            "base": artifact(base),
            "executable": artifact(executable),
            "resources": { "primary": digest },
            "inputs": digest,
            "receipts": [],
        });
        Active::parse(&serde_json::to_vec(&pointer).unwrap()).unwrap()
    }

    #[test]
    fn a_new_profile_targets_its_saved_pin_instead_of_follow_latest() {
        let saved = "nightly-2026-11-16-aaaaaaa".to_owned();
        let newest = "nightly-2026-11-20-bbbbbbb";
        let releases = [release(newest, true, true), release(&saved, true, true)];
        let target = |pin| profile_target(pin, Channel::Nightly, &releases, &[], None, None);
        assert_eq!(target(Some(&saved)), Some(&saved));
        // With no installed build, an unbound copy or empty creation can follow the newest.
        assert_eq!(target(None).map(String::as_str), Some(newest));
    }

    #[test]
    fn play_keeps_the_installed_nightly_until_an_explicit_update() {
        let old = "nightly-2026-11-16-aaaaaaa".to_owned();
        let latest = "nightly-2026-11-18-ccccccc";
        let releases = [
            release("nightly-2026-11-19-ddddddd", true, false),
            release(latest, true, true),
            release(&old, true, true),
        ];
        let installed = [old.clone()];
        let target = |releases: &[Release]| {
            profile_target(
                None,
                Channel::Nightly,
                releases,
                &installed,
                Some(&old),
                Some(&old),
            )
            .cloned()
        };
        assert_eq!(target(&releases), Some(old.clone()));
        assert_eq!(
            profile_target(None, Channel::Nightly, &releases, &[], None, None).map(String::as_str),
            Some(latest)
        );
        assert_eq!(target(&[]), Some(old));
    }

    #[test]
    fn channel_less_home_never_uses_stable_as_its_offline_fallback() {
        let stable = "26.3-0.1.0".to_owned();
        let nightly = "nightly-2026-11-18-ccccccc";
        let installed = [stable.clone()];
        let releases = [release(nightly, true, true)];
        let channel = Channel::default();
        let target =
            |pin, releases| profile_target(pin, channel, releases, &installed, None, Some(&stable));
        assert_eq!(channel, Channel::Nightly);
        assert_eq!(target(None, &[]), None);
        assert_eq!(target(None, &releases).map(String::as_str), Some(nightly));
        assert_eq!(target(Some(&stable), &[]), Some(&stable));
        assert_eq!(
            profile_target(None, Channel::Stable, &releases, &[], None, None).map(String::as_str),
            Some(nightly)
        );

        let legacy = "v2026.10.13".to_owned();
        assert_eq!(
            profile_target(
                None,
                channel,
                &[],
                std::slice::from_ref(&legacy),
                None,
                Some(&legacy)
            ),
            Some(&legacy)
        );
    }

    #[test]
    fn a_pin_names_the_play_target_even_before_installing_it() {
        let pin = "nightly-2026-11-17-bbbbbbb".to_owned();
        let old = "nightly-2026-11-16-aaaaaaa".to_owned();
        let releases = [release("nightly-2026-11-18-ccccccc", true, true)];
        assert_eq!(
            profile_target(
                Some(&pin),
                Channel::Nightly,
                &releases,
                std::slice::from_ref(&old),
                Some(&old),
                Some(&old)
            ),
            Some(&pin),
        );
    }

    fn release(tag: &str, nightly: bool, asset: bool) -> Release {
        Release {
            tag: tag.into(),
            name: String::new(),
            body: String::new(),
            date: String::new(),
            time: String::new(),
            published_at: String::new(),
            asset: asset.then(|| ("sunrise.tar.gz".into(), 1)),
            prerelease: false,
            nightly: nightly.then(|| "nightly".into()),
        }
    }

    #[test]
    fn stable_requires_a_versioned_non_prerelease_for_this_platform() {
        let mut releases = vec![
            release("nightly", false, true),
            // The rolling release after catalogue normalization. Its flag is false so updates
            // can follow it, not because the build is stable.
            release("nightly-2026-11-17-aaaaaaa", true, true),
            Release {
                prerelease: true,
                ..release("26.3-0.1.2", false, true)
            },
            Release {
                prerelease: true,
                nightly: Some("nightly-2026-11-16".into()),
                ..release("nightly-2026-11-16-bbbbbbb", true, true)
            },
        ];
        assert!(Channel::Stable.builds(&releases).next().is_none());
        assert_eq!(Channel::Nightly.builds(&releases).count(), 1);

        releases.push(release("26.3-0.1.1", false, false));
        releases.push(release("26.3-0.1.0", false, true));
        assert_eq!(
            Channel::Stable
                .builds(&releases)
                .map(|r| r.tag.as_str())
                .collect::<Vec<_>>(),
            ["26.3-0.1.0"]
        );
    }

    #[test]
    fn a_stable_profile_follows_the_newest_nightly_until_a_stable_release_exists() {
        let stable = Channel::Stable;
        let mut releases = vec![
            release("nightly-2026-11-17-aaaaaaa", true, true),
            Release {
                prerelease: true,
                ..release("26.3-0.1.2", false, true)
            },
        ];
        let plays = |releases: &[Release]| {
            stable
                .following(releases)
                .builds(releases)
                .next()
                .map(|r| r.tag.clone())
        };
        assert_eq!(
            plays(&releases).as_deref(),
            Some("nightly-2026-11-17-aaaaaaa")
        );
        releases.push(release("26.3-0.1.0", false, true));
        assert_eq!(plays(&releases).as_deref(), Some("26.3-0.1.0"));
    }

    #[test]
    fn the_dialog_offers_the_current_nightly_then_installed_builds() {
        let releases = [
            release("nightly-2026-11-03-aaaaaaa", true, true),
            // Stable has its own publication channel.
            release("26.3-0.1.0", false, true),
        ];
        let installed = [
            "nightly-2026-11-01-ccccccc".to_string(),
            "nightly-2026-10-20-ddddddd".to_string(),
            "imported-2026-10-30".to_string(),
        ];
        // A build made with mods, even one left on disk after the permission went, is not
        // offered.
        let choices = |releases: &[Release], installed: &[String]| {
            profile_build_choices(Channel::Nightly, releases, installed)
        };
        let mut with_native = installed.to_vec();
        with_native.insert(0, "native-aaaaaaa-0123456789abcdef".to_string());
        with_native.push("dev-2026-11-01-aaaaaaa".into());
        assert!(native_mods::is_native(&with_native[0]));
        assert_eq!(
            choices(&releases, &with_native),
            choices(&releases, &installed)
        );
        assert_eq!(choices(&[], &with_native), installed);
        assert_eq!(
            choices(&releases, &installed),
            [
                "nightly-2026-11-03-aaaaaaa",
                "nightly-2026-11-01-ccccccc",
                "nightly-2026-10-20-ddddddd",
                "imported-2026-10-30",
            ]
        );
        // Offline, or before the list arrives: what is installed.
        assert_eq!(choices(&[], &installed), installed);
    }

    #[test]
    fn dev_profile_keeps_its_installed_build_after_pruning() {
        let newest = "dev-2026-11-21-aaaaaaa".to_owned();
        let pruned = "dev-2026-11-20-bbbbbbb".to_owned();
        let nightly = "nightly-2026-11-20-ccccccc".to_owned();
        let legacy = "v2026.10.13".to_owned();
        let other_builds = [nightly.clone(), "26.3-0.1.1".into(), legacy.clone()];
        let mut installed = other_builds.to_vec();
        installed.push(pruned.clone());
        let releases = [
            release(&nightly, true, true),
            Release {
                prerelease: true,
                nightly: Some(newest.clone()),
                ..release(&newest, false, true)
            },
            release("26.3-0.1.1", false, true),
        ];
        let play = |releases: &[Release], installed: &[String]| {
            profile_target(
                None,
                Channel::Dev,
                releases,
                installed,
                Some(&nightly),
                Some(&legacy),
            )
            .cloned()
        };
        assert_eq!(play(&releases, &other_builds), Some(newest.clone()));
        assert_eq!(play(&releases, &installed), Some(pruned.clone()));
        assert_eq!(play(&releases[..1], &installed), Some(pruned.clone()));
        assert_eq!(play(&[], &other_builds), None);
        assert_eq!(
            profile_target(
                Some(&pruned),
                Channel::Dev,
                &releases,
                &installed,
                None,
                None
            ),
            Some(&pruned)
        );
        assert_eq!(
            profile_build_choices(Channel::Dev, &releases, &installed),
            [newest, pruned]
        );
    }
}
