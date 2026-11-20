//! Small pieces the three Mods screens share. Colours are the launcher's (`crate::frame`).
use super::*;
use crate::paint::palette;
use gpui::{rgb, Hsla};
use std::path::PathBuf;
use std::rc::Rc;

pub(super) fn c(x: u32) -> Hsla {
    rgb(x).into()
}

pub(super) fn tag(label: &'static str) -> Div {
    div()
        .flex_none()
        .px(px(6.))
        .py(px(1.))
        .rounded(crate::tokens::r(crate::tokens::R_SMALL))
        .border_1()
        .border_color(c(palette().line))
        .text_size(px(tokens::CAPTION))
        .text_color(c(palette().muted))
        .child(label)
}

/// What kind of thing a card or page is, where a plain tag would not stand out: "Collection",
/// "Library".
pub(super) fn badge(label: &'static str) -> Div {
    div()
        .flex_none()
        .px(px(6.))
        .py(px(1.))
        .rounded(crate::tokens::r(crate::tokens::R_SMALL))
        .border_1()
        .border_color(c(palette().accent_ink))
        .text_size(px(tokens::CAPTION))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(c(palette().accent_ink))
        .child(label)
}

pub(super) fn muted(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(tokens::SMALL))
        .text_color(c(palette().muted))
        .child(text.into())
}

/// `muted` text a reader may want to copy: an error, a reason, a note.
pub(super) fn note(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(tokens::SMALL))
        .text_color(c(palette().muted))
        .child(crate::selectable::selectable(text))
}

/// The main action on a screen, in cream: gold is Play's alone.
pub(super) use crate::frame::primary;

/// A control that can't be used now: drawn dim, with no handler.
pub(super) fn dim(b: Stateful<Div>) -> Stateful<Div> {
    b.opacity(0.45).cursor_default().tab_stop(false)
}

pub(crate) fn chip(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    on: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .px(px(14.))
        .h(px(tokens::CONTROL))
        .flex()
        .items_center()
        .justify_center()
        // A button's corners: a choice is a row of buttons, not of pills.
        .rounded(tokens::r(tokens::R_CONTROL))
        .border_1()
        .border_color(if on {
            c(palette().text)
        } else {
            c(palette().line)
        })
        .bg(if on {
            c(palette().text)
        } else {
            c(palette().field)
        })
        .text_color(if on {
            c(palette().background)
        } else {
            c(palette().text)
        })
        .text_size(px(tokens::LABEL))
        .cursor_pointer()
        .focus_visible(|s| s.border_color(c(palette().focus)))
        .child(label.into())
}

/// What choosing one of [`choices`] does.
pub(crate) type Choose = Rc<dyn Fn(&mut Launcher, &mut Context<Launcher>)>;

/// One choice among chips (`label`, chosen, what choosing it does): tabs, or a setting with a
/// few values. The group is one Tab stop with the ring around it, and the arrow keys choose the
/// chip before or after, as a click on it would.
pub(crate) fn choices(
    id: &'static str,
    chips: Vec<(SharedString, bool, Choose)>,
    cx: &mut Context<Launcher>,
) -> Stateful<Div> {
    let at = chips.iter().position(|(_, on, _)| *on).unwrap_or(0);
    let picks: Vec<Choose> = chips.iter().map(|(_, _, pick)| pick.clone()).collect();
    let mut group = div()
        .id(id)
        .tab_index(0)
        .seen()
        .flex()
        .flex_wrap()
        .items_center()
        // Room for the ring, taken back so the chips stay where they were.
        .p(px(3.))
        .m(px(-4.))
        .rounded(tokens::r(tokens::R_CONTROL) + px(4.))
        .border_1()
        .border_color(gpui::transparent_black())
        .focus_visible(|s| s.border_color(c(palette().focus)));
    for (n, (label, on, pick)) in chips.into_iter().enumerate() {
        // A few options side by side: short words don't make one smaller than the others.
        group = group.child(
            chip((id, n), label, on)
                .min_w(px(96.))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    pick(this, cx);
                    cx.notify();
                })),
        );
    }
    group.on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
        let step = match ev.keystroke.key.as_str() {
            "left" | "up" => picks.len() - 1,
            "right" | "down" => 1,
            _ => return,
        };
        picks[(at + step) % picks.len()](this, cx);
        cx.stop_propagation();
        cx.notify();
    }))
}

