//! Which build a profile's mods have releases for, when the build it is on leaves some out.
//!
//! A player picked the build their mods asked for and was still told they were on the latest
//! one, with no mod working (2026-11-26). Home now says it plainly: the newest build each wanted
//! mod has a release for (one the player has on, or one an earlier switch held off while it was
//! on), with one press that downloads, prepares and switches to it; or, when no build has them
//! all, which mods each build leaves out. A release marked for a build is each project's own
//! claim, not proof the whole set works together, so Home says "has releases for", not
//! "works on".
//!
//! The check reads the signed mod index and the profile's mods on a background thread, once
//! per state of the profile, its installed mods, the installed builds and the release list, and
//! again after a failed read or while Home stays open, as the index changes on its own.

use super::*;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use sunrise_launcher::browser::sunrise::{Blocker, FitIssue};

/// How many builds the details name when no build has every mod.
const SAID_BUILDS: usize = 6;
/// A failed read (no network, no index yet) is tried again after this long.
const RETRY: Duration = Duration::from_secs(30);
/// A good answer is checked again after this long: the signed index changes on its own.
const RECHECK: Duration = Duration::from_secs(5 * 60);

/// The answer for one profile on one build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Fit {
    /// The first build, in the caller's order of preference, that each wanted mod has a
    /// release for.
    Needs(String),
    /// None of the `checked` builds has every wanted mod: each with what it leaves out.
    Blocked {
        checked: usize,
        builds: Vec<(String, Vec<Blocker>)>,
    },
}

/// What the check was asked, so a change of profile, mods, builds or list asks again. All of
/// it is already in memory: the profile's own pointer is read on the check's thread.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Key {
    home: PathBuf,
    pin: Option<String>,
    channel: instances::Channel,
    /// The inventory Home last read (mods, archives, pins and holds change it).
    inventory: u64,
    mods_off: usize,
    held_on: usize,
    installed: Vec<String>,
    /// Every release's tag and whether it has a build for this computer: a new, withdrawn or
    /// re-uploaded build asks again.
    releases: u64,
    /// The verified index in memory, by serial: a newer one asks again.
    index: Option<u64>,
}

/// [`Key::releases`]: the candidates the picker can offer, without keeping a copy of them.
fn releases_digest(releases: &[sunrise_launcher::backend::Release]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for release in releases {
        release.tag.hash(&mut hasher);
        release
            .asset
            .as_ref()
            .map(|(url, size)| (url, size))
            .hash(&mut hasher);
    }
    hasher.finish()
}

struct Answer {
    /// None while the index or the profile's mods can't be read. Shared, not copied, by each
    /// frame that draws it.
    fit: Option<Arc<Fit>>,
    failed: bool,
}

#[derive(Default)]
pub(crate) struct ModsFit {
    key: Option<Key>,
    /// When the current check finished: retry/recheck age and the background timer both count
    /// from completion so a slow failure still pauses before retrying.
    completed: Option<Instant>,
    /// Set by the periodic refresh timer once it fires for this check.
    due: bool,
    answer: Arc<OnceLock<Answer>>,
    /// The last answer, shown while a recheck runs.
    shown: Option<Arc<Fit>>,
    /// Why Home last didn't ask, as logged: said again only when it changes.
    idle: Option<&'static str>,
}

fn refresh_delay(failed: bool) -> Duration {
    if failed {
        RETRY
    } else {
        RECHECK
    }
}

impl ModsFit {
    fn is_stale(&self, now: Instant) -> bool {
        self.due
            || self.answer.get().is_some_and(|answer| {
                self.completed.is_some_and(|completed| {
                    now.saturating_duration_since(completed) >= refresh_delay(answer.failed)
                })
            })
    }
}

/// One line in launcher.log about Home's suggestion, so a report shows why it was or wasn't made.
fn note(text: impl AsRef<str>) {
    sunrise_launcher::launcher_log::line(format!("Home's build suggestion: {}", text.as_ref()));
}

/// The first of `checked` (in the caller's order of preference) that leaves no mod out, else
/// every checked build with what it leaves out. None when nothing was checked.
pub(crate) fn decide(checked: Vec<(String, Vec<Blocker>)>) -> Option<Fit> {
    if let Some((tag, _)) = checked.iter().find(|(_, blockers)| blockers.is_empty()) {
        return Some(Fit::Needs(tag.clone()));
    }
    (!checked.is_empty()).then_some(Fit::Blocked {
        checked: checked.len(),
        builds: checked,
    })
}

