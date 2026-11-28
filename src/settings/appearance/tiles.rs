//! Settings › Appearance: the theme gallery. "Themes" is a grid of the built-in themes and "Your
//! themes" the player's saved ones, each a tile: a small launcher window drawn in the theme's
//! own colours with its name under it. Pressing a tile applies it at once.
use super::state::Dialog;
use super::*;
use crate::widgets::button::{icon_button, secondary, Style};
use crate::widgets::check::{checkbox, Tick};
use crate::widgets::focus::c;
use crate::widgets::{hover_fade, track_hover};
use gpui::SharedString;
use sunrise_launcher::appearance;
use sunrise_launcher::theme_files::{self, Roles};
use sunrise_launcher::themes::{self, library};

/// The space between tiles.
const GAP: f32 = 12.;
/// A tile's picture is this tall.
const PREVIEW_H: f32 = 110.;
/// The ring around a tile: its edge and the gap inside it, and so the frame's corner radius.
const RING: f32 = 2.;
const FRAME_R: f32 = 14.;
/// The settings pane at least this wide has four tiles across; narrower has two, and a tile
/// is never drawn narrower than [`MIN_TILE`].
const FOUR_ACROSS: f32 = 640.;
const MIN_TILE: f32 = 140.;

/// One tile of the gallery.
struct Tile {
    /// Names the tile's element, and its hover.
    key: SharedString,
    /// The id `settings theme` takes: pressing the tile chooses it.
    theme_id: String,
    name: SharedString,
    roles: Roles,
    selected: bool,
    /// A saved theme: its place in the list and its folder, for the tick and the "…" menu.
    saved: Option<(usize, String)>,
}

/// A theme's picture: the sidebar in the Sidebar colour, a content area in Background holding a
/// banner with an Accent pill and two rows, each a Text line, a Muted line and an Accent dot,
/// all on Cards. A tile draws it at `scale` 1, the Edit dialog bigger.
pub(super) fn picture(roles: &Roles, scale: f32) -> Div {
    let s = |n: f32| px(n * scale);
    let bar = |colour: u32, width: f32| {
        div()
            .flex_none()
            .h(s(4.))
            .w(s(width))
            .rounded_full()
            .bg(rgb(colour))
    };
    let line = || {
        div()
            .flex()
            .items_center()
            .gap(s(6.))
            .h(s(20.))
            .px(s(8.))
            .rounded(s(5.))
            .bg(rgb(roles.card))
            .child(bar(roles.text, 40.))
            .child(bar(roles.muted, 26.))
            .child(div().flex_1())
            .child(div().size(s(8.)).rounded_full().bg(rgb(roles.accent)))
    };
    let banner = div()
        .flex()
        .items_center()
        .h(s(34.))
        .px(s(8.))
        .rounded(s(6.))
        .bg(rgb(roles.card))
        .child(
            div()
                .w(s(44.))
                .h(s(14.))
                .rounded_full()
                .bg(rgb(roles.accent)),
        );
    let side = div().flex_none().w(s(26.)).h_full().bg(rgb(roles.chrome));
    div()
        .flex()
        .w_full()
        .h(s(PREVIEW_H))
        .overflow_hidden()
        .rounded(s(10.))
        .border_1()
        .border_color(rgb(palette().divider))
        .bg(rgb(roles.background))
        .child(side)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(s(8.))
                .p(s(10.))
                .child(banner)
                .child(line())
                .child(line()),
        )
}

impl Launcher {
    /// How many tiles stand across: four, two when the pane is narrow, one when two would be
    /// squashed.
    fn tile_columns(&self) -> u16 {
        let width = self.settings_pane_width();
        if width >= FOUR_ACROSS {
            4
        } else if (width - GAP) / 2. >= MIN_TILE {
            2
        } else {
            1
        }
    }

