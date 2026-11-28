//! A "…" menu over the page: its rows (a 16 px icon and a label, a hairline before a danger row)
//! in a card hung from the button that opened it, over a layer that takes the window's presses so
//! one outside the card shuts it. The Servers pages' cards share it; the caller says what each
//! row does and how the menu closes.
use super::*;
use crate::frame::Reach as _;
use crate::paint::palette;
use crate::tokens;
use crate::widgets::focus::c;
use gpui::{rgb, MouseButton, MouseDownEvent};

/// The menu card's width, and how close to the window's edge it may come.
const WIDTH: f32 = 210.;
const EDGE: f32 = 8.;

/// One row of the menu. `off` leaves it dimmed and dead, with the reason as its tooltip ("" for
/// none).
pub(crate) struct Row {
    pub(crate) id: SharedString,
    pub(crate) icon: &'static str,
    pub(crate) label: SharedString,
    pub(crate) danger: bool,
    pub(crate) off: Option<SharedString>,
}

impl Row {
    pub(crate) fn new(
        id: impl Into<SharedString>,
        icon: &'static str,
        label: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            icon,
            label: label.into(),
            danger: false,
            off: None,
        }
    }

    /// The row in the danger colour, set apart by a hairline above it.
    pub(crate) fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    /// The row dimmed and dead while `why` is Some (its words, "" for none).
    pub(crate) fn off(mut self, why: Option<SharedString>) -> Self {
        self.off = why;
        self
    }
}

/// How tall a menu of `rows` rows is, `dangers` of them set apart by a hairline (9) before
/// them, with 8 padding each side and the border.
pub(crate) fn height(rows: usize, dangers: usize) -> f32 {
    rows as f32 * tokens::CONTROL + dangers as f32 * 9. + 18.
}

/// A 32 px menu row, as the sidebar profile menu and the Worlds menu draw theirs. The hairline
/// before a danger row comes with it.
pub(crate) fn row(
    entry: Row,
    go: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let p = palette();
    let ink = if entry.danger { p.danger } else { p.text };
    let line = div()
        .id(entry.id)
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(tokens::CONTROL))
        .px(px(8.))
        .rounded(tokens::r(tokens::R_SMALL))
        .text_size(px(tokens::LABEL))
        .text_color(rgb(ink))
        .child(crate::glass_ui::icon(entry.icon, 16., c(ink)))
        .child(entry.label);
    let line = match entry.off {
        Some(why) => line
            .opacity(0.45)
            .cursor_default()
            .when(!why.is_empty(), |d| d.tooltip(crate::glass_ui::tip(why)))
            .into_any_element(),
        None => line
            .reach()
            .cursor_pointer()
            .hover(|s| s.bg(rgb(palette().hover)))
            .on_click(go)
            .into_any_element(),
    };
    if !entry.danger {
        return line;
    }
    div()
        .child(div().h(px(1.)).my(px(4.)).bg(rgb(p.divider)))
        .child(line)
        .into_any_element()
}

/// The layer for a menu of `rows` (`height` tall), hung from `button` (window coordinates) in a
/// window of `window` size. A press outside the card calls `close`.
pub(crate) fn layer(
    id: &'static str,
    rows: Vec<AnyElement>,
    height: f32,
    button: gpui::Bounds<gpui::Pixels>,
    window: (f32, f32),
    cx: &mut Context<Launcher>,
    close: fn(&mut Launcher),
) -> Stateful<Div> {
    let p = palette();
    let left = (f32::from(button.right()) - WIDTH)
        .min(window.0 - WIDTH - EDGE)
        .max(EDGE);
    let below = f32::from(button.bottom()) + 4.;
    let top = if below + height + EDGE <= window.1 {
        below
    } else {
        (f32::from(button.top()) - 4. - height).max(EDGE)
    };
    div()
        .id(id)
        .absolute()
        .inset_0()
        .occlude()
        .on_scroll(|_, _, cx| cx.stop_propagation())
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                close(this);
                cx.notify();
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                close(this);
                cx.notify();
            }),
        )
        .child(
            div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(WIDTH))
                .p(px(8.))
                .rounded(tokens::r(tokens::R_CARD))
                .border_1()
                .border_color(rgb(p.line))
                .bg(rgb(p.raised))
                .text_color(rgb(p.text))
                .shadow_lg()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .children(rows),
        )
}
