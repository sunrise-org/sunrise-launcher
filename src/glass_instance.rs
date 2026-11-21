//! The instance dialog in the glass look (the mockup's `InstanceModal`, sunrise-site
//! `components/launcher/instance-modal.tsx`): one plate round at 28 over the dimmed page. On the
//! left, what the instance will be (its icon, name and what it plays) and its kind; on the right,
//! what it plays, its name and group, and its icon; Cancel and Create at the foot. Dark
//! appearance only: the light look keeps `instance_dialog`.
use super::*;

use std::path::Path;
use sunrise_launcher::{
    instances::Channel,
    managed_play::{display_active, Active},
};

impl Launcher {
    /// Queue the saved profile's Play target, not the former dialog draft.
    pub(super) fn profile_creation_download(&self, saved: &instances::Instance) -> Option<Release> {
        let tag = self.target_version(saved.version.as_ref(), saved.channel)?;
        self.release_to_install(tag)
    }

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
        Channel::Stable.newest_installed(&self.installed).is_some()
            || Channel::Stable
                .builds(self.game_releases())
                .next()
                .is_some()
    }

    /// For a Stable profile, exact choices come from both the catalogue and the player's disk.
    pub(super) fn stable_choices(&self) -> Vec<String> {
        let mut tags: Vec<_> = Channel::Stable
            .builds(self.game_releases())
            .map(|r| r.tag.clone())
            .collect();
        for tag in &self.installed {
            if Channel::of_build(tag) == Some(Channel::Stable) && !tags.contains(tag) {
                tags.push(tag.clone());
            }
        }
        tags.sort_by(|a, b| backend::compare_game_versions(b, a));
        tags
    }

    /// The Sunrise versions the dialog offers (the mockup's `data.nightlies`): see
    /// [`sunrise_choices`].
    pub(super) fn sunrise_choices(&self) -> Vec<String> {
        sunrise_choices(self.game_releases(), &self.installed)
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
    if profile.kind == instances::Kind::Pack {
        return None;
    }
    // Compare only a settled base, never a legacy pin, derived executable, or cache guess.
    let active = read_active(&profile.home).ok()??;
    let base = active.base();
    let release = profile.channel.builds(releases).next()?;
    let base_channel = Channel::of_build(base)?;
    // Stable may have started on Nightly while no Stable was published. Once a Stable
    // release is available, offer that channel directly without comparing version formats.
    if (profile.channel, base_channel) == (Channel::Stable, Channel::Nightly) {
        return Some(release);
    }
    if release.tag == base || base_channel != profile.channel {
        return None;
    }
    let newer = backend::compare_game_versions(&release.tag, base);
    match profile.channel {
        Channel::Stable => newer.is_gt(),
        Channel::Nightly => newer.is_ge(),
    }
    .then_some(release)
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
    pin.or_else(|| channel.play_target(releases, installed, nightly, launcher_version))
}

/// The current Nightly if published for this platform, then the installed
/// versions that list lacks (older Nightlies, imported builds). Picking one not installed installs
/// it on Create. Never a build made with mods: that is one profile's, made from
/// its mods with the player's permission, not a game to start another profile on.
pub(super) fn sunrise_choices(releases: &[Release], installed: &[String]) -> Vec<String> {
    let published = Channel::Nightly.builds(releases).map(|r| &r.tag);
    let games = installed.iter().filter(|t| !native_mods::is_native(t));
    let mut out: Vec<String> = Vec::new();
    for tag in published.chain(games) {
        if !out.contains(tag) {
            out.push(tag.clone());
        }
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
            Channel::Stable
                .play_target(&releases, installed, None, None)
                .cloned()
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
    fn home_offers_stable_but_not_a_new_nightly_to_a_committed_stable_fallback() {
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
        assert_eq!(offer(&releases, fallback).as_deref(), Some(stable));
        assert_eq!(offer(&releases, "imported-2026-11-18"), None);
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
    fn creation_download_uses_the_saved_copy_pin_instead_of_follow_latest() {
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
        let mut with_native = installed.to_vec();
        with_native.insert(0, "native-aaaaaaa-0123456789abcdef".to_string());
        assert!(native_mods::is_native(&with_native[0]));
        assert_eq!(
            sunrise_choices(&releases, &with_native),
            sunrise_choices(&releases, &installed)
        );
        assert_eq!(sunrise_choices(&[], &with_native), installed);
        assert_eq!(
            sunrise_choices(&releases, &installed),
            [
                "nightly-2026-11-03-aaaaaaa",
                "nightly-2026-11-01-ccccccc",
                "nightly-2026-10-20-ddddddd",
                "imported-2026-10-30",
            ]
        );
        // Offline, or before the list arrives: what is installed.
        assert_eq!(sunrise_choices(&[], &installed), installed);
    }
}
