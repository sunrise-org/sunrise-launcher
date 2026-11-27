//! Navigation parts: underlined tabs with a sliding bar (a profile page's sub-pages, Installed |
//! Discover, Join | Host), the sidebar's nav row, and the pixel sun its selected row ends with.
use super::*;
use crate::frame::{c, mix, Reach};
use crate::{glass_ui, motion, paint::palette, tokens};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

// The sun is the Sunrise mark's own: these colours are the logo's, the same in every theme, so
// they are not palette roles.
/// The sun's rays and its top pixels.
pub(crate) const SUN_RAY: u32 = 0xF6E7B0;
/// The sun's upper glow.
pub(crate) const SUN_GLOW: u32 = 0xF9E08A;
/// The sun's lower glow.
pub(crate) const SUN_CORE: u32 = 0xF4CF63;
/// The sun's horizon, the band it sits on.
pub(crate) const SUN_HORIZON: u32 = 0xE0457E;

/// The pixel sun's width, in px (its height is 10 px).
pub(crate) const SUN_WIDTH: f32 = 14.;
const SUN_HEIGHT: f32 = 10.;
/// The room between a label and the sun at the end of its row.
const SUN_GAP: f32 = 8.;

/// The pixel sun's squares: (x, y, width, height, colour) in its own 14×10 box, 2 px tall rows.
const SUN_CELLS: [(f32, f32, f32, f32, u32); 6] = [
    (6., 0., 2., 2., SUN_RAY),
    (1., 1., 2., 2., SUN_RAY),
    (11., 1., 2., 2., SUN_RAY),
    (4., 4., 6., 2., SUN_GLOW),
    (2., 6., 10., 2., SUN_CORE),
    (0., 8., 14., 2., SUN_HORIZON),
];

/// The pixel sun (the Sunrise mark's sun), 14×10, drawn from 2 px squares.
pub(crate) fn sun() -> Div {
    let mut sun = div()
        .relative()
        .flex_none()
        .w(px(SUN_WIDTH))
        .h(px(SUN_HEIGHT));
    for (x, y, w, h, colour) in SUN_CELLS {
        sun = sun.child(
            div()
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(w))
                .h(px(h))
                .bg(gpui::rgb(colour)),
        );
    }
    sun
}

/// Whether a label `label_width` px wide, at full length, with the sun after it fits in `room`.
/// The caller shows the sun only when this holds; the label is never shortened for it.
pub(crate) fn sun_fits(label_width: f32, room: f32) -> bool {
    label_width + SUN_GAP + SUN_WIDTH <= room
}

thread_local! {
    /// Each tab row's selected tab, and the one the selection last moved from: the way its bars
    /// sweep.
    static MOVES: RefCell<HashMap<String, (usize, usize)>> = RefCell::new(HashMap::new());
}

/// The tab the selection of row `id` came from, when it moved to `now`. A row whose selection
/// never moved reports `now` itself.
fn moved_from(id: &str, now: usize) -> usize {
    MOVES.with_borrow_mut(|moves| {
        let entry = moves.entry(id.to_string()).or_insert((now, now));
        if entry.0 != now {
            *entry = (now, entry.0);
        }
        entry.1
    })
}

/// Underlined tabs for a page's sub-pages: the selected one has a 2 px accent underline that
/// slides to the tab picked. The row is one Tab stop (the arrow keys move the selection, wrapping
/// at the ends) and a click picks a tab. `on_pick` opens the tab; the caller redraws.
pub(crate) fn tabs<P: Copy + PartialEq + 'static>(
    id: &str,
    tabs: &[(P, SharedString)],
    current: P,
    on_pick: impl Fn(P, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let pick: Rc<dyn Fn(P, &mut Window, &mut App)> = Rc::new(on_pick);
    let picks: Vec<P> = tabs.iter().map(|(p, _)| *p).collect();
    let now = picks.iter().position(|p| *p == current);
    let from = moved_from(id, now.unwrap_or(usize::MAX));
    let key_pick = pick.clone();
    let mut row = div()
        .id(SharedString::from(id.to_owned()))
        .tab_index(0)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(28.))
        .px(px(4.))
        .mx(px(-4.))
        .mb(px(-2.))
        .rounded(tokens::r(tokens::R_SMALL))
        .border_1()
        .border_color(gpui::transparent_black())
        .focus_visible(|s| s.border_color(c(palette().focus)))
        .on_key_down(move |ev: &KeyDownEvent, window, cx| {
            let step: isize = match ev.keystroke.key.as_str() {
                "left" | "up" => -1,
                "right" | "down" => 1,
                _ => return,
            };
            cx.stop_propagation();
            if let Some(at) = now {
                let next = (at as isize + step).rem_euclid(picks.len() as isize) as usize;
                key_pick(picks[next], window, cx);
            }
        });
    for (idx, (target, label)) in tabs.iter().enumerate() {
        let on = *target == current;
        let key = format!("{id}-{idx}");
        let slide = motion::sliding(&key, on);
        // The new tab grows out from the side the selection came from; the old one leaves towards
        // the side it went to.
        let rightward = if on {
            from < idx
        } else {
            now.is_some_and(|n| n > idx)
        };
        let item = tab_item(key, label, *target, on, slide, rightward, pick.clone());
        row = row.child(item);
    }
    row
}

