//! Small pieces the three Mods screens share. Colours are the launcher's (`crate::frame`).
use super::*;
use crate::frame::{BG, CARD, LINE, MUTED, SOFT, TEXT};
use gpui::{rgb, Hsla};
use std::path::PathBuf;
use std::rc::Rc;

pub(super) fn c(x: u32) -> Hsla {
    rgb(x).into()
}

pub(super) fn tag(label: &'static str) -> Div {
    div()
        .flex_none()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(tokens::R_SMALL))
        .border_1()
        .border_color(c(LINE))
        .text_size(px(tokens::CAPTION))
        .text_color(c(MUTED))
        .child(label)
}

pub(super) fn muted(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(tokens::SMALL))
        .text_color(c(MUTED))
        .child(text.into())
}

/// The main action on a screen, in cream: gold is Play's alone.
pub(super) use crate::frame::primary;

/// A control that can't be used now: drawn dim, with no handler.
pub(super) fn dim(b: Stateful<Div>) -> Stateful<Div> {
    b.opacity(0.45).cursor_default().tab_stop(false)
}

pub(crate) fn chip(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    on: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .px(px(14.))
        // Shaped like a button.
        .h(px(tokens::CONTROL))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(tokens::R_CONTROL))
        // Solid, both: an outline on one would make it read larger than the other.
        .border_1()
        .border_color(if on { c(TEXT) } else { c(LINE) })
        .bg(if on { c(TEXT) } else { c(LINE) })
        .text_color(if on { c(BG) } else { c(TEXT) })
        .text_size(px(tokens::LABEL))
        .cursor_pointer()
        .focus_visible(|s| s.border_color(c(tokens::GOLD)))
        .child(label.into())
}

/// What choosing one of [`choices`] does.
pub(crate) type Choose = Rc<dyn Fn(&mut Launcher, &mut Context<Launcher>)>;

/// One choice among chips (`label`, chosen, what choosing it does): tabs, or a setting with a
/// few values. The group is one Tab stop with the ring around it, and the arrow keys choose the
/// chip before or after, as a click on it would.
pub(crate) fn choices(
    id: &'static str,
    chips: Vec<(SharedString, bool, Choose)>,
    cx: &mut Context<Launcher>,
) -> Stateful<Div> {
    let at = chips.iter().position(|(_, on, _)| *on).unwrap_or(0);
    let picks: Vec<Choose> = chips.iter().map(|(_, _, pick)| pick.clone()).collect();
    let mut group = div()
        .id(id)
        .tab_index(0)
        .seen()
        .flex()
        .flex_wrap()
        .items_center()
        // Room for the ring, taken back so the chips stay where they were.
        .p(px(3.))
        .m(px(-4.))
        .rounded(px(tokens::R_CONTROL + 4.))
        .border_1()
        .border_color(gpui::transparent_black())
        .focus_visible(|s| s.border_color(c(tokens::GOLD)));
    for (n, (label, on, pick)) in chips.into_iter().enumerate() {
        // A few options side by side: short words don't make one smaller than the others.
        group = group.child(
            chip((id, n), label, on)
                .min_w(px(96.))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    pick(this, cx);
                    cx.notify();
                })),
        );
    }
    group.on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
        let step = match ev.keystroke.key.as_str() {
            "left" | "up" => picks.len() - 1,
            "right" | "down" => 1,
            _ => return,
        };
        picks[(at + step) % picks.len()](this, cx);
        cx.stop_propagation();
        cx.notify();
    }))
}

pub(crate) fn switch(id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .tab_index(0)
        .seen()
        .focus_visible(|s| s.shadow(crate::frame::focus_ring(tokens::GOLD)))
        .w(px(40.))
        .h(px(22.))
        .rounded(px(tokens::R_CONTROL))
        .p(px(2.))
        .flex()
        .bg(if on { c(TEXT) } else { c(LINE) })
        .when(on, |d| d.justify_end())
        .cursor_pointer()
        .child(
            div()
                .size(px(18.))
                .rounded(px(tokens::R_CONTROL))
                .bg(if on { c(BG) } else { c(MUTED) }),
        )
}

