//! The parts every detail page is built from (a mod, a pack, a collection, a source): the header
//! block with the artwork, name, by-line and key facts; the card; the small muted lines; and the
//! widths the action column and the About column share. Colours come from the palette roles.
use super::*;

/// The action column beside a header, and the About column beside the body.
pub(in crate::mods) const ACTION_WIDTH: f32 = 240.;
pub(in crate::mods) const SIDE_WIDTH: f32 = 320.;

/// A rounded plate with a hairline edge, for a block of rows or a side panel.
pub(in crate::mods) fn card() -> Div {
    div()
        .rounded(tokens::r(tokens::R_CARD))
        .border_1()
        .border_color(palette().rule())
        .bg(palette().card())
}

/// Small muted text: a fact, a reason, a line that says what a control does.
pub(in crate::mods) fn muted(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(tokens::SMALL))
        .text_color(c(palette().muted))
        .child(text.into())
}

/// `muted` text a reader may want to copy: an error, a reason, a note.
pub(in crate::mods) fn note(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(tokens::SMALL))
        .text_color(c(palette().muted))
        .child(crate::selectable::selectable(text))
}

/// A section's title, above its rows.
pub(in crate::mods) fn section(title: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(tokens::HEADING))
        .font_weight(FontWeight::SEMIBOLD)
        .child(title.into())
}

/// The header of a detail page: the artwork (a 64 px tile the caller draws), the name at title
/// size, the by-line with the chips (source, kind, support) beside it, the summary, and the key
/// facts (downloads, updated, version, loaders) as one small muted line under them.
pub(in crate::mods) fn head(
    artwork: AnyElement,
    name: &str,
    by: Option<String>,
    chips: Vec<AnyElement>,
    summary: Option<String>,
    facts: Vec<String>,
) -> Div {
    let meta = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_x(px(8.))
        .gap_y(px(4.))
        .children(by.map(|by| {
            div()
                .text_size(px(tokens::LABEL))
                .text_color(c(palette().muted))
                .child(crate::selectable::selectable(by))
        }))
        .children(chips);
    div()
        .flex()
        .items_start()
        .gap(px(16.))
        .child(artwork)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .text_size(px(tokens::TITLE))
                        .font_weight(FontWeight::BOLD)
                        .child(crate::selectable::selectable(name.to_owned())),
                )
                .child(meta)
                .children(summary.filter(|s| !s.trim().is_empty()).map(|summary| {
                    div()
                        .text_size(px(tokens::LABEL))
                        .text_color(c(palette().text))
                        .child(crate::selectable::selectable(summary))
                }))
                .children((!facts.is_empty()).then(|| muted(facts.join("  ·  ")))),
        )
}

/// The column the main action and its secondary links stack in, to the right of the header.
pub(in crate::mods) fn action_column() -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .w(px(ACTION_WIDTH))
        .flex_none()
}