/// One tab of [`tabs`]: its label, and its underline while the selection is sliding or rests on
/// it.
fn tab_item<P: Copy + 'static>(
    key: String,
    label: &SharedString,
    target: P,
    on: bool,
    slide: Option<usize>,
    rightward: bool,
    pick: Rc<dyn Fn(P, &mut Window, &mut App)>,
) -> Stateful<Div> {
    let p = palette();
    let (active, muted) = (c(p.text), c(p.muted));
    let bar = match slide {
        Some(slide) => Some(sweep(&key, slide, on, rightward)),
        None if on => Some(underline().left_0().right_0().into_any_element()),
        None => None,
    };
    let label_el = match slide {
        Some(slide) => div()
            .child(label.clone())
            .with_animation(
                ElementId::named_usize(format!("{key}-label"), slide),
                motion::slide_motion(),
                move |d, v| {
                    let v = if on { v } else { 1. - v };
                    d.text_color(mix(muted, active, v))
                },
            )
            .into_any_element(),
        None => div().child(label.clone()).into_any_element(),
    };
    div()
        .id(SharedString::from(format!("{key}-tab")))
        .relative()
        .pt(px(10.))
        .cursor_pointer()
        .text_size(px(tokens::BODY))
        .font_weight(FontWeight::MEDIUM)
        .when(slide.is_none() && on, |d| d.text_color(active))
        .when(slide.is_none() && !on, |d| {
            d.text_color(muted).hover(move |s| s.text_color(active))
        })
        // The underline sits 8 px below the label: the label's box keeps 10 px under it, and the
        // 2 px underline is its last 2.
        .child(div().relative().pb(px(10.)).child(label_el).children(bar))
        .on_click(move |_: &ClickEvent, window, cx| {
            if !on {
                pick(target, window, cx);
            }
        })
}

/// The underline's base: 2 px, at the foot of its label, in the accent's ink.
fn underline() -> Div {
    div()
        .absolute()
        .bottom_0()
        .h(px(2.))
        .rounded(tokens::r(tokens::R_BAR))
        .bg(c(palette().accent_ink))
}

/// The underline of a tab while the selection slides: the new one grows out of the old one's
/// side, the old one shrinks away towards the new one, both in the direction of the move.
fn sweep(key: &str, slide: usize, on: bool, rightward: bool) -> AnyElement {
    underline()
        .with_animation(
            ElementId::named_usize(format!("{key}-bar"), slide),
            motion::slide_motion(),
            move |d, p| match (on, rightward) {
                (true, true) => d.left_0().right(gpui::relative(1. - p)).opacity(p),
                (true, false) => d.right_0().left(gpui::relative(1. - p)).opacity(p),
                (false, true) => d.right_0().left(gpui::relative(p)).opacity(1. - p),
                (false, false) => d.left_0().right(gpui::relative(p)).opacity(1. - p),
            },
        )
        .into_any_element()
}

/// A sidebar nav row: its icon and label, 36 px tall. Not selected, it has no fill and lights
/// `hover` (at 60 %) as the pointer comes to it; selected, it is filled `selected` in full text
/// colour, with the pixel sun at its end when `show_sun`. The caller adds the click handler and
/// decides `show_sun` with [`sun_fits`].
pub(crate) fn nav_row(
    id: impl Into<SharedString>,
    icon: &'static str,
    label: impl Into<SharedString>,
    selected: bool,
    show_sun: bool,
) -> Stateful<Div> {
    let id: SharedString = id.into();
    let p = palette();
    let ink = c(if selected { p.text } else { p.muted });
    let mut row = div()
        .id(ElementId::from(id.clone()))
        .reach()
        .rounded(tokens::r(tokens::R_CONTROL))
        .relative()
        .h(px(36.))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(12.))
        .cursor_pointer();
    if selected {
        row = row.bg(gpui::rgb(p.selected));
    } else {
        row = track_hover(row, id.clone()).child(hover_layer(
            &id,
            c(p.hover).opacity(0.6),
            tokens::R_CONTROL,
            0.,
        ));
    }
    row.child(glass_ui::icon(icon, 20., ink))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(tokens::LABEL))
                .text_color(ink)
                .child(label.into()),
        )
        // The sun's gap is 8 px (see `sun_fits`): the row's 10 px gap less the 2 px it is pulled in.
        .when(selected && show_sun, |d| d.child(sun().ml(px(-2.))))
}

#[cfg(test)]
mod tests;
