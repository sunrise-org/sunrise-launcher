//! What Play says and does, decided once: the profile header on Home and the status strip on
//! every other page both draw [`View`], so neither can show a state the other lacks.
//!
//! [`Facts`] is what the launcher knows (a download under way, a build's error, a held update),
//! read by [`Launcher::launch_facts`]; [`Facts::view`] is the whole table, with nothing drawn and
//! nothing read, so every row of it is tested. The launch itself is `play_game`'s, which owns
//! the sign-in, install, prepare, build and trust gates.
use super::*;
use crate::sheets::{Sheet, WayOut};

/// What the launcher knows that Play depends on.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Facts {
    /// A game build downloading, and how far it is (0 to 1).
    pub installing: Option<f32>,
    pub preparing: bool,
    /// Native mods are being built.
    pub building: bool,
    pub starting: bool,
    /// The step the start is at, once it has said (`Main::Step`).
    pub step: Option<backend::PlayStep>,
    /// Mods are being added, removed or updated.
    pub mutating: bool,
    /// The game runs.
    pub running: bool,
    /// An account is selected, or the developer's offline Play is on.
    pub signed_in: bool,
    /// The selected profile has an installed build to play.
    pub installed: bool,
    /// The configured data folder could be opened, regardless of any move notice.
    pub located: bool,
    pub client_error: Option<String>,
    pub install_recovery: Option<(std::path::PathBuf, String)>,
    /// The game's own files were not found.
    pub prepare_error: Option<String>,
    pub native_error: Option<String>,
    pub install_error: Option<String>,
    pub crash: Option<String>,
    /// The build the profile stays on can't be built for any more (`build_gone`).
    pub build_gone: Option<instances::Channel>,
    /// The name of the build a held update would move to.
    pub held: Option<String>,
    /// The name of a newer build, for a profile that follows the newest Nightly.
    pub update: Option<String>,
    /// How many mods the game was made without.
    pub mods_off: usize,
}

/// Play itself: what it says, in the order the states win.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Main {
    Installing(f32),
    Preparing,
    Building,
    Starting,
    /// Starting, and the step it is at before the game opens.
    Step(backend::PlayStep),
    ChangingMods,
    /// The game runs: the button stops it.
    Stop,
    SignIn,
    Install,
    Play,
}

/// The one action beside Play: one at most, and one name for each.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Act {
    /// Stops the build of native mods.
    StopBuild,
    /// Opens the problem's whole text: "Fix…" with a way out of it, "Details…" with only more
    /// to read.
    Notice {
        text: String,
        way_out: Option<WayOut>,
    },
    /// "Fix…" for a held update: choose what to do with it.
    Resolve,
    /// "Update game".
    Update,
}

/// What the strip's one button does, off Home.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum StripAct {
    /// One of Play's own actions, under its own name.
    Act(Act),
    StopGame,
    /// Nothing can be done from here: Home has Play and the whole story.
    Home,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct View {
    pub main: Main,
    /// Whether Play can be pressed.
    pub enabled: bool,
    /// One sentence beside Play: a problem, a held update, a newer build.
    pub said: Option<String>,
    /// That sentence is a problem.
    pub failed: bool,
    pub act: Option<Act>,
    /// "2 mods not on this build": said only when nothing else is.
    pub mods_off: Option<usize>,
}

/// The first sentence of `text`, or its first line when that is shorter.
fn first_sentence(text: &str) -> &str {
    let line = text.trim().lines().next().unwrap_or_default();
    match line.find(". ") {
        Some(end) => &line[..=end],
        None => line,
    }
}

/// The step of Play a problem came from: what the plain sentence says could not be done.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    /// Checking the builds, signing in, starting the game's process.
    Start,
    /// Copying the game's own files.
    Prepare,
    /// Building native mods.
    Build,
    /// Downloading and unpacking a game build.
    Install,
    /// The game ran and stopped: the launcher wrote the summary itself.
    Crash,
}

/// The longest sentence said beside Play: more is cut at a word, and opens whole as Details.
const BRIEF: usize = 110;
/// How every message for a connection that could not be made starts (`backend`, `github`,
/// `browser::download`).
const NO_CONNECTION: &str = "No connection";

