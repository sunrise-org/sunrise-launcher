//! The one dropdown: a field that opens a menu of choices. Every dropdown in the launcher is this,
//! so their sizes cannot drift apart.
//!
//! The closed field is `FIELD` tall, drawn as a field box, with an optional icon first and a
//! chevron last. Its width does not follow the choice: it is as wide as the widest label it could
//! show (the real text laid out invisibly inside it), or the room it is given with
//! [`Dropdown::fill`]. The open menu hangs 4 px under the field, exactly as wide (or wider with
//! [`Dropdown::menu_min`]), and goes above it when there is no room below. The open state is the
//! caller's: it passes `open` and what a press, a pick and a close do.
#![allow(dead_code)] // Callers move over one page at a time.
use super::focus::{c, Reach};
use super::*;
use crate::paint::palette;
use crate::tokens;
use gpui::AnimationExt as _;
use std::rc::Rc;

/// What the dropdown's controls call: toggling it, closing it.
pub(crate) type Close = Rc<dyn Fn(&mut Window, &mut App)>;

/// A menu row's height.
const ROW_H: f32 = 32.;
/// The most a menu grows before it scrolls.
const MENU_MAX: f32 = 300.;
/// The least a menu is allowed when the window has little room.
const MENU_MIN: f32 = 96.;
/// The gap between the field and its menu.
const GAP: f32 = 4.;
/// A menu with more rows than this can be filtered by typing.
const FILTER_OVER: usize = 6;
/// The most that can be typed into the filter.
const FILTER_MAX: usize = 40;

/// Where a field was last drawn, and how tall the window was: what decides which side of the field
/// its menu opens on, and how wide it is.
#[derive(Clone, Copy)]
struct Place {
    top: f32,
    bottom: f32,
    width: f32,
    window: f32,
}

thread_local! {
    /// Each open menu's scroll handle, so its position survives a redraw. It is dropped when the
    /// menu closes (`forget`), so the next opening starts at its current row again.
    static HANDLES: std::cell::RefCell<std::collections::HashMap<SharedString, gpui::ScrollHandle>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    /// What was typed into each open menu's filter, by the field's id. Dropped when the menu
    /// closes (`forget`).
    static FILTER: std::cell::RefCell<std::collections::HashMap<SharedString, String>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    static PLACES: std::cell::RefCell<std::collections::HashMap<SharedString, Place>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Drops the scroll positions of the menus of the field `id`: call it while that menu is closed.
pub(crate) fn forget(id: &str) {
    HANDLES.with_borrow_mut(|m| m.retain(|key, _| !key.starts_with(id)));
    FILTER.with_borrow_mut(|m| m.retain(|key, _| key.as_ref() != id));
}

/// What has been typed into the menu of the field `id`.
fn typed(id: &SharedString) -> String {
    FILTER.with_borrow(|m| m.get(id).cloned().unwrap_or_default())
}

fn set_typed(id: &SharedString, text: String) {
    FILTER.with_borrow_mut(|m| {
        if text.is_empty() {
            m.remove(id);
        } else {
            m.insert(id.clone(), text);
        }
    });
}

/// The indices of the rows that match `typed` (case-insensitive, anywhere in the label or the
/// sub line), best first: a label that starts with it, then one that has it at a word, then
/// anywhere in the label, then the sub line. Ties keep their order. Nothing typed keeps all.
fn ranked(rows: &[(String, Option<String>)], typed: &str) -> Vec<usize> {
    let needle = typed.trim().to_lowercase();
    if needle.is_empty() {
        return (0..rows.len()).collect();
    }
    let mut hits: Vec<(u8, usize)> = rows
        .iter()
        .enumerate()
        .filter_map(|(n, (label, sub))| {
            let label = label.to_lowercase();
            let score = if label.starts_with(&needle) {
                0
            } else if label.contains(&format!(" {needle}")) {
                1
            } else if label.contains(&needle) {
                2
            } else if sub
                .as_ref()
                .is_some_and(|sub| sub.to_lowercase().contains(&needle))
            {
                3
            } else {
                return None;
            };
            Some((score, n))
        })
        .collect();
    hits.sort();
    hits.into_iter().map(|(_, n)| n).collect()
}

/// The line at the top of a filterable menu: a muted magnifier and what has been typed (a hint
/// before anything is).
fn search_line(text: &str) -> Div {
    let p = palette();
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(ROW_H))
        .px(px(10.))
        .mb(px(3.))
        .border_b_1()
        .border_color(c(p.divider))
        .child(crate::glass_ui::icon("search-01", 16., c(p.muted)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(tokens::BODY))
                .text_color(c(if text.is_empty() { p.muted } else { p.text }))
                .child(if text.is_empty() {
                    let hint: SharedString = tr("Type to filter").into();
                    hint
                } else {
                    SharedString::from(text.to_owned())
                }),
        )
}

