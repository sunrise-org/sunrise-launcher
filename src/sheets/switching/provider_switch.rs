//! Switching a mod's provider: the worker, its question and its sheet.
use super::*;
use crate::widgets::button::{primary, secondary};

impl Launcher {
    pub(crate) fn begin_provider_switch(&mut self, switch: PortSwitch) {
        if self.launch_busy() || self.client.is_some() || self.switch_busy() {
            return;
        }
        let Some(instance) = self.selected_instance().cloned() else {
            return;
        };
        let target_profile = match Target::of(&instance) {
            Ok(target) => target,
            Err(error) => {
                self.open_notice(error.into(), Some(WayOut::ProfileFolder(instance.home)));
                return;
            }
        };
        let identity = target_profile.identity();
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
                &target_profile,
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

    pub(crate) fn cancel_provider_switch(&mut self) {
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
                crate::eta::stage_fraction(stage),
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
            (self.ui.sheets.switching.provider.as_mut()).filter(|run| !run.stop.is_stopped())
        {
            running.question = Some((preview, reply));
            // As a build switch's: a hidden switch comes back for its answer.
            if self.ui.sheet.is_none() {
                self.open_sheet(Sheet::SwitchProvider);
            }
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

    pub(crate) fn on_provider_message(&mut self, msg: Msg) {
        match msg {
            Msg::ProviderSwitchProgress(stage) => self.provider_switch_progress(stage),
            Msg::ProviderSwitchQuestion(question, reply) => {
                self.provider_switch_question(question, reply)
            }
            Msg::ProviderSwitched(home, result) => self.provider_switch_finished(&home, result),
            Msg::ProviderChoice(need, reply) => self.provider_choice_question(need, reply),
            Msg::PublisherChangeQuestion(question, reply) => {
                self.publisher_change_question(question, reply)
            }
            Msg::ReceiptAcknowledged(home, result) => self.receipt_acknowledged(&home, result),
            _ => unreachable!("not a provider or publisher message"),
        }
    }

    pub(in crate::sheets) fn provider_switch_sheet(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        use crate::eta::{stage_fraction, stage_label};
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
                            secondary("provider-switch-cancel", tr("Cancel"))
                                .on_click(answer(false)),
                        )
                        .child(
                            primary("provider-switch-confirm", tr("Switch"))
                                .on_click(answer(true)),
                        ),
                )
        } else {
            let publishing = running.progress == InstallProgress::Publishing;
            let cancel = secondary("provider-switch-cancel", tr("Cancel"));
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
                .child(crate::widgets::feedback::progress(
                    stage_label(running.progress),
                    (!publishing)
                        .then(|| running.eta.remaining(Instant::now()))
                        .flatten()
                        .map(|left| crate::eta::eta_label(left).into()),
                    stage_fraction(running.progress),
                ))
                .child(muted(tr("Your current port stays intact if you cancel.")))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(hide_button("hide-provider-switch", cx))
                        .child(cancel),
                )
        };
        Some(frame("provider-switch-sheet", 560., body, cx))
    }
}
