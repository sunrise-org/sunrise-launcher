//! Switching a profile's build: download, prepare, the worker, its questions and its sheet.
use super::*;

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
            let home = pending.target.home().to_owned();
            if let Err(error) = result {
                self.switch_finished(&home, Err(error.clone()));
            } else if running.stop.is_stopped() {
                self.switch_finished(&home, Ok(Switched::Stayed));
            } else {
                self.prepare_switch();
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
        let target = match Target::of(&instance) {
            Ok(target) => target,
            Err(error) => {
                self.open_notice(error.into(), Some(WayOut::ProfileFolder(instance.home)));
                return;
            }
        };
        let identity = target.identity();
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
                    if let Err(error) = target.save_build_choice(&tag, choice) {
                        self.open_notice(error.into(), Some(WayOut::ProfileFolder(instance.home)));
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
        let from = (instance.home.clone(), self.profile_target(&instance));
        let stop = self.stage_switch(tag.clone(), from, target, identity, choice, policy);
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
        from: (PathBuf, Option<String>),
        target: Target,
        identity: Identity,
        choice: Option<BuildChoice>,
        policy: SwitchPolicy,
    ) -> native_mods::Cancellation {
        let stop = native_mods::Cancellation::default();
        self.ui.sheets.switching.running = Some(Running {
            tag: tag.clone(),
            from,
            back: self.ui.sheet,
            pending: Some(Pending {
                target,
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
            refused: Arc::default(),
        });
        self.switch_log_line(format!("Switching to {tag}: started"));
        self.open_sheet(Sheet::Switching);
        stop
    }

    pub(super) fn prepare_switch(&mut self) {
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
            self.prepare_version_cancellable(tag, Some(stop));
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
        let home = pending.target.home().to_owned();
        if let Some(error) = error {
            self.switch_finished(&home, Err(error));
        } else if running.stop.is_stopped() {
            self.switch_finished(&home, Ok(Switched::Stayed));
        } else {
            self.start_switch_worker();
        }
    }

    pub(super) fn start_switch_worker(&mut self) {
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
            target,
            identity,
            choice,
        } = running.pending.take().expect("switch queued");
        let (tag, stop, op) = (
            running.tag.clone(),
            running.stop.clone(),
            running.op.clone(),
        );
        let (policy, refused) = (running.policy, running.refused.clone());
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
                &target,
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
            let _ = tx.send(Msg::Switched(target.home().to_owned(), Box::new(outcome)));
        });
    }

    pub(crate) fn cancel_switch(&mut self) {
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

    /// Build-switch messages routed from `launcher_messages::drain`.
    pub(crate) fn on_switch_message(&mut self, msg: Msg) {
        match msg {
            Msg::SwitchProgress(stage) => self.switch_progress(stage),
            Msg::SwitchDetail(text) => self.switch_detail(text),
            Msg::SwitchQuestion(question, reply) => self.switch_question(question, reply),
            Msg::Switched(home, result) => self.switch_finished(&home, *result),
            _ => unreachable!("launcher_messages::drain only routes build-switch messages here"),
        }
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
                    crate::eta::stage_label(stage)
                ));
            }
            running.progress = stage;
            running.eta.update(
                crate::eta::install_stage_key(stage),
                crate::eta::stage_fraction(stage),
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
            .filter(|run| !run.stop.is_stopped())
        {
            running.question = Some((question, reply));
            self.ui.sheets.details = false;
            // A hidden switch comes back for its answer, unless another sheet is open: then the
            // status strip says it waits, and Show opens it.
            if self.ui.sheet.is_none() {
                self.open_sheet(Sheet::Switching);
            }
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
        let tag = running.as_ref().map(|run| run.tag.clone());
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
                } else if let Some(tag) = tag {
                    // Its sheet was hidden: the status strip's row is gone, so say it ended.
                    toast::say(trf("Switched to {build}.", &[("build", &build_name(&tag))]));
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
            // A hidden switch that failed says so too, over the page when nothing else is open.
            Err(error) if visible || self.ui.sheet.is_none() => {
                self.open_notice(error, Some(WayOut::ProfileFolder(home.into())))
            }
            Err(error) => toast::say(error),
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
        use crate::eta::stage_label;
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

    pub(in crate::sheets) fn switch_sheet(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        use crate::eta::stage_fraction;
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
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(hide_button("hide-switch", cx))
                        .child(if stopping || publishing {
                            crate::widgets::disabled(cancel)
                        } else {
                            cancel.on_click(cx.listener(|this, _, _, cx| {
                                this.cancel_switch();
                                cx.notify();
                            }))
                        }),
                )
        };
        frame("switch-game", 610., body, cx)
    }
}

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
pub(super) fn can_fast_save_choice(
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