/// A child, taking no room, that reads the keyboard while the menu is drawn: letters go to the
/// filter of the field `id`, Backspace edits it, Escape clears it before it closes the menu,
/// and Enter picks the first match (`first`).
fn typing(id: SharedString, first: Option<usize>, pick: Pick, close: Close) -> Div {
    let owner = Owner::default();
    div().absolute().size_0().child(owner.noting()).child(
        gpui::canvas(
            |_, _, _| {},
            move |_, _, window: &mut Window, _: &mut App| {
                window.on_key_event(move |ev: &gpui::KeyDownEvent, phase, window, cx| {
                    let k = &ev.keystroke;
                    if phase != gpui::DispatchPhase::Capture
                        || k.modifiers.control
                        || k.modifiers.platform
                        || k.modifiers.alt
                    {
                        return;
                    }
                    let mut text = typed(&id);
                    match k.key.as_str() {
                        "escape" if !text.is_empty() => text.clear(),
                        "backspace" if !text.is_empty() => {
                            text.pop();
                        }
                        "enter" if !text.is_empty() => {
                            cx.stop_propagation();
                            if let Some(n) = first {
                                pick(n, window, cx);
                                close(window, cx);
                            }
                            return;
                        }
                        // The first Space is a press on the row, as it was.
                        "space" if !text.is_empty() => text.push(' '),
                        _ => match k.key_char.as_deref() {
                            Some(ch)
                                if !ch.is_empty()
                                    && !ch.chars().any(char::is_control)
                                    && !(text.is_empty() && ch.trim().is_empty()) =>
                            {
                                if text.chars().count() < FILTER_MAX {
                                    text.push_str(ch);
                                }
                            }
                            _ => return,
                        },
                    }
                    set_typed(&id, text);
                    cx.stop_propagation();
                    owner.redraw(window, cx);
                });
            },
        )
        .absolute()
        .size_0(),
    )
}