pub(crate) fn checkbox(id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .tab_index(0)
        .seen()
        .size(px(18.))
        .rounded(px(tokens::R_SMALL))
        .border_1()
        .border_color(if on { c(TEXT) } else { c(MUTED) })
        .focus_visible(|s| s.border_color(c(tokens::GOLD)))
        .bg(if on { c(TEXT) } else { c(BG) })
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .children(on.then(|| blocks::icon("check", 12., c(BG))))
}

/// A mod's picture, or its first letter until it arrives.
pub(super) fn tile(path: Option<PathBuf>, name: &str, side: f32) -> Div {
    let t = div()
        .flex_none()
        .size(px(side))
        .rounded(px(tokens::R_CONTROL))
        .overflow_hidden()
        .bg(c(CARD))
        .border_1()
        .border_color(c(LINE))
        .flex()
        .items_center()
        .justify_center();
    match path {
        Some(p) => t.child(gpui::img(p).size(px(side))),
        None => t
            .text_size(px(side * 0.45))
            .text_color(c(MUTED))
            .child(name.chars().next().map(String::from).unwrap_or_default()),
    }
}

/// How many cards fit side by side in a Discover list `width` wide: three on a roomy window,
/// fewer as it narrows, so a card never gets narrower than its own text needs.
pub(super) fn columns(width: f32) -> u16 {
    (((width + 12.) / 332.).floor() as u16).clamp(1, 3)
}

/// One thing to add in a Discover list, for mods and packs alike: picture, name and author, one
/// line of what it is, then its tags and numbers with the control beside them, and under them
/// a sentence when there is something to say (why it can't be added, how well it runs). Every
/// text shortens or wraps inside the card, whatever its width.
#[allow(clippy::too_many_arguments)]
pub(super) fn browse_card(
    id: impl Into<ElementId>,
    picture: Option<PathBuf>,
    name: &str,
    author: &str,
    summary: &str,
    facts: Vec<Div>,
    control: Option<AnyElement>,
    note: Option<String>,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .gap(px(12.))
        .p(px(12.))
        .min_w_0()
        .overflow_hidden()
        .rounded(px(tokens::R_CARD))
        .border_1()
        .border_color(c(LINE))
        .bg(c(CARD))
        .cursor_pointer()
        .hover(|s| s.border_color(c(MUTED)))
        .tab_index(0)
        .seen()
        .focus_visible(|s| s.border_color(c(tokens::GOLD)))
        .child(tile(picture, name, 48.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(8.))
                        .min_w_0()
                        .child(
                            div()
                                .min_w_0()
                                .font_weight(FontWeight::SEMIBOLD)
                                .truncate()
                                .child(name.to_owned()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(tokens::SMALL))
                                .text_color(c(MUTED))
                                .truncate()
                                .child(author.to_owned()),
                        ),
                )
                .child(
                    div()
                        .text_size(px(tokens::LABEL))
                        .text_color(c(MUTED))
                        .line_clamp(2)
                        .text_ellipsis()
                        .child(summary.to_owned()),
                )
                // Cards in a row are as tall as the tallest: the last line sits at the bottom.
                .child(div().flex_1())
                .child(
                    div()
                        .flex()
                        .items_end()
                        .gap(px(8.))
                        .min_w_0()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap_x(px(8.))
                                .gap_y(px(4.))
                                .children(facts),
                        )
                        .children(control.map(|c| div().flex_none().child(c))),
                )
                // Two lines at most: the whole sentence is the tooltip.
                .children(note.map(|note| {
                    muted(note.clone())
                        .id("note")
                        .line_clamp(2)
                        .text_ellipsis()
                        .tooltip(crate::glass_ui::tip(note))
                })),
        )
}

/// "Added", where a card's control would be.
pub(super) fn added_mark() -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(5.))
        .text_size(px(tokens::LABEL))
        .text_color(c(0x7fd6a0))
        .child(blocks::icon("check", 12., c(0x7fd6a0)))
        .child(tr("Added"))
        .into_any_element()
}

pub(super) fn card() -> Div {
    div()
        .rounded(px(tokens::R_CARD))
        .border_1()
        .border_color(c(SOFT))
        .bg(c(CARD))
}

/// The theme the shared text field draws with, in this look's colours.
pub(crate) fn field_theme(mut t: crate::theme::Theme) -> crate::theme::Theme {
    t.ink = c(TEXT);
    t.muted = c(MUTED);
    t.glass = false;
    t
}
