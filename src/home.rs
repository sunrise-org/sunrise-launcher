//! The selected profile, its launch controls, and four browsable collections.
use super::*;
use crate::frame::Reach as _;
use crate::paint::palette;
use crate::worlds_page;
use gpui::rgb;
use std::cell::Cell;
use std::rc::Rc;
use sunrise_launcher::worlds::World;

pub(crate) mod tabs;

mod banner;

/// A world's picture in a row.
const WORLD_ICON: f32 = 40.;
/// A world's card row, and the room between two: taller than the other tabs' rows, for the
/// picture and the two lines beside it.
const WORLD_ROW: f32 = 52.;
const WORLD_ROW_GAP: f32 = 4.;
/// And as a tile: its 64 pixels, doubled.
const WORLD_TILE: f32 = 128.;
/// How far the tabs and their content stand in from the panel's edge: as far as the banner's
/// own padding insets the profile (Figma: 24).
const INSET: f32 = 24.;

/// What stands in the profile banner.
pub(crate) enum Art {
    /// The design's own art; the banner washes it with the panel where its text stands.
    Fading(Div),
    /// A scene or a player's picture: the banner washes it too, and veils it a little.
    Scene(Div),
}

#[derive(Default)]
pub(crate) struct HomeState {
    /// Where the build picker was last drawn: the build chooser opens under it.
    picker: Rc<Cell<Option<Bounds<gpui::Pixels>>>>,
    /// Where the profile menu's button was last drawn: the menu opens under it.
    menu: Rc<Cell<Option<Bounds<gpui::Pixels>>>>,
    /// How far the profile banner's drawn left edge sits from where the sidebar and the frame's
    /// margin put it, in px (see `left_edge`). Added to that reckoned edge it gives the banner's
    /// left in the window, so a header on the window's middle holds still while the sidebar
    /// folds.
    pub(crate) banner_left: Rc<Cell<Option<f32>>>,
    /// The current frame's unboxed profile, measured before the backdrop is painted.
    hero: Rc<Cell<Option<Bounds<gpui::Pixels>>>>,
    /// The art the frame drew for this frame's banner, taken by the banner.
    pub(crate) art: Option<Art>,
    /// What the selected profile has installed.
    pub(crate) mods: crate::home_mods::Inventory,
    pub(crate) tabs: tabs::Tabs,
}

/// A row in any profile tab.
pub(crate) const ROW: f32 = 40.;
/// A world's tile with its two lines, and the room between tiles.
const TILE_TALL: f32 = 180.;
const TILE_GAP: f32 = 16.;

/// Which of `count` items to build, `across` to a band and the bands `pitch` apart, when the
/// page shows the list from `from` to `to` (measured down from the list's top): the bands in
/// view and a few on either side, so the next Tab stop and the next turn of the wheel are there.
pub(crate) fn near(
    count: usize,
    across: usize,
    pitch: f32,
    (from, to): (f32, f32),
) -> std::ops::Range<usize> {
    const SPARE: f32 = 3.;
    let band = |y: f32| (y / pitch).max(0.) as usize;
    let first = band(from - SPARE * pitch) * across;
    let last = band(to + SPARE * pitch + pitch) * across;
    first.min(count)..last.min(count)
}

/// Where a long list of Home was last drawn. All of its items stay reachable (the list is as
/// tall as all of them) while only those near the view are built.
#[derive(Clone, Default)]
struct Near(Rc<Cell<Option<Bounds<gpui::Pixels>>>>);

impl Near {
    /// How wide the list was last drawn.
    fn width(&self) -> Option<f32> {
        self.0.get().map(|at| f32::from(at.size.width))
    }

