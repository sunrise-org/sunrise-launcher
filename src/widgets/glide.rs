//! Rows of a list that glide to their new place when one moves: the sidebar's profiles and
//! Home's saved servers share this. The list notes its order each frame, [`swapped`] says when
//! two neighbours traded places, and [`glide`] draws each row held off its place at the start of
//! the glide. [`arrive`] and [`leave`] do the same for a row that is added or removed.
use gpui::{div, prelude::*, px, AnimationExt, AnyElement, ElementId, SharedString};

/// The two places (the first and its next) that differ between `before` and `now`, when `now` is
/// `before` with two neighbours swapped: how a row moves up or down. None for any other change.
pub(crate) fn swapped(before: &[String], now: &[String]) -> Option<(usize, usize)> {
    let changed: Vec<usize> = (0..now.len().min(before.len()))
        .filter(|n| now[*n] != before[*n])
        .collect();
    let [first, second] = changed[..] else {
        return None;
    };
    (now.len() == before.len()
        && second == first + 1
        && now[first] == before[second]
        && now[second] == before[first])
        .then_some((first, second))
}

/// `entry` held `from` px off its place at the start of a glide, easing to it; at once under
/// reduced motion. Layout moves with it, so the row stays clickable and its hover goes along.
/// `scope` and `id` name the row, and `moved` counts the move, so each glide plays once.
pub(crate) fn glide(
    entry: impl IntoElement,
    scope: &str,
    id: &str,
    (moved, from): (usize, f32),
) -> AnyElement {
    let row = div().flex_none().relative().child(entry);
    // A row that never moved is simply there: an animation on every row would keep the window
    // drawing frames (and a benchmark counting them) for nothing.
    if moved == 0 || from == 0. || !crate::motion::arrivals() {
        return row.into_any_element();
    }
    row.with_animation(
        ElementId::named_usize(SharedString::from(format!("{scope}-{id}")), moved),
        crate::motion::eased(sunrise_launcher_theme::MOVE),
        move |d, progress| d.top(px(from * (1. - progress))),
    )
    .into_any_element()
}

/// `entry` fading in while it slides `rise` px down into its place, once, as the row `id` that
/// was added in `born` (a count of changes to the list). Plain at once under reduced motion or
/// while a benchmark runs.
pub(crate) fn arrive(
    entry: impl IntoElement,
    scope: &str,
    id: &str,
    born: usize,
    rise: f32,
) -> AnyElement {
    let row = div().flex_none().relative().child(entry);
    if born == 0 || !crate::motion::arrivals() {
        return row.into_any_element();
    }
    row.with_animation(
        ElementId::named_usize(SharedString::from(format!("{scope}-in-{id}")), born),
        crate::motion::eased(sunrise_launcher_theme::MOVE),
        move |d, progress| d.opacity(progress).top(px(-rise * (1. - progress))),
    )
    .into_any_element()
}

/// What is left of a removed row, `height` px tall with `gap` px of the list's gap beside it:
/// it fades and folds shut, and the rows after it follow its height up. Nothing under reduced
/// motion or while a benchmark runs, where the row is simply gone.
pub(crate) fn leave(
    ghost: impl IntoElement,
    scope: &str,
    id: &str,
    gone: usize,
    (height, gap): (f32, f32),
) -> Option<AnyElement> {
    if !crate::motion::arrivals() {
        return None;
    }
    let shell = div()
        .flex_none()
        .overflow_hidden()
        .h(px(height))
        .child(ghost);
    Some(
        shell
            .with_animation(
                ElementId::named_usize(SharedString::from(format!("{scope}-out-{id}")), gone),
                crate::motion::eased(sunrise_launcher_theme::MOVE),
                move |d, progress| {
                    let open = 1. - progress;
                    d.h(px(height * open)).opacity(open).mb(px(-gap * progress))
                },
            )
            .into_any_element(),
    )
}
