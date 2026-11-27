//! The scroll registry: each frame's scrolling pages, and keeping the focused control in view.

use super::focus::focus_mark;
use super::*;

/// The pages that scroll, for [`Reach::seen`].
#[derive(Default)]
struct Scrolls {
    /// A page's handle lives while the page is drawn, so coming back to a page starts at its top.
    kept: HashMap<&'static str, gpui::ScrollHandle>,
    /// The handles of the frame being drawn, and their names.
    live: Vec<(&'static str, gpui::ScrollHandle)>,
    /// The focused control that was last scrolled to. It is shown once, so the wheel can then
    /// move the page away from it.
    shown: Option<FocusHandle>,
    /// Where the frame being drawn put the focused control, once it has been laid out.
    focused: Option<Bounds<gpui::Pixels>>,
}

thread_local! {
    static SCROLLS: std::cell::RefCell<Scrolls> = std::cell::RefCell::default();
}

/// Starts a frame: the pages that scroll name themselves again as they are built.
pub(crate) fn scrolls_begin() {
    SCROLLS.with_borrow_mut(|s| {
        let live = std::mem::take(&mut s.live);
        s.focused = None;
        s.kept = live
            .into_iter()
            .filter(|(key, _)| !key.is_empty())
            .collect();
    });
}

/// The scroll handle of the page called `key`, for `track_scroll` beside its `overflow_y_scroll`:
/// what lets the keyboard's focus scroll the page.
pub(crate) fn scrolls(key: &'static str) -> gpui::ScrollHandle {
    SCROLLS.with_borrow_mut(|s| {
        let handle = match s.live.iter().find(|(live, _)| *live == key) {
            Some((_, handle)) => return handle.clone(),
            None => s.kept.get(key).cloned().unwrap_or_default(),
        };
        s.live.push((key, handle.clone()));
        handle
    })
}

/// The handle the last frame drew the page called `key` with, if it drew that page. Read
/// between frames: unlike [`scrolls`], it doesn't keep the page's handle for the next one.
pub(crate) fn drawn_scroll(key: &'static str) -> Option<gpui::ScrollHandle> {
    SCROLLS.with_borrow(|s| {
        s.live
            .iter()
            .find(|(live, _)| *live == key)
            .map(|(_, handle)| handle.clone())
    })
}

/// Where the frame being drawn has so far put the focused control, for a list that keeps the
/// row it is in built (`cull`).
pub(crate) fn focused_bounds() -> Option<Bounds<gpui::Pixels>> {
    SCROLLS.with_borrow(|s| s.focused)
}

/// The same for a page that keeps its own handle.
pub(crate) fn scrolls_with(handle: &gpui::ScrollHandle) {
    SCROLLS.with_borrow_mut(|s| s.live.push(("", handle.clone())));
}

/// Scrolls the page a control is on so the control shows, with a little room around it, when
/// the keyboard has just landed on it. `bounds` is where the control is.
pub(crate) fn scroll_to_show(bounds: Bounds<gpui::Pixels>, window: &mut Window, cx: &mut App) {
    const ROOM: f32 = 24.;
    if window.text_style().background_color != Some(focus_mark()) {
        return;
    }
    SCROLLS.with_borrow_mut(|s| s.focused = Some(bounds));
    if !window.last_input_was_keyboard() {
        return;
    }
    let focused = window.focused(cx);
    let mask = window.content_mask().bounds;
    let Some(page) = SCROLLS.with_borrow_mut(|s| {
        if s.shown == focused {
            return None;
        }
        s.shown = focused;
        // The page is the smallest one the control lies in. A control beside the pages (the
        // rail, the play bar, a sheet) is not cut off by any of them.
        s.live
            .iter()
            .map(|(_, page)| page)
            .filter(|page| {
                let view = page.bounds();
                let all = Bounds::new(
                    view.origin + page.offset(),
                    view.size + gpui::Size::from(page.max_offset()),
                );
                all.contains(&bounds.center()) && (mask.is_empty() || view.intersect(&mask) == mask)
            })
            .min_by(|a, b| {
                let area = |page: &gpui::ScrollHandle| {
                    let size = page.bounds().size;
                    f32::from(size.width) * f32::from(size.height)
                };
                area(a).total_cmp(&area(b))
            })
            .cloned()
    }) else {
        return;
    };
    let view = page.bounds();
    let by = if bounds.top() < view.top() + px(ROOM) {
        view.top() + px(ROOM) - bounds.top()
    } else if bounds.bottom() > view.bottom() - px(ROOM) {
        (view.bottom() - px(ROOM) - bounds.bottom()).max(view.top() + px(ROOM) - bounds.top())
    } else {
        return;
    };
    let mut to = page.offset();
    to.y = (to.y + by).clamp(-page.max_offset().y, px(0.));
    if to == page.offset() {
        return;
    }
    // A frame that is being drawn can't ask for another: do it right after.
    let window = window.window_handle();
    cx.defer(move |cx| {
        page.set_offset(to);
        window.update(cx, |_, window, _| window.refresh()).ok();
    });
}
