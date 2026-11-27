//! Text entry: the input box, the search field with its key hint, and the select (dropdown) button.
use super::*;
use crate::frame::{c, mix};
use crate::glass_ui::{fold_chevron, icon};
use crate::paint::palette;
use crate::tokens;

/// The edge a field draws: `danger` when invalid (over focus), else the focus ring, else `line`.
fn edge(focused: bool, invalid: bool) -> u32 {
    let p = palette();
    if invalid {
        p.danger
    } else if focused {
        p.focus
    } else {
        p.line
    }
}

/// The frame a line_edit sits in: a field `FIELD` tall, `field` fill, a 1 px edge, body text. The
/// caller puts the line_edit (or its text) inside it.
pub(crate) fn field_box(focused: bool, invalid: bool) -> Div {
    let p = palette();
    div()
        .flex()
        .items_center()
        .h(px(tokens::FIELD))
        .px(px(12.))
        .overflow_hidden()
        .rounded(tokens::r(tokens::R_CONTROL))
        .bg(c(p.field))
        .border_1()
        .border_color(c(edge(focused, invalid)))
        .text_size(px(tokens::BODY))
        .text_color(c(p.text))
}

/// A search field: a magnifier, then `slot` (the line_edit or a label) filling the room between,
/// then the key hint at the end when there is one (`Ctrl K`, `Esc`). `slot` is passed in rather
/// than appended, so the key chip stays last.
pub(crate) fn search_box(
    focused: bool,
    key_hint: Option<SharedString>,
    slot: impl IntoElement,
) -> Div {
    let has_key = key_hint.is_some();
    field_box(focused, false)
        .gap(px(8.))
        .pr(px(if has_key { 6. } else { 12. }))
        .child(icon("search-01", 16., c(palette().muted)))
        .child(div().flex_1().min_w_0().overflow_hidden().child(slot))
        .when_some(key_hint, |d, key| d.child(key_chip(key)))
}

/// A key hint: its label in a 1 px `line` outline, no fill, small caption text.
pub(crate) fn key_chip(label: impl Into<SharedString>) -> Div {
    let p = palette();
    div()
        .flex_none()
        .whitespace_nowrap()
        .px(px(6.))
        .py(px(1.))
        .rounded(tokens::r(tokens::R_SMALL))
        .border_1()
        .border_color(c(p.line))
        .text_size(px(tokens::CAPTION))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(c(p.muted))
        .child(label.into())
}

/// A dropdown's trigger: a field box with `value` in `text` and a chevron at the end that turns
/// while `open`. The edge lightens a step under the pointer (fading), and the button takes Tab
/// focus with the focus edge. The caller adds the click.
pub(crate) fn select_button(
    id: impl Into<SharedString>,
    value: impl Into<SharedString>,
    open: bool,
) -> Stateful<Div> {
    let id: SharedString = id.into();
    let p = palette();
    let base = field_box(false, false)
        .id(id.clone())
        .justify_between()
        .gap(px(8.))
        .cursor_pointer()
        .tab_index(0)
        .focus(move |s| s.border_color(c(p.focus)))
        .child(div().flex_1().min_w_0().truncate().child(value.into()))
        .child(fold_chevron(format!("{id}-chevron"), open, 16., c(p.muted)));
    // A layer over the edge, a pixel out so it covers the border ring; it fades in on hover.
    let lighter = mix(c(p.line), c(p.text), 0.25);
    let layer = || {
        div()
            .absolute()
            .top(px(-1.))
            .left(px(-1.))
            .right(px(-1.))
            .bottom(px(-1.))
            .rounded(tokens::r(tokens::R_CONTROL))
            .border_1()
    };
    let paint = move |d: Div, a: f32| d.border_color(lighter.opacity(a));
    track_hover(base, id.clone()).child(hover_fade(&id, layer, paint))
}
