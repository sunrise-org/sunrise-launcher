//! The tabs' Add file button and their empty states. On the Mods tab a file is a `.smod`, read
//! and installed through the Add from source sheet (so the unreviewed-code warning comes
//! first); on the Packs and Shaders tabs it is a pack, copied into the profile's pack folder.
use super::*;
use crate::widgets::button::{button, Look, Size, Style};

impl Launcher {
    /// The header's action: a compact secondary with a plus.
    pub(super) fn add_file_button(
        &self,
        collection: Collection,
        id: &'static str,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        button(
            id,
            tr("Add file"),
            Look::new(Style::Secondary, Size::Medium).icon("plus-sign"),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            match collection.pack_kind() {
                Some(kind) => this.add_pack_files(kind, cx),
                None => this.add_smod_file(cx),
            }
            cx.notify();
        }))
    }

    /// A tab with nothing in it: what it is for, with Add file and Find beside each other.
    pub(super) fn home_installed_empty(
        &mut self,
        collection: Collection,
        cx: &mut Context<Self>,
    ) -> Div {
        let add = self.add_file_button(
            collection,
            match collection {
                Collection::Mods => "home-mods-empty-add",
                Collection::Resource => "home-packs-empty-add",
                Collection::Shader => "home-shaders-empty-add",
            },
            cx,
        );
        let find = secondary(
            match collection {
                Collection::Resource => "home-find-packs",
                Collection::Shader => "home-find-shaders",
                Collection::Mods => "home-browse-mods",
            },
            match collection {
                Collection::Resource => tr("Find packs"),
                Collection::Shader => tr("Find shader packs"),
                Collection::Mods => tr("Find mods"),
            },
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            this.home_open_discover(collection);
            cx.notify();
        }));
        let (title, line) = match collection {
            Collection::Resource => (
                tr("No resource packs yet"),
                tr("Resource packs you add to this profile show up here."),
            ),
            Collection::Shader => (
                tr("No shaders yet"),
                tr("Shader packs you add to this profile show up here."),
            ),
            Collection::Mods => (
                tr("No mods yet"),
                tr("Mods you add to this profile show up here."),
            ),
        };
        crate::widgets::feedback::empty_state(
            title,
            line,
            Some(
                div()
                    .id("home-installed-empty-actions")
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(add)
                    .child(find)
                    .into_any_element(),
            ),
        )
    }
}
