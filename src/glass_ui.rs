//! The glass look's controls ("Glass" in `docs/design-system.md`): the parts of the mockup's
//! `components/launcher/launcher-ui.tsx` and its shell (`launcher-app.tsx`), at its sizes. White
//! text and fills at a few opacities over [`glass`](crate::glass) plates; Geist throughout.
use gpui::{div, prelude::*, px, rgb, svg, App, Div, FontWeight, Hsla, SharedString, Svg, Window};
use std::cell::Cell;
use std::cell::RefCell;

use crate::paint::palette;
use sunrise_launcher_theme::glass::{radius, Glass};

thread_local! {
    /// Whether this frame's pages draw the glass look (the dark appearance).
    static ON: Cell<bool> = const { Cell::new(false) };
    /// Its corners' radius ([`set_corners`]).
    static CORNERS: Cell<f32> = const { Cell::new(0.) };
    /// The look's colours for the player's accent.
    static LOOK: Cell<Glass> = Cell::new(Glass::default_look());
    /// Whether the rows being built go in a Settings card ([`flat_rows`]).
    static FLAT: Cell<bool> = const { Cell::new(false) };
    /// The light appearance: the glass look's white ink and fills turn to [`INK`].
    static LIGHT: Cell<bool> = const { Cell::new(false) };
}

/// The light appearance's ink: the mockup's text on white (`#100a1f`), at the opacities the dark
/// look gives white.
const INK: u32 = sunrise_launcher_theme::glass::ON_LIGHT;

/// Set as each frame's pages start being built: whether the appearance is light.
#[cfg_attr(not(windows), allow(dead_code))] // the Windows setup window
pub fn set_light(light: bool) {
    LIGHT.set(light);
}

/// Whether the light appearance is drawn.
pub fn light() -> bool {
    LIGHT.get()
}

/// Whether the pages being built draw the glass look: helpers with no theme at hand ask.
pub fn on() -> bool {
    ON.get()
}

/// Set as each frame's pages start being built.
#[cfg_attr(not(windows), allow(dead_code))] // the Windows setup window
pub fn set_on(on: bool) {
    ON.set(on);
    crate::glass::set_outline(on);
}

/// A corner of the glass parts: square, as the Outline look draws them (`docs/design-system.md`).
pub fn rad(r: f32) -> f32 {
    let r = if on() { CORNERS.get() } else { r };
    r * crate::tokens::roundness()
}

/// Set as each frame's pages start being built: the colours for the player's accent.
#[cfg_attr(not(windows), allow(dead_code))] // the Windows setup window
pub fn set_look(look: Glass) {
    LOOK.set(look);
}

/// White at `a`: the dark look's ink and fills. In light, the ink at `a`, so every part keeps
/// its weight on the light plates.
pub fn white(a: f32) -> Hsla {
    if light() {
        color(INK).opacity(a)
    } else {
        gpui::white().opacity(a)
    }
}

/// A colour of the look (`0xrrggbb`).
pub fn color(c: u32) -> Hsla {
    rgb(c).into()
}

/// Text at `size` px in Geist, white at `a`, at `weight`.
pub fn text(size: f32, weight: FontWeight, a: f32) -> Div {
    div()
        .font_family(sunrise_launcher_theme::glass::FONT)
        .text_size(px(size))
        .line_height(px((size * 1.4).round()))
        .font_weight(weight)
        .text_color(white(a))
}

/// One of the mockup's Hugeicons, stroked in `color`.
pub fn icon(name: &'static str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(format!("icons/glass/{name}.svg"))
        .flex_none()
        .size(px(size))
        .text_color(color)
}

/// What a dialog or the palette lays over the launcher (`bg-[#07050f]/50 backdrop-blur-xl`):
/// GPUI can't blur what it drew, so the scene's blurred copy (a menu's, `hzl-float`) covers the
/// page. The scrim goes over it.
pub fn scrim_backdrop() -> impl IntoElement {
    crate::glass::backdrop(0.)
}

