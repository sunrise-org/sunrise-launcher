//! Variable-height lists in the page's scroll area. Only nearby rows build elements;
//! unseen rows use estimated heights and consecutive hidden rows share one spacer.
use gpui::{
    div, point, px, size, Bounds, Div, IntoElement, ParentElement, Pixels, ScrollHandle, Styled,
};
use std::{cell::RefCell, rc::Rc};

/// A row's height before any row of the list has been measured.
const ESTIMATED_HEIGHT: f32 = 96.;
/// Until the enclosing scroll area completes its first layout.
const INITIAL_VIEW_HEIGHT: f32 = 800.;

#[derive(Clone, Default)]
pub struct Cull(Rc<RefCell<Drawn>>);

#[derive(Default)]
struct Drawn {
    heights: Vec<Pixels>,
    /// Which of `heights` were measured; the rest are the list's estimate.
    measured: Vec<bool>,
    top: Pixels,
    offset: Pixels,
    width: Pixels,
    /// What the list held when it was measured ([`Cull::items`]).
    items: Option<u64>,
    /// The row the keyboard's focus was in when last drawn: it stays built, so Tab goes on
    /// from it however far the list has scrolled.
    focused: Option<usize>,
    /// Whether the last drawn frame left nothing to correct (no mismeasured row and no visible
    /// spacer). Starts false until a frame has been measured.
    settled: bool,
}

#[derive(Debug)]
enum Span {
    Row(usize),
    /// Hidden rows, this tall together.
    Space(Pixels),
}

impl Cull {
    /// Invalidate measurements when filtering, ordering or collection identity changes.
    pub fn reset(&self) {
        *self.0.borrow_mut() = Drawn::default();
    }

    /// Forget the measurements when the list now holds other items than when they were taken:
    /// `items` names what it holds, in order (a hash of their identities). A heading where a row
    /// was, or another row, would otherwise inherit that one's height.
    pub fn items(&self, items: u64) {
        let mut drawn = self.0.borrow_mut();
        if drawn.items != Some(items) {
            *drawn = Drawn {
                items: Some(items),
                ..Drawn::default()
            };
        }
    }

    /// Render nearby rows plus those retained by `keep` (such as active confirmations, or a row
    /// another page asked to reveal) and the one the keyboard's focus is in.
    /// `gap` is the bottom margin already supplied by each row.
    pub fn column<E: IntoElement>(
        &self,
        handle: &ScrollHandle,
        rows: usize,
        gap: f32,
        keep: impl Fn(usize) -> bool,
        mut row: impl FnMut(usize) -> E,
    ) -> Div {
        let max = handle.max_offset().y;
        if max > px(0.)
            && handle.offset().y <= -max + px(0.5)
            && self.0.borrow().heights.len() == rows
        {
            // Measuring taller rows at the end must keep the final row in view. Appending
            // results deliberately leaves the current viewport in place instead.
            handle.scroll_to_bottom();
        }
        let spans = {
            let mut drawn = self.0.borrow_mut();
            let focused = drawn.focused;
            drawn.plan(handle.bounds(), handle.offset().y, rows, gap, |index| {
                keep(index) || retain_for_focus(focused, index)
            })
        };
        let children = spans
            .iter()
            .map(|span| match span {
                // The wrapper supplies the row's gap as bottom padding.
                Span::Row(index) => {
                    let mut d = div().flex().flex_col().flex_none();
                    if gap > 0. {
                        d = d.pb(px(gap));
                    }
                    d.child(row(*index))
                }
                Span::Space(height) => div().flex_none().h(*height),
            })
            .collect::<Vec<_>>();
        let state = self.0.clone();
        let scroll = handle.clone();
        div()
            .flex()
            .flex_col()
            .children(children)
            .on_children_prepainted(move |bounds, window, cx| {
                let mut drawn = state.borrow_mut();
                drawn.focused = crate::frame::focused_bounds().and_then(|focus| {
                    spans
                        .iter()
                        .zip(&bounds)
                        .find_map(|(span, bounds)| match span {
                            Span::Row(index) if bounds.contains(&focus.center()) => Some(*index),
                            _ => None,
                        })
                });
                let unsettled = drawn.record(&spans, &bounds, scroll.bounds(), scroll.offset().y);
                drawn.settled = !unsettled;
                if unsettled {
                    // A frame being drawn can't ask for the next one itself: ask right after.
                    let window = window.window_handle();
                    cx.defer(move |cx| {
                        window.update(cx, |_, window, _| window.refresh()).ok();
                    });
                }
            })
    }

    /// Whether the last drawn frame left nothing to correct (no mismeasured row and no visible
    /// spacer).
    pub(crate) fn settled(&self) -> bool {
        self.0.borrow().settled
    }
}

