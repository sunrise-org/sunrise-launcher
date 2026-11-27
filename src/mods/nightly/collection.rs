//! A source collection's page: pick mods to add, with one native consent for the selection.
use super::*;
use crate::mods_page::install::{stage_label, SourcePicks};
use crate::widgets::button::primary;
use crate::widgets::check::{checkbox, tick_of, Tick};
use crate::widgets::chip::{self, tag};
use std::collections::BTreeSet;

/// The open collection and the mods ticked on its page.
pub(in crate::mods) struct CollectionOpen {
    id: String,
    ticked: BTreeSet<String>,
}

impl Launcher {
    /// Source mods already added to this profile.
    fn added_sources(&self) -> BTreeSet<String> {
        self.discover_installed()
            .into_iter()
            .flat_map(|rows| &rows.local)
            .filter_map(|local| local.github.as_ref())
            .map(|origin| origin.id.clone())
            .collect()
    }

    fn add_source_collection(&mut self) {
        if !self.nightly_catalogue_current() || self.nightly_busy() {
            return;
        }
        let added = self.added_sources();
        let state = &self.mods_state.nightly;
        let (Some(open), Some(catalogue)) = (&state.collection, &state.catalogue) else {
            return;
        };
        let Some(collection) = catalogue.collections.iter().find(|c| c.id == open.id) else {
            return;
        };
        let picks: Vec<_> = collection
            .members
            .iter()
            .filter(|member| open.ticked.contains(&member.id) && !added.contains(&member.id))
            .filter_map(|member| catalogue.sources.iter().find(|s| s.id() == member.id))
            .cloned()
            .collect();
        if !picks.is_empty() {
            self.add_sources(SourcePicks::Collection {
                id: collection.id.clone(),
                name: collection.name.clone(),
                sources: picks,
            });
        }
    }

