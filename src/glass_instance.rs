//! The instance dialog in the glass look (the mockup's `InstanceModal`, sunrise-site
//! `components/launcher/instance-modal.tsx`): one plate round at 28 over the dimmed page. On the
//! left, what the instance will be (its icon, name and what it plays) and its kind; on the right,
//! what it plays, its name and group, and its icon; Cancel and Create at the foot. Dark
//! appearance only: the light look keeps `instance_dialog`.
use super::*;

use sunrise_launcher::instances::Channel;

impl Launcher {
    /// A successful creation can queue its saved build, not the former dialog draft.
    pub(super) fn profile_creation_download(&self, saved: &instances::Instance) -> Option<Release> {
        let tag = creation_target(
            saved.version.as_ref(),
            saved.channel,
            self.game_releases(),
            self.launcher_version(),
        )?;
        self.release_to_install(tag)
    }

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
            self.launcher_version(),
        )
    }

    fn game_releases(&self) -> &[Release] {
        match &self.releases {
            Load::Ready(releases) => releases,
            _ => &[],
        }
    }

    /// The channel the profile actually follows, without changing its saved preference.
    pub(super) fn followed_channel(&self, channel: Channel) -> Channel {
        channel.following(self.game_releases())
    }

    /// The Stable channel's builds, newest first (none until the release list is here).
    pub(super) fn stable_builds(&self) -> impl Iterator<Item = &Release> {
        Channel::Stable.builds(self.game_releases())
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

/// Keep the saved pin; with no pin, Create uses the newest build for the followed channel.
fn creation_target<'a>(
    pin: Option<&'a String>,
    channel: Channel,
    releases: &'a [Release],
    launcher_version: Option<&'a String>,
) -> Option<&'a String> {
    pin.or_else(|| {
        channel
            .following(releases)
            .builds(releases)
            .next()
            .map(|release| &release.tag)
            .or(launcher_version)
    })
}

/// Pick one tag for both the Home picker and Play. A pin is never replaced by another
/// build just because it isn't installed yet. Channel release lists are newest first.
fn profile_target<'a>(
    pin: Option<&'a String>,
    channel: Channel,
    releases: &'a [Release],
    installed: &'a [String],
    launcher_version: Option<&'a String>,
) -> Option<&'a String> {
    pin.or_else(|| match channel.following(releases) {
        Channel::Stable => {
            // Keep the existing policy: prefer an installed Stable build.
            Channel::Stable
                .builds(releases)
                .find(|r| installed.contains(&r.tag))
                .or_else(|| Channel::Stable.builds(releases).next())
                .map(|r| &r.tag)
        }
        Channel::Nightly => Channel::Nightly
            .builds(releases)
            .next()
            .map(|r| &r.tag)
            .or(launcher_version),
    })
}

/// Every Nightly published with a build for this platform, newest first, then the installed
/// versions that list lacks (older Nightlies, imported builds). Picking one not installed installs
/// it on Create. Never a build made with mods (as on Versions): that is one profile's, made from
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
    /// A version as a player reads it: the Versions page's name for a build of the game ("Nov 13
    /// update", with its time when another came out that day; its commit stays in the details),
    /// "Imported · 2026-10-30", a developer build's name, or the tag. Players who don't know what
    /// a nightly is see the day it came out, not the channel's jargon.
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
        match sunrise_launcher::nightly::date_of(tag) {
            Some(_) => self.version_name(tag),
            None => build_import::label(tag).unwrap_or_else(|| tag.to_string()),
        }
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
    fn creation_download_uses_the_saved_copy_pin_instead_of_follow_latest() {
        let saved = "nightly-2026-11-16-aaaaaaa".to_owned();
        let newest = "nightly-2026-11-20-bbbbbbb";
        let releases = [release(newest, true, true), release(&saved, true, true)];
        assert_eq!(
            creation_target(Some(&saved), Channel::Nightly, &releases, None),
            Some(&saved)
        );
        // An unbound copy or empty creation can still follow the newest build.
        assert_eq!(
            creation_target(None, Channel::Nightly, &releases, None).map(String::as_str),
            Some(newest)
        );
    }

    #[test]
    fn nightly_follows_the_newest_download_for_this_platform_then_falls_back_offline() {
        let old = "nightly-2026-11-16-aaaaaaa".to_owned();
        let latest = "nightly-2026-11-18-ccccccc";
        let releases = [
            release("nightly-2026-11-19-ddddddd", true, false),
            release(latest, true, true),
            release(&old, true, true),
        ];
        let installed = [old.clone()];
        let target = |releases: &[Release]| {
            profile_target(None, Channel::Nightly, releases, &installed, Some(&old)).cloned()
        };
        assert_eq!(target(&releases).as_deref(), Some(latest));
        assert_eq!(target(&[]), Some(old));
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
                ..release("v1.1.0", false, true)
            },
            Release {
                prerelease: true,
                ..release("nightly-2026-11-16-bbbbbbb", true, true)
            },
        ];
        assert!(Channel::Stable.builds(&releases).next().is_none());
        assert_eq!(Channel::Nightly.builds(&releases).count(), 3);

        releases.push(release("v1.0.1", false, false));
        releases.push(release("v1.0.0", false, true));
        assert_eq!(
            Channel::Stable
                .builds(&releases)
                .map(|r| r.tag.as_str())
                .collect::<Vec<_>>(),
            ["v1.0.0"]
        );
    }

    #[test]
    fn a_stable_profile_follows_the_newest_nightly_until_a_stable_release_exists() {
        let stable = Channel::Stable;
        let mut releases = vec![
            release("nightly-2026-11-17-aaaaaaa", true, true),
            release("nightly-2026-11-16-bbbbbbb", true, true),
            Release {
                prerelease: true,
                ..release("v1.1.0", false, true)
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
        releases.push(release("v1.0.0", false, true));
        assert_eq!(plays(&releases).as_deref(), Some("v1.0.0"));
    }

    #[test]
    fn the_dialog_offers_every_published_nightly_then_what_only_this_computer_has() {
        let releases = [
            release("nightly-2026-11-03-aaaaaaa", true, true),
            // No build for this platform: nothing to install.
            release("nightly-2026-11-02-bbbbbbb", true, false),
            release("nightly-2026-11-01-ccccccc", true, true),
            // The old Alpha channel's releases aren't Nightlies.
            release("v0.2.1", false, true),
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