impl Drawn {
    /// What a row not yet measured is taken to be: the mean of those measured, else
    /// [`ESTIMATED_HEIGHT`]. Rows of one list are much alike, so a row far down lands near where
    /// it will be once the rows above it are drawn.
    fn estimate(&self, gap: f32) -> Pixels {
        let (sum, count) = self
            .heights
            .iter()
            .zip(&self.measured)
            .filter(|(_, measured)| **measured)
            .fold((px(0.), 0usize), |(sum, count), (height, _)| {
                (sum + *height, count + 1)
            });
        if count == 0 {
            px(ESTIMATED_HEIGHT + gap)
        } else {
            sum / count as f32
        }
    }

    fn plan(
        &mut self,
        mut view: Bounds<Pixels>,
        offset: Pixels,
        rows: usize,
        gap: f32,
        keep: impl Fn(usize) -> bool,
    ) -> Vec<Span> {
        if self.width != view.size.width {
            // A resize changes wrapping, including in rows currently off screen.
            self.measured.fill(false);
        }
        self.heights.truncate(rows);
        self.measured.resize(rows, false);
        let estimate = self.estimate(gap);
        self.heights.resize(rows, estimate);
        for (height, measured) in self.heights.iter_mut().zip(&self.measured) {
            if !measured {
                *height = estimate;
            }
        }
        if view.size.height <= px(0.) {
            view.size.height = px(INITIAL_VIEW_HEIGHT);
        }
        let mut top = self.top + offset - self.offset;
        let mut spans = Vec::new();
        for (index, height) in self.heights.iter().copied().enumerate() {
            let bounds = Bounds::new(point(view.left(), top), size(view.size.width, height));
            if !away(bounds, view) || keep(index) {
                spans.push(Span::Row(index));
            } else if let Some(Span::Space(total)) = spans.last_mut() {
                *total += height;
            } else {
                spans.push(Span::Space(height));
            }
            top += height;
        }
        spans
    }

    /// A moved header or scrollbar jump can expose a spacer: request another frame when
    /// measurements or the estimated visible range need correcting.
    fn record(
        &mut self,
        spans: &[Span],
        bounds: &[Bounds<Pixels>],
        view: Bounds<Pixels>,
        offset: Pixels,
    ) -> bool {
        let mut changed = false;
        for (span, bounds) in spans.iter().zip(bounds) {
            match span {
                Span::Row(index) => {
                    let height = &mut self.heights[*index];
                    changed |= (*height - bounds.size.height).abs() > px(0.5);
                    *height = bounds.size.height;
                    self.measured[*index] = true;
                }
                Span::Space(_) => changed |= !away(*bounds, view),
            }
        }
        self.top = bounds.first().map_or(view.top(), Bounds::top);
        self.offset = offset;
        self.width = view.size.width;
        changed
    }
}

/// Keep the focused row and its immediate neighbors (up to two indices away, to bridge a
/// single non-focusable heading or gap between rows) built so Tab and Shift+Tab step across
/// spacers after a scroll jump instead of leaving the list.
fn retain_for_focus(focused: Option<usize>, index: usize) -> bool {
    focused.is_some_and(|f| index.abs_diff(f) <= 2)
}

