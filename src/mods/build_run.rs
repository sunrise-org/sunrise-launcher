//! One native build as the build window shows it: a line per mod with where it stands, how far
//! the whole build is, and how it ended. Nothing here draws or touches a file: the build's own
//! steps (`native_mods::Step`) and its result go in, and the window reads the answer.
use crate::i18n::{tr, trf};
use crate::launcher_play::PlayIntent;
use crate::native_mods::{self, Stage, Step, Why};
use std::time::{Duration, Instant};

/// How long the strip says how a build ended, when its window was closed.
pub(crate) const SAID_FOR: Duration = Duration::from_secs(8);

/// Where one mod stands in the build.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum State {
    /// The build has not come to it yet.
    Waiting,
    /// Its own code compiles now.
    Building,
    /// It is in the game the build made.
    Built,
    /// The game changed under it: it waits for its author. `why` is one plain sentence and
    /// `details` the technical lines (file and line, the changed items), for Details only.
    NeedsUpdate { why: String, details: Vec<String> },
    /// The build went on without it for another reason (it needs a mod that was left out, it
    /// clashes with another): one plain sentence.
    Skipped(String),
    /// Its own code did not compile, or the build refused it: one plain sentence.
    Failed(String),
}

/// One mod of the build.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Line {
    pub id: String,
    pub title: String,
    pub version: String,
    /// Its picture's address, when the registry has one.
    pub icon: Option<String>,
    pub state: State,
}

/// How a build ended.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum End {
    Done,
    Cancelled,
    /// The build stopped: what the player reads, in a sentence.
    Stopped(String),
}

/// What the summary counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub built: usize,
    pub needs_update: usize,
    pub failed: usize,
    /// Left out for another reason, or never reached because the build stopped.
    pub other: usize,
}

/// A build, from the press on Build until its window is closed after it ended.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Run {
    /// The profile it builds for, by id and by name.
    pub instance: String,
    pub profile: String,
    /// The game build it builds on, as people read it ("Nov 17").
    pub on: String,
    pub lines: Vec<Line>,
    /// What the build is doing now, in a player's words.
    pub step: String,
    pub progress: Option<native_mods::Progress>,
    /// The compiler's size when this build downloads it, once.
    pub first_build: Option<u64>,
    pub end: Option<End>,
    /// Whether Details is open.
    pub details: bool,
    /// Play started it: the game starts by itself when it ends well.
    pub for_play: bool,
    /// When it ended with its window closed: the strip says how it went for [`SAID_FOR`].
    pub said_at: Option<Instant>,
}

/// How much of the whole each stage is: compiling is nearly all of a build.
const WEIGHT: [(Stage, f32); 3] = [
    (Stage::Prepare, 0.15),
    (Stage::Compile, 0.8),
    (Stage::Install, 0.05),
];

impl Run {
    /// A build of `mods` (id, title, version, picture) about to start.
    pub(crate) fn start(
        instance: String,
        profile: String,
        on: String,
        mods: Vec<(String, String, String, Option<String>)>,
        for_play: bool,
    ) -> Self {
        Self {
            instance,
            profile,
            on,
            lines: mods
                .into_iter()
                .map(|(id, title, version, icon)| Line {
                    id,
                    title,
                    version,
                    icon,
                    state: State::Waiting,
                })
                .collect(),
            step: tr("Starting the build").into(),
            progress: None,
            first_build: None,
            end: None,
            details: false,
            for_play,
            said_at: None,
        }
    }

    /// The build said where it is.
    pub(crate) fn step(&mut self, step: &Step) {
        self.step = step.shown();
        self.progress = Some(step.progress());
        match step {
            Step::Mod { id, done, total } => {
                // The mod by the name its row has, not its id.
                if let Some(line) = self.lines.iter().find(|l| l.id == *id) {
                    self.step = trf(
                        "Building {mod} ({done} of {total})",
                        &[
                            ("mod", &line.title),
                            ("done", &done.to_string()),
                            ("total", &total.to_string()),
                        ],
                    );
                }
                for line in &mut self.lines {
                    match (&line.state, line.id == *id) {
                        (State::Building, false) => line.state = State::Built,
                        (State::Waiting, true) => line.state = State::Building,
                        _ => {}
                    }
                }
            }
            Step::LeftOut(left) => self.mark_left_out(left),
            // A build that runs again without a mod starts its mods over.
            Step::PrebuiltMods => {
                for line in &mut self.lines {
                    if matches!(line.state, State::Building | State::Built) {
                        line.state = State::Waiting;
                    }
                }
            }
            _ => {}
        }
    }

