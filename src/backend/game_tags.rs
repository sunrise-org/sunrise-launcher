//! Game releases have their own tags. The launcher's `vX.Y.Z` feed is separate.
use crate::nightly;
use std::cmp::Ordering;
use sunrise_launcher_release_verify::NIGHTLY;

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum GameOrder<'a> {
    Other,
    Stable(Vec<u64>, [u64; 3]),
    Nightly(&'a str),
    CurrentNightly,
}

fn stable(tag: &str) -> Option<(GameOrder<'_>, &str)> {
    let (game, version) = tag.split_once('-')?;
    let game = game
        .split('.')
        .map(|n| {
            if !n.bytes().all(|digit| digit.is_ascii_digit()) {
                return None;
            }
            n.parse::<u64>().ok()
        })
        .collect::<Option<Vec<_>>>()?;
    let semver = semver::Version::parse(version).ok()?;
    if game.len() < 2 || !semver.pre.is_empty() || !semver.build.is_empty() {
        return None;
    }
    Some((
        GameOrder::Stable(game, [semver.major, semver.minor, semver.patch]),
        version,
    ))
}

/// The release version of a stable game tag, separate from the Minecraft version before it.
pub fn stable_version(tag: &str) -> Option<&str> {
    stable(tag).map(|(_, version)| version)
}

pub fn is_game_tag(tag: &str) -> bool {
    stable(tag).is_some()
}

/// Catalogue / installed display order, not an update policy. Old installed builds remain
/// visible but are not interpreted as historical dates or semver.
pub fn compare_game_versions(a: &str, b: &str) -> Ordering {
    fn order(tag: &str) -> GameOrder<'_> {
        if tag == NIGHTLY {
            GameOrder::CurrentNightly
        } else if let Some(date) = nightly::date_of(tag) {
            GameOrder::Nightly(date)
        } else {
            stable(tag).map_or(GameOrder::Other, |(order, _)| order)
        }
    }
    order(a).cmp(&order(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_game_tag_has_two_numbered_versions_and_no_suffix() {
        for (tag, version) in [("26.3-0.1.0", "0.1.0"), ("26.3.1-1.40.2", "1.40.2")] {
            assert_eq!(stable_version(tag), Some(version));
            assert!(is_game_tag(tag));
        }
        for tag in [
            "v2026.10.13",
            "v1.1.3",
            "nightly",
            "26.3-0.1.0-rc.1",
            "26.3-0.1.0+build",
            "26.3-0.01.0",
            "26.3-0.1",
            "26.3-0.1.0.0",
            "26-0.1.0",
            "26..3-0.1.0",
            "26.a-0.1.0",
            "26.+3-0.1.0",
            " 26.3-0.1.0",
        ] {
            assert!(!is_game_tag(tag), "{tag}");
        }
    }

    #[test]
    fn stable_order_is_game_version_then_numeric_semver() {
        let tags = [
            "26.3-0.1.0",
            "26.3-0.1.1",
            "26.3-0.1.10",
            "26.3-0.9.0",
            "26.3.1-0.1.0",
            "26.4-0.1.0",
            "26.10-0.1.0",
        ];
        for (i, old) in tags.iter().enumerate() {
            assert_eq!(compare_game_versions(old, old), Ordering::Equal);
            for new in &tags[i + 1..] {
                assert_eq!(compare_game_versions(old, new), Ordering::Less);
                assert_eq!(compare_game_versions(new, old), Ordering::Greater);
            }
        }
        assert_eq!(
            compare_game_versions("v2026.10.13", "v0.2.0"),
            Ordering::Equal
        );
    }
}
