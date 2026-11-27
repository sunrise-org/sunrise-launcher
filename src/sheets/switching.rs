use super::*;
use crate::mods::PortSwitch;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use sunrise_launcher::{
    browser::sunrise::{
        install_target, save_build_choice, switch_profile_build_v2, switch_provider_v2,
        BuildChoice, InstallError, InstallHooks, InstallProgress, ProviderSwitchPreview,
        SwitchDecision, SwitchHooks, SwitchPolicy, SwitchQuestion, Switched, V2Hooks,
        V2InstallOptions, V2ProviderSwitchHooks, V2SwitchHooks,
    },
    managed_play::{self, Active, FirstBuild, Profile, ReceiptHint},
    profile_txn::Identity,
};

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
    /// The policy its worker started with, once it has.
    started_with: Option<SwitchPolicy>,
    /// The build choice its worker started with, once it has.
    started_choice: Option<BuildChoice>,
    /// Set by the worker when the policy stopped it.
    refused: Arc<std::sync::atomic::AtomicBool>,
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
    instance: instances::Instance,
    identity: Identity,
    choice: Option<BuildChoice>,
}

impl Launcher {
    pub(crate) fn download_for_switch(&mut self, release: &backend::Release) {
        if self.installing.is_some()
            || self.installed.contains(&release.tag)
            || self.removing.contains(&release.tag)
            || release.asset.is_none()
        {
            return;
        }
        self.ui.sheets.switching.cache_download = Some(release.tag.clone());
        self.install(release);
    }

    pub(crate) fn finish_switch_download(
        &mut self,
        tag: &str,
        result: &Result<backend::InstallOutcome, String>,
    ) -> bool {
        if self
            .ui
            .sheets
            .switching
            .cache_download
            .take_if(|current| current == tag)
            .is_none()
        {
            return false;
        }
        if let Some(running) = &self.ui.sheets.switching.running {
            let pending = running.pending.as_ref().expect("switch download is queued");
            let home = pending.instance.home.clone();
            match result {
                Err(error) => self.switch_finished(&home, Err(error.clone())),
                Ok(_) if running.stop.is_stopped() => {
                    self.switch_finished(&home, Ok(Switched::Stayed))
                }
                Ok(_) => self.prepare_switch(),
            }
        } else {
            self.ui.sheets.error = result.as_ref().err().cloned();
        }
        true
    }

    pub(crate) fn switch_download_progress(&mut self, tag: &str, phase: backend::InstallPhase) {
        if self.ui.sheets.switching.cache_download.as_deref() == Some(tag) {
            self.switch_progress(phase.into());
        }
    }

    /// The build `home` played when a switch running on it began.
    pub(crate) fn switch_base(&self, home: &Path) -> Option<&str> {
        let running = self.ui.sheets.switching.running.as_ref()?;
        (running.from.0 == home)
            .then_some(running.from.1.as_deref())
            .flatten()
    }

    pub(crate) fn switch_busy(&self) -> bool {
        self.ui.sheets.switching.running.is_some()
            || self.ui.sheets.switching.provider.is_some()
            || self.ui.sheets.switching.ack
    }

    pub(crate) fn begin_switch(&mut self, tag: String, choice: Option<BuildChoice>) {
        self.begin_switch_as(tag, choice, SwitchPolicy::Ask);
    }

    /// [`Self::begin_switch`] under `policy`, which the switch holds from its first moment: a
    /// prepared target starts its worker before this returns (review of #616).
    pub(crate) fn begin_switch_as(
        &mut self,
        tag: String,
        choice: Option<BuildChoice>,
        policy: SwitchPolicy,
    ) {
        if self.launch_busy() || self.client.is_some() || self.removing.contains(&tag) {
            return;
        }
        let Some(instance) = self.selected_instance().cloned() else {
            return;
        };
        let identity = match install_target(&instance) {
            Ok(identity) => identity,
            Err(error) => {
                self.open_notice(error, Some(WayOut::ProfileFolder(instance.home)));
                return;
            }
        };
        // Changing only the channel/pin preference when the profile is already on `tag` (and has
        // no pending recovery or first-Play bootstrap) needs no switch modal, registry fetch, or
        // mod rebuild. Keep artifact hashing off the UI thread.
        let unplayed_target = self
            .target_version(instance.version.as_ref(), instance.channel)
            .map(String::as_str);
        let already_there = self.profile_target(&instance).as_deref() == Some(tag.as_str());
        let start = switch_start(policy, already_there, || {
            can_fast_save_choice(
                &instance,
                &tag,
                unplayed_target,
                self.installed.contains(&tag),
            )
        });
        match start {
            SwitchStart::Refuse => {
                self.open_notice(
                    trf(
                        "This profile is already on {build}, so nothing changed.",
                        &[("build", &build_name(&tag))],
                    ),
                    None,
                );
                return;
            }
            SwitchStart::SaveChoiceOnly => {
                if let Some(choice) = choice {
                    if let Err(error) = save_build_choice(&instance.id, &tag, choice) {
                        self.open_notice(error, Some(WayOut::ProfileFolder(instance.home)));
                        return;
                    }
                    self.reload_instances();
                }
                self.close_sheet();
                return;
            }
            SwitchStart::Switch => {}
        }
        let release = self.release_to_install(&tag);
        let stop = self.stage_switch(tag.clone(), instance, identity, choice, policy);
        if let Some(release) = release {
            self.ui.sheets.switching.cache_download = Some(tag);
            self.switch_progress(InstallProgress::CheckingBuild);
            self.install_cancellable(&release, Some(stop));
        } else {
            self.prepare_switch();
        }
    }

    fn stage_switch(
        &mut self,
        tag: String,
        instance: instances::Instance,
        identity: Identity,
        choice: Option<BuildChoice>,
        policy: SwitchPolicy,
    ) -> native_mods::Cancellation {
        let stop = native_mods::Cancellation::default();
        let from = (instance.home.clone(), self.profile_target(&instance));
        self.ui.sheets.switching.running = Some(Running {
            tag: tag.clone(),
            from,
            back: self.ui.sheet,
            pending: Some(Pending {
                instance,
                identity,
                choice,
            }),
            progress: InstallProgress::PreparingBuild,
            eta: crate::eta::StageEta::default(),
            stop: stop.clone(),
            question: None,
            log_from: sunrise_launcher::launcher_log::since(u64::MAX).0,
            log: Vec::new(),
            log_scroll: gpui::ScrollHandle::new(),
            detail: None,
            declined: false,
            op: sunrise_launcher::launcher_log::Operation::new({
                let tx = self.tx.clone();
                move |text| {
                    let _ = tx.send(Msg::SwitchDetail(text));
                }
            }),
            policy,
            started_with: None,
            started_choice: None,
            refused: Arc::default(),
        });
        self.switch_log_line(format!("Switching to {tag}: started"));
        self.open_sheet(Sheet::Switching);
        stop
    }

    fn prepare_switch(&mut self) {
        let running = self
            .ui
            .sheets
            .switching
            .running
            .as_ref()
            .expect("switch queued");
        if backend::check_prepared(&running.tag) {
            self.start_switch_worker();
        } else {
            let (tag, stop) = (running.tag.clone(), running.stop.clone());
            self.switch_progress(InstallProgress::Minecraft(
                backend::PrepareStep::Files,
                None,
            ));
            self.prepare_version_cancellable(tag, backend::MinecraftFrom::Install, Some(stop));
        }
    }

    /// A line from the preparation a switch waits on: the dialog follows its step, and the
    /// files done while Mojang's download runs.
    pub(crate) fn follow_switch_preparation(&mut self) {
        if self
            .ui
            .sheets
            .switching
            .running
            .as_ref()
            .is_some_and(|run| matches!(run.progress, InstallProgress::Minecraft(..)))
        {
            // Through the same transition as every other stage, so a step's count never stays
            // beside the next step's label.
            self.switch_progress(InstallProgress::Minecraft(
                backend::prepare_step(),
                sunrise_launcher::mojang::progress(),
            ));
        }
    }

    pub(crate) fn finish_switch_preparation(&mut self) {
        let Some(running) = self.ui.sheets.switching.running.as_ref() else {
            return;
        };
        let Some(pending) = running.pending.as_ref() else {
            return;
        };
        let error = self.prepare_error.take();
        let home = pending.instance.home.clone();
        if let Some(error) = error {
            self.switch_finished(&home, Err(error));
        } else if running.stop.is_stopped() {
            self.switch_finished(&home, Ok(Switched::Stayed));
        } else {
            self.start_switch_worker();
        }
    }

