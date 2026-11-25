//! When a registry build fails: the notice that says why, and saving its log.

use super::*;

impl Launcher {
    pub(super) fn registry_build_failed(
        &self,
        failure: &sunrise_launcher::browser::sunrise::BuildNotice,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let show = self.ui.sheets.details;
        let mut list = div()
            .id("registry-install-disposition")
            .flex()
            .flex_col()
            .max_h(px(222.))
            .overflow_y_scroll()
            .rounded(crate::tokens::r(crate::tokens::R_CARD))
            .border_1()
            .border_color(rgb(palette().divider));
        for (i, change) in failure.changes.iter().enumerate() {
            list = list.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .px(px(12.))
                    .py(px(7.))
                    .when(i > 0, |row| {
                        row.border_t_1().border_color(rgb(palette().divider))
                    })
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(change.title.clone()),
                    )
                    .child(muted(trf(
                        "{disposition} · {reason}",
                        &[
                            ("disposition", &change.disposition()),
                            (
                                "reason",
                                &sunrise_launcher::browser::installed::hold_short_reason(
                                    &change.reason,
                                ),
                            ),
                        ],
                    )))
                    .children(show.then(|| {
                        muted(sunrise_launcher::browser::installed::hold_detail(
                            &change.reason,
                        ))
                    })),
            );
        }
        let actions = show.then(|| {
            let mut controls = div().flex().gap(px(8.)).items_center();
            if let Some(log) = &failure.log {
                let copy = log.clone();
                let save = log.clone();
                controls = controls
                    .child(
                        button("install-log-copy", tr("Copy")).on_click(move |_, _, cx| {
                            crate::widgets::copy_to_clipboard(
                                cx,
                                gpui::ClipboardItem::new_string(copy.to_string()),
                            );
                        }),
                    )
                    .child(
                        button("install-log-save", tr("Save log")).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.save_install_log(save.clone(), cx);
                            },
                        )),
                    );
            } else {
                controls = controls.child(muted(tr("No build log was saved for this attempt.")));
            }
            controls
        });
        let body = div().flex().flex_col()
            .child(title(tr("Some mods couldn't be built").into()))
            .child(muted(tr("Stay to leave all your mods unchanged. Continue to install the checked set with the changes below.")))
            .child(list)
            .children(actions)
            .children(self.ui.sheets.error.as_ref().map(|error| muted(error.clone())))
            .child(div().flex().justify_end().gap(px(8.))
                .child(button("install-show-why", if show { tr("Hide why") } else { tr("Show why") })
                    .on_click(cx.listener(|this, _, _, cx| { this.ui.sheets.details ^= true; cx.notify(); })))
                .child(crate::frame::primary("install-stay", tr("Stay"))
                    .on_click(cx.listener(|this, _, _, cx| { this.close_sheet(); cx.notify(); })))
                .child(button("install-continue", tr("Continue"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some((_, answer)) = this.mods.installs.question.take() { let _ = answer.send(true); }
                        this.ui.sheets.details = false;
                        this.ui.sheets.error = None;
                        cx.notify();
                    }))));
        frame("registry-build-failed", 690., body, cx)
    }

    fn save_install_log(&mut self, log: std::sync::Arc<str>, cx: &mut Context<Self>) {
        let directory = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(backend::data_dir);
        let save = cx.prompt_for_new_path(&directory, Some("sunrise-build.log"));
        cx.spawn(async move |this, cx| {
            let result = match save.await {
                Ok(Ok(Some(path))) => {
                    cx.background_executor()
                        .spawn(async move {
                            std::fs::write(path, log.as_bytes()).map_err(|e| e.to_string())
                        })
                        .await
                }
                Ok(Ok(None)) => return,
                Ok(Err(error)) => Err(error.to_string()),
                Err(_) => return,
            };
            let _ = this.update(cx, |this, cx| {
                if this.mods.installs.question.is_some() {
                    this.ui.sheets.error = Some(match result {
                        Ok(()) => tr("Build log saved.").into(),
                        Err(reason) => trf(
                            "Couldn't save the build log: {reason}",
                            &[("reason", &reason)],
                        ),
                    });
                    cx.notify();
                }
            });
        })
        .detach();
    }
}
