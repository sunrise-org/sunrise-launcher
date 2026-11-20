//! What a mod's card says about its install beyond fit and "Installed": typed, so the card
//! decides how to show it.
use crate::i18n::{tr, trf};
use sunrise_launcher::browser::installed::InstallError;
use sunrise_launcher::native_mods::BuildFailure;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InstallState {
    /// Another install runs; this one starts when it ends.
    Waiting,
    /// The mod failed to compile on this computer: it was removed again.
    BuildFailed,
    /// The game runs, so nothing installs: "Close the game to install".
    CloseGame,
}

/// The state of mod `id`: `waiting` is whether its install waits its turn, `failed` the mods
/// the last install's build removed because they didn't compile, `game_running` whether the
/// game runs.
pub(crate) fn of(
    id: &str,
    waiting: bool,
    failed: &[BuildFailure],
    game_running: bool,
) -> Option<InstallState> {
    if waiting {
        Some(InstallState::Waiting)
    } else if failed.iter().any(|f| f.id == id) {
        Some(InstallState::BuildFailed)
    } else {
        game_running.then_some(InstallState::CloseGame)
    }
}

/// Where an install of one mod is, in place on its card.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Stage {
    /// The file is coming, 0 to 1.
    Downloading(f32),
    /// The compiler is coming, once: its size in bytes.
    BuildTools(u64),
    /// The mod is building, 0 to 1 of the whole build.
    Building(f32),
}

/// A size in whole megabytes, rounded down so that "at least" stays true: "207 MB".
pub(crate) fn whole_mb(bytes: u64) -> String {
    format!("{} MB", bytes / 1_000_000)
}

impl Stage {
    pub(crate) fn label(&self) -> String {
        match self {
            Stage::Downloading(f) => trf(
                "Downloading {percent}%",
                &[("percent", &format!("{:.0}", f * 100.))],
            ),
            Stage::BuildTools(bytes) => trf(
                "Getting build tools (at least {size})",
                &[("size", &whole_mb(*bytes))],
            ),
            Stage::Building(_) => tr("Building").to_string(),
        }
    }

    pub(crate) fn fraction(&self) -> f32 {
        match self {
            Stage::Downloading(f) | Stage::Building(f) => *f,
            Stage::BuildTools(_) => 0.,
        }
    }
}

/// The stage of mod `id`: `downloading` is the install on screen (its mod and how far),
/// `builds` whether the build that takes it up is waiting or running, `tools` the compiler's
/// size while it downloads, `build` how far the build is.
pub(crate) fn stage(
    id: &str,
    downloading: Option<(&str, f32)>,
    builds: bool,
    tools: Option<u64>,
    build: f32,
) -> Option<Stage> {
    match downloading {
        Some((for_id, f)) if for_id == id => Some(Stage::Downloading(f)),
        _ if builds => Some(match tools {
            Some(bytes) => Stage::BuildTools(bytes),
            None => Stage::Building(build),
        }),
        _ => None,
    }
}

/// "A, B", or "A, B and 12 more" past two names.
fn names_line(names: &[String]) -> Option<String> {
    Some(match names {
        [] => return None,
        [a] => a.clone(),
        [a, b] => format!("{a}, {b}"),
        [a, b, rest @ ..] => trf(
            "{a}, {b} and {count} more",
            &[("a", a), ("b", b), ("count", &rest.len().to_string())],
        ),
    })
}

/// "Also installed: A, B", or "A, B and 12 more" past two names.
pub(crate) fn also_line(names: &[String]) -> Option<String> {
    let shown = names_line(names)?;
    Some(trf("Also installed: {names}", &[("names", &shown)]))
}

/// "Also updated: Flywheel 0.3 → 0.4", for the dependencies an install moved to a new version
/// (title, version before, version after).
pub(crate) fn updated_line(updated: &[(String, String, String)]) -> Option<String> {
    let moves: Vec<String> = updated
        .iter()
        .map(|(title, from, to)| format!("{title} {from} → {to}"))
        .collect();
    let shown = names_line(&moves)?;
    Some(trf("Also updated: {names}", &[("names", &shown)]))
}

/// One plain line for why an install failed; the whole text stays behind "Details". What the
/// index told apart (a mod it needs that isn't made for this version, installed mods that pin
/// what it needs) is said by its type, not read from the words.
pub(crate) fn failure_line(error: &InstallError) -> String {
    match error {
        InstallError::Unavailable { dependency, .. } => trf(
            "Needs {mod}, which isn't available for your version",
            &[("mod", dependency)],
        ),
        InstallError::Conflict { mods, .. } => {
            trf("Conflicts with {mods}", &[("mods", &mods.join(", "))])
        }
        InstallError::Refused(_) => tr("Can't be installed with the mods you have").to_string(),
        other => plain_failure(&other.to_string()),
    }
}