    fn start_switch_worker(&mut self) {
        // Both routes first check the profile's mods. In particular, the donor's counted
        // detail belongs to the install, not to the switch worker.
        self.switch_progress(InstallProgress::Checking);
        let running = self
            .ui
            .sheets
            .switching
            .running
            .as_mut()
            .expect("switch queued");
        let Pending {
            instance,
            identity,
            choice,
        } = running.pending.take().expect("switch queued");
        let (tag, stop, op) = (
            running.tag.clone(),
            running.stop.clone(),
            running.op.clone(),
        );
        let (policy, refused) = (running.policy, running.refused.clone());
        running.started_with = Some(policy);
        running.started_choice = choice;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            // The steps inside the switch (sealing and checking the game files, the mod set)
            // count what they do into the switch's own operation, so its dialog shows those
            // counts and not a bare "Getting ready…", and no other job's.
            let _in = op.enter();
            let hooks = SwitchHooks {
                cancellation: &stop,
                progress: &|stage| {
                    let _ = tx.send(Msg::SwitchProgress(stage));
                },
                at_cutoff: &|| Ok(()),
                approve: &|question| {
                    if stop.is_stopped() {
                        return Ok(SwitchDecision::Cancel);
                    }
                    if let Some(decision) = policy.answer(question) {
                        refused.store(true, std::sync::atomic::Ordering::Relaxed);
                        sunrise_launcher::launcher_log::line(format!(
                            "Switching to {tag}: stopped before any change, it would leave out {}",
                            question.wanted_left_out.join(", ")
                        ));
                        return Ok(decision);
                    }
                    let (answer, reply) = std::sync::mpsc::channel();
                    tx.send(Msg::SwitchQuestion(Box::new(question.clone()), answer))
                        .map_err(|_| tr("Switch cancelled.").to_owned())?;
                    Ok(reply.recv().unwrap_or(SwitchDecision::Cancel))
                },
            };
            // Mods installed before the signed registry keep their exact bytes through it.
            let outcome = switch_profile_build_v2(
                &instance,
                identity,
                &tag,
                choice,
                V2SwitchHooks {
                    switch: hooks,
                    confirm_publisher_change: Some(&|question| {
                        let (answer, reply) = std::sync::mpsc::channel();
                        tx.send(Msg::PublisherChangeQuestion(
                            Box::new(question.clone()),
                            answer,
                        ))
                        .is_ok()
                            && reply.recv().unwrap_or(false)
                    }),
                    pin_dir: None,
                    recover_held: policy.recover_held(),
                },
            );
            let _ = tx.send(Msg::Switched(instance.home, Box::new(outcome)));
        });
    }

    pub(super) fn cancel_switch(&mut self) {
        let Some(running) = &mut self.ui.sheets.switching.running else {
            return;
        };
        // The request is logged with whether it was taken: past the publication cutoff the
        // switch finishes and Stop is ignored. How the switch ended follows from the worker.
        let _in = running.op.enter();
        let tag = running.tag.clone();
        if running.stop.is_stopped() {
            return;
        }
        if running.progress != InstallProgress::Publishing {
            running.stop.stop();
        }
        if running.stop.is_stopped() {
            sunrise_launcher::launcher_log::line(format!(
                "Switching to {tag}: the player asked to cancel"
            ));
            if let Some((_, reply)) = running.question.take() {
                let _ = reply.send(SwitchDecision::Cancel);
            }
        } else {
            sunrise_launcher::launcher_log::line(format!(
                "Switching to {tag}: cancel not taken, the change is already being applied"
            ));
        }
    }

    /// Home's switch to the build the profile's mods need: kept on that build, every held-off
    /// mod the player had on turned back on, and stopped before any change when the fresh check
    /// would still leave one out.
    pub(crate) fn begin_mods_switch(&mut self, tag: String) {
        self.begin_switch_as(tag, Some(BuildChoice::Keep), SwitchPolicy::KeepEveryMod);
    }

    /// A line in the running switch's own log (its dialog's box and launcher.log).
    pub(crate) fn switch_log_line(&self, text: String) {
        match &self.ui.sheets.switching.running {
            Some(running) => {
                let _in = running.op.enter();
                sunrise_launcher::launcher_log::line(text);
            }
            None => sunrise_launcher::launcher_log::line(text),
        }
    }

    /// The running switch's operation when it is the one for `tag`: its install and
    /// preparation log into it.
    pub(crate) fn switch_operation(
        &self,
        tag: &str,
    ) -> Option<sunrise_launcher::launcher_log::Operation> {
        self.ui
            .sheets
            .switching
            .running
            .as_ref()
            .filter(|running| running.tag == tag)
            .map(|running| running.op.clone())
    }

    pub(crate) fn switch_progress(&mut self, stage: InstallProgress) {
        if let Some(running) = &mut self.ui.sheets.switching.running {
            // Each new step once in launcher.log, so the dialog's log box and a bug report show
            // which step a switch is in, and since when, even a step that counts nothing.
            let key = crate::eta::install_stage_key(stage);
            if key != crate::eta::install_stage_key(running.progress) {
                running.detail = None;
                let _in = running.op.enter();
                sunrise_launcher::launcher_log::line(format!(
                    "Switching to {}: {}",
                    running.tag,
                    crate::mods_page::install::stage_label(stage)
                ));
            }
            running.progress = stage;
            running.eta.update(
                crate::eta::install_stage_key(stage),
                crate::mods_page::install::stage_fraction(stage),
                Instant::now(),
            );
        }
    }

    /// A counted line from the switch's current step.
    pub(crate) fn switch_detail(&mut self, text: String) {
        if let Some(running) = &mut self.ui.sheets.switching.running {
            running.detail = Some(text);
        }
    }

    pub(crate) fn switch_question(
        &mut self,
        question: Box<SwitchQuestion>,
        reply: Sender<SwitchDecision>,
    ) {
        if let Some(running) = self
            .ui
            .sheets
            .switching
            .running
            .as_mut()
            .filter(|run| self.ui.sheet == Some(Sheet::Switching) && !run.stop.is_stopped())
        {
            running.question = Some((question, reply));
            self.ui.sheets.details = false;
        } else {
            let _ = reply.send(SwitchDecision::Cancel);
        }
    }

    pub(crate) fn switch_finished(
        &mut self,
        home: &std::path::Path,
        result: Result<Switched, String>,
    ) {
        // How it ended, in its own log before the dialog and its log go: a failure, a refused
        // cancel and a cancel all say so.
        if let Some(running) = &self.ui.sheets.switching.running {
            let _in = running.op.enter();
            let refused = running.refused.load(std::sync::atomic::Ordering::Relaxed);
            let ended = if refused && matches!(result, Ok(Switched::Stayed)) {
                "stopped before any change, it would have left out wanted mods".to_owned()
            } else {
                switch_ending(&result, running.stop.is_stopped(), running.declined)
            };
            sunrise_launcher::launcher_log::line(format!("Switching to {}: {ended}", running.tag));
        }
        self.pull_switch_log();
        let running = self.ui.sheets.switching.running.take();
        let refused = running
            .as_ref()
            .is_some_and(|run| run.refused.load(std::sync::atomic::Ordering::Relaxed));
        let refused_build = running.as_ref().map(|run| build_name(&run.tag));
        let stopped = running.as_ref().is_some_and(|run| run.stop.is_stopped());
        let back = running.and_then(|run| run.back);
        let result = shown_outcome(result, stopped);
        self.refresh_installed();
        if matches!(result, Ok(Switched::Changed(_) | Switched::AlreadySelected)) {
            self.refresh_instances_list();
        }
        if self.selected_instance().is_none_or(|i| i.home != home) {
            return;
        }
        let visible = self.ui.sheet == Some(Sheet::Switching);
        match result {
            Ok(Switched::Changed(active)) => {
                if self
                    .updates
                    .sunrise
                    .as_ref()
                    .is_some_and(|r| r.tag == active.base())
                {
                    self.updates.sunrise = None;
                }
                if active.pending_changes().is_some() {
                    self.ui.sheets.switching.receipt = None;
                    self.open_sheet(Sheet::BuildChanges);
                } else if visible {
                    self.close_sheet();
                }
            }
            Ok(Switched::Stayed) if refused => self.open_notice(
                trf(
                    "Not every mod could be kept on {build}, so nothing changed.",
                    &[("build", &refused_build.unwrap_or_default())],
                ),
                None,
            ),
            Ok(Switched::Stayed) => {
                if visible {
                    if let Some(back) = back {
                        self.open_sheet(back);
                    } else {
                        self.close_sheet();
                    }
                }
            }
            Ok(Switched::AlreadySelected) => {
                if visible {
                    self.close_sheet();
                }
            }
            Err(error) if visible => {
                self.open_notice(error, Some(WayOut::ProfileFolder(home.into())))
            }
            Err(_) => (),
        }
    }

    /// From the tick, while a switch runs: the launcher lines logged since it began, so its
    /// dialog shows only this install's log. The box follows the newest line until the player
    /// scrolls up, as the Logs panel does. Whether any arrived, so the window redraws only then.
    pub(crate) fn pull_switch_log(&mut self) -> bool {
        let Some(running) = self.ui.sheets.switching.running.as_mut() else {
            return false;
        };
        let (revision, lines) =
            sunrise_launcher::launcher_log::since_in(running.log_from, running.op.id());
        running.log_from = revision;
        if lines.is_empty() {
            return false;
        }
        let scroll = &running.log_scroll;
        let at_bottom = f32::from(scroll.offset().y) + f32::from(scroll.max_offset().y) < 1.;
        running.log.extend(
            lines
                .iter()
                .map(|line| sunrise_launcher::launcher_log::scrub(line)),
        );
        let over = running.log.len().saturating_sub(SWITCH_LOG_KEEP);
        running.log.drain(..over);
        if at_bottom {
            running.log_scroll.scroll_to_bottom();
        }
        true
    }

    /// The switch's log, drawn as the Logs panel draws its list: a fixed box that scrolls.
    fn switch_log(&self, running: &Running) -> Div {
        let rows = running.log.iter().enumerate().map(|(n, line)| {
            div().child(crate::selectable::Selectable::styled(
                ("switch-log-line", n),
                gpui::StyledText::new(line.clone()),
            ))
        });
        let empty = running
            .log
            .is_empty()
            .then(|| tr("Nothing has been logged yet."));
        let list = div()
            .id("switch-log-list")
            .overflow_y_scroll()
            .px(px(12.))
            .py(px(8.))
            .font_family(crate::theme::MONO)
            .text_size(px(tokens::SMALL))
            .line_height(px(18.))
            .text_color(rgb(palette().muted))
            .children(empty)
            .children(rows);
        let list =
            crate::scrollbar::Scrollbar::with_handle("switch-log", running.log_scroll.clone())
                .wrap(list, &self.theme)
                .flex_1();
        div()
            .mt(px(14.))
            .h(px(SWITCH_LOG_HEIGHT))
            .flex()
            .flex_col()
            .rounded(crate::tokens::r(crate::tokens::R_CARD))
            .border_1()
            .border_color(rgb(palette().divider))
            .bg(rgb(palette().background))
            .overflow_hidden()
            .child(list)
    }

    /// What the switch dialog's one line says while a switch runs: the step and the time left,
    /// or that it is stopping or being applied. Home says the same under the Play row, so the
    /// step can be followed with the dialog closed. Nothing while the switch asks a question.
    pub(crate) fn switch_line(&self) -> Option<String> {
        use crate::mods_page::install::stage_label;
        let running = self.ui.sheets.switching.running.as_ref()?;
        if running.question.is_some() {
            return None;
        }
        Some(if running.stop.is_stopped() {
            tr("Cancelling…").to_string()
        } else if running.progress == InstallProgress::Publishing {
            tr("Applying the switch.").to_string()
        } else {
            let now = Instant::now();
            let remaining = match running.progress {
                InstallProgress::Minecraft(backend::PrepareStep::Prepare, _) => {
                    match backend::prepare_step_state() {
                        (backend::PrepareStep::Prepare, Some(started)) => {
                            crate::eta::expected_prepare_duration().and_then(|expected| {
                                crate::eta::remaining_from_expected(started, expected, now)
                            })
                        }
                        _ => None,
                    }
                }
                _ => running.eta.remaining(now),
            };
            let step = match (&running.detail, running.progress) {
                (
                    Some(detail),
                    InstallProgress::DownloadingBuild(_) | InstallProgress::PreparingBuild,
                ) => detail.clone(),
                (Some(detail), stage) => format!("{} · {detail}", stage_label(stage)),
                (None, stage) => stage_label(stage),
            };
            crate::eta::with_eta(step, remaining)
        })
    }

    pub(super) fn switch_sheet(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        use crate::mods_page::install::stage_fraction;
        let running = self
            .ui
            .sheets
            .switching
            .running
            .as_ref()
            .expect("Switch sheet follows its worker");
        let build = build_name(&running.tag);
        let body = if let Some((question, _)) = &running.question {
            let show = self.ui.sheets.details;
            let omissions = question.has_omissions();
            let one = question.held.len() + question.unproved.len() == 1;
            let answer = |decision| {
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Some((_, reply)) =
                        this.ui
                            .sheets
                            .switching
                            .running
                            .as_mut()
                            .and_then(|running| {
                                running.declined = decision == SwitchDecision::Cancel;
                                running.question.take()
                            })
                    {
                        if decision == SwitchDecision::Cancel {
                            this.switch_log_line(
                                "Switch: the player declined the changes it asked about".into(),
                            );
                        }
                        let _ = reply.send(decision);
                    }
                    cx.notify();
                })
            };
            let has_mod_block = omissions || !question.removed.is_empty();
            let mod_block = has_mod_block.then(|| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .children(omissions.then(|| muted(question.omission_summary())))
                    .child(
                        div()
                            .id("switch-mods")
                            .max_h(px(184.))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap(px(5.))
                            .children((omissions && !show).then(|| {
                                let mut names: Vec<_> = question
                                    .held
                                    .iter()
                                    .chain(&question.unproved)
                                    .map(String::as_str)
                                    .collect();
                                names.sort_unstable();
                                div().child(names.join(", "))
                            }))
                            .children(
                                show.then(|| question.hold_lines().map(|line| div().child(line)))
                                    .into_iter()
                                    .flatten(),
                            )
                            .children(question.removed.iter().map(|(name, reason)| {
                                div().child(if show {
                                    trf(
                                        "{mod} was taken down: {reason}",
                                        &[("mod", name), ("reason", reason)],
                                    )
                                } else {
                                    trf(
                                        "{mod} was taken down and will be removed.",
                                        &[("mod", name)],
                                    )
                                })
                            })),
                    )
            });
            let world_block = question.worlds.needed().then(|| {
                let mut block = div().flex().flex_col().gap(px(6.));
                if has_mod_block {
                    block = block
                        .pt(px(12.))
                        .border_t_1()
                        .border_color(rgb(palette().line));
                }
                block
                    .child(muted(tr(
                        "Your worlds stay in place. Check what could change before you switch.",
                    )))
                    .child(
                        div()
                            .id("switch-worlds")
                            .max_h(px(150.))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap(px(5.))
                            .children(
                                question
                                    .world_lines()
                                    .into_iter()
                                    .map(|line| div().child(line)),
                            ),
                    )
            });
            div()
                .flex()
                .flex_col()
                .child(title(trf("Switch to {build}?", &[("build", &build)])))
                .children(mod_block)
                .children(world_block)
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .justify_end()
                        .gap(px(8.))
                        .children((omissions || !question.removed.is_empty()).then(|| {
                            button(
                                "switch-why",
                                if show {
                                    tr("Hide details")
                                } else {
                                    tr("Show why")
                                },
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.ui.sheets.details ^= true;
                                cx.notify();
                            }))
                        }))
                        .child(
                            button("switch-stay", tr("Cancel"))
                                .on_click(answer(SwitchDecision::Cancel)),
                        )
                        .children(omissions.then(|| {
                            button(
                                "switch-remove",
                                if one {
                                    tr("Switch and remove it")
                                } else {
                                    tr("Switch and remove them")
                                },
                            )
                            .text_color(rgb(palette().danger))
                            .on_click(answer(SwitchDecision::Remove))
                        }))
                        .child(
                            crate::frame::primary(
                                "switch-anyway",
                                if omissions && one {
                                    tr("Switch and turn it off")
                                } else if omissions {
                                    tr("Switch and turn them off")
                                } else {
                                    tr("Switch anyway")
                                },
                            )
                            .on_click(answer(SwitchDecision::TurnOff)),
                        ),
                )
        } else {
            let stopping = running.stop.is_stopped();
            let publishing = running.progress == InstallProgress::Publishing;
            let cancel = button("cancel-switch", tr("Cancel"));
            // The step's line, the bar and Cancel keep their places while the steps change, so
            // the dialog never jumps: one line of text, a bar that shuttles while a step has no
            // known length, and Cancel dimmed once it can no longer stop the switch.
            div()
                .flex()
                .flex_col()
                .child(title(trf("Switch to {build}", &[("build", &build)])))
                .child(muted(self.switch_line().unwrap_or_default()).truncate())
                .child(crate::widgets::progress_bar(
                    stage_fraction(running.progress)
                        .filter(|_| !stopping)
                        .map(|f| (f, f)),
                    &self.theme,
                ))
                .child(self.switch_log(running))
                .child(div().flex().justify_end().child(if stopping || publishing {
                    crate::widgets::disabled(cancel)
                } else {
                    cancel.on_click(cx.listener(|this, _, _, cx| {
                        this.cancel_switch();
                        cx.notify();
                    }))
                }))
        };
        frame("switch-game", 610., body, cx)
    }

    pub(crate) fn check_build_changes(&mut self, home: &Path, cx: &mut Context<Self>) -> bool {
        self.read_receipt(home, false, cx);
        self.receipt_status(home)
            .is_some_and(|status| matches!(status, Load::Ready(Some(_)) | Load::Failed(_)))
    }

    pub(crate) fn open_build_changes(&mut self, cx: &mut Context<Self>) {
        if let Some(home) = self.selected_instance().map(|profile| profile.home.clone()) {
            self.open_sheet(Sheet::BuildChanges);
            self.read_receipt(&home, true, cx);
        }
    }

    fn receipt_status(&self, home: &Path) -> Option<&Load<Option<(Identity, Active)>>> {
        self.ui
            .sheets
            .switching
            .receipt
            .as_ref()
            .filter(|read| read.home == home)
            .map(|read| &read.status)
    }

    fn read_receipt(&mut self, home: &Path, explicit: bool, cx: &mut Context<Self>) {
        if !explicit && (self.launch_busy() || self.client.is_some()) {
            return;
        }
        let hint = managed_play::build_changes_hint(home);
        let read = &mut self.ui.sheets.switching.receipt;
        if read.as_ref().is_some_and(|read| {
            read.home == home
                && (matches!(read.status, Load::Loading) || !explicit && read.hint == hint)
        }) {
            return;
        }
        if !explicit && hint == Ok(None) {
            *read = None;
            return;
        }
        let home = home.to_owned();
        let ticket = Arc::new(());
        *read = Some(ReceiptRead {
            home: home.clone(),
            hint,
            ticket: ticket.clone(),
            status: Load::Loading,
        });
        let work = cx
            .background_executor()
            .spawn(async move { managed_play::read_build_changes(&home) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(read) = this
                    .ui
                    .sheets
                    .switching
                    .receipt
                    .as_mut()
                    .filter(|read| Arc::ptr_eq(&read.ticket, &ticket))
                {
                    read.status = result.map_or_else(Load::Failed, Load::Ready);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn switch_receipt_sheet(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        if self.ui.sheets.switching.ack {
            // The receipt worker owns the exclusive publication claim until its reply.
            // Don't turn an expected busy snapshot during that window into a false error.
            return Some(frame(
                "switch-receipt",
                600.,
                div()
                    .flex()
                    .flex_col()
                    .child(title(tr("Build changes").into()))
                    .child(muted(tr("Closing this report…"))),
                cx,
            ));
        }
        let instance = self.selected_instance()?;
        let home = instance.home.clone();
        if self.receipt_status(&home).is_none() {
            self.read_receipt(&home, true, cx);
        }
        let (identity, active) = match self.receipt_status(&home)? {
            Load::Ready(Some((identity, active))) => (*identity, active.clone()),
            Load::Loading | Load::Ready(None) => {
                let message = if matches!(self.receipt_status(&home), Some(Load::Loading)) {
                    tr("Reading build changes…")
                } else {
                    tr("There are no pending build changes for this profile.")
                };
                return Some(frame(
                    "switch-receipt",
                    600.,
                    div()
                        .flex()
                        .flex_col()
                        .child(title(tr("Build changes").into()))
                        .child(muted(message)),
                    cx,
                ));
            }
            Load::Failed(error) => {
                let error = error.clone();
                self.open_notice(error, Some(WayOut::ProfileFolder(home)));
                return Some(self.notice_sheet(cx));
            }
        };
        let changes = active.pending_changes()?;
        let id = changes.id;
        let removed = changes.removed;
        let show = self.ui.sheets.details;
        let removed_line = |(name, reason): &(String, String)| {
            trf(
                "{mod} was removed: {reason}",
                &[("mod", name), ("reason", reason)],
            )
        };
        let lines = changes
            .held_lines()
            .chain(removed.iter().map(removed_line))
            .collect::<Vec<_>>();
        let report = std::sync::Arc::new(format!(
            "{}\n{}\n",
            changes.base().map_or_else(
                || tr("Original game build not recorded.").to_owned(),
                |base| trf("Profile game build: {build}", &[("build", base)]),
            ),
            lines.join("\n")
        ));
        let copy = report.clone();
        let list = div()
            .id("switch-receipt-lines")
            .max_h(px(220.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(5.))
            .children(changes.held_lines().map(|line| div().child(line)))
            .children(removed.iter().map(|row| {
                div().child(if show {
                    removed_line(row)
                } else {
                    trf("{mod} was removed.", &[("mod", &row.0)])
                })
            }));
        let body = div()
            .flex()
            .flex_col()
            .child(title(tr("Build changes").into()))
            .child(list)
            .children(self.ui.sheets.error.clone().map(muted))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_end()
                    .gap(px(8.))
                    .children(show.then(|| {
                        crate::frame::copy_button(
                            "receipt-copy",
                            tr("Copy report"),
                            copy.as_ref().clone(),
                        )
                    }))
                    .children(show.then(|| {
                        button("receipt-save", tr("Save report")).on_click(cx.listener(
                            move |this, _, _, cx| {
                                use std::io::Write;
                                let saved = (|| {
                                    let folder = dirs::download_dir().ok_or_else(|| {
                                        tr("The Downloads folder isn't available.").to_owned()
                                    })?;
                                    let key = u128::from_be_bytes(id);
                                    let file = folder
                                        .join(format!("sunrise-build-changes-{key:032x}.txt"));
                                    publish_report(&folder, &file, |output| {
                                        output.write_all(report.as_bytes())
                                    })
                                    .map_err(|e| e.to_string())?;
                                    Ok::<_, String>(file)
                                })();
                                this.ui.sheets.error = Some(match saved {
                                    Ok(file) => trf(
                                        "Saved the report to {file}.",
                                        &[("file", &file.display().to_string())],
                                    ),
                                    Err(error) => trf(
                                        "Couldn't save the build report: {error}",
                                        &[("error", &error)],
                                    ),
                                });
                                cx.notify();
                            },
                        ))
                    }))
                    .child(
                        button(
                            "receipt-why",
                            if show {
                                tr("Hide details")
                            } else {
                                tr("Show why")
                            },
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.ui.sheets.details ^= true;
                            cx.notify();
                        })),
                    )
                    .child(button("receipt-done", tr("Done")).on_click(cx.listener(
                        move |this, _, _, cx| {
                            if this.ui.sheets.switching.ack {
                                return;
                            }
                            this.ui.sheets.switching.ack = true;
                            let (tx, home, active) =
                                (this.tx.clone(), home.clone(), active.clone());
                            std::thread::spawn(move || {
                                let result = Profile::open_for(&home, None, Some(identity))
                                    .map_err(String::from)
                                    .and_then(|mut profile| {
                                        profile.acknowledge_receipt(&active, id)
                                    });
                                let _ = tx.send(Msg::ReceiptAcknowledged(home, result));
                            });
                            cx.notify();
                        },
                    ))),
            );
        Some(frame("switch-receipt", 600., body, cx))
    }

    pub(crate) fn begin_provider_switch(&mut self, switch: PortSwitch) {
        if self.launch_busy() || self.client.is_some() || self.switch_busy() {
            return;
        }
        let Some(instance) = self.selected_instance().cloned() else {
            return;
        };
        let identity = match install_target(&instance) {
            Ok(identity) => identity,
            Err(error) => {
                self.open_notice(error, Some(WayOut::ProfileFolder(instance.home)));
                return;
            }
        };
        let target = self.version_of(&instance).cloned();
        let release = target
            .as_deref()
            .and_then(|tag| self.release_to_install(tag));
        let stop = native_mods::Cancellation::default();
        self.ui.sheets.switching.provider = Some(ProviderRunning {
            switch: switch.clone(),
            profile_name: instance.name.clone(),
            build_label: target.as_deref().map(build_name).unwrap_or_default(),
            progress: InstallProgress::Checking,
            eta: crate::eta::StageEta::default(),
            stop: stop.clone(),
            question: None,
        });
        self.open_sheet(Sheet::SwitchProvider);
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let choose = || {
                if let Some(tag) = target
                    .as_deref()
                    .filter(|tag| backend::installed().iter().any(|local| local == tag))
                {
                    FirstBuild::installed(tag)
                } else if let Some(release) = &release {
                    FirstBuild::download(release)
                } else {
                    Err(backend::no_version_installed())
                }
            };
            // Each question is a fresh round trip to the window; a stopped switch asks nothing.
            let ask = |send: &dyn Fn(Sender<bool>) -> Msg| {
                if stop.is_stopped() {
                    return false;
                }
                let (answer, reply) = std::sync::mpsc::channel();
                tx.send(send(answer)).is_ok() && reply.recv().unwrap_or(false)
            };
            let outcome = switch_provider_v2(
                &instance,
                identity,
                &switch.mod_id,
                &switch.to_project_id,
                V2InstallOptions::default(),
                choose,
                V2ProviderSwitchHooks {
                    install: V2Hooks {
                        install: InstallHooks {
                            progress: &|stage| {
                                let _ = tx.send(Msg::ProviderSwitchProgress(stage));
                            },
                            continue_after_failure: &|_| false,
                            cancellation: None,
                        },
                        choose_provider: Some(&|need| {
                            let (answer, reply) = std::sync::mpsc::channel();
                            tx.send(Msg::ProviderChoice(Box::new(need.clone()), answer))
                                .ok()
                                .and_then(|()| reply.recv().ok().flatten())
                        }),
                        confirm_publisher_change: Some(&|question| {
                            ask(&|answer| {
                                Msg::PublisherChangeQuestion(Box::new(question.clone()), answer)
                            })
                        }),
                    },
                    approve_switch: &|preview| {
                        ask(&|answer| {
                            Msg::ProviderSwitchQuestion(Box::new(preview.clone()), answer)
                        })
                    },
                },
            );
            let outcome = match outcome {
                Ok(_) => Ok(true),
                Err(InstallError::Cancelled) => Ok(false),
                Err(error) => Err(error.to_string()),
            };
            let _ = tx.send(Msg::ProviderSwitched(instance.home, outcome));
        });
    }

    pub(super) fn cancel_provider_switch(&mut self) {
        let Some(running) = &mut self.ui.sheets.switching.provider else {
            return;
        };
        if running.progress != InstallProgress::Publishing {
            running.stop.stop();
            if let Some((_, reply)) = running.question.take() {
                let _ = reply.send(false);
            }
        }
    }

    pub(crate) fn provider_switch_progress(&mut self, stage: InstallProgress) {
        if let Some(running) = &mut self.ui.sheets.switching.provider {
            running.progress = stage;
            running.eta.update(
                crate::eta::install_stage_key(stage),
                crate::mods_page::install::stage_fraction(stage),
                Instant::now(),
            );
        }
    }

    pub(crate) fn provider_switch_question(
        &mut self,
        preview: Box<ProviderSwitchPreview>,
        reply: Sender<bool>,
    ) {
        if let Some(running) =
            self.ui.sheets.switching.provider.as_mut().filter(|run| {
                self.ui.sheet == Some(Sheet::SwitchProvider) && !run.stop.is_stopped()
            })
        {
            running.question = Some((preview, reply));
        } else {
            let _ = reply.send(false);
        }
    }

    pub(crate) fn provider_switch_finished(&mut self, home: &Path, result: Result<bool, String>) {
        let running = self.ui.sheets.switching.provider.take();
        let stopped = running.as_ref().is_some_and(|run| run.stop.is_stopped());
        let result = if result.is_err() && stopped {
            Ok(false)
        } else {
            result
        };
        self.refresh_installed();
        if self.selected_instance().is_none_or(|i| i.home != home) {
            return;
        }
        let visible = self.ui.sheet == Some(Sheet::SwitchProvider);
        match (result, running) {
            (Ok(true), Some(run)) => {
                let notice = trf(
                    "Switched to {repo} ({version}).",
                    &[
                        ("repo", &run.switch.to_repo),
                        ("version", &run.switch.to_version),
                    ],
                );
                self.refresh_mods_after_port_switch(Ok(notice), Some(run.switch.to_project_id));
                if visible {
                    self.close_sheet();
                }
            }
            (Ok(_), _) => {
                if visible {
                    self.close_sheet();
                }
            }
            (Err(error), _) if visible => {
                self.open_notice(error, Some(WayOut::ProfileFolder(home.into())))
            }
            (Err(error), _) => {
                self.refresh_mods_after_port_switch(Err(error), None);
            }
        }
    }

    pub(super) fn provider_switch_sheet(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        use crate::mods_page::install::{stage_fraction, stage_label};
        let running = self.ui.sheets.switching.provider.as_ref()?;
        let switch = &running.switch;
        let body = if let Some((preview, _)) = &running.question {
            let answer = |switch: bool| {
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Some((_, reply)) = this
                        .ui
                        .sheets
                        .switching
                        .provider
                        .as_mut()
                        .and_then(|run| run.question.take())
                    {
                        let _ = reply.send(switch);
                    }
                    cx.notify();
                })
            };
            let profile_line = if running.build_label.is_empty() {
                trf("Profile: {profile}", &[("profile", &running.profile_name)])
            } else {
                trf(
                    "Profile: {profile} ({build})",
                    &[
                        ("profile", &running.profile_name),
                        ("build", &running.build_label),
                    ],
                )
            };
            let summary = trf(
                "This replaces {from_repo} ({from_version}) with {to_repo} ({to_version}). Both provide the {id} in-game ID; only one can be installed at a time.",
                &[
                    ("from_repo", &switch.from_repo),
                    ("from_version", &switch.from_version),
                    ("to_repo", &switch.to_repo),
                    ("to_version", &preview.to_version),
                    ("id", &preview.mod_id),
                ],
            );
            let world_lines: Vec<String> = preview
                .worlds
                .iter()
                .filter(|risk| !risk.mods.is_empty())
                .map(|risk| {
                    trf(
                        "{world}: content from {mod} may be affected.",
                        &[("world", &risk.name), ("mod", &switch.mod_name)],
                    )
                })
                .collect();
            let world_box = (!world_lines.is_empty()).then(|| {
                card()
                    .p(px(14.))
                    .gap(px(6.))
                    .children(world_lines.into_iter().map(muted))
            });
            div()
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(title(trf(
                            "Switch {mod} to {repo}?",
                            &[("mod", &switch.mod_name), ("repo", &switch.to_repo)],
                        )))
                        .child(muted(profile_line)),
                )
                .child(
                    div()
                        .text_size(px(tokens::BODY))
                        .text_color(rgb(palette().text))
                        .child(crate::selectable::selectable(summary)),
                )
                .children(world_box)
                .child(muted(trf(
                    "Your current port stays intact if you cancel. Future updates on this profile will follow {repo}.",
                    &[("repo", &switch.to_repo)],
                )))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            button("provider-switch-cancel", tr("Cancel"))
                                .on_click(answer(false)),
                        )
                        .child(
                            crate::frame::primary("provider-switch-confirm", tr("Switch"))
                                .on_click(answer(true)),
                        ),
                )
        } else {
            let publishing = running.progress == InstallProgress::Publishing;
            let cancel = button("provider-switch-cancel", tr("Cancel"));
            let cancel = if publishing {
                cancel.opacity(0.45).cursor_default().tab_stop(false)
            } else {
                cancel.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.cancel_provider_switch();
                    cx.notify();
                }))
            };
            div()
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(title(trf(
                    "Switching {mod} to {repo}",
                    &[("mod", &switch.mod_name), ("repo", &switch.to_repo)],
                )))
                .child(muted(if publishing {
                    stage_label(running.progress)
                } else {
                    crate::eta::with_eta(
                        stage_label(running.progress),
                        running.eta.remaining(Instant::now()),
                    )
                }))
                .children(
                    stage_fraction(running.progress)
                        .map(|f| crate::widgets::progress_bar(Some((f, f)), &self.theme)),
                )
                .child(muted(tr("Your current port stays intact if you cancel.")))
                .child(div().flex().justify_end().child(cancel))
        };
        Some(frame("provider-switch-sheet", 560., body, cx))
    }

    pub(crate) fn receipt_acknowledged(
        &mut self,
        home: &std::path::Path,
        result: Result<bool, String>,
    ) {
        self.ui.sheets.switching.ack = false;
        if self.ui.sheet != Some(Sheet::BuildChanges)
            || self.selected_instance().is_none_or(|i| i.home != home)
        {
            return;
        }
        match result {
            Ok(true) => {
                self.ui.sheets.switching.receipt = None;
                self.close_sheet();
            }
            Ok(false) => self.ui.sheets.error = Some(tr("These build changes are no longer current. Close this report and open Build changes again.").into()),
            Err(error) => self.ui.sheets.error = Some(error),
        }
    }
}