/// Two views of one page side by side, the chosen one filled (`id`, `label`, chosen, its page):
/// Mods' Discover | Installed and Servers' Join | Host. The pair is one Tab stop, and the arrow
/// keys go to the other view, as a click on it would; `go` opens a view.
pub(crate) fn segmented(
    id: &'static str,
    views: [(&'static str, SharedString, bool, Page); 2],
    go: fn(&mut Launcher, Page),
    cx: &mut Context<Launcher>,
) -> Stateful<Div> {
    let other = views
        .iter()
        .find(|(_, _, on, _)| !*on)
        .map(|(_, _, _, page)| *page);
    let mut pair = div()
        .id(id)
        .tab_index(0)
        .seen()
        .focus_visible(|s| s.border_color(c(palette().focus)))
        .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
            if !matches!(ev.keystroke.key.as_str(), "left" | "right" | "up" | "down") {
                return;
            }
            if let Some(page) = other {
                go(this, page);
            }
            cx.stop_propagation();
            cx.notify();
        }))
        .flex()
        .flex_none()
        .p(px(3.))
        .rounded(tokens::r(tokens::R_CARD))
        .border_1()
        .border_color(c(palette().line))
        .bg(palette().card());
    for (view, label, on, page) in views {
        pair = pair.child(
            div()
                .id(view)
                .px(px(18.))
                .h(px(tokens::FIELD))
                .flex()
                .items_center()
                .rounded(tokens::r(tokens::R_CONTROL))
                .text_size(px(tokens::BODY))
                .cursor_pointer()
                .when(on, |d| {
                    d.bg(c(palette().line)).font_weight(FontWeight::SEMIBOLD)
                })
                .child(label)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if !on {
                        go(this, page);
                    }
                    cx.notify();
                })),
        );
    }
    pair
}

/// An on/off switch: a knob that slides across its track when it changes, the track and the
/// knob turning to their new colours as it goes.
pub(crate) fn switch(id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    let id = id.into();
    let key = id.to_string();
    // `v` runs from 0 (off) to 1 (on).
    let fill = move |d: Div, v: f32| d.opacity(v);
    let knob = move |d: Div, v: f32| {
        d.left(px(INSET + TRAVEL * v))
            .bg(mix(c(palette().muted), c(palette().background), v))
    };
    let (fill_el, knob_el): (AnyElement, AnyElement) = {
        let fill_div = div()
            .absolute()
            .size_full()
            .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
            .bg(c(palette().text));
        let knob_div = div()
            .absolute()
            .top(px(INSET))
            .size(px(KNOB))
            .rounded((crate::tokens::r(crate::tokens::R_CONTROL) - px(INSET)).max(px(0.)));
        match crate::motion::sliding(&key, on) {
            Some(slide) => {
                let toward = move |p: f32| if on { p } else { 1. - p };
                (
                    fill_div
                        .with_animation(
                            ElementId::named_usize(format!("{key}-fill"), slide),
                            crate::motion::slide_motion(),
                            move |d, p| fill(d, toward(p)),
                        )
                        .into_any_element(),
                    knob_div
                        .with_animation(
                            ElementId::named_usize(format!("{key}-knob"), slide),
                            crate::motion::slide_motion(),
                            move |d, p| knob(d, toward(p)),
                        )
                        .into_any_element(),
                )
            }
            None => {
                let v = if on { 1. } else { 0. };
                (
                    fill(fill_div, v).into_any_element(),
                    knob(knob_div, v).into_any_element(),
                )
            }
        }
    };
    div()
        .id(id)
        .flex_none()
        .tab_index(0)
        .seen()
        .focus_visible(|s| s.shadow(crate::frame::focus_ring(palette().focus)))
        .relative()
        .w(px(40.))
        .h(px(22.))
        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
        .bg(c(palette().line))
        .cursor_pointer()
        .child(fill_el)
        .child(knob_el)
}

