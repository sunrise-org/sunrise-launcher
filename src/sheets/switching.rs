use super::*;
use crate::widgets::button::secondary;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use sunrise_launcher::{
    browser::sunrise::{
        switch_profile_build_v2, switch_provider_v2, BuildChoice, InstallError, InstallHooks,
        InstallProgress, ProviderSwitchPreview, SwitchDecision, SwitchHooks, SwitchQuestion,
        Switched, Target, V2Hooks, V2InstallOptions, V2ProviderSwitchHooks, V2SwitchHooks,
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
    secondary(id, tr("Hide")).on_click(cx.listener(|this, _, _, cx| {
        this.close_sheet();
        cx.notify();
    }))
}

/// The selected profile and build survive download and preparation, not a new read of Home.
struct Pending {
    target: Target,
    identity: Identity,
    choice: Option<BuildChoice>,
}

#[cfg(test)]
mod tests;
