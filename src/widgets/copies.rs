//! Which copy buttons just copied: a copy shows a tick for a moment, then the button eases back.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{SharedString, Window};

/// How long the tick shows after a copy.
const SHOWN: Duration = Duration::from_millis(1400);

thread_local! {
    /// Each box's last copy: when, and how many so far (a new copy restarts the pop).
    static COPIED: RefCell<HashMap<SharedString, (Instant, usize)>> = RefCell::new(HashMap::new());
}

/// How many copies `id` has had, while its last one is still showing.
pub(crate) fn copied(id: &SharedString) -> Option<usize> {
    COPIED.with_borrow(|m| {
        m.get(id)
            .filter(|(at, _)| at.elapsed() < SHOWN)
            .map(|&(_, n)| n)
    })
}

/// Records a copy from `id`'s button, so [`copied`] shows it, and redraws once it has shown.
pub(crate) fn note_copy(id: SharedString, window: &mut Window, cx: &mut gpui::App) {
    let n = remember_copy(&id);
    // The view isn't known outside a draw, so redraw the window.
    window.refresh();
    // Back to the copy glyph once the tick has shown, unless another copy came since.
    window
        .spawn(cx, async move |cx| {
            cx.background_executor().timer(SHOWN).await;
            let last = COPIED.with_borrow(|m| m.get(&id).map(|&(_, n)| n));
            if last == Some(n) {
                let _ = cx.update(|window, _| window.refresh());
            }
        })
        .detach();
}

/// Counts a copy from `id`'s button and returns its number, for [`copied`].
pub(crate) fn remember_copy(id: &SharedString) -> usize {
    COPIED.with_borrow_mut(|m| {
        let e = m.entry(id.clone()).or_insert((Instant::now(), 0));
        *e = (Instant::now(), e.1 + 1);
        e.1
    })
}
