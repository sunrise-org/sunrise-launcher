//! The names a game build goes by: its channel, date or version, and when it was published.

use super::*;

/// A channel and version for game builds; the local tag otherwise.
pub(crate) fn build_name(tag: &str) -> String {
    // A game built with mods is named by the build it was made from.
    if native_mods::is_native(tag) {
        if let Some(base) = crate::profile_builds::native_base(tag) {
            return build_name(&base);
        }
    }
    channel_build(tag).map_or_else(
        || tag.to_owned(),
        |(channel, version)| {
            trf(
                "{channel} · {version}",
                &[("channel", channel_name(channel)), ("version", &version)],
            )
        },
    )
}

pub(crate) fn channel_name(channel: instances::Channel) -> &'static str {
    match channel {
        instances::Channel::Nightly => tr("Nightly"),
        instances::Channel::Stable => tr("Stable"),
        instances::Channel::Dev => tr("Dev"),
    }
}

/// [`build_name`] with a dated build's short commit, which tells apart two builds of one day.
pub(crate) fn build_name_with_commit(tag: &str) -> String {
    let name = build_name(tag);
    match dated_channel(tag).and(tag.rsplit_once('-')) {
        Some((_, commit)) => trf(
            "{build} ({commit})",
            &[("build", &name), ("commit", commit)],
        ),
        None => name,
    }
}

/// The channel name of a dated Nightly or Dev build, which Home's picker shows with its age.
pub(crate) fn dated_channel(tag: &str) -> Option<&'static str> {
    channel_build(tag)
        .filter(|(channel, _)| *channel != instances::Channel::Stable)
        .map(|(channel, _)| channel_name(channel))
}

impl Launcher {
    /// How old the build `tag` is, as the profile banner says it: "built today", "1 day old",
    /// "6 days old". `None` when its date isn't known.
    pub(crate) fn build_age(&self, tag: &str) -> Option<String> {
        let date = match self.game_releases().iter().find(|r| r.tag == tag) {
            Some(release) => release.date.clone(),
            None => backend::dev_date(tag)
                .or_else(|| sunrise_launcher::nightly::date_of(tag))?
                .to_string(),
        };
        let day = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d").ok()?;
        Some(match (chrono::Utc::now().date_naive() - day).num_days() {
            ..=0 => tr("built today").into(),
            1 => tr("1 day old").into(),
            days => trf("{days} days old", &[("days", &days.to_string())]),
        })
    }
}

/// The channel an unpinned profile plays now: a Stable profile plays Nightly until Stable has a
/// build ([`instances::play_target`]), and the picker marks what plays.
pub(super) fn following_now(
    saved: instances::Channel,
    waits_for_stable: bool,
) -> instances::Channel {
    if waits_for_stable {
        instances::Channel::Nightly
    } else {
        saved
    }
}

/// The version part of the build label, shared with the picker row whose heading is its channel.
pub(super) fn channel_build(tag: &str) -> Option<(instances::Channel, String)> {
    use instances::Channel;
    use sunrise_launcher::nightly::{date_of, day};
    if let Some(version) = backend::stable_version(tag) {
        Some((Channel::Stable, version.to_owned()))
    } else if let Some(date) = backend::dev_date(tag) {
        Some((Channel::Dev, day(date)))
    } else {
        date_of(tag).map(|date| (Channel::Nightly, day(date)))
    }
}
