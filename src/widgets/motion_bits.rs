//! Small shared animations: the copy button's tick, and the chevron that turns as a menu or section opens.
#![allow(dead_code)] // Used by the shell and pages that the next redesign steps move over; removed in the final sweep.
use super::*;

use std::cell::RefCell;
use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, PI};

use gpui::{px, rgb, AnimationExt, AnyElement, ElementId, Hsla, InteractiveElement, IntoElement};
use sunrise_launcher_theme::{Motion, MOVE};

use crate::frame::{c, focus_ring, Reach};
use crate::paint::palette;

/// The copy tick's pop: it grows from a little under its size and fades in.
const POP: Motion = Motion {
    name: "pop",
    ms: 150,
    curve: [0.2, 0.7, 0.2, 1.],
};

/// A chevron's turn between closed and open.
const TURN: Motion = Motion {
    name: "turn",
    ms: 180,
    curve: MOVE.curve,
};

thread_local! {
    /// Each chevron's last state (open or not) and how often it has changed, so it turns only
    /// when its menu opens or closes, never as a page is first drawn.
    static FOLDS: RefCell<HashMap<SharedString, (bool, usize)>> = RefCell::new(HashMap::new());
}

/// How often the chevron `id` has turned, drawn now with `open`: the first time it is drawn it
/// hasn't turned, whatever the state; each change after that is one more turn.
fn turns_after(
    folds: &mut HashMap<SharedString, (bool, usize)>,
    id: SharedString,
    open: bool,
) -> usize {
    let (was, turns) = folds.entry(id).or_insert((open, 0));
    if *was != open {
        *was = open;
        *turns += 1;
    }
    *turns
}

/// A copy button's face: `label` with a copy glyph, or "Copied" with a tick while copy `shown`
/// (its count) shows. The focusable element stays in place while its label and glyph change, so
/// keyboard focus stays on it. Ghost small: no fill, a `hover` fill that eases in.
pub(crate) fn copy_face(id: &'static str, label: &str, shown: Option<usize>) -> Stateful<Div> {
    let name = SharedString::from(id);
    let text = shown.map_or_else(|| label.to_string(), |_| tr("Copied").to_string());
    let fill = hover_fade(
        &name,
        || {
            div()
                .absolute()
                .inset_0()
                .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
        },
        |d, a| {
            let hover: Hsla = c(palette().hover);
            d.bg(hover.opacity(hover.a * a))
        },
    );
    let glyph = match shown {
        Some(n) => tick(&name, n, 16., c(palette().success)),
        None => crate::glass_ui::icon("copy-01", 16., c(palette().text)).into_any_element(),
    };
    let face = div()
        .id(id)
        .tab_index(0)
        .relative()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .h(px(28.))
        .px(px(10.))
        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
        .text_color(rgb(palette().text))
        .text_size(px(crate::tokens::LABEL))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .focus_visible(|s| s.shadow(focus_ring(palette().focus)))
        .seen()
        .child(fill)
        .child(glyph)
        .child(text);
    track_hover(face, name)
}

/// A button that copies `text`, then shows the copy on itself for a moment, as [`copy_face`].
/// The copy is remembered with `text`, so a new text (another log, a new report) starts at
/// `label` again. Copies without a toast: the button shows the copy itself.
pub(crate) fn copy_button(id: &'static str, label: &str, text: String) -> Stateful<Div> {
    let key = copy_key(id, &text);
    copy_face(id, label, crate::code_box::copied(&key)).on_click(move |_, window, cx| {
        crate::widgets::copy_quietly(cx, gpui::ClipboardItem::new_string(text.clone()));
        crate::code_box::note_copy(key.clone(), window, cx);
    })
}

/// The key a copy button's copies are counted under: its id and a hash of its text.
fn copy_key(id: &str, text: &str) -> SharedString {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{id}:{:016x}", hasher.finish()).into()
}

/// The tick that pops in for copy `n` of `id`: a new copy is a new element, so it pops again.
fn tick(id: &SharedString, n: usize, size: f32, color: Hsla) -> AnyElement {
    crate::glass_ui::icon("tick-02", size, color)
        .with_animation(
            ElementId::named_usize(SharedString::from(format!("{id}-tick")), n),
            crate::motion::eased(POP),
            move |g, t| g.size(px(size * (0.6 + 0.4 * t))).opacity(t),
        )
        .into_any_element()
}

/// A menu's chevron: pointing down while closed and up while open, turning a half turn between
/// the two as it opens and closes. `id` names it among the others.
pub(crate) fn chevron(
    id: impl Into<SharedString>,
    open: bool,
    size: f32,
    color: Hsla,
) -> AnyElement {
    turning(id.into(), open, "arrow-down-01", PI, size, color)
}

/// A section's chevron: pointing right while folded and down while open, turning a quarter
/// between the two, as [`chevron`] turns.
pub(crate) fn disclosure(
    id: impl Into<SharedString>,
    open: bool,
    size: f32,
    color: Hsla,
) -> AnyElement {
    turning(id.into(), open, "arrow-right-01", FRAC_PI_2, size, color)
}

/// `glyph` turned by `turn` (radians, clockwise) while open, easing between closed and open each
/// time `open` changes after its first frame. Under reduced motion it jumps to its end.
fn turning(
    id: SharedString,
    open: bool,
    glyph: &'static str,
    turn: f32,
    size: f32,
    color: Hsla,
) -> AnyElement {
    let turns = FOLDS.with_borrow_mut(|folds| turns_after(folds, id.clone(), open));
    let at = move |t: f32| gpui::Transformation::rotate(gpui::radians(turn * t));
    let chevron = crate::glass_ui::icon(glyph, size, color);
    if turns == 0 || crate::motion::still() {
        return chevron
            .with_transformation(at(if open { 1. } else { 0. }))
            .into_any_element();
    }
    chevron
        .with_animation(
            ElementId::Name(format!("{id}-turn-{turns}").into()),
            crate::motion::eased(TURN),
            move |d, t| d.with_transformation(at(if open { t } else { 1. - t })),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests;
