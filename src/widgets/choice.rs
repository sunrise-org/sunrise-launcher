//! The segmented control: one choice among a few, drawn as one box split into segments. It is as
//! tall as a button (`tokens::CONTROL`) and sets its words at a button's size, so it sits level
//! with the buttons beside it.
use super::focus::{c, Reach};
use super::*;
use crate::paint::palette;
use crate::tokens::{self, r, R_CONTROL};
use std::rc::Rc;

/// The least width each segment takes, so short words don't make one smaller than the rest.
const SEGMENT_MIN_WIDTH: f32 = 72.;
/// A segment's side padding: a button's.
const PADDING: f32 = 14.;

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

/// One choice among a few segments (`options`: the value each stands for, and its label), with
/// `current` on. The group is one Tab stop and its edge takes the focus colour. The arrow keys
/// choose the segment before or after `current`, as a click on it would, and a click chooses the
/// segment clicked. `on_pick` runs with the value chosen; it must redraw whatever shows the
/// choice.
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
    let p = palette();
    let inner = r(R_CONTROL) - px(1.);
    let last = options.len().saturating_sub(1);
    let mut group = div()
        .id(SharedString::from(id))
        .tab_index(0)
        .seen()
        .flex_none()
        .flex()
        .items_center()
        .h(px(tokens::CONTROL))
        .rounded(r(R_CONTROL))
        .border_1()
        .border_color(c(p.line))
        .focus_visible(|s| s.border_color(c(palette().focus)));
    for (n, (value, label)) in options.iter().enumerate() {
        let value = *value;
        let on = value == current;
        let pick = on_pick.clone();
        let mut segment = div()
            .id(SharedString::from(format!("{id}-{n}")))
            .h_full()
            .min_w(px(SEGMENT_MIN_WIDTH))
            .px(px(PADDING))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_size(px(tokens::BODY))
            .text_color(c(if on { p.text } else { p.muted }))
            .when(n > 0, |d| d.border_l_1().border_color(c(p.divider)))
            .when(n == 0, |d| d.rounded_tl(inner).rounded_bl(inner))
            .when(n == last, |d| d.rounded_tr(inner).rounded_br(inner));
        segment = if on {
            segment.bg(c(p.selected))
        } else {
            segment.hover(|s| s.bg(c(palette().hover)))
        };
        group = group.child(
            segment
                .child(label.clone())
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

/// The compact segmented control: like [`choices`], but only as wide as its words, on the
/// chrome's darker fill, with the chosen segment raised. Each option may lead with a small bar
/// glyph (`bar`: its width in pixels, 12 tall) drawn in the accent ink when chosen and in a dim
/// muted tone otherwise. Keys and clicks work as in [`choices`].
pub(crate) fn compact_choices<P: Copy + PartialEq + 'static>(
    id: &str,
    options: &[(P, SharedString, Option<f32>)],
    current: P,
    on_pick: impl Fn(P, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let at = options
        .iter()
        .position(|(value, _, _)| *value == current)
        .unwrap_or(0);
    let values: Vec<P> = options.iter().map(|(value, _, _)| *value).collect();
    let on_pick = Rc::new(on_pick);
    let p = palette();
    let inner = r(R_CONTROL) - px(3.);
    let mut group = div()
        .id(SharedString::from(id))
        .tab_index(0)
        .seen()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(2.))
        .h(px(tokens::CONTROL))
        .p(px(2.))
        .rounded(r(R_CONTROL))
        .border_1()
        .border_color(c(p.line))
        .bg(c(p.chrome))
        .focus_visible(|s| s.border_color(c(palette().focus)));
    for (n, (value, label, bar)) in options.iter().enumerate() {
        let value = *value;
        let on = value == current;
        let pick = on_pick.clone();
        let mut segment = div()
            .id(SharedString::from(format!("{id}-{n}")))
            .h_full()
            .px(px(12.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .rounded(inner)
            .cursor_pointer()
            .text_size(px(tokens::BODY))
            .text_color(c(if on { p.text } else { p.muted }));
        segment = if on {
            segment.bg(c(p.raised))
        } else {
            segment.hover(|s| s.bg(c(palette().hover)))
        };
        if let Some(width) = *bar {
            segment = segment.child(
                div()
                    .flex_none()
                    .w(px(width))
                    .h(px(12.))
                    .rounded(px(2.))
                    .bg(if on {
                        c(p.accent_ink)
                    } else {
                        c(p.muted).opacity(0.45)
                    }),
            );
        }
        group = group.child(
            segment
                .child(label.clone())
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