/// A child of the field, taking no room, that notes where the field is as it is laid out.
fn marker(id: SharedString) -> impl IntoElement {
    gpui::canvas(
        move |bounds, window, _| {
            let place = Place {
                top: f32::from(bounds.top()),
                bottom: f32::from(bounds.bottom()),
                width: f32::from(bounds.size.width) + 2.,
                window: f32::from(window.viewport_size().height),
            };
            PLACES.with_borrow_mut(|m| {
                m.insert(id, place);
            });
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}

/// What the closed field shows.
pub(crate) struct Face {
    /// The choice now.
    pub(crate) text: SharedString,
    /// Every text the field can show, which sets its width.
    pub(crate) sizers: Vec<SharedString>,
    pub(crate) glyph: Option<&'static str>,
    /// A muted note before the chevron.
    pub(crate) aside: Option<SharedString>,
}

/// The closed field. The caller adds the press (`on_click`). `fill` makes it as wide as its
/// parent instead of as wide as its labels.
pub(crate) fn trigger(
    id: impl Into<SharedString>,
    open: bool,
    face: Face,
    enabled: bool,
    fill: bool,
) -> Stateful<Div> {
    let id: SharedString = id.into();
    let p = palette();
    let label = div()
        .flex_grow_1()
        .flex_shrink_1()
        .min_w_0()
        .flex()
        .flex_col()
        // Every label laid out in no height: the field is as wide as the widest.
        .child(
            div().h_0().overflow_hidden().flex().flex_col().children(
                face.sizers
                    .into_iter()
                    .map(|s| div().whitespace_nowrap().child(s)),
            ),
        )
        .child(div().truncate().child(face.text));
    let el = super::field::field_box(open, false)
        .id(id.clone())
        .relative()
        .reach_edge()
        .gap(px(8.))
        .when(fill, |d| d.w_full())
        .when(!fill, |d| d.flex_none())
        .when(enabled, |d| {
            d.cursor_pointer()
                .hover(|s| s.border_color(gpui::Hsla::from(gpui::rgb(p.text)).opacity(0.35)))
        })
        .when(!enabled, |d| d.opacity(0.6).tab_stop(false))
        .children(
            face.glyph
                .map(|g| crate::glass_ui::icon(g, 16., c(p.muted))),
        )
        .child(label)
        .children(face.aside.map(|a| {
            div()
                .flex_none()
                .text_size(px(tokens::SMALL))
                .text_color(c(p.muted))
                .child(a)
        }))
        .child(div().flex_none().child(super::motion_bits::chevron(
            format!("{id}-chevron"),
            open,
            14.,
            c(p.muted),
        )))
        .child(marker(id.clone()));
    // The field sinks a pixel while it is held, as a button does.
    if enabled {
        press(el, &id, |d| d.top(px(1.)))
    } else {
        el
    }
}

/// One row of a menu: a heading, a muted line under it when there is one, and a tick when it is
/// the current choice. The caller adds the press.
pub(crate) fn row(
    id: impl Into<ElementId>,
    head: impl Into<SharedString>,
    sub: Option<SharedString>,
    current: bool,
) -> Stateful<Div> {
    row_when(id, head, sub, current, true)
}

/// A [`row`] that is pressable only when `enabled`: a disabled one is dimmed, takes no hover and
/// is not a Tab stop. The caller adds no press to it.
pub(crate) fn row_when(
    id: impl Into<ElementId>,
    head: impl Into<SharedString>,
    sub: Option<SharedString>,
    current: bool,
    enabled: bool,
) -> Stateful<Div> {
    let p = palette();
    div()
        .id(id)
        .reach()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(10.))
        .py(px(4.))
        .min_h(px(ROW_H))
        .rounded(tokens::r(tokens::R_SMALL))
        .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(c(p.hover))))
        .when(!enabled, |d| {
            d.opacity(0.5).cursor_default().tab_stop(false)
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .truncate()
                        .text_size(px(tokens::BODY))
                        .text_color(c(if current { p.accent_ink } else { p.text }))
                        .child(head.into()),
                )
                .children(sub.map(|sub| {
                    div()
                        .truncate()
                        .text_size(px(tokens::SMALL))
                        .text_color(c(p.muted))
                        .child(sub)
                })),
        )
        .children(current.then(|| crate::glass_ui::icon("tick-02", 16., c(p.accent))))
}

/// A thin rule between groups of rows.
pub(crate) fn rule() -> AnyElement {
    div()
        .h(px(1.))
        .my(px(3.))
        .bg(c(palette().divider))
        .into_any_element()
}