    /// The build ended well: every mod it did not leave out is in the game.
    pub(crate) fn done(&mut self) {
        for line in &mut self.lines {
            if matches!(line.state, State::Waiting | State::Building) {
                line.state = State::Built;
            }
        }
        self.end = Some(End::Done);
    }

    /// The player cancelled and the build heard it: it gave no game, so nothing reads as built.
    pub(crate) fn cancelled(&mut self) {
        self.unbuild();
        self.end = Some(End::Cancelled);
    }

    /// Records the actual reason, from either a progress event or the final result.
    pub(crate) fn mark_left_out(&mut self, left: &native_mods::LeftOut) {
        if let Some(line) = self
            .lines
            .iter_mut()
            .find(|line| line.id == left.id && line.version == left.version)
        {
            line.state = left_out(left, &self.on);
        }
    }

    /// The build stopped with `said`; `failed` names the mods blamed by its diagnostic.
    pub(crate) fn stopped(&mut self, said: String, failed: &[String]) {
        self.unbuild();
        for line in &mut self.lines {
            if failed.contains(&line.id) && matches!(line.state, State::Waiting) {
                line.state = State::Failed(said.clone());
            }
        }
        self.end = Some(End::Stopped(said));
    }

    /// The build stopped before it left out `mods` (worlds use them): each line says what it
    /// waits for, never "Waiting", since the build can't take them as they are.
    pub(crate) fn at_stake(&mut self, mods: &[String], unbuilt: &[String]) {
        let on = self.on.clone();
        for line in &mut self.lines {
            if unbuilt.contains(&line.id) {
                line.state = State::Failed(did_not_build());
            } else if mods.contains(&line.id) {
                line.state = State::NeedsUpdate {
                    why: trf(
                        "Needs an update for {build}. It comes back when its author releases one.",
                        &[("build", &on)],
                    ),
                    details: Vec::new(),
                };
            }
        }
    }

    /// No game was installed: what compiled so far is in no game.
    fn unbuild(&mut self) {
        for line in &mut self.lines {
            if matches!(line.state, State::Building | State::Built) {
                line.state = State::Waiting;
            }
        }
    }

    pub(crate) fn running(&self) -> bool {
        self.end.is_none()
    }

    /// How far the whole build is, 0 to 1.
    pub(crate) fn fraction(&self) -> f32 {
        if self.end == Some(End::Done) {
            return 1.;
        }
        let Some(now) = self.progress else {
            return 0.;
        };
        WEIGHT
            .iter()
            .map(|(stage, weight)| match stage.cmp(&now.stage) {
                std::cmp::Ordering::Less => *weight,
                std::cmp::Ordering::Equal => weight * now.fraction.unwrap_or(0.).clamp(0., 1.),
                std::cmp::Ordering::Greater => 0.,
            })
            .sum()
    }

    pub(crate) fn counts(&self) -> Counts {
        let mut counts = Counts::default();
        for line in &self.lines {
            match line.state {
                State::Built => counts.built += 1,
                State::NeedsUpdate { .. } => counts.needs_update += 1,
                State::Failed(_) => counts.failed += 1,
                State::Skipped(_) | State::Waiting | State::Building => counts.other += 1,
            }
        }
        counts
    }