/// The builds to check: every candidate but the one the profile plays, read from its pointer.
/// The channel's newest stays a candidate when the profile plays another build.
pub(crate) fn without_played(builds: Vec<String>, plays: Option<&str>) -> Vec<String> {
    builds
        .into_iter()
        .filter(|tag| Some(tag.as_str()) != plays)
        .collect()
}

/// "Wayfinder, Minimap and 2 more": a short list for one line.
pub(crate) fn names<'a>(mods: impl IntoIterator<Item = &'a str>) -> String {
    const SHOWN: usize = 2;
    let mods: Vec<&str> = mods.into_iter().collect();
    if mods.len() <= SHOWN + 1 {
        return mods.join(", ");
    }
    trf(
        "{mods} and {count} more",
        &[
            ("mods", &mods[..SHOWN].join(", ")),
            ("count", &(mods.len() - SHOWN).to_string()),
        ],
    )
}

/// Each build's name, with its short commit when two of `tags` would read the same
/// ("Nightly · Nov 20 · 0c02067"), so a suggestion and a blocker row can't be confused.
pub(crate) fn distinct_names(tags: &[&str]) -> Vec<String> {
    let plain: Vec<String> = tags
        .iter()
        .map(|tag| crate::sheets::build_name(tag))
        .collect();
    tags.iter()
        .zip(&plain)
        .map(|(tag, name)| {
            let twins = plain.iter().filter(|other| *other == name).count() > 1;
            match tag.rsplit('-').next().filter(|_| twins) {
                Some(commit) if commit != *tag => format!(
                    "{name} · {}",
                    sunrise_launcher::nightly::short_commit(commit)
                ),
                _ => name.clone(),
            }
        })
        .collect()
}

/// What one build leaves out, in one sentence: the mods with no release for it first, then
/// those not marked for it yet, then those that can't be checked or were withdrawn.
pub(crate) fn build_line(build: &str, blockers: &[Blocker]) -> String {
    let of = |issue: FitIssue| {
        names(
            blockers
                .iter()
                .filter(|blocker| blocker.issue == issue)
                .map(|blocker| blocker.name.as_str()),
        )
    };
    let mut parts = Vec::new();
    for (issue, text) in [
        (FitIssue::NoRelease, "no release of {mods}"),
        (FitIssue::Unproved, "{mods} not marked for it yet"),
        (
            FitIssue::NotFromRegistry,
            "{mods} not from the registry, so not checked",
        ),
        (FitIssue::Withdrawn, "{mods} withdrawn from the registry"),
    ] {
        let mods = of(issue);
        if !mods.is_empty() {
            parts.push(trf(text, &[("mods", &mods)]));
        }
    }
    trf(
        "{build}: {what}",
        &[("build", build), ("what", &parts.join("; "))],
    )
}

/// The details of a "no build has every mod" answer, one build a line.
pub(crate) fn blocked_details(checked: usize, builds: &[(String, Vec<Blocker>)]) -> String {
    let shown = &builds[..builds.len().min(SAID_BUILDS)];
    let tags: Vec<&str> = shown.iter().map(|(tag, _)| tag.as_str()).collect();
    let mut lines = vec![trf(
        "Checked {count} builds. None has a release of every mod you use.",
        &[("count", &checked.to_string())],
    )];
    lines.extend(
        distinct_names(&tags)
            .iter()
            .zip(shown)
            .map(|(name, (_, blockers))| build_line(name, blockers)),
    );
    if builds.len() > shown.len() {
        lines.push(trf(
            "And {count} older builds.",
            &[("count", &(builds.len() - shown.len()).to_string())],
        ));
    }
    lines.join("\n")
}