/// One viewport of overdraw above and below the visible area.
fn away(row: Bounds<Pixels>, view: Bounds<Pixels>) -> bool {
    row.bottom() < view.top() - view.size.height || row.top() > view.bottom() + view.size.height
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(top: f32, height: f32, width: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(0.), px(top)), size(px(width), px(height)))
    }

    fn rendered(spans: &[Span]) -> Vec<usize> {
        spans
            .iter()
            .filter_map(|span| match span {
                Span::Row(index) => Some(*index),
                Span::Space(_) => None,
            })
            .collect()
    }

    #[test]
    fn ten_thousand_rows_build_only_nearby_elements_on_the_first_frame() {
        let list = Cull::default();
        let mut count = 0;
        list.column(
            &ScrollHandle::new(),
            10_000,
            8.,
            |_| false,
            |_| {
                count += 1;
                div().h(px(96.)).mb(px(8.))
            },
        );
        assert!(count <= 17, "built {count} rows before the first layout");
        let spans = list
            .0
            .borrow_mut()
            .plan(at(0., 800., 600.), px(0.), 10_000, 8., |_| false);
        assert!(spans.len() <= count + 1, "hidden rows share one spacer");
    }

    #[test]
    fn large_scroll_jumps_reach_the_last_rows_and_return_to_the_first() {
        let mut state = Drawn::default();
        let view = at(0., 500., 600.);
        state.plan(view, px(0.), 10_000, 8., |_| false);
        state.width = view.size.width;
        let end = state.plan(view, px(-1_039_500.), 10_000, 8., |_| false);
        assert!(rendered(&end).contains(&9_999));
        assert!(rendered(&end).len() < 20);
        assert_eq!(
            rendered(&state.plan(view, px(0.), 10_000, 8., |_| false))[0],
            0
        );
    }

    #[test]
    fn resizing_and_appending_keep_render_work_bounded_and_retained_rows_available() {
        let mut state = Drawn::default();
        let view = at(0., 500., 600.);
        state.plan(view, px(0.), 5_000, 8., |_| false);
        state.width = view.size.width;
        state.heights[100] = px(900.);
        let spans = state.plan(at(0., 500., 300.), px(0.), 10_000, 8., |index| {
            index == 9_000
        });
        assert_eq!(state.heights[100], px(104.));
        assert!(rendered(&spans).contains(&9_000));
        assert!(spans.len() < 20);
        assert_eq!(state.heights.len(), 10_000);
    }

    #[test]
    fn unmeasured_rows_are_taken_to_be_as_tall_as_the_measured_ones() {
        let mut state = Drawn::default();
        let view = at(0., 500., 600.);
        state.plan(view, px(0.), 1_000, 0., |_| false);
        assert_eq!(
            state.heights[500],
            px(ESTIMATED_HEIGHT),
            "nothing measured yet"
        );
        let spans: Vec<_> = (0..4).map(Span::Row).collect();
        let drawn: Vec<_> = (0..4).map(|n| at(n as f32 * 53., 53., 600.)).collect();
        state.record(&spans, &drawn, view, px(0.));
        state.plan(view, px(0.), 1_000, 0., |_| false);
        // A row far down lands about where it will be once the rows above it are drawn.
        assert_eq!(state.heights[500], px(53.));
        let top_of_500: Pixels = state.heights[..500].iter().copied().sum();
        assert_eq!(top_of_500, px(500. * 53.));
    }

    #[test]
    fn focused_row_keeps_adjacent_rows_built_across_spacers_after_scroll_jump() {
        let mut state = Drawn::default();
        let view = at(0., 500., 600.);
        state.plan(view, px(0.), 10_000, 8., |_| false);
        state.width = view.size.width;
        state.focused = Some(50);

        let focused = state.focused;
        // Jump far away so rows around 50 are off-screen behind spacers.
        let jumped = state.plan(view, px(-500_000.), 10_000, 8., |index| {
            retain_for_focus(focused, index)
        });
        let built = rendered(&jumped);
        for neighbor in [48, 49, 50, 51, 52] {
            assert!(
                built.contains(&neighbor),
                "expected row {neighbor} to stay built for Tab/Shift+Tab, got {built:?}"
            );
        }
        assert!(
            !built.contains(&47) && !built.contains(&53),
            "distant rows should stay culled: {built:?}"
        );
    }

    #[test]
    fn other_items_forget_the_measurements_and_the_same_keep_them() {
        let list = Cull::default();
        list.items(1);
        list.0.borrow_mut().heights = vec![px(53.); 10];
        list.items(1);
        assert_eq!(
            list.0.borrow().heights.len(),
            10,
            "the same items keep them"
        );
        list.items(2);
        assert!(
            list.0.borrow().heights.is_empty(),
            "a heading may now be where a row was"
        );
    }

    #[test]
    fn the_row_with_the_keyboards_focus_stays_built_far_from_view() {
        let list = Cull::default();
        list.0.borrow_mut().focused = Some(9_000);
        let mut built = Vec::new();
        list.column(
            &ScrollHandle::new(),
            10_000,
            8.,
            |_| false,
            |index| {
                built.push(index);
                div().h(px(96.))
            },
        );
        assert!(built.contains(&8_999), "Shift+Tab reaches the previous row");
        assert!(built.contains(&9_000), "Tab goes on from the focused row");
        assert!(built.contains(&9_001), "Tab reaches the next row");
        assert!(built.len() <= 22, "built {} rows", built.len());
    }

    #[test]
    fn measurements_include_wrapping_and_reveal_spacers_after_headers_move() {
        let mut state = Drawn::default();
        let view = at(0., 500., 600.);
        state.plan(view, px(0.), 100, 8., |_| false);
        let spans = vec![Span::Row(0), Span::Space(px(10_296.))];
        assert!(state.record(
            &spans,
            &[at(0., 240., 600.), at(240., 10_296., 600.)],
            view,
            px(0.)
        ));
        assert_eq!(state.heights[0], px(240.));
        assert!(rendered(&state.plan(view, px(0.), 100, 8., |_| false)).contains(&1));
        let list = Cull::default();
        list.0.borrow_mut().heights = vec![px(400.); 100];
        list.reset();
        assert!(list.0.borrow().heights.is_empty());
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;
    use gpui::{prelude::*, Context, Render, Window};

    /// A list and nothing else: no animation of its own asks for frames, so a frame that leaves
    /// something to correct gets the next one only if it asks.
    struct List {
        list: Cull,
        scroll: ScrollHandle,
    }

    impl Render for List {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let rows = self.list.column(
                &self.scroll,
                2_000,
                8.,
                |_| false,
                |index| {
                    div()
                        .h(px(if index % 2 == 0 { 72. } else { 180. }))
                        .child(format!("Row {index}"))
                },
            );
            div()
                .id("viewport")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .child(rows)
        }
    }

    #[test]
    fn a_jump_with_one_refresh_settles_with_no_further_input() {
        let mut app = gpui::TestAppContext::single();
        let (view, cx) = app.add_window_view(|_, _| List {
            list: Cull::default(),
            scroll: ScrollHandle::new(),
        });
        let frames = |cx: &mut gpui::VisualTestContext| {
            for _ in 0..8 {
                cx.update(|window, cx| window.simulate_next_frame(cx));
                cx.run_until_parked();
            }
        };
        frames(cx);
        let (list, scroll) = view.read_with(cx, |v, _| (v.list.clone(), v.scroll.clone()));
        assert!(list.settled(), "the first screen settles");
        // A jump to the middle, as a drag of the scrollbar makes it: one refresh, no more. The
        // rows there were never measured, so the first frame lays them out at the estimate.
        scroll.set_offset(point(px(0.), -(scroll.max_offset().y / 2.)));
        cx.update(|window, _| window.refresh());
        frames(cx);
        assert!(
            list.settled(),
            "a spacer or a mismeasured row stayed in view"
        );
    }
}

