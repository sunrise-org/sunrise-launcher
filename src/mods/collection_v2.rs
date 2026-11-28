//! A registry collection's page (Mods › Discover › a collection): its publisher's mods, each
//! picked or not as the collection says, and one Install for the picked ones that fit. Each mod
//! installs through the ordinary registry install and is its own afterwards; the collection owns
//! nothing.
use super::model::discover_v2::{V2Card, V2Collection};
use super::project::look::{action_column, card, head, muted, section};
use super::project::prose;
use super::ui::*;
use super::*;
use crate::paint::palette;
use crate::widgets::button::primary;
use crate::widgets::check::{checkbox, Tick};
use crate::widgets::chip::{self, tag};
use sunrise_launcher::browser::images;

/// The picks made on the open collection's page.
pub(super) struct V2Picks {
    id: String,
    picked: BTreeSet<String>,
}

type Shown = (Rc<[V2Card]>, V2Collection, BTreeSet<String>);

impl Launcher {
    /// Opens a registry collection's page; Back returns to where it was opened from.
    pub(super) fn mods_show_v2_collection(&mut self, id: String, from: Page) {
        self.mods_state.nightly.open = None;
        self.mods_state.nightly.collection = None;
        crate::frame::scrolls("project").set_offset(gpui::point(px(0.), px(0.)));
        self.mods_state.page_request += 1;
        self.mods_state.open = Some(Open {
            id,
            installed: None,
            collection: true,
            v2: true,
            request: self.mods_state.page_request,
            scope: self.mods_page_scope(),
            from,
            v2_page: None,
            parent: None,
            removing: Vec::new(),
        });
        self.ui.enter_details();
        self.ui.page = Page::Project;
    }

    /// The open collection with its members' cards and the picks made so far.
    fn v2_collection_view(&mut self, id: &str) -> Option<Shown> {
        let V2Catalog::Ready {
            cards, collections, ..
        } = &self.mods_state.v2_catalog
        else {
            return None;
        };
        let collection = collections.iter().find(|c| c.id == id)?.clone();
        let cards = cards.clone();
        let slot = &mut self.mods_state.v2_picks;
        if slot.as_ref().is_none_or(|p| p.id != id) {
            *slot = Some(V2Picks {
                id: id.to_owned(),
                picked: collection.initial_picks(&cards),
            });
        }
        let picked = slot.as_ref()?.picked.clone();
        Some((cards, collection, picked))
    }

    fn v2_collection_flip(&mut self, member: &str) {
        if let Some(picks) = self.mods_state.v2_picks.as_mut() {
            if !picks.picked.remove(member) {
                picks.picked.insert(member.to_owned());
            }
        }
    }

    fn v2_installed(&self, project: &str) -> bool {
        self.discover_installed()
            .is_some_and(|rows| rows.registry_entry(project).is_some())
    }