    pub(in crate::mods) fn source_collection(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let activity: Option<(Vec<String>, sunrise::InstallProgress)> =
            self.source_activity().map(|a| (a.ids.to_vec(), a.progress));
        let added = self.added_sources();
        let can_start = self.nightly_catalogue_current() && !self.nightly_busy();
        let state = &self.mods_state.nightly;
        let back = super::super::project::back(tr("Nightly mods")).on_click(cx.listener(
            |this, _, _, cx| {
                this.mods_state.nightly.collection = None;
                this.choose_view(Page::Discover);
                cx.notify();
            },
        ));
        let page = div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .p(px(24.))
            .gap(px(12.))
            .text_color(c(palette().text))
            .children(self.strip(cx));
        let found = state.collection.as_ref().and_then(|open| {
            let catalogue = state.catalogue.as_ref()?;
            let collection = catalogue
                .collections
                .iter()
                .find(|collection| collection.id == open.id)?
                .clone();
            let members: Vec<Source> = collection
                .members
                .iter()
                .filter_map(|member| {
                    catalogue
                        .sources
                        .iter()
                        .find(|source| source.id() == member.id)
                        .cloned()
                })
                .collect();
            Some((
                collection,
                members,
                open.ticked.clone(),
                catalogue.error.is_none(),
            ))
        });
        let Some((collection, members, ticked, allow_fetch)) = found else {
            return page
                .child(back)
                .child(muted(tr("This collection can't be shown.")))
                .into_any_element();
        };
        let icon = sources::SourceIcon::pages(&collection.icon, allow_fetch);
        let picture = state.work.icons.get(&icon).cloned().flatten();
        let load = (!state.work.icons.contains_key(&icon))
            .then(|| when_visible(cx, move |this| this.request_nightly_icon(icon)));
        let owners = collection.owners.join(", ");
        let head = div()
            .relative()
            .flex()
            .items_center()
            .gap(px(16.))
            .child(tile(picture.clone(), &collection.id, &collection.name, 64.))
            .children(load)
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
                            .child(tag(tr("Collection"), chip::Kind::Neutral)),
                    ),
            );

        // The list: a header that picks all, then one row per mod.
        let pickable: Vec<&Source> = members
            .iter()
            .filter(|source| !added.contains(source.id()))
            .collect();
        let picked = pickable
            .iter()
            .filter(|source| ticked.contains(source.id()))
            .count();
        let all_state = tick_of(picked, pickable.len());
        let all_ids: Vec<String> = pickable.iter().map(|s| s.id().to_owned()).collect();
        let list_head = div()
            .id("nightly-collection-all")
            .reach()
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(14.))
            .py(px(10.))
            .cursor_pointer()
            .text_size(px(tokens::LABEL))
            .text_color(c(palette().muted))
            .child(checkbox("nightly-collection-all-box", all_state).tab_stop(false))
            .child(trf(
                "{picked} of {all} selected",
                &[
                    ("picked", &picked.to_string()),
                    ("all", &pickable.len().to_string()),
                ],
            ))
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(open) = this.mods_state.nightly.collection.as_mut() {
                    if all_ids.iter().all(|id| open.ticked.contains(id)) {
                        open.ticked.clear();
                    } else {
                        open.ticked.extend(all_ids.iter().cloned());
                    }
                }
                cx.notify();
            }));
        let mut rows = card().child(list_head);
        for (i, source) in members.iter().enumerate() {
            let installed = added.contains(source.id());
            let on = installed || ticked.contains(source.id());
            let open = source.clone();
            let name = div()
                .id(("nightly-collection-name", i))
                .reach()
                .truncate()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(tokens::LABEL))
                .cursor_pointer()
                .hover(|s| s.text_color(c(palette().accent_ink)))
                .child(source.name().to_owned())
                // The row's own press ticks it: this one only opens the page.
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.mods_state.nightly.collection = None;
                    this.inspect_nightly(open.clone(), false);
                    cx.notify();
                }));
            // The installer's own result for this mod, as its page shows it.
            let running = activity
                .as_ref()
                .filter(|(ids, _)| ids.iter().any(|id| id == source.id()));
            let failed = self
                .source_outcome(source.id())
                .and_then(|outcome| outcome.as_ref().err())
                .filter(|_| !installed && running.is_none());
            let pending = self
                .discover_installed()
                .is_some_and(|rows| rows.unbuilt.contains(source.id()));
            let right = if installed && pending {
                Some(status("clock", tr(ADDED_PENDING), palette().warning))
            } else if installed {
                Some(status("check", tr("Installed"), palette().success))
            } else if let Some((_, progress)) = running {
                Some(status("hammer", stage_label(*progress), palette().muted))
            } else {
                failed.map(|_| status("information-circle", tr("Couldn’t add"), palette().warning))
            };
            // Why it was refused, in the one modal every refusal uses.
            let why = failed.and_then(|error| {
                let refused = crate::mods::refused::Refused::one(
                    source.name(),
                    Some(source.repo().to_owned()),
                    false,
                    error,
                )?;
                Some(div().pt(px(4.)).child(self.refused_line(
                    ("nightly-collection-refused", i),
                    refused,
                    cx,
                )))
            });
            let tick = checkbox(
                ("nightly-collection-box", i),
                if on { Tick::On } else { Tick::Off },
            )
            .tab_stop(false);
            let member = source.id().to_owned();
            rows = rows.child(
                div()
                    .id(("nightly-collection-row", i))
                    .reach()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(14.))
                    .py(px(10.))
                    .border_t_1()
                    .border_color(palette().rule())
                    .when(!installed, |d| d.cursor_pointer())
                    .child(match installed {
                        true => dim(tick),
                        false => tick,
                    })
                    .child(self.source_tile(source, None, allow_fetch, None, 36., cx))
                    .child(
                        div()
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
                                    .child(source.summary().to_owned()),
                            )
                            .children(why),
                    )
                    .children(right)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if installed {
                            return;
                        }
                        if let Some(open) = this.mods_state.nightly.collection.as_mut() {
                            if !open.ticked.remove(&member) {
                                open.ticked.insert(member.clone());
                            }
                        }
                        cx.notify();
                    })),
            );
        }

        // The side panel: one Add for the selected mods.
        let label = match picked {
            _ if pickable.is_empty() => tr("Installed").into(),
            1 => tr("Add 1 mod").into(),
            count => trf("Add {count} mods", &[("count", &count.to_string())]),
        };
        let action = primary("nightly-collection-build", label)
            .w_full()
            .h(px(tokens::ACTION));
        let action = if can_start && picked > 0 {
            action.on_click(cx.listener(|this, _, _, cx| {
                this.add_source_collection();
                cx.notify();
            }))
        } else {
            dim(action)
        };
        let right = card()
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(tile(picture, &collection.id, &collection.name, 40.))
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
            .child(muted(tr(
                "Each mod builds from its own repository on this computer. You're asked once, for all of them.",
            )))
            .child(muted(tr(
                "Adds source code from GitHub. Native mods build when you play.",
            )));
        let (above, beside) = match self.content_width() < 860. {
            true => (Some(right), None),
            false => (None, Some(right.w(px(360.)).flex_none())),
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
            .child(super::super::project::prose(self.theme, &about_text))
            .child(
                div()
                    .text_size(px(tokens::HEADING))
                    .font_weight(FontWeight::BOLD)
                    .child(tr("Mods")),
            )
            .child(rows);
        page.child(crate::scrollbar::wrap_page_in_margin(
            "nightly-collection",
            div()
                .id("nightly-collection-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(
                    div().flex().flex_col().gap(px(14.)).child(back).child(
                        div()
                            .flex()
                            .items_start()
                            .gap(px(24.))
                            .child(left)
                            .children(beside),
                    ),
                ),
            &self.theme,
        ))
        .into_any_element()
    }
}