/// Escape closes an open menu, and the arrows open it or move through it. Enter and Space press
/// the field as a click does.
pub(crate) fn with_keys(
    el: Stateful<Div>,
    open: bool,
    toggle: Close,
    close: Close,
) -> Stateful<Div> {
    el.on_key_down(move |ev: &gpui::KeyDownEvent, window, cx| {
        match (ev.keystroke.key.as_str(), open) {
            ("escape", true) => close(window, cx),
            ("down" | "up", false) => toggle(window, cx),
            ("down", true) => window.focus_next(cx),
            ("up", true) => window.focus_prev(cx),
            _ => return,
        }
        cx.stop_propagation();
    })
}

/// The open menu for the field `id` (which sits in a `relative` parent as tall as the field),
/// over everything after it, with a backdrop whose press calls `close`. `rows` is what to list,
/// `count` how many rows that is (to tell whether it fits below), `min_width` a floor under the
/// field's own width, `current` the index in `rows` of the row to scroll into view when the menu
/// opens. The menu is at most `MENU_MAX` tall, less where the window has less room on its side
/// of the field, and scrolls inside past that.
pub(crate) fn layer(
    id: &SharedString,
    rows: Vec<AnyElement>,
    count: usize,
    current: Option<usize>,
    min_width: Option<f32>,
    close: Close,
) -> [gpui::Deferred; 2] {
    layer_with(id, None, rows, count, current, min_width, close)
}

/// [`layer`] with a `head` above the rows that does not scroll (the filter's line).
pub(crate) fn layer_with(
    id: &SharedString,
    head: Option<Div>,
    rows: Vec<AnyElement>,
    count: usize,
    current: Option<usize>,
    min_width: Option<f32>,
    close: Close,
) -> [gpui::Deferred; 2] {
    let p = palette();
    let place = PLACES.with_borrow(|m| m.get(id).copied());
    let head_h = if head.is_some() { ROW_H + 4. } else { 0. };
    let height = (count as f32 * (ROW_H + 1.) + 8. + head_h).min(MENU_MAX);
    let room_below = place.map(|at| at.window - at.bottom - 12. - GAP);
    let room_above = place.map(|at| at.top - 12. - GAP);
    let flip = match (room_below, room_above) {
        (Some(below), Some(above)) => below < height && above > below,
        _ => false,
    };
    let room = if flip { room_above } else { room_below };
    let cap = room.map_or(MENU_MAX, |room| room.clamp(MENU_MIN, MENU_MAX));
    // A menu of other rows (the build menu's versions) starts at its top again.
    let key = SharedString::from(format!("{id}:{count}"));
    let (scroll, fresh) = HANDLES.with_borrow_mut(|m| match m.get(&key) {
        Some(handle) => (handle.clone(), false),
        None => {
            let handle = gpui::ScrollHandle::new();
            m.insert(key.clone(), handle.clone());
            (handle, true)
        }
    });
    if let Some(row) = current.filter(|_| fresh) {
        scroll.scroll_to_item(row);
    }
    let width = place
        .map(|at| at.width)
        .unwrap_or(0.)
        .max(min_width.unwrap_or(0.));
    let backdrop = {
        let close = close.clone();
        div()
            .absolute()
            .top(px(-4000.))
            .left(px(-4000.))
            .w(px(8000.))
            .h(px(8000.))
            .occlude()
            .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                close(window, cx)
            })
    };
    let menu = div()
        .id(SharedString::from(format!("{id}-menu")))
        .absolute()
        .left_0()
        .when(width > 0., |d| d.w(px(width)))
        .when(width <= 0., |d| d.w_full())
        .flex()
        .flex_col()
        .p(px(4.))
        .rounded(tokens::r(tokens::R_CONTROL))
        .border_1()
        .border_color(p.input_edge())
        .bg(c(p.raised))
        .shadow_lg()
        .max_h(px(cap))
        .occlude()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_scroll(|_: &gpui::ScrollEvent, _, cx| cx.stop_propagation())
        .children(head)
        .child(
            div()
                .id(SharedString::from(format!("{id}-menu-rows")))
                .flex()
                .flex_col()
                .gap(px(1.))
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&scroll)
                // Rows keep their height when there are more than fit: the menu scrolls.
                .children(rows.into_iter().map(|row| div().flex_none().child(row))),
        );
    let menu = with_keys(menu, true, close.clone(), close).with_animation(
        SharedString::from(format!("{id}-menu-in")),
        crate::motion::eased(sunrise_launcher_theme::QUICK),
        move |d, t| {
            // It fades in from 4 px nearer the field.
            let away = tokens::FIELD + GAP - GAP * (1. - t);
            let d = d.opacity(t);
            if flip {
                d.bottom(px(away))
            } else {
                d.top(px(away))
            }
        },
    );
    [
        gpui::deferred(backdrop).with_priority(9),
        gpui::deferred(menu).with_priority(10),
    ]
}