    pub(super) fn v2_collection_page(
        &mut self,
        header: Div,
        page: Div,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(id) = self.mods_state.open.as_ref().map(|o| o.id.clone()) else {
            return page.child(header).into_any_element();
        };
        let Some((cards, collection, picked)) = self.v2_collection_view(&id) else {
            return page
                .child(header)
                .child(muted(tr("This collection can't be shown.")))
                .into_any_element();
        };
        let icon = collection
            .icon
            .as_deref()
            .and_then(|rel| self.v2_mod_image(&id, rel, images::ICON));
        let publisher = trf("by {authors}", &[("authors", &collection.publisher)]);
        let facts = vec![trf(
            "Collection · {count} mods",
            &[("count", &collection.members.len().to_string())],
        )];
        let header_block = head(
            tile(icon, &collection.id, &collection.name, 64.).into_any_element(),
            &collection.name,
            Some(publisher),
            vec![tag(tr("Collection"), chip::Kind::Neutral).into_any_element()],
            Some(collection.summary.clone()).filter(|s| !s.trim().is_empty()),
            facts,
        );

        let to_install =
            collection.to_install(&cards, &picked, &|project| self.v2_installed(project));
        let picked_count = collection
            .members
            .iter()
            .filter(|&&(i, _)| picked.contains(&cards[i].id))
            .count();
        let mut rows = card();
        for (n, &(i, _)) in collection.members.iter().enumerate() {
            rows = rows.child(self.v2_collection_line(&id, n, &cards[i], &picked, cx));
        }

        let working = self.registry_card_control(&id, |label| {
            dim(primary("alt-collection-install", label)
                .w_full()
                .h(px(tokens::ACTION)))
            .into_any_element()
        });
        let action = match working {
            Some(progress) => progress,
            None => {
                let label = match to_install.len() {
                    1 => tr("Install 1 mod").to_owned(),
                    count => trf("Install {count} mods", &[("count", &count.to_string())]),
                };
                let button = primary("alt-collection-install", label)
                    .w_full()
                    .h(px(tokens::ACTION));
                if to_install.is_empty() || self.mods_locked().is_some() {
                    dim(button).into_any_element()
                } else {
                    let (card, ids) = (id.clone(), to_install.clone());
                    button
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.add_v2_collection(&card, ids.clone());
                            cx.notify();
                        }))
                        .into_any_element()
                }
            }
        };
        let action_col = action_column()
            .child(action)
            .child(muted(trf(
                "{picked} of {all} selected",
                &[
                    ("picked", &picked_count.to_string()),
                    ("all", &collection.members.len().to_string()),
                ],
            )))
            .children(self.registry_card_outcome(&id, cx));

        let side = card()
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(muted(tr(
                "The picked mods and what they need install together. Each one then updates and is removed on its own, from Installed.",
            )));

        let mut body: Vec<AnyElement> = Vec::new();
        if !collection.description.trim().is_empty() {
            body.push(prose(self.theme, &collection.description));
        }
        body.push(section(tr("Mods")).into_any_element());
        body.push(rows.into_any_element());
        self.project_layout(
            "project",
            super::project::Layout {
                header,
                head: header_block,
                action: Some(action_col),
                gallery: None,
                side: Some(side),
            },
            body,
        )
    }

    /// One member: its box picks it, its name opens its own page. A member that doesn't fit the
    /// build, or is already installed, can't be picked. The status and version sit in the same
    /// fixed columns as the profile's Mods tab.
    fn v2_collection_line(
        &mut self,
        collection: &str,
        n: usize,
        member: &V2Card,
        picked: &BTreeSet<String>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let installed = self.v2_installed(&member.id);
        let fits = member.fit.fits();
        let open = !installed && fits;
        let picture = member
            .icon
            .as_deref()
            .and_then(|rel| self.v2_mod_image(&member.id, rel, images::ROW));
        let name = {
            let (project, parent) = (member.id.clone(), collection.to_owned());
            div()
                .id(("alt-collection-name", n))
                .reach()
                .truncate()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(tokens::LABEL))
                .cursor_pointer()
                .hover(|s| s.text_color(c(palette().accent_ink)))
                .child(member.name.clone())
                // The row's own press picks it: this one only opens the page.
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.mods_show_v2_project(project.clone());
                    if let Some(open) = this.mods_state.open.as_mut() {
                        open.parent = Some(Parent::Collection(parent.clone()));
                    }
                    cx.notify();
                }))
        };
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
                    .child(member.summary.clone()),
            );
        if let Some(why) = member.fit.reason() {
            text = text.child(div().truncate().child(muted(why)));
        }
        let state = if installed && self.registry_pending(&member.id) {
            Some(status("clock", tr(ADDED_PENDING), palette().warning))
        } else if installed {
            Some(status("check", tr("Installed"), palette().success))
        } else {
            None
        };
        let version = (!installed)
            .then(|| member.fit.fitting_version().map(str::to_owned))
            .flatten();
        let tick = checkbox(
            ("alt-collection-box", n),
            if open && picked.contains(&member.id) {
                Tick::On
            } else {
                Tick::Off
            },
        )
        .tab_stop(false);
        let project = member.id.clone();
        div()
            .id(("alt-collection-row", n))
            .reach()
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(14.))
            .py(px(10.))
            .min_h(px(52.))
            .border_t_1()
            .border_color(palette().rule())
            .when(open, |d| d.cursor_pointer())
            .child(match open {
                true => tick,
                false => dim(tick),
            })
            .child(tile(picture, &member.id, &member.name, 36.))
            .child(text)
            .child(
                div()
                    .w(px(model::column::STATUS))
                    .flex_none()
                    .flex()
                    .items_center()
                    .children(state),
            )
            .child(
                div()
                    .w(px(model::column::VERSION))
                    .flex_none()
                    .truncate()
                    .font_family(crate::theme::MONO)
                    .text_size(px(tokens::LABEL))
                    .text_color(c(palette().muted))
                    .children(version),
            )
            .when(open, |d| {
                d.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.v2_collection_flip(&project);
                    cx.notify();
                }))
            })
    }

    fn registry_pending(&self, project: &str) -> bool {
        self.discover_installed()
            .is_some_and(|rows| rows.registry_pending(project))
    }
}