/// One plain line for a failure that is only words: offline, no space, or a download.
pub(crate) fn plain_failure(why: &str) -> String {
    let why = why.to_lowercase();
    if why.contains("offline") || why.contains("connect") || why.contains("dns") {
        tr("You're offline").to_string()
    } else if why.contains("no space") || why.contains("disk full") {
        tr("Not enough space").to_string()
    } else {
        tr("Download failed. Try again").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mod_removed_for_not_compiling_reads_so_and_waiting_comes_first() {
        let failed = [BuildFailure {
            id: "broken".into(),
            reason: "r".into(),
            log: "l".into(),
        }];
        assert_eq!(
            of("broken", false, &failed, false),
            Some(InstallState::BuildFailed)
        );
        assert_eq!(
            of("broken", true, &failed, false),
            Some(InstallState::Waiting)
        );
        assert_eq!(of("fine", false, &failed, false), None);
        assert_eq!(of("fine", false, &[], false), None);
    }

    #[test]
    fn a_running_game_says_to_close_it_for_a_mod_with_nothing_else_to_say() {
        assert_eq!(of("a", false, &[], true), Some(InstallState::CloseGame));
        assert_eq!(of("a", true, &[], true), Some(InstallState::Waiting));
    }

    #[test]
    fn a_card_reads_where_its_install_is_in_the_words_of_the_mock() {
        let at = |id, down, builds, tools, f| stage(id, down, builds, tools, f);
        assert_eq!(
            at("a", Some(("a", 0.4)), false, None, 0.),
            Some(Stage::Downloading(0.4))
        );
        assert_eq!(Stage::Downloading(0.4).label(), "Downloading 40%");
        // Another mod's download is not this card's, and a mod nothing builds says nothing.
        assert_eq!(at("b", Some(("a", 0.4)), false, None, 0.), None);
        assert_eq!(
            at("a", None, true, Some(207 << 20), 0.),
            Some(Stage::BuildTools(207 << 20))
        );
        assert!(Stage::BuildTools(207 << 20)
            .label()
            .starts_with("Getting build tools (at least 2"));
        assert_eq!(at("a", None, true, None, 0.3), Some(Stage::Building(0.3)));
        assert_eq!(Stage::Building(0.3).label(), "Building");
        assert_eq!(Stage::Building(0.3).fraction(), 0.3);
    }

    #[test]
    fn the_also_installed_line_names_two_and_counts_the_rest() {
        let n = |l: &[&str]| also_line(&l.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(n(&[]), None);
        assert_eq!(n(&["Flywheel"]).unwrap(), "Also installed: Flywheel");
        assert_eq!(n(&["A", "B"]).unwrap(), "Also installed: A, B");
        assert_eq!(
            n(&["A", "B", "C", "D"]).unwrap(),
            "Also installed: A, B and 2 more"
        );
    }

    #[test]
    fn the_also_updated_line_shows_each_move_and_counts_past_two() {
        let m = |t: &str, a: &str, b: &str| (t.to_string(), a.to_string(), b.to_string());
        assert_eq!(updated_line(&[]), None);
        assert_eq!(
            updated_line(&[m("Flywheel", "0.3", "0.4")]).unwrap(),
            "Also updated: Flywheel 0.3 → 0.4"
        );
        assert_eq!(
            updated_line(&[m("A", "1", "2"), m("B", "1", "2"), m("C", "1", "2")]).unwrap(),
            "Also updated: A 1 → 2, B 1 → 2 and 1 more"
        );
    }

    #[test]
    fn the_build_tools_size_is_whole_megabytes() {
        assert_eq!(whole_mb(207_000_000), "207 MB");
        assert_eq!(whole_mb(207_640_000), "207 MB");
        assert_eq!(
            Stage::BuildTools(207_000_000).label(),
            "Getting build tools (at least 207 MB)"
        );
    }

    #[test]
    fn a_failed_install_says_one_plain_thing() {
        assert_eq!(
            plain_failure("Couldn't connect to the registry"),
            "You're offline"
        );
        assert_eq!(plain_failure("No space left on device"), "Not enough space");
        assert_eq!(
            plain_failure("its SHA-256 isn't the one listed"),
            "Download failed. Try again"
        );
    }

    #[test]
    fn a_planning_failure_is_told_by_its_type_and_only_a_download_says_try_again() {
        let reason = "Lumen can't be installed: flywheel (lumen needs ^0.4): no listed version \
                      satisfies all requirements";
        assert_eq!(
            failure_line(&InstallError::Unavailable {
                dependency: "Flywheel".into(),
                reason: reason.into()
            }),
            "Needs Flywheel, which isn't available for your version"
        );
        assert_eq!(
            failure_line(&InstallError::Conflict {
                mods: vec!["Lamps".into(), "Owls".into()],
                reason: reason.into()
            }),
            "Conflicts with Lamps, Owls"
        );
        assert_eq!(
            failure_line(&InstallError::Refused(reason.into())),
            "Can't be installed with the mods you have"
        );
        // Words that happen to look alike are not a planning failure.
        assert_eq!(
            failure_line(&InstallError::Message(reason.into())),
            "Download failed. Try again"
        );
        assert_eq!(
            failure_line(&InstallError::Message(
                "its SHA-256 isn't the one listed".into()
            )),
            "Download failed. Try again"
        );
    }
}
