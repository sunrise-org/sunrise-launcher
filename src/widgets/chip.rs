//! Chips: filter chips, source chips (Sunrise, Modrinth, GitHub) and tags.
#![allow(dead_code)] // Used by the shell and pages that the next redesign steps move over; removed in the final sweep.
use super::focus::c;
use super::*;
use crate::paint::palette;

/// Modrinth's brand green (#1BD96A). A brand colour, so the one hex value a chip is drawn with.
pub(crate) const MODRINTH: u32 = 0x1bd96a;

/// A chip's height, padding and gap between its parts, as the Figma file draws them.
const HEIGHT: f32 = 24.;
const PADDING: f32 = 10.;
const GAP: f32 = 4.;
/// The leading icon and the chevron.
const ICON: f32 = 14.;

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