/// Whether `sentence` is one the launcher could have written for a player: it starts with a
/// capital or a digit, ends as a sentence does, and holds only words, numbers and a
/// sentence's punctuation. An address, a path, an identifier, a code or a bare diagnostic is
/// not one. This lets through what reads as plain words, not what is known to be harmless: a
/// message that fails it is never said, only kept for Details.
fn plain_sentence(sentence: &str) -> bool {
    let starts = sentence
        .chars()
        .next()
        .is_some_and(|c| c.is_uppercase() || c.is_ascii_digit());
    let ends = sentence.ends_with(['.', '!', '?', '…']);
    let words = sentence.split_whitespace().count() >= 3;
    let plain = sentence
        .chars()
        .all(|c| c.is_alphanumeric() || " .,'’\"“”!?…-–():;%".contains(c));
    // "name: value" chains and codes in brackets are a program's, not a sentence's.
    let coded = sentence.matches(": ").count() > 1 || sentence.contains("::");
    starts && ends && words && plain && !coded
}

/// What a step could not do, when nothing more can be said in plain words.
fn stopped(step: Step) -> &'static str {
    match step {
        Step::Start => tr("The game couldn't start."),
        Step::Prepare => tr("The game's files couldn't be prepared."),
        Step::Build => tr("The mods couldn't be built."),
        Step::Install => tr("The game couldn't be installed."),
        Step::Crash => tr("The game stopped."),
    }
}

/// One short, plain sentence for a problem. A connection that could not be made says so, by
/// the step that needed it. A message whose first sentence reads as plain words is the
/// launcher's own and is said as written, cut at a word when long. Anything else (an address,
/// a system error, a path, a program's output, a diagnostic) is not said at all: the step says
/// what it could not do, and the whole message opens word for word behind "Details…".
fn plain(step: Step, raw: &str) -> String {
    if raw.trim_start().starts_with(NO_CONNECTION) {
        return match step {
            Step::Install => tr("No connection: the game couldn't be downloaded."),
            Step::Build => tr("No connection: the mod tools couldn't be downloaded."),
            Step::Prepare => tr("No connection: the game's files couldn't be downloaded."),
            Step::Start | Step::Crash => {
                tr("No connection: Sunrise couldn't check what it needs to play.")
            }
        }
        .to_string();
    }
    let sentence = first_sentence(raw);
    // The system's own words end in its error number, however much like a sentence they read.
    let systems = raw.lines().next().is_some_and(|l| l.contains("(os error"));
    if systems || !plain_sentence(sentence) {
        return stopped(step).to_string();
    }
    if sentence.chars().count() <= BRIEF {
        return sentence.to_string();
    }
    // Cut at a word, never inside one.
    let cut: String = sentence.chars().take(BRIEF).collect();
    let at = cut.rfind(' ').unwrap_or(cut.len());
    format!("{}…", cut[..at].trim_end_matches([',', ';', ':']))
}

impl Facts {
    /// Something is under way that Play waits for (`Launcher::launch_busy`).
    pub(crate) fn busy(&self) -> bool {
        self.mutating
            || self.installing.is_some()
            || self.preparing
            || self.building
            || self.starting
    }

