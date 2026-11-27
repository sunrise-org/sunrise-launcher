//! Text the mouse can select and Ctrl/Cmd-C copies, as on a web page: drag to select, across
//! paragraphs too, double-click for a word, triple-click for a whole block, Ctrl/Cmd-A for the
//! whole block clicked. GPUI's own text can't be selected.
//!
//! There is one selection in the window, from an anchor (a block and a place in it) to a head.
//! The blocks name themselves in reading order as they paint, so a selection knows which blocks
//! lie between its ends; a drag follows the column it started in.

pub(crate) mod clipboard;

use std::cell::RefCell;
use std::ops::Range;

use gpui::{
    fill, rgb, App, Bounds, CursorStyle, DispatchPhase, Element, ElementId, EntityId, FocusHandle,
    GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, KeyDownEvent,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, PlatformInput,
    Point, ScrollDelta, ScrollEvent, SharedString, StyledText, TextLayout, Window,
};

use crate::paint::palette;

#[derive(Clone, PartialEq)]
struct Place {
    block: GlobalElementId,
    ix: usize,
}

struct Active {
    anchor: Place,
    head: Place,
    dragging: bool,
    view: EntityId,
    /// The left and right edges of the block the drag started in: other blocks join only
    /// when they share some of that width, so a drag down a page stays out of a side panel.
    column: (Pixels, Pixels),
    /// For the move being handled: where it is and the top of the lowest block found above it.
    probe: Option<(Point<Pixels>, Pixels)>,
    /// The part of the page the text shows in, and the pointer: a drag past its top or bottom
    /// scrolls the page while `scrolling`.
    viewport: Bounds<Pixels>,
    pointer: Point<Pixels>,
    scrolling: bool,
}

struct Painted {
    block: GlobalElementId,
    text: SharedString,
}

thread_local! {
    static ACTIVE: RefCell<Option<Active>> = const { RefCell::new(None) };
    static FOCUS: RefCell<Option<FocusHandle>> = const { RefCell::new(None) };
    /// The blocks painted in this frame, then in the last whole one, in reading order.
    static ORDER: RefCell<(Vec<Painted>, Vec<Painted>)> = const { RefCell::new((Vec::new(), Vec::new())) };
}

/// Starts a frame: the blocks name themselves again as they paint.
pub(crate) fn begin() {
    ORDER.with_borrow_mut(|(now, last)| {
        if !now.is_empty() {
            *last = std::mem::take(now);
        }
    });
}

fn position(block: &GlobalElementId) -> Option<usize> {
    ORDER.with_borrow(|(now, last)| {
        let found = |list: &Vec<Painted>| list.iter().position(|p| p.block == *block);
        found(last).or_else(|| found(now))
    })
}

/// The selection's ends, first in reading order first.
fn ends(a: &Active) -> (Place, Place) {
    let key = |p: &Place| (position(&p.block), p.ix);
    if key(&a.head) < key(&a.anchor) && position(&a.head.block).is_some() {
        (a.head.clone(), a.anchor.clone())
    } else {
        (a.anchor.clone(), a.head.clone())
    }
}

/// The part of `block` (of `len` bytes) the selection covers.
fn covered(a: &Active, block: &GlobalElementId, len: usize) -> Option<Range<usize>> {
    let (first, last) = ends(a);
    let start = match first.block == *block {
        true => first.ix,
        false => {
            let (me, from) = (position(block)?, position(&first.block)?);
            (me > from).then_some(0)?
        }
    };
    let end = match last.block == *block {
        true => last.ix,
        false => {
            let (me, to) = (position(block)?, position(&last.block)?);
            (me < to).then_some(len)?
        }
    };
    let range = start.min(len)..end.min(len);
    (!range.is_empty()).then_some(range)
}

/// The selected text, one line per block.
fn selected_text() -> Option<String> {
    ACTIVE.with_borrow(|a| {
        let a = a.as_ref()?;
        let parts: Vec<String> = ORDER.with_borrow(|(now, last)| {
            let list = if last.is_empty() { now } else { last };
            list.iter()
                .filter_map(|p| Some(p.text[covered(a, &p.block, p.text.len())?].to_string()))
                .collect()
        });
        (!parts.is_empty()).then(|| parts.join("\n"))
    })
}

/// Text that can be selected. Its id only has to be unique among its siblings with the same text.
pub struct Selectable {
    id: ElementId,
    text: StyledText,
    copy_preview: bool,
}

/// Plain selectable text.
pub fn selectable(text: impl Into<SharedString>) -> Selectable {
    let text = text.into();
    Selectable::styled(ElementId::Name(text.clone()), StyledText::new(text))
}

