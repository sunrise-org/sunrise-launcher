//! Buttons: Primary, Secondary, Ghost and Accent outline, in Medium and Small, with an optional
//! leading icon, and the square icon button. Every page draws its buttons from here.
use super::*;
use crate::frame::{c, focus_ring, mix};
use crate::i18n::tr;
use crate::paint::palette;
use crate::tokens;

/// What a button says it does: the fill and edge it wears.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Style {
    /// The one main action of a view: a fill in the accent.
    Primary,
    /// An outlined control: a hairline edge, no fill.
    Secondary,
    /// Text only, until the pointer is over it.
    Ghost,
    /// The accent outline (the "Update" button): the accent as an edge, text in ink.
    Accent,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Size {
    Medium,
    Small,
}

/// A button's look: its style and size, an optional leading icon, and whether it is disabled.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Look {
    pub style: Style,
    pub size: Size,
    pub icon: Option<&'static str>,
    pub disabled: bool,
}

impl Look {
    pub(crate) const fn new(style: Style, size: Size) -> Self {
        Self {
            style,
            size,
            icon: None,
            disabled: false,
        }
    }

    /// The same look with `name` (a glass icon, see [`crate::glass_ui::icon`]) before the label.
    pub(crate) const fn icon(self, name: &'static str) -> Self {
        Self {
            icon: Some(name),
            ..self
        }
    }

    pub(crate) const fn disabled(self, yes: bool) -> Self {
        Self {
            disabled: yes,
            ..self
        }
    }
}

/// Medium is the page's control height; Small sits beside dense rows.
impl Size {
    fn height(self) -> f32 {
        match self {
            Size::Medium => tokens::CONTROL,
            Size::Small => 28.,
        }
    }

    fn pad(self) -> f32 {
        match self {
            Size::Medium => 14.,
            Size::Small => 10.,
        }
    }

    fn gap(self) -> f32 {
        match self {
            Size::Medium => 8.,
            Size::Small => 6.,
        }
    }

    fn icon(self) -> f32 {
        match self {
            Size::Medium => 18.,
            Size::Small => 16.,
        }
    }

    fn text(self) -> f32 {
        match self {
            Size::Medium => tokens::BODY,
            Size::Small => tokens::LABEL,
        }
    }
}

/// Opacity of a disabled button (the Figma look: 45 %).
const DISABLED_OPACITY: f32 = 0.45;

/// The label and icon colour: white on the accent, the text colour everywhere else.
fn ink(style: Style) -> u32 {
    match style {
        Style::Primary => palette().accent_on,
        Style::Secondary | Style::Ghost | Style::Accent => palette().text,
    }
}

/// The edge: a hairline in `line`, or in the accent ink for the accent outline.
fn edge(style: Style) -> Option<u32> {
    match style {
        Style::Secondary => Some(palette().line),
        Style::Accent => Some(palette().accent_ink),
        Style::Primary | Style::Ghost => None,
    }
}

/// The colour the hover fades in over the face. Primary lightens its accent a little toward
/// the hover tint, so it still reads as the accent.
fn hover_colour(style: Style) -> gpui::Hsla {
    match style {
        Style::Primary => mix(c(palette().accent), c(palette().hover), 0.25),
        Style::Secondary | Style::Ghost | Style::Accent => c(palette().hover),
    }
}

/// A button with `label` (translated here, as [`tr`] does) in `look`. The caller attaches the
/// click handler; a disabled button takes no hover, press or keyboard focus, so the caller
/// should not attach one either.
pub(crate) fn button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    look: Look,
) -> Stateful<Div> {
    let label: SharedString = label.into();
    let text = SharedString::from(tr(&label).to_string());
    make(id.into(), look, false, Some(text))
}

/// A square icon button (as tall as a Medium button) showing `icon` centred, with `tooltip` (also
/// translated here) on hover. Its look is a Medium `style` button with the icon alone.
pub(crate) fn icon_button(
    id: impl Into<SharedString>,
    icon: &'static str,
    tooltip: impl Into<SharedString>,
    style: Style,
) -> Stateful<Div> {
    let tooltip: SharedString = tooltip.into();
    let tip = SharedString::from(tr(&tooltip).to_string());
    let look = Look::new(style, Size::Medium).icon(icon);
    make(id.into(), look, true, None).tooltip(crate::glass_ui::tip(tip))
}

/// The shared body of [`button`] and [`icon_button`]: the box, its fill and edge, its hover and
/// press, its icon and label, and the keyboard ring. `square` makes it as wide as it is tall, with
/// no side padding.
fn make(id: SharedString, look: Look, square: bool, text: Option<SharedString>) -> Stateful<Div> {
    let (style, size) = (look.style, look.size);
    let text_ink = c(ink(style));
    let edge_colour = edge(style).map_or(gpui::transparent_black(), c);
    let el = div()
        .id(id.clone())
        .relative()
        .tab_index(0)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(size.gap()))
        .h(px(size.height()))
        .px(px(if square { 0. } else { size.pad() }))
        .when(square, |d| d.w(px(size.height())))
        .rounded(tokens::r(tokens::R_CONTROL))
        .border_1()
        .border_color(edge_colour)
        .text_color(text_ink)
        .text_size(px(size.text()))
        .font_weight(FontWeight::MEDIUM)
        .when(style == Style::Primary, |d| d.bg(c(palette().accent)))
        .focus_visible(|s| s.shadow(focus_ring(palette().focus)));
    if look.disabled {
        return el
            .opacity(DISABLED_OPACITY)
            .cursor_default()
            .tab_stop(false)
            .children(
                look.icon
                    .map(|name| crate::glass_ui::icon(name, size.icon(), text_ink)),
            )
            .children(text);
    }
    let hover = hover_colour(style);
    let layer = hover_fade(
        &id,
        || {
            div()
                .absolute()
                .inset_0()
                .rounded(tokens::r(tokens::R_CONTROL))
        },
        move |d, a| {
            d.bg(gpui::Hsla {
                a: hover.a * a,
                ..hover
            })
        },
    );
    let el = track_hover(el.cursor_pointer(), id.clone())
        .child(layer)
        .children(
            look.icon
                .map(|name| crate::glass_ui::icon(name, size.icon(), text_ink)),
        )
        .children(text);
    press(el, &id, |d| d.top(px(1.)))
}

#[cfg(test)]
mod tests;