/// How a switch begins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SwitchStart {
    /// Home's switch, for a profile already on its target: nothing is saved.
    Refuse,
    /// The picker's choice for a profile already on its target: the choice is saved alone.
    SaveChoiceOnly,
    /// A real switch, whose worker checks the profile as it is admitted.
    Switch,
}

/// Decides once how a switch begins. `fast_save` reads the profile's live state, so it is asked
/// exactly once: a second read could see another switch land on the target in between. Only the
/// picker's policy may save its choice alone; Home's switch either refuses or runs the worker,
/// whose admission checks every wanted mod (review of #616).
fn switch_start(
    policy: SwitchPolicy,
    already_there: bool,
    mut fast_save: impl FnMut() -> bool,
) -> SwitchStart {
    let fast = fast_save();
    match policy {
        SwitchPolicy::KeepEveryMod if already_there || fast => SwitchStart::Refuse,
        SwitchPolicy::Ask if fast => SwitchStart::SaveChoiceOnly,
        SwitchPolicy::Ask | SwitchPolicy::KeepEveryMod => SwitchStart::Switch,
    }
}

/// Whether a picker selection can be saved directly to `instances.json` without running a build
/// switch: the profile must be ordinary, have no pending transaction recovery or first-Play
/// bootstrap, and already be on `tag`.
fn can_fast_save_choice(
    instance: &instances::Instance,
    tag: &str,
    unplayed_target: Option<&str>,
    installed: bool,
) -> bool {
    if instance.kind == instances::Kind::Pack {
        return false;
    }
    if !matches!(
        sunrise_launcher::profile_txn::recovery_pending(&instance.home),
        Ok(false)
    ) {
        return false;
    }
    match managed_play::display_active(&instance.home) {
        Ok(Some(active)) => active.base() == tag,
        Ok(None) => unplayed_target == Some(tag) && installed,
        Err(_) => false,
    }
}

