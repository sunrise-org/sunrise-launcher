//! Mods › Installed (mocks 05, 05b): what needs the player first, then updates, then the rest;
//! one switch per row; bulk actions that name their count and scope.
use super::model::{self, Group, Make, Row, Show};
use super::ui::*;
use super::*;
use crate::paint::palette;
use std::rc::Rc;
use sunrise_launcher::browser::images;

#[derive(Clone, Copy)]
enum Item {
    Group(Group, usize),
    Mod(usize),
}

struct Results {
    rows: Rc<Vec<Row>>,
    filter: model::RowFilter,
    items: Vec<Item>,
    selectable: Vec<String>,
    shown: usize,
    builds: Option<usize>,
    updates: usize,
    on: usize,
    native: usize,
    content: usize,
}

impl Results {
    fn new(rows: Rc<Vec<Row>>, filter: model::RowFilter) -> Self {
        let visible = model::visible(&rows, &filter);
        let builds = model::build_in_head(&rows, &visible);
        let mut items = Vec::new();
        let mut selectable = Vec::new();
        for group in [Group::NeedsYou, Group::Updates, Group::Rest] {
            let count = visible
                .iter()
                .filter(|&&n| rows[n].group() == group)
                .count();
            if count == 0 {
                continue;
            }
            items.push(Item::Group(group, count));
            for &n in visible.iter().filter(|&&n| rows[n].group() == group) {
                items.push(Item::Mod(n));
                if rows[n].local.is_none() {
                    selectable.push(rows[n].id.clone());
                }
            }
        }
        Self {
            shown: visible.len(),
            updates: rows.iter().filter(|r| r.update.is_some()).count(),
            on: rows.iter().filter(|r| r.on).count(),
            native: rows.iter().filter(|r| r.make == Some(Make::Native)).count(),
            content: rows
                .iter()
                .filter(|r| r.make == Some(Make::Content))
                .count(),
            rows,
            filter,
            items,
            selectable,
            builds,
        }
    }

    fn row(&self, at: usize) -> Option<&Row> {
        let Item::Mod(n) = *self.items.get(at)? else {
            return None;
        };
        Some(&self.rows[n])
    }
}

#[derive(Default)]
pub(super) struct InstalledView {
    results: Option<Rc<Results>>,
    scroll: gpui::UniformListScrollHandle,
    focus: HashMap<String, FocusHandle>,
}

