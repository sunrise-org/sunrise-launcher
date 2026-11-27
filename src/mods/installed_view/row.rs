//! One installed row, in fixed columns so every row lines up: the selection box, the picture,
//! the name and author, the source chip, the status, the action, the version, the switch and the
//! more button.
use super::*;
use crate::mods::selection::Gesture;
use crate::widgets::button::{icon_button, secondary, Style};
use crate::widgets::check::{checkbox, switch, Tick};
use crate::widgets::chip::{self, tag};

/// The row's own fixed widths (the shared ones are in `model::column`).
mod cols {
    pub(super) const PICTURE: f32 = 32.;
    pub(super) const SOURCE: f32 = 132.;
    pub(super) const MORE: f32 = 32.;
    pub(super) const CHECK: f32 = 18.;
}

/// The more button: the mod's details, as its row opens them too.
fn row_more(n: usize, r: &Row, cx: &mut Context<Launcher>) -> Stateful<Div> {
    let open = r.clone();
    icon_button(
        ("alt-row-more", n),
        "more-vertical",
        tr("Show details"),
        Style::Ghost,
    )
    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
        cx.stop_propagation();
        this.mods_show_installed_project(&open);
        cx.notify();
    }))
}

/// The source chip: the source's name, in the Sunrise chip, or the GitHub one for a mod added
/// from a repository or a local file.
fn row_source(r: &Row) -> Div {
    tag(
        r.source_label().to_owned(),
        if r.is_manual() {
            chip::Kind::GitHub
        } else {
            chip::Kind::Sunrise
        },
    )
}

/// The version, mono and muted, at its fixed width.
fn row_version(cells: &model::Cells) -> Div {
    div()
        .w(px(model::column::VERSION))
        .flex_none()
        .truncate()
        .text_size(px(tokens::LABEL))
        .font_family(crate::theme::MONO)
        .text_color(c(palette().muted))
        .child(cells.version.clone())
}

fn tone_colour(tone: model::Tone) -> u32 {
    match tone {
        model::Tone::Warning => palette().warning,
        model::Tone::Info => palette().info,
        model::Tone::Accent => palette().accent_ink,
        model::Tone::Success => palette().success,
        model::Tone::Muted => palette().muted,
    }
}

/// A fixed-width cell of the row, its contents at the start (or `end`, to the right).
fn cell(width: f32) -> Div {
    div().w(px(width)).flex_none().flex().items_center()
}