#[cfg(all(test, target_os = "windows"))]
mod desktop_tests {
    use super::*;
    use gpui::{prelude::*, AppContext, Context, Render, Window, WindowBounds, WindowOptions};
    use std::time::Duration;

    struct ListWindow {
        list: Cull,
        scroll: ScrollHandle,
        rendered: Vec<usize>,
    }

    impl Render for ListWindow {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.rendered.clear();
            let rows = self.list.column(
                &self.scroll,
                10_000,
                8.,
                |_| false,
                |index| {
                    self.rendered.push(index);
                    div()
                        .h(px(if index % 2 == 0 { 72. } else { 180. }))
                        .mb(px(8.))
                        .child(format!("Row {index}"))
                },
            );
            div()
                .id("viewport")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .child(div().pt(px(320.)).child(rows))
        }
    }

    #[test]
    #[ignore = "requires an interactive desktop and graphics adapter"]
    fn desktop_list_renders_the_viewport_after_scrollbar_jumps() {
        gpui_platform::application().run(|cx| {
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                            point(px(0.), px(0.)),
                            size(px(600.), px(500.)),
                        ))),
                        ..WindowOptions::default()
                    },
                    |_, cx| {
                        cx.new(|_| ListWindow {
                            list: Cull::default(),
                            scroll: ScrollHandle::new(),
                            rendered: Vec::new(),
                        })
                    },
                )
                .unwrap();
            cx.spawn(async move |cx| {
                for step in 0..3 {
                    cx.background_executor()
                        .timer(Duration::from_millis(300))
                        .await;
                    window
                        .update(cx, |view, _, cx| {
                            assert!(
                                view.rendered.len() < 40,
                                "rendered {} rows",
                                view.rendered.len()
                            );
                            let wanted = if step == 1 { 9_999 } else { 0 };
                            assert!(
                                view.rendered.contains(&wanted),
                                "row {wanted} missing: {:?}",
                                view.rendered
                            );
                            let measured = view.list.0.borrow();
                            let mut top = measured.top;
                            for (index, height) in measured.heights.iter().enumerate() {
                                if top < view.scroll.bounds().bottom()
                                    && top + *height > view.scroll.bounds().top()
                                {
                                    assert!(
                                        view.rendered.contains(&index),
                                        "visible row {index} was a spacer"
                                    );
                                }
                                top += *height;
                            }
                            if step == 1 {
                                assert!(
                                    top <= view.scroll.bounds().bottom() + px(1.),
                                    "the last row stayed below the viewport by {:?}",
                                    top - view.scroll.bounds().bottom()
                                );
                            }
                            drop(measured);
                            let offset = if step == 0 {
                                -view.scroll.max_offset().y
                            } else {
                                px(0.)
                            };
                            view.scroll.set_offset(point(px(0.), offset));
                            cx.notify();
                        })
                        .unwrap();
                }
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    }
}
