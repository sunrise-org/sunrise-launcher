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

pub(super) fn channel_name(channel: instances::Channel) -> &'static str {
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

/// What the build menu calls a channel: the newest build it publishes.
pub(super) fn latest_name(channel: instances::Channel) -> &'static str {
    match channel {
        instances::Channel::Stable => tr("Latest stable"),
        instances::Channel::Nightly => tr("Latest nightly"),
        instances::Channel::Dev => tr("Latest dev"),
    }
}

/// When a build was published, from its `YYYY-MM-DD` date and `HH:MM` UTC time (empty when
/// unknown): "13 minutes ago", "1 hour ago", "2 days ago" within a week, else "23/11/2026 16:42
/// UTC". Without a time it counts whole days: "today", "yesterday", "2 days ago", else the date.
pub(super) fn built_when(
    date: &str,
    time: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<String> {
    use chrono::{NaiveDate, NaiveTime};
    let day = NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    let count = |n: i64| n.to_string();
    let Ok(clock) = NaiveTime::parse_from_str(time, "%H:%M") else {
        return Some(match (now.date_naive() - day).num_days() {
            ..=0 => tr("today").into(),
            1 => tr("yesterday").into(),
            days @ 2..=6 => trf("{days} days ago", &[("days", &count(days))]),
            _ => day.format("%d/%m/%Y").to_string(),
        });
    };
    let at = day.and_time(clock).and_utc();
    let ago = now - at;
    Some(match ago.num_minutes() {
        // A clock a little behind the publisher's reads as now, not as the future.
        ..=0 => tr("just now").into(),
        1 => tr("1 minute ago").into(),
        minutes @ 2..=59 => trf("{count} minutes ago", &[("count", &count(minutes))]),
        _ => match ago.num_hours() {
            1 => tr("1 hour ago").into(),
            hours @ 2..=23 => trf("{count} hours ago", &[("count", &count(hours))]),
            _ => match ago.num_days() {
                1 => tr("1 day ago").into(),
                days @ 2..=6 => trf("{days} days ago", &[("days", &count(days))]),
                _ => at.format("%d/%m/%Y %H:%M UTC").to_string(),
            },
        },
    })
}

impl Launcher {
    /// When `tag` was published: the catalogue's time when it lists the build, else the date
    /// in a dated tag.
    pub(crate) fn build_when(&self, tag: &str) -> Option<String> {
        let now = chrono::Utc::now();
        match self.game_releases().iter().find(|r| r.tag == tag) {
            Some(release) => built_when(&release.date, &release.time, now),
            None => backend::dev_date(tag)
                .or_else(|| sunrise_launcher::nightly::date_of(tag))
                .and_then(|date| built_when(date, "", now)),
        }
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
