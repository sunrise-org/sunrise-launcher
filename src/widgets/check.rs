//! On/off controls: the switch, the checkbox (with its dash for a partly ticked set) and the radio.
//! Each takes its id and state and returns a Tab stop; the caller adds the click, and `disabled`
//! (from `widgets`) applies to them like any other control.
#![allow(dead_code)] // Used by the shell and pages that the next redesign steps move over; removed in the final sweep.
use super::*;
use crate::frame::{c, focus_ring, Reach};
use crate::motion;
use crate::paint::palette;
use crate::tokens::{r, R_BAR, R_SMALL};

/// A switch track: 36×20, fully round. Its knob is 16 px, inset 2 px, and travels 16 px.
const TRACK: (f32, f32) = (36., 20.);
const KNOB: f32 = 16.;
const INSET: f32 = 2.;
const TRAVEL: f32 = TRACK.0 - KNOB - 2. * INSET;

/// A checkbox or radio: 18 px square, with a 1.5 px edge.
const BOX: f32 = 18.;
const EDGE: f32 = 1.5;

/// One of the three marks a checkbox can show: off, ticked, or a dash for part of a set ticked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Tick {
    Off,
    On,
    /// Part of a set is ticked: a dash.
    Some,
}

/// How much of a set is ticked, as a checkbox over the set draws it: none is `Off`, all is `On`,
/// anything in between is `Some`.
pub(crate) fn tick_of(ticked: usize, total: usize) -> Tick {
    match ticked {
        0 => Tick::Off,
        n if n >= total => Tick::On,
        _ => Tick::Some,
    }
}

/// An on/off switch: a knob that slides across its track when it changes, the track turning to
/// the accent as it goes.
pub(crate) fn switch(id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    let id = id.into();
    let key = id.to_string();
    let hover_id = SharedString::from(key.clone());
    let knob = div()
        .absolute()
        .top(px(INSET))
        .size(px(KNOB))
        .rounded_full()
        .bg(c(palette().text));
    let el = div()
        .id(id)
        .flex_none()
        .tab_index(0)
        .seen()
        .focus_visible(|s| s.shadow(focus_ring(palette().focus)))
        .relative()
        .w(px(TRACK.0))
        .h(px(TRACK.1))
        .rounded_full()
        .bg(c(palette().line))
        .cursor_pointer()
        .child(switch_fill(&key, on))
        .child(hover_layer(
            &hover_id,
            c(palette().text).opacity(0.1),
            TRACK.1 / 2.,
            0.,
        ))
        .child(motion::knob(&key, on, knob, INSET, TRAVEL));
    track_hover(el, hover_id)
}

/// The accent over the track while the switch is on: its opacity runs from 0 (off) to 1 (on),
/// easing over the knob's slide.
fn switch_fill(key: &str, on: bool) -> AnyElement {
    let fill = div()
        .absolute()
        .size_full()
        .rounded_full()
        .bg(c(palette().accent));
    match motion::sliding(key, on) {
        Some(slide) => {
            let toward = move |p: f32| if on { p } else { 1. - p };
            fill.with_animation(
                ElementId::named_usize(format!("{key}-fill"), slide),
                motion::slide_motion(),
                move |d, p| d.opacity(toward(p)),
            )
            .into_any_element()
        }
        None => fill.opacity(if on { 1. } else { 0. }).into_any_element(),
    }
}

/// A checkbox: 18 px, its corners `R_SMALL`. Ticked it is the accent with a white tick; a dash
/// shows part of a set ticked; off it is an edge of `line`.
pub(crate) fn checkbox(id: impl Into<ElementId>, state: Tick) -> Stateful<Div> {
    let id = id.into();
    let hover_id = SharedString::from(id.to_string());
    let on = state != Tick::Off;
    let layer = hover_layer(&hover_id, hover_tint(on), R_SMALL, 0.);
    let mark = match state {
        Tick::Off => None,
        Tick::On => Some(tick_icon()),
        Tick::Some => Some(
            div()
                .w(px(8.))
                .h(px(2.))
                .rounded(r(R_BAR))
                .bg(c(palette().accent_on))
                .into_any_element(),
        ),
    };
    let el = box_base(id, on, r(R_SMALL)).child(layer).children(mark);
    track_hover(el, hover_id)
}

/// A radio: 18 px and round. Off it is an edge of `line`; on, an edge of `accent` with an 8 px
/// accent dot in the middle.
pub(crate) fn radio(id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    let id = id.into();
    let hover_id = SharedString::from(id.to_string());
    let layer = hover_layer(&hover_id, hover_tint(on), BOX / 2., 0.);
    let dot = on.then(|| {
        div()
            .size(px(8.))
            .rounded_full()
            .bg(c(palette().accent))
            .into_any_element()
    });
    let el = box_base(id, on, px(BOX / 2.)).child(layer).children(dot);
    track_hover(el, hover_id)
}

/// The tick a checkbox shows when it is fully ticked: the `tick-02` icon, white on the accent.
fn tick_icon() -> AnyElement {
    crate::glass_ui::icon("tick-02", 14., c(palette().accent_on)).into_any_element()
}

/// The layer a checkbox or radio fades in under the pointer: the `hover` fill when off, a light
/// white over the accent when on.
fn hover_tint(on: bool) -> gpui::Hsla {
    if on {
        c(palette().accent_on).opacity(0.15)
    } else {
        c(palette().hover)
    }
}

/// The square or round frame shared by checkboxes and radios: 18 px, an edge of 1.5 px that is
/// the accent when on and `line` when off, filled with the accent when on, centring its marks.
fn box_base(id: ElementId, on: bool, radius: gpui::Pixels) -> Stateful<Div> {
    let edge = if on {
        c(palette().accent)
    } else {
        c(palette().line)
    };
    div()
        .id(id)
        .flex_none()
        .tab_index(0)
        .seen()
        .focus_visible(|s| s.shadow(focus_ring(palette().focus)))
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .size(px(BOX))
        .rounded(radius)
        .border(px(EDGE))
        .border_color(edge)
        .bg(if on {
            c(palette().accent)
        } else {
            gpui::transparent_black()
        })
        .cursor_pointer()
}

#[cfg(test)]
mod tests;
