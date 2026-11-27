use super::*;
use crate::widgets::button::{button, secondary, Look, Size, Style};
use crate::widgets::focus::c;
use sunrise_launcher::worlds::World;

impl Launcher {
    pub(super) fn home_worlds(
        &mut self,
        instance: &instances::Instance,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Content {
        self.load_worlds(instance.home.clone(), cx);
        let count = self
            .worlds
            .fresh
            .then_some(self.worlds.list.len())
            .or_else(|| (!self.worlds.list.is_empty()).then_some(self.worlds.list.len()));
        let view = &self.ui.home.tabs.views[Tab::Worlds.index()];
        let tiles = self.settings.world_layout == backend::WorldLayout::Tiles;
        let across = if tiles {
            view.near.width().map_or(1, |wide| {
                ((wide + TILE_GAP) / (WORLD_TILE + TILE_GAP)).max(1.) as usize
            })
        } else {
            1
        };
        let (height, gap) = if tiles {
            (TILE_TALL, TILE_GAP)
        } else {
            (WORLD_ROW, WORLD_ROW_GAP)
        };
        let (built, place) =
            view.near
                .list(&view.scroll, self.worlds.list.len(), across, height, gap);
        let items = if count == Some(0) {
            // A world copied into the folder shows up here.
            let saves = instance.home.join("saves");
            let open =
                // Figma: a plain text link under the empty state's words.
                button(
                    "home-worlds-folder",
                    tr("Open worlds folder"),
                    Look::new(Style::Ghost, Size::Medium),
                )
                .on_click(move |_, _, cx| {
                    match std::fs::create_dir_all(&saves) {
                        Ok(()) => crate::toast::open(cx, &saves),
                        Err(e) => crate::toast::say(format!("{}: {e}", saves.display())),
                    }
                });
            empty(
                tr("No worlds yet"),
                tr("Start the game and choose Singleplayer to make one, or copy a world into its folder."),
                Some(open),
            )
        } else if self.worlds.list.is_empty() {
            div()
        } else {
            let worlds = self.worlds.list[built].to_vec();
            let mut rows = div().flex().gap(px(gap));
            if tiles {
                rows = rows.flex_wrap();
                for world in &worlds {
                    rows = rows.child(self.world_tile(world, window, cx));
                }
            } else {
                rows = rows.flex_col();
                for world in &worlds {
                    rows = rows.child(self.world_list_row(world, window, cx));
                }
            }
            place(rows)
        };
        Content {
            count,
            manage: None,
            turned_off: 0,
            items,
        }
    }

    /// One world as a row, in fixed columns: its picture, its name over what it is (mode, and
    /// when it was last played), then its data packs, folder and play buttons. A long name is
    /// cut, with all of it in a tooltip, and in the row while the keyboard is on Play.
    fn world_list_row(
        &mut self,
        world: &World,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let (folder, path, saves) = (world.folder.clone(), world.path.clone(), world.path.clone());
        let focus = self
            .worlds
            .tile_focus
            .entry(world.folder.clone())
            .or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone();
        let on_play = focus.is_focused(window);
        let packs = secondary("alt-world-packs", tr("Data packs")).on_click(cx.listener(
            move |this, _: &ClickEvent, window, cx| {
                this.ui.home.tabs.remember(window.focused(cx));
                this.show_world_packs(path.clone());
                cx.notify();
            },
        ));
        let folder_button = secondary("alt-world-folder", tr("Open folder")).on_click(cx.listener(
            move |_, _: &ClickEvent, _, cx| {
                crate::toast::open(cx, &saves);
                cx.notify();
            },
        ));
        let play = secondary("alt-world-play", tr("Play this world"))
            .track_focus(&focus)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.play_to("--world", folder.clone());
                cx.notify();
            }));
        let name = div()
            .id("alt-world-name")
            .text_size(px(tokens::LABEL))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(c(palette().text))
            .when(!on_play, |d| d.truncate())
            .child(world.name.clone())
            .tooltip(crate::glass_ui::tip(world.name.clone()));
        card()
            .id(SharedString::from(format!("alt-world:{}", world.folder)))
            .flex_none()
            .h(px(WORLD_ROW))
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(12.))
            .child(self.world_icon(world, WORLD_ICON))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(name)
                    .child(crate::home::muted(Self::world_facts(world)).truncate()),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .child(packs)
                    .child(folder_button)
                    .child(play),
            )
    }
}