/// What holds a row back, for its status cell.
#[derive(Clone, Copy)]
struct Held {
    /// The last build failed.
    failed: bool,
    /// A switch is held until the game closes.
    switch_held: bool,
    /// The list is busy (a build or a change running).
    busy: bool,
}

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
        // Its box shows under the pointer, and on every row once one is ticked.
        let any_selected = !self.mods_state.selection.is_empty();
        let artwork = self.row_artwork(&r, cx);
        let check = self.row_check(n, &r, selected, shown.clone(), cx);
        let error = self.mods_state.errors.get(&r.id).cloned();
        use super::build_run::State;
        let building = self
            .build_here()
            .filter(|_| r.on)
            .and_then(|run| run.state_of(&r.id, &r.version));
        let failed = matches!(building, Some(State::Failed(_)));
        let cells = model::cells(&r, error.as_deref(), building);
        let hold_label = self.row_hold(&r);
        let status = self.row_status(
            n,
            &r,
            &cells,
            Held {
                failed,
                switch_held: hold_label.is_some(),
                busy,
            },
            cx,
        );
        let action = self.row_action(n, &r, cells.action, busy, cx);
        let sw = self.row_switch(n, &r, &cells, busy, cx);
        let more = row_more(n, &r, cx);
        let no_auto_updates = r.is_manual() && action.is_none();
        let title = self.row_title(n, &r, label, hold_label, cells.kind);
        let version = row_version(&cells);
        let row = div()
            .id(("alt-row", n))
            .group("installed-row")
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(14.))
            .min_h(px(52.))
            .border_t_1()
            .border_color(palette().rule())
            .when(selected, |d| d.bg(c(palette().selected)))
            .when_some(self.row_reveal(&r), |d, reveal| {
                d.bg(c(palette().hover)).child(reveal)
            })
            .child(if r.selectable_archive() {
                cell(cols::CHECK)
                    .opacity(if any_selected { 1. } else { 0. })
                    .group_hover("installed-row", |s| s.opacity(1.))
                    .child(check)
            } else {
                div().flex_none().w(px(cols::CHECK))
            })
            .child(cell(cols::PICTURE).child(artwork))
            .child(title.flex_1().min_w(px(140.)))
            .child(cell(cols::SOURCE).min_w_0().child(row_source(&r)))
            .child(cell(model::column::STATUS).min_w_0().child(status))
            .child(
                div()
                    .w(px(model::column::ACTION))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .children(action)
                    .when(no_auto_updates, |d| d.child(muted(tr("No auto-updates")))),
            )
            .child(version)
            .child(
                div()
                    .w(px(model::column::SWITCH))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .child(sw),
            )
            .child(cell(cols::MORE).justify_end().child(more));
        row.reach()
            .cursor_pointer()
            .hover(|s| s.bg(c(palette().hover)))
            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                // Ctrl (Cmd) or Shift ticks the row where a plain press opens it.
                let gesture = Gesture::of(&ev.modifiers());
                if gesture.selects() && r.selectable_archive() {
                    if let Some(file) = r.archive_file() {
                        this.mods_state
                            .select_installed_row(file, &shown, gesture.range);
                        cx.notify();
                        return;
                    }
                }
                this.mods_show_installed_project(&r);
                cx.notify();
            }))
    }

    /// The picture: the source's tile for a mod from a source, else the mod's own icon.
    fn row_artwork(&mut self, r: &Row, cx: &mut Context<Self>) -> AnyElement {
        let package = self
            .archive_display(r)
            .and_then(|archive| archive.icon.clone());
        if let Some(source) = &r.source_listing {
            self.source_tile(
                source,
                None,
                self.nightly_catalogue_current(),
                package,
                cols::PICTURE,
                cx,
            )
            .into_any_element()
        } else {
            let listed = r
                .icon
                .as_ref()
                .and_then(|url| self.mods_image(url, images::ROW));
            tile(mod_artwork(listed, package), &r.id, &r.title, cols::PICTURE).into_any_element()
        }
    }

    fn row_check(
        &self,
        n: usize,
        r: &Row,
        selected: bool,
        shown: Rc<Vec<(String, String)>>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let file = r.archive_file().map(str::to_owned);
        checkbox(
            ("alt-row-check", n),
            if selected { Tick::On } else { Tick::Off },
        )
        .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
            cx.stop_propagation();
            if let Some(file) = &file {
                this.mods_state
                    .select_installed_row(file, &shown, ev.modifiers().shift);
            }
            cx.notify();
        }))
    }

    /// The mod another page opened Installed at, marked with what it was opened at.
    fn row_reveal(&self, r: &Row) -> Option<AnyElement> {
        (self.mods_state.reveal == Some(search::Reveal::Mod(r.id.clone()))).then(|| {
            super::ui::reveal_in(
                crate::frame::scrolls("installed"),
                self.mods_state.revealed.clone(),
                None,
                Some(self.mods.installed_list.clone()),
            )
            .into_any_element()
        })
    }

    /// Why the mod's switch is held, if it is: the label, and the detail when there is one.
    fn row_hold(&self, r: &Row) -> Option<(&'static str, Option<String>)> {
        let hold = self
            .mods_state
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.rows.switch_held.get(&r.id))?;
        Some((hold.label(), hold.detail().map(str::to_owned)))
    }

    /// The line for a source mod whose source has a problem, if it has one.
    fn row_source_issue(&self, n: usize, r: &Row) -> Option<Stateful<Div>> {
        nightly::installed_source(r)
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
            })
    }

    /// The name, with the author and kind under it, then the lines that say what is going on.
    fn row_title(
        &self,
        n: usize,
        r: &Row,
        label: Option<String>,
        hold_label: Option<(&'static str, Option<String>)>,
        kind: &str,
    ) -> Div {
        let manual_badge = r.manual_badge();
        let byline = if r.authors.is_empty() {
            kind.to_owned()
        } else if kind.is_empty() {
            r.authors.clone()
        } else {
            trf(
                "{author} · {source}",
                &[("author", &r.authors), ("source", kind)],
            )
        };
        div()
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
                    .children(manual_badge.map(|badge| tag(badge.label(), chip::Kind::Neutral))),
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
                nightly::installed_source(r)
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
            .children(self.row_source_issue(n, r))
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
            })
    }

    /// The status: what a switch held, an install hold, a failed build or the row's own status,
    /// each with what it opens when it has one.
    fn row_status(
        &mut self,
        n: usize,
        r: &Row,
        cells: &model::Cells,
        held: Held,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Held {
            failed,
            switch_held,
            busy,
        } = held;
        let install_hold = r
            .entry
            .as_ref()
            .and_then(|entry| entry.install_hold.as_ref())
            .filter(|_| !r.on);
        if switch_held {
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
                    self.unfit(n, r, unfit, busy, cx).into_any_element()
                }
                // A mod the last build could not build: its status opens that build's window
                // again, where Details and the build log are.
                (Some(said), _) if failed && said.label == tr("Failed") => {
                    status(said.icon, said.label.clone(), tone_colour(said.tone))
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
                (Some(said), _) => status(said.icon, said.label.clone(), tone_colour(said.tone))
                    .id(("alt-row-status", n))
                    .min_w_0()
                    .tooltip(crate::glass_ui::tip(said.tip.clone()))
                    .into_any_element(),
                (None, _) => div().into_any_element(),
            }
        }
    }

    /// The row's one action (Update), drawn dim while busy or while its source is stale.
    fn row_action(
        &mut self,
        n: usize,
        r: &Row,
        action: Option<model::RowAction>,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        action.map(|action| {
            let (id, label) = match action {
                model::RowAction::Update => ("alt-row-update", tr("Update")),
            };
            let b = secondary(format!("{id}-{n}"), label);
            let stale_source = action == model::RowAction::Update
                && nightly::installed_source(r).is_some()
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
        })
    }

    /// The switch that turns the mod on or off, dim where it can't be flipped.
    fn row_switch(
        &mut self,
        n: usize,
        r: &Row,
        cells: &model::Cells,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let s = switch(("alt-row-switch", n), r.on).when(cells.unavailable, |s| {
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
    }
}
