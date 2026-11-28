use super::*;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use sunrise_launcher::{
    browser::sunrise::{
        switch_profile_build_v2, switch_provider_v2, BuildChoice, InstallError, InstallHooks,
        InstallProgress, ProviderSwitchPreview, SwitchDecision, SwitchHooks, SwitchPolicy,
        SwitchQuestion, Switched, Target, V2Hooks, V2InstallOptions, V2ProviderSwitchHooks,
        V2SwitchHooks,
    },
    managed_play::{self, Active, FirstBuild, Profile, ReceiptHint},
    profile_txn::Identity,
};

mod build_switch;
mod provider_switch;
mod receipt;

/// A switch the player can start from another port's row: the in-game ID, and both publishers
/// as the player reads them. The backend re-reads the release from the signed index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PortSwitch {
    pub(crate) mod_id: String,
    pub(crate) mod_name: String,
    pub(crate) from_repo: String,
    pub(crate) from_version: String,
    pub(crate) to_project_id: String,
    pub(crate) to_repo: String,
    pub(crate) to_version: String,
}

#[derive(Default)]
pub(crate) struct State {
    pub(super) cache_download: Option<String>,
    running: Option<Running>,
    provider: Option<ProviderRunning>,
    ack: bool,
    receipt: Option<ReceiptRead>,
}

struct ProviderRunning {
    switch: PortSwitch,
    profile_name: String,
    build_label: String,
    progress: InstallProgress,
    eta: crate::eta::StageEta,
    stop: native_mods::Cancellation,
    question: Option<(Box<ProviderSwitchPreview>, Sender<bool>)>,
}

struct ReceiptRead {
    home: PathBuf,
    hint: Result<Option<ReceiptHint>, String>,
    ticket: Arc<()>,
    status: Load<Option<(Identity, Active)>>,
}

struct Running {
    tag: String,
    /// The profile and the build it played when the switch began, shown until the switch ends.
    from: (PathBuf, Option<String>),
    back: Option<Sheet>,
    pending: Option<Pending>,
    progress: InstallProgress,
    eta: crate::eta::StageEta,
    stop: native_mods::Cancellation,
    question: Option<(Box<SwitchQuestion>, Sender<SwitchDecision>)>,
    /// This switch's own launcher lines: the ring's revision it has read up to (where the
    /// switch began, at first), what it read, scrubbed, and where its box is scrolled to.
    log_from: u64,
    log: Vec<String>,
    log_scroll: gpui::ScrollHandle,
    /// The step's own counted line, while the step gives one; cleared when the step changes.
    detail: Option<String>,
    /// This switch's own lines and counts: its download, preparation and profile change log
    /// in it, and its step text reaches only this dialog.
    op: sunrise_launcher::launcher_log::Operation,
    /// The player answered its question with Cancel: how it ended says so.
    declined: bool,
    /// How it treats the player's mods: set when it begins, before any worker can start.
    policy: SwitchPolicy,
    /// Set by the worker when the policy stopped it.
    refused: Arc<std::sync::atomic::AtomicBool>,
}

/// A switch under way, as the status strip lists it beside the launcher's other work.
pub(crate) struct Activity {
    /// What the switch does: "Switch to Nov 17".
    pub(crate) what: String,
    pub(crate) progress: InstallProgress,
    pub(crate) eta: Option<std::time::Duration>,
    pub(crate) stopping: bool,
    /// The switch waits for the player's answer in its sheet.
    pub(crate) asking: bool,
    /// The game build switch, not a mod's provider switch.
    pub(crate) build: bool,
}

