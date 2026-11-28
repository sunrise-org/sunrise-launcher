//! Which source the Logs tab opens on: the populated one written most recently, and when to pick
//! again. The facts (is there anything, when was it last written) are read off the UI thread by
//! `scan`; what to do with them is plain data in, a source out.
use super::sources::{list_files, Source};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Writes closer together than this are one burst of activity (a game printing, a build running);
/// the first write after a quieter spell is new activity.
const BURST: Duration = Duration::from_secs(30);

/// What a source has: anything at all, and when it was last written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Fact {
    pub source: Source,
    pub populated: bool,
    pub at: Option<SystemTime>,
}

/// The populated source written most recently. A source with no known time ranks last, and on a
/// tie the one earlier in the list wins.
pub(super) fn most_recent(facts: &[Fact]) -> Option<Source> {
    let mut best: Option<&Fact> = None;
    for fact in facts.iter().filter(|f| f.populated) {
        if best.is_none_or(|b| fact.at > b.at) {
            best = Some(fact);
        }
    }
    best.map(|f| f.source)
}

/// The sources the dropdown lists, in its order: the populated ones, and `keep` even when it has
/// nothing (the log an error pointed at).
pub(super) fn listed(facts: &[Fact], keep: Option<Source>) -> Vec<Source> {
    Source::ALL
        .iter()
        .copied()
        .filter(|source| {
            Some(*source) == keep
                || facts
                    .iter()
                    .any(|fact| fact.source == *source && fact.populated)
        })
        .collect()
}

/// Reads what each source has for the profile at `home`, with the launcher's diagnostics in
/// `launcher_logs` and the native mods' build log at `build_log`.
pub(super) fn scan(home: &Path, launcher_logs: &Path, build_log: &Path) -> Vec<Fact> {
    let latest = file_fact(&home.join("logs/latest.log"));
    let crashes = list_fact(&home.join("crash-reports"), Source::Crashes);
    let failures = list_fact(launcher_logs, Source::Launcher);
    let build = file_fact(build_log);
    let fact = |source: Source, (populated, at): (bool, Option<SystemTime>)| Fact {
        source,
        populated,
        at,
    };
    vec![
        // The game's output is its latest.log until this session has lines of its own.
        fact(Source::Game, latest),
        fact(Source::Latest, latest),
        fact(Source::Crashes, crashes),
        fact(Source::Launcher, failures),
        fact(Source::Build, build),
    ]
}

/// A file that has something in it, and when it was written.
fn file_fact(path: &Path) -> (bool, Option<SystemTime>) {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() && meta.len() > 0 => (true, meta.modified().ok()),
        _ => (false, None),
    }
}

/// A folder with any file of `source`'s list, and when the newest was written.
fn list_fact(dir: &Path, source: Source) -> (bool, Option<SystemTime>) {
    let files: Vec<PathBuf> = list_files(dir, source);
    match files.first() {
        Some(newest) => (
            true,
            std::fs::metadata(newest).and_then(|m| m.modified()).ok(),
        ),
        None => (false, None),
    }
}

/// What the tab remembers between frames about picking a source.
#[derive(Default)]
pub(super) struct Auto {
    /// The profile the last pick was for; `None` before the first pick or after leaving the tab.
    home: Option<PathBuf>,
    /// The player picked the source by hand.
    hand: bool,
    /// The source was opened on purpose from elsewhere ("View logs"), even if it is empty.
    pub(super) pinned: bool,
    /// What each source had at the last look, to tell new activity from the same.
    seen: Vec<Fact>,
}

impl Auto {
    /// Forgets everything: the next look picks afresh (the tab was opened again).
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    /// The player chose a source from the dropdown: keep it until something else is written.
    pub(super) fn picked_by_hand(&mut self) {
        self.hand = true;
        self.pinned = false;
    }

    /// The sources with activity since the last look: written after a quiet spell, or newly there.
    fn fresh(&mut self, facts: &[Fact]) -> Vec<Fact> {
        let fresh = facts
            .iter()
            .filter(|fact| fact.populated)
            .filter(|fact| {
                let before = self
                    .seen
                    .iter()
                    .find(|old| old.source == fact.source && old.populated);
                match (before, fact.at) {
                    (None, _) => true,
                    (Some(old), Some(at)) => match old.at {
                        None => true,
                        Some(was) => at.duration_since(was).is_ok_and(|gap| gap > BURST),
                    },
                    (Some(_), None) => false,
                }
            })
            .copied()
            .collect();
        self.seen = facts.to_vec();
        fresh
    }