    /// The summary's headline.
    pub(crate) fn headline(&self) -> String {
        let counts = self.counts();
        let count = counts.built.to_string();
        match (&self.end, counts.built) {
            (None, _) if self.for_play => tr("Getting ready to play").into(),
            (None, _) => tr("Building your mods").into(),
            (Some(End::Cancelled), _) => tr("Build cancelled").into(),
            (Some(End::Stopped(_)), _) => tr("The build stopped").into(),
            (Some(End::Done), 0) => tr("None of your mods is in this build").into(),
            (Some(End::Done), _) if counts.built == self.lines.len() => {
                tr("Your mods are ready").into()
            }
            (Some(End::Done), 1) => tr("1 mod is ready").into(),
            (Some(End::Done), _) => trf("{count} mods are ready", &[("count", &count)]),
        }
    }
}

impl Run {
    /// The Installed row follows the build for this version of the mod. An incompatibility
    /// or compile failure remains visible after the build ends, but progress does not.
    pub(crate) fn state_of(&self, id: &str, version: &str) -> Option<&State> {
        self.lines
            .iter()
            .find(|line| line.id == id && line.version == version)
            .map(|line| &line.state)
            .filter(|state| {
                self.running() || matches!(state, State::NeedsUpdate { .. } | State::Failed(_))
            })
    }

    /// Which mod of those the build compiles it is at, and of how many: "2 of 3". The mods it
    /// left out are not counted.
    pub(crate) fn at(&self) -> Option<(usize, usize)> {
        let count = |f: fn(&State) -> bool| self.lines.iter().filter(|l| f(&l.state)).count();
        let built = count(|s| *s == State::Built);
        let building = count(|s| *s == State::Building);
        let total = built + building + count(|s| *s == State::Waiting);
        (building > 0).then_some((built + 1, total))
    }

    /// The one line the strip, Home and the Build button say while it runs:
    /// "Building your mods · 2 of 3 · 51%".
    pub(crate) fn line(&self) -> String {
        let percent = format!("{:.0}", self.fraction() * 100.);
        match self.at() {
            Some((at, total)) => trf(
                "Building your mods · {at} of {total} · {percent}%",
                &[
                    ("at", &at.to_string()),
                    ("total", &total.to_string()),
                    ("percent", &percent),
                ],
            ),
            None => trf("Building your mods · {percent}%", &[("percent", &percent)]),
        }
    }

    /// What the strip says for a moment once a build ended with its window closed: "3 mods
    /// ready · 1 needs an update". None for a build that runs, one whose window showed its end,
    /// or once the moment is over.
    pub(crate) fn said(&self, now: Instant) -> Option<String> {
        let at = self.said_at?;
        if now.saturating_duration_since(at) >= SAID_FOR {
            return None;
        }
        let counts = self.counts();
        let ready = match (self.end.as_ref()?, counts.built) {
            (End::Cancelled, _) => return Some(tr("Build cancelled").into()),
            (End::Stopped(_), _) => return None,
            (End::Done, 1) => tr("1 mod ready").to_string(),
            (End::Done, n) => trf("{count} mods ready", &[("count", &n.to_string())]),
        };
        let mut parts = vec![ready];
        match counts.needs_update {
            0 => {}
            1 => parts.push(tr("1 needs an update").into()),
            n => parts.push(trf("{count} need an update", &[("count", &n.to_string())])),
        }
        if counts.failed > 0 {
            parts.push(trf(
                "{count} failed",
                &[("count", &counts.failed.to_string())],
            ));
        }
        Some(parts.join(" · "))
    }
}

/// The player asked the build to stop: from this press on nothing waits to start the game,
/// however the build ends. A build that had already finished its last check for a cancel, or
/// had not started checking yet, can still end well; `wanted` (the profile Play was pressed
/// for) is forgotten here, at the press, so that result starts nothing.
pub(crate) fn stop_asked(run: Option<&mut Run>, wanted: &mut Option<PlayIntent>) {
    *wanted = None;
    if let Some(run) = run {
        run.for_play = false;
    }
}

/// Selects the Play handoff when a `Native` result completes. Resume only on the profile that
/// asked, still playing the game just built. A skipped mod doesn't stop this: the build still
/// gave a game. Keep the original account.
pub(crate) fn play_on_finish(
    wanted: Option<PlayIntent>,
    result: &Result<String, native_mods::Failed>,
    plays: Option<(&str, Option<&str>)>,
) -> Option<PlayIntent> {
    let play = wanted?;
    let built = result.as_deref().ok()?;
    let (selected, version) = plays?;
    (play.instance == selected && version == Some(built)).then_some(play)
}

