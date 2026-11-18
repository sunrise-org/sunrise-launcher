//! Text the mouse can select and Ctrl/Cmd-C copies, as on a web page: drag to select, across
//! paragraphs too, double-click for a word, triple-click for a whole block, Ctrl/Cmd-A for the
//! whole block clicked. GPUI's own text can't be selected.
//!
//! There is one selection in the window, from an anchor (a block and a place in it) to a head.
//! The blocks name themselves in reading order as they paint, so a selection knows which blocks
//! lie between its ends; a drag follows the column it started in.

use std::cell::RefCell;
use std::ops::Range;

use gpui::{
    fill, rgb, App, Bounds, CursorStyle, DispatchPhase, Element, ElementId, EntityId, FocusHandle,
    GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, KeyDownEvent,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    SharedString, StyledText, TextLayout, Window,
};

use crate::tokens::GOLD;

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
}

/// Plain selectable text.
pub fn selectable(text: impl Into<SharedString>) -> Selectable {
    let text = text.into();
    Selectable {
        id: ElementId::Name(text.clone()),
        text: StyledText::new(text),
    }
}

impl Selectable {
    /// Selectable text with its own runs or highlights; `id` tells it apart from its siblings.
    pub fn styled(id: impl Into<ElementId>, text: StyledText) -> Self {
        Self {
            id: id.into(),
            text,
        }
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

/// The rectangles a selection covers: from its start to the end of that row, every row between,
/// and the last row up to its end.
fn rows(layout: &TextLayout, range: &Range<usize>, bounds: Bounds<Pixels>) -> Vec<Bounds<Pixels>> {
    let line = layout.line_height();
    let (Some(start), Some(end)) = (
        layout.position_for_index(range.start),
        layout.position_for_index(range.end),
    ) else {
        return Vec::new();
    };
    let span = |top: Pixels, left: Pixels, right: Pixels| {
        Bounds::from_corners(gpui::point(left, top), gpui::point(right, top + line))
    };
    if start.y == end.y {
        return vec![span(start.y, start.x, end.x)];
    }
    let mut out = vec![span(start.y, start.x, bounds.right())];
    let mut top = start.y + line;
    while top < end.y {
        out.push(span(top, bounds.left(), bounds.right()));
        top += line;
    }
    out.push(span(end.y, bounds.left(), end.x));
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
            let handle = focus(cx);
            window.set_focus_handle(&handle, cx);
            let whole = text.len();
            window.on_key_event(move |ev: &KeyDownEvent, phase, _, cx| {
                if phase != DispatchPhase::Bubble || !ev.keystroke.modifiers.secondary(cx) {
                    return;
                }
                match ev.keystroke.key.as_str() {
                    "c" => {
                        if let Some(text) = selected_text() {
                            crate::widgets::copy_to_clipboard(
                                cx,
                                gpui::ClipboardItem::new_string(text),
                            );
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
            window.on_mouse_event(move |ev: &MouseMoveEvent, phase, _, cx| {
                if phase != DispatchPhase::Bubble || ev.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                let y = ev.position.y;
                let moved = ACTIVE.with_borrow_mut(|a| {
                    let a = a.as_mut().filter(|a| a.dragging)?;
                    let (left, right) = a.column;
                    let shares = bounds.left() < right && bounds.right() > left;
                    if !shares || bounds.top() > y {
                        return None;
                    }
                    // Every block hears the move: the lowest one starting above the pointer
                    // takes the head.
                    let best = a.probe.filter(|(at, _)| *at == ev.position).map(|(_, t)| t);
                    if best.is_some_and(|top| top >= bounds.top()) {
                        return None;
                    }
                    a.probe = Some((ev.position, bounds.top()));
                    let head = Place {
                        block: mine.clone(),
                        ix: index(&drag, ev.position),
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

        let press = layout.clone();
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
                    let w = word(&text, ix);
                    (w.start, w.end)
                }
                _ => (0, text.len()),
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
            let tint = Hsla::from(rgb(GOLD)).opacity(0.35);
            for row in rows(&layout, range, bounds) {
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
    use super::word;

    #[test]
    fn a_double_click_takes_the_word_or_the_mark_it_lands_on() {
        let text = "Item list, @mod_name; né";
        assert_eq!(&text[word(text, 2)], "Item");
        assert_eq!(&text[word(text, 9)], ",");
        assert_eq!(&text[word(text, 13)], "mod_name");
        assert_eq!(&text[word(text, text.len())], "né");
    }
}
