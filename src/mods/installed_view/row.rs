//! One installed row: its switch, labels and actions.
use super::*;

impl Launcher {
    pub(super) fn row(
        &mut self,
        n: usize,
        r: Row,
        label: Option<String>,
        shown: Rc<Vec<(String, String)>>,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let selected = self.mods_state.selection.contains(&r.id);
        let package = self
            .archive_display(&r)
            .and_then(|archive| archive.icon.clone());
        let artwork = if let Some(source) = &r.source_listing {
            self.source_tile(
                source,
                None,
                self.nightly_catalogue_current(),
                package,
                32.,
                cx,
            )
        } else {
            let listed = r
                .icon
                .as_ref()
                .and_then(|url| self.mods_image(url, images::ROW));
            tile(mod_artwork(listed, package), &r.id, &r.title, 32.)
        };
        let error = self.mods_state.errors.get(&r.id).cloned();

        let check = {
            let file = r.archive_file().map(str::to_owned);
            checkbox(("alt-row-check", n), selected).on_click(cx.listener(
                move |this, ev: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    if let Some(file) = &file {
                        this.mods_state
                            .select_installed_row(file, &shown, ev.modifiers().shift);
                    }
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
        let cells = model::cells(&r, error.as_deref(), building);
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
        let hold = self
            .mods_state
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.rows.switch_held.get(&r.id));
        let hold_label = hold.map(|hold| (hold.label(), hold.detail().map(str::to_owned)));
        let switch_held = hold_label.is_some();
        let status_cell: AnyElement = if switch_held {
            div().into_any_element()
        } else if let Some(hold) = install_hold {
            let detail = installed::hold_detail(hold);
            status("warning", installed::hold_label(hold), palette().warning)
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
                model::RowAction::Update => ("alt-row-update", tr("Update")),
            };
            let b = crate::frame::button((id, n), label);
            let stale_source = action == model::RowAction::Update
                && nightly::installed_source(&r).is_some()
                && !self.nightly_catalogue_current();
            if busy || stale_source {
                return dim(b).on_click(|_, _, cx| cx.stop_propagation());
            }
            let row = r.clone();
            b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                match action {
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
            if cells.switch && !busy {
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
        let manual_badge = r.manual_badge();
        let no_auto_updates = r.is_manual() && action.is_none();
        let source = r.source_label();
        let byline = if label.is_some() || r.authors.is_empty() {
            source.to_owned()
        } else {
            trf(
                "{author} · {source}",
                &[("author", &r.authors), ("source", source)],
            )
        };
        let title = div()
            .flex()
            .flex_col()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .min_w_0()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(label.unwrap_or_else(|| r.title.clone())),
                    )
                    .children(manual_badge.map(ui::manual_badge_pill)),
            )
            .child(
                div()
                    .id(("installed-origin", n))
                    .text_size(px(tokens::SMALL))
                    .text_color(c(palette().muted))
                    .truncate()
                    .tooltip(crate::glass_ui::tip(byline.clone()))
                    .child(byline),
            )
            .children(
                nightly::installed_source(&r)
                    .and(r.update.as_ref())
                    .map(|version| {
                        muted(trf("Update available: {version}", &[("version", version)]))
                    }),
            )
            .children(hold_label.map(|(label, detail)| {
                div()
                    .id(("alt-row-hold", n))
                    .text_size(px(tokens::SMALL))
                    .text_color(c(palette().warning))
                    .truncate()
                    .when_some(detail, |hold, detail| {
                        hold.tooltip(crate::glass_ui::tip(detail))
                    })
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
            .when_some(r.registry_issue.clone(), |d, issue| {
                d.child(
                    div()
                        .id(("installed-registry-issue", n))
                        .text_size(px(tokens::SMALL))
                        .text_color(c(palette().muted))
                        .truncate()
                        .tooltip(crate::glass_ui::tip(issue.clone()))
                        .child(issue),
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
                self.mods_state.reveal == Some(search::Reveal::Mod(r.id.clone())),
                |d| {
                    d.bg(c(palette().hover)).child(super::ui::reveal_in(
                        crate::frame::scrolls("installed"),
                        self.mods_state.revealed.clone(),
                        None,
                        Some(self.mods.installed_list.clone()),
                    ))
                },
            )
            .child(if r.selectable_archive() {
                div().flex_none().child(check)
            } else {
                div().flex_none().w(px(18.))
            })
            .child(artwork);
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
                        .children(action)
                        .when(no_auto_updates, |d| d.child(muted(tr("No auto-updates")))),
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
            .when(no_auto_updates, |d| d.child(muted(tr("No auto-updates"))))
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
}