    pub(crate) fn view(&self) -> View {
        let busy = self.busy();
        let main = if let Some(progress) = self.installing {
            Main::Installing(progress)
        } else if self.preparing {
            Main::Preparing
        } else if self.building {
            Main::Building
        } else if self.starting {
            self.step.map_or(Main::Starting, Main::Step)
        } else if self.mutating {
            Main::ChangingMods
        } else if self.running {
            Main::Stop
        } else if !self.signed_in {
            Main::SignIn
        } else if !self.installed {
            Main::Install
        } else {
            Main::Play
        };
        // A held update says so itself and its sheet carries the build's words: only then is
        // the build's error left to it.
        let step = |step: Step, error: &Option<String>| error.clone().map(|e| (step, e));
        let problem = step(Step::Start, &self.client_error)
            .or_else(|| step(Step::Prepare, &self.prepare_error))
            .or_else(|| step(Step::Build, &self.native_error).filter(|_| self.held.is_none()))
            .or_else(|| step(Step::Install, &self.install_error))
            .or_else(|| step(Step::Crash, &self.crash));
        // A problem is said in one plain sentence; whatever else there is opens as a sheet,
        // word for word.
        let brief = problem.as_ref().map(|(step, raw)| plain(*step, raw));
        // The way out belongs to the step whose problem is said, never to another's left over.
        let failed_at = problem.as_ref().map(|(step, _)| *step);
        let problem = problem.map(|(_, raw)| raw);
        let more = brief
            .as_deref()
            .is_some_and(|b| b != problem.as_deref().unwrap_or_default().trim());
        let hold = self
            .held
            .as_ref()
            .map(|to| trf("The update to {build} is on hold", &[("build", to)]));
        let update = self
            .update
            .as_ref()
            .filter(|_| self.installed && !busy && !self.running);
        let recovery = self
            .install_recovery
            .as_ref()
            .filter(|_| !busy && !self.running);
        let failed = (recovery.is_some() || brief.is_some()) && !self.building;
        let said = recovery
            .map(|_| tr("A mod install couldn't be finished.").into())
            .or(brief)
            .filter(|_| !self.building)
            .or_else(|| hold.clone())
            .or_else(|| update.map(|build| trf("{build} is available", &[("build", build)])));
        let way_out = match failed_at {
            Some(Step::Prepare) => Some(WayOut::MinecraftFolder),
            Some(Step::Build) => self.build_gone.filter(|_| !busy).map(WayOut::Newest),
            _ => None,
        };
        let act = if self.building {
            Some(Act::StopBuild)
        } else if let Some((home, detail)) = recovery {
            Some(Act::Notice {
                text: detail.clone(),
                way_out: Some(WayOut::ProfileFolder(home.clone())),
            })
        } else if let Some(text) = problem.filter(|_| way_out.is_some() || more) {
            Some(Act::Notice { text, way_out })
        } else if hold.is_some() && !busy {
            Some(Act::Resolve)
        } else {
            update.map(|_| Act::Update)
        };
        // Mods the game was made without are no problem to raise: one quiet line, at most.
        let mods_off =
            (said.is_none() && !self.building && self.mods_off > 0).then_some(self.mods_off);
        View {
            main,
            enabled: location_allows_action(self.located, busy) && main != Main::SignIn,
            said,
            failed,
            act,
            mods_off,
        }
    }
}

impl Main {
    pub(crate) fn label(self) -> SharedString {
        match self {
            Main::Installing(progress) => trf(
                "Installing · {percent}%",
                &[("percent", &format!("{:.0}", 100. * progress))],
            )
            .into(),
            Main::Preparing => tr("Preparing…").into(),
            Main::Building => tr("Building mods…").into(),
            Main::Starting => tr("Starting…").into(),
            Main::Step(backend::PlayStep::Assets) => tr("Checking game files…").into(),
            Main::Step(backend::PlayStep::Mods) => tr("Updating mods…").into(),
            Main::Step(backend::PlayStep::SignIn) => tr("Signing in…").into(),
            Main::ChangingMods => tr("Changing mods…").into(),
            Main::Stop => tr("Stop game").into(),
            // Signed out, Play stays where it is and can't be pressed; signing in is the account
            // chip's job.
            Main::SignIn => tr("Play").into(),
            Main::Install => tr("Install and play").into(),
            Main::Play => tr("Play").into(),
        }
    }

    /// Why Play can't be pressed, said only as its tooltip: no line is added to the page.
    pub(crate) fn tooltip(self) -> Option<SharedString> {
        matches!(self, Main::SignIn).then(|| tr("Sign in to play").into())
    }

    /// Something Play waits for, as the strip says it off Home.
    fn under_way(self) -> bool {
        matches!(
            self,
            Main::Installing(_)
                | Main::Preparing
                | Main::Building
                | Main::Starting
                | Main::Step(_)
                | Main::ChangingMods
        )
    }
}

impl Act {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Act::StopBuild => tr("Stop"),
            Act::Notice {
                way_out: Some(_), ..
            }
            | Act::Resolve => tr("Fix…"),
            Act::Notice { way_out: None, .. } => tr("Details…"),
            Act::Update => tr("Update game"),
        }
    }
}

impl StripAct {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            StripAct::Act(act) => act.label(),
            StripAct::StopGame => tr("Stop"),
            StripAct::Home => tr("Go to Home"),
        }
    }
}