/// A mod the build left out, as its line says it.
fn left_out(left: &native_mods::LeftOut, on: &str) -> State {
    match &left.why {
        Why::Unfit(_) | Why::OtherVersion => {
            let detail = left.detail.clone().unwrap_or_default();
            State::NeedsUpdate {
                why: trf(
                    "Needs an update for {build}. It comes back when its author releases one.",
                    &[("build", on)],
                ),
                details: detail.changes,
            }
        }
        Why::Unproved(_) => State::Skipped(left.label()),
        Why::Needs(other) => State::Skipped(trf(
            "Waits for {other}, which is not in this build.",
            &[("other", other)],
        )),
        Why::Clash(other) => State::Skipped(trf(
            "Clashes with {other}: turn one of them off.",
            &[("other", other)],
        )),
        Why::DidNotBuild => State::Failed(did_not_build()),
    }
}

/// A mod whose own code did not compile, as its line says it.
fn did_not_build() -> String {
    tr("Didn't build. The problem is in the mod, not in your game.").into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_mods::{Detail, LeftOut, Plan, Profile};

    fn run(for_play: bool) -> Run {
        let mods = ["jei", "minimap", "veinminer", "zoom"]
            .map(|id| (id.to_owned(), id.to_uppercase(), "1.0.0".to_owned(), None));
        Run::start(
            "autumn".into(),
            "Autumn".into(),
            "Nov 17".into(),
            mods.into(),
            for_play,
        )
    }

    fn unfit(id: &str) -> Step {
        Step::LeftOut(LeftOut {
            id: id.into(),
            version: "1.0.0".into(),
            why: Why::Unfit("the type Block changed".into()),
            detail: Some(Detail {
                made_for: Some("nightly-2026-11-16-abc1234".into()),
                changes: vec!["src/sources/client.rs:219 builds it".into()],
            }),
        })
    }

    fn compiling(id: &str, done: usize) -> Step {
        Step::Mod {
            id: id.into(),
            done,
            total: 3,
        }
    }

    fn states(run: &Run) -> Vec<&'static str> {
        run.lines
            .iter()
            .map(|l| match l.state {
                State::Waiting => "waiting",
                State::Building => "building",
                State::Built => "built",
                State::NeedsUpdate { .. } => "update",
                State::Skipped(_) => "skipped",
                State::Failed(_) => "failed",
            })
            .collect()
    }

    #[test]
    fn a_build_takes_every_mod_it_is_given_and_each_waits_at_first() {
        let run = run(false);
        assert_eq!(states(&run), ["waiting"; 4]);
        assert!(run.running());
        assert_eq!(run.fraction(), 0.);
        assert_eq!(run.headline(), "Building your mods");
        assert_eq!(run.at(), None);
    }

    #[test]
    fn the_lines_follow_the_builds_own_reports() {
        let mut run = run(false);
        run.step(&Step::Checking(4));
        run.step(&unfit("jei"));
        assert_eq!(states(&run), ["update", "waiting", "waiting", "waiting"]);
        run.step(&Step::PrebuiltMods);
        run.step(&compiling("minimap", 1));
        assert_eq!(states(&run), ["update", "building", "waiting", "waiting"]);
        assert_eq!(run.at(), Some((1, 3)));
        run.step(&compiling("veinminer", 2));
        assert_eq!(states(&run), ["update", "built", "building", "waiting"]);
        assert_eq!(run.at(), Some((2, 3)));
        assert_eq!(run.step, "Building VEINMINER (2 of 3)");
        // Getting ready is done, and one of three mods: 15% + 80% / 3.
        assert!((run.fraction() - (0.15 + 0.8 / 3.)).abs() < 1e-5);
        assert_eq!(run.line(), "Building your mods · 2 of 3 · 42%");
        // What the line says of the mod left out: an outcome, and no file or type name.
        let State::NeedsUpdate { why, details } = &run.lines[0].state else {
            panic!("{:?}", run.lines[0].state);
        };
        assert_eq!(
            why,
            "Needs an update for Nov 17. It comes back when its author releases one."
        );
        assert_eq!(details, &["src/sources/client.rs:219 builds it"]);
    }

    #[test]
    fn a_build_that_runs_again_without_a_mod_starts_its_mods_over() {
        let mut run = run(false);
        run.step(&compiling("jei", 1));
        run.step(&compiling("minimap", 2));
        // minimap did not compile: the build leaves it out and runs again.
        run.step(&Step::LeftOut(LeftOut {
            id: "minimap".into(),
            version: "1.0.0".into(),
            why: Why::DidNotBuild,
            detail: None,
        }));
        run.step(&Step::PrebuiltMods);
        assert_eq!(states(&run), ["waiting", "failed", "waiting", "waiting"]);
        run.step(&compiling("jei", 1));
        assert_eq!(run.at(), Some((1, 3)));
    }

    #[test]
    fn the_summary_counts_what_was_built_what_waits_for_its_author_and_what_failed() {
        let mut run = run(false);
        run.step(&unfit("jei"));
        run.step(&Step::LeftOut(LeftOut {
            id: "zoom".into(),
            version: "1.0.0".into(),
            why: Why::DidNotBuild,
            detail: None,
        }));
        run.step(&compiling("minimap", 1));
        run.step(&Step::Done {
            secs: 40,
            profile: Profile::Optimized,
            plan: Plan::Prebuilt,
        });
        run.done();
        assert_eq!(
            run.counts(),
            Counts {
                built: 2,
                needs_update: 1,
                failed: 1,
                other: 0
            }
        );
        assert!(!run.running());
        assert_eq!(run.fraction(), 1.);
        assert_eq!(run.headline(), "2 mods are ready");
        // Every mod built: the headline says so without a count.
        let mut all = self::run(false);
        all.done();
        assert_eq!(all.counts().built, 4);
        assert_eq!(all.headline(), "Your mods are ready");
        // Every mod left out: a game, and none of them in it.
        let mut none = self::run(false);
        for id in ["jei", "minimap", "veinminer", "zoom"] {
            none.step(&unfit(id));
        }
        none.done();
        assert_eq!(none.counts().needs_update, 4);
        assert_eq!(none.headline(), "None of your mods is in this build");
    }

    #[test]
    fn a_cancelled_build_leaves_nothing_built() {
        let mut run = run(false);
        run.step(&unfit("jei"));
        run.step(&compiling("minimap", 1));
        run.step(&compiling("veinminer", 2));
        run.cancelled();
        // No game was installed: what compiled is in no game. What the build found out stays.
        assert_eq!(states(&run), ["update", "waiting", "waiting", "waiting"]);
        assert_eq!(run.counts().built, 0);
        assert_eq!(run.end, Some(End::Cancelled));
        assert_eq!(run.headline(), "Build cancelled");
    }

    #[test]
    fn a_build_that_stopped_names_the_mods_that_stopped_it() {
        let mut run = run(false);
        run.step(&compiling("minimap", 1));
        run.stopped("Zoom couldn't be built.".into(), &["zoom".into()]);
        assert_eq!(states(&run), ["waiting", "waiting", "waiting", "failed"]);
        assert_eq!(run.counts().failed, 1);
        assert_eq!(run.headline(), "The build stopped");
        // Its row on Mods says so after the window is closed, for the version that failed.
        assert_eq!(
            run.state_of("zoom", "1.0.0"),
            Some(&State::Failed("Zoom couldn't be built.".into()))
        );
        assert_eq!(
            run.state_of("zoom", "1.0.1"),
            None,
            "an updated mod starts over"
        );
        assert_eq!(
            run.state_of("minimap", "1.0.0"),
            None,
            "progress is not live"
        );
    }

    #[test]
    fn a_build_that_ended_with_its_window_closed_is_said_for_a_moment() {
        let mut run = run(false);
        run.step(&unfit("jei"));
        let now = Instant::now();
        // While it runs, and when its window showed the end, the strip has nothing to add.
        assert_eq!(run.said(now), None);
        run.done();
        assert_eq!(run.said(now), None);
        // It ended in the background: the strip says how it went, then lets go.
        run.said_at = Some(now);
        assert_eq!(
            run.said(now).as_deref(),
            Some("3 mods ready · 1 needs an update")
        );
        assert_eq!(run.said(now + SAID_FOR), None);
        // A build that stopped is the play bar's to say, with its way on.
        let mut stopped = self::run(false);
        stopped.stopped("why".into(), &[]);
        stopped.said_at = Some(now);
        assert_eq!(stopped.said(now), None);
    }

    #[test]
    fn plays_build_says_so_and_the_game_starts_when_it_worked() {
        let run = run(true);
        assert_eq!(run.headline(), "Getting ready to play");
        let press_play = || PlayIntent {
            instance: "autumn".into(),
            incarnation: None,
            account: Some("original-account".into()),
        };
        let built = Ok("native-abc".to_owned());
        let version = Some("native-abc");
        let playing = Some(("autumn", version));
        // Auto-launch on success: the same profile on the game just built, and the account
        // that originally pressed Play, independent of who is selected when the build ends.
        let ready = play_on_finish(Some(press_play()), &built, playing).unwrap();
        assert_eq!(ready.account.as_deref(), Some("original-account"));
        // A mod the build skipped doesn't block: the build still gave a game. Closing the
        // window changes none of these: what starts the game is the build's result.
        let mut skipped = self::run(true);
        skipped.step(&unfit("jei"));
        skipped.done();
        assert_eq!(skipped.counts().needs_update, 1);
        assert!(play_on_finish(Some(press_play()), &built, playing).is_some());
        // Cancel, or a build that stopped: no game, so no launch.
        assert!(play_on_finish(
            Some(press_play()),
            &Err(native_mods::Failed::Nothing),
            playing
        )
        .is_none());
        // Cancel pressed, and the build ended well all the same (the press came after its last
        // check): the game it gave is kept and said, and nothing starts.
        let mut late = self::run(true);
        let mut wanted = Some(press_play());
        stop_asked(Some(&mut late), &mut wanted);
        assert!(!late.for_play);
        assert!(play_on_finish(wanted, &built, playing).is_none());
        late.done();
        assert_eq!(late.headline(), "Your mods are ready");
        // Another profile was picked meanwhile, or this one's game changed.
        assert!(play_on_finish(Some(press_play()), &built, Some(("winter", version))).is_none());
        assert!(play_on_finish(Some(press_play()), &built, Some(("autumn", Some("v1")))).is_none());
    }

    #[test]
    fn mods_a_stop_for_worlds_names_wait_for_an_update_not_in_line() {
        let mut run = run(false);
        let ids: Vec<String> = run.lines.iter().map(|l| l.id.clone()).collect();
        run.at_stake(&ids[..1], &[]);
        run.stopped("stop".into(), &[]);
        assert!(matches!(run.lines[0].state, State::NeedsUpdate { .. }));
        assert_eq!(run.counts().needs_update, 1);
    }

    #[test]
    fn a_stopped_build_keeps_the_classified_reasons_for_the_installed_rows() {
        let mut run = run(false);
        let Step::LeftOut(mismatch) = unfit("jei") else {
            unreachable!()
        };
        run.mark_left_out(&mismatch);
        let compile_failure = LeftOut {
            id: "minimap".into(),
            why: Why::DidNotBuild,
            detail: None,
            ..mismatch
        };
        run.mark_left_out(&compile_failure);
        run.stopped(
            "The build stopped.".into(),
            &["jei".into(), "minimap".into()],
        );
        assert!(matches!(
            run.state_of("jei", "1.0.0"),
            Some(State::NeedsUpdate { .. })
        ));
        assert!(
            matches!(run.state_of("minimap", "1.0.0"), Some(State::Failed(why)) if !why.is_empty())
        );
        assert_eq!(run.state_of("jei", "1.0.1"), None);
        assert_eq!(run.state_of("zoom", "1.0.0"), None);
        assert_eq!(run.counts().needs_update, 1);
        assert_eq!(run.counts().failed, 1);
    }
}