impl Selectable {
    /// Selectable text with its own runs or highlights; `id` tells it apart from its siblings.
    pub fn styled(id: impl Into<ElementId>, text: StyledText) -> Self {
        Self {
            id: id.into(),
            text,
            copy_preview: true,
        }
    }

    /// Keep selected diagnostics out of the copy toast; they belong inside their pane.
    pub(crate) fn without_copy_preview(mut self) -> Self {
        self.copy_preview = false;
        self
    }
}

fn focus(cx: &mut App) -> FocusHandle {
    FOCUS.with_borrow_mut(|f| f.get_or_insert_with(|| cx.focus_handle()).clone())
}

fn index(layout: &TextLayout, position: Point<Pixels>) -> usize {
    layout
        .index_for_position(position)
        .unwrap_or_else(|nearest| nearest)
}

/// The word around `ix`: a run of letters and digits, or the single other character there.
fn word(text: &str, ix: usize) -> Range<usize> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let Some(at) = text[ix..]
        .chars()
        .next()
        .or_else(|| text[..ix].chars().next_back())
    else {
        return ix..ix;
    };
    if !is_word(at) {
        let start = if text[ix..].starts_with(at) {
            ix
        } else {
            ix - at.len_utf8()
        };
        return start..start + at.len_utf8();
    }
    let start = text[..ix]
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_word(c))
        .last()
        .map_or(ix, |(i, _)| i);
    let end = text[ix..]
        .char_indices()
        .find(|&(_, c)| !is_word(c))
        .map_or(text.len(), |(i, _)| ix + i);
    start..end
}

/// How far to scroll the page per frame while a drag holds the pointer `past` pixels beyond
/// its edge: faster the further out, as in a browser.
fn scroll_step(past: f32) -> f32 {
    (past / 4.).clamp(2., 40.)
}

/// Scrolls the page every frame while a drag holds the pointer above or below it, then replays
/// the pointer so the selection follows into the text that came into view.
fn auto_scroll(window: &Window, cx: &mut App) {
    window
        .spawn(cx, async move |cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(16))
                .await;
            let step = ACTIVE.with_borrow_mut(|a| {
                let a = a.as_mut()?;
                let (vp, at) = (a.viewport, a.pointer);
                let dy = match () {
                    _ if !a.dragging => None,
                    _ if at.y < vp.top() => Some(scroll_step(f32::from(vp.top() - at.y))),
                    _ if at.y > vp.bottom() => Some(-scroll_step(f32::from(at.y - vp.bottom()))),
                    _ => None,
                };
                if dy.is_none() {
                    a.scrolling = false;
                }
                let (left, right) = a.column;
                let inside = gpui::point((left + right) / 2., vp.center().y);
                Some((dy?, inside, at))
            });
            let Some((dy, inside, at)) = step else {
                break;
            };
            let sent = cx.update(|window, cx| {
                window.dispatch_event(
                    PlatformInput::Scroll(ScrollEvent {
                        position: inside,
                        delta: ScrollDelta::Pixels(gpui::point(Pixels::ZERO, gpui::px(dy))),
                        ..Default::default()
                    }),
                    cx,
                );
                window.dispatch_event(
                    PlatformInput::MouseMove(MouseMoveEvent {
                        position: at,
                        pressed_button: Some(MouseButton::Left),
                        ..Default::default()
                    }),
                    cx,
                );
            });
            if sent.is_err() {
                break;
            }
        })
        .detach();
}

/// The rectangles a selection covers, one per visual row, each from the first selected
/// character on that row to the end of the last, as a browser draws it.
fn rows(layout: &TextLayout, text: &str, range: &Range<usize>) -> Vec<Bounds<Pixels>> {
    let edges = text[range.clone()]
        .char_indices()
        .map(|(i, c)| (range.start + i, Some(c)))
        .chain([(range.end, None)])
        .filter_map(|(ix, c)| Some((layout.position_for_index(ix)?, c)));
    row_bounds(edges, layout.bounds().left(), layout.line_height())
}