impl View {
    /// What the strip shows on a page other than Home, and its one action: work under way
    /// first, then what failed, then a held update, then a running game. None when there is
    /// nothing to tell (a newer build and mods left out wait on Home).
    pub(crate) fn strip(&self) -> Option<(SharedString, StripAct)> {
        if self.main.under_way() {
            let act = match &self.act {
                Some(Act::StopBuild) => StripAct::Act(Act::StopBuild),
                _ => StripAct::Home,
            };
            return Some((self.main.label(), act));
        }
        match (&self.said, &self.act) {
            (Some(said), Some(act @ Act::Notice { .. })) if self.failed => {
                Some((said.clone().into(), StripAct::Act(act.clone())))
            }
            (Some(said), _) if self.failed => Some((said.clone().into(), StripAct::Home)),
            (Some(said), Some(Act::Resolve)) => {
                Some((said.clone().into(), StripAct::Act(Act::Resolve)))
            }
            _ if self.main == Main::Stop => {
                Some((tr("The game is running").into(), StripAct::StopGame))
            }
            _ => None,
        }
    }
}

/// A data-folder notice is not a lock. Play and profile switching require access to the
/// configured data and no work that would be interrupted by the action.
pub(crate) fn location_allows_action(located: bool, busy: bool) -> bool {
    located && !busy
}

impl Launcher {
    /// The selected profile's channel, when its last build failed because the build it stays on
    /// has no mod tools any more and another build is the way on (`sdk_cache::way_on`).
    fn build_gone(&self) -> Option<instances::Channel> {
        let error = self.native_error.as_ref()?;
        let instance = self.selected_instance()?;
        let mine = self.native_for.as_deref() == Some(instance.id.as_str());
        (mine
            && instance.version.is_some()
            && native_mods::sdk_cache::way_on(error)
                == Some(native_mods::sdk_cache::WayOn::OtherBuild))
        .then_some(instance.channel)
    }

    pub(super) fn launch_activity(&self) -> Facts {
        Facts {
            installing: self.installing.as_ref().map(|(_, progress)| *progress),
            preparing: self.preparing,
            building: self.native_step.is_some(),
            starting: self.starting_client.is_some() || self.play_admission.is_some(),
            step: self.play_step,
            mutating: self.mutating() || self.pinning_build,
            running: self.client.is_some(),
            ..Facts::default()
        }
    }

    pub(crate) fn launch_facts(&self) -> Facts {
        let active = self.selected_instance();
        let launch = self.ui.home.mods.launch();
        let held = self.held_for_launch(launch.and_then(|summary| summary.update_check.as_deref()));
        Facts {
            signed_in: self.accounts.selected().is_some()
                || sunrise_launcher::developer_offline::selected(),
            installed: active.and_then(|i| self.version_of(i)).is_some(),
            located: !self.location_unavailable,
            client_error: self.client_error.clone(),
            install_recovery: self
                .play_install_recovery
                .as_ref()
                .filter(|(home, _)| active.is_some_and(|i| i.home == *home))
                .cloned(),
            prepare_error: self.prepare_error.clone(),
            native_error: self.native_error.clone(),
            install_error: self.install_error.as_ref().map(|(_, e)| e.clone()),
            crash: self.crash.as_ref().map(|c| c.summary.clone()),
            build_gone: self.build_gone(),
            held: held.map(|h| h.to),
            update: self
                .game_update()
                .map(|r| crate::sheets::build_name(&r.tag)),
            mods_off: launch.map_or(0, |summary| summary.mods_off),
            ..self.launch_activity()
        }
    }

    /// Play was pressed: it stops a running game, else starts the selected profile.
    pub(crate) fn press_play(&mut self) {
        if let Some(client) = &self.client {
            client.kill();
        } else {
            self.quick_play = None;
            self.joining = None;
            self.play_game();
        }
    }

    pub(crate) fn launch_act(&mut self, act: &Act) {
        match act {
            Act::StopBuild => self.stop_build(),
            Act::Notice { text, way_out } => self.open_notice(text.clone(), way_out.clone()),
            Act::Resolve => self.open_sheet(Sheet::Resolve),
            Act::Update => {
                let (Some(release), Some(instance)) =
                    (self.game_update().cloned(), self.selected_instance())
                else {
                    return;
                };
                if self.native_state_of(instance) != native_status::State::NoMods {
                    // Never overwrite a working native build while staging its candidate.
                    self.instance_install = Some(release.tag.clone());
                    self.install(&release);
                } else {
                    self.install_update(release);
                }
            }
        }
    }

    pub(crate) fn strip_act(&mut self, act: &StripAct) {
        match act {
            StripAct::Act(act) => self.launch_act(act),
            StripAct::StopGame => {
                if let Some(client) = &self.client {
                    client.kill();
                }
            }
            StripAct::Home => self.navigate(crate::frame::Page::Home),
        }
    }
}

#[cfg(test)]
#[path = "launch_state/tests.rs"]
mod tests;