/// The scrim over [`scrim_backdrop`]: 50 % in the dark look, 20 % in light, so the dimmed
/// launcher stays day-lit.
pub fn scrim() -> Hsla {
    color(sunrise_launcher_theme::glass::SCRIM).opacity(if light() { 0.2 } else { 0.5 })
}

/// A dialog's plate, round at `radius` (`hzl-glass` over the scrim): its piece of the scene
/// under the scrim's dimming, then the plates' white. The content goes after it.
pub fn dialog_plate(radius: f32) -> Div {
    let layer = |c: Hsla| {
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .rounded(px(crate::glass_ui::rad(radius)))
            .bg(c)
    };
    crate::glass::plate(radius)
        // Light plates are already day-lit: no dimming under them.
        .child(layer(
            color(sunrise_launcher_theme::glass::SCRIM).opacity(if light() { 0. } else { 0.5 }),
        ))
        .child(layer(white(sunrise_launcher_theme::glass::TINT)))
}

thread_local! {
    /// The slider being dragged, if one is.
    static SLIDING: RefCell<Option<SharedString>> = const { RefCell::new(None) };
}

/// A tooltip's view: one line on a solid float, for controls shown by their icon alone.
pub struct Tip(SharedString);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        // Offset below the pointer so tooltips never sit directly on top of the hovered button.
        div().pt(px(20.)).pl(px(4.)).child(
            div()
                .max_w(px(360.))
                .px(px(10.))
                .py(px(6.))
                .rounded(px(rad(radius::CONTROL)))
                .bg(rgb(palette().raised))
                .border_1()
                .border_color(rgb(palette().line))
                .font_family(sunrise_launcher_theme::glass::FONT)
                .text_size(px(13.))
                .text_color(rgb(palette().text))
                .child(self.0.clone()),
        )
    }
}

thread_local! {
    /// Whether a menu, sheet or dialog lies over the page in the frame drawn last.
    static PAGE_COVERED: Cell<bool> = const { Cell::new(false) };
    /// Whether the frame is drawing the page (rail, header, page) rather than what lies over it.
    static DRAWING_PAGE: Cell<bool> = const { Cell::new(false) };
}

/// Before the page is drawn: whether a layer will lie over it (`Launcher::layer_open`).
pub fn tips_page(covered: bool) {
    PAGE_COVERED.set(covered);
    DRAWING_PAGE.set(true);
}

/// Before the layers over the page are drawn: their own tooltips always show.
pub fn tips_layers() {
    DRAWING_PAGE.set(false);
}

/// Whether a tooltip made while drawing the page (`on_page`) may show now. GPUI shows one half
/// a second after the pointer stops, and a click doesn't cancel that: the button that opened a
/// menu or dialog put its tooltip on top of it. A page under a layer shows none.
fn tip_shows(on_page: bool) -> bool {
    !(on_page && PAGE_COVERED.get())
}

/// What a tooltip that may not show draws: nothing.
struct NoTip;

impl Render for NoTip {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
    }
}

/// `.tooltip(ui::tip(label))`: the label of an icon-only control, on hover.
pub fn tip(label: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView {
    let label: SharedString = label.into();
    tip_with(move |_| label.clone())
}

/// A tooltip whose label is read when it shows, not when the pointer arrived: a click in
/// between can change what the control does.
pub fn tip_with(
    label: impl Fn(&App) -> SharedString + 'static,
) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView {
    let on_page = DRAWING_PAGE.get();
    move |_, cx| {
        if tip_shows(on_page) {
            let label = label(cx);
            cx.new(|_| Tip(label)).into()
        } else {
            cx.new(|_| NoTip).into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_tooltip_waits_under_a_layer_and_a_layers_own_always_shows() {
        tips_page(false);
        assert!(tip_shows(true));
        tips_page(true);
        assert!(!tip_shows(true), "the page is covered");
        tips_layers();
        assert!(tip_shows(false), "a menu's own tooltip");
        tips_page(false);
        assert!(tip_shows(true), "uncovered again");
    }
}
