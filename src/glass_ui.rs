//! The glass look's controls ("Glass" in `docs/design-system.md`): the parts of the mockup's
//! `components/launcher/launcher-ui.tsx` and its shell (`launcher-app.tsx`), at its sizes. White
//! text and fills at a few opacities over [`glass`](crate::glass) plates; Geist throughout.
use crate::{hover_fade, hover_is, keyboard, theme::Theme, track_hover, Kind};
use gpui::{
    canvas, div, prelude::*, px, rgb, rgba, svg, AnyElement, App, Div, FontWeight, Hsla,
    SharedString, Stateful, Svg, Window,
};
use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;

use crate::paint::palette;
use std::sync::Arc;
use sunrise_launcher_theme::glass::{fill, ink, radius, Glass};

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
    /// How many [`hover_back`] layers this frame has built so far: the next one's name.
    static BACKS: Cell<usize> = const { Cell::new(0) };
    /// Each [`hover_back`] layer's pointer: easing on or off, from where.
    static BACK_HOVER: RefCell<HashMap<SharedString, crate::motion::Fade>> =
        RefCell::new(HashMap::new());
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
    BACKS.set(0);
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

/// The look's colours.
pub fn look() -> Glass {
    LOOK.get()
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

/// The side of one texel in a [`pixel_fill`].
pub(crate) const TEXEL: f32 = 4.;

/// How far a texel's lightness strays from its fill: on a coloured button, and on a sidebar,
/// where it stays well under the rows.
pub const TEXTURE_BUTTON: f32 = 0.02;

/// A fill of `c` as a block texture: square texels `step` of lightness off it either way, the
/// same every frame, so it never shimmers. The steps are fixed, not a share of `c`'s lightness, so
/// a near-black sidebar shows them too. Put it first in a `relative` element, so its content is
/// drawn over it.
pub fn pixel_fill(c: Hsla, step: f32) -> AnyElement {
    let shades = texel_shades(c, step);
    // The look's corners, which a canvas can't clip to: the fill is drawn round, and the
    // texels in its corners keep the fill's colour.
    let r = rad(0.);
    canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            let r = r.min(w / 2.).min(h / 2.);
            window.paint_quad(gpui::fill(bounds, c).corner_radii(px(r)));
            let (cols, rows) = ((w / TEXEL).ceil() as u32, (h / TEXEL).ceil() as u32);
            // A texel is a square: one that reaches into a round corner would square it off,
            // so those keep the base colour.
            let inside = |x: u32, y: u32| {
                let (left, top) = (x as f32 * TEXEL, y as f32 * TEXEL);
                let side = |a: f32, len: f32| a < r || a + TEXEL > len - r;
                r <= 0. || !(side(left, w) && side(top, h))
            };
            // Most texels keep the base colour; the rest take one of the shades.
            let tone = |x: u32, y: u32| {
                shades
                    .get((texel_noise(x, y) % 9) as usize)
                    .copied()
                    .filter(|_| inside(x, y))
            };
            for y in 0..rows {
                let top = y as f32 * TEXEL;
                let tall = TEXEL.min(h - top);
                let mut x = 0;
                while x < cols {
                    let Some(this) = tone(x, y) else {
                        x += 1;
                        continue;
                    };
                    // A run of one shade along the row is one quad, so a tall sidebar stays cheap.
                    let start = x;
                    while x < cols && tone(x, y) == Some(this) {
                        x += 1;
                    }
                    let left = start as f32 * TEXEL;
                    let wide = (x as f32 * TEXEL).min(w) - left;
                    let at = gpui::point(bounds.origin.x + px(left), bounds.origin.y + px(top));
                    let size = gpui::size(px(wide), px(tall));
                    window.paint_quad(gpui::fill(gpui::Bounds::new(at, size), this));
                }
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
    .into_any_element()
}

thread_local! {
    /// The grain tile last baked, and what for (colour, step and scale, as bits).
    static GRAIN: RefCell<Option<([u32; 6], Arc<gpui::RenderImage>)>> = const { RefCell::new(None) };
}

thread_local! {
    /// Each fold chevron's last state (open or not) and how often it has changed, so it turns
    /// only when its menu opens or closes, never as a page is first drawn.
    static FOLDS: RefCell<HashMap<SharedString, (bool, usize)>> = RefCell::new(HashMap::new());
}

/// How often the chevron `id` has turned, drawn now with its menu `open`: the first time it is
/// drawn it hasn't turned, whatever the state; each change after that is one more turn.
fn fold_turns(
    folds: &mut HashMap<SharedString, (bool, usize)>,
    id: SharedString,
    open: bool,
) -> usize {
    let (was, turns) = folds.entry(id).or_insert((open, 0));
    if *was != open {
        *was = open;
        *turns += 1;
    }
    *turns
}

/// A menu's chevron: pointing down while it is closed and up while it is open, turning between
/// the two as the menu opens and closes. Never on a page's first frame, and under reduced
/// motion it is at its end at once (`motion::eased`). `id` names it among the others.
pub fn fold_chevron(id: impl Into<SharedString>, open: bool, size: f32, ink: Hsla) -> AnyElement {
    turning(
        id.into(),
        open,
        "arrow-down-01",
        std::f32::consts::PI,
        size,
        ink,
    )
}

/// A section's chevron: pointing right while it is folded and down while it is open, turning
/// a quarter between the two, as [`fold_chevron`] turns.
pub fn disclosure_chevron(
    id: impl Into<SharedString>,
    open: bool,
    size: f32,
    ink: Hsla,
) -> AnyElement {
    turning(
        id.into(),
        open,
        "arrow-right-01",
        std::f32::consts::FRAC_PI_2,
        size,
        ink,
    )
}

/// `glyph` turned by `turn` (radians, clockwise) while open, easing between closed and open
/// each time `open` changes after its first frame.
fn turning(
    id: SharedString,
    open: bool,
    glyph: &'static str,
    turn: f32,
    size: f32,
    ink: Hsla,
) -> AnyElement {
    use gpui::AnimationExt as _;
    let turns = FOLDS.with_borrow_mut(|folds| fold_turns(folds, id.clone(), open));
    let at = move |t: f32| gpui::Transformation::rotate(gpui::radians(turn * t));
    let chevron = icon(glyph, size, ink);
    if turns == 0 {
        return chevron
            .with_transformation(at(if open { 1. } else { 0. }))
            .into_any_element();
    }
    chevron
        .with_animation(
            gpui::ElementId::Name(format!("{id}-fold-{turns}").into()),
            crate::motion::eased(sunrise_launcher_theme::MOVE),
            move |d, t| d.with_transformation(at(if open { t } else { 1. - t })),
        )
        .into_any_element()
}

/// The texture's three shades of `c`, `step` of lightness off it.
pub(crate) fn texel_shades(c: Hsla, step: f32) -> [Hsla; 3] {
    let shade = move |d: f32| gpui::hsla(c.h, c.s, (c.l + d).clamp(0., 1.), c.a);
    [shade(step), shade(-step), shade(1.75 * step)]
}

/// A fixed scramble of a texel's place.
pub(crate) fn texel_noise(x: u32, y: u32) -> u32 {
    let mut n = x.wrapping_mul(0x9e37_79b1) ^ y.wrapping_mul(0x85eb_ca77);
    n ^= n >> 15;
    n = n.wrapping_mul(0x2c1b_3c6d);
    n ^ (n >> 12)
}

/// `accent` a little lighter, for a coloured button under the pointer.
pub fn lit_accent(accent: u32) -> Hsla {
    let c = color(accent);
    gpui::hsla(c.h, c.s, (c.l * 1.1).min(1.), 1.)
}

/// How opaque white outlines are: every control and inner card at rest, under the pointer, and
/// open or chosen (redesign pass 2: outlines everywhere, stronger as the pointer comes).
pub mod edge {
    pub const REST: f32 = 0.10;
    pub const HOVER: f32 = 0.24;
}

/// `id`'s hover layer: a white fill at `fill`, eased in and out (no outline: a line flashing round
/// a row read as a glitch, not a hover). `bordered`: the element has its own 1 px outline, which
/// the layer reaches over.
pub(crate) fn hover_edge(id: &SharedString, r: f32, fill: f32, bordered: bool) -> gpui::AnyElement {
    let inset = if bordered { -1. } else { 0. };
    hover_fade(
        id,
        move || {
            div()
                .absolute()
                .top(px(inset))
                .left(px(inset))
                .right(px(inset))
                .bottom(px(inset))
                .rounded(px(rad(r)))
        },
        move |d, t| d.bg(white(fill * t)),
    )
}

/// A hover layer that hears the pointer itself, for an element with no id, or with hoverable
/// parts in it ([`track_hover`] lights one element at a time, so a row would go dark under its own
/// button): `paint` draws it at 0 (rest) to 1 (hovered), eased in and out on QUICK, at once under
/// reduced motion. Put it first in a `relative` element, so the content is drawn over it. At rest
/// it draws nothing and plays nothing.
fn hover_back(inset: f32, r: f32, paint: impl Fn(Div, f32) -> Div + 'static) -> AnyElement {
    let n = BACKS.get();
    BACKS.set(n + 1);
    // Named by its place in the frame's build: the pages are built in the same order each frame.
    // shortcut: a row hovered as its page is swapped by the keyboard may stay lit until the
    // pointer moves over it; give the callers ids if that shows.
    let id = SharedString::from(format!("hover-back-{n}"));
    let now = std::time::Instant::now();
    let fade = BACK_HOVER.with_borrow(|all| all.get(&id).copied());
    let shown = fade.filter(|f| !f.rested(now)).map(|fade| {
        fade.play(
            &id,
            div()
                .absolute()
                .top(px(inset))
                .left(px(inset))
                .right(px(inset))
                .bottom(px(inset))
                .rounded(px(rad(r))),
            paint,
        )
    });
    let key = id.clone();
    let owner = crate::widgets::Owner::default();
    div()
        .id(id)
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .child(owner.noting())
        .on_hover(move |hovered, window, cx| {
            BACK_HOVER.with_borrow_mut(|all| {
                let last = all.get(&key).copied();
                let fade = crate::motion::Fade::toward(last, *hovered, std::time::Instant::now());
                all.insert(key.clone(), fade);
            });
            owner.redraw(window, cx);
        })
        .children(shown)
        .into_any_element()
}

/// [`hover_back`] as [`hover_edge`]: white at `fill` and the hover outline. `bordered`: the
/// element has its own 1 px outline, which the layer's covers.
pub fn back_edge(r: f32, fill: f32, bordered: bool) -> AnyElement {
    hover_back(if bordered { -1. } else { 0. }, r, move |d, t| {
        d.bg(white(fill * t))
            .border_1()
            .border_color(white(edge::HOVER * t))
    })
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

/// A quiet button (`QUIET`): white at 8 %, 14 % under the pointer, 14 px medium, `h` tall.
pub fn quiet(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    h: f32,
    t: &Theme,
) -> Stateful<Div> {
    quiet_icon(id, None, label, h, t)
}

/// [`quiet`] with a 16 px icon before its label.
pub fn quiet_icon(
    id: impl Into<SharedString>,
    name: Option<&'static str>,
    label: impl Into<SharedString>,
    h: f32,
    t: &Theme,
) -> Stateful<Div> {
    let id = id.into();
    let d = div()
        .id(id.clone())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .h(px(h))
        .px(px(14.))
        .rounded(px(rad(radius::CONTROL)))
        .bg(white(fill::BUTTON))
        .border_1()
        .border_color(white(edge::REST))
        .cursor_pointer()
        .font_family(sunrise_launcher_theme::glass::FONT)
        .text_size(px(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(white(ink::FULL))
        .child(hover_edge(&id, radius::CONTROL, 0.06, true))
        .children(name.map(|n| icon(n, 16., white(ink::FULL)).relative()))
        .child(div().relative().child(label.into()));
    crate::press(keyboard(track_hover(d, id.clone()), t), &id, |d| {
        d.bg(white(0.18)).top(px(1.))
    })
}

/// A field (`WIDE_FIELD`): 44 px, round at 12, white at 7 %, 11 % with the focus, 14 px. Red at
/// 12 % while what it holds is refused.
pub fn field(focused: bool, invalid: bool) -> Div {
    let bg: Hsla = if invalid {
        rgba(sunrise_launcher_theme::glass::DANGER_HOVER << 8 | 0x1f).into()
    } else {
        white(if focused {
            fill::FIELD_FOCUS
        } else {
            fill::FIELD
        })
    };
    div()
        .relative()
        .flex()
        .items_center()
        .h(px(44.))
        .px(px(16.))
        .overflow_hidden()
        .rounded(px(crate::glass_ui::rad(radius::ROW)))
        .bg(bg)
        .border_1()
        .border_color(if invalid {
            color(sunrise_launcher_theme::glass::DANGER_HOVER).opacity(0.6)
        } else if focused {
            color(look().accent)
        } else {
            white(edge::REST)
        })
        .when(!focused, |d| d.child(back_edge(radius::ROW, 0., true)))
        .font_family(sunrise_launcher_theme::glass::FONT)
        .text_size(px(14.))
}

thread_local! {
    /// The slider being dragged, if one is.
    static SLIDING: RefCell<Option<SharedString>> = const { RefCell::new(None) };
}

/// A switch (`Toggle`): 40×24, round; the accent with an `on_accent` knob on the right while on,
/// white at 14 % with a white knob at 70 % on the left while off.
#[cfg_attr(not(windows), allow(dead_code))] // the Windows setup window
pub fn toggle(
    id: impl Into<SharedString>,
    on: bool,
    accent: u32,
    on_accent: u32,
    t: &Theme,
) -> Stateful<Div> {
    let id = id.into();
    let knob = div()
        .absolute()
        .top(px(4.))
        .size(px(16.))
        .rounded_full()
        .bg(if on { color(on_accent) } else { white(0.7) });
    let knob = crate::motion::knob(&id, on, knob, 4., 16.);
    keyboard(
        div()
            .id(id)
            .relative()
            .flex_none()
            .w(px(40.))
            .h(px(24.))
            .rounded_full()
            .bg(if on {
                color(accent)
            } else {
                white(fill::BUTTON_HOVER)
            })
            .cursor_pointer()
            .child(knob),
        t,
    )
}

/// The generic [`button`](crate::button) in the glass look, 36 px: `Primary` is the mockup's
/// `PLAY` (the accent, semibold), `Secondary` its `QUIET` (white at 8 %, 14 % under the pointer),
/// `Ghost` its text button (white at 70 %, white on 6 % under the pointer).
pub fn button(id: SharedString, label: SharedString, kind: Kind, t: &Theme) -> Stateful<Div> {
    let look = look();
    let d = div()
        .id(id.clone())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .h(px(36.))
        .px(px(14.))
        .rounded(px(crate::glass_ui::rad(radius::CONTROL)))
        .cursor_pointer()
        .font_family(sunrise_launcher_theme::glass::FONT)
        .text_size(px(14.))
        .line_height(px(20.))
        .font_weight(FontWeight::MEDIUM);
    let d = match kind {
        Kind::Primary => {
            let c = if hover_is(&id) {
                lit_accent(look.accent)
            } else {
                color(look.accent)
            };
            d.text_color(color(look.on_accent))
                .font_weight(FontWeight::SEMIBOLD)
                .child(pixel_fill(c, TEXTURE_BUTTON))
        }
        Kind::Secondary => d
            .bg(white(fill::BUTTON))
            .border_1()
            .border_color(white(edge::REST))
            .text_color(white(ink::FULL))
            .child(hover_edge(&id, radius::CONTROL, 0.06, true)),
        Kind::Ghost => d
            .text_color(white(ink::BODY))
            .hover(|s| s.text_color(white(ink::FULL)))
            .child(hover_edge(&id, radius::CONTROL, 0.06, false)),
    };
    let el = keyboard(
        track_hover(d.child(div().relative().child(label)), id.clone()),
        t,
    );
    crate::press(el, &id, move |d| match kind {
        Kind::Primary => d.opacity(0.92).top(px(1.)),
        Kind::Secondary => d.bg(white(0.18)).top(px(1.)),
        Kind::Ghost => d.bg(white(0.1)).top(px(1.)),
    })
}

/// A tooltip's view: one line on a solid float, for controls shown by their icon alone.
pub struct Tip(SharedString);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        // Offset below the pointer so tooltips never sit directly on top of the hovered button.
        // Found by its text in window tests: `tooltip: <text>`.
        let text = self.0.clone();
        div()
            .pt(px(20.))
            .pl(px(4.))
            .debug_selector(move || format!("tooltip: {text}"))
            .child(
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

    #[test]
    fn a_fold_chevron_turns_only_when_its_menu_opens_or_closes() {
        let mut folds = HashMap::new();
        // First drawn, closed or open, it is where it belongs, without turning.
        assert_eq!(fold_turns(&mut folds, "a".into(), false), 0);
        assert_eq!(fold_turns(&mut folds, "b".into(), true), 0);
        // Drawn again the same: still no turn.
        assert_eq!(fold_turns(&mut folds, "a".into(), false), 0);
        // Each opening and closing is one turn, and each chevron counts its own.
        assert_eq!(fold_turns(&mut folds, "a".into(), true), 1);
        assert_eq!(fold_turns(&mut folds, "a".into(), true), 1);
        assert_eq!(fold_turns(&mut folds, "a".into(), false), 2);
        assert_eq!(fold_turns(&mut folds, "b".into(), true), 0);
        assert_eq!(fold_turns(&mut folds, "b".into(), false), 1);
    }
}
