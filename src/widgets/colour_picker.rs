//! A colour picker popover: a square to drag over for saturation and value, a bar for the hue,
//! the colour's hex (the caller's field) and the colour it was before beside the one it is now.
//!
//! The picker keeps nothing. The caller owns the colour (as [`Hsv`], so a grey keeps its hue
//! while the bar moves), the drag in progress and the field, and passes what a press, a drag, a
//! release and a close do. A press or drag is turned into a colour by [`Areas::pick`], using
//! where the square and the bar were drawn last. The maths is in `themes::hsv`.
use super::*;
use gpui::{rgb, AnimationExt as _};
use std::cell::Cell;
use std::rc::Rc;
use sunrise_launcher::themes::hsv::{self, Hsv};

/// The square's width, and the hue bar's: the bar is six gradients of 35 px, so no seam falls
/// between two pixels.
const WIDTH: f32 = 210.;
/// The square's height.
const HEIGHT: f32 = 140.;
/// The hue bar's height.
const BAR: f32 = 14.;
/// The ring on the square, and the one on the bar.
const RING: f32 = 14.;
const KNOB: f32 = 18.;
/// The popover's padding, and the gap between its parts.
const PAD: f32 = 12.;
const GAP: f32 = 10.;

/// What a drag moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    Square,
    Hue,
}

/// Where the square and the bar were drawn (window coordinates), written as each frame draws
/// them and read by the next press or drag.
#[derive(Clone, Default)]
pub(crate) struct Areas {
    square: Rc<Cell<Bounds<gpui::Pixels>>>,
    hue: Rc<Cell<Bounds<gpui::Pixels>>>,
}

impl Areas {
    /// The colour a pointer at `at` picks with `part`, from `now`: the square moves saturation
    /// and value and keeps the hue, the bar moves the hue and keeps the rest.
    pub(crate) fn pick(&self, part: Part, now: Hsv, at: gpui::Point<gpui::Pixels>) -> Hsv {
        let (x, y) = (f32::from(at.x), f32::from(at.y));
        match part {
            Part::Square => {
                let b = self.square.get();
                hsv::square_pick(
                    now.h,
                    (x, y),
                    (f32::from(b.origin.x), f32::from(b.origin.y)),
                    (f32::from(b.size.width), f32::from(b.size.height)),
                )
            }
            Part::Hue => {
                let b = self.hue.get();
                Hsv {
                    h: hsv::hue_pick(x, f32::from(b.origin.x), f32::from(b.size.width)),
                    ..now
                }
            }
        }
    }
}

/// What the picker's parts do. Each is called with the window and the app, so a caller reaches
/// its state through an entity.
#[derive(Clone)]
pub(crate) struct Handlers {
    /// A press on a part, at this pointer position.
    pub(crate) press: OnPress,
    /// The pointer moved over the popover or the layer around it.
    pub(crate) drag: OnDrag,
    /// The button was let go.
    pub(crate) release: OnEvent,
    /// A press outside the popover, or Escape.
    pub(crate) close: OnEvent,
}

/// A press on a part of the picker, at the pointer's position.
pub(crate) type OnPress = Rc<dyn Fn(Part, gpui::Point<gpui::Pixels>, &mut Window, &mut App)>;
/// The pointer moving while a part is held.
pub(crate) type OnDrag = Rc<dyn Fn(&gpui::MouseMoveEvent, &mut Window, &mut App)>;
/// A plain event: the button let go, or the picker closed.
pub(crate) type OnEvent = Rc<dyn Fn(&mut Window, &mut App)>;

/// What a caller keeps for one picker: the colour as [`Hsv`], where the square and bar were
/// drawn, the part being dragged, whether the popover is open and where the keyboard is. The
/// caller keeps the colour itself (a setting) and is handed it by the methods that change it.
pub(crate) struct PickerState {
    pub(crate) hsv: Hsv,
    /// The colour when the popover opened.
    pub(crate) before: u32,
    pub(crate) areas: Areas,
    pub(crate) dragging: Option<Part>,
    pub(crate) open: bool,
    focus: Option<FocusHandle>,
}

impl Default for PickerState {
    fn default() -> Self {
        PickerState {
            hsv: Hsv::from_rgb(0),
            before: 0,
            areas: Areas::default(),
            dragging: None,
            open: false,
            focus: None,
        }
    }
}

