//! The Build changes report: reading a profile's receipt, its sheet and acknowledging it.
use super::*;
use crate::widgets::button::secondary;

impl Launcher {
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

    pub(super) fn receipt_status(&self, home: &Path) -> Option<&Load<Option<(Identity, Active)>>> {
        self.ui
            .sheets
            .switching
            .receipt
            .as_ref()
            .filter(|read| read.home == home)
            .map(|read| &read.status)
    }

    pub(super) fn read_receipt(&mut self, home: &Path, explicit: bool, cx: &mut Context<Self>) {
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

    pub(in crate::sheets) fn switch_receipt_sheet(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
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
                        crate::widgets::motion_bits::copy_button(
                            "receipt-copy",
                            tr("Copy report"),
                            copy.as_ref().clone(),
                        )
                    }))
                    .children(show.then(|| {
                        secondary("receipt-save", tr("Save report")).on_click(cx.listener(
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
                        secondary(
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
                    .child(secondary("receipt-done", tr("Done")).on_click(cx.listener(
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
pub(super) fn publish_report(
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