/// A switch's knob, the room around it, and how far it slides.
const KNOB: f32 = 16.;
const INSET: f32 = 3.;
const TRAVEL: f32 = 40. - KNOB - 2. * INSET;

/// `a` turning into `b`, `v` of the way.
fn mix(a: Hsla, b: Hsla, v: f32) -> Hsla {
    let (a, b) = (gpui::Rgba::from(a), gpui::Rgba::from(b));
    let m = |x: f32, y: f32| x + (y - x) * v;
    Hsla::from(gpui::Rgba {
        r: m(a.r, b.r),
        g: m(a.g, b.g),
        b: m(a.b, b.b),
        a: m(a.a, b.a),
    })
}

pub(crate) fn checkbox(id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    checkbox_of(id, on, false)
}

/// A box over a set: ticked when all of it is, a dash when only some is, empty when none is.
pub(super) fn checkbox_of_set(
    id: impl Into<ElementId>,
    ticks: model::collection::Ticks,
) -> Stateful<Div> {
    use model::collection::Ticks;
    checkbox_of(id, ticks != Ticks::None, ticks == Ticks::Some)
}

fn checkbox_of(id: impl Into<ElementId>, on: bool, dash: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .tab_index(0)
        .seen()
        .size(px(18.))
        .rounded(crate::tokens::r(crate::tokens::R_SMALL))
        .border_1()
        .border_color(if on {
            c(palette().text)
        } else {
            c(palette().muted)
        })
        .focus_visible(|s| s.border_color(c(palette().focus)))
        .bg(if on {
            c(palette().text)
        } else {
            c(palette().field)
        })
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .children((on && !dash).then(|| blocks::icon("check", 12., c(palette().background))))
        .children(dash.then(|| {
            div()
                .w(px(8.))
                .h(px(2.))
                .rounded(crate::tokens::r(1.))
                .bg(c(palette().background))
        }))
}

pub(super) use crate::mod_icons::tile;

/// How many cards fit side by side in a Discover list `width` wide: three on a roomy window,
/// fewer as it narrows, so a card never gets narrower than its own text needs.
pub(super) fn columns(width: f32) -> u16 {
    (((width + 12.) / 332.).floor() as u16).clamp(1, 3)
}

