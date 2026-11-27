//! A keyboard- and pointer-operated slider, carried from timboer-gif's #47.
//!
//! The track is found by a scroll handle that scrolls nothing: GPUI Fast moves a scrolled
//! page's layer without drawing it again, and only a handle's bounds follow that move.
use crate::{paint::palette, Launcher};
use gpui::{
    canvas, div, px, relative, rgb, App, Context, DispatchPhase, Div, InteractiveElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels,
    Point, ScrollHandle, Stateful, StatefulInteractiveElement, Styled, Window,
};
use std::cell::Cell;
use std::rc::Rc;

const THUMB: f32 = 16.;

/// The track and current pointer value survive redraws while the slider is being dragged.
#[derive(Clone, Default)]
pub(crate) struct Slider {
    track: ScrollHandle,
    held: Rc<Cell<Option<f32>>>,
}

impl Slider {
    pub(crate) fn preview(&self) -> Option<f32> {
        self.held.get()
    }

    pub(crate) fn cancel(&self) {
        self.held.set(None);
    }

    /// The value under the pointer, from its x against the track's bounds alone (so a pointer
    /// that wanders above or below the track still counts), clamped and snapped to `step`.
    fn along(&self, at: Point<Pixels>, step: f32) -> f32 {
        let bounds = self.track.bounds();
        let travel = f32::from(bounds.size.width) - THUMB;
        if travel <= 0. {
            return 0.;
        }
        let raw = ((f32::from(at.x - bounds.left()) - THUMB / 2.) / travel).clamp(0., 1.);
        if step > 0. {
            ((raw / step).round() * step).clamp(0., 1.)
        } else {
            raw
        }
    }

    /// A slider at `value` (0 to 1), stepping by `step` from the keyboard. The callback runs
    /// on release or a keyboard step, never during a drag; the knob itself follows `preview()`.
    pub(crate) fn render(
        &self,
        id: &'static str,
        value: f32,
        step: f32,
        commit: impl Fn(&mut Launcher, f32, &mut Context<Launcher>) + 'static,
        cx: &mut Context<Launcher>,
    ) -> Stateful<Div> {
        let me = cx.entity().downgrade();
        let commit = Rc::new(commit);
        let released = commit.clone();
        let pointer = self.clone();
        let follow = canvas(
            |_, _, _| {},
            move |_, _, window: &mut Window, _: &mut App| {
                let (me_move, moving) = (me.clone(), pointer.clone());
                let (step_move, step_up) = (step, step);
                window.on_mouse_event(move |ev: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture && moving.preview().is_some() {
                        // If the window never received the release, discard the preview.
                        moving.held.set(
                            (ev.pressed_button == Some(MouseButton::Left))
                                .then(|| moving.along(ev.position, step_move)),
                        );
                        let _ = me_move.update(cx, |_, cx| cx.notify());
                    }
                });
                let (me_up, released, lifting) = (me.clone(), released.clone(), pointer.clone());
                window.on_mouse_event(move |ev: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture
                        && ev.button == MouseButton::Left
                        && lifting.held.take().is_some()
                    {
                        let v = lifting.along(ev.position, step_up);
                        let _ = me_up.update(cx, |this, cx| released(this, v, cx));
                    }
                });
            },
        )
        .absolute()
        .size_full();
        let pointer = self.clone();
        let keyboard = self.clone();
        let p = palette();
        let dragging = self.preview().is_some();
        // The knob and fill follow the pointer while it is held; the callers pass the saved
        // value, which only changes on release.
        let value = self.preview().unwrap_or(value);
        div()
            .id(id)
            .tab_index(0)
            .relative()
            .flex_none()
            .w(px(184.))
            .max_w_full()
            .h(px(28.))
            .rounded_full()
            .border_1()
            .border_color(gpui::transparent_black())
            .cursor_pointer()
            .track_scroll(&self.track)
            .focus_visible(move |s| s.border_color(rgb(p.focus)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, ev: &MouseDownEvent, _, cx| {
                    pointer.held.set(Some(pointer.along(ev.position, step)));
                    cx.notify();
                }),
            )
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
                let value = match ev.keystroke.key.as_str() {
                    "left" | "down" => value - step,
                    "right" | "up" => value + step,
                    "home" => 0.,
                    "end" => 1.,
                    "escape" if keyboard.preview().is_some() => {
                        keyboard.cancel();
                        cx.notify();
                        cx.stop_propagation();
                        return;
                    }
                    _ => return,
                };
                keyboard.cancel();
                commit(this, value.clamp(0., 1.), cx);
                cx.stop_propagation();
            }))
            .child(
                div()
                    .absolute()
                    .left(px(THUMB / 2.))
                    .right(px(THUMB / 2.))
                    .top(px(11.))
                    .h(px(4.))
                    .rounded_full()
                    .bg(rgb(p.line))
                    .child(
                        div()
                            .h_full()
                            .w(relative(value))
                            .rounded_full()
                            .bg(rgb(p.accent_ink)),
                    ),
            )
            // The thumb travels from half a thumb in at each end.
            .child(
                div()
                    .absolute()
                    .left(px(THUMB / 2.))
                    .right(px(THUMB / 2.))
                    .top(px(5.))
                    .h(px(THUMB))
                    .child(
                        div()
                            .absolute()
                            .left(relative(value))
                            .ml(px(-THUMB / 2.))
                            .size(px(THUMB))
                            .rounded_full()
                            .bg(rgb(if dragging { p.accent_ink } else { p.text })),
                    ),
            )
            .child(follow)
    }
}
