//! A sheet's plates and titles, and stepping the keyboard through them.

use super::*;
use gpui::AnimationExt as _;

pub(super) fn card() -> Div {
    div()
        .flex()
        .flex_col()
        .rounded(crate::tokens::r(crate::tokens::R_CARD))
        .border_1()
        .border_color(palette().input_edge())
        .bg(rgb(palette().background))
}

pub(super) fn muted(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(tokens::LABEL))
        .text_color(rgb(palette().muted))
        .child(crate::selectable::selectable(text))
}

/// The scrim and the sheet's frame. A press outside closes the sheet; inside, nothing does.
pub(crate) fn frame(
    id: &'static str,
    width: f32,
    body: Div,
    cx: &mut Context<Launcher>,
) -> Stateful<Div> {
    modal_frame(id, width, body, cx, Launcher::close_sheet)
}

pub(crate) fn modal_frame(
    id: &'static str,
    width: f32,
    body: Div,
    cx: &mut Context<Launcher>,
    close: fn(&mut Launcher),
) -> Stateful<Div> {
    div()
        .id(id)
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .p(px(16.))
        .bg(palette().scrim())
        .on_mouse_down(MouseButton::Right, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation()
        })
        .on_mouse_down(MouseButton::Middle, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation()
        })
        .on_scroll(|_, _, cx| cx.stop_propagation())
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                close(this);
                cx.notify();
            }),
        )
        .child(arriving(
            id,
            // The plate sits in its own box, so the arrival moves the box, not the plate's
            // scroll state.
            div()
                .relative()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    // Never taller or wider than the window: a small one scrolls the sheet.
                    body.id((id, 1usize))
                        .w(px(width))
                        .max_w_full()
                        .max_h_full()
                        .overflow_y_scroll()
                        .track_scroll(&crate::frame::scrolls("sheet"))
                        .p(px(PLATE_PAD))
                        .gap(px(16.))
                        .rounded(crate::tokens::r(crate::tokens::R_PANEL))
                        .border_1()
                        .border_color(rgb(palette().line))
                        .bg(rgb(palette().raised))
                        .shadow(plate_shadow())
                        .text_color(rgb(palette().text))
                        .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                            cx.stop_propagation()
                        }),
                ),
        ))
}

/// The plate's box arriving with a quick fade and a short rise, except while a benchmark runs
/// (its frames measure the list in the sheet, not the fade) and under reduced motion.
fn arriving(id: &'static str, plate: Div) -> gpui::AnyElement {
    if !crate::motion::arrivals() {
        return plate.into_any_element();
    }
    plate
        .with_animation(
            gpui::ElementId::Name(format!("{id}-open").into()),
            crate::motion::eased(OPEN),
            |d, t| d.opacity(t).top(px(tokens::RISE * (1. - t))),
        )
        .into_any_element()
}

/// A popup's padding, from its edge to its content.
const PLATE_PAD: f32 = 24.;

/// How a popup arrives: a quick fade with the short rise of content arriving.
const OPEN: sunrise_launcher_theme::Motion = sunrise_launcher_theme::Motion {
    name: "sheet-open",
    ms: 160,
    curve: sunrise_launcher_theme::MOVE.curve,
};

/// The soft shadow that lifts a popup off the scrim.
pub(super) fn plate_shadow() -> Vec<gpui::BoxShadow> {
    vec![gpui::BoxShadow {
        color: gpui::hsla(0., 0., 0., 0.35),
        offset: gpui::point(px(0.), px(8.)),
        blur_radius: px(32.),
        spread_radius: px(0.),
        inset: false,
    }]
}

/// A prominent warning modal frame: strong warning border, edge-to-edge warning header band
/// supplied by `body`, and press-outside closes just like [`frame`].
pub(crate) fn warning_frame(
    id: &'static str,
    width: f32,
    border: Hsla,
    body: Div,
    cx: &mut Context<Launcher>,
) -> Stateful<Div> {
    div()
        .id(id)
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .p(px(16.))
        .bg(palette().scrim())
        .on_mouse_down(MouseButton::Right, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation()
        })
        .on_mouse_down(MouseButton::Middle, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation()
        })
        .on_scroll(|_, _, cx| cx.stop_propagation())
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, _, cx| {
                this.close_sheet();
                cx.notify();
            }),
        )
        .child(
            body.id((id, 1usize))
                .w(px(width))
                .max_w_full()
                .max_h_full()
                .overflow_y_scroll()
                .track_scroll(&crate::frame::scrolls("sheet"))
                .rounded(crate::tokens::r(crate::tokens::R_PANEL))
                .border_2()
                .border_color(border)
                .bg(rgb(palette().raised))
                .text_color(rgb(palette().text))
                .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                    cx.stop_propagation()
                }),
        )
}

pub(crate) fn title(text: String) -> Div {
    div()
        .text_size(px(tokens::HEADING))
        .font_weight(FontWeight::SEMIBOLD)
        .child(crate::selectable::selectable(text))
}

/// Moves the keyboard to the next Tab stop inside the layer that `focus` holds, or the one
/// before it (`back`); the page's own stops are passed over.
pub(super) fn step_inside(focus: &FocusHandle, back: bool, window: &mut Window, cx: &mut App) {
    for _ in 0..TAB_STOPS {
        if back {
            window.focus_prev(cx);
        } else {
            window.focus_next(cx);
        }
        if focus.contains_focused(window, cx) {
            break;
        }
    }
    if !focus.contains_focused(window, cx) {
        window.focus(focus, cx);
    }
}
