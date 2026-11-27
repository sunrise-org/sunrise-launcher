//! Switching a profile's build: download, prepare, the worker, its questions and its sheet.
use super::*;
use crate::widgets::button::{primary, secondary};

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
            if running.stop.is_stopped() {
                let home = pending.target.home().to_owned();
                self.switch_finished(&home, Ok(Switched::Stayed));
            } else if let Err(error) = result {
                let home = pending.target.home().to_owned();
                self.switch_finished(&home, Err(error.clone()));
            } else {
                self.prepare_switch();
            }
        } else {
            self.ui.sheets.error = result.as_ref().err().cloned();
        }
        true
    }

    pub(crate) fn switch_download_progress(&mut self, tag: &str, fraction: f32) {
        if self.ui.sheets.switching.cache_download.as_deref() == Some(tag) {
            self.switch_progress(InstallProgress::DownloadingBuild(fraction));
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

    pub(in crate::sheets) fn begin_switch(&mut self, tag: String, choice: Option<BuildChoice>) {
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
        // Changing only the channel/pin preference (the profile is already on `tag`, or has never
        // been played) needs no switch modal, download, registry fetch, or mod rebuild: Install
        // or Play installs what it then names. Keep artifact hashing off the UI thread.
        if can_fast_save_choice(&instance, &tag) {
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
        let release = self.release_to_install(&tag);
        let stop = native_mods::Cancellation::default();
        let from = (instance.home.clone(), self.profile_target(&instance));
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
        });
        self.open_sheet(Sheet::Switching);
        if let Some(release) = release {
            self.ui.sheets.switching.cache_download = Some(tag);
            self.switch_progress(InstallProgress::DownloadingBuild(0.));
            self.install_cancellable(&release, Some(stop));
        } else {
            self.prepare_switch();
        }
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
            self.prepare_version_cancellable(tag, backend::MinecraftFrom::Install, Some(stop));
        }
    }

    /// A line from the preparation a switch waits on: the dialog follows its step, and the
    /// files done while Mojang's download runs.
    pub(crate) fn follow_switch_preparation(&mut self) {
        if let Some(running) = self
            .ui
            .sheets
            .switching
            .running
            .as_mut()
            .filter(|run| matches!(run.progress, InstallProgress::Minecraft(..)))
        {
            let stage = InstallProgress::Minecraft(
                backend::prepare_step(),
                sunrise_launcher::mojang::progress(),
            );
            running.progress = stage;
            running.eta.update(
                crate::eta::install_stage_key(stage),
                crate::eta::stage_fraction(stage),
                Instant::now(),
            );
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
        if running.stop.is_stopped() {
            let home = pending.target.home().to_owned();
            self.switch_finished(&home, Ok(Switched::Stayed));
        } else if let Some(error) = error {
            let home = pending.target.home().to_owned();
            self.switch_finished(&home, Err(error));
        } else {
            self.start_switch_worker();
        }
    }

    pub(super) fn start_switch_worker(&mut self) {
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
        // Both switch routes start by checking the profile's mods; a v2 profile first
        // refreshes its index, so don't leave the last download or preparation step showing.
        running.progress = InstallProgress::PreparingBuild;
        running.eta.update(
            crate::eta::install_stage_key(InstallProgress::PreparingBuild),
            None,
            Instant::now(),
        );
        let (tag, stop) = (running.tag.clone(), running.stop.clone());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
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
                },
            );
            let _ = tx.send(Msg::Switched(target.home().to_owned(), Box::new(outcome)));
        });
    }

    pub(crate) fn cancel_switch(&mut self) {
        let Some(running) = &mut self.ui.sheets.switching.running else {
            return;
        };
        if running.progress != InstallProgress::Publishing {
            running.stop.stop();
            if let Some((_, reply)) = running.question.take() {
                let _ = reply.send(SwitchDecision::Cancel);
            }
        }
    }

    /// Build-switch messages routed from `launcher_messages::drain`.
    pub(crate) fn on_switch_message(&mut self, msg: Msg) {
        match msg {
            Msg::SwitchProgress(stage) => self.switch_progress(stage),
            Msg::SwitchQuestion(question, reply) => self.switch_question(question, reply),
            Msg::Switched(home, result) => self.switch_finished(&home, *result),
            _ => unreachable!("launcher_messages::drain only routes build-switch messages here"),
        }
    }

    pub(crate) fn switch_progress(&mut self, stage: InstallProgress) {
        if let Some(running) = &mut self.ui.sheets.switching.running {
            running.progress = stage;
            running.eta.update(
                crate::eta::install_stage_key(stage),
                crate::eta::stage_fraction(stage),
                Instant::now(),
            );
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
        let running = self.ui.sheets.switching.running.take();
        let stopped = running.as_ref().is_some_and(|run| run.stop.is_stopped());
        let tag = running.as_ref().map(|run| run.tag.clone());
        let back = running.and_then(|run| run.back);
        let result = if result.is_err() && stopped {
            Ok(Switched::Stayed)
        } else {
            result
        };
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

    pub(in crate::sheets) fn switch_sheet(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        use crate::eta::{stage_fraction, stage_label};
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
                    if let Some((_, reply)) = this
                        .ui
                        .sheets
                        .switching
                        .running
                        .as_mut()
                        .and_then(|running| running.question.take())
                    {
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
                    .children(omissions.then(|| {
                        crate::widgets::feedback::banner(
                            crate::widgets::feedback::Tone::Warning,
                            question.omission_summary(),
                            None,
                        )
                    }))
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
                            secondary(
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
                            secondary("switch-stay", tr("Cancel"))
                                .on_click(answer(SwitchDecision::Cancel)),
                        )
                        .children(omissions.then(|| {
                            secondary(
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
                            primary(
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
            let cancel = secondary("cancel-switch", tr("Cancel"));
            // The step's line, the bar and Cancel keep their places while the steps change, so
            // the dialog never jumps: the step's name with its time left as the figure, a bar
            // that shuttles while a step has no known length, and Cancel dimmed once it can no
            // longer stop the switch.
            let (label, left): (SharedString, Option<SharedString>) = if stopping {
                (tr("Cancelling…").into(), None)
            } else if publishing {
                (tr("Applying the switch.").into(), None)
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
                (
                    stage_label(running.progress).into(),
                    remaining.map(|left| crate::eta::eta_label(left).into()),
                )
            };
            div()
                .flex()
                .flex_col()
                .child(title(trf("Switch to {build}", &[("build", &build)])))
                .child(crate::widgets::feedback::progress(
                    label,
                    left,
                    stage_fraction(running.progress).filter(|_| !stopping),
                ))
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

/// Whether a picker selection can be saved directly to `instances.json` without running a build
/// switch: the profile must be ordinary, have no pending transaction recovery or first-Play
/// bootstrap, and be already on `tag` or never played.
pub(super) fn can_fast_save_choice(instance: &instances::Instance, tag: &str) -> bool {
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
        // Never played: the choice is all there is to change. Nothing downloads or builds until
        // the player presses Install (or Play), which installs whatever the profile then names.
        Ok(None) => true,
        Err(_) => false,
    }
}
