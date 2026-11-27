//! A name with a line of status right under it: an account's name over "Signed out", a profile's
//! over its build. The one default for every such pair, so the status never drifts away from the
//! name.
use super::*;
use crate::tokens;

/// `name` at `size`, and `status` under it (its words and colour) when there is one, the two
/// lines as tall as their text needs and no gap between them. It fills the room its parent gives
/// it and is centred vertically in it, so beside a face or buttons it sits level with them
/// whether or not it has a status line.
pub(crate) fn name_over(
    name: impl Into<SharedString>,
    size: f32,
    status: Option<(SharedString, u32)>,
) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .justify_center()
        .child(
            div()
                .truncate()
                .text_size(px(size))
                .line_height(px((size + 3.).round()))
                .font_weight(FontWeight::MEDIUM)
                .child(name.into()),
        )
        .children(status.map(|(text, ink)| {
            div()
                .truncate()
                .text_size(px(tokens::SMALL))
                .line_height(px(tokens::SMALL + 2.))
                .text_color(gpui::rgb(ink))
                .child(text)
        }))
}
