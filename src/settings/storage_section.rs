//! Settings › Storage: game builds, downloads and where files are kept.
use super::*;
use crate::widgets::button::secondary;

impl Launcher {
    pub(super) fn settings_storage(&mut self, cx: &mut Context<Self>) -> Div {
        let game = backend::game_home();
        let files = row(
            tr("Game files and worlds"),
            short(&game),
            div().child({
                let dir = game.clone();
                secondary("alt-open-game", tr("Open folder"))
                    .on_click(move |_, _, cx| toast::open(cx, &dir))
            }),
        );

        let busy = self.storage_pending;
        let total = self
            .storage_report
            .as_ref()
            .map(|report| report.rows.iter().map(|r| r.remaining_bytes).sum::<u64>());
        let nothing = busy
            || self
                .storage_report
                .as_ref()
                .is_none_or(|report| report.rows.iter().all(|row| row.clearable_bytes() == 0));
        // Counting that failed is not still counting.
        let uncounted = if self.storage_error.is_some() && !busy {
            tr("Not counted")
        } else {
            tr("Counting…")
        };
        let clear_all = secondary("alt-storage-clear-all", tr("Clear all…"));
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
            div()
                .child(
                    secondary("alt-storage-recount", tr("Recount")).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.refresh_storage(None, false);
                            cx.notify();
                        },
                    )),
                )
                .child(if nothing {
                    off(clear_all)
                } else {
                    clear_all.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.storage_confirm = Some(None);
                        cx.notify();
                    }))
                }),
        )];
        // A problem no row owns is said once, quietly, under the total.
        if let Some(error) = self.storage_error.clone() {
            let total = cache.remove(0);
            cache.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(total)
                    .child(hint(error)),
            );
        }
        if let Some(asked) = self.storage_confirm {
            let bytes: u64 = self
                .storage_report
                .iter()
                .flat_map(|report| &report.rows)
                .filter(|r| asked.is_none_or(|category| r.category == category))
                .map(storage::Row::clearable_bytes)
                .sum();
            let size = storage::format_bytes(bytes);
            let question = match asked {
                None => trf(
                    "Clear everything kept for reuse? Up to {size} is freed. Files a game or build is using stay.",
                    &[("size", &size)],
                ),
                Some(category) => trf(
                    "Clear {category}? Up to {size} is freed. Files a game or build is using stay.",
                    &[
                        ("category", tr(storage_report::category_label(category))),
                        ("size", &size),
                    ],
                ),
            };
            cache.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .child(div().flex_1().min_w_0().child(question))
                    .child(
                        secondary("alt-storage-keep", tr("Keep")).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.storage_confirm = None;
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        danger(secondary("alt-storage-clear-yes", tr("Clear"))).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                if let Some(clear) = this.storage_confirm.take() {
                                    this.refresh_storage(clear, clear.is_none());
                                }
                                cx.notify();
                            }),
                        ),
                    ),
            );
        }
        for (n, category) in Category::ALL.into_iter().enumerate() {
            let found = self
                .storage_report
                .as_ref()
                .and_then(|report| report.rows.iter().find(|r| r.category == category));
            let size = found.map_or_else(
                || uncounted.to_string(),
                |r| storage::format_bytes(r.remaining_bytes),
            );
            let empty = busy || found.is_none_or(|r| r.clearable_bytes() == 0);
            let clear = secondary(format!("alt-storage-clear-{}", n), tr("Clear…"));
            let mut line = row(
                trf(
                    "{category} · {size}",
                    &[
                        ("category", tr(storage_report::category_label(category))),
                        ("size", &size),
                    ],
                ),
                tr(storage_report::category_about(category)),
                div().child(if empty {
                    off(clear)
                } else {
                    clear.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.storage_confirm = Some(Some(category));
                        cx.notify();
                    }))
                }),
            );
            let kept: Vec<Div> = found
                .iter()
                .flat_map(|r| &r.kept)
                .map(|k| {
                    // A unit that was refused whole was not counted either.
                    if k.bytes == 0 {
                        return hint(k.reason.clone());
                    }
                    hint(trf(
                        "Space it couldn't free: {bytes}. {reason}",
                        &[
                            ("bytes", &storage::format_bytes(k.bytes)),
                            ("reason", &k.reason),
                        ],
                    ))
                })
                .collect();
            if !kept.is_empty() {
                line = div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(line)
                    .children(kept);
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
