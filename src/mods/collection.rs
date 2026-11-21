//! A collection's page (Mods › Discover › a collection): its mods to pick from, one Install.
//! Each mod installs, updates and is removed on its own afterwards; the collection owns nothing.
use super::model::collection::{self as plan, Line};
use super::project::{about_row, prose};
use super::ui::*;
use super::*;
use crate::paint::palette;
use std::sync::Arc;
use sunrise_launcher::browser::images;
use sunrise_launcher_smod::index::{Collection, Index};

/// The picks made on the open collection's page, and what the planner said about them.
pub(super) struct Picks {
    id: String,
    ticked: BTreeSet<String>,
    planned: plan::Planned,
}

type Shown = (Arc<Index>, Collection, plan::View);

impl Launcher {
    /// The collection's page for the picks made so far.
    fn collection_view(&mut self, id: &str) -> Option<Shown> {
        let index = sunrise::known_index()?;
        let collection = index.collection(id)?.clone();
        let held = sunrise::held_mods(
            self.ui
                .mods
                .snapshot
                .as_ref()
                .map_or(&[][..], |s| s.entries.as_slice()),
        );
        let game = self.mods_game();
        let slot = &mut self.ui.mods.picks;
        if slot.as_ref().is_none_or(|p| p.id != id) {
            let mut planned = plan::Planned::default();
            let ticked =
                plan::initial(&collection, planned.base(&index, &collection, &held, &game));
            *slot = Some(Picks {
                id: id.to_owned(),
                ticked,
                planned,
            });
        }
        let picks = slot.as_mut()?;
        let view = picks
            .planned
            .view(&index, &collection, &picks.ticked, &held, &game)
            .clone();
        Some((index, collection, view))
    }

    fn collection_flip(&mut self, id: &str, line: Option<&str>) {
        let Some((_, _, view)) = self.collection_view(id) else {
            return;
        };
        let Some(picks) = self.ui.mods.picks.as_mut() else {
            return;
        };
        match line {
            Some(line) => view.flip(&mut picks.ticked, line),
            None => view.flip_all(&mut picks.ticked),
        }
    }

    /// Install what is ticked and not yet installed, planned together.
    fn collection_install(&mut self, id: &str) {
        let Some((_, _, view)) = self.collection_view(id) else {
            return;
        };
        if view.conflict.is_some() {
            return;
        }
        self.ui.mods.notice = None;
        if let Err(why) = self.queue_registry_install(id.into(), view.to_install()) {
            self.ui.mods.notice = Some(Err(why));
        }
    }

    pub(super) fn collection_page(
        &mut self,
        header: Div,
        page: Div,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(id) = self.ui.mods.open.as_ref().map(|o| o.id.clone()) else {
            return page.child(header).into_any_element();
        };
        let Some((index, collection, view)) = self.collection_view(&id) else {
            return page
                .child(header)
                .child(muted(tr("This collection can't be shown.")))
                .into_any_element();
        };
        let working = self.registry_card_control(&id, |label| {
            dim(primary("alt-collection-install", label)
                .w_full()
                .h(px(tokens::ACTION)))
            .into_any_element()
        });
        let owners = collection.owners.join(", ");
        let icon = self.mods_image(&collection.icon, images::ICON);

        let head = div()
            .flex()
            .items_center()
            .gap(px(16.))
            .child(tile(icon.clone(), &collection.id, &collection.name, 64.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(tokens::DISPLAY))
                            .font_weight(FontWeight::BOLD)
                            .child(crate::selectable::selectable(collection.name.clone())),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_x(px(8.))
                            .gap_y(px(4.))
                            .child(note(trf("by {authors}", &[("authors", &owners)])))
                            .child(badge(tr("Collection"))),
                    ),
            );