/// One choice in a [`Dropdown`].
pub(crate) struct Item {
    label: SharedString,
    shown: Option<SharedString>,
    sub: Option<SharedString>,
    current: bool,
    enabled: bool,
    rule_after: bool,
}

impl Item {
    pub(crate) fn new(label: impl Into<SharedString>) -> Item {
        Item {
            label: label.into(),
            shown: None,
            sub: None,
            current: false,
            enabled: true,
            rule_after: false,
        }
    }

    /// A muted second line under the label.
    pub(crate) fn sub(mut self, sub: impl Into<SharedString>) -> Item {
        self.sub = Some(sub.into());
        self
    }

    /// What the closed field says while this is the choice, when that is not its label.
    pub(crate) fn shown(mut self, shown: impl Into<SharedString>) -> Item {
        self.shown = Some(shown.into());
        self
    }

    pub(crate) fn current(mut self, current: bool) -> Item {
        self.current = current;
        self
    }

    /// Dimmed and not pressable.
    pub(crate) fn disabled(mut self) -> Item {
        self.enabled = false;
        self
    }

    /// A rule under this row.
    pub(crate) fn rule_after(mut self) -> Item {
        self.rule_after = true;
        self
    }

    fn face(&self) -> SharedString {
        self.shown.clone().unwrap_or_else(|| self.label.clone())
    }
}

/// A dropdown: `Dropdown::new(id, items, open).on_toggle(..).on_pick(..).on_close(..).build()`.
pub(crate) struct Dropdown {
    id: SharedString,
    items: Vec<Item>,
    open: bool,
    glyph: Option<&'static str>,
    placeholder: Option<SharedString>,
    fill: bool,
    max_width: Option<f32>,
    menu_min: Option<f32>,
    toggle: Close,
    close: Close,
    pick: Pick,
}

/// What a pick does: the option's index.
type Pick = Rc<dyn Fn(usize, &mut Window, &mut App)>;

impl Dropdown {
    pub(crate) fn new(id: impl Into<SharedString>, items: Vec<Item>, open: bool) -> Dropdown {
        Dropdown {
            id: id.into(),
            items,
            open,
            glyph: None,
            placeholder: None,
            fill: false,
            max_width: None,
            menu_min: None,
            toggle: Rc::new(|_: &mut Window, _: &mut App| {}),
            close: Rc::new(|_: &mut Window, _: &mut App| {}),
            pick: Rc::new(|_: usize, _: &mut Window, _: &mut App| {}),
        }
    }

    /// An icon before the text.
    pub(crate) fn icon(mut self, glyph: &'static str) -> Dropdown {
        self.glyph = Some(glyph);
        self
    }

    /// What the field says while no item is current.
    pub(crate) fn placeholder(mut self, text: impl Into<SharedString>) -> Dropdown {
        self.placeholder = Some(text.into());
        self
    }

    /// As wide as its parent, not as its labels.
    pub(crate) fn fill(mut self) -> Dropdown {
        self.fill = true;
        self
    }

    /// The field is never wider than this, however long a label.
    pub(crate) fn max_width(mut self, width: f32) -> Dropdown {
        self.max_width = Some(width);
        self
    }

