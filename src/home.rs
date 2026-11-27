//! The selected profile, its launch controls, and four browsable collections.
use super::*;
use crate::frame::Reach as _;
use crate::paint::palette;
use crate::widgets::button::secondary;
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
/// How much of a tile's data packs button shows while nothing points at the tile.
const PACKS_AT_REST: f32 = 0.6;
/// How far the tabs and their content stand in from the panel's edge: as far as the banner's
/// own padding insets the profile (Figma: 24).
const INSET: f32 = 24.;

/// What stands in the profile banner.
pub(crate) enum Art {
    /// The design's own art, which fades into the panel on its left by itself.
    Fading(Div),
    /// A scene or a player's picture: the banner washes its left part with the panel.
    Scene(Div),
}

#[derive(Default)]
pub(crate) struct HomeState {
    /// Where the build picker was last drawn: the build chooser opens under it.
    picker: Rc<Cell<Option<Bounds<gpui::Pixels>>>>,
    /// Where the profile menu's button was last drawn: the menu opens under it.
    menu: Rc<Cell<Option<Bounds<gpui::Pixels>>>>,
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

    /// One world as a tile: its picture, which plays it, with what it is under it. Its data
    /// packs button sits on the picture, faint until the pointer is over the tile or the
    /// keyboard is on the picture or on the button.
    fn world_tile(
        &mut self,
        world: &World,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let (folder, path) = (world.folder.clone(), world.path.clone());
        let focus = self
            .worlds
            .tile_focus
            .entry(world.folder.clone())
            .or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone();
        let group = SharedString::from(format!("alt-world-tile-{}", world.folder));
        let picture = div()
            .id("alt-world-play")
            .track_focus(&focus)
            .relative()
            .cursor_pointer()
            .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
            .focus_visible(|s| s.shadow(crate::frame::focus_ring(palette().focus)))
            .seen()
            .child(self.world_icon(world, WORLD_TILE))
            // What a press does, said on the picture itself.
            .child(
                div()
                    .absolute()
                    .right(px(8.))
                    .bottom(px(8.))
                    .size(px(32.))
                    .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgb(palette().text))
                    .child(blocks::icon("play", 15., rgb(palette().background).into())),
            )
            .tooltip(crate::glass_ui::tip(trf(
                "Play {world}",
                &[("world", &world.name)],
            )))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.play_to("--world", folder.clone());
                cx.notify();
            }));
        let packs = secondary(
            SharedString::from(format!("alt-world-packs-{}", world.folder)),
            "",
        )
        .absolute()
        .left(px(8.))
        .bottom(px(8.))
        .size(px(32.))
        .px_0()
        .bg(rgb(palette().raised))
        .occlude()
        .when(!focus.is_focused(window), |d| d.opacity(PACKS_AT_REST))
        .group_hover(group.clone(), |s| s.opacity(1.))
        .focus_visible(|s| s.opacity(1.).border_color(rgb(palette().focus)))
        .child(blocks::icon("archive", 17., rgb(palette().text).into()))
        .tooltip(crate::glass_ui::tip(tr("Data packs")))
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.ui.home.tabs.remember(window.focused(cx));
            this.show_world_packs(path.clone());
            cx.notify();
        }));
        div()
            // The tile's own id scopes what is in it: no world's control is another's.
            .id(SharedString::from(format!("alt-world:{}", world.folder)))
            .flex_none()
            .w(px(WORLD_TILE))
            .h(px(TILE_TALL))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().group(group).relative().child(picture).child(packs))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        // A long name is cut at the picture's width: the tooltip has all of it.
                        div()
                            .id("alt-world-name")
                            .text_size(px(tokens::BODY))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette().text))
                            .truncate()
                            .child(world.name.clone())
                            .tooltip(crate::glass_ui::tip(world.name.clone())),
                    )
                    .child(muted(Self::world_facts(world))),
            )
    }

    /// Every destination goes through `play_game`, which runs the sign-in, install, prepare,
    /// build and trust gates before the game starts.
    pub(crate) fn play_to(&mut self, flag: &'static str, value: String) {
        self.play_game(launcher_play::PlayDestination::Quick(flag, value));
    }
}

#[cfg(test)]
mod tests;
