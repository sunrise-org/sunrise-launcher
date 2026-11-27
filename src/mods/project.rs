//! A mod's page (mock 04): one button, and a plain list of what adding it changes.
use super::model;
use super::ui::*;
use super::*;
use crate::paint::palette;
use crate::widgets::button::{primary, secondary};
use crate::widgets::check::switch;
use crate::widgets::chip::{self, tag};
use sunrise_launcher::browser::images;

mod data;
pub(in crate::mods) mod look;
mod page;

use crate::widgets::feedback::{banner, Tone};
use look::{action_column, head, muted, note};

#[cfg(test)]
mod tests;

/// Show every registry screenshot. Pack feeds can list many more: the full-size viewer steps
/// through those too, without fetching all of a large pack gallery on opening its page.
/// The parts of a detail page around its body (see [`Launcher::project_layout`]).
pub(in crate::mods) struct Layout {
    /// The strip above everything (back link, notices).
    pub header: Div,
    /// The artwork, name, chips and facts.
    pub head: Div,
    /// The main action and what goes under it, right of the head.
    pub action: Option<Div>,
    pub gallery: Option<Div>,
    /// The About card and the like, beside the body on a wide window.
    pub side: Option<Div>,
}

impl Launcher {
    pub(super) fn gallery(
        &mut self,
        urls: Vec<super::viewer::Picture>,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        if urls.is_empty() {
            return None;
        }
        let urls: Rc<[super::viewer::Picture]> = urls.into();
        const PREVIEWS: usize = sunrise_launcher_smod::index::page::MAX_SCREENSHOTS;
        let count = urls.len().min(PREVIEWS);
        let columns = if self.window_width < 950. { 3 } else { 4 };
        let height = if count == 1 { 220. } else { 104. };
        // A tile's tooltip would float over its own picture once that is open.
        let viewing = self.mods_state.viewer.is_some();
        let mut tiles = urls.iter().take(PREVIEWS).enumerate().map(|(i, url)| {
            let photo = self.picture(url, images::GALLERY);
            // No wider than the picture's own shape at the row's height: a wide window would
            // otherwise blow it up and crop it.
            let ratio = photo
                .as_deref()
                .and_then(super::viewer::picture_size)
                .map_or(16. / 9., |(w, h)| w.max(1) as f32 / h.max(1) as f32);
            let all = urls.clone();
            let label = trf(
                "Picture {number} of {total}",
                &[
                    ("number", &(i + 1).to_string()),
                    ("total", &urls.len().to_string()),
                ],
            );
            div()
                .id(ElementId::named_usize("gallery-picture", i))
                .reach_edge()
                .relative()
                .flex_1()
                .min_h_0()
                .w_full()
                .max_w(px(height * ratio))
                .cursor_pointer()
                .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                .border_1()
                .border_color(palette().rule())
                .bg(palette().card())
                .overflow_hidden()
                .when(!viewing, |d| d.tooltip(crate::glass_ui::tip(label)))
                .children(photo.map(|path| {
                    gpui::img(path)
                        .size_full()
                        .object_fit(gpui::ObjectFit::Contain)
                }))
                .when(i + 1 == PREVIEWS && urls.len() > PREVIEWS, |d| {
                    d.child(
                        div()
                            .absolute()
                            .right_0()
                            .bottom_0()
                            .px(px(6.))
                            .bg(c(palette().raised))
                            .child(format!("+{}", urls.len() - PREVIEWS)),
                    )
                })
                .track_scroll(&super::viewer::tile(i))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.viewer_open(all.clone(), i);
                    cx.notify();
                }))
        });
        let mut grid = div().flex().flex_col().flex_none().gap(px(6.));
        for _ in 0..count.div_ceil(columns) {
            grid = grid.child(
                div()
                    .flex()
                    .gap(px(6.))
                    .h(px(height))
                    .children(tiles.by_ref().take(columns)),
            );
        }
        Some(grid)
    }
}

/// A page's description, written in Markdown, in this look's colours.
pub(super) fn prose(theme: crate::theme::Theme, text: &str) -> AnyElement {
    let mut t = field_theme(theme);
    t.ink_soft = c(palette().text);
    t.plate_soft = c(palette().card_color);
    t.rule = c(palette().divider);
    crate::markdown::render(text, &t)
}

