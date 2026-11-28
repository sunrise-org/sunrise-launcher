//! Settings › Storage: game builds, downloads and where files are kept.
use super::*;

impl Launcher {
    pub(super) fn settings_storage(&mut self, cx: &mut Context<Self>) -> Div {
        let game = backend::game_home();
        let files = row(
            tr("Game files and worlds"),
            short(&game),
            div().child({
                let dir = game.clone();
                button("alt-open-game", tr("Open folder"))
                    .on_click(move |_, _, cx| toast::open(cx, &dir))
            }),
        );

        let busy = self.ui.settings.storage_busy();
        // The total waits for every row; a row shows its own size as soon as it is counted.
        let complete = self.storage_report.as_ref().filter(|r| r.is_complete());
        let total =
            complete.map(|report| report.rows.iter().map(|r| r.remaining_bytes).sum::<u64>());
        // Clear needs no size: a row not counted yet can be cleared like any other.
        let offered = storage_report::clear_offered(
            self.ui.settings.storage_work.as_ref(),
            self.location_unavailable,
        );
        // Counting that failed is not still counting.
        let uncounted = if self.storage_error.is_some() && !busy {
            tr("Not counted")
        } else {
            tr("Counting…")
        };
        let clear_all = button("alt-storage-clear-all", tr("Clear all…"));
        let size = total.map_or_else(|| uncounted.to_string(), storage::format_bytes);
        let detail = trf(
            "{size}. Files Sunrise can fetch or make again; your worlds, mods and settings are never part of it.",
            &[("size", &size)],
        );
        let detail = if size.ends_with('…') {
            detail.replacen("….", "…", 1)
        } else {
            detail
        };
        let mut cache = vec![row(
            tr("Downloads kept for reuse"),
            detail,
            div().child(
                if offered
                    && complete.is_none_or(|r| {
                        r.rows
                            .iter()
                            .any(|row| storage_report::row_clearable(Some(row)))
                    })
                {
                    clear_all.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.storage_confirm = Some(None);
                        cx.notify();
                    }))
                } else {
                    off(clear_all)
                },
            ),
        )];
        // Neither starting nor finishing an automatic count replaces Clear feedback.
        cache.extend(self.storage_error.as_ref().map(|error| hint(error.clone())));
        let cleared = match &self.ui.settings.storage_clear {
            Some(Ok(report)) => Some(report),
            Some(Err(error)) => {
                cache.push(hint(error.clone()));
                None
            }
            None => None,
        };
        if let Some(asked) = self.storage_confirm {
            // The size to free is offered only for rows that are counted.
            let counted: Vec<_> = self
                .storage_report
                .iter()
                .flat_map(|report| &report.rows)
                .filter(|r| asked.is_none_or(|category| r.category == category))
                .collect();
            let wanted = if asked.is_none() {
                Category::ALL.len()
            } else {
                1
            };
            let bytes = (counted.len() == wanted
                && counted.iter().all(|r| r.measured_bytes().is_some()))
            .then(|| counted.iter().map(|r| r.clearable_bytes()).sum())
            .filter(|&bytes| bytes > 0);
            let question = storage_report::clear_question(asked, bytes);
            cache.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .child(div().flex_1().min_w_0().child(question))
                    .child(button("alt-storage-keep", tr("Keep")).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.storage_confirm = None;
                            cx.notify();
                        },
                    )))
                    .child(
                        danger(button("alt-storage-clear-yes", tr("Clear"))).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                if let Some(clear) = this.storage_confirm.take() {
                                    this.refresh_storage(clear, clear.is_none());
                                }
                                cx.notify();
                            },
                        )),
                    ),
            );
        }
        for (n, category) in Category::ALL.into_iter().enumerate() {
            let found = self
                .storage_report
                .as_ref()
                .and_then(|report| report.rows.iter().find(|r| r.category == category));
            let size = found
                .and_then(storage::Row::measured_bytes)
                .map_or_else(|| uncounted.to_string(), storage::format_bytes);
            let clear = button(ElementId::named_usize("alt-storage-clear", n), tr("Clear…"));
            let mut line = row(
                trf(
                    "{category} · {size}",
                    &[("category", category.label()), ("size", &size)],
                ),
                storage_report::category_about(category),
                div().child(if offered && storage_report::row_clearable(found) {
                    clear.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.storage_confirm = Some(Some(category));
                        cx.notify();
                    }))
                } else {
                    off(clear)
                }),
            );
            let feedback = cleared.and_then(|report| {
                report
                    .rows
                    .iter()
                    .find(|r| r.category == category && r.after_clear)
            });
            if let Some(kept) = feedback
                .and_then(storage_report::kept_line)
                .or_else(|| found.and_then(storage_report::kept_line))
            {
                line = div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(line)
                    .child(hint(kept));
            }
            cache.push(line);
        }
        let notes = [
            self.client
                .is_some()
                .then(|| hint(tr("A game is running. Files it may use are kept."))),
            busy.then(|| hint(tr("Counting, or clearing…"))),
        ];
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(group(vec![self.location_row(cx)]))
            .child(self.storage_build_rows(cx))
            .child(self.storage_build_options(cx))
            .child(group(vec![files]))
            .child(group(cache))
            .children(notes.into_iter().flatten())
    }
}