    /// The source to show now, given the one shown and what every source has. `None` when none
    /// has anything (and none was opened on purpose): keep what is shown.
    pub(super) fn decide(
        &mut self,
        home: &Path,
        current: Source,
        facts: &[Fact],
    ) -> Option<Source> {
        let fresh = self.fresh(facts);
        if self.home.as_deref() != Some(home) {
            self.home = Some(home.to_path_buf());
            if self.pinned {
                self.hand = true;
                return Some(current);
            }
            self.hand = false;
            return most_recent(facts);
        }
        if let Some(source) = most_recent(&fresh) {
            self.hand = false;
            self.pinned = false;
            return Some(source);
        }
        let holds = facts.iter().any(|f| f.source == current && f.populated);
        if self.pinned || (self.hand && holds) {
            return Some(current);
        }
        self.hand = false;
        most_recent(facts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64) -> Option<SystemTime> {
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
    }

    fn fact(source: Source, populated: bool, secs: Option<u64>) -> Fact {
        Fact {
            source,
            populated,
            at: secs.and_then(at),
        }
    }

    #[test]
    fn the_newest_populated_source_wins() {
        let facts = [
            fact(Source::Game, true, Some(100)),
            fact(Source::Crashes, true, Some(300)),
            fact(Source::Build, true, Some(200)),
            fact(Source::Launcher, false, Some(900)),
        ];
        assert_eq!(most_recent(&facts), Some(Source::Crashes));
    }

    #[test]
    fn a_tie_goes_to_the_earlier_source_and_no_time_ranks_last() {
        let facts = [
            fact(Source::Game, true, Some(100)),
            fact(Source::Latest, true, Some(100)),
            fact(Source::Build, true, None),
        ];
        assert_eq!(most_recent(&facts), Some(Source::Game));
        assert_eq!(
            most_recent(&[fact(Source::Build, true, None)]),
            Some(Source::Build)
        );
        assert_eq!(most_recent(&[fact(Source::Build, false, Some(1))]), None);
    }

    #[test]
    fn only_populated_sources_are_listed() {
        let facts = [
            fact(Source::Game, false, None),
            fact(Source::Latest, true, Some(1)),
            fact(Source::Build, true, Some(2)),
        ];
        assert_eq!(listed(&facts, None), [Source::Latest, Source::Build]);
        assert_eq!(
            listed(&facts, Some(Source::Crashes)),
            [Source::Latest, Source::Crashes, Source::Build]
        );
        assert!(listed(&[], None).is_empty());
    }

    #[test]
    fn the_scan_sees_files_with_something_in_them() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("profile");
        std::fs::create_dir_all(home.join("logs")).unwrap();
        std::fs::create_dir_all(home.join("crash-reports")).unwrap();
        std::fs::write(home.join("logs/latest.log"), "").unwrap();
        std::fs::write(home.join("crash-reports/crash-1-client.txt"), "x").unwrap();
        let build = dir.path().join("build.log");
        std::fs::write(&build, "built").unwrap();
        let facts = scan(&home, &dir.path().join("none"), &build);
        let has = |source| {
            facts
                .iter()
                .find(|f| f.source == source)
                .is_some_and(|f| f.populated)
        };
        assert!(!has(Source::Game) && !has(Source::Latest));
        assert!(has(Source::Crashes) && has(Source::Build));
        assert!(!has(Source::Launcher));
    }

    #[test]
    fn a_hand_pick_stays_until_something_new_is_written() {
        let home = Path::new("/profile");
        let mut auto = Auto::default();
        let mut facts = vec![
            fact(Source::Latest, true, Some(1_000)),
            fact(Source::Build, true, Some(2_000)),
        ];
        // Opening picks the newest.
        assert_eq!(auto.decide(home, Source::Game, &facts), Some(Source::Build));
        // The player picks latest.log; the same facts leave it be.
        auto.picked_by_hand();
        assert_eq!(
            auto.decide(home, Source::Latest, &facts),
            Some(Source::Latest)
        );
        // The build keeps writing within one burst: still theirs.
        facts[1].at = at(2_010);
        assert_eq!(
            auto.decide(home, Source::Latest, &facts),
            Some(Source::Latest)
        );
        // A new build after a quiet spell takes over.
        facts[1].at = at(5_000);
        assert_eq!(
            auto.decide(home, Source::Latest, &facts),
            Some(Source::Build)
        );
        // A crash appearing does too.
        auto.picked_by_hand();
        facts.push(fact(Source::Crashes, true, Some(6_000)));
        assert_eq!(
            auto.decide(home, Source::Build, &facts),
            Some(Source::Crashes)
        );
    }

    #[test]
    fn opening_again_or_another_profile_picks_afresh() {
        let facts = [
            fact(Source::Latest, true, Some(1)),
            fact(Source::Build, true, Some(2)),
        ];
        let mut auto = Auto::default();
        auto.decide(Path::new("/a"), Source::Game, &facts);
        auto.picked_by_hand();
        assert_eq!(
            auto.decide(Path::new("/b"), Source::Latest, &facts),
            Some(Source::Build)
        );
        auto.picked_by_hand();
        auto.reset();
        assert_eq!(
            auto.decide(Path::new("/b"), Source::Latest, &facts),
            Some(Source::Build)
        );
    }

    #[test]
    fn a_log_opened_on_purpose_wins_until_the_player_moves() {
        let facts = [fact(Source::Latest, true, Some(1))];
        let mut auto = Auto::default();
        auto.reset();
        auto.pinned = true;
        let home = Path::new("/a");
        assert_eq!(
            auto.decide(home, Source::Build, &facts),
            Some(Source::Build)
        );
        assert_eq!(
            auto.decide(home, Source::Build, &facts),
            Some(Source::Build)
        );
        auto.picked_by_hand();
        // The hand pick on a source with nothing falls back to what there is.
        assert_eq!(
            auto.decide(home, Source::Build, &facts),
            Some(Source::Latest)
        );
    }

    #[test]
    fn nothing_populated_picks_nothing() {
        let mut auto = Auto::default();
        let facts = [fact(Source::Game, false, None)];
        assert_eq!(auto.decide(Path::new("/a"), Source::Game, &facts), None);
    }
}