impl Launcher {
    /// The build and provider switches under way, whether their sheet is open or hidden.
    pub(crate) fn switch_activities(&self) -> Vec<Activity> {
        let now = std::time::Instant::now();
        let state = &self.ui.sheets.switching;
        let build = state.running.as_ref().map(|run| Activity {
            what: trf("Switch to {build}", &[("build", &build_name(&run.tag))]),
            progress: run.progress,
            eta: run.eta.remaining(now),
            stopping: run.stop.is_stopped(),
            asking: run.question.is_some(),
            build: true,
        });
        let provider = state.provider.as_ref().map(|run| Activity {
            what: trf(
                "Switching {mod} to {repo}",
                &[("mod", &run.switch.mod_name), ("repo", &run.switch.to_repo)],
            ),
            progress: run.progress,
            eta: run.eta.remaining(now),
            stopping: run.stop.is_stopped(),
            asking: run.question.is_some(),
            build: false,
        });
        build.into_iter().chain(provider).collect()
    }

    /// The game build a switch downloads: the switch's job says it, not a second one.
    pub(crate) fn switch_download(&self) -> Option<&str> {
        self.ui.sheets.switching.cache_download.as_deref()
    }

    /// Closing a switch's sheet hides it: the switch goes on and the status strip shows it. Only a
    /// question on screen is answered no, as closing any sheet answers one.
    pub(in crate::sheets) fn hide_switch(&mut self) {
        let state = &self.ui.sheets.switching;
        let build = (state.running.as_ref()).is_some_and(|run| run.question.is_some());
        let provider = (state.provider.as_ref()).is_some_and(|run| run.question.is_some());
        if build {
            self.cancel_switch();
        }
        if provider {
            self.cancel_provider_switch();
        }
    }

    /// The sheet of the switch that waits, shown again from the status strip.
    pub(crate) fn show_switch(&mut self, build: bool) {
        self.open_sheet(if build {
            Sheet::Switching
        } else {
            Sheet::SwitchProvider
        });
    }
}

/// Hide on a switch's sheet: the switch goes on in the status strip.
fn hide_button(id: &'static str, cx: &mut Context<Launcher>) -> Stateful<Div> {
    button(id, tr("Hide")).on_click(cx.listener(|this, _, _, cx| {
        this.close_sheet();
        cx.notify();
    }))
}

/// How a switch ended, in its log: what it did, not what was last pressed. A cancel counts
/// only when it was taken and the switch stopped for it; a switch that failed for another
/// reason while a cancel was pending says why it failed.
fn switch_ending(result: &Result<Switched, String>, cancel_taken: bool, declined: bool) -> String {
    let cancelled = is_cancel;
    match result {
        Ok(Switched::Changed(_)) => "done".into(),
        Ok(Switched::AlreadySelected) => "already on this build, nothing changed".into(),
        Ok(Switched::Stayed) if declined => {
            "the player declined the changes it asked about, nothing changed".into()
        }
        Ok(Switched::Stayed) if cancel_taken => "cancelled, nothing changed".into(),
        Ok(Switched::Stayed) => "nothing changed".into(),
        Err(error) if cancel_taken && cancelled(error) => "cancelled, nothing changed".into(),
        Err(error) => format!(
            "failed: {}",
            sunrise_launcher::launcher_log::safe_error(error)
        ),
    }
}

/// Whether `error` is what a cancelled step returns. Only such an error counts as a cancel: a
/// step that failed for another reason while a cancel was pending keeps its own error.
fn is_cancel(error: &str) -> bool {
    error == native_mods::CANCELLED
        || error == sunrise_launcher::downloads::CANCELLED
        || error == tr("Switch cancelled.")
}

/// What the player is shown of how a switch ended: a cancel ends it quietly; any other failure
/// is shown, even with a cancel pending when it came in.
fn shown_outcome(result: Result<Switched, String>, stopped: bool) -> Result<Switched, String> {
    match result {
        Err(error) if stopped && is_cancel(&error) => Ok(Switched::Stayed),
        result => result,
    }
}

/// The lines a switch's log box keeps: more than a download and a preparation write.
const SWITCH_LOG_KEEP: usize = 2000;
/// The log box's height: nine lines, so the dialog keeps one size while lines arrive.
const SWITCH_LOG_HEIGHT: f32 = 9. * 18. + 16.;

/// The selected profile and build survive download and preparation, not a new read of Home.
struct Pending {
    target: Target,
    identity: Identity,
    choice: Option<BuildChoice>,
}

#[cfg(test)]
mod tests;