    /// A section's heading, with its controls at the right.
    fn section_head(title: SharedString, actions: Div) -> Div {
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(px(10.))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(tokens::HEADING))
                    .child(title),
            )
            .child(actions.flex().items_center().gap(px(6.)))
    }

    /// Chooses the theme `id` (a built-in's, or a saved one's `local:` id) and paints it.
    fn theme_choose(&mut self, id: &str, window: &Window) {
        self.appearance_change(|s| appearance::select_theme(s, id), window);
    }

    /// "Themes": the built-in themes, in the gallery's order.
    pub(super) fn themes_section(&mut self, cx: &mut Context<Self>) -> Div {
        let current = appearance::effective_id(&self.settings);
        let mut grid = div().grid().grid_cols(self.tile_columns()).gap(px(GAP));
        for theme in themes::GALLERY {
            let Some(night) = theme.palette else {
                continue;
            };
            grid = grid.child(self.theme_tile(
                Tile {
                    key: SharedString::from(format!("alt-theme-tile-{}", theme.id)),
                    theme_id: theme.id.to_string(),
                    name: tr(theme.name).to_string().into(),
                    roles: library::roles_of(night),
                    selected: current == theme.id,
                    saved: None,
                },
                cx,
            ));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(Self::section_head(tr("Themes").into(), div()))
            .child(grid)
    }

    /// "Your themes": the saved and imported ones, then a tile to make another, a copy of the
    /// theme now painted.
    pub(super) fn your_themes_section(&mut self, cx: &mut Context<Self>) -> Div {
        let saved = library::listed(&theme_files::loaded());
        let ticked: Vec<String> = saved
            .iter()
            .filter(|local| self.ui.settings.appearance.ticked.contains(&local.slug))
            .map(|local| local.slug.clone())
            .collect();
        let mut grid = div().grid().grid_cols(self.tile_columns()).gap(px(GAP));
        for (n, local) in saved.iter().enumerate() {
            let selected = self.settings.theme.as_deref() == Some(local.id().as_str());
            grid = grid.child(self.theme_tile(
                Tile {
                    key: SharedString::from(format!("alt-theme-mine-{}", local.slug)),
                    theme_id: local.id(),
                    name: local.name.clone().into(),
                    roles: local.roles,
                    selected,
                    saved: Some((n, local.slug.clone())),
                },
                cx,
            ));
        }
        grid = grid.child(self.new_theme_tile(cx));
        let import = secondary("alt-library-import", tr("Import theme")).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.dialog_open(Dialog::Import);
                cx.notify();
            },
        ));
        let mut actions = div();
        if !ticked.is_empty() {
            let count = ticked.len().to_string();
            actions = actions.child(
                secondary(
                    "alt-library-share",
                    trf("Share {count}…", &[("count", count.as_str())]),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.export_open(&ticked);
                    cx.notify();
                })),
            );
        }
        let actions = actions.child(import);
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(Self::section_head(tr("Your themes").into(), actions))
            .child(grid)
    }

    /// The dashed tile that makes a new theme, a copy of the one now painted, and opens it to
    /// edit. It is as big as a theme's tile: the same frame, and a line under it as tall as a
    /// name's.
    fn new_theme_tile(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = palette();
        let frame = div()
            .flex()
            .items_center()
            .justify_center()
            .h(px(PREVIEW_H + 4. * RING))
            .rounded(px(FRAME_R))
            .border_2()
            .border_dashed()
            .border_color(rgb(p.line))
            .child(blocks::icon("plus", 20., rgb(p.muted).into()));
        let label = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .px(px(4.))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(14.))
                    .child(tr("New theme")),
            )
            .child(div().flex_none().text_size(px(tokens::SMALL)).child(" "));
        div()
            .id("alt-theme-new")
            .reach()
            .cursor_pointer()
            .flex()
            .flex_col()
            .gap(px(6.))
            .text_color(rgb(p.muted))
            .hover(move |s| s.text_color(rgb(p.text)))
            .child(frame)
            .child(label)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.theme_new(window);
                cx.notify();
            }))
    }

    /// One tile: its picture in a ring that marks the chosen one and lights under the pointer,
    /// the name and its look under it.
    fn theme_tile(&mut self, tile: Tile, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = palette();
        let Tile {
            key,
            theme_id,
            name,
            roles,
            selected,
            saved,
        } = tile;
        let accent = c(p.accent);
        let ring = hover_fade(
            &key,
            || div().absolute().inset_0().rounded(px(FRAME_R)),
            move |d, a| d.border_2().border_color(accent.opacity(0.55 * a)),
        );
        let mut frame = div()
            .relative()
            .p(px(RING))
            .rounded(px(FRAME_R))
            .border_2()
            .border_color(if selected {
                accent
            } else {
                gpui::hsla(0., 0., 0., 0.)
            })
            .child(picture(&roles, 1.))
            .child(ring);
        if selected {
            frame = frame.child(
                div()
                    .absolute()
                    .top(px(-6.))
                    .right(px(-6.))
                    .size(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(accent)
                    .child(blocks::icon("check", 12., c(p.accent_on))),
            );
        }
        let group = key.clone();
        match saved {
            Some((n, slug)) => frame = frame.children(self.saved_overlays(n, slug, &group, cx)),
            None => frame = frame.child(self.customize_overlay(&key, &name, &roles, cx)),
        }
        let look = if roles.light { "Light" } else { "Dark" };
        let label = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .px(px(4.))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(14.))
                    .child(name),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(p.muted))
                    .child(tr(look)),
            );
        let el = div()
            .id(key.clone())
            .group(group)
            .reach()
            .cursor_pointer()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(frame)
            .child(label)
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.theme_choose(&theme_id, window);
                cx.notify();
            }));
        track_hover(el, key)
    }

    /// A built-in tile's Customize button: a copy of it, opened to edit. Faint at rest like a
    /// saved tile's "…".
    fn customize_overlay(
        &mut self,
        key: &SharedString,
        name: &SharedString,
        roles: &Roles,
        cx: &mut Context<Self>,
    ) -> Div {
        let (name, roles) = (name.clone(), *roles);
        let button = icon_button(
            SharedString::from(format!("{key}-customize")),
            "pencil-edit-02",
            tr("Customize"),
            Style::Ghost,
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            cx.stop_propagation();
            this.theme_copy(&name, &roles, window);
            cx.notify();
        }));
        div()
            .absolute()
            .top(px(14.))
            .right(px(14.))
            .opacity(0.55)
            .child(button)
    }

    /// A saved theme's tick (to share several at once) and "…" menu, shown while the pointer is
    /// on the tile, or for good while ticked or open.
    fn saved_overlays(
        &mut self,
        n: usize,
        slug: String,
        group: &SharedString,
        cx: &mut Context<Self>,
    ) -> [Div; 2] {
        let state = &self.ui.settings.appearance;
        let ticked = state.ticked.contains(&slug);
        let open = state.more.as_deref() == Some(slug.as_str());
        let tick = checkbox(
            ("alt-theme-tick", n),
            if ticked { Tick::On } else { Tick::Off },
        )
        .on_click({
            let slug = slug.clone();
            cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                let ticked = &mut this.ui.settings.appearance.ticked;
                if !ticked.remove(&slug) {
                    ticked.insert(slug.clone());
                }
                cx.notify();
            })
        });
        let menu = self.theme_menu(n, slug, open, cx);
        // Faint at rest so they can be found, full while ticked or open: as Home's cards show
        // their tick boxes (GPUI's group hover isn't always drawn again in this window).
        let _ = group;
        let shown = |d: Div, always: bool| d.absolute().opacity(if always { 1. } else { 0.55 });
        [
            shown(div().top(px(14.)).left(px(14.)).child(tick), ticked),
            shown(div().top(px(14.)).right(px(14.)).child(menu), open),
        ]
    }
}