    /// The menu is at least this wide (it is the field's width otherwise).
    pub(crate) fn menu_min(mut self, width: f32) -> Dropdown {
        self.menu_min = Some(width);
        self
    }

    /// What a press on the field does: flip `open`.
    pub(crate) fn on_toggle(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Dropdown {
        self.toggle = Rc::new(f);
        self
    }

    /// What an outside press, Escape or a pick does: set `open` false.
    pub(crate) fn on_close(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Dropdown {
        self.close = Rc::new(f);
        self
    }

    /// What picking row `n` does. The menu is closed after it.
    pub(crate) fn on_pick(
        mut self,
        f: impl Fn(usize, &mut Window, &mut App) + 'static,
    ) -> Dropdown {
        self.pick = Rc::new(f);
        self
    }

    /// The field and, while open, its menu, in a parent that sizes to the field.
    pub(crate) fn build(self) -> Div {
        let Dropdown {
            id,
            items,
            open,
            glyph,
            placeholder,
            fill,
            max_width,
            menu_min,
            toggle,
            close,
            pick,
        } = self;
        let text = items
            .iter()
            .find(|i| i.current)
            .map(Item::face)
            .or_else(|| placeholder.clone())
            .unwrap_or_else(|| SharedString::from(""));
        let sizers: Vec<SharedString> = items.iter().map(Item::face).chain(placeholder).collect();
        let face = Face {
            text,
            sizers,
            glyph,
            aside: None,
        };
        let press = toggle.clone();
        let field = trigger(id.clone(), open, face, true, fill)
            .when_some(max_width, |d, w| d.max_w(px(w)))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| press(window, cx));
        let field = with_keys(field, open, toggle, close.clone());
        let wrap = div()
            .relative()
            .flex()
            .flex_col()
            .when(fill, |d| d.w_full())
            .when(!fill, |d| d.flex_none().self_start())
            .child(field);
        if !open {
            forget(&id);
            return wrap;
        }
        let key = SharedString::from(format!("{id}-row"));
        let filterable = items.len() > FILTER_OVER;
        let text = if filterable {
            typed(&id)
        } else {
            String::new()
        };
        let order = if filterable {
            let names: Vec<(String, Option<String>)> = items
                .iter()
                .map(|i| (i.label.to_string(), i.sub.as_ref().map(|s| s.to_string())))
                .collect();
            ranked(&names, &text)
        } else {
            (0..items.len()).collect()
        };
        let first = order.iter().copied().find(|&n| items[n].enabled);
        let filtering = !text.trim().is_empty();
        let mut at: Option<usize> = None;
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut items: Vec<Option<Item>> = items.into_iter().map(Some).collect();
        for n in order {
            let item = items[n].take().expect("each index is listed once");
            if item.current && at.is_none() {
                at = Some(rows.len());
            }
            let line = row_when(
                ElementId::from((key.clone(), n)),
                item.label,
                item.sub,
                item.current,
                item.enabled,
            );
            let line = if item.enabled {
                let (pick, close) = (pick.clone(), close.clone());
                line.on_click(move |_, window, cx| {
                    pick(n, window, cx);
                    close(window, cx);
                })
            } else {
                line
            };
            rows.push(line.into_any_element());
            if item.rule_after && !filtering {
                rows.push(rule());
            }
        }
        if rows.is_empty() {
            rows.push(
                div()
                    .px(px(10.))
                    .min_h(px(ROW_H))
                    .flex()
                    .items_center()
                    .text_size(px(tokens::BODY))
                    .text_color(c(palette().muted))
                    .child(tr("No matches"))
                    .into_any_element(),
            );
        }
        let count = rows.len();
        let head = filterable
            .then(|| search_line(&text).child(typing(id.clone(), first, pick, close.clone())));
        wrap.children(layer_with(&id, head, rows, count, at, menu_min, close))
    }
}
