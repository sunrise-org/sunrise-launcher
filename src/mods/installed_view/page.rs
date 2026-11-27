//! The Installed page itself: filters, sections and the rows they hold.

use super::items::{installed_items_key, InstalledItem};
use super::*;
use crate::widgets::button::{primary, secondary};
use crate::widgets::check::{checkbox, Tick};
use crate::widgets::choice::choices;
use crate::widgets::chip;
use crate::widgets::feedback::empty_state;
use gpui::SharedString;

impl Launcher {
    pub(in crate::mods) fn archive_display(&self, row: &Row) -> Option<&ArchiveDisplay> {
        self.mods_state
            .snapshot
            .as_ref()?
            .archives
            .get(row.archive_file()?)
    }

    pub(in crate::mods) fn installed(
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

        let refresh = match InstalledRefresh::of(self.mods_state.snapshot.as_ref()) {
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
                        let action = secondary("installed-recovery-details", tr("Show details"))
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
                                .child(secondary("retry-installed", tr("Retry")).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        if let Some(snapshot) = &mut this.mods_state.snapshot {
                                            snapshot.key.clear();
                                        }
                                        this.mods_refresh();
                                        cx.notify();
                                    }),
                                )),
                        ),
                )
            }
        };
        let refresh_ready = refresh.is_none();
        page = page.children(refresh);
        let (rows, labels) = self.labeled_installed_rows();
        let (reading, failed) = match InstalledRefresh::of(self.mods_state.snapshot.as_ref()) {
            InstalledRefresh::Ready => (false, false),
            InstalledRefresh::Reading => (true, false),
            InstalledRefresh::Failed(..) => (false, true),
        };
        self.mods_state.drawn = Some(InstalledDrawn::of(reading, failed, rows.len()));
        if rows.is_empty() && !refresh_ready {
            return page.into_any_element();
        }
        let model::FilterCounts { shows, kinds } = self.mods_state.installed_filters(&rows);
        if rows.is_empty() {
            return page
                .child(empty_state(
                    tr("No mods yet"),
                    tr("Mods you add to this profile show up here."),
                    Some(
                        primary("alt-installed-browse", tr("Find mods"))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.choose_view(Page::Discover);
                                cx.notify();
                            }))
                            .into_any_element(),
                    ),
                ))
                .into_any_element();
        }

        let visible = model::visible(&rows, &self.mods_state.row_filter);
        let shown: Rc<Vec<_>> = Rc::new(
            visible
                .iter()
                .filter_map(|&i| {
                    let row = &rows[i];
                    let file = row.archive_file().filter(|_| row.selectable_archive())?;
                    Some((file.to_owned(), row.id.clone()))
                })
                .collect(),
        );
        let filter = self.mods_state.row_filter.clone();
        let selected: Vec<&Row> = rows
            .iter()
            .filter(|r| r.selectable_archive() && self.mods_state.selection.contains(&r.id))
            .collect();

        // ---- what to show: the same row of chips Discover filters with
        let counted = |label: &str, n: usize| {
            trf("{label} · {n}", &[("label", label), ("n", &n.to_string())])
        };
        let mut chips = div().flex().flex_wrap().items_center().gap(px(8.));
        let show_options: Vec<(Show, SharedString)> = shows
            .into_iter()
            .map(|(which, n)| {
                let label = match which {
                    Show::All => tr("All"),
                    Show::On => tr("Turned on"),
                    Show::Off => tr("Turned off"),
                };
                (which, counted(label, n).into())
            })
            .collect();
        let this = cx.entity();
        let show_choice = choices(
            "alt-show",
            &show_options,
            filter.show,
            move |which, _, cx| {
                this.update(cx, |this, cx| {
                    let m = &mut this.mods_state;
                    m.row_filter.show = which;
                    m.selection.clear();
                    m.anchor = None;
                    cx.notify();
                })
            },
        );
        chips = chips.child(show_choice);
        if !kinds.is_empty() {
            chips = chips.child(div().flex_none().w(px(1.)).h(px(20.)).bg(c(palette().line)));
        }
        for (make, n) in kinds {
            let i = make as usize;
            chips = chips.child(
                // Two switches, not one choice: each is its own Tab stop.
                chip::chip(
                    format!("alt-kind-{i}"),
                    counted(make.label(), n),
                    if filter.makes.contains(&make) {
                        chip::Kind::Selected
                    } else {
                        chip::Kind::Neutral
                    },
                )
                .tab_index(0)
                .seen()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let m = &mut this.mods_state;
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
        let selected_here = shown
            .iter()
            .filter(|(_, id)| self.mods_state.selection.contains(id))
            .count();
        let all_on = selected_here > 0 && selected_here == shown.len();
        let n_sel = selected.len();
        let registry_selected = selected.iter().filter(|row| row.local.is_none()).count();
        let busy = !refresh_ready || self.mods_busy();
        let with_update = selected.iter().filter(|r| model::bulk_updatable(r)).count();
        let registry_updates = rows.iter().filter(|r| model::bulk_updatable(r)).count();
        let has_manual_mods = rows.iter().any(|r| r.manual_badge().is_some());
        let source_update = self.nightly_catalogue_current()
            && rows
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
            let b = secondary(id, label);
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
            let all_shown = shown.clone();
            let select_all = checkbox("alt-select-all", if all_on { Tick::On } else { Tick::Off })
                .when(selected_here > 0 && !all_on, |d| {
                    d.bg(c(palette().text))
                        .border_color(c(palette().text))
                        .child(div().w(px(10.)).h(px(2.)).bg(c(palette().background)))
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let m = &mut this.mods_state;
                    match all_shown.iter().all(|(_, id)| m.selection.contains(id)) {
                        true => m.selection.clear(),
                        false => m
                            .selection
                            .extend(all_shown.iter().map(|(_, id)| id.clone())),
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
                .when(n_sel > 0 || has_tracked_sources || has_manual_mods, |bar| {
                    bar.flex_wrap().py(px(7.)).gap_y(px(6.))
                })
                .border_b_1()
                .border_color(palette().rule())
                .child(select_all);
            if n_sel == 0 {
                bar = bar
                    .child(muted(model::visible_count(&rows, &visible)))
                    .child(div().flex_1());
                if has_manual_mods {
                    bar = bar.child(muted(tr(
                        "From source and Local mods refresh only via Add from source",
                    )));
                }
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
                            this.check_source_updates(None);
                            cx.notify();
                        }))
                    });
                }
                if registry_updates > 0 || source_update {
                    let label = if registry_updates > 0 && !source_update {
                        trf(
                            "Update all ({count})",
                            &[("count", &registry_updates.to_string())],
                        )
                    } else {
                        tr("Update all").into()
                    };
                    let update_all = bulk("alt-update-all", label, true, false);
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
                                ("shown", &shown.len().to_string()),
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
                                this.mods_state.selection.clear();
                                this.mods_state.anchor = None;
                                cx.notify();
                            })),
                    );
            }
            bar
        };

        let mut list = div().flex().flex_col().children(
            self.source_update_notice(None)
                .map(|notice| note(notice.to_owned()).p(px(10.))),
        );
        // Collection headings, gaps and rows in one flat list, so only those near the
        // screen are built: a big pack has hundreds of rows, each with its own controls.
        let mut items = Vec::new();
        let mut n = 0;
        for (collection, group) in model::installed_groups(&rows, &visible) {
            if let Some(title) = collection {
                items.push(InstalledItem::Head(title, group.len()));
            } else if n != 0 {
                // A gap ends the previous collection without giving loose mods a heading.
                items.push(InstalledItem::Gap);
            }
            for &i in group {
                items.push(InstalledItem::Row(i, n));
                n += 1;
            }
        }
        let cull = self.mods.installed_list.clone();
        // Measurements belong to the items they were taken of, including local archive paths
        // when two archives share a mod ID.
        cull.items(installed_items_key(&items, &rows));
        // The row another page opened Installed at stays built until it has been scrolled to.
        let reveal = match &self.mods_state.reveal {
            Some(search::Reveal::Mod(id)) if !self.mods_state.revealed.get() => items
                .iter()
                .position(|item| matches!(item, InstalledItem::Row(i, _) if rows[*i].id == *id)),
            _ => None,
        };
        let scroll = crate::frame::scrolls("installed");
        list = list.child(cull.column(
            &scroll,
            items.len(),
            0.,
            |k| Some(k) == reveal,
            |k| {
                match items[k] {
                    InstalledItem::Head(title, count) => div()
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
                        )
                        .into_any_element(),
                    InstalledItem::Gap => div()
                        .h(px(12.))
                        .bg(c(palette().background))
                        .into_any_element(),
                    InstalledItem::Row(i, n) => self
                        .row(
                            n,
                            rows[i].clone(),
                            labels[i].clone(),
                            shown.clone(),
                            busy,
                            cx,
                        )
                        .into_any_element(),
                }
            },
        ));
        if visible.is_empty() && refresh_ready {
            list = list.child(
                div()
                    .p(px(16.))
                    .child(muted(tr("No mods match this search or filter."))),
            );
        }
        let hidden = self
            .mods_state
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
            .child(crate::scrollbar::wrap_page_in_margin(
                "installed",
                div()
                    .id("alt-installed-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(list),
                &self.theme,
            ))
            .children((hidden > 0).then(|| {
                div().px(px(14.)).py(px(8.)).border_t_1().border_color(palette().rule()).child(muted(trf(
                    "{count} other files the launcher installed (packs, or from other sources) aren't listed here.",
                    &[("count", &hidden.to_string())],
                )))
            }));
        page = page.child(right);
        page.into_any_element()
    }

    pub(in crate::mods) fn labeled_installed_rows(&self) -> (Vec<Row>, Vec<Option<String>>) {
        let mut rows = self.rows();
        // Use the exact installed package's display name here, as in the Remove review.
        // Compute collisions before filtering so a row keeps its label when peers are hidden.
        for row in &mut rows {
            if let Some(archive) = self.archive_display(row) {
                row.title.clone_from(&archive.name);
                if row.authors.is_empty() {
                    row.authors = archive.authors.join(", ");
                }
            }
            if let Some(origin) = nightly::installed_source(row) {
                let update = self.source_update_version(origin).map(str::to_owned);
                if let Some(source) = self.source_listing(origin) {
                    row.authors = source.owner().to_owned();
                    row.collection = self.source_collection_for(source).map(Into::into);
                    row.source_listing = Some(source.clone());
                }
                row.update = update;
            }
            if let Some(origin) = row.local.as_ref().and_then(|local| local.github.as_ref()) {
                if row.authors.is_empty() && !origin.owner.is_empty() {
                    row.authors.clone_from(&origin.owner);
                }
            }
        }
        model::group_installed(&mut rows);
        use installed::{ModLabel, ModLabelSource};
        let labels = installed::colliding_mod_labels(rows.iter().map(|row| {
            let (authors, version) = self.archive_display(row).map_or_else(
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
}
