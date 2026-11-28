//! Settings › Appearance: how a theme's colours are shown outside the editor: a thin strip in
//! the library's rows, and in the import and export sheets a small mock of a window in them
//! over all nine colours as swatches.
use super::*;
use sunrise_launcher::theme_files::Roles;
use sunrise_launcher::themes::library::{self, SLOTS};

/// A theme's colours side by side: the page, the sidebar, the cards, the text, the accent and
/// the danger colour.
pub(super) fn strip(roles: &Roles) -> Div {
    let band = |colour: u32| div().w(px(14.)).h(px(22.)).bg(rgb(colour));
    div()
        .flex()
        .flex_none()
        .overflow_hidden()
        .rounded(tokens::r(tokens::R_SMALL))
        .border_1()
        .border_color(rgb(palette().line))
        .children(
            [
                roles.background,
                roles.chrome,
                roles.card,
                roles.text,
                roles.accent,
                roles.danger,
            ]
            .map(band),
        )
}

/// A theme as a small window in its own colours: the sidebar, a card with a name, a muted line
/// and a button in the accent, a line of success and danger.
fn mock(roles: &Roles) -> Div {
    let side = div()
        .flex_none()
        .w(px(22.))
        .h_full()
        .bg(rgb(roles.chrome))
        .border_r_1()
        .border_color(rgb(roles.line));
    let dot = |colour: u32| div().size(px(8.)).rounded_full().bg(rgb(colour));
    let bar = |colour: u32, width: f32| div().h(px(5.)).w(px(width)).rounded_full().bg(rgb(colour));
    let card = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(6.))
        .p(px(8.))
        .rounded(tokens::r(tokens::R_SMALL))
        .bg(rgb(roles.card))
        .border_1()
        .border_color(rgb(roles.line))
        .child(
            div()
                .text_size(px(tokens::LABEL))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(roles.text))
                .child("Aa"),
        )
        .child(bar(roles.muted, 70.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    div()
                        .w(px(34.))
                        .h(px(12.))
                        .rounded_full()
                        .bg(rgb(roles.accent)),
                )
                .child(dot(roles.success))
                .child(dot(roles.danger)),
        );
    div()
        .flex()
        .flex_none()
        .w(px(150.))
        .h(px(78.))
        .overflow_hidden()
        .rounded(tokens::r(tokens::R_SMALL))
        .border_1()
        .border_color(rgb(roles.line))
        .bg(rgb(roles.background))
        .child(side)
        .child(div().flex_1().min_w_0().p(px(6.)).child(card))
}

/// The nine colours as swatches in the editor's order, each named by its tooltip.
fn swatches(id: SharedString, roles: &Roles) -> Div {
    let mut row = div().flex().flex_wrap().gap(px(4.));
    for (i, slot) in SLOTS.iter().enumerate() {
        let colour = (slot.get)(roles);
        row = row.child(
            div()
                .id(SharedString::from(format!("{id}-{i}")))
                .size(px(18.))
                .rounded(tokens::r(tokens::R_SMALL))
                .border_1()
                .border_color(rgb(palette().line))
                .bg(rgb(colour))
                .tooltip(crate::glass_ui::tip(format!(
                    "{} #{colour:06x}",
                    tr(slot.label)
                ))),
        );
    }
    row
}

/// One theme of a code, to look at before it is added or after it is made: its name, a mock of
/// a window in its colours and the palette.
pub(super) fn theme_card(id: SharedString, theme: &library::Custom) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .p(px(10.))
        .rounded(tokens::r(tokens::R_CONTROL))
        .border_1()
        .border_color(palette().rule())
        .child(mock(&theme.roles))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .truncate()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(tokens::LABEL))
                        .child(theme.name.clone()),
                )
                .child(swatches(id, &theme.roles)),
        )
}
