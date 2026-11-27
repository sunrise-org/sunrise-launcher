//! The tabs' Add file button and their empty states. On the Mods tab a file is a `.smod`, read
//! and installed through the Add from source sheet (so the unreviewed-code warning comes
//! first); on the Packs tab it is a pack, copied into the profile's pack folder.
use super::*;
use crate::widgets::button::{button, Look, Size, Style};

impl Launcher {
    /// The header's action: a compact secondary with a plus.
    pub(super) fn add_file_button(
        &self,
        packs: bool,
        id: &'static str,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        button(
            id,
            tr("Add file"),
            Look::new(Style::Secondary, Size::Medium).icon("plus-sign"),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            if packs {
                this.add_pack_files(cx);
            } else {
                this.add_smod_file(cx);
            }
            cx.notify();
        }))
    }

    /// A tab with nothing in it: what it is for, with Add file and Find beside each other.
    pub(super) fn home_installed_empty(&mut self, packs: bool, cx: &mut Context<Self>) -> Div {
        let add = self.add_file_button(
            packs,
            if packs {
                "home-packs-empty-add"
            } else {
                "home-mods-empty-add"
            },
            cx,
        );
        let find = if packs {
            secondary("home-find-packs", tr("Find packs")).on_click(cx.listener(
                |this, _, _, cx| {
                    this.show_resource_pack_discover();
                    cx.notify();
                },
            ))
        } else {
            secondary("home-browse-mods", tr("Find mods")).on_click(cx.listener(
                |this, _, _, cx| {
                    this.show_discover();
                    cx.notify();
                },
            ))
        };
        let (title, line) = if packs {
            (
                tr("No resource packs yet"),
                tr("Resource packs you add to this profile show up here."),
            )
        } else {
            (
                tr("No mods yet"),
                tr("Mods you add to this profile show up here."),
            )
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
