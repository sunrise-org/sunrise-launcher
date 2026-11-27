//! Chips: filter chips, source chips (Sunrise, Modrinth, GitHub), tags, and a group of chips that acts as one choice.
use super::*;
use crate::frame::{c, Reach};
use crate::paint::palette;
use std::rc::Rc;

/// Modrinth's brand green (#1BD96A). A brand colour, so the one hex value a chip is drawn with.
pub(crate) const MODRINTH: u32 = 0x1bd96a;

/// A chip's height, padding and gap between its parts, as the Figma file draws them.
const HEIGHT: f32 = 24.;
const PADDING: f32 = 10.;
const GAP: f32 = 4.;
/// The leading icon and the chevron.
const ICON: f32 = 14.;
/// The least width each chip of a [`choices`] group takes, so short words don't make one smaller.
const CHOICE_MIN_WIDTH: f32 = 96.;

/// What a chip says: a plain filter, the one that is on, or the source it comes from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    /// An outline chip, off.
    Neutral,
    /// A filled chip, on.
    Selected,
    /// Sunrise's own mods and servers: the peach.
    Sunrise,
    /// From Modrinth.
    Modrinth,
    /// From GitHub.
    GitHub,
}

/// The colours a chip of one kind is drawn in.
struct Look {
    /// The fill, or none for an outline.
    fill: Option<Hsla>,
    stroke: Hsla,
    label: Hsla,
}

fn look(kind: Kind) -> Look {
    let p = palette();
    match kind {
        Kind::Neutral => Look {
            fill: None,
            stroke: c(p.line),
            label: c(p.muted),
        },
        // A step stronger than the outline: the selected fill, with a muted edge.
        Kind::Selected => Look {
            fill: Some(c(p.selected)),
            stroke: c(p.muted).opacity(0.4),
            label: c(p.text),
        },
        Kind::Sunrise => Look {
            fill: Some(c(p.accent_ink)),
            stroke: c(p.accent_ink),
            label: c(p.chrome),
        },
        Kind::Modrinth => Look {
            fill: Some(c(MODRINTH)),
            stroke: c(MODRINTH),
            label: c(p.chrome),
        },
        Kind::GitHub => Look {
            fill: Some(c(p.muted)),
            stroke: c(p.muted),
            label: c(p.chrome),
        },
    }
}

/// The box every chip is: its height, padding, 1 px stroke, corners and label size, in the
/// colours of `kind`. The callers add the label and any icon.
fn shape(kind: Kind) -> Div {
    let look = look(kind);
    let base = div()
        .flex_none()
        .h(px(HEIGHT))
        .px(px(PADDING))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(GAP))
        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
        .border_1()
        .border_color(look.stroke)
        .text_size(px(crate::tokens::SMALL))
        .text_color(look.label);
    match look.fill {
        Some(fill) => base.bg(fill),
        None => base,
    }
}

/// A clickable chip (`id` names it for hover and press; the caller adds `on_click`). Its Tab
/// stop, if it has one, is the caller's: a row of chips adds `tab_index(0)` itself.
pub(crate) fn chip(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    kind: Kind,
) -> Stateful<Div> {
    chip_with(id, label, kind, None, false)
}

/// [`chip`] with an optional leading icon (14 px, by name as `glass_ui::icon` takes it) and a
/// chevron at its end, for a chip that opens a menu.
pub(crate) fn chip_with(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    kind: Kind,
    leading_icon: Option<&'static str>,
    chevron: bool,
) -> Stateful<Div> {
    let id: SharedString = id.into();
    let text = look(kind).label;
    let base = shape(kind).id(id.clone()).relative().cursor_pointer();
    // Only the outline chip fades a hover fill in: the filled ones keep their colour.
    let base = if kind == Kind::Neutral {
        let hover = hover_layer(&id, c(palette().hover), crate::tokens::R_CONTROL, 0.);
        track_hover(base.child(hover), id.clone())
    } else {
        base
    };
    let base = base
        .children(leading_icon.map(|name| glass_ui::icon(name, ICON, text)))
        .child(label.into())
        .children(chevron.then(|| glass_ui::icon("arrow-down-01", ICON, text)));
    press(base, &id, |d| d.opacity(theme::PRESSED))
}

/// A chip that is not clicked: a label in a chip's box, for saying what a thing is.
pub(crate) fn tag(label: impl Into<SharedString>, kind: Kind) -> Div {
    shape(kind).child(label.into())
}

/// Where the arrow key `key` moves a choice from `at` among `len` choices: the one before for
/// left and up, the one after for right and down, wrapping round. None for any other key, or
/// for no choices.
fn next_choice(len: usize, at: usize, key: &str) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let step = match key {
        "left" | "up" => len - 1,
        "right" | "down" => 1,
        _ => return None,
    };
    Some((at + step) % len)
}

/// One choice among a few chips (`options`: the value each chip stands for, and its label), with
/// `current` on. The group is one Tab stop with the ring around it. The arrow keys choose the
/// chip before or after `current`, as a click on it would, and a click chooses the chip clicked.
/// `on_pick` runs with the value chosen; it must redraw whatever shows the choice.
pub(crate) fn choices<P: Copy + PartialEq + 'static>(
    id: &str,
    options: &[(P, SharedString)],
    current: P,
    on_pick: impl Fn(P, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let at = options
        .iter()
        .position(|(value, _)| *value == current)
        .unwrap_or(0);
    let values: Vec<P> = options.iter().map(|(value, _)| *value).collect();
    let on_pick = Rc::new(on_pick);
    let mut group = div()
        .id(SharedString::from(id))
        .tab_index(0)
        .seen()
        .flex()
        .flex_wrap()
        .items_center()
        // Room for the ring, taken back so the chips stay where they were.
        .p(px(3.))
        .m(px(-4.))
        .rounded(crate::tokens::r(crate::tokens::R_CONTROL) + px(4.))
        .border_1()
        .border_color(gpui::transparent_black())
        .focus_visible(|s| s.border_color(c(palette().focus)));
    for (n, (value, label)) in options.iter().enumerate() {
        let value = *value;
        let kind = if value == current {
            Kind::Selected
        } else {
            Kind::Neutral
        };
        let pick = on_pick.clone();
        group = group.child(
            chip(SharedString::from(format!("{id}-{n}")), label.clone(), kind)
                .min_w(px(CHOICE_MIN_WIDTH))
                .on_click(move |_, window, cx| pick(value, window, cx)),
        );
    }
    group.on_key_down(move |ev: &KeyDownEvent, window, cx| {
        let Some(next) = next_choice(values.len(), at, ev.keystroke.key.as_str()) else {
            return;
        };
        on_pick(values[next], window, cx);
        cx.stop_propagation();
    })
}

#[cfg(test)]
mod tests;
