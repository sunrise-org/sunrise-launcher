//! Mods › Installed (mocks 05, 05b): what needs the player first, then updates, then the rest;
//! one switch per row; bulk actions that name their count and scope.
use super::model::{self, Group, Make, Row, Show};
use super::ui::*;
use super::*;
use crate::paint::palette;
use std::{collections::BTreeMap, rc::Rc};
use sunrise_launcher::browser::images;

/// Keep removal reviews on the same labels Installed paints, including hidden dependents.
pub(super) fn remove_names(rows: &[Row], labels: &[Option<String>]) -> BTreeMap<String, String> {
    rows.iter()
        .zip(labels)
        .filter(|(row, _)| row.selectable_archive())
        .map(|(row, label)| (row.id.clone(), label.as_ref().unwrap_or(&row.title).clone()))
        .collect()
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
        let refresh = match InstalledRefresh::of(self.ui.mods.snapshot.as_ref()) {
            InstalledRefresh::Ready => None,
            InstalledRefresh::Reading => Some(
                card()
                    .p(px(16.))
                    .child(muted(tr("Refreshing Installed mods…"))),
            ),
            InstalledRefresh::Failed(error, home) => {
                let (summary, action) = match error {
                    InstalledReadError::GameInstall(problem) => {
                        let home = home.to_owned();
                        let summary = problem.summary().to_owned();
                        let text = problem.details();
                        let action =
                            crate::frame::button("installed-recovery-details", tr("Show details"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_notice(
                                        text.clone(),
                                        Some(crate::sheets::WayOut::ProfileFolder(home.clone())),
                                    );
                                    cx.notify();
                                }));
                        (summary, Some(action))
                    }
                    InstalledReadError::Other(error) => (
                        trf(
                            "Could not refresh Installed mods: {error}",
                            &[("error", error)],
                        ),
                        None,
                    ),
                };
                Some(
                    card()
                        .p(px(16.))
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap(px(8.))
                        .child(muted(summary))
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(px(8.))
                                .children(action)
                                .child(
                                    crate::frame::button("retry-installed", tr("Retry")).on_click(
                                        cx.listener(|this, _, _, cx| {
                                            if let Some(snapshot) = &mut this.ui.mods.snapshot {
                                                snapshot.key.clear();
                                            }
                                            this.mods_refresh();
                                            cx.notify();
                                        }),
                                    ),
                                ),
                        ),
                )
            }
        };
        let refresh_ready = refresh.is_none();
        page = page.children(refresh);
        let (rows, labels) = self.labeled_installed_rows();
        if rows.is_empty() {
            if !refresh_ready {
                return page.into_any_element();
            }
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
        // Whether this page draws the Build button, and with it a running build's bar: the
        // status strip, drawn after the page, leaves its own bar out only then.
        let build = model::build_in_head(&rows, &visible);
        self.ui.mods.build_button_drawn = build.is_some();
        let shown_ids: Rc<Vec<String>> = Rc::new(
            visible
                .iter()
                .filter(|&&i| rows[i].selectable_archive())
                .map(|&i| rows[i].id.clone())
                .collect(),
        );
        let selected: Vec<&Row> = rows
            .iter()
            .filter(|r| r.selectable_archive() && self.ui.mods.selection.contains(&r.id))
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
        let registry_selected = selected.iter().filter(|row| row.local.is_none()).count();
        let busy = !refresh_ready || self.mods_busy();
        let with_update = selected.iter().filter(|r| model::bulk_updatable(r)).count();
        let any_update = rows.iter().any(model::bulk_updatable);
        let source_update = rows
            .iter()
            .any(|row| nightly::installed_source(row).is_some() && row.update.is_some());
        let has_tracked_sources = rows
            .iter()
            .filter_map(nightly::installed_source)
            .any(|origin| {
                origin.source.as_ref().is_some_and(|source| {
                    !matches!(
                        source.track,
                        sunrise_launcher::github_mods::SourceTrack::Pinned
                    )
                })
            });
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
                .min_h(px(48.))
                .when(n_sel > 0 || has_tracked_sources, |bar| {
                    bar.flex_wrap().py(px(7.)).gap_y(px(6.))
                })
                .border_b_1()
                .border_color(c(palette().divider))
                .child(select_all);
            if n_sel == 0 {
                bar = bar
                    .child(muted(model::visible_count(&rows, &visible)))
                    .child(div().flex_1());
                if has_tracked_sources {
                    let checking = self.source_updates_checking();
                    let check = bulk(
                        "nightly-check-source-updates",
                        if checking {
                            tr("Checking source updates…")
                        } else {
                            tr("Check for source updates")
                        }
                        .into(),
                        !checking,
                        false,
                    );
                    bar = bar.child(if checking || busy {
                        check
                    } else {
                        check.on_click(cx.listener(|this, _, _, cx| {
                            this.check_source_updates();
                            cx.notify();
                        }))
                    });
                }
                if any_update || source_update {
                    let update_all = bulk("alt-update-all", tr("Update all").into(), true, false);
                    bar = bar.child(if busy {
                        update_all
                    } else {
                        let names = remove_names(&rows, &labels);
                        update_all.on_click(cx.listener(move |this, _, _, cx| {
                            this.submit_mod_update_all(names.clone());
                            cx.notify();
                        }))
                    });
                }
            } else {
                let on = bulk(
                    "alt-bulk-on",
                    trf(
                        "Turn on {count}",
                        &[("count", &registry_selected.to_string())],
                    ),
                    registry_selected > 0,
                    false,
                );
                let off = bulk(
                    "alt-bulk-off",
                    trf(
                        "Turn off {count}",
                        &[("count", &registry_selected.to_string())],
                    ),
                    registry_selected > 0,
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
                )
                .whitespace_nowrap();
                let remove = if busy {
                    rm
                } else {
                    let names = remove_names(&rows, &labels);
                    let chosen: Vec<_> = selected.iter().map(|row| row.id.clone()).collect();
                    let home = self.mods_home();
                    rm.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.ask_remove(&home, chosen.clone(), names.clone());
                        cx.notify();
                    }))
                };
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
                    .child(if !busy && registry_selected > 0 {
                        on.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.bulk_on();
                            cx.notify();
                        }))
                    } else {
                        on
                    })
                    .child(if !busy && registry_selected > 0 {
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
                    .child(remove)
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

        // ---- rows, in three groups
        let mut list = div().flex().flex_col().children(
            self.source_update_notice()
                .map(|notice| note(notice.to_owned()).p(px(10.))),
        );
        let mut last: Option<Group> = None;
        let mut position = 0;
        let group_counts = |g: Group| visible.iter().filter(|&&i| rows[i].group() == g).count();
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
                    .min_h(px(tokens::FIELD))
                    .bg(c(palette().background))
                    .child(
                        div()
                            .text_size(px(tokens::CAPTION))
                            .text_color(c(palette().muted))
                            .child(crate::selectable::selectable(trf(
                                "{title} · {count}",
                                &[("title", title), ("count", &count.to_string())],
                            ))),
                    );
                if let Some(waiting) = build.filter(|_| g == Group::NeedsYou) {
                    // The button has as much room above and below as beside it.
                    head = head.py(px(14.)).child(self.build_button(waiting, busy, cx));
                }
                list = list.child(head);
            }
            let pos = position;
            if r.selectable_archive() {
                position += 1;
            }
            list = list.child(self.row(n, pos, r, labels[i].clone(), shown_ids.clone(), busy, cx));
        }
        if visible.is_empty() && refresh_ready {
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

    pub(super) fn labeled_installed_rows(&self) -> (Vec<Row>, Vec<Option<String>>) {
        let mut rows = self.rows();
        // Use the exact installed package's display name here, as in the Remove review.
        // Compute collisions before filtering so a row keeps its label when peers are hidden.
        let archive_names = self.ui.mods.snapshot.as_ref().map(|s| &s.archive_names);
        let archive_name = |r: &Row| {
            archive_names
                .filter(|_| r.selectable_archive())
                .and_then(|names| names.get(&r.id))
        };
        let mut renamed = false;
        for row in &mut rows {
            if let Some(version) =
                nightly::installed_source(row).and_then(|origin| self.source_update_version(origin))
            {
                row.update = Some(version.to_owned());
                renamed = true; // The new offer may move the row to the Updates group.
            }
            if let Some(archive) = archive_name(row).filter(|archive| row.title != archive.name) {
                row.title.clone_from(&archive.name);
                renamed = true;
            }
        }
        if renamed {
            model::sort(&mut rows);
        }
        use installed::{ModLabel, ModLabelSource};
        let labels = installed::colliding_mod_labels(rows.iter().map(|row| {
            let (authors, version) = archive_name(row).map_or_else(
                || (std::slice::from_ref(&row.authors), row.version.as_str()),
                |archive| (archive.authors.as_slice(), archive.version.as_str()),
            );
            let source = match &row.local {
                None => ModLabelSource::Registry,
                Some(local) if local.github.is_some() => ModLabelSource::Github,
                Some(_) => ModLabelSource::Local,
            };
            ModLabel {
                id: &row.id,
                name: &row.title,
                authors,
                version,
                source,
            }
        }));
        (rows, labels)
    }

    #[allow(clippy::too_many_arguments)]
    fn row(
        &mut self,
        n: usize,
        pos: usize,
        r: Row,
        label: Option<String>,
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

        // The list's shared columns (`model::cells`) keep their places across every group.
        // Narrow, the version starts under the name, then kind and status; the action and
        // switch stay at the right.
        let wide = self.content_width() >= 860.;
        use super::build_run::State;
        let building = self
            .build_here()
            .filter(|_| r.on)
            .and_then(|run| run.state_of(&r.id, &r.version));
        let failed = matches!(building, Some(State::Failed(_)));
        let cells = model::cells(
            &r,
            error.as_deref(),
            building,
            self.settings.native_mods_allowed_at.is_some(),
        );
        let tone = |tone: model::Tone| match tone {
            model::Tone::Warning => palette().warning,
            model::Tone::Info => palette().info,
            model::Tone::Accent => palette().accent_ink,
            model::Tone::Success => palette().success,
            model::Tone::Muted => palette().muted,
        };
        let install_hold = r
            .entry
            .as_ref()
            .and_then(|entry| entry.install_hold.as_ref())
            .filter(|_| !r.on);
        let hold_label = self
            .ui
            .mods
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.rows.switch_held.get(&r.id))
            .map(|hold| hold.label());
        let switch_held = hold_label.is_some();
        let status_cell: AnyElement = if switch_held {
            div().into_any_element()
        } else if let Some(hold) = install_hold {
            let detail = hold.detail();
            status("warning", hold.label(), palette().warning)
                .id(("alt-row-install-hold", n))
                .reach()
                .min_w_0()
                .cursor_pointer()
                .tooltip(crate::glass_ui::tip(tr("Show why")))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.open_notice(detail.clone(), None);
                    cx.stop_propagation();
                    cx.notify();
                }))
                .into_any_element()
        } else {
            match (&cells.status, r.unfit.clone()) {
                (Some(_), Some(unfit)) if cells.popover => {
                    self.unfit(n, &r, unfit, busy, cx).into_any_element()
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
            }
        };
        let action = cells.action.map(|action| {
            let (id, label) = match action {
                model::RowAction::Allow => ("alt-row-allow", tr("Allow…")),
                model::RowAction::Update => ("alt-row-update", tr("Update")),
            };
            let b = crate::frame::button((id, n), label);
            if busy {
                return dim(b).on_click(|_, _, cx| cx.stop_propagation());
            }
            let row = r.clone();
            b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                match action {
                    model::RowAction::Allow => this.allow(),
                    model::RowAction::Update if nightly::installed_source(&row).is_some() => {
                        this.submit_source_update(&row.id);
                    }
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
            if cells.switch && !switch_held && !busy {
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
        let title = {
            let plain_name = label.is_none();
            div()
                .min_w_0()
                .flex()
                .items_baseline()
                .gap(px(8.))
                .child(
                    div()
                        .min_w_0()
                        .font_weight(FontWeight::SEMIBOLD)
                        .truncate()
                        .child(label.unwrap_or_else(|| r.title.clone())),
                )
                .children(plain_name.then(|| {
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(tokens::SMALL))
                        .text_color(c(palette().muted))
                        .truncate()
                        .child(r.authors.clone())
                }))
        };
        let title = div()
            .flex()
            .flex_col()
            .min_w_0()
            .child(title)
            .children(
                r.local
                    .as_ref()
                    .and_then(|local| local.github.as_ref())
                    .map(|origin| {
                        let label = if origin.source.is_some() {
                            nightly::source_provenance(origin)
                        } else {
                            trf("GitHub: {source}", &[("source", &origin.label())])
                        };
                        div()
                            .id(("installed-origin", n))
                            .text_size(px(tokens::SMALL))
                            .text_color(c(palette().muted))
                            .truncate()
                            .tooltip(crate::glass_ui::tip(label.clone()))
                            .child(label)
                    }),
            )
            .children(
                nightly::installed_source(&r)
                    .and(r.update.as_ref())
                    .map(|version| {
                        muted(trf("Update available: {version}", &[("version", version)]))
                    }),
            )
            .children(hold_label.map(|label| {
                div()
                    .text_size(px(tokens::SMALL))
                    .text_color(c(palette().warning))
                    .child(label)
            }))
            .children(
                nightly::installed_source(&r)
                    .and_then(|_| {
                        self.source_outcome(&r.id)
                            .and_then(|result| result.as_ref().err())
                            .or_else(|| self.source_update_issue(&r.id))
                            .and_then(|error| nightly::source_error(error, None))
                    })
                    .map(|line| {
                        div()
                            .id(("installed-source-issue", n))
                            .text_color(c(palette().warning))
                            .text_size(px(tokens::SMALL))
                            .truncate()
                            .tooltip(crate::glass_ui::tip(line.clone()))
                            .child(line)
                    }),
            )
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
                .when(wide, |d| d.font_family(crate::theme::MONO))
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
            .child(if r.selectable_archive() {
                div().flex_none().child(check)
            } else {
                div().flex_none().w(px(18.))
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
                            .child(version(60.))
                            .child(kind(column::KIND))
                            .child(div().flex_1().min_w_0().flex().child(status_cell)),
                    ),
            )
            .children(action)
        };
        row.reach()
            .cursor_pointer()
            .hover(|s| s.bg(c(palette().hover)))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.mods_show_installed_project(&r);
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

    /// Where a row's switch would be, for a mod the game was made without: "Not on Nov 17" and
    /// a "?" that says why in one sentence, the changed items for its author, and the way on.
    pub(super) fn unfit(
        &mut self,
        n: usize,
        r: &Row,
        unfit: model::Unfit,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Div {
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
                            .child(tr("Details"))
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
                                    .font_family(crate::theme::MONO)
                                    .text_color(c(palette().muted))
                                    .child(change.clone())
                            }))
                    }))
            });
            // One way on: a version that fits when the registry has a newer one, else back to
            // the build it was made for when that build is still here.
            let action = match (busy, &r.update, &unfit.made_for) {
                (true, _, _) => None,
                (false, Some(_), _) => {
                    let b = button(
                        "alt-unfit-update",
                        trf("Update {mod}", &[("mod", &r.title)]),
                    );
                    let row = r.clone();
                    Some(b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.ui.mods.unfit_open = None;
                        if nightly::installed_source(&row).is_some() {
                            this.submit_source_update(&row.id);
                        } else {
                            this.update_one(&row);
                        }
                        cx.stop_propagation();
                        cx.notify();
                    })))
                }
                (false, None, Some((tag, made))) if self.installed.contains(tag) => {
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
                            if this.rows().iter().find(|row| row.id == id).and_then(nightly::installed_source).is_some() {
                                this.check_source_updates();
                            } else {
                                this.check_mod_update(&id);
                            }
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
