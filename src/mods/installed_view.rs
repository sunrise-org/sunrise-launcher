//! Mods › Installed (mocks 05, 05b): what needs the player first, then updates, then the rest;
//! one switch per row; bulk actions that name their count and scope.
use super::model::{self, Make, Row, Show};
use super::ui::*;
use super::*;
use crate::paint::palette;
use std::rc::Rc;
use sunrise_launcher::browser::images;

impl Launcher {
    pub(super) fn installed(
        &mut self,
        window: &mut Window,
        focus: &FocusHandle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let header = self.mods_header(true, focus, window, cx);
        let mut page = div()
            .w_full()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(24.))
            .text_color(c(palette().text))
            .child(header)
            .children(self.strip(cx));

        let rows = self.rows();
        if rows.is_empty() {
            return page
                .child(
                    card()
                        .p(px(24.))
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap(px(12.))
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(tr("No mods yet")),
                        )
                        .child(primary("alt-installed-browse", tr("Find mods")).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.choose_view(Page::Discover);
                                cx.notify();
                            }),
                        )),
                )
                .into_any_element();
        }

        let filter = self.ui.mods.row_filter.clone();
        let visible = model::visible(&rows, &filter);
        let shown_ids: Rc<Vec<String>> = Rc::new(
            visible
                .iter()
                .filter(|&&i| rows[i].local.is_none())
                .map(|&i| rows[i].id.clone())
                .collect(),
        );
        let selected: Vec<&Row> = rows
            .iter()
            .filter(|r| r.local.is_none() && self.ui.mods.selection.contains(&r.id))
            .collect();

        // ---- what to show: the same row of chips Discover filters with
        let count = |f: &dyn Fn(&Row) -> bool| rows.iter().filter(|r| f(r)).count();
        let counted = |label: &str, n: usize| {
            trf("{label} · {n}", &[("label", label), ("n", &n.to_string())])
        };
        let mut chips = div().flex().flex_wrap().items_center().gap(px(8.));
        let shown = [
            (tr("All"), rows.len(), Show::All),
            (tr("Turned on"), count(&|r| r.on), Show::On),
            (tr("Turned off"), count(&|r| !r.on), Show::Off),
        ]
        .into_iter()
        .map(|(label, n, which)| {
            let pick: Choose = Rc::new(move |this, _| {
                let m = &mut this.ui.mods;
                m.row_filter.show = which;
                m.selection.clear();
                m.anchor = None;
            });
            (counted(label, n).into(), filter.show == which, pick)
        })
        .collect();
        chips = chips.child(choices("alt-show", shown, cx).gap(px(8.)));
        chips = chips.child(div().flex_none().w(px(1.)).h(px(20.)).bg(c(palette().line)));
        for (i, make) in [Make::Native, Make::Content].into_iter().enumerate() {
            let n = count(&|r| r.make == Some(make));
            chips = chips.child(
                // Two switches, not one choice: each is its own Tab stop.
                chip(
                    ("alt-kind", i),
                    counted(make.label(), n),
                    filter.makes.contains(&make),
                )
                .tab_index(0)
                .seen()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let m = &mut this.ui.mods;
                    if !m.row_filter.makes.remove(&make) {
                        m.row_filter.makes.insert(make);
                    }
                    m.selection.clear();
                    m.anchor = None;
                    cx.notify();
                })),
            );
        }
        page = page.child(chips);

        // ---- toolbar: select-all and the bulk actions, naming their scope
        let selected_here = shown_ids
            .iter()
            .filter(|id| self.ui.mods.selection.contains(*id))
            .count();
        let all_on = selected_here > 0 && selected_here == shown_ids.len();
        let n_sel = selected.len();
        let busy = self.mods_busy();
        let with_update = {
            let chosen: Vec<Row> = selected.iter().map(|r| (*r).clone()).collect();
            model::updatable(&chosen).len()
        };
        let bulk = |id: &'static str, label: String, enabled: bool, danger: bool| {
            let b = crate::frame::button(id, label);
            let b = if danger {
                b.text_color(c(palette().danger))
            } else {
                b
            };
            if enabled && !busy {
                b
            } else {
                dim(b)
            }
        };
        let toolbar = {
            let ids = shown_ids.clone();
            let select_all = checkbox("alt-select-all", all_on)
                .when(selected_here > 0 && !all_on, |d| {
                    d.bg(c(palette().text))
                        .border_color(c(palette().text))
                        .child(div().w(px(10.)).h(px(2.)).bg(c(palette().background)))
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let m = &mut this.ui.mods;
                    match ids.iter().all(|id| m.selection.contains(id)) {
                        true => m.selection.clear(),
                        false => m.selection.extend(ids.iter().cloned()),
                    }
                    m.anchor = None;
                    cx.notify();
                }));
            let mut bar = div()
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(14.))
                .h(px(48.))
                .border_b_1()
                .border_color(c(palette().divider))
                .child(select_all);
            if n_sel == 0 {
                bar = bar.child(muted(model::visible_count(&rows, &visible)));
            } else {
                let on = bulk(
                    "alt-bulk-on",
                    trf("Turn on {count}", &[("count", &n_sel.to_string())]),
                    true,
                    false,
                );
                let off = bulk(
                    "alt-bulk-off",
                    trf("Turn off {count}", &[("count", &n_sel.to_string())]),
                    true,
                    false,
                );
                let upd = bulk(
                    "alt-bulk-update",
                    trf("Update {count}", &[("count", &with_update.to_string())]),
                    with_update > 0,
                    false,
                );
                let rm = bulk(
                    "alt-bulk-remove",
                    trf("Remove {count}…", &[("count", &n_sel.to_string())]),
                    true,
                    true,
                );
                bar =
                    bar.child(div().text_size(px(tokens::LABEL)).child(
                        crate::selectable::selectable(trf(
                            "{count} selected of {shown} shown",
                            &[
                                ("count", &n_sel.to_string()),
                                ("shown", &shown_ids.len().to_string()),
                            ],
                        )),
                    ))
                    .child(if !busy {
                        on.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.bulk_on();
                            cx.notify();
                        }))
                    } else {
                        on
                    })
                    .child(if !busy {
                        off.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.bulk_off();
                            cx.notify();
                        }))
                    } else {
                        off
                    })
                    .child(if !busy && with_update > 0 {
                        upd.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.update_selected();
                            cx.notify();
                        }))
                    } else {
                        upd
                    })
                    .child(if !busy {
                        rm.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.ask_remove();
                            cx.notify();
                        }))
                    } else {
                        rm
                    })
                    .child(div().flex_1())
                    .child(
                        muted(tr("Clear"))
                            .id("alt-selection-clear")
                            .reach()
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.ui.mods.selection.clear();
                                this.ui.mods.anchor = None;
                                cx.notify();
                            })),
                    );
            }
            bar
        };

        // ---- the confirmation, when one is open: names, counts, nothing started yet
        let confirm = self.ui.mods.confirm.as_ref().map(|confirm| {
            let panel = card()
                .m(px(12.))
                .p(px(16.))
                .flex()
                .flex_col()
                .gap(px(10.))
                .border_color(c(palette().warning));
            match confirm {
                Confirm::Remove(chosen) => {
                    let ids: HashSet<String> = chosen.iter().map(|(id, ..)| id.clone()).collect();
                    let index = sunrise::fetched_index();
                    let dependents = model::dependents(&rows, &ids, index.as_deref());
                    let names = chosen
                        .iter()
                        .map(|(_, _, t)| t.as_str())
                        .take(12)
                        .collect::<Vec<_>>()
                        .join(", ");
                    let more = chosen.len().saturating_sub(12);
                    let count = chosen.len().to_string();
                    panel
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(trf(
                            "Remove {count} mods?",
                            &[("count", &chosen.len().to_string())],
                        )))
                        .child(div().text_size(px(tokens::LABEL)).child(match more {
                            0 => names,
                            n => trf(
                                "{names} and {more} more",
                                &[("names", &names), ("more", &n.to_string())],
                            ),
                        }))
                        .children(dependents.into_iter().map(|(who, needed)| {
                            div()
                                .text_size(px(tokens::LABEL))
                                .text_color(c(palette().warning))
                                .child(crate::selectable::selectable(trf(
                                    "{who} needs {needed}: it may stop working.",
                                    &[("who", &who), ("needed", &needed.join(", "))],
                                )))
                        }))
                        .child(muted(tr("Their files are deleted.")))
                        .child(
                            div()
                                .flex()
                                .gap(px(10.))
                                .child(
                                    crate::frame::button("alt-confirm-cancel", tr("Cancel"))
                                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                            this.ui.mods.confirm = None;
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    crate::frame::button(
                                        "alt-confirm-remove",
                                        trf("Remove {count}", &[("count", &count)]),
                                    )
                                    .text_color(c(palette().danger))
                                    .on_click(cx.listener(
                                        move |this, _: &ClickEvent, _, cx| {
                                            this.remove_confirmed();
                                            cx.notify();
                                        },
                                    )),
                                ),
                        )
                }
            }
        });

        // ---- rows, with one button above them when any update is waiting
        let mut list = div().flex().flex_col();
        let mut position = 0;
        let waiting = model::updatable(&rows).len();
        if waiting > 0 {
            let b = crate::frame::button("alt-update-all", tr("Update all"));
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(14.))
                    .min_h(px(tokens::FIELD))
                    .bg(c(palette().background))
                    .child(
                        div()
                            .text_size(px(tokens::CAPTION))
                            .text_color(c(palette().muted))
                            .child(crate::selectable::selectable(trf(
                                "Updates waiting: {count}",
                                &[("count", &waiting.to_string())],
                            ))),
                    )
                    .child(if !busy {
                        b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.update_all();
                            cx.notify();
                        }))
                    } else {
                        dim(b)
                    }),
            );
        }
        for (n, &i) in visible.iter().enumerate() {
            let r = rows[i].clone();
            let pos = position;
            if r.local.is_none() {
                position += 1;
            }
            list = list.child(self.row(n, pos, r, shown_ids.clone(), busy, cx));
        }
        let reading = self.ui.mods.snapshot.as_ref().is_some_and(|s| s.reading);
        if reading {
            list = list.child(
                div()
                    .p(px(16.))
                    .child(muted(tr("Reading the mods you added by hand…"))),
            );
        }
        // Hand-added mods may still arrive, so an empty list isn't final while reading.
        if visible.is_empty() && !reading {
            list = list.child(
                div()
                    .p(px(16.))
                    .child(muted(tr("No mods match this search or filter."))),
            );
        }
        let hidden = self
            .ui
            .mods
            .snapshot
            .as_ref()
            .map_or(0, |s| s.entries.len().saturating_sub(rows.len()));
        let right = card()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(toolbar)
            .children(confirm)
            .child(div().id("alt-installed-scroll").flex_1().min_h_0().overflow_y_scroll().track_scroll(&crate::frame::scrolls("installed")).child(list))
            .children((hidden > 0).then(|| {
                div().px(px(14.)).py(px(8.)).border_t_1().border_color(c(palette().divider)).child(muted(trf(
                    "{count} other files the launcher installed (packs, or from other sources) aren't listed here.",
                    &[("count", &hidden.to_string())],
                )))
            }));
        page = page.child(right);
        page.into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn row(
        &mut self,
        n: usize,
        pos: usize,
        r: Row,
        ids: Rc<Vec<String>>,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let selected = self.ui.mods.selection.contains(&r.id);
        let picture = r
            .icon
            .clone()
            .and_then(|u| self.mods_image(&u, images::ROW));
        let error = self.ui.mods.errors.get(&r.id).cloned();

        let check = {
            let id = r.id.clone();
            checkbox(("alt-row-check", n), selected).on_click(cx.listener(
                move |this, ev: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    let m = &mut this.ui.mods;
                    match (ev.modifiers().shift, m.anchor) {
                        (true, Some(a)) => {
                            let (lo, hi) = (a.min(pos), a.max(pos));
                            m.selection
                                .extend(ids[lo..=hi.min(ids.len() - 1)].iter().cloned());
                        }
                        _ => {
                            if !m.selection.remove(&id) {
                                m.selection.insert(id.clone());
                            }
                        }
                    }
                    m.anchor = Some(pos);
                    cx.notify();
                },
            ))
        };

        // The list's shared columns (`model::cells`): kind, version, status, the one action
        // and the switch, each in its own fixed place on every row of every group. Narrow,
        // kind and version move under the name with the status after them, in fixed places
        // too; the action and the switch keep theirs.
        let wide = self.content_width() >= 860.;
        let cells = model::cells(&r, error.as_deref());
        let tone = |tone: model::Tone| match tone {
            model::Tone::Warning => palette().warning,
            model::Tone::Info => palette().info,
            model::Tone::Muted => palette().muted,
        };
        let status_cell: AnyElement = match &cells.status {
            Some(said) => status(said.icon, said.label.clone(), tone(said.tone))
                .id(("alt-row-status", n))
                .when(matches!(r.need, Some(model::Need::Waiting(_))), |label| {
                    label.flex_1()
                })
                .min_w_0()
                .tooltip(crate::glass_ui::tip(said.tip.clone()))
                .into_any_element(),
            None => div().into_any_element(),
        };
        let action = cells.action.map(|action| {
            let model::RowAction::Update = action;
            let b = crate::frame::button(("alt-row-update", n), tr("Update"));
            if busy {
                return dim(b).on_click(|_, _, cx| cx.stop_propagation());
            }
            let row = r.clone();
            b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.update_one(&row);
                cx.notify();
            }))
        });
        let sw = {
            let s = ui::switch(("alt-row-switch", n), r.on);
            if cells.switch {
                let row = r.clone();
                s.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.toggle(&row);
                    cx.notify();
                }))
            } else {
                s.opacity(0.45)
                    .cursor_default()
                    .tab_stop(false)
                    .on_click(|_, _, cx| cx.stop_propagation())
            }
        };
        let title = div()
            .min_w_0()
            .flex()
            .items_baseline()
            .gap(px(8.))
            .child(
                div()
                    .min_w_0()
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(r.title.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(tokens::SMALL))
                    .text_color(c(palette().muted))
                    .truncate()
                    .child(r.authors.clone()),
            );
        let title =
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(title)
                .when(r.registry_unchecked, |d| {
                    d.child(
                        div()
                            .text_size(px(tokens::SMALL))
                            .text_color(c(palette().muted))
                            .child(tr("Registry not checked")),
                    )
                });
        use model::column;
        let kind = |width: f32| {
            div()
                .w(px(width))
                .flex_none()
                .text_size(px(tokens::LABEL))
                .text_color(c(palette().muted))
                .child(cells.kind)
        };
        let version = |width: f32| {
            div()
                .w(px(width))
                .flex_none()
                .truncate()
                .text_size(px(tokens::LABEL))
                .font_family(crate::theme::MONO)
                .child(cells.version.clone())
        };
        let row = div()
            .id(("alt-row", n))
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(14.))
            .min_h(px(52.))
            .border_t_1()
            .border_color(c(palette().divider))
            .when(selected, |d| d.bg(c(palette().selected)))
            // The mod another page opened Installed at.
            .when(
                self.ui.mods.reveal == Some(search::Reveal::Mod(r.id.clone())),
                |d| {
                    d.bg(c(palette().hover)).child(super::ui::reveal(
                        crate::frame::scrolls("installed"),
                        self.ui.mods.revealed.clone(),
                        None,
                    ))
                },
            )
            .child(match r.local {
                Some(_) => div().flex_none().w(px(18.)),
                None => div().flex_none().child(check),
            })
            .child(tile(picture, &r.id, &r.title, 32.));
        let row = if wide {
            row.child(title.flex_1().min_w(px(140.)))
                .child(kind(column::KIND))
                .child(version(column::VERSION))
                .child(
                    div()
                        .w(px(column::STATUS))
                        .flex_none()
                        .flex()
                        .items_center()
                        .child(status_cell),
                )
                .child(
                    div()
                        .w(px(column::ACTION))
                        .flex_none()
                        .flex()
                        .justify_end()
                        .children(action),
                )
        } else {
            row.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .py(px(8.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(title)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(kind(64.))
                            .child(version(60.))
                            .child(div().flex_1().min_w_0().flex().child(status_cell)),
                    ),
            )
            .children(action)
        };
        row.reach()
            .cursor_pointer()
            .hover(|s| s.bg(c(palette().hover)))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.mods_show_installed_project(r.clone());
                cx.notify();
            }))
            .child(
                div()
                    .w(px(column::SWITCH))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .child(sw),
            )
    }
}