impl PickerState {
    /// Opens the popover on `rgb` and gives it the keyboard, so Escape reaches it.
    pub(crate) fn open_on(&mut self, rgb: u32, window: &mut Window, cx: &mut App) {
        self.open = true;
        self.hsv = Hsv::from_rgb(rgb);
        self.before = rgb;
        self.dragging = None;
        let focus = self.focus.get_or_insert_with(|| cx.focus_handle()).clone();
        window.focus(&focus, cx);
    }

    /// Closes the popover.
    pub(crate) fn close(&mut self) {
        self.open = false;
        self.dragging = None;
    }

    /// Reads `shown` again when something other than the picker changed the colour; a colour
    /// the picker made keeps its hue, and the grey it may be on.
    pub(crate) fn sync(&mut self, shown: u32) {
        if self.hsv.to_rgb() != shown {
            self.hsv = Hsv::from_rgb(shown);
        }
    }

    /// What [`Spec`] needs while the popover is open: its colour, the colour it was, where its
    /// parts were drawn and where the keyboard is.
    pub(crate) fn view(&mut self, cx: &mut App) -> (Hsv, u32, Areas, FocusHandle) {
        let focus = self.focus.get_or_insert_with(|| cx.focus_handle()).clone();
        (self.hsv, self.before, self.areas.clone(), focus)
    }

    /// A press on `part` at `at`: the colour it picks.
    pub(crate) fn press(&mut self, part: Part, at: gpui::Point<gpui::Pixels>) -> Option<u32> {
        self.dragging = Some(part);
        self.pick(at)
    }

    /// The pointer moved: the colour it picks while the left button holds a part, else none
    /// (and a drag whose button was let go outside the window ends).
    pub(crate) fn drag(&mut self, ev: &gpui::MouseMoveEvent) -> Option<u32> {
        if ev.pressed_button == Some(gpui::MouseButton::Left) {
            self.pick(ev.position)
        } else {
            self.dragging = None;
            None
        }
    }

    /// The button was let go.
    pub(crate) fn release(&mut self) {
        self.dragging = None;
    }

    fn pick(&mut self, at: gpui::Point<gpui::Pixels>) -> Option<u32> {
        let part = self.dragging?;
        self.hsv = self.areas.pick(part, self.hsv, at);
        Some(self.hsv.to_rgb())
    }
}

/// The popover's [`Handlers`] for a picker kept in a view `V`: `state` finds its
/// [`PickerState`], and `set` is told each colour the pointer picks.
pub(crate) fn handlers<V: 'static>(
    this: gpui::Entity<V>,
    state: fn(&mut V) -> &mut PickerState,
    set: fn(&mut V, u32, &mut gpui::Context<V>),
) -> Handlers {
    Handlers {
        press: Rc::new({
            let this = this.clone();
            move |part: Part, at: gpui::Point<gpui::Pixels>, _: &mut Window, cx: &mut App| {
                this.update(cx, |v, cx| {
                    if let Some(rgb) = state(v).press(part, at) {
                        set(v, rgb, cx);
                    }
                    cx.notify();
                })
            }
        }),
        drag: Rc::new({
            let this = this.clone();
            move |ev: &gpui::MouseMoveEvent, _: &mut Window, cx: &mut App| {
                this.update(cx, |v, cx| {
                    if let Some(rgb) = state(v).drag(ev) {
                        set(v, rgb, cx);
                    }
                    cx.notify();
                })
            }
        }),
        release: Rc::new({
            let this = this.clone();
            move |_: &mut Window, cx: &mut App| {
                this.update(cx, |v, cx| {
                    state(v).release();
                    cx.notify();
                })
            }
        }),
        close: Rc::new(move |_: &mut Window, cx: &mut App| {
            this.update(cx, |v, cx| {
                state(v).close();
                cx.notify();
            })
        }),
    }
}

/// What a popover shows and where it hangs.
pub(crate) struct Spec<'a> {
    pub(crate) id: &'static str,
    /// The colour as picked.
    pub(crate) now: Hsv,
    /// The colour it was when the popover opened.
    pub(crate) before: u32,
    /// How far under the control (which sits in a `relative` parent as tall as itself) the
    /// popover starts.
    pub(crate) drop: f32,
    pub(crate) areas: &'a Areas,
    /// Where the keyboard is while the popover is open.
    pub(crate) focus: &'a FocusHandle,
}