/// A partial write or an interrupted writer must not reserve the final report name.
fn publish_report(
    folder: &std::path::Path,
    file: &std::path::Path,
    write: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut temporary = tempfile::NamedTempFile::new_in(folder)?;
    write(temporary.as_file_mut())?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(file)
        .map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        can_fast_save_choice, publish_report, shown_outcome, switch_ending, switch_start,
        SwitchStart,
    };
    use crate::sheets::Sheet;
    use crate::Load;

    /// Runs `test` again in a child process with scratch homes, as the other window tests do:
    /// false here, true in the child.
    fn in_scratch_child(test: &str) -> bool {
        const CHILD: &str = "SUNRISE_SWITCH_WINDOW_TEST";
        if std::env::var_os(CHILD).is_some() {
            return true;
        }
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        let nowhere = "http://127.0.0.1:9";
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &format!("sheets::switching::tests::{test}")])
            .env(CHILD, root)
            // The window runs on a 16 MiB stack (`startup.rs`); a test thread has 2 MiB, too
            // little for the switch dialog's frame in a debug build.
            .env("RUST_MIN_STACK", (16 * 1024 * 1024).to_string())
            .env("SUNRISE_HOME", root.join("game"))
            .env("SUNRISE_LAUNCHER_HOME", root.join("launcher"))
            .env("SUNRISE_LAUNCHER_LOCATION_FILE", root.join("location.json"))
            .env("SUNRISE_MOD_INDEX_V2", root.join("missing-index.signed"))
            .env("SUNRISE_NO_KEYCHAIN", "1")
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("SUNRISE_TELEMETRY_URL", "off")
            .env("XDG_DATA_HOME", root.join("data"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("HTTP_PROXY", nowhere)
            .env("HTTPS_PROXY", nowhere)
            .env("ALL_PROXY", nowhere)
            .env_remove("NO_PROXY")
            .env_remove("SUNRISE_GITHUB_MIRROR")
            .env_remove("SUNRISE_LAUNCHER_BENCH")
            .status()
            .unwrap();
        assert!(status.success(), "{test} failed in its child process");
        false
    }

    /// The selected profile, as `instances.json` has it now.
    fn saved_profile(id: &str) -> sunrise_launcher::instances::Instance {
        sunrise_launcher::instances::list()
            .into_iter()
            .find(|instance| instance.id == id)
            .unwrap()
    }

    /// A pick of a build that isn't downloaded is kept only once its download and preparation
    /// have both succeeded: one whose download fails leaves the profile following its channel,
    /// with nothing saved (review of #616). A download that succeeds can't be faked here: the
    /// game's releases are checked against the built-in release keys only.
    #[test]
    fn a_pick_whose_download_fails_is_not_kept() {
        if !in_scratch_child("a_pick_whose_download_fails_is_not_kept") {
            return;
        }
        // Its download thread's messages reach the window: let the pump's tick read them.
        crate::wake::UNDER_TEST_SCHEDULER.store(true, std::sync::atomic::Ordering::Relaxed);
        const TAG: &str = "26.3-0.1.0";
        let profile = sunrise_launcher::instances::create("Cabin", None).unwrap();
        let mut settings = sunrise_launcher::backend::Settings::load();
        settings.instance = Some(profile.id.clone());
        settings.save().unwrap();
        let mut app = gpui::TestAppContext::single();
        let (shell, cx) = app.add_window_view(crate::window_root::Shell::new_for_test);
        let launcher = shell.read_with(cx, |shell, _| shell.launcher());
        cx.run_until_parked();
        launcher.update(cx, |l, _| {
            l.releases = Load::Ready(vec![sunrise_launcher::backend::Release {
                tag: TAG.into(),
                name: TAG.into(),
                body: String::new(),
                date: "2026-11-20".into(),
                time: String::new(),
                published_at: String::new(),
                asset: Some((
                    format!("https://github.com/sunrise-org/sunrise/releases/download/{TAG}/sunrise-linux-x86_64.tar.gz"),
                    1000,
                )),
                prerelease: false,
                nightly: None,
            }]);
            // As the picker's Switch does.
            l.begin_switch(TAG.into(), Some(sunrise_launcher::browser::sunrise::BuildChoice::Keep));
            assert!(l.ui.sheets.switching.running.is_some(), "the switch began");
        });
        assert_eq!(
            saved_profile(&profile.id).version,
            None,
            "nothing is kept while it downloads"
        );
        let mut ended = false;
        for _ in 0..300 {
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(200));
            cx.run_until_parked();
            if launcher.read_with(cx, |l, _| l.ui.sheets.switching.running.is_none()) {
                ended = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        assert!(ended, "the failed download ended the switch");
        assert_eq!(
            saved_profile(&profile.id).version,
            None,
            "a pick whose download failed is not kept: the profile still follows its channel"
        );
        assert!(!sunrise_launcher::backend::version_dir(TAG)
            .join(".installed")
            .exists());
    }

    /// A picker switch stays unsaved while an unprepared build prepares (and when preparation
    /// fails), and a completed download advances through `finish_switch_download` into the switch
    /// worker carrying `SwitchPolicy::Ask` and `BuildChoice::Keep` without saving early before
    /// publication (review of #616).
    #[test]
    fn an_undownloaded_pick_waits_for_download_and_preparation_before_starting_the_worker() {
        if !in_scratch_child(
            "an_undownloaded_pick_waits_for_download_and_preparation_before_starting_the_worker",
        ) {
            return;
        }
        crate::wake::UNDER_TEST_SCHEDULER.store(true, std::sync::atomic::Ordering::Relaxed);
        let profile = sunrise_launcher::instances::create("Cabin", None).unwrap();
        let mut settings = sunrise_launcher::backend::Settings::load();
        settings.instance = Some(profile.id.clone());
        settings.save().unwrap();
        let mut app = gpui::TestAppContext::single();
        let (shell, cx) = app.add_window_view(crate::window_root::Shell::new_for_test);
        let launcher = shell.read_with(cx, |shell, _| shell.launcher());
        cx.run_until_parked();

        let wait_until_idle = |cx: &mut gpui::VisualTestContext| {
            let mut ended = false;
            for _ in 0..200 {
                cx.executor()
                    .advance_clock(std::time::Duration::from_millis(50));
                cx.run_until_parked();
                if launcher.read_with(cx, |l, _| {
                    l.ui.sheets.switching.running.is_none() && !l.preparing
                }) {
                    ended = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            assert!(ended, "background switch job finished cleanly");
        };

        // Install a newer build so the following profile targets OTHER (not TAG_PREP_FAIL),
        // keeping its saved pin None while ensuring a pick of TAG_PREP_FAIL runs a real switch.
        const OTHER: &str = "26.3-0.1.9";
        let other_build = sunrise_launcher::backend::version_dir(OTHER);
        std::fs::create_dir_all(&other_build).unwrap();
        std::fs::write(other_build.join(".installed"), []).unwrap();

        // 1. An installed but unprepared build starts real preparation (no download thread) and
        //    leaves instances.json untouched both during preparation and after preparation fails.
        const TAG_PREP_FAIL: &str = "26.3-0.1.0";
        let prep_build = sunrise_launcher::backend::version_dir(TAG_PREP_FAIL);
        std::fs::create_dir_all(&prep_build).unwrap();
        std::fs::write(prep_build.join(".installed"), []).unwrap();
        launcher.update(cx, |l, _| {
            l.refresh_installed();
            l.begin_switch(
                TAG_PREP_FAIL.into(),
                Some(sunrise_launcher::browser::sunrise::BuildChoice::Keep),
            );
            let running =
                l.ui.sheets
                    .switching
                    .running
                    .as_ref()
                    .expect("preparation switch began");
            assert!(
                matches!(
                    running.progress,
                    InstallProgress::Minecraft(sunrise_launcher::backend::PrepareStep::Files, None)
                ),
                "unprepared build enters preparation: {:?}",
                running.progress
            );
            assert!(matches!(
                running.pending.as_ref().and_then(|p| p.choice),
                Some(sunrise_launcher::browser::sunrise::BuildChoice::Keep)
            ));
        });
        assert_eq!(
            saved_profile(&profile.id).version,
            None,
            "nothing saved while preparing"
        );
        wait_until_idle(cx);
        assert_eq!(
            launcher.read_with(cx, |l, _| l.ui.sheet),
            Some(Sheet::Notice),
            "failed preparation reports a notice"
        );
        assert_eq!(
            saved_profile(&profile.id).version,
            None,
            "failed preparation leaves no saved build"
        );

        // 2. A distinct tag staged in the download phase (no competing installer thread) advances
        //    through finish_switch_download into start_switch_worker with Ask + Keep once its
        //    downloaded and prepared files are in place, and still saves nothing before worker
        //    publication.
        const TAG_DOWNLOAD_OK: &str = "26.3-0.1.1";
        let dl_build = sunrise_launcher::backend::version_dir(TAG_DOWNLOAD_OK);
        std::fs::create_dir_all(&dl_build).unwrap();
        std::fs::write(dl_build.join(".installed"), []).unwrap();
        let dl_resources = sunrise_launcher::backend::resources(TAG_DOWNLOAD_OK);
        std::fs::create_dir_all(&dl_resources).unwrap();
        std::fs::write(dl_resources.join("sunrise-resources.json"), "{}").unwrap();
        assert!(sunrise_launcher::backend::check_prepared(TAG_DOWNLOAD_OK));
        launcher.update(cx, |l, _| {
            l.close_sheet();
            let instance = l.selected_instance().cloned().unwrap();
            let identity = sunrise_launcher::browser::sunrise::install_target(&instance).unwrap();
            l.stage_switch(
                TAG_DOWNLOAD_OK.into(),
                instance,
                identity,
                Some(sunrise_launcher::browser::sunrise::BuildChoice::Keep),
                sunrise_launcher::browser::sunrise::SwitchPolicy::Ask,
            );
            l.ui.sheets.switching.cache_download = Some(TAG_DOWNLOAD_OK.into());
            use sunrise_launcher::backend::InstallPhase;
            l.switch_download_progress(TAG_DOWNLOAD_OK, InstallPhase::Checking);
            assert_eq!(l.switch_line().as_deref(), Some("Checking the build…"));
            l.switch_download_progress("other", InstallPhase::Finishing);
            assert_eq!(l.switch_line().as_deref(), Some("Checking the build…"));
            l.switch_download_progress(TAG_DOWNLOAD_OK, InstallPhase::Downloading(0.25));
            l.switch_detail("Transfer detail".into());
            l.switch_download_progress(TAG_DOWNLOAD_OK, InstallPhase::Downloading(0.50));
            assert_eq!(l.switch_line().as_deref(), Some("Transfer detail"));
            l.switch_download_progress(TAG_DOWNLOAD_OK, InstallPhase::Unpacking);
            assert_eq!(l.switch_line().as_deref(), Some("Unpacking the build"));
            l.switch_download_progress(TAG_DOWNLOAD_OK, InstallPhase::Finishing);
            assert_eq!(
                l.switch_line().as_deref(),
                Some("Putting the build in place")
            );
            l.switch_download_progress(TAG_DOWNLOAD_OK, InstallPhase::Preparing);
            assert_eq!(l.switch_line().as_deref(), Some("Getting the build ready…"));
            l.switch_detail("Copying the prepared game files: 2 of 8".into());
            assert_eq!(
                l.switch_line().as_deref(),
                Some("Copying the prepared game files: 2 of 8")
            );
            assert_eq!(
                saved_profile(&profile.id).version,
                None,
                "nothing saved while download is pending"
            );
            assert!(l.finish_switch_download(
                TAG_DOWNLOAD_OK,
                &Ok(sunrise_launcher::backend::InstallOutcome::Installed(None)),
            ));
            let running =
                l.ui.sheets
                    .switching
                    .running
                    .as_ref()
                    .expect("switch worker started after download");
            assert!(running.pending.is_none(), "pending handoff consumed");
            assert_eq!(
                running.started_with,
                Some(sunrise_launcher::browser::sunrise::SwitchPolicy::Ask),
                "worker started with picker Ask policy"
            );
            assert!(
                matches!(
                    running.started_choice,
                    Some(sunrise_launcher::browser::sunrise::BuildChoice::Keep)
                ),
                "worker received BuildChoice::Keep"
            );
            assert_eq!(running.progress, InstallProgress::Checking);
        });
        wait_until_idle(cx);
        assert_eq!(
            saved_profile(&profile.id).version,
            None,
            "an unpublished switch leaves no saved build"
        );
    }

    #[test]
    fn switch_can_cancel_install_finishing_but_not_profile_publication() {
        if !in_scratch_child("switch_can_cancel_install_finishing_but_not_profile_publication") {
            return;
        }
        let profile = sunrise_launcher::instances::create("Cabin", None).unwrap();
        let mut settings = sunrise_launcher::backend::Settings::load();
        settings.instance = Some(profile.id.clone());
        settings.save().unwrap();
        let mut app = gpui::TestAppContext::single();
        let (shell, cx) = app.add_window_view(crate::window_root::Shell::new_for_test);
        let launcher = shell.read_with(cx, |shell, _| shell.launcher());
        launcher.update(cx, |l, _| {
            let instance = l.selected_instance().cloned().unwrap();
            let identity = sunrise_launcher::browser::sunrise::install_target(&instance).unwrap();
            let tag = "26.3-0.1.0";
            let stop = l.stage_switch(
                tag.into(),
                instance,
                identity,
                None,
                sunrise_launcher::browser::sunrise::SwitchPolicy::Ask,
            );
            l.switch_progress(InstallProgress::Publishing);
            assert_eq!(l.switch_line().as_deref(), Some("Applying the switch."));
            l.cancel_switch();
            assert!(!stop.is_stopped());

            l.ui.sheets.switching.cache_download = Some(tag.into());
            l.switch_download_progress(tag, sunrise_launcher::backend::InstallPhase::Finishing);
            l.cancel_switch();
            assert!(stop.is_stopped());
            assert_eq!(l.switch_line().as_deref(), Some("Cancelling…"));
            assert!(l.finish_switch_download(
                tag,
                &Err(sunrise_launcher::native_mods::CANCELLED.into()),
            ));
            assert!(l.ui.sheets.switching.running.is_none());
        });
        assert!(saved_profile(&profile.id).version.is_none());
    }

    /// Home's switch to a build that is already prepared starts its worker inside
    /// `begin_switch`: the worker must start with Home's policy, not the default one it had
    /// before the policy was set (review of #616).
    #[test]
    fn homes_switch_to_a_prepared_build_starts_its_worker_with_homes_policy() {
        if !in_scratch_child("homes_switch_to_a_prepared_build_starts_its_worker_with_homes_policy")
        {
            return;
        }
        crate::wake::UNDER_TEST_SCHEDULER.store(true, std::sync::atomic::Ordering::Relaxed);
        const TAG: &str = "26.3-0.1.0";
        let build = sunrise_launcher::backend::version_dir(TAG);
        std::fs::create_dir_all(&build).unwrap();
        std::fs::write(build.join(".installed"), []).unwrap();
        let resources = sunrise_launcher::backend::resources(TAG);
        std::fs::create_dir_all(&resources).unwrap();
        std::fs::write(resources.join("sunrise-resources.json"), "{}").unwrap();
        assert!(sunrise_launcher::backend::check_prepared(TAG));
        // The profile is kept on another installed build: a switch to `TAG` is a real one, not
        // the saved choice alone that a profile already on its target gets.
        const OTHER: &str = "26.3-0.0.9";
        let other = sunrise_launcher::backend::version_dir(OTHER);
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join(".installed"), []).unwrap();
        let profile = sunrise_launcher::instances::create("Cabin", None).unwrap();
        sunrise_launcher::browser::sunrise::save_build_choice(
            &profile.id,
            OTHER,
            sunrise_launcher::browser::sunrise::BuildChoice::Keep,
        )
        .unwrap();
        let mut settings = sunrise_launcher::backend::Settings::load();
        settings.instance = Some(profile.id.clone());
        settings.save().unwrap();
        let mut app = gpui::TestAppContext::single();
        let (shell, cx) = app.add_window_view(crate::window_root::Shell::new_for_test);
        let launcher = shell.read_with(cx, |shell, _| shell.launcher());
        cx.run_until_parked();
        let started = launcher.update(cx, |l, _| {
            l.begin_mods_switch(TAG.into());
            l.ui.sheets
                .switching
                .running
                .as_ref()
                .map(|running| (running.policy, running.started_with))
        });
        assert_eq!(
            started,
            Some((
                sunrise_launcher::browser::sunrise::SwitchPolicy::KeepEveryMod,
                Some(sunrise_launcher::browser::sunrise::SwitchPolicy::KeepEveryMod)
            )),
            "the worker started, and with Home's policy"
        );
    }
    /// Home's switch to the build a following profile already plays saves no pin: Home worked it
    /// out for the profile on another build, and the profile may have reached this one with a
    /// wanted mod held off (review of #616). Its fast path is the picker's alone.
    #[test]
    fn homes_switch_to_the_build_a_profile_already_plays_saves_nothing() {
        if !in_scratch_child("homes_switch_to_the_build_a_profile_already_plays_saves_nothing") {
            return;
        }
        crate::wake::UNDER_TEST_SCHEDULER.store(true, std::sync::atomic::Ordering::Relaxed);
        const TAG: &str = "26.3-0.1.0";
        let build = sunrise_launcher::backend::version_dir(TAG);
        std::fs::create_dir_all(&build).unwrap();
        std::fs::write(build.join(".installed"), []).unwrap();
        let profile = sunrise_launcher::instances::create("Cabin", None).unwrap();
        let mut settings = sunrise_launcher::backend::Settings::load();
        settings.instance = Some(profile.id.clone());
        settings.save().unwrap();
        let mut app = gpui::TestAppContext::single();
        let (shell, cx) = app.add_window_view(crate::window_root::Shell::new_for_test);
        let launcher = shell.read_with(cx, |shell, _| shell.launcher());
        cx.run_until_parked();
        launcher.update(cx, |l, _| {
            l.refresh_installed();
            let instance = l.selected_instance().cloned().unwrap();
            let tag = l
                .profile_target(&instance)
                .expect("the following profile goes to the installed build");
            let unplayed = l.target_version(None, instance.channel).cloned();
            assert!(
                can_fast_save_choice(&instance, &tag, unplayed.as_deref(), true),
                "the picker would only save its choice here"
            );
            l.begin_switch_as(
                tag,
                Some(sunrise_launcher::browser::sunrise::BuildChoice::Keep),
                sunrise_launcher::browser::sunrise::SwitchPolicy::KeepEveryMod,
            );
            assert!(l.ui.sheets.switching.running.is_none(), "no switch began");
            assert_eq!(l.ui.sheet, Some(Sheet::Notice), "Home's switch says why");
        });
        assert_eq!(
            saved_profile(&profile.id).version,
            None,
            "no pin was saved: the profile still follows its channel"
        );
    }

    use sunrise_launcher::browser::sunrise::{InstallProgress, Switched};

    #[test]
    fn a_switch_ends_in_its_log_as_it_did_not_as_last_pressed() {
        let failed: Result<Switched, String> = Err("No space left on device (os error 28)".into());
        // A cancel was taken, but the switch failed for another reason first: it says why.
        assert_eq!(
            switch_ending(&failed, true, false),
            format!(
                "failed: {} (os error 28)",
                std::io::Error::from_raw_os_error(28).kind()
            )
        );
        // The player is shown that failure too, and only a cancel's own error ends quietly.
        let space = "No space left on device (os error 28)";
        assert!(matches!(shown_outcome(Err(space.into()), true), Err(e) if e == space));
        assert!(matches!(
            shown_outcome(Err(sunrise_launcher::downloads::CANCELLED.into()), true),
            Ok(Switched::Stayed)
        ));
        let cancelled: Result<Switched, String> =
            Err(sunrise_launcher::native_mods::CANCELLED.into());
        assert_eq!(
            switch_ending(&cancelled, true, false),
            "cancelled, nothing changed"
        );
        assert_eq!(
            switch_ending(&Ok(Switched::Stayed), false, true),
            "the player declined the changes it asked about, nothing changed"
        );
        assert_eq!(
            switch_ending(&Ok(Switched::Stayed), false, false),
            "nothing changed"
        );
    }
    use std::io::{self, Write};
    use sunrise_launcher::instances::{Channel, Instance, Kind as InstanceKind};

    /// The profile's state flips between reads, as when an ordinary switch lands on Home's
    /// target while Home's switch begins: it is read once, and Home's switch never saves its
    /// choice alone (review of #616).
    #[test]
    fn homes_switch_reads_the_profile_once_and_never_saves_its_choice_alone() {
        use sunrise_launcher::browser::sunrise::SwitchPolicy::{Ask, KeepEveryMod};
        let reads = std::cell::Cell::new(0);
        let flipping = || {
            reads.set(reads.get() + 1);
            reads.get() > 1
        };
        assert_eq!(
            switch_start(KeepEveryMod, false, flipping),
            SwitchStart::Switch
        );
        assert_eq!(reads.get(), 1, "the profile's state is read once");
        for fast in [false, true] {
            assert_ne!(
                switch_start(KeepEveryMod, false, || fast),
                SwitchStart::SaveChoiceOnly
            );
        }
        assert_eq!(
            switch_start(KeepEveryMod, true, || false),
            SwitchStart::Refuse
        );
        assert_eq!(
            switch_start(KeepEveryMod, false, || true),
            SwitchStart::Refuse
        );
        assert_eq!(
            switch_start(Ask, false, || true),
            SwitchStart::SaveChoiceOnly
        );
        assert_eq!(switch_start(Ask, true, || false), SwitchStart::Switch);
    }

    #[test]
    fn can_fast_save_choice_checks_kind_recovery_and_target() {
        let dir = tempfile::tempdir().unwrap();
        let mut instance = Instance {
            id: "own-1".into(),
            name: "Own".into(),
            home: dir.path().to_path_buf(),
            work: dir.path().to_path_buf(),
            version: Some("dev-2026-11-24-abc".into()),
            channel: Channel::Dev,
            kind: InstanceKind::Own,
            group: None,
            icon: None,
            native_profile: Default::default(),
            last_played: None,
            played: Default::default(),
            launch: Default::default(),
            incarnation: None,
        };
        let tag = "dev-2026-11-24-abc";
        // Ordinary unplayed profile already targeting `tag` with `tag` installed -> fast path.
        assert!(can_fast_save_choice(&instance, tag, Some(tag), true));
        // Not installed yet or different target -> full switch.
        assert!(!can_fast_save_choice(&instance, tag, Some(tag), false));
        assert!(!can_fast_save_choice(
            &instance,
            tag,
            Some("dev-2026-11-20-old"),
            true
        ));
        // Pack profiles never take the fast path.
        instance.kind = InstanceKind::Pack;
        assert!(!can_fast_save_choice(&instance, tag, Some(tag), true));
        instance.kind = InstanceKind::Own;
        // Pending transaction/bootstrap record -> never take the fast path.
        let txn = dir.path().join("mod-browser/install-txn.json");
        std::fs::create_dir_all(txn.parent().unwrap()).unwrap();
        std::fs::write(&txn, "{}").unwrap();
        assert!(!can_fast_save_choice(&instance, tag, Some(tag), true));
    }

    #[test]
    fn report_save_can_retry_after_a_partial_write_and_never_clobbers() {
        let folder = tempfile::tempdir().unwrap();
        let report = folder.path().join("report.txt");
        let save =
            |text: &[u8]| publish_report(folder.path(), &report, |file| file.write_all(text));
        let error = publish_report(folder.path(), &report, |file| {
            file.write_all(b"partial")?;
            Err(io::Error::new(io::ErrorKind::StorageFull, "fixture"))
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::StorageFull);
        assert!(!report.exists());
        save(b"complete report\n").unwrap();
        assert_eq!(
            save(b"other").unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read(&report).unwrap(), b"complete report\n");
    }
}
