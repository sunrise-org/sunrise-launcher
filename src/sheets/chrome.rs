//! A sheet's plates and titles, and stepping the keyboard through them.

use super::*;

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
        .child(
            // Never taller or wider than the window: a small one scrolls the sheet.
            body.id((id, 1usize))
                .w(px(width))
                .max_w_full()
                .max_h_full()
                .overflow_y_scroll()
                .track_scroll(&crate::frame::scrolls("sheet"))
                .p(px(28.))
                .gap(px(16.))
                .rounded(crate::tokens::r(crate::tokens::R_PANEL))
                .border_1()
                // The soft rule the header, the page and the Logs panel use: the raised fill
                // already sets the sheet off the scrim, so a bright edge only shouts.
                .border_color(rgb(palette().divider))
                .bg(rgb(palette().raised))
                .text_color(rgb(palette().text))
                .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                    cx.stop_propagation()
                }),
        )
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
        .text_size(px(tokens::TITLE))
        .font_weight(FontWeight::BOLD)
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