        // The list: a header that picks all, then one row per mod, libraries last.
        let list_head = {
            let id = id.clone();
            div()
                .id("alt-collection-all")
                .reach()
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(14.))
                .py(px(10.))
                .cursor_pointer()
                .text_size(px(tokens::LABEL))
                .text_color(c(palette().muted))
                .child(checkbox_of_set("alt-collection-all-box", view.header).tab_stop(false))
                .child(trf(
                    "{picked} of {all} selected",
                    &[
                        ("picked", &view.ticked.to_string()),
                        ("all", &view.members.to_string()),
                    ],
                ))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.collection_flip(&id, None);
                    cx.notify();
                }))
        };
        let mut rows = card().child(list_head);
        for (i, line) in view.lines.iter().enumerate() {
            rows = rows.child(self.collection_line(&id, i, line, cx));
        }

        // The side panel: the one Install, and what it does.
        let summary = &view.summary;
        let done = view.done();
        let ready = view.can_install() && self.mods_locked().is_none();
        let action: AnyElement = if let Some(progress) = working {
            progress
        } else if done {
            div()
                .h(px(tokens::ACTION))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                .border_1()
                .border_color(c(palette().divider))
                .text_color(c(palette().success))
                .child(blocks::icon("check", 14., c(palette().success)))
                .child(tr("Installed"))
                .into_any_element()
        } else {
            let b = primary("alt-collection-install", self.registry_install_label())
                .w_full()
                .h(px(tokens::ACTION));
            if ready {
                let id = id.clone();
                b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.collection_install(&id);
                    cx.notify();
                }))
                .into_any_element()
            } else {
                dim(b).into_any_element()
            }
        };
        let plan_line = view.plan_line();
        let fact = |icon: &'static str, label: String| {
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(9.))
                .h(px(28.))
                .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                .border_1()
                .border_color(c(palette().divider))
                .bg(c(palette().field))
                .text_size(px(tokens::SMALL))
                .child(crate::glass_ui::icon(icon, 14., c(palette().muted)))
                .child(label)
        };
        let mut facts = div().flex().flex_wrap().gap(px(6.));
        if summary.bytes > 0 {
            facts = facts.child(fact("download-04", model::size(summary.bytes)));
        }
        if view
            .lines
            .iter()
            .any(|l| l.ticked && sunrise::is_native_mod(&l.id))
        {
            facts = facts.child(fact("shield", tr("Native code").to_string()));
        }
        facts = facts.child(fact("user", trf("By {authors}", &[("authors", &owners)])));
        let tail = match done {
            true => tr("Each mod updates and is removed on its own, from Installed."),
            false => tr("The selected mods and their dependencies install together."),
        };
        let downloads: u64 = collection
            .members
            .iter()
            .filter_map(|m| index.project(&m.id))
            .map(|p| p.downloads)
            .sum();
        let updated = collection
            .members
            .iter()
            .filter_map(|m| index.project(&m.id))
            .flat_map(|p| p.versions.iter().map(|v| v.published.clone()))
            .max()
            .unwrap_or_default();
        let libraries = view.lines.iter().filter(|l| l.library).count();
        let mut about = card()
            .p(px(16.))
            .flex()
            .flex_col()
            .child(
                div()
                    .text_size(px(tokens::CAPTION))
                    .text_color(c(palette().muted))
                    .pb(px(6.))
                    .child(tr("ABOUT")),
            )
            .child(about_row(tr("Mods"), collection.members.len().to_string()));
        if libraries > 0 {
            about = about.child(about_row(tr("Libraries"), libraries.to_string()));
        }
        about = about
            .child(about_row(tr("Downloads"), downloads.to_string()))
            .child(about_row(tr("Updated"), updated));
        let right = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                card()
                    .p(px(16.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(tile(icon, &collection.id, &collection.name, 40.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .truncate()
                                            .font_weight(FontWeight::BOLD)
                                            .child(collection.name.clone()),
                                    )
                                    .child(muted(trf(
                                        "Collection · {count} mods",
                                        &[("count", &collection.members.len().to_string())],
                                    ))),
                            ),
                    )
                    .child(action)
                    .children(self.registry_card_outcome(&id, cx))
                    .child(
                        div()
                            .text_size(px(tokens::LABEL))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(plan_line),
                    )
                    .child(facts)
                    .child(muted(tail)),
            )
            .child(about);

        let scroll = crate::frame::scrolls("project");
        let (above, beside) = match self.content_width() < 860. {
            true => (Some(right), None),
            // The panel stays in view while the list scrolls.
            false => (
                None,
                Some(
                    right
                        .relative()
                        .top(px(self.mods_sticky.shift(&scroll, 1, 1)))
                        .w(px(360.))
                        .flex_none(),
                ),
            ),
        };
        let about_text = match collection.description.trim().is_empty() {
            true => collection.summary.clone(),
            false => collection.description.clone(),
        };
        let left = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(head)
            .children(above)
            .child(prose(self.theme, &about_text))
            .child(
                div()
                    .text_size(px(tokens::HEADING))
                    .font_weight(FontWeight::BOLD)
                    .child(tr("Mods")),
            )
            .child(rows);
        page.child(
            div()
                .id("alt-project-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&scroll)
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(header)
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
        )
        .into_any_element()
    }

    /// One mod of the list: its box ticks it, its name opens its own page.
    fn collection_line(
        &mut self,
        id: &str,
        i: usize,
        line: &Line,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let picture = line
            .icon
            .clone()
            .and_then(|url| self.mods_image(&url, images::ROW));
        let locked = line.locked() || line.library || line.refused.is_some();
        let mut name = div().flex().items_baseline().gap(px(8.)).min_w_0().child({
            let open = line.id.clone();
            div()
                .id(("alt-collection-name", i))
                .reach()
                .truncate()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(tokens::LABEL))
                .cursor_pointer()
                .hover(|s| s.text_color(c(palette().accent_ink)))
                .child(line.name.clone())
                // The row's own press ticks it: this one only opens the page.
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.mods_show_project(open.clone());
                    cx.notify();
                }))
        });
        if line.labelled {
            name = name.child(badge(tr("Library")));
        }
        let mut text = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(name)
            .child(
                div()
                    .truncate()
                    .text_size(px(tokens::SMALL))
                    .text_color(c(palette().muted))
                    .child(line.summary.clone()),
            );
        if line.locked() {
            text = text.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .pt(px(2.))
                    .text_size(px(tokens::SMALL))
                    .text_color(c(palette().muted))
                    .child(crate::glass_ui::icon("link-01", 12., c(palette().muted)))
                    .child(div().truncate().child(trf(
                        "Needed by {mods}",
                        &[("mods", &line.needed_by.join(", "))],
                    ))),
            );
        } else if line.library {
            text = text.child(muted(tr("Not needed by your picks")));
        }
        if let Some(why) = line.refused.as_ref().or(line.conflict.as_ref()) {
            text = text.child(div().truncate().child(muted(why.clone())));
        }
        let right = if line.installed {
            status("check", tr("Installed"), palette().success)
        } else {
            div()
                .flex_none()
                .font_family(crate::theme::MONO)
                .text_size(px(tokens::SMALL))
                .text_color(c(palette().muted))
                .child(line.version.clone())
        };
        let tick = checkbox(("alt-collection-box", i), line.ticked).tab_stop(false);
        let (collection, member) = (id.to_owned(), line.id.clone());
        div()
            .id(("alt-collection-row", i))
            .reach()
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(14.))
            .py(px(10.))
            .border_t_1()
            .border_color(c(palette().divider))
            .when(!locked, |d| d.cursor_pointer())
            .child(match locked {
                true => dim(tick),
                false => tick,
            })
            .child(tile(picture, &line.id, &line.name, 36.))
            .child(text)
            .child(right)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.collection_flip(&collection, Some(&member));
                cx.notify();
            }))
    }
}