    /// The items to build of `count`, and the list they stand in: `items` is given the built
    /// ones' place in `list`, between two blanks as tall as the ones left out.
    fn list(
        &self,
        scroll: &gpui::ScrollHandle,
        count: usize,
        across: usize,
        tall: f32,
        gap: f32,
    ) -> (std::ops::Range<usize>, impl FnOnce(Div) -> Div + use<>) {
        let pitch = tall + gap;
        let height = f32::from(scroll.bounds().size.height);
        let from = f32::from(-scroll.offset().y);
        let built = if height > 0. {
            near(count, across, pitch, (from, from + height))
        } else {
            // Not drawn yet: a first screenful, measured by the next frame.
            0..count.min(across * 12)
        };
        let bands = |items: usize| items.div_ceil(across) as f32;
        let blank =
            |items: usize| (items > 0).then(|| div().flex_none().h(px(bands(items) * pitch - gap)));
        let (above, below) = (blank(built.start), blank(count - built.end));
        let at = self.0.clone();
        let list = move |items: Div| {
            div()
                .relative()
                .flex_none()
                .flex()
                .flex_col()
                .gap(px(gap))
                .children(above)
                .child(items.flex_none())
                .children(below)
                .child(
                    gpui::canvas(
                        move |bounds, window, cx| {
                            if at.replace(Some(bounds)) != Some(bounds) {
                                // Measure the viewport on first paint and when it resizes.
                                let window = window.window_handle();
                                cx.defer(move |cx| {
                                    window.update(cx, |_, window, _| window.refresh()).ok();
                                });
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .inset_0(),
                )
        };
        (built, list)
    }
}

impl HomeState {
    /// Measure the uncovered text, not the full width of the header or the controls.
    fn shade_behind(&self, text: Div, on: bool) -> Div {
        text.when(on, |d| {
            let hero = self.hero.clone();
            d.relative().child(
                gpui::canvas(
                    move |bounds, _, _| {
                        if !bounds.is_empty() {
                            hero.set(Some(hero.get().map_or(bounds, |old| old.union(&bounds))));
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
        })
    }

    /// Where the build chooser opens: the left edge and the top, in the window.
    pub(crate) fn chooser_at(&self) -> (f32, f32) {
        match self.picker.get() {
            Some(picker) => (f32::from(picker.left()), f32::from(picker.bottom()) + 6.),
            // Home was not drawn yet; it is by the next frame.
            None => (INSET, 96.),
        }
    }

    /// Where the profile menu opens: the left edge and the top, in the window.
    pub(crate) fn menu_at(&self) -> (f32, f32) {
        match self.menu.get() {
            Some(button) => (f32::from(button.left()), f32::from(button.bottom()) + 6.),
            None => (INSET, 96.),
        }
    }
}

pub(crate) fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

pub(crate) fn card() -> Div {
    div()
        .bg(palette().card())
        .border_1()
        .border_color(palette().rule())
        .rounded(crate::tokens::r(crate::tokens::R_CARD))
        .overflow_hidden()
}

pub(crate) fn heading(label: &str) -> Div {
    div()
        .text_size(px(tokens::HEADING))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(palette().text))
        .child(crate::selectable::selectable(label.to_string()))
}

pub(crate) fn muted(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(tokens::SMALL))
        .text_color(rgb(palette().muted))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(text.into())
}

impl Launcher {
    pub(crate) fn home(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.ui.home.hero.set(None);
        self.refresh_home_inventory(cx);
        let Some(instance) = self.selected_instance().cloned() else {
            return div()
                .p(px(32.))
                .text_color(rgb(palette().muted))
                .child(tr("Getting ready…"))
                .into_any_element();
        };
        self.ui.home.tabs.select_profile(&instance.id);
        let compact = self.window_width < 960. || self.window_height < 680.;
        let inset = if compact { 16. } else { INSET };
        let header = self.profile_header(&instance, compact, window, cx);
        let body = self.profile_tabs(&instance, inset, compact, window, cx);
        div()
            .id("alt-home")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            // The banner runs to the panel's edges; its own padding insets the profile.
            .child(header.flex_none())
            .child(body)
            .into_any_element()
    }

    /// A world's picture, `side` points square: its own `icon.png` (64 pixels square, as the
    /// game takes it) at a whole multiple of its pixels, so it stays sharp; else `dirt.png` of
    /// the player's own game files; else a plain tile with the map glyph.
    fn world_icon(&mut self, world: &World, side: f32) -> Div {
        let tile = div()
            .flex_none()
            .size(px(side))
            .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(palette().selected));
        let stamp = world
            .last_played
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let own = world.icon.clone().and_then(|icon| {
            let key = format!("alt-world:{}:{stamp}", icon.display());
            self.icon_image(key, side, move || Some(icon))
        });
        // Always dirt, from the player's own prepared files; none shipped.
        let picture = own.or_else(|| {
            let file = self
                .game_resources()
                .join("assets/minecraft/textures/block/dirt.png");
            let key = format!("alt-dirt:{}", file.display());
            self.icon_image(key, side, move || Some(file))
        });
        match picture {
            Some(image) => tile.child(gpui::img(image).size(px(side))),
            None => tile.child(blocks::icon("map", side * 0.4, rgb(palette().muted).into())),
        }
    }

    /// "Survival · Today".
    fn world_facts(world: &World) -> String {
        format!(
            "{} · {}",
            tr(world.mode),
            sentence(&worlds_page::ago(world.last_played))
        )
    }

    /// One world as a tile: its picture, clean until the pointer is on it, then its tick box at
    /// the top-left and Play in its middle (the tick box stays while anything is ticked). Under
    /// it, what the world is, with the row's "…" menu beside the name. A press on the picture
    /// ticks while anything is ticked (or with Ctrl or Shift), and plays otherwise.
    fn world_tile(
        &mut self,
        world: &World,
        ticks: &crate::mods::selection::Selection,
        order: &Rc<Vec<String>>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        const GROUP: &str = "world-tile";
        let folder = world.folder.clone();
        let focus = self
            .worlds
            .tile_focus
            .entry(world.folder.clone())
            .or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone();
        let id = SharedString::from(format!("alt-world:{}", world.folder));
        let (ticked, any) = (ticks.contains(&world.folder), !ticks.is_empty());
        let radius = crate::tokens::r(crate::tokens::R_CONTROL);
        let (key, order_of) = (world.folder.clone(), order.clone());
        let picture = div()
            .id("alt-world-picture")
            .track_focus(&focus)
            .relative()
            .cursor_pointer()
            .rounded(radius)
            .focus_visible(|s| s.shadow(crate::frame::focus_ring(palette().focus)))
            .seen()
            .child(self.world_icon(world, WORLD_TILE))
            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                let gesture = crate::mods::selection::Gesture::of(&ev.modifiers());
                if gesture.selects() || any {
                    this.home_worlds_pick()
                        .click(&order_of, &key, gesture.range);
                } else {
                    this.play_to("--world", folder.clone());
                }
                cx.notify();
            }));
        // Play in the picture's middle, in the accent, only while the pointer is on the tile.
        let play_folder = world.folder.clone();
        let play = div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .opacity(0.)
            .group_hover(GROUP, |s| s.opacity(1.))
            .child(
                div()
                    .id(SharedString::from(format!(
                        "alt-world-play-{}",
                        world.folder
                    )))
                    .size(px(44.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .occlude()
                    .bg(rgb(palette().accent))
                    .shadow_md()
                    .child(crate::glass_ui::icon("play", 20., gpui::white()))
                    .tooltip(crate::glass_ui::tip(trf(
                        "Play {world}",
                        &[("world", &world.name)],
                    )))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.play_to("--world", play_folder.clone());
                        cx.notify();
                    })),
            );
        let (pick_key, pick_order) = (world.folder.clone(), order.clone());
        let check = self.tick_column(&id, ticked, any, {
            cx.listener(move |this, ev: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.home_worlds_pick()
                    .click(&pick_order, &pick_key, ev.modifiers().shift);
                cx.notify();
            })
        });
        // On a small backing so it reads on any picture; out of sight until it is wanted.
        let check = div()
            .absolute()
            .left(px(8.))
            .top(px(8.))
            .p(px(4.))
            .rounded(radius)
            .occlude()
            .bg(gpui::hsla(0., 0., 0., 0.4))
            .when(!(ticked || any), |d| {
                d.opacity(0.).group_hover(GROUP, |s| s.opacity(1.))
            })
            .child(check);
        let tile = div()
            .relative()
            .rounded(radius)
            .child(picture)
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(radius)
                    .when(ticked, |d| d.border_2().border_color(rgb(palette().focus))),
            )
            .child(play)
            .child(check);
        // The row's "…" menu, beside the name.
        let drawn = Rc::new(std::cell::Cell::new(gpui::Bounds::<gpui::Pixels>::default()));
        let (write, read) = (drawn.clone(), drawn);
        let menu_of = world.clone();
        let more = crate::widgets::button::icon_button_opening(
            SharedString::from(format!("alt-world-more-{}", world.folder)),
            "more-horizontal",
            tr("More actions"),
            crate::widgets::button::Style::Ghost,
            self.world_menu_open(&world.folder),
        )
        .flex_none()
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
        let caption = div()
            .flex()
            .items_start()
            .gap(px(4.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(tokens::BODY))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette().text))
                            .truncate()
                            .child(world.name.clone()),
                    )
                    .child(muted(Self::world_facts(world)).truncate()),
            )
            .child(more);
        let lit = id.clone();
        let outer = div()
            // The tile's own id scopes what is in it: no world's control is another's.
            .id(id)
            .group(GROUP)
            .flex_none()
            .w(px(WORLD_TILE))
            .h(px(TILE_TALL))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(tile)
            .child(caption);
        Self::watch_hover(outer, &lit, cx)
    }

    /// Every destination goes through `play_game`, which runs the sign-in, install, prepare,
    /// build and trust gates before the game starts.
    pub(crate) fn play_to(&mut self, flag: &'static str, value: String) {
        self.play_game(launcher_play::PlayDestination::Quick(flag, value));
    }
}

#[cfg(test)]
mod tests;