impl Launcher {
    /// The answer for the selected profile while its build leaves mods out (or an earlier
    /// switch holds some off) and nothing else is under way; starts the check when it hasn't
    /// been asked for this state yet, after a failed read, or after a while.
    pub(crate) fn mods_fit(&mut self, cx: &mut Context<Self>) -> Option<Arc<Fit>> {
        let instance = self.selected_instance()?.clone();
        let summary = self.ui.home.mods.launch();
        let (mods_off, held_on) =
            summary.map_or((0, 0), |summary| (summary.mods_off, summary.held_on));
        let idle = if instance.kind == instances::Kind::Pack {
            Some("not for a modpack")
        } else if summary.is_none() {
            Some("waiting for the profile's mods to be read")
        } else if mods_off == 0 && held_on == 0 {
            Some("not needed, no wanted mod is left out")
        } else if self.launch_busy() || self.switch_busy() || self.client.is_some() {
            Some("waiting, something else is under way")
        } else {
            None
        };
        if let Some(why) = idle {
            if self.ui.home.fit.idle != Some(why) {
                self.ui.home.fit.idle = Some(why);
                note(why);
            }
            return None;
        }
        self.ui.home.fit.idle = None;
        let releases = self.game_releases();
        let key = Key {
            home: instance.home.clone(),
            pin: instance.version.clone(),
            channel: instance.channel,
            inventory: self.ui.home.mods.revision(),
            mods_off,
            held_on,
            installed: self.installed.clone(),
            releases: releases_digest(releases),
            index: sunrise_launcher::browser::sunrise::v2::loaded_serial(),
        };
        let now = Instant::now();
        let state = &self.ui.home.fit;
        let stale = state.is_stale(now);
        if state.key.as_ref() != Some(&key) || stale {
            // Only now, not on every frame: the candidates are the picker's list, in the
            // order Home prefers. The build the profile is on is dropped on the check's thread,
            // which reads its pointer: the channel's newest (`fallback`) can be another build
            // than the one it plays, and is a candidate then (review of #616).
            let fallback = self
                .target_version(instance.version.as_ref(), instance.channel)
                .cloned();
            let builds = self.switch_candidates(&instance, fallback.as_deref());
            let state = &mut self.ui.home.fit;
            if state.key.as_ref() != Some(&key) {
                state.shown = None;
            }
            let answer = Arc::new(OnceLock::new());
            state.key = Some(key.clone());
            state.completed = None;
            state.due = false;
            state.answer = answer.clone();
            let home = key.home.clone();
            let work = cx.background_executor().spawn(async move {
                let plays = sunrise_launcher::managed_play::display_active(&home)
                    .ok()
                    .flatten()
                    .map(|active| active.base().to_owned())
                    .or(fallback);
                let builds = without_played(builds, plays.as_deref());
                note(format!(
                    "{mods_off} mods left out and {held_on} held off; checking {} builds",
                    builds.len()
                ));
                let failed = Answer {
                    fit: None,
                    failed: true,
                };
                let catalog = match sunrise_launcher::browser::sunrise::v2::index() {
                    Ok(catalog) => catalog,
                    Err(error) => {
                        note(format!(
                            "the mod index couldn't be read: {}",
                            sunrise_launcher::launcher_log::safe_error(&error.to_string())
                        ));
                        return (failed, None);
                    }
                };
                let serial = catalog.index.serial;
                let Some(checked) = sunrise_launcher::browser::sunrise::mods_fit_by_build_v2(
                    &catalog.index,
                    &home,
                    &builds,
                ) else {
                    note("the profile's mods couldn't be read");
                    return (failed, None);
                };
                let fit = decide(checked);
                note(match &fit {
                    Some(Fit::Needs(tag)) => format!("{tag} has releases for every wanted mod"),
                    Some(Fit::Blocked { checked, .. }) => {
                        format!("none of {checked} builds has every wanted mod")
                    }
                    None => "no other build to check".to_owned(),
                });
                (
                    Answer {
                        fit: fit.map(Arc::new),
                        failed: false,
                    },
                    Some(serial),
                )
            });
            cx.spawn(async move |this, cx| {
                let (result, used_serial) = work.await;
                let delay = refresh_delay(result.failed);
                let _ = answer.set(result);
                let finished = Instant::now();
                let active = this
                    .update(cx, |this, cx| {
                        let state = &mut this.ui.home.fit;
                        let current = Arc::ptr_eq(&state.answer, &answer);
                        if current {
                            state.completed = Some(finished);
                            if let (Some(serial), Some(key)) = (used_serial, state.key.as_mut()) {
                                key.index = Some(serial);
                            }
                            cx.notify();
                        }
                        current
                    })
                    .unwrap_or(false);
                if !active {
                    return;
                }
                cx.background_executor().timer(delay).await;
                let _ = this.update(cx, |this, cx| {
                    let state = &mut this.ui.home.fit;
                    if Arc::ptr_eq(&state.answer, &answer) {
                        state.due = true;
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        let state = &mut self.ui.home.fit;
        if let Some(answer) = state.answer.get() {
            if !answer.failed {
                state.shown = answer.fit.clone();
            }
        }
        state.shown.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocker(name: &str, issue: FitIssue) -> Blocker {
        Blocker {
            name: name.into(),
            issue,
        }
    }

    #[test]
    fn the_first_build_with_every_mod_is_the_one_named() {
        let rows = vec![
            (
                "nightly-2026-11-25-aaaaaaa".into(),
                vec![blocker("Wayfinder", FitIssue::NoRelease)],
            ),
            ("nightly-2026-11-20-bbbbbbb".into(), Vec::new()),
            ("nightly-2026-11-18-ccccccc".into(), Vec::new()),
        ];
        assert_eq!(
            decide(rows),
            Some(Fit::Needs("nightly-2026-11-20-bbbbbbb".into()))
        );
    }

    #[test]
    fn when_no_build_has_every_mod_every_checked_build_is_kept_and_named() {
        let rows: Vec<(String, Vec<Blocker>)> = (0..20)
            .map(|n| {
                (
                    format!("nightly-2026-11-{:02}-{n:07}", 25 - n),
                    vec![blocker("Wayfinder", FitIssue::Unproved)],
                )
            })
            .collect();
        let Some(Fit::Blocked { checked, builds }) = decide(rows) else {
            panic!("expected no build to fit");
        };
        assert_eq!(
            checked, 20,
            "every available build was checked, not the newest 12"
        );
        let details = blocked_details(checked, &builds);
        assert!(details.starts_with("Checked 20 builds."), "{details}");
        assert!(details.contains("And 14 older builds."), "{details}");
    }

    #[test]
    fn what_a_build_leaves_out_says_what_is_known_not_only_no() {
        let line = build_line(
            "Nightly · Nov 25",
            &[
                blocker("Wayfinder", FitIssue::NoRelease),
                blocker("Minimap", FitIssue::Unproved),
                blocker("My Pack", FitIssue::NotFromRegistry),
            ],
        );
        assert_eq!(
            line,
            "Nightly · Nov 25: no release of Wayfinder; Minimap not marked for it yet; My Pack not from the registry, so not checked"
        );
    }

    #[test]
    fn two_builds_of_one_day_are_told_apart_by_commit() {
        let names = distinct_names(&[
            "nightly-2026-11-20-aaaaaaa",
            "nightly-2026-11-20-bbbbbbb",
            "nightly-2026-11-18-ccccccc",
        ]);
        assert_ne!(names[0], names[1]);
        assert!(names[0].ends_with("aaaaaaa"), "{names:?}");
        assert_eq!(
            names[2],
            crate::sheets::build_name("nightly-2026-11-18-ccccccc")
        );
    }

    #[test]
    fn the_channels_newest_is_a_candidate_when_the_profile_plays_another_build() {
        let a = "nightly-2026-11-22-aaaaaaa".to_owned();
        let b = "nightly-2026-11-20-bbbbbbb".to_owned();
        // The profile follows Nightly, whose newest is A, but plays B: A is offered, B isn't.
        assert_eq!(
            without_played(vec![a.clone(), b.clone()], Some(&b)),
            std::slice::from_ref(&a)
        );
        assert_eq!(without_played(vec![a.clone(), b.clone()], None), [a, b]);
    }

    #[test]
    fn nothing_to_check_says_nothing() {
        assert_eq!(decide(Vec::new()), None);
    }

    #[test]
    fn a_long_list_of_mods_is_cut_to_two_names_and_a_count() {
        assert_eq!(names(["A", "B", "C"]), "A, B, C");
        assert_eq!(names(["A", "B", "C", "D"]), "A, B and 2 more");
    }

    #[test]
    fn mods_fit_goes_stale_at_exact_retry_and_recheck_boundaries_or_when_due() {
        let completed = Instant::now();
        let failed = Arc::new(OnceLock::new());
        let _ = failed.set(Answer {
            fit: None,
            failed: true,
        });
        let mut state = ModsFit {
            completed: Some(completed),
            answer: failed,
            ..Default::default()
        };
        assert!(!state.is_stale(completed + RETRY - Duration::from_millis(1)));
        assert!(state.is_stale(completed + RETRY));
        state.due = true;
        assert!(state.is_stale(completed));

        let ok = Arc::new(OnceLock::new());
        let _ = ok.set(Answer {
            fit: None,
            failed: false,
        });
        let state = ModsFit {
            completed: Some(completed),
            answer: ok,
            ..Default::default()
        };
        assert!(!state.is_stale(completed + RECHECK - Duration::from_millis(1)));
        assert!(state.is_stale(completed + RECHECK));
    }
}