/// One thing to add in a Discover list, for mods and packs alike: picture, name and author, one
/// line of what it is, then its tags and numbers with the control beside them, and under them
/// a sentence when there is something to say (why it can't be added, how well it runs). Every
/// text shortens or wraps inside the card, whatever its width.
#[allow(clippy::too_many_arguments)]
pub(super) fn browse_card(
    id: impl Into<ElementId>,
    picture: Option<PathBuf>,
    project: &str,
    name: &str,
    author: &str,
    summary: &str,
    facts: Vec<Div>,
    control: Option<AnyElement>,
    wide_control: bool,
    note: Option<String>,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .gap(px(12.))
        .p(px(12.))
        .min_w_0()
        .overflow_hidden()
        .rounded(crate::tokens::r(crate::tokens::R_CARD))
        .border_1()
        .border_color(c(palette().line))
        .bg(palette().card())
        .cursor_pointer()
        .hover(|s| s.border_color(c(palette().muted)))
        .tab_index(0)
        .seen()
        .focus_visible(|s| s.border_color(c(palette().focus)))
        .child(tile(picture, project, name, 48.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(8.))
                        .min_w_0()
                        .child(
                            div()
                                .min_w_0()
                                .font_weight(FontWeight::SEMIBOLD)
                                .truncate()
                                .child(name.to_owned()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(tokens::SMALL))
                                .text_color(c(palette().muted))
                                .truncate()
                                .child(author.to_owned()),
                        ),
                )
                .child(
                    div()
                        .text_size(px(tokens::LABEL))
                        .text_color(c(palette().muted))
                        .line_clamp(2)
                        .text_ellipsis()
                        .child(summary.to_owned()),
                )
                // Cards in a row are as tall as the tallest: the last line sits at the bottom.
                .child(div().flex_1())
                .child({
                    // The facts and the control never share space: the control sits beside
                    // them while both fit, and drops to a row of its own under them when the
                    // facts are too full (or the control too long) for that.
                    let facts = div()
                        .flex_auto()
                        .min_w_0()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_x(px(8.))
                        .gap_y(px(4.))
                        .children(facts);
                    div()
                        .flex()
                        .flex_wrap()
                        .items_end()
                        .min_w_0()
                        .gap_x(px(8.))
                        .gap_y(px(8.))
                        .child(facts)
                        .children(control.map(|c| {
                            let held = div().child(c);
                            if wide_control {
                                held.w_full()
                            } else {
                                held.flex_none().ml_auto()
                            }
                        }))
                })
                // Two lines at most: the whole sentence is the tooltip.
                .children(note.map(|note| {
                    muted(note.clone())
                        .id("note")
                        .line_clamp(2)
                        .text_ellipsis()
                        .tooltip(crate::glass_ui::tip(note))
                })),
        )
}

/// "Installed", where a card's control would be.
pub(super) fn added_mark() -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(5.))
        .text_size(px(tokens::LABEL))
        .text_color(c(palette().success))
        .child(blocks::icon("check", 12., c(palette().success)))
        .child(tr("Installed"))
        .into_any_element()
}

/// A row's quiet status: a small glyph in `color` and a few words, never a sentence.
pub(super) fn status(icon: &'static str, label: impl Into<SharedString>, color: u32) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(tokens::LABEL))
        .text_color(c(palette().muted))
        .child(crate::glass_ui::icon(icon, 14., c(color)))
        .child(label.into())
}

pub(super) fn card() -> Div {
    div()
        .rounded(crate::tokens::r(crate::tokens::R_CARD))
        .border_1()
        .border_color(c(palette().divider))
        .bg(palette().card())
}

/// The theme the shared text field draws with, in this look's colours.
pub(crate) fn field_theme(mut t: crate::theme::Theme) -> crate::theme::Theme {
    t.ink = c(palette().text);
    t.muted = c(palette().muted);
    t.glass = false;
    t
}

/// For the row or control a page was opened at: once, the page `scroll` moves so it shows, and
/// the keyboard is put on `focus` when it has one. Laid over the row, it takes no room.
pub(crate) fn reveal(
    scroll: gpui::ScrollHandle,
    done: Rc<std::cell::Cell<bool>>,
    focus: Option<FocusHandle>,
) -> impl IntoElement {
    const ROOM: f32 = 24.;
    gpui::canvas(
        move |row: Bounds<gpui::Pixels>, window, cx| {
            if done.get() {
                return;
            }
            // A frame being drawn can't ask for the next one itself: that is asked after it.
            let again = |window: &mut Window, cx: &mut App| {
                let window = window.window_handle();
                cx.defer(move |cx| {
                    window.update(cx, |_, window, _| window.refresh()).ok();
                });
            };
            let view = scroll.bounds();
            // The page is not laid out before its first frame: wait for the next.
            if view.size.height <= px(0.) {
                return again(window, cx);
            }
            done.set(true);
            if let Some(focus) = &focus {
                focus.focus(window, cx);
            }
            let lift = if row.top() < view.top() {
                row.top() - view.top() - px(ROOM)
            } else if row.bottom() > view.bottom() {
                row.bottom() - view.bottom() + px(ROOM)
            } else {
                return;
            };
            let at = scroll.offset();
            scroll.set_offset(gpui::point(at.x, at.y - lift));
            again(window, cx);
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}