/// "‹ Discover": the way back from a page. Its padding widens the hit area and the negative
/// margin keeps the label lined up with the page.
pub(super) fn back(label: impl Into<SharedString>) -> Stateful<Div> {
    let p = palette();
    div()
        .id("alt-project-back")
        .group("alt-project-back")
        .reach()
        .self_start()
        .flex()
        .items_center()
        .gap(px(4.))
        .px(px(8.))
        .py(px(4.))
        .ml(px(-8.))
        .my(px(-4.))
        .rounded(px(6.))
        .text_size(px(tokens::LABEL))
        .text_color(c(p.muted))
        .cursor_pointer()
        .hover(|s| s.bg(c(p.text).opacity(0.06)).text_color(c(p.text)))
        .child(
            crate::blocks::icon("chevron-right", 14., c(p.muted))
                .with_transformation(gpui::Transformation::rotate(gpui::radians(
                    std::f32::consts::PI,
                )))
                .group_hover("alt-project-back", |s| s.text_color(c(p.text))),
        )
        .child(label.into())
}

/// One line of a page's About card.
pub(super) fn about_row(key: &'static str, value: String) -> Div {
    div()
        .flex()
        .justify_between()
        .gap(px(12.))
        .py(px(5.))
        .text_size(px(tokens::LABEL))
        .child(div().flex_none().text_color(c(palette().muted)).child(key))
        .child(div().min_w_0().truncate().child(value))
}

/// The line of an About card that opens a web page.
pub(super) fn about_link(id: &'static str, key: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .reach()
        .flex()
        .justify_between()
        .py(px(5.))
        .text_size(px(tokens::LABEL))
        .cursor_pointer()
        .child(div().text_color(c(palette().muted)).child(key))
        .child(
            div()
                .text_color(c(palette().accent_ink))
                .child(tr("Open ↗")),
        )
}

pub(super) fn project_frame() -> Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .gap(px(14.))
        .p(px(24.))
        .text_color(c(palette().text))
}