/// [`rows`] from each selected character's place (and the end's, with no character), the
/// text's left edge and its line height. A row's indentation is left out: it starts at its first
/// character that isn't a space, and a row of only spaces has none. Copy still takes the spaces.
fn row_bounds(
    edges: impl IntoIterator<Item = (Point<Pixels>, Option<char>)>,
    edge: Pixels,
    line: Pixels,
) -> Vec<Bounds<Pixels>> {
    // The row being built: its top, where its highlight starts once known, and how far it goes.
    let mut row: Option<(Pixels, Option<Pixels>, Pixels)> = None;
    let mut out = Vec::new();
    let mut close = |row: Option<(Pixels, Option<Pixels>, Pixels)>| {
        if let Some((top, Some(left), right)) = row {
            out.push(Bounds::new(
                gpui::point(left, top),
                gpui::size((right - left).max(Pixels::ZERO), line),
            ));
        }
    };
    for (at, c) in edges {
        match &mut row {
            Some((top, _, right)) if *top == at.y => *right = (*right).max(at.x),
            _ => {
                // A row that a long line wrapped onto is already under way where its first
                // place shows (the index at a wrap reports the end of the row above): it starts
                // at the text's edge. A new line starts where its first character is.
                let wrapped = row.is_some() && at.x > edge;
                close(row.take());
                row = Some((at.y, wrapped.then_some(edge), at.x));
            }
        }
        if let (Some((_, left @ None, _)), Some(c)) = (&mut row, c) {
            if !c.is_whitespace() {
                *left = Some(at.x);
            }
        }
    }
    close(row);
    out
}

/// Ends the selection (a click elsewhere) and redraws the text it was in. The keyboard goes back
/// to the page itself (the launcher takes it when nothing has it), not to the control that had
/// it before: landing on a control scrolls the page to it.
fn clear(window: &mut Window, cx: &mut App) {
    if let Some(old) = ACTIVE.with_borrow_mut(Option::take) {
        if focus(cx).is_focused(window) {
            window.blur(cx);
        }
        cx.notify(old.view);
    }
}

impl Element for Selectable {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        self.text.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Hitbox {
        self.text
            .prepaint(id, inspector_id, bounds, state, window, cx);
        // The block the selection started in holds the keyboard, for Ctrl/Cmd-C and -A. GPUI
        // takes a focus handle only while prepainting.
        let anchored = id.is_some_and(|id| {
            ACTIVE.with_borrow(|a| a.as_ref().is_some_and(|a| a.anchor.block == *id))
        });
        if anchored {
            window.set_focus_handle(&focus(cx), cx);
        }
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut (),
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(id) = id.cloned() else {
            return;
        };
        let layout = self.text.layout().clone();
        let text: SharedString = layout.text().into();
        let view = window.current_view();
        window.set_cursor_style(CursorStyle::IBeam, hitbox);
        ORDER.with_borrow_mut(|(now, _)| {
            now.push(Painted {
                block: id.clone(),
                text: text.clone(),
            })
        });

