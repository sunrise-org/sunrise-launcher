use super::*;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use sunrise_launcher::{
    browser::sunrise::{
        install_target, switch_build, InstallProgress, SwitchHooks, SwitchQuestion, Switched,
    },
    managed_play::{self, Active, Profile, ReceiptHint},
    profile_txn::Identity,
};

#[derive(Default)]
pub(crate) struct State {
    pub(super) cache_download: Option<String>,
    running: Option<Running>,
    ack: bool,
    receipt: Option<ReceiptRead>,
}

struct ReceiptRead {
    home: PathBuf,
    hint: Result<Option<ReceiptHint>, String>,
    ticket: Arc<()>,
    status: Load<Option<(Identity, Active)>>,
}

struct Running {
    tag: String,
    progress: InstallProgress,
    stop: native_mods::Cancellation,
    question: Option<(Box<SwitchQuestion>, Sender<bool>)>,
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
        result: &Result<(), String>,
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
        if let Err(error) = result {
            self.ui.sheets.error = Some(error.clone());
        }
        true
    }

    pub(crate) fn switch_busy(&self) -> bool {
        self.ui.sheets.switching.running.is_some() || self.ui.sheets.switching.ack
    }

    pub(super) fn begin_switch(&mut self, tag: String) {
        if self.launch_busy() || self.client.is_some() {
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
        let stop = native_mods::Cancellation::default();
        self.ui.sheets.switching.running = Some(Running {
            tag: tag.clone(),
            progress: InstallProgress::Checking,
            stop: stop.clone(),
            question: None,
        });
        self.open_sheet(Sheet::Switching);
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let outcome = switch_build(
                &instance,
                identity,
                &tag,
                SwitchHooks {
                    cancellation: &stop,
                    progress: &|stage| {
                        let _ = tx.send(Msg::SwitchProgress(stage));
                    },
                    registry_crates: Some(&|inventory| {
                        let (answer, reply) = std::sync::mpsc::channel();
                        tx.send(Msg::NativeRegistryConsent(inventory.clone(), answer))
                            .is_ok()
                            && reply.recv().unwrap_or(false)
                    }),
                    at_cutoff: &|| Ok(()),
                    approve: &|question| {
                        let (answer, reply) = std::sync::mpsc::channel();
                        tx.send(Msg::SwitchQuestion(Box::new(question.clone()), answer))
                            .map_err(|_| tr("Switch cancelled.").to_owned())?;
                        Ok(reply.recv().unwrap_or(false))
                    },
                },
            );
            let _ = tx.send(Msg::Switched(instance.home, Box::new(outcome)));
        });
    }

    pub(super) fn cancel_switch(&mut self) {
        let Some(running) = &mut self.ui.sheets.switching.running else {
            return;
        };
        if running.progress != InstallProgress::Publishing {
            running.stop.stop();
            if let Some((_, reply)) = running.question.take() {
                let _ = reply.send(false);
            }
        }
    }

    pub(crate) fn switch_progress(&mut self, stage: InstallProgress) {
        if let Some(running) = &mut self.ui.sheets.switching.running {
            running.progress = stage;
        }
    }

    pub(crate) fn switch_question(&mut self, question: Box<SwitchQuestion>, reply: Sender<bool>) {
        if let Some(running) = self
            .ui
            .sheets
            .switching
            .running
            .as_mut()
            .filter(|_| self.ui.sheet == Some(Sheet::Switching))
        {
            running.question = Some((question, reply));
            self.ui.sheets.details = false;
        } else {
            let _ = reply.send(false);
        }
    }

    pub(crate) fn switch_finished(
        &mut self,
        home: &std::path::Path,
        result: Result<Switched, String>,
    ) {
        self.ui.sheets.switching.running = None;
        self.installed = backend::installed();
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
            Ok(Switched::Stayed | Switched::AlreadySelected) => {
                if visible {
                    self.open_sheet(Sheet::Versions);
                }
            }
            Err(error) if visible => {
                self.open_notice(error, Some(WayOut::ProfileFolder(home.into())))
            }
            Err(_) => (),
        }
    }

    pub(super) fn switch_sheet(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        use crate::mods_page::install::{stage_fraction, stage_label};
        let running = self
            .ui
            .sheets
            .switching
            .running
            .as_ref()
            .expect("Switch sheet follows its worker");
        let build = build_name(&running.tag);
        let head = title(trf("Switch to {build}", &[("build", &build)]));
        let body = if let Some((question, _)) = &running.question {
            let show = self.ui.sheets.details;
            div()
                .flex()
                .flex_col()
                .child(head)
                .child(muted(tr(
                    "Your worlds stay in place. Check what could change before you switch.",
                )))
                .child(
                    div()
                        .id("switch-worlds")
                        .max_h(px(208.))
                        .overflow_y_scroll()
                        .gap(px(5.))
                        .children(
                            question
                                .worlds
                                .lines()
                                .into_iter()
                                .map(|line| div().child(line)),
                        )
                        .children(question.hold_lines().map(|line| div().child(line)))
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
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .justify_end()
                        .gap(px(8.))
                        .child(
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
                            })),
                        )
                        .child(
                            crate::frame::primary("switch-stay", tr("Stay on this version"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_switch();
                                    this.open_sheet(Sheet::Versions);
                                    cx.notify();
                                })),
                        )
                        .child(
                            button("switch-anyway", tr("Switch anyway")).on_click(cx.listener(
                                |this, _, _, cx| {
                                    if let Some((_, reply)) = this
                                        .ui
                                        .sheets
                                        .switching
                                        .running
                                        .as_mut()
                                        .and_then(|running| running.question.take())
                                    {
                                        let _ = reply.send(true);
                                    }
                                    cx.notify();
                                },
                            )),
                        ),
                )
        } else {
            div()
                .flex()
                .flex_col()
                .child(head)
                .child(muted(if running.progress == InstallProgress::Publishing {
                    tr("Applying the switch.").into()
                } else {
                    stage_label(running.progress)
                }))
                .children(
                    stage_fraction(running.progress)
                        .map(|f| crate::widgets::progress_bar(Some((f, f)), &self.theme)),
                )
                .children((running.progress != InstallProgress::Publishing).then(|| {
                    div().flex().justify_end().child(
                        button("cancel-switch", tr("Cancel")).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.close_sheet();
                                cx.notify();
                            },
                        )),
                    )
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
                "{mod} was taken down: {reason}",
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
                    trf("{mod} was taken down and removed.", &[("mod", &row.0)])
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
                        button("receipt-copy", tr("Copy report")).on_click(cx.listener(
                            move |_, _, _, cx| {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                    copy.as_ref().clone(),
                                ));
                            },
                        ))
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
    use super::publish_report;
    use std::io::{self, Write};

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