impl Launcher {
    pub(super) fn installed_project_controls(
        &self,
        row: &Row,
        cells: &model::Cells,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let hold = self
            .mods_state
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.switch_held.get(&row.id))
            .cloned();
        let status = if let Some(hold) = &hold {
            Some(note(match hold.detail() {
                Some(detail) => format!("{} {detail}", hold.label()),
                None => hold.label().to_owned(),
            }))
        } else if cells.popover {
            row.unfit
                .clone()
                .map(|unfit| self.unfit(0, row, unfit, busy, cx))
        } else {
            cells.status.as_ref().map(|status| note(status.tip.clone()))
        };
        let toggle = switch("alt-project-switch", row.on);
        let toggle = if busy || !cells.switch {
            dim(toggle)
        } else {
            let row = row.clone();
            toggle.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.toggle(&row);
                cx.notify();
            }))
        };
        let manual_badge = row.manual_badge();
        let manual_refresh = manual_badge.map(|_| {
            if let Some(progress) = self.registry_card_control(&row.id, |label| {
                dim(secondary("alt-project-github-progress", label)).into_any_element()
            }) {
                return progress;
            }
            let button = secondary("alt-project-manual-refresh", tr("Add from source"));
            if busy {
                dim(button).into_any_element()
            } else {
                button
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.open_add_from_source();
                        cx.notify();
                    }))
                    .into_any_element()
            }
        });
        let remove = row
            .selectable_archive()
            .then(|| self.remove_mod_button(&row.id, busy, cx));
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .children(manual_badge.map(|badge| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(tag(badge.label(), chip::Kind::Neutral))
                    .child(muted(tr(
                        "No automatic updates. Refresh via Add from source.",
                    )))
            }))
            .children(status)
            .children(
                hold.as_ref()
                    .and_then(|reason| reason.detail())
                    .map(|detail| note(detail.to_owned())),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.))
                    .child(toggle)
                    .child(note(if row.on {
                        tr("Turned on")
                    } else {
                        tr("Turned off")
                    }))
                    .children(manual_refresh)
                    .children(remove),
            )
    }

    pub(super) fn project(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let back_to = self
            .mods_state
            .open
            .as_ref()
            .map_or(Page::Discover, |o| o.from);
        let parent = self.mods_state.open.as_ref().and_then(|o| o.parent.clone());
        let parent_name = parent.as_ref().and_then(|parent| match parent {
            Parent::Project { name, .. } => Some(name.clone()),
            Parent::Collection(id) => match &self.mods_state.v2_catalog {
                V2Catalog::Ready { collections, .. } => collections
                    .iter()
                    .find(|c| c.id == *id)
                    .map(|c| c.name.clone()),
                _ => None,
            },
        });
        let back_label = match (back_to, &parent_name) {
            (_, Some(name)) => name.clone(),
            (Page::Installed, _) => tr("Installed").to_string(),
            _ => tr("Discover").to_string(),
        };
        // The back link scrolls away with the page, above the side panel.
        let header = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                back(back_label).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    match &parent {
                        Some(Parent::Collection(collection)) => {
                            this.mods_show_v2_collection(collection.clone(), back_to)
                        }
                        Some(Parent::Project { id, parent, .. }) => this.mods_return_to_project(
                            id.clone(),
                            back_to,
                            parent.as_deref().cloned(),
                        ),
                        None => this.ui.page = back_to,
                    }
                    cx.notify();
                })),
            )
            .children(self.strip(cx));
        let page = project_frame();
        let Some(open) = self.mods_state.open.as_ref() else {
            return page
                .child(header)
                .child(muted(tr("No mod is open.")))
                .into_any_element();
        };
        if open.collection {
            return self.v2_collection_page(header, page, cx);
        }
        if open.v2 {
            return self.v2_project_page(header, page, cx);
        }
        let (rows, labels) = self.labeled_installed_rows();
        let Some(i) = rows.iter().position(|row| match &open.installed {
            Some(InstalledProject::Local(file)) => {
                row.id == open.id && row.local.as_ref().is_some_and(|local| &local.file == file)
            }
            Some(InstalledProject::Registry) | None => row
                .entry
                .as_ref()
                .is_some_and(|entry| entry.project == open.id),
        }) else {
            return page
                .child(header)
                .child(muted(tr("No mod is open.")))
                .into_any_element();
        };
        let row = &rows[i];
        // These controls mutate by mod ID, so showing a specific archive mustn't choose
        // which duplicate to mutate.
        let conflict = rows
            .iter()
            .any(|other| other.id == row.id && other.archive_file() != row.archive_file());
        let title = labels[i].as_ref().unwrap_or(&row.title);
        let pending = self
            .discover_installed()
            .is_some_and(|rows| rows.registry_pending(&row.id));
        let uses_registry = !matches!(open.installed, Some(InstalledProject::Local(_)));
        let package = self
            .archive_display(row)
            .and_then(|archive| archive.icon.clone());
        let picture = mod_artwork(
            row.icon
                .as_ref()
                .and_then(|url| self.mods_image(url, images::ICON)),
            package,
        );
        let by =
            (!row.authors.is_empty()).then(|| trf("by {authors}", &[("authors", &row.authors)]));
        let kind = row
            .make
            .map(|make| tag(make.label(), chip::Kind::Neutral).into_any_element());
        let header_block = head(
            tile(picture, &row.id, title.as_str(), 64.).into_any_element(),
            title.as_str(),
            by,
            kind.into_iter().collect(),
            None,
            vec![row.version.clone()],
        );

        // The right column: the action, then the installed mod's controls.
        let busy = conflict
            || self.mods_busy()
            || self.mods_locked().is_some()
            || self
                .mods_state
                .snapshot
                .as_ref()
                .is_none_or(|snapshot| !snapshot.ready());
        let cells = model::cells(
            row,
            self.mods_state.errors.get(&row.id).map(String::as_str),
            self.build_here()
                .filter(|_| row.on)
                .and_then(|run| run.state_of(&row.id, &row.version)),
        );
        let update = row
            .update
            .as_ref()
            .filter(|_| cells.action == Some(model::RowAction::Update));
        let action: AnyElement = if let Some(progress) = uses_registry
            .then(|| {
                self.registry_card_control(&row.id, |label| {
                    dim(primary("alt-project-add", label)
                        .w_full()
                        .h(px(tokens::ACTION)))
                    .into_any_element()
                })
            })
            .flatten()
        {
            progress
        } else if let Some(to) = update {
            let b = primary(
                "alt-project-update",
                trf(
                    "Update {from} → {to}",
                    &[("from", &row.version), ("to", to)],
                ),
            )
            .w_full()
            .h(px(tokens::ACTION));
            let stale_source =
                nightly::installed_source(row).is_some() && !self.nightly_catalogue_current();
            if busy || stale_source {
                dim(b).into_any_element()
            } else {
                let row = row.clone();
                b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if nightly::installed_source(&row).is_some() {
                        this.submit_source_update(&row.id);
                    } else {
                        this.update_one(&row);
                    }
                    cx.notify();
                }))
                .into_any_element()
            }
        } else {
            div()
                .h(px(tokens::ACTION))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                .border_1()
                .border_color(palette().rule())
                .child(super::discover::install_fact(&row.version, pending))
                .into_any_element()
        };
        let refusal = nightly::installed_source(row)
            .filter(|_| !conflict)
            .and_then(|origin| {
                let problem = self
                    .source_outcome(&row.id)
                    .and_then(|status| status.as_ref().err())
                    .or_else(|| self.source_update_issue(&row.id))?;
                let repo = format!("{}/{}", origin.owner, origin.repo);
                let refused = super::refused::Refused::one(&row.title, Some(repo), true, problem)?;
                Some(self.refused_line("alt-project-source-refused", refused, cx))
            });
        let origin = row.local.as_ref().and_then(|local| local.github.as_ref());
        let installed_controls = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .children(origin.map(|origin| note(nightly::source_provenance(origin))))
            .children(
                origin
                    .and_then(|origin| origin.tag.as_ref())
                    .map(|tag| note(trf("Release: {tag}", &[("tag", tag)]))),
            )
            .children(refusal)
            .children(conflict.then(|| note(tr(nightly::SOURCE_ID_CONFLICT))))
            .child(self.installed_project_controls(row, &cells, busy, cx));
        let source_button = origin
            .and_then(|origin| model::SourceLink::github(&origin.owner, &origin.repo))
            .and_then(|link| match link {
                model::SourceLink::GitHub(url) => Some(url),
                model::SourceLink::Other(_) => None,
            })
            .map(|url| {
                secondary("alt-project-github", tr("Open on GitHub"))
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| cx.open_url(&url)))
            });
        // A source mod is built from code no one at Sunrise has read: said here, on its page.
        let unreviewed = origin.map(|_| {
            banner(
                Tone::Warning,
                tr("This mod has not been reviewed"),
                Some(tr(sunrise_launcher::github_mods::WARNING).into()),
            )
        });
        let actions = action_column()
            .child(action)
            .children(
                uses_registry
                    .then(|| self.registry_card_outcome(&row.id, cx))
                    .flatten(),
            )
            .child(installed_controls)
            .children(source_button);
        let side = self.other_ports_card(&row.id, Some(row), busy, cx);
        self.project_layout(
            "project",
            Layout {
                header,
                head: header_block,
                action: Some(actions),
                gallery: None,
                side,
            },
            unreviewed.into_iter().map(IntoElement::into_any_element),
        )
    }

    /// The one detail layout for registry, source, local, pack and collection pages. The header
    /// block sits left and the `action` column right of it. Beside the body, `side` (the About
    /// card and the like) takes a column of its own; on a narrow window the action and the side
    /// move above the body, one under the other. `key` is the page's scroll position.
    pub(super) fn project_layout(
        &self,
        key: &'static str,
        layout: Layout,
        body: impl IntoIterator<Item = AnyElement>,
    ) -> AnyElement {
        let Layout {
            header,
            head,
            action,
            gallery,
            side,
        } = layout;
        let scroll = crate::frame::scrolls(key);
        let wide = self.content_width() >= 860.;
        let (top_action, above_action) = match (wide, action) {
            (true, action) => (
                action.map(|a| a.w(px(look::ACTION_WIDTH)).flex_none()),
                None,
            ),
            (false, action) => (None, action),
        };
        let (beside, above_side) = match (wide, side) {
            (true, Some(side)) => (
                Some(
                    side.relative()
                        .top(px(self.mods_sticky.shift(&scroll, 1, 1)))
                        .w(px(look::SIDE_WIDTH))
                        .flex_none(),
                ),
                None,
            ),
            (_, side) => (None, side),
        };
        let top = div()
            .flex()
            .flex_wrap()
            .items_start()
            .justify_between()
            .gap(px(24.))
            .child(div().flex_1().min_w(px(280.)).min_w_0().child(head))
            .children(top_action);
        let left = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(20.))
            .children(above_action)
            .children(above_side)
            .children(gallery)
            .children(body);
        project_frame()
            .child(crate::scrollbar::wrap_page_in_margin(
                key,
                div()
                    .id("alt-project-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(20.))
                    .child(header)
                    .child(top)
                    .child(
                        div()
                            .id("alt-project-columns")
                            .track_scroll(self.mods_sticky.row())
                            .flex()
                            .gap(px(24.))
                            .items_start()
                            .child(left)
                            .children(beside),
                    ),
                &self.theme,
            ))
            .into_any_element()
    }
}

/// The build a release was made for, by its short commit too: two Dev builds can share a day.
fn made_for(line: &model::discover_v2::V2VersionLine) -> Div {
    muted(trf(
        "Made for {build}",
        &[(
            "build",
            &crate::sheets::build_name_with_commit(&line.made_for),
        )],
    ))
}
