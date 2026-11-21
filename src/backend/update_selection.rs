//! An explicit game update can advance a followed build, not change its channel.
use super::compare_game_versions;
use crate::instances::Channel;

/// An unselected launcher can select its first build. Otherwise, move only from the latest
/// install on the same channel. Legacy, imported, developer and snapshot choices stay put.
pub fn select_after_update(
    selected: Option<&str>,
    installed_before: &[String],
    target: &str,
) -> bool {
    let Some(channel) = Channel::of_build(target) else {
        return false;
    };
    let Some(selected) = selected else {
        return true;
    };
    Channel::of_build(selected) == Some(channel)
        && installed_before
            .iter()
            .filter(|tag| Channel::of_build(tag) == Some(channel))
            .all(|tag| compare_game_versions(selected, tag).is_ge())
        && compare_game_versions(target, selected).is_ge()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_selection_stays_stable() {
        let installed = ["nightly-2026-11-20-aaaaaaa".into(), "26.3-0.1.0".into()];
        assert!(!select_after_update(
            Some("26.3-0.1.0"),
            &installed,
            "nightly-2026-11-21-aaaaaaa"
        ));
        assert!(select_after_update(
            Some("26.3-0.1.0"),
            &installed,
            "26.3-0.1.1"
        ));
        assert!(!select_after_update(
            Some("nightly-2026-11-20-aaaaaaa"),
            &installed,
            "26.3-0.1.1"
        ));
        assert!(!select_after_update(
            Some("v0.2.95"),
            &installed,
            "26.3-0.1.1"
        ));
    }
}
