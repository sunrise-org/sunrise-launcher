use super::*;
use crate::mods::selection::bar;
use crate::widgets::button::{button, icon_button, icon_button_opening, Look, Size, Style};
use crate::widgets::focus::c;
use std::{cell::Cell, rc::Rc};
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
        let query = self.ui.home.tabs.query_of(Tab::Worlds);
        let found: Vec<World> = self
            .worlds
            .list
            .iter()
            .filter(|world| search::matches(&query, &[world.name.as_str(), world.folder.as_str()]))
            .cloned()
            .collect();
        // A world that is gone (deleted, or the game removed it) drops out of the ticks.
        let every: Vec<String> = self
            .worlds
            .list
            .iter()
            .map(|world| world.folder.clone())
            .collect();
        if self.worlds.fresh || !every.is_empty() {
            self.home_worlds_pick().retain(&every);
        }
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
        let (built, place) = view
            .near
            .list(&view.scroll, found.len(), across, height, gap);
        let items = if count == Some(0) {
            // A world copied into the folder shows up here.
            let saves = instance.home.join("saves");
            let open =
                // An outlined button, as every other empty state has.
                button(
                    "home-worlds-folder",
                    tr("Open worlds folder"),
                    Look::new(Style::Secondary, Size::Medium),
                )
                .on_click(move |_, _, cx| {
                    match std::fs::create_dir_all(&saves) {
                        Ok(()) => crate::toast::open(cx, &saves),
                        Err(e) => crate::toast::say(format!("{}: {e}", saves.display())),
                    }
                });
            // Add world beside Open worlds folder.
            let add = self.add_world_button("home-worlds-empty-add", cx);
            empty(
                tr("No worlds yet"),
                tr("Start the game and choose Singleplayer to make one, or copy a world into its folder."),
                Some(
                    div()
                        .id("home-worlds-empty-actions")
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(add)
                        .child(open),
                ),
            )
        } else if self.worlds.list.is_empty() {
            div()
        } else if found.is_empty() {
            search::no_matches()
        } else {
            let worlds = found[built].to_vec();
            let order: Rc<Vec<String>> =
                Rc::new(found.iter().map(|world| world.folder.clone()).collect());
            let ticks = self.home_worlds_pick().clone();
            let pick_bar = self.world_pick_bar(&ticks, &order, cx);
            self.home_set_bar(pick_bar);
            let mut rows = div().flex().gap(px(gap));
            if tiles {
                rows = rows.flex_wrap();
                for world in &worlds {
                    rows = rows.child(self.world_tile(world, window, cx));
                }
            } else {
                rows = rows.flex_col();
                for world in &worlds {
                    rows = rows.child(self.world_list_row(world, &ticks, &order, window, cx));
                }
            }
            place(rows)
        };
        // The header's action: Add world, once there are worlds (the empty state has its own).
        let manage = (count.unwrap_or(0) > 0).then(|| self.add_world_button("home-worlds-add", cx));
        Content {
            count,
            manage,
            items,
        }
    }

    /// The folders of the worlds Ctrl+A ticks: those the search leaves, in the list layout (the
    /// tiles have no tick boxes).
    pub(crate) fn home_world_ids(&self) -> Vec<String> {
        if self.settings.world_layout == backend::WorldLayout::Tiles {
            return Vec::new();
        }
        let query = self.ui.home.tabs.query_of(Tab::Worlds);
        self.worlds
            .list
            .iter()
            .filter(|world| search::matches(&query, &[world.name.as_str(), world.folder.as_str()]))
            .map(|world| world.folder.clone())
            .collect()
    }

    /// The bar under the list while worlds are ticked: how many, Select all / Clear, and
    /// Duplicate and Delete, which wait while the game runs from the profile.
    fn world_pick_bar(
        &mut self,
        ticks: &Selection,
        order: &Rc<Vec<String>>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if ticks.is_empty() {
            return None;
        }
        let toggle = bar::link(
            "home-worlds-pick-all",
            if ticks.all_of(order) {
                tr("Clear")
            } else {
                tr("Select all")
            },
        )
        .on_click({
            let order = order.clone();
            cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.home_worlds_pick().toggle_all(&order);
                cx.notify();
            })
        });
        let locked = self.world_lock();
        let usable = locked.is_none();
        let copy = bar::action("home-worlds-copy", tr("Duplicate"), usable, false);
        let delete = bar::action("home-worlds-delete", tr("Delete…"), usable, true);
        let (copy, delete) = match locked {
            Some(why) => (
                copy.tooltip(crate::glass_ui::tip(why.clone())),
                delete.tooltip(crate::glass_ui::tip(why)),
            ),
            None => (
                copy.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.copy_picked_worlds(cx);
                    cx.notify();
                })),
                delete.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.ask_delete_picked_worlds();
                    cx.notify();
                })),
            ),
        };
        Some(bar::bar(
            ticks.len(),
            toggle,
            vec![copy.into_any_element(), delete.into_any_element()],
        ))
    }

    /// One world as a row, in fixed columns: its tick box, its picture, its name over what it is (mode, and
    /// when it was last played), then a Play icon button and a "…" menu (data packs, folder,
    /// copy, rename, delete). A long name is cut, with all of it in a tooltip, and in the row
    /// while the keyboard is on Play.
    fn world_list_row(
        &mut self,
        world: &World,
        ticks: &Selection,
        order: &Rc<Vec<String>>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let folder = world.folder.clone();
        let focus = self
            .worlds
            .tile_focus
            .entry(world.folder.clone())
            .or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone();
        let on_play = focus.is_focused(window);
        let play = icon_button(
            SharedString::from(format!("alt-world-play-{}", world.folder)),
            "play",
            trf("Play {world}", &[("world", &world.name)]),
            Style::Secondary,
        )
        .track_focus(&focus)
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            cx.stop_propagation();
            this.play_to("--world", folder.clone());
            cx.notify();
        }));
        // Where the "…" was drawn, for the menu that hangs from it.
        let drawn = Rc::new(Cell::new(gpui::Bounds::<gpui::Pixels>::default()));
        let (write, read) = (drawn.clone(), drawn);
        let menu_of = world.clone();
        let more = icon_button_opening(
            SharedString::from(format!("alt-world-more-{}", world.folder)),
            "more-horizontal",
            tr("More actions"),
            Style::Ghost,
            self.world_menu_open(&world.folder),
        )
        // A step brighter than the row's own light, so it shows on a lit row.
        .hover(|s| s.bg(rgb(palette().selected)))
        .child(
            gpui::canvas(move |bounds, _, _| write.set(bounds), |_, _, _, _| {})
                .absolute()
                .inset_0(),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            cx.stop_propagation();
            this.open_world_menu(&menu_of, read.get());
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
        let id = SharedString::from(format!("alt-world:{}", world.folder));
        let (ticked, any) = (ticks.contains(&world.folder), !ticks.is_empty());
        let check = self.tick_column(&id, ticked, any, {
            let (key, order) = (world.folder.clone(), order.clone());
            cx.listener(move |this, ev: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.home_worlds_pick()
                    .click(&order, &key, ev.modifiers().shift);
                cx.notify();
            })
        });
        let lit = id.clone();
        let (key, order) = (world.folder.clone(), order.clone());
        let row = card()
            .id(id)
            // Ctrl or Shift ticks the row; once any is ticked, a plain press does too.
            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                let gesture = Gesture::of(&ev.modifiers());
                if gesture.selects() || any {
                    this.home_worlds_pick().click(&order, &key, gesture.range);
                    cx.notify();
                }
            }))
            .when(ticked, |d| d.bg(rgb(palette().selected)))
            .flex_none()
            .h(px(WORLD_ROW))
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(12.))
            .child(check)
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
                    .gap(px(6.))
                    .child(play)
                    .child(more),
            );
        Self::watch_hover(row, &lit, cx)
    }
}
