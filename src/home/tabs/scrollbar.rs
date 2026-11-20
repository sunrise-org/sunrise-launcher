//! An overlay thumb that uses the active list's own geometry, including its virtual tail.
use super::*;
use gpui::{DispatchPhase, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels};

#[derive(Clone, Default)]
pub(super) struct Thumb {
    held: Rc<Cell<Option<Pixels>>>,
}

fn geometry(scroll: &ScrollHandle) -> Option<(Pixels, Pixels)> {
    let view = scroll.bounds();
    let height = view.size.height;
    let max = scroll.max_offset().y;
    if max <= px(0.) || height <= px(1.) {
        return None;
    }
    let thumb = (height * (height / (height + max)))
        .max(px(28.))
        .min(height);
    Some((thumb, height - thumb))
}

impl Thumb {
    pub(super) fn draw(&self, scroll: &ScrollHandle, cx: &mut Context<Launcher>) -> Div {
        let scroll = scroll.clone();
        let held = self.held.clone();
        let me = cx.entity().downgrade();
        let color = Hsla::from(rgb(palette().muted)).opacity(0.72);
        div()
            .child(
                gpui::canvas(
                    |_, _, _| {},
                    move |area, (), window, _| {
                        let Some((thumb, travel)) = geometry(&scroll) else {
                            return;
                        };
                        let max = scroll.max_offset().y;
                        let top = area.top() + travel * ((-scroll.offset().y / max).clamp(0., 1.));
                        let bounds = Bounds::new(
                            gpui::point(area.left() + px(3.), top),
                            gpui::size(px(4.), thumb),
                        );
                        window.paint_quad(gpui::fill(bounds, color).corner_radii(px(2.)));
                        if travel <= px(0.) {
                            return;
                        }
                        let (down, start, owner) = (scroll.clone(), held.clone(), me.clone());
                        window.on_mouse_event(move |ev: &MouseDownEvent, phase, _, cx| {
                            if phase != DispatchPhase::Bubble
                                || ev.button != MouseButton::Left
                                || !area.contains(&ev.position)
                            {
                                return;
                            }
                            // A press on the track centres the thumb there; a press on the thumb grabs it.
                            let by = if ev.position.y >= top && ev.position.y <= top + thumb {
                                ev.position.y - top
                            } else {
                                thumb / 2.
                            };
                            start.set(Some(by));
                            let fraction =
                                ((ev.position.y - area.top() - by) / travel).clamp(0., 1.);
                            down.set_offset(gpui::point(px(0.), -max * fraction));
                            let _ = owner.update(cx, |_, cx| cx.notify());
                            cx.stop_propagation();
                        });
                        let (moving, grabbed, owner) = (scroll.clone(), held.clone(), me.clone());
                        window.on_mouse_event(move |ev: &MouseMoveEvent, phase, _, cx| {
                            if phase != DispatchPhase::Capture {
                                return;
                            }
                            let Some(by) = grabbed.get() else {
                                return;
                            };
                            if ev.pressed_button != Some(MouseButton::Left) {
                                grabbed.set(None);
                                return;
                            }
                            let fraction =
                                ((ev.position.y - area.top() - by) / travel).clamp(0., 1.);
                            moving.set_offset(gpui::point(px(0.), -max * fraction));
                            let _ = owner.update(cx, |_, cx| cx.notify());
                        });
                        let released = held.clone();
                        window.on_mouse_event(move |ev: &MouseUpEvent, phase, _, _| {
                            if phase == DispatchPhase::Capture && ev.button == MouseButton::Left {
                                released.set(None);
                            }
                        });
                    },
                )
                .size_full(),
            )
            .absolute()
            .right(px(-11.))
            .top_0()
            .bottom_0()
            .w(px(11.))
    }
}