        let (selected, anchor_here, dragging) = ACTIVE.with_borrow_mut(|a| match a.as_mut() {
            None => (None, false, false),
            Some(a) => {
                a.view = view;
                (
                    covered(a, &id, text.len()),
                    a.anchor.block == id,
                    a.dragging,
                )
            }
        });
        if selected.is_some() {
            let hit = hitbox.clone();
            window.on_mouse_event(move |_: &MouseDownEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture && !hit.is_hovered(window) {
                    clear(window, cx);
                }
            });
        }
        if anchor_here {
            let whole = text.len();
            let copy_preview = self.copy_preview;
            window.on_key_event(move |ev: &KeyDownEvent, phase, _, cx| {
                if phase != DispatchPhase::Bubble || !ev.keystroke.modifiers.secondary(cx) {
                    return;
                }
                match ev.keystroke.key.as_str() {
                    "c" => {
                        if let Some(text) = selected_text() {
                            let item = gpui::ClipboardItem::new_string(text);
                            if copy_preview {
                                clipboard::copy_to_clipboard(cx, item);
                            } else {
                                clipboard::copy_quietly(cx, item.clone());
                                crate::toast::copied(&item.text().unwrap_or_default());
                            }
                        }
                    }
                    "a" => {
                        ACTIVE.with_borrow_mut(|a| {
                            if let Some(a) = a {
                                a.anchor.ix = 0;
                                a.head = Place {
                                    block: a.anchor.block.clone(),
                                    ix: whole,
                                };
                            }
                        });
                        cx.notify(view);
                    }
                    _ => return,
                }
                cx.stop_propagation();
            });
        }
        if dragging {
            let drag = layout.clone();
            let mine = id.clone();
            // Only the part of the block that shows counts: text scrolled out of a box above or
            // below its edge is not where the pointer is.
            let visible = bounds.intersect(&window.content_mask().bounds);
            window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture || ev.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                let y = ev.position.y;
                let start_scrolling = ACTIVE.with_borrow_mut(|a| {
                    let a = a.as_mut().filter(|a| a.dragging)?;
                    a.pointer = ev.position;
                    let outside = y < a.viewport.top() || y > a.viewport.bottom();
                    (outside && !a.scrolling).then(|| a.scrolling = true)
                });
                if start_scrolling.is_some() {
                    auto_scroll(window, cx);
                }
                let moved = ACTIVE.with_borrow_mut(|a| {
                    let a = a.as_mut().filter(|a| a.dragging)?;
                    let (left, right) = a.column;
                    let shares = visible.left() < right && visible.right() > left;
                    let hidden = visible.size.height <= Pixels::ZERO;
                    if !shares || hidden || visible.top() > y {
                        return None;
                    }
                    // Every block hears the move: the lowest one whose shown part starts above
                    // the pointer takes the head.
                    let best = a.probe.filter(|(at, _)| *at == ev.position).map(|(_, t)| t);
                    if best.is_some_and(|top| top >= visible.top()) {
                        return None;
                    }
                    a.probe = Some((ev.position, visible.top()));
                    // Below the shown part, the head stays on its last shown line until the box
                    // scrolls more into view.
                    let at = gpui::point(ev.position.x, y.min(visible.bottom() - gpui::px(1.)));
                    let head = Place {
                        block: mine.clone(),
                        ix: index(&drag, at),
                    };
                    (head != a.head).then(|| a.head = head)
                });
                if moved.is_some() {
                    cx.notify(view);
                }
            });
            window.on_mouse_event(move |_: &MouseUpEvent, _, _, _| {
                ACTIVE.with_borrow_mut(|a| {
                    if let Some(a) = a {
                        a.dragging = false;
                    }
                });
            });
        }

        let viewport = window.content_mask().bounds;
        let press = layout.clone();
        let pressed_text = text.clone();
        let hit = hitbox.clone();
        window.on_mouse_event(move |ev: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble
                || ev.button != MouseButton::Left
                || !hit.is_hovered(window)
            {
                return;
            }
            let ix = index(&press, ev.position);
            let (start, end) = match ev.click_count {
                1 => (ix, ix),
                2 => {
                    let w = word(&pressed_text, ix);
                    (w.start, w.end)
                }
                _ => (0, pressed_text.len()),
            };
            let handle = focus(cx);
            clear(window, cx);
            let place = |ix| Place {
                block: id.clone(),
                ix,
            };
            ACTIVE.with_borrow_mut(|a| {
                *a = Some(Active {
                    anchor: place(start),
                    head: place(end),
                    dragging: ev.click_count == 1,
                    view,
                    column: (bounds.left(), bounds.right()),
                    probe: None,
                    viewport,
                    pointer: ev.position,
                    scrolling: false,
                })
            });
            window.focus(&handle, cx);
            // The page's own containers take the keyboard on a click unless told not to.
            window.prevent_default();
            cx.notify(view);
        });

        // The selection goes between the runs' own backgrounds (inline code) and the glyphs, so
        // neither hides it.
        let _ = layout.paint_background(window, cx);
        if let Some(range) = &selected {
            let tint = Hsla::from(rgb(palette().accent)).opacity(0.35);
            for row in rows(&layout, &text, range) {
                window.paint_quad(fill(row, tint));
            }
        }
        let _ = layout.paint_foreground(window, cx);
    }
}

impl IntoElement for Selectable {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{row_bounds, word};
    use gpui::{point, px};

    #[test]
    fn a_highlight_leaves_out_each_lines_indentation() {
        // "a\n  bc": a at 0, its line ends at 10; b at 20 and c at 30 on the next, after two
        // spaces at 0 and 10; the end at 40.
        let y2 = px(20.);
        let edges = [
            (point(px(0.), px(0.)), Some('a')),
            (point(px(10.), px(0.)), Some('\n')),
            (point(px(0.), y2), Some(' ')),
            (point(px(10.), y2), Some(' ')),
            (point(px(20.), y2), Some('b')),
            (point(px(30.), y2), Some('c')),
            (point(px(40.), y2), None),
        ];
        let rows = row_bounds(edges, px(0.), px(20.));
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].left(), rows[0].right()), (px(0.), px(10.)));
        assert_eq!((rows[1].left(), rows[1].right()), (px(20.), px(40.)));
    }

    #[test]
    fn a_double_click_takes_the_word_or_the_mark_it_lands_on() {
        let text = "Item list, @mod_name; né";
        assert_eq!(&text[word(text, 2)], "Item");
        assert_eq!(&text[word(text, 9)], ",");
        assert_eq!(&text[word(text, 13)], "mod_name");
        assert_eq!(&text[word(text, text.len())], "né");
    }
}
