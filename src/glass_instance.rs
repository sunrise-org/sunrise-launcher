//! The instance dialog in the glass look (the mockup's `InstanceModal`, sunrise-site
//! `components/launcher/instance-modal.tsx`): one plate round at 28 over the dimmed page. On the
//! left, what the instance will be (its icon, name and what it plays) and its kind; on the right,
//! what it plays, its name and group, and its icon; Cancel and Create at the foot. Dark
//! appearance only: the light look keeps `instance_dialog`.
use super::*;

use sunrise_launcher::instances::{profile_target, Channel};

impl Launcher {
    /// The Sunrise version an instance plays: the one it keeps, else the one it follows.
    pub(super) fn draft_sunrise(&self) -> Option<String> {
        let d = self.instances.dialog.as_ref()?;
        d.pin.clone().or_else(|| self.followed_version(d.channel))
    }

    /// Create installs the newest Stable build. Nightly uses the same target as Play.
    fn followed_version(&self, channel: Channel) -> Option<String> {
        match self.followed_channel(channel) {
            Channel::Stable => self.stable_builds().next().map(|b| b.tag.clone()),
            Channel::Nightly => self.target_version(None, Channel::Nightly).cloned(),
        }
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

    pub(super) fn game_releases(&self) -> &[Release] {
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

/// Every Nightly published with a build for this platform, newest first, then the installed
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
    /// A version as a player reads it: a build of the game ("Nov 13
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
            Some(date) => {
                let time = sunrise_launcher::nightly::same_day_time(tag).unwrap_or_default();
                trf("{date} update", &[("date", &published(date, &time))])
            }
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

/// A release's day as the mockup writes it ("Nov 1"), with its time in UTC when known.
pub(super) fn published(date: &str, time: &str) -> String {
    let day = sunrise_launcher::nightly::day(date);
    if time.is_empty() {
        day
    } else {
        trf("{day}, {time} UTC", &[("day", &day), ("time", time)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
