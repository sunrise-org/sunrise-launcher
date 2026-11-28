//! What a card on the Mods or Packs tab opens. A registry mod, and a local or from-source mod,
//! open their own page (`mods`): the page a mod has in Discover, or the installed row's page.
//! A pack has no page of its own in the registry, so its card opens this sheet: what it is,
//! where it lies, and what can be done to it.
use super::*;
use crate::widgets::button::{danger, primary, secondary};

impl Launcher {
    /// Opens what a press on a card opens.
    pub(super) fn open_home_item(&mut self, open: Open) {
        match open {
            Open::Project(id) => self.mods_show_project(id),
            Open::Installed(id) => {
                if !self.open_mod_named(&id) {
                    crate::toast::say(tr(
                        "This mod's page isn't ready yet. Try again in a moment.",
                    ));
                }
            }
            Open::Pack(_, file) => self.open_pack_details(&file),
        }
    }

    /// Opens the sheet of the resource pack that is this file.
    pub(crate) fn open_pack_details(&mut self, file: &str) {
        let Some(item) = self.home_item(true, file) else {
            return;
        };
        self.ui.home.tabs.pack_details = Some(item);
        self.open_sheet(crate::sheets::Sheet::PackDetails);
    }

    /// The search's pack: Home's Packs tab, with the pack's sheet open when it is listed.
    pub(crate) fn open_pack_named(&mut self, kind: PackKind, title: String) {
        if kind == PackKind::Shader {
            self.open_installed(crate::mods::Installed::PackNamed(kind, title));
            return;
        }
        self.show_home_tab(true);
        let file = self.ui.home.mods.snapshot.as_ref().and_then(|snapshot| {
            snapshot
                .tabs
                .resources
                .iter()
                .find(|item| item.name == title || item.id == title)
                .map(|item| item.id.clone())
        });
        if let Some(file) = file {
            self.open_pack_details(&file);
        }
    }

    /// The pack's sheet: its picture and name, whether it is on, its file and where it lies,
    /// with Show in folder, the switch and Remove. None when the pack is gone (a change
    /// elsewhere removed it).
    pub(crate) fn pack_details_sheet(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let shown = self.ui.home.tabs.pack_details.clone()?;
        let item = self.home_item(true, &shown.id)?;
        let home = self.home_item_home()?;
        let path = home.join(PackKind::Resource.folder()).join(&item.id);
        let p = crate::paint::palette();
        let row = |key: &'static str, value: String, mono: bool| {
            div()
                .flex()
                .gap(px(12.))
                .py(px(4.))
                .text_size(px(crate::tokens::LABEL))
                .child(
                    div()
                        .w(px(96.))
                        .flex_none()
                        .text_color(gpui::rgb(p.muted))
                        .child(key),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .when(mono, |d| d.font_family(crate::theme::MONO))
                        .child(crate::selectable::selectable(value)),
                )
        };
        let picture = self.home_item_picture(&item, cx);
        let head = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(picture)
            .child(
                div()
                    .min_w_0()
                    .child(crate::sheets::title(item.name.clone())),
            );
        let state = if item.on { tr("On") } else { tr("Off") };
        let actions = self.pack_actions(&item, path.clone(), cx);
        let body = div()
            .flex()
            .flex_col()
            .child(head)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(row(tr("Type"), tr("Resource pack").to_owned(), false))
                    .child(row(tr("State"), state.to_owned(), false))
                    .child(row(tr("File"), item.id.clone(), true))
                    .child(row(tr("Location"), path.display().to_string(), true)),
            )
            .child(actions);
        Some(crate::sheets::frame("home-pack-details", 560., body, cx))
    }

    /// The row of buttons under a pack's sheet: Show in folder, the switch, Remove and Close.
    fn pack_actions(&mut self, item: &Item, path: PathBuf, cx: &mut Context<Self>) -> Div {
        let key = item.id.clone();
        let toggle_key = key.clone();
        let remove_key = key.clone();
        let reveal = path;
        let locked = self.mods_locked();
        let dim = |button: Stateful<Div>| match locked {
            Some(why) => crate::widgets::disabled(button).tooltip(crate::glass_ui::tip(why)),
            None => button,
        };
        let toggle = dim(secondary(
            "home-pack-toggle",
            if item.on { tr("Disable") } else { tr("Enable") },
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.home_item_toggle(true, &toggle_key, cx);
            this.close_sheet();
            cx.notify();
        })));
        let remove = dim(
            danger("home-pack-remove", tr("Remove…")).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    this.close_sheet();
                    this.home_pack_remove(&remove_key);
                    cx.notify();
                },
            )),
        );
        let actions = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                secondary("home-pack-folder", tr("Show in folder")).on_click(move |_, _, cx| {
                    cx.reveal_path(&reveal);
                }),
            )
            .child(toggle)
            .child(remove)
            .child(div().flex_1())
            .child(
                primary("home-pack-close", tr("Close")).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.close_sheet();
                        cx.notify();
                    },
                )),
            );
        actions
    }
}