/// The popover: a layer that takes every press outside it, and the popover over that, with
/// `hex` (the caller's field for the colour) beside the preview.
pub(crate) fn popover(
    spec: Spec<'_>,
    hex: Stateful<Div>,
    handlers: &Handlers,
) -> [gpui::Deferred; 2] {
    let Spec {
        id,
        now,
        before,
        drop,
        areas,
        focus,
    } = spec;
    let body = div()
        .flex()
        .flex_col()
        .gap(px(GAP))
        .child(square(id, now, areas, handlers))
        .child(bar(id, now, areas, handlers))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(GAP))
                .child(preview(id, before, now.to_rgb()))
                .child(hex.flex_1().min_w_0()),
        );
    let placed = div().absolute().top(px(drop)).left_0().child(
        gpui::anchored()
            .snap_to_window_with_margin(px(12.))
            .child(panel(id, body, focus, handlers)),
    );
    [
        gpui::deferred(backdrop(handlers)).with_priority(9),
        gpui::deferred(placed).with_priority(10),
    ]
}

/// The layer under the popover that takes every press outside it (which closes the popover),
/// and goes on with a drag that leaves the popover.
fn backdrop(handlers: &Handlers) -> Div {
    let (close, drag, release) = (
        handlers.close.clone(),
        handlers.drag.clone(),
        handlers.release.clone(),
    );
    div()
        .absolute()
        .top(px(-4000.))
        .left(px(-4000.))
        .w(px(8000.))
        .h(px(8000.))
        .occlude()
        .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
            close(window, cx)
        })
        .on_mouse_move(move |ev, window, cx| drag(ev, window, cx))
        .on_mouse_up(gpui::MouseButton::Left, move |_, window, cx| {
            release(window, cx)
        })
}

/// The popover's plate around `body`: it takes the presses and the keyboard inside it, fades
/// in, and Escape closes it.
fn panel(
    id: &'static str,
    body: Div,
    focus: &FocusHandle,
    handlers: &Handlers,
) -> impl IntoElement {
    let p = crate::paint::palette();
    let stop = |_: &gpui::MouseDownEvent, _: &mut Window, cx: &mut App| cx.stop_propagation();
    let (close, drag, release) = (
        handlers.close.clone(),
        handlers.drag.clone(),
        handlers.release.clone(),
    );
    div()
        .id(SharedString::from(format!("{id}-popover")))
        .track_focus(focus)
        .w(px(WIDTH + 2. * PAD + 2.))
        .p(px(PAD))
        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
        .border_1()
        .border_color(p.input_edge())
        .bg(rgb(p.raised))
        .text_color(rgb(p.text))
        .shadow_lg()
        .occlude()
        .on_mouse_down(gpui::MouseButton::Left, stop)
        .on_mouse_down(gpui::MouseButton::Right, stop)
        .on_scroll(|_: &gpui::ScrollEvent, _, cx| cx.stop_propagation())
        .on_mouse_move(move |ev, window, cx| drag(ev, window, cx))
        .on_mouse_up(gpui::MouseButton::Left, move |_, window, cx| {
            release(window, cx)
        })
        .on_key_down(move |ev: &KeyDownEvent, window, cx| {
            if ev.keystroke.key == "escape" {
                close(window, cx);
                cx.stop_propagation();
            }
        })
        .child(body)
        .with_animation(
            SharedString::from(format!("{id}-popover-in")),
            crate::motion::eased(sunrise_launcher_theme::QUICK),
            |d, t| d.opacity(t),
        )
}

/// A ring handle of `size` px with its centre at (`x`, `y`) inside its parent, filled with `fill`.
fn handle(size: f32, x: f32, y: f32, fill: Option<u32>) -> Div {
    div()
        .absolute()
        .left(px(x - size / 2.))
        .top(px(y - size / 2.))
        .size(px(size))
        .rounded_full()
        .border_2()
        .border_color(gpui::white())
        .when_some(fill, |d, colour| d.bg(rgb(colour)))
        .shadow(vec![gpui::BoxShadow {
            color: gpui::hsla(0., 0., 0., 0.5),
            offset: gpui::point(px(0.), px(0.)),
            blur_radius: px(3.),
            spread_radius: px(0.),
            inset: false,
        }])
}

