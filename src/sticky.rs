//! A panel that stays in view beside a long column as the page scrolls (CSS `position: sticky;
//! top: 0`, which GPUI has no style for): a mod page's side panel.
//!
//! The panel scrolls with the page until its top reaches the top of the view, then is drawn moved
//! down (`top`, which takes nothing from the layout) by how far the page has scrolled past the
//! row's top, never past the row's end.

use gpui::ScrollHandle;

#[derive(Clone, Default)]
pub struct Sticky {
    row: ScrollHandle,
}

impl Sticky {
    /// For the row's `track_scroll`: it records how tall the panel, the row's second child, is.
    pub fn row(&self) -> &ScrollHandle {
        &self.row
    }

    /// Forgets the row (the page left), so a new one starts unmoved.
    pub fn reset(&mut self) {
        self.row = ScrollHandle::new();
    }

    /// How far down to draw the panel. `page` scrolls the row, its child `index`; both handles
    /// hold the last frame's layout, which is unscrolled.
    pub fn shift(&self, page: &ScrollHandle, index: usize) -> f32 {
        let (Some(row), Some(panel)) = (page.bounds_for_item(index), self.row.bounds_for_item(1))
        else {
            return 0.;
        };
        // The wheel moves the offset past either end until the next layout clamps it.
        let scrolled = f32::from(-page.offset().y).clamp(0., f32::from(page.max_offset().y));
        let past = scrolled - f32::from(row.top() - page.bounds().top());
        let room = f32::from(row.size.height - panel.size.height).max(0.);
        past.clamp(0., room)
    }
}