fn select_mod(mods: &mut ModsState, selectable: &[String], id: &str, range: bool) {
    let anchor = range.then_some(mods.anchor.as_ref()).flatten();
    let a = anchor.and_then(|a| selectable.iter().position(|id| id == a));
    let b = a.and_then(|_| selectable.iter().position(|current| current == id));
    match (a, b) {
        (Some(a), Some(b)) => mods
            .selection
            .extend(selectable[a.min(b)..=a.max(b)].iter().cloned()),
        _ => {
            if !mods.selection.remove(id) {
                mods.selection.insert(id.to_owned());
            }
        }
    }
    mods.anchor = Some(id.to_owned());
}

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
            .text_color(c(palette().text))
            .child(header)
            .children(self.strip(cx));

        self.ui.mods.build_button_drawn = false;
        let rows = self.installed_rows();
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

        let view = &mut self.ui.mods.installed_view;
        let filter = &self.ui.mods.row_filter;
        if view
            .results
            .as_ref()
            .is_none_or(|old| !Rc::ptr_eq(&old.rows, &rows) || &old.filter != filter)
        {
            if view
                .results
                .as_ref()
                .is_some_and(|old| &old.filter != filter)
            {
                view.scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
            }
            view.results = Some(Rc::new(Results::new(rows.clone(), filter.clone())));
        }
        let results = view.results.as_ref().expect("computed above").clone();
        let filter = &results.filter;
        self.ui.mods.build_button_drawn = results.builds.is_some();
        let selected: Vec<&Row> = if self.ui.mods.selection.is_empty() {
            Vec::new()
        } else {
            rows.iter()
                .filter(|r| r.local.is_none() && self.ui.mods.selection.contains(&r.id))
                .collect()
        };

        // ---- what to show: the same row of chips Discover filters with
        let counted = |label: &str, n: usize| {
            trf("{label} · {n}", &[("label", label), ("n", &n.to_string())])
        };
        let mut chips = div().flex().flex_wrap().items_center().gap(px(8.));
        let shown = [
            (tr("All"), rows.len(), Show::All),
            (tr("Turned on"), results.on, Show::On),
            (tr("Turned off"), rows.len() - results.on, Show::Off),
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
        for (i, (make, n)) in [
            (Make::Native, results.native),
            (Make::Content, results.content),
        ]
        .into_iter()
        .enumerate()
        {
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
        let selected_here = results
            .selectable
            .iter()
            .filter(|id| self.ui.mods.selection.contains(*id))
            .count();
        let all_on = selected_here > 0 && selected_here == results.selectable.len();
        let n_sel = selected.len();
        let busy = self.mods_busy();
        let with_update = selected.iter().filter(|r| r.update.is_some()).count();
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
            let scope = results.clone();
            let select_all = checkbox("alt-select-all", all_on)
                .when(selected_here > 0 && !all_on, |d| {
                    d.bg(c(palette().text))
                        .border_color(c(palette().text))
                        .child(div().w(px(10.)).h(px(2.)).bg(c(palette().background)))
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let m = &mut this.ui.mods;
                    if scope.selectable.iter().all(|id| m.selection.contains(id)) {
                        for id in &scope.selectable {
                            m.selection.remove(id);
                        }
                    } else {
                        m.selection.extend(scope.selectable.iter().cloned());
                    }
                    m.anchor = None;
                    cx.notify();
                }));
            let mut bar = div()
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(14.))
                .min_h(px(40.))
                .py(px(2.))
                .child(select_all);
            if n_sel == 0 {
                bar = bar.child(muted(trf(
                    "{shown} of {total} mods shown",
                    &[
                        ("shown", &results.shown.to_string()),
                        ("total", &rows.len().to_string()),
                    ],
                )));
            } else {
                let narrow = self.content_width() < 860.;
                let on = bulk("alt-bulk-on", tr("Turn on").into(), true, false);
                let off = bulk("alt-bulk-off", tr("Turn off").into(), true, false);
                let upd = bulk(
                    "alt-bulk-update",
                    tr("Update").into(),
                    with_update > 0,
                    false,
                );
                let rm = bulk("alt-bulk-remove", tr("Remove…").into(), true, true);
                bar = bar
                    .child(muted(trf(
                        "{count} selected",
                        &[("count", &n_sel.to_string())],
                    )))
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
                let actions = div()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.))
                    .items_center()
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
                    });
                bar = if narrow {
                    div()
                        .flex()
                        .flex_col()
                        .child(bar)
                        .child(actions.px(px(14.)).pb(px(8.)))
                } else {
                    bar.child(actions)
                };
            }
            let current_actions = div().flex().flex_wrap().gap(px(6.)).items_center();
            let build = results.builds.map(|n| self.build_button(n, busy, cx));
            let update = (results.updates > 0).then(|| {
                let b = crate::frame::button(
                    "alt-update-all",
                    trf(
                        "Update {count} mods",
                        &[("count", &results.updates.to_string())],
                    ),
                );
                if busy {
                    dim(b)
                } else {
                    b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.ask_update(false);
                        cx.notify();
                    }))
                }
            });
            div()
                .flex()
                .flex_col()
                .flex_none()
                .border_b_1()
                .border_color(c(palette().divider))
                .child(bar)
                .children((build.is_some() || update.is_some()).then(|| {
                    current_actions
                        .px(px(14.))
                        .pb(px(8.))
                        .children(build)
                        .children(update)
                }))
        };

        let scroll = self.ui.mods.installed_view.scroll.clone();
        crate::frame::scrolls_with(&scroll.0.borrow().base_handle);
        if !self.ui.mods.revealed.get() {
            if let Some(search::Reveal::Mod(id)) = &self.ui.mods.reveal {
                if let Some(at) = (0..results.items.len())
                    .find(|&at| results.row(at).is_some_and(|r| &r.id == id))
                {
                    scroll.scroll_to_item(at, gpui::ScrollStrategy::Nearest);
                    self.ui.mods.revealed.set(true);
                }
            }
        }
        let height = if self.content_width() >= 860. {
            56.
        } else {
            72.
        };
        let displayed = results.clone();
        let list = gpui::uniform_list(
            "alt-installed-scroll",
            results.items.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|at| match displayed.items[at] {
                        Item::Group(group, count) => {
                            let title = match group {
                                Group::NeedsYou => tr("NEEDS YOU"),
                                Group::Updates => tr("UPDATES"),
                                Group::Rest => tr("EVERYTHING ELSE"),
                            };
                            div()
                                .id(("installed-group", at))
                                .w_full()
                                .h(px(height))
                                .px(px(14.))
                                .flex()
                                .items_center()
                                .bg(c(palette().background))
                                .text_size(px(tokens::CAPTION))
                                .text_color(c(palette().muted))
                                .child(trf(
                                    "{title} · {count}",
                                    &[("title", title), ("count", &count.to_string())],
                                ))
                        }
                        Item::Mod(n) => this
                            .row(n, displayed.clone(), busy, cx)
                            .w_full()
                            .h(px(height)),
                    })
                    .collect()
            }),
        )
        .track_scroll(&scroll)
        .flex_1()
        .min_h_0();
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
            .when(results.items.is_empty(), |d| d.child(div().p(px(16.)).child(muted(tr("No mods match this search or filter.")))))
            .when(!results.items.is_empty(), |d| d.child(list))
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
                let views = &mut this.ui.mods.installed_view;
                let at = (0..results.items.len()).find(|&at| results.row(at).is_some_and(|row| {
                    views.focus.get(&row.id).is_some_and(|h| h.contains_focused(window, cx))
                }));
                let Some(at) = at else { return; };
                let next = match ev.keystroke.key.as_str() {
                    "up" => (0..at).rev().find(|&n| results.row(n).is_some()),
                    "down" => (at + 1..results.items.len()).find(|&n| results.row(n).is_some()),
                    "home" => (0..results.items.len()).find(|&n| results.row(n).is_some()),
                    "end" => (0..results.items.len()).rev().find(|&n| results.row(n).is_some()),
                    "space" => {
                        let row = results.row(at).expect("focused row");
                        if row.local.is_none() && views.focus.get(&row.id).is_some_and(|h| h.is_focused(window)) {
                            select_mod(&mut this.ui.mods, &results.selectable, &row.id, ev.keystroke.modifiers.shift);
                            cx.stop_propagation();
                            cx.notify();
                        }
                        return;
                    }
                    _ => return,
                };
                if let Some(next) = next {
                    let row = results.row(next).expect("selecting only rows");
                    views.scroll.scroll_to_item(next, gpui::ScrollStrategy::Nearest);
                    let focus = views.focus.entry(row.id.clone()).or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true)).clone();
                    window.focus(&focus, cx);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .children((hidden > 0).then(|| {
                div().px(px(14.)).py(px(8.)).border_t_1().border_color(c(palette().divider)).child(muted(trf(
                    "{count} other files the launcher installed (packs, or from other sources) aren't listed here.",
                    &[("count", &hidden.to_string())],
                )))
            }));
        page = page.child(right);
        page.into_any_element()
    }

    fn row(
        &mut self,
        n: usize,
        results: Rc<Results>,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let r = &results.rows[n];
        let focus = self
            .ui
            .mods
            .installed_view
            .focus
            .entry(r.id.clone())
            .or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone();
        let selected = self.ui.mods.selection.contains(&r.id);
        let picture = r
            .icon
            .clone()
            .and_then(|u| self.mods_image(&u, images::ROW));
        let error = self.ui.mods.errors.get(&r.id).cloned();
        let project = r.entry.as_ref().map(|e| e.project.clone());

        let check = {
            let id = r.id.clone();
            let results = results.clone();
            checkbox(("alt-row-check", n), selected).on_click(cx.listener(
                move |this, ev: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    select_mod(
                        &mut this.ui.mods,
                        &results.selectable,
                        &id,
                        ev.modifiers().shift,
                    );
                    cx.notify();
                },
            ))
        };

        // The list's shared columns (`model::cells`): kind, version, status, the one action
        // and the switch, each in its own fixed place on every row of every group. Narrow,
        // kind and version move under the name with the status after them, in fixed places
        // too; the action and the switch keep theirs.
        let wide = self.content_width() >= 860.;
        use super::build_run::State;
        let building = self
            .build_here()
            .filter(|_| r.on)
            .and_then(|run| run.state_of(&r.id, &r.version));
        let failed = matches!(building, Some(State::Failed(_)));
        let cells = model::cells(r, error.as_deref(), building);
        let tone = |tone: model::Tone| match tone {
            model::Tone::Warning => palette().warning,
            model::Tone::Info => palette().info,
            model::Tone::Accent => palette().accent_ink,
            model::Tone::Success => palette().success,
            model::Tone::Muted => palette().muted,
        };
        let status_cell: AnyElement = match (&cells.status, r.unfit.clone()) {
            (Some(_), Some(unfit)) if cells.popover => {
                self.unfit(n, r, unfit, cx).into_any_element()
            }
            // A mod the last build could not build: its status opens that build's window
            // again, where Details and the build log are.
            (Some(said), _) if failed && said.label == tr("Failed") => {
                status(said.icon, said.label.clone(), tone(said.tone))
                    .id(("alt-row-failed", n))
                    .reach()
                    .min_w_0()
                    .cursor_pointer()
                    .hover(|s| s.text_color(c(palette().text)))
                    .tooltip(crate::glass_ui::tip(said.tip.clone()))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.open_sheet(crate::sheets::Sheet::ModBuild);
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    .into_any_element()
            }
            (Some(said), _) => status(said.icon, said.label.clone(), tone(said.tone))
                .id(("alt-row-status", n))
                .min_w_0()
                .tooltip(crate::glass_ui::tip(said.tip.clone()))
                .into_any_element(),
            (None, _) => div().into_any_element(),
        };
        let action = cells.action.map(|action| {
            let (id, label) = match action {
                model::RowAction::Allow => ("alt-row-allow", tr("Allow…")),
                model::RowAction::Update => ("alt-row-update", tr("Update")),
            };
            let b = crate::frame::button((id, n), label);
            if busy {
                return dim(b);
            }
            let row = r.clone();
            b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                match action {
                    model::RowAction::Allow => this.allow(),
                    model::RowAction::Update => this.update_one(&row),
                }
                cx.notify();
            }))
        });
        let sw = {
            let s = ui::switch(("alt-row-switch", n), r.on).when(cells.unavailable, |s| {
                s.opacity(0.52).tooltip(crate::glass_ui::tip(tr(
                    "Enabled for this profile, but not included in the build.",
                )))
            });
            if cells.switch {
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
                .tooltip(crate::glass_ui::tip(format!("{} · {}", r.title, r.id)))
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
                .id(("row-version", n))
                .tooltip(crate::glass_ui::tip(cells.version.clone()))
                .w(px(width))
                .flex_none()
                .truncate()
                .text_size(px(tokens::LABEL))
                .font_family(crate::theme::MONO)
                .child(cells.version.clone())
        };
        let row =
            div()
                .id(SharedString::from(format!("installed-row-{}", r.id)))
                .track_focus(&focus)
                .focus_visible(|d| d.shadow(crate::frame::focus_ring(palette().focus)))
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
                    self.ui.mods.reveal.as_ref().is_some_and(
                        |reveal| matches!(reveal, search::Reveal::Mod(id) if id == &r.id),
                    ),
                    |d| d.bg(c(palette().hover)),
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
        row.child(
            div()
                .w(px(column::SWITCH))
                .flex_none()
                .flex()
                .justify_end()
                .child(sw),
        )
    }

    pub(crate) fn dismiss_mod_batch(&mut self) {
        self.ui.mods.confirm = None;
        self.ui.mods.reviewed_home = None;
    }

    /// Render the same reviewed selection in a focus-trapped sheet, not above the row list.
    pub(crate) fn mod_batch_sheet(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        use crate::frame::button;
        let rows = self.rows();
        let busy = self.mods_busy();
        let confirm = self.ui.mods.confirm.as_ref()?;
        let heading = |label, n: usize| {
            muted(trf(
                "{label} · {count}",
                &[("label", label), ("count", &n.to_string())],
            ))
            .flex_none()
            .px(px(12.))
            .py(px(6.))
        };
        let line = |n, name: &str, id: &str, detail: &str, full: &str| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .min_h(px(32.))
                .px(px(12.))
                .gap(px(12.))
                .border_t_1()
                .border_color(c(palette().divider))
                .text_size(px(tokens::LABEL))
                .child(
                    div()
                        .id(("batch-name", n))
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .tooltip(crate::glass_ui::tip(format!("{name} · {id}")))
                        .child(name.to_owned()),
                )
                .children((!detail.is_empty()).then(|| {
                    div()
                        .id(("batch-version", n))
                        .max_w(px(190.))
                        .truncate()
                        .text_color(c(palette().muted))
                        .tooltip(crate::glass_ui::tip(full.to_owned()))
                        .child(detail.to_owned())
                }))
        };
        let mut inventory = card().flex_none();
        let (title, warning, native, go) = match confirm {
            Confirm::Remove(chosen) => {
                let count = chosen.len();
                let ids: HashSet<String> = chosen.iter().map(|(id, ..)| id.clone()).collect();
                let index = sunrise::fetched_index();
                let dependents = model::dependents(&rows, &ids, index.as_deref());
                inventory = inventory.child(heading(tr("Selected"), count)).children(
                    chosen
                        .iter()
                        .enumerate()
                        .map(|(n, (id, _, name))| line(n, name, id, "", "")),
                );
                if !dependents.is_empty() {
                    inventory = inventory
                        .child(heading(tr("May stop working"), dependents.len()))
                        .children(dependents.iter().enumerate().map(|(n, (name, needed))| {
                            let mut short = needed
                                .iter()
                                .take(2)
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(", ");
                            if needed.len() > 2 {
                                short = trf(
                                    "{names} and {count} more",
                                    &[
                                        ("names", &short),
                                        ("count", &(needed.len() - 2).to_string()),
                                    ],
                                );
                            }
                            line(count + n, name, name, &short, &needed.join(", "))
                        }));
                }
                let items = chosen.clone();
                let go = button(
                    "alt-confirm-remove",
                    trf("Remove {count}", &[("count", &count.to_string())]),
                )
                .text_color(c(palette().danger));
                let go = if busy {
                    dim(go)
                } else {
                    go.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.run(actions::Action::Remove(items.clone()));
                        this.close_sheet();
                        cx.notify();
                    }))
                };
                let title = if count == 1 {
                    tr("Remove 1 mod?").into()
                } else {
                    trf("Remove {count} mods?", &[("count", &count.to_string())])
                };
                (title, tr("Their files are deleted. Worlds saved with them may permanently lose what they added."), None, go)
            }
            Confirm::Update(plan) => {
                let included = plan.included.clone();
                let count = included.len();
                let natives = included.iter().filter(|i| i.native).count();
                inventory = inventory.child(heading(tr("Included"), count)).children(
                    included.iter().enumerate().map(|(n, i)| {
                        let version = trf("{from} → {to}", &[("from", &i.from), ("to", &i.to)]);
                        line(n, &i.title, &i.id, &version, &version)
                    }),
                );
                if !plan.skipped.is_empty() {
                    let mut groups = std::collections::BTreeMap::<&str, Vec<&str>>::new();
                    for (title, why) in &plan.skipped {
                        groups.entry(why).or_default().push(title);
                    }
                    inventory = inventory.child(heading(tr("Left out"), plan.skipped.len()));
                    let mut n = count;
                    for (why, names) in groups {
                        inventory = inventory
                            .child(muted(why.to_owned()).flex_none().px(px(12.)).py(px(4.)));
                        for name in names {
                            inventory = inventory.child(line(n, name, name, "", ""));
                            n += 1;
                        }
                    }
                }
                let native = match natives {
                    0 => None,
                    1 => Some(
                        tr("1 mod runs native code. The next build runs the usual checks.").into(),
                    ),
                    n => Some(trf(
                        "{count} mods run native code. The next build runs the usual checks.",
                        &[("count", &n.to_string())],
                    )),
                };
                let label = if count == 1 {
                    tr("Update 1 mod").into()
                } else {
                    trf("Update {count} mods", &[("count", &count.to_string())])
                };
                let go = primary("alt-confirm-update", label);
                let go = if busy || included.is_empty() {
                    dim(go)
                } else {
                    go.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.run(actions::Action::Update(included.clone()));
                        this.close_sheet();
                        cx.notify();
                    }))
                };
                let title = if count == 1 {
                    tr("Update 1 mod?").into()
                } else {
                    trf("Update {count} mods?", &[("count", &count.to_string())])
                };
                (title, tr("Each file is checked against the registry's size, hash and signature before it replaces anything. Mods that are off stay off."), native, go)
            }
        };
        let max = (self.window_height * 0.4 - 64.).clamp(176., 256.);
        let body = div()
            .flex()
            .flex_col()
            .child(crate::sheets::title(title).flex_none())
            .child(
                div()
                    .id("batch-inventory")
                    .min_h_0()
                    .max_h(px(max))
                    .overflow_y_scroll()
                    .track_scroll(&crate::frame::scrolls("batch-inventory"))
                    .child(inventory),
            )
            .child(muted(warning).flex_none())
            .children(native.map(|text| muted(text).flex_none()))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .flex_wrap()
                    .justify_end()
                    .gap(px(10.))
                    .child(
                        button("alt-confirm-cancel", tr("Cancel")).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.close_sheet();
                                cx.notify();
                            },
                        )),
                    )
                    .child(go),
            );
        Some(crate::sheets::bounded_frame(
            "batch-confirm",
            580.,
            body,
            cx,
        ))
    }

    /// a "?" that says why in one sentence, the changed items for its author, and the way on.
    fn unfit(&mut self, n: usize, r: &Row, unfit: model::Unfit, cx: &mut Context<Self>) -> Div {
        use crate::frame::button;
        let open = self.ui.mods.unfit_open.as_deref() == Some(r.id.as_str());
        let id = r.id.clone();
        let help = status(
            "information-circle",
            tr("Needs an update"),
            palette().warning,
        )
        .id(("alt-unfit-help", n))
        .reach()
        // Its glyph stands where every other row's does.
        .px(px(8.))
        .ml(px(-8.))
        .h(px(28.))
        .when(open, |d| d.bg(c(palette().hover)))
        .hover(|s| s.bg(c(palette().hover)))
        .cursor_pointer()
        .tooltip(crate::glass_ui::tip(trf(
            "{mod} needs an update for {build}. It comes back when its author releases one.",
            &[("mod", &r.title), ("build", &unfit.on)],
        )))
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
            let sentence = match &unfit.made_for {
                Some((_, made)) => trf(
                    "{mod} needs an update for {on}. It was made for {made}, and comes back when its author releases a new version. It stays installed.",
                    &[("mod", &r.title), ("on", &unfit.on), ("made", made)],
                ),
                None => trf(
                    "{mod} needs an update for {on}. It comes back when its author releases a new version. It stays installed.",
                    &[("mod", &r.title), ("on", &unfit.on)],
                ),
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
                            .text_color(c(palette().muted))
                            .cursor_pointer()
                            .child(blocks::icon(
                                if details { "chevron-down" } else { "chevron-right" },
                                12.,
                                c(palette().muted),
                            ))
                            .child(trf("Details · {count} changes", &[("count", &unfit.changes.len().to_string())]))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.ui.mods.unfit_details = !this.ui.mods.unfit_details;
                                cx.stop_propagation();
                                cx.notify();
                            })),
                    )
                    .children(details.then(|| {
                        div().id("unfit-changes").min_h_0().max_h(px(176.))
                            .overflow_y_scroll().track_scroll(&crate::frame::scrolls("unfit-changes"))
                            .child(div().flex().flex_col().gap(px(4.)).text_size(px(tokens::SMALL))
                                .font_family(crate::theme::MONO).text_color(c(palette().muted))
                                .children(unfit.changes.iter().cloned().map(crate::selectable::selectable)))
                    }))
            });
            // One way on: a version that fits when the registry has a newer one, else back to
            // the build it was made for when that build is still here.
            let action = match (&r.update, &unfit.made_for) {
                (Some(_), _) => {
                    let b = button("alt-unfit-update", tr("Update mod")).tooltip(crate::glass_ui::tip(r.title.clone()));
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
                _ => {
                    let id = r.id.clone();
                    Some(button("alt-unfit-check", tr("Check for update")).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.check_mod_update(&id);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ))
                }
            };
            // Opens under the "?", or over it when the row is low in the window.
            let popup = div()
                    .w(px(360.))
                    .max_w(px(self.content_width() - 28.))
                    .p(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                    .border_1()
                    .border_color(c(palette().line))
                    .bg(c(palette().raised))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shifted_selection_uses_the_current_order_and_stable_anchor() {
        let mut mods = ModsState::default();
        let before = ["first", "moved", "last"].map(String::from);
        select_mod(&mut mods, &before, "moved", false);
        let after = ["moved", "first", "last"].map(String::from);
        select_mod(&mut mods, &after, "last", true);
        assert_eq!(mods.selection, after.into_iter().collect());
        let filtered = ["moved".to_string()];
        select_mod(&mut mods, &filtered, "moved", true);
        assert!(
            !mods.selection.contains("moved"),
            "an anchor filtered away is not an index"
        );
    }
}
