//! Mods › Installed (mocks 05, 05b): what needs the player first, then updates, then the rest;
//! one switch per row; bulk actions that name their count and scope.
use super::model::{self, Group, Make, Need, Row, Show};
use super::ui::*;
use super::*;
use crate::frame::{BG, GOLD, MUTED, SOFT, TEXT};
use std::rc::Rc;
use sunrise_launcher::browser::images;

const BLUE: u32 = 0x7fb0ff;
const DANGER: u32 = 0xf08a7a;

impl Launcher {
    pub(super) fn installed(
        &mut self,
        window: &mut Window,
        focus: &FocusHandle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let header = self.mods_header(true, focus, window, cx);
        let mut page = div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(24.))
            .text_color(c(TEXT))
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
        chips = chips.child(
            div()
                .flex_none()
                .w(px(1.))
                .h(px(20.))
                .bg(c(crate::frame::LINE)),
        );
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
        let with_update = selected.iter().filter(|r| r.update.is_some()).count();
        let bulk = |id: &'static str, label: String, enabled: bool, danger: bool| {
            let b = crate::frame::button(id, label);
            let b = if danger { b.text_color(c(DANGER)) } else { b };
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
                    d.bg(c(TEXT))
                        .border_color(c(TEXT))
                        .child(div().w(px(10.)).h(px(2.)).bg(c(BG)))
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
                .border_color(c(SOFT))
                .child(select_all);
            if n_sel == 0 {
                bar = bar.child(muted(trf(
                    "{shown} of {total} mods shown",
                    &[
                        ("shown", &shown_ids.len().to_string()),
                        ("total", &rows.len().to_string()),
                    ],
                )));
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
                            this.ask_update(true);
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
            let panel = card().m(px(12.)).p(px(16.)).flex().flex_col().gap(px(10.)).border_color(c(GOLD));
            match confirm {
                Confirm::Remove(chosen) => {
                    let ids: HashSet<String> = chosen.iter().map(|(id, ..)| id.clone()).collect();
                    let index = sunrise::known_index();
                    let dependents = model::dependents(&rows, &ids, index.as_deref());
                    let names = chosen.iter().map(|(_, _, t)| t.as_str()).take(12).collect::<Vec<_>>().join(", ");
                    let more = chosen.len().saturating_sub(12);
                    let items = chosen.clone();
                    panel
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(trf("Remove {count} mods?", &[("count", &chosen.len().to_string())])))
                        .child(div().text_size(px(tokens::LABEL)).child(match more {
                            0 => names,
                            n => trf("{names} and {more} more", &[("names", &names), ("more", &n.to_string())]),
                        }))
                        .children(dependents.into_iter().map(|(who, needed)| {
                            div().text_size(px(tokens::LABEL)).text_color(c(GOLD)).child(crate::selectable::selectable(trf(
                                "{who} needs {needed}: it may stop working.",
                                &[("who", &who), ("needed", &needed.join(", "))],
                            )))
                        }))
                        .child(muted(tr("Their files are deleted. Worlds saved with them may lose what they added.")))
                        .child(
                            div()
                                .flex()
                                .gap(px(10.))
                                .child(crate::frame::button("alt-confirm-cancel", tr("Cancel")).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.ui.mods.confirm = None;
                                    cx.notify();
                                })))
                                .child(
                                    crate::frame::button("alt-confirm-remove", trf("Remove {count}", &[("count", &items.len().to_string())]))
                                        .text_color(c(DANGER))
                                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                            this.run(actions::Action::Remove(items.clone()), false);
                                            cx.notify();
                                        })),
                                ),
                        )
                }
                Confirm::Stake(action, worlds) => {
                    let action = action.clone();
                    let n = worlds.len();
                    panel
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(crate::selectable::selectable(trf(
                            "{count} of your worlds were saved with these mods",
                            &[("count", &n.to_string())],
                        ))))
                        .children(worlds.iter().map(|(world, mods)| {
                            div().text_size(px(tokens::LABEL)).child(trf("{world}: {mods}", &[("world", world), ("mods", &mods.join(", "))]))
                        }))
                        .child(div().text_size(px(tokens::LABEL)).text_color(c(GOLD)).child(crate::selectable::selectable(trf(
                            "{what} can permanently remove blocks, items and other content from these worlds. Nothing has been changed yet.",
                            &[("what", action.consequence())],
                        ))))
                        .child(muted(tr("Sunrise backs up each of these worlds first (a zip in backups/) and reads the backup back. If one can't be made or checked, nothing is changed.")))
                        .child(
                            div()
                                .flex()
                                .gap(px(10.))
                                .child(crate::frame::button("alt-confirm-cancel", tr("Cancel")).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.ui.mods.confirm = None;
                                    cx.notify();
                                })))
                                .child(primary("alt-confirm-backup", trf("Back up {count} worlds and continue", &[("count", &n.to_string())])).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.run(action.clone(), true);
                                    cx.notify();
                                }))),
                        )
                }
                Confirm::Update(plan) => {
                    let included = plan.included.clone();
                    let natives = included.iter().filter(|i| i.native).count();
                    panel
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(trf("Update {count} mods?", &[("count", &included.len().to_string())])))
                        .children(included.iter().take(14).map(|i| {
                            div().text_size(px(tokens::LABEL)).child(crate::selectable::selectable(trf(
                                "{title} {from} → {to}{native}",
                                &[
                                    ("title", &i.title),
                                    ("from", &i.from),
                                    ("to", &i.to),
                                    ("native", &if i.native { trf("  ({code})", &[("code", tr("native code"))]) } else { String::new() }),
                                ],
                            )))
                        }))
                        .children((included.len() > 14).then(|| muted(trf("and {count} more", &[("count", &(included.len() - 14).to_string())]))))
                        .children((!plan.skipped.is_empty()).then(|| {
                            div().flex().flex_col().gap(px(2.)).child(muted(tr("Left out:"))).children(plan.skipped.iter().map(|(t, why)| {
                                div().text_size(px(tokens::SMALL)).text_color(c(GOLD)).child(trf("{title}: {why}", &[("title", t), ("why", why)]))
                            }))
                        }))
                        .child(muted(tr("Each file is checked against the registry's size, hash and signature before it replaces anything. Mods that are off stay off.")))
                        .children((natives > 0).then(|| {
                            div().text_size(px(tokens::SMALL)).text_color(c(GOLD)).child(crate::selectable::selectable(trf(
                                "{count} of these run native code. A new build is made only when you Play or choose Build…, after the usual checks.",
                                &[("count", &natives.to_string())],
                            )))
                        }))
                        .child(
                            div()
                                .flex()
                                .gap(px(10.))
                                .child(crate::frame::button("alt-confirm-cancel", tr("Cancel")).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.ui.mods.confirm = None;
                                    cx.notify();
                                })))
                                .child({
                                    let b = primary("alt-confirm-update", trf("Update {count} mods", &[("count", &included.len().to_string())]));
                                    if included.is_empty() || busy {
                                        dim(b)
                                    } else {
                                        b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                            this.run(actions::Action::Update(included.clone()), false);
                                            cx.notify();
                                        }))
                                    }
                                }),
                        )
                }
            }
        });

        // ---- rows, in three groups
        let mut list = div().flex().flex_col();
        let mut last: Option<Group> = None;
        let mut position = 0;
        let group_counts = |g: Group| visible.iter().filter(|&&i| rows[i].group() == g).count();
        let updates_total = rows.iter().filter(|r| r.update.is_some()).count();
        for (n, &i) in visible.iter().enumerate() {
            let r = rows[i].clone();
            let g = r.group();
            if last != Some(g) {
                last = Some(g);
                let (title, count) = match g {
                    Group::NeedsYou => (tr("NEEDS YOU"), group_counts(g)),
                    Group::Updates => (tr("UPDATES"), group_counts(g)),
                    Group::Rest => (tr("EVERYTHING ELSE"), group_counts(g)),
                };
                let mut head = div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(14.))
                    .h(px(tokens::FIELD))
                    .bg(c(BG))
                    .child(
                        div()
                            .text_size(px(tokens::CAPTION))
                            .text_color(c(MUTED))
                            .child(crate::selectable::selectable(trf(
                                "{title} · {count}",
                                &[("title", title), ("count", &count.to_string())],
                            ))),
                    );
                if g == Group::Updates {
                    let b = crate::frame::button(
                        "alt-update-all",
                        trf(
                            "Update {count} mods",
                            &[("count", &updates_total.to_string())],
                        ),
                    );
                    head = head.child(if !busy {
                        b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.ask_update(false);
                            cx.notify();
                        }))
                    } else {
                        dim(b)
                    });
                }
                list = list.child(head);
            }
            let pos = position;
            if r.local.is_none() {
                position += 1;
            }
            list = list.child(self.row(n, pos, r, shown_ids.clone(), busy, cx));
        }
        if visible.is_empty() {
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
                div().px(px(14.)).py(px(8.)).border_t_1().border_color(c(SOFT)).child(muted(trf(
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
        let project = r.entry.as_ref().map(|e| e.project.clone());

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

        // What the row says on the right: one verdict, with its one action.
        // Roomy, the row is one line of columns; narrow, what it says moves under the name and
        // wraps there, so nothing is pushed out of the row.
        let wide = self.content_width() >= 860.;
        let say = move |text: String, color: u32| {
            div()
                .min_w_0()
                .text_size(px(tokens::LABEL))
                .text_color(c(color))
                .when(wide, |d| d.truncate())
                .child(text)
        };
        let mut notes: Vec<Div> = Vec::new();
        let act = |id: &'static str, label: &'static str, on: bool| {
            let b = crate::frame::button((id, n), tr(label));
            if on && !busy {
                b
            } else {
                dim(b)
            }
        };
        let mut verdict = div().flex().flex_none().items_center().gap(px(12.));
        if let Some(local) = &r.local {
            notes.push(say(
                match local.github {
                    true => tr("From GitHub, not the registry: read-only here").to_string(),
                    false => tr("A local file, not from the registry: read-only here").to_string(),
                },
                0x9a948a,
            ));
        }
        if let Some(why) = error {
            notes.push(say(why, GOLD));
        }
        if let Some(why) = &r.left_out {
            notes.push(say(why.clone(), GOLD));
        }
        match &r.need {
            Some(Need::TakenDown(why)) => {
                notes.push(say(trf("Taken down: {reason}", &[("reason", why)]), GOLD));
            }
            Some(Need::Build) => {
                let building = self.native_step.is_some();
                let b = act(
                    "alt-row-build",
                    if building { "Building…" } else { "Build…" },
                    true,
                );
                notes.push(say(tr("Not built for this computer yet").to_string(), GOLD));
                verdict = verdict.child(if !busy && !building {
                    b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.build();
                        cx.notify();
                    }))
                } else {
                    b
                });
            }
            Some(Need::Permission) => {
                let b = act("alt-row-allow", "Allow…", true);
                notes.push(say(
                    tr("Needs your permission to run native code").to_string(),
                    GOLD,
                ));
                verdict = verdict.child(if !busy {
                    b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.allow();
                        cx.notify();
                    }))
                } else {
                    b
                });
            }
            None => {}
        }
        if let Some(to) = &r.update {
            let b = act("alt-row-update", "Update", true);
            let row = r.clone();
            notes.push(say(trf("→ {to}", &[("to", to)]), BLUE));
            verdict = verdict.child(if !busy {
                b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.update_one(&row);
                    cx.notify();
                }))
            } else {
                b
            });
        }
        let sw = {
            let s = ui::switch(("alt-row-switch", n), r.on);
            if r.local.is_none() && !matches!(r.need, Some(Need::TakenDown(_))) {
                let row = r.clone();
                s.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.toggle(&row);
                    cx.notify();
                }))
            } else {
                s.opacity(0.45).cursor_default().tab_stop(false)
            }
        };
        let title = {
            let d = div()
                .id(("alt-row-title", n))
                .reach()
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
                        .text_color(c(MUTED))
                        .truncate()
                        .child(r.authors.clone()),
                );
            match project {
                Some(id) => {
                    d.cursor_pointer()
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.mods_show_project(id.clone());
                            cx.notify();
                        }))
                }
                None => d,
            }
        };
        let kind = r.make.map_or("", Make::label);
        let row = div()
            .id(("alt-row", n))
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(14.))
            .min_h(px(52.))
            .border_t_1()
            .border_color(c(SOFT))
            .when(selected, |d| d.bg(c(0x2a2724)))
            .child(match r.local {
                Some(_) => div().flex_none().w(px(18.)),
                None => div().flex_none().child(check),
            })
            .child(tile(picture, &r.title, 32.));
        let row = if wide {
            row.child(title.flex_1().min_w(px(140.)))
                .child(
                    div()
                        .w(px(64.))
                        .flex_none()
                        .text_size(px(tokens::LABEL))
                        .text_color(c(MUTED))
                        .child(kind),
                )
                .child(
                    div()
                        .w(px(64.))
                        .flex_none()
                        .text_size(px(tokens::LABEL))
                        .font_family(crate::theme::MONO)
                        .child(r.version.clone()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .flex_shrink(1.)
                        .min_w_0()
                        .children(notes),
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
                            .flex_wrap()
                            .items_baseline()
                            .gap_x(px(10.))
                            .child(muted(match kind {
                                "" => r.version.clone(),
                                kind => trf(
                                    "{kind} · {version}",
                                    &[("kind", kind), ("version", &r.version)],
                                ),
                            }))
                            .children(notes),
                    ),
            )
        };
        match r.unfit.clone() {
            Some(unfit) => row.child(verdict).child(self.unfit(n, &r, unfit, cx)),
            None => row.child(verdict).child(sw),
        }
    }

    /// Where a row's switch would be, for a mod the game was made without: "Not on Nov 17" and
    /// a "?" that says why in one sentence, the changed items for its author, and the way on.
    fn unfit(&mut self, n: usize, r: &Row, unfit: model::Unfit, cx: &mut Context<Self>) -> Div {
        use crate::frame::{button, CARD, LINE, TEXT};
        let open = self.ui.mods.unfit_open.as_deref() == Some(r.id.as_str());
        let id = r.id.clone();
        let help = div()
            .id(("alt-unfit-help", n))
            .tab_index(0)
            .seen()
            .flex_none()
            .size(px(16.))
            .rounded_full()
            .border_1()
            .border_color(c(if open { TEXT } else { LINE }))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(tokens::CAPTION))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(c(if open { TEXT } else { MUTED }))
            .cursor_pointer()
            .focus_visible(|s| s.border_color(c(GOLD)))
            .child("?")
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                let m = &mut this.ui.mods;
                m.unfit_open = match m.unfit_open.as_ref() == Some(&id) {
                    true => None,
                    false => Some(id.clone()),
                };
                m.unfit_details = false;
                cx.stop_propagation();
                cx.notify();
            }));
        let said = open.then(|| {
            let sentence = match (&unfit.made_for, unfit.changes.is_empty()) {
                (Some((_, made)), false) => trf(
                    "Made for {made}. {on} changed things it uses, so it waits for an update from its author. It stays installed.",
                    &[("made", made), ("on", &unfit.on)],
                ),
                (Some((_, made)), true) => trf(
                    "Made for {made}, so it waits for an update from its author. It stays installed.",
                    &[("made", made)],
                ),
                (None, false) => trf(
                    "Made for another build. {on} changed things it uses, so it waits for an update from its author. It stays installed.",
                    &[("on", &unfit.on)],
                ),
                (None, true) => tr(
                    "Made for another build, so it waits for an update from its author. It stays installed.",
                )
                .to_string(),
            };
            let details = self.ui.mods.unfit_details;
            let technical = (!unfit.changes.is_empty()).then(|| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .id("alt-unfit-details")
.reach()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(tokens::SMALL))
                            .text_color(c(MUTED))
                            .cursor_pointer()
                            .child(blocks::icon(
                                if details { "chevron-down" } else { "chevron-right" },
                                12.,
                                c(MUTED),
                            ))
                            .child(tr("Technical details"))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.ui.mods.unfit_details = !this.ui.mods.unfit_details;
                                cx.stop_propagation();
                                cx.notify();
                            })),
                    )
                    .children(details.then(|| {
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .children(unfit.changes.iter().map(|change| {
                                div()
                                    .text_size(px(tokens::SMALL))
                                    .text_color(c(MUTED))
                                    .child(format!("• {change}"))
                            }))
                    }))
            });
            // One way on: a version that fits when the registry has a newer one, else back to
            // the build it was made for when that build is still here.
            let action = match (&r.update, &unfit.made_for) {
                (Some(_), _) => {
                    let b = button(
                        "alt-unfit-update",
                        trf("Update {mod}", &[("mod", &r.title)]),
                    );
                    let row = r.clone();
                    Some(b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.ui.mods.unfit_open = None;
                        this.update_one(&row);
                        cx.stop_propagation();
                        cx.notify();
                    })))
                }
                (None, Some((tag, made))) if self.installed.contains(tag) => {
                    let b = button(
                        "alt-unfit-stay",
                        trf("Stay on {build}", &[("build", made)]),
                    );
                    let tag = tag.clone();
                    Some(b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.ui.mods.unfit_open = None;
                        this.stay_on(tag.clone());
                        cx.stop_propagation();
                        cx.notify();
                    })))
                }
                _ => None,
            };
            // Opens under the "?", or over it when the row is low in the window.
            let popup = div()
                    .w(px(360.))
                    .max_w(px(self.content_width() - 28.))
                    .p(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .rounded(px(tokens::R_CONTROL))
                    .border_1()
                    .border_color(c(LINE))
                    .bg(c(CARD))
                    .shadow_lg()
                    .occlude()
                    .child(div().text_size(px(tokens::LABEL)).child(sentence))
                    .children(technical)
                    .children(action.map(|a| div().flex().justify_end().child(a)));
            gpui::deferred(
                div().absolute().top(px(24.)).right_0().child(
                    gpui::anchored()
                        .anchor(gpui::Anchor::TopRight)
                        .snap_to_window_with_margin(px(8.))
                        .child(popup),
                ),
            )
            .with_priority(10)
        });
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .child(muted(trf("Not on {build}", &[("build", &unfit.on)])))
            .child(div().relative().flex_none().child(help).children(said))
    }

    /// "Stay on Nov 16" for a mod made for it: the setup stays on that build, where the mod
    /// runs. The next Play builds there as usual.
    fn stay_on(&mut self, tag: String) {
        let Some(id) = self.selected_instance().map(|i| i.id.clone()) else {
            return;
        };
        if !self.guard() {
            return;
        }
        match instances::edit(&id, |meta| meta.version = Some(tag)) {
            Ok(_) => self.reload_instances(),
            Err(e) => self.ui.mods.notice = Some(Err(e)),
        }
    }
}