/// A canvas filling its parent that notes where it was drawn in `cell`.
fn note(cell: &Rc<Cell<Bounds<gpui::Pixels>>>) -> impl gpui::IntoElement {
    let write = cell.clone();
    gpui::canvas(move |bounds, _, _| write.set(bounds), |_, _, _, _| {})
        .absolute()
        .inset_0()
}

/// The saturation and value square: the hue as a fill, white fading out to the right over it,
/// and black fading in downwards over that.
fn square(id: &'static str, now: Hsv, areas: &Areas, handlers: &Handlers) -> Stateful<Div> {
    let white = gpui::white();
    let press = handlers.press.clone();
    let layers = div()
        .absolute()
        .inset_0()
        .overflow_hidden()
        .rounded(crate::tokens::r(crate::tokens::R_SMALL))
        .bg(rgb(Hsv::pure(now.h)))
        .child(div().absolute().inset_0().bg(gpui::linear_gradient(
            90.,
            gpui::linear_color_stop(white, 0.),
            gpui::linear_color_stop(white.opacity(0.), 1.),
        )))
        .child(div().absolute().inset_0().bg(gpui::linear_gradient(
            180.,
            gpui::linear_color_stop(gpui::black().opacity(0.), 0.),
            gpui::linear_color_stop(gpui::black(), 1.),
        )));
    div()
        .id(SharedString::from(format!("{id}-square")))
        .relative()
        .flex_none()
        .w(px(WIDTH))
        .h(px(HEIGHT))
        .cursor_pointer()
        .child(layers)
        .child(handle(
            RING,
            now.s.clamp(0., 1.) * WIDTH,
            (1. - now.v.clamp(0., 1.)) * HEIGHT,
            None,
        ))
        .child(note(&areas.square))
        .on_mouse_down(gpui::MouseButton::Left, move |ev, window, cx| {
            press(Part::Square, ev.position, window, cx)
        })
}

/// The hue bar: the rainbow as six gradients side by side, red to red, with a handle on it.
fn bar(id: &'static str, now: Hsv, areas: &Areas, handlers: &Handlers) -> Stateful<Div> {
    let press = handlers.press.clone();
    let segment = |k: u32| {
        let (from, to) = (k as f32 * 60., (k + 1) as f32 * 60.);
        div().flex_1().h_full().bg(gpui::linear_gradient(
            90.,
            gpui::linear_color_stop(rgb(Hsv::pure(from)), 0.),
            gpui::linear_color_stop(rgb(Hsv::pure(to)), 1.),
        ))
    };
    let rainbow = div()
        .absolute()
        .inset_0()
        .flex()
        .overflow_hidden()
        .rounded(crate::tokens::r(crate::tokens::R_SMALL))
        .children((0..6).map(segment));
    div()
        .id(SharedString::from(format!("{id}-hue")))
        .relative()
        .flex_none()
        .w(px(WIDTH))
        .h(px(BAR))
        .cursor_pointer()
        .child(rainbow)
        .child(handle(
            KNOB,
            now.h.rem_euclid(360.) / 360. * WIDTH,
            BAR / 2.,
            Some(Hsv::pure(now.h)),
        ))
        .child(note(&areas.hue))
        .on_mouse_down(gpui::MouseButton::Left, move |ev, window, cx| {
            press(Part::Hue, ev.position, window, cx)
        })
}

/// The colour it was and the colour it is, side by side.
fn preview(id: &'static str, before: u32, now: u32) -> Div {
    let p = crate::paint::palette();
    let half = |name: &'static str, colour: u32, label: &'static str| {
        div()
            .id(SharedString::from(format!("{id}-{name}")))
            .flex_1()
            .h_full()
            .bg(rgb(colour))
            .tooltip(crate::glass_ui::tip(tr(label)))
    };
    div()
        .flex()
        .flex_none()
        .w(px(56.))
        .h(px(crate::tokens::CONTROL))
        .overflow_hidden()
        .rounded(crate::tokens::r(crate::tokens::R_SMALL))
        .border_1()
        .border_color(rgb(p.line))
        .child(half("before", before, crate::i18n::tr_key("Before")))
        .child(half("now", now, crate::i18n::tr_key("Now")))
}
