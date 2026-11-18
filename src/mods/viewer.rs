//! A page's picture enlarged over the window: it grows out of its thumbnail, steps through the
//! gallery with ‹ › or the arrow keys, and shrinks back into its thumbnail on Esc, ✕ or a click
//! on the dark around it.
use super::ui::*;
use super::*;
use crate::frame::TEXT;
use gpui::{
    img, AnimationExt, Bounds, ElementId, MouseButton, ObjectFit, Pixels, Point, ScrollHandle, Size,
};
use std::path::PathBuf;

/// How long growing and shrinking take, on the launcher's curve for things that move.
const GROW: sunrise_launcher_theme::Motion = sunrise_launcher_theme::Motion {
    name: "grow",
    ms: 280,
    curve: sunrise_launcher_theme::MOVE.curve,
};
/// How far a click on the enlarged picture zooms in, and how long that takes.
const ZOOM: f32 = 2.5;
const ZOOMING: sunrise_launcher_theme::Motion = sunrise_launcher_theme::Motion {
    name: "zoom",
    ms: 220,
    curve: sunrise_launcher_theme::MOVE.curve,
};
/// The room kept free around the picture: the title bar above, the arrows beside it, the count
/// below.
const ROOM: (f32, f32, f32) = (56., 72., 64.);

thread_local! {
    /// Each thumbnail of the open page's gallery tracked by a handle, for the picture to grow
    /// out of and shrink back into. A scrolled page moves its drawn layer without drawing it
    /// again, so only a handle knows where a thumbnail is now; nothing scrolls by these.
    static THUMBS: [ScrollHandle; 4] = Default::default();
}

thread_local! {
    /// Each picture's size in pixels, read once from its file's header.
    static SIZES: std::cell::RefCell<std::collections::HashMap<PathBuf, Option<(u32, u32)>>> =
        Default::default();
}

/// A picture's width and height in pixels, when its file says.
pub(super) fn picture_size(path: &std::path::Path) -> Option<(u32, u32)> {
    SIZES.with_borrow_mut(|sizes| {
        *sizes
            .entry(path.to_path_buf())
            .or_insert_with(|| image::image_dimensions(path).ok())
    })
}

/// Where the thumbnail at `slot` is on the screen now, once it has been drawn.
fn thumb_bounds(slot: usize) -> Option<Bounds<Pixels>> {
    let bounds = THUMBS.with(|t| t.get(slot).map(ScrollHandle::bounds))?;
    (bounds.size.width > px(0.)).then_some(bounds)
}

/// One of the gallery's pictures: its place among the thumbnails, its file and its size.
#[derive(Clone)]
struct Shot {
    slot: usize,
    path: PathBuf,
    size: Option<(u32, u32)>,
}

pub(super) struct Viewer {
    shots: Vec<Shot>,
    at: usize,
    closing: bool,
    /// Counts opens and closes, so each plays its own animation.
    turn: usize,
    /// Zoomed in by a click, and the point of the picture (0 to 1 across and down) that was
    /// clicked, which stays where it was while zoomed.
    zoomed: bool,
    focus: (f32, f32),
    /// Counts zooms in and out, so each plays its own animation.
    zoom_turn: usize,
}

/// A thumbnail at `slot` that opens the viewer on a click and notes where it is drawn.
pub(super) fn thumb(
    slot: usize,
    el: Div,
    shots: &[Option<PathBuf>],
    cx: &mut Context<Launcher>,
) -> AnyElement {
    let shots: Vec<_> = shots
        .iter()
        .enumerate()
        .filter_map(|(i, p)| Some((i, p.clone()?)))
        .collect();
    if !shots.iter().any(|(i, _)| *i == slot) {
        return el.into_any_element();
    }
    el.id(ElementId::named_usize("alt-shot", slot))
        .track_scroll(&THUMBS.with(|t| t[slot].clone()))
        .cursor_pointer()
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            let shots = shots
                .iter()
                .map(|(i, p)| Shot {
                    slot: *i,
                    path: p.clone(),
                    size: picture_size(p),
                })
                .collect::<Vec<_>>();
            let at = shots.iter().position(|s| s.slot == slot).unwrap_or(0);
            let turn = this.ui.mods.viewer.as_ref().map_or(0, |v| v.turn + 1);
            this.ui.mods.viewer = Some(Viewer {
                shots,
                at,
                closing: false,
                turn,
                zoomed: false,
                focus: (0.5, 0.5),
                zoom_turn: 0,
            });
            cx.notify();
        }))
        .into_any_element()
}

/// Where a picture of `size` sits: as large as fits in `window` with [`ROOM`] around it, up to
/// its own size.
fn fitted(size: Option<(u32, u32)>, window: Size<Pixels>) -> Bounds<Pixels> {
    let (top, side, bottom) = ROOM;
    let (w, h) = (f32::from(window.width), f32::from(window.height));
    let (room_w, room_h) = ((w - 2. * side).max(1.), (h - top - bottom).max(1.));
    let (pw, ph) = size.map_or((16., 9.), |(pw, ph)| (pw.max(1) as f32, ph.max(1) as f32));
    // Never past its own size: a bigger window would only blur it. A click zooms in instead.
    let scale = (room_w / pw).min(room_h / ph).min(1.);
    let (fw, fh) = (pw * scale, ph * scale);
    Bounds::new(
        gpui::point(px((w - fw) / 2.), px(top + (room_h - fh) / 2.)),
        gpui::size(px(fw), px(fh)),
    )
}

fn lerp(a: Bounds<Pixels>, b: Bounds<Pixels>, t: f32) -> Bounds<Pixels> {
    let mix = |a: Pixels, b: Pixels| a + (b - a) * t;
    Bounds::new(
        Point::new(mix(a.origin.x, b.origin.x), mix(a.origin.y, b.origin.y)),
        Size::new(
            mix(a.size.width, b.size.width),
            mix(a.size.height, b.size.height),
        ),
    )
}

/// A round control over the dark: ✕, ‹ or ›.
fn control(id: &'static str, glyph: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(40.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        // Over the dark, which closes the viewer on a press: the press is this control's.
        .occlude()
        .bg(gpui::black().opacity(0.45))
        .hover(|s| s.bg(gpui::black().opacity(0.7)))
        .cursor_pointer()
        .text_size(px(20.))
        .text_color(c(TEXT))
        .child(glyph)
}

impl Launcher {
    /// Shrinks the picture back into its thumbnail; the viewer goes once it has.
    pub(crate) fn viewer_close(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(v) = self.ui.mods.viewer.as_mut().filter(|v| !v.closing) else {
            return false;
        };
        v.closing = true;
        v.zoomed = false;
        v.turn += 1;
        let turn = v.turn;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(GROW.duration()).await;
            let _ = this.update(cx, |this, cx| {
                if this.ui.mods.viewer.as_ref().is_some_and(|v| v.turn == turn) {
                    this.ui.mods.viewer = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
        true
    }

    /// Shows the next (`by` 1) or the previous (`by` -1) picture, round the ends.
    pub(crate) fn viewer_step(&mut self, by: isize, cx: &mut Context<Self>) -> bool {
        let Some(v) = self.ui.mods.viewer.as_mut().filter(|v| !v.closing) else {
            return false;
        };
        let n = v.shots.len() as isize;
        v.at = (v.at as isize + by).rem_euclid(n) as usize;
        v.zoomed = false;
        cx.notify();
        true
    }

    /// The viewer over everything, when a picture is open.
    pub(crate) fn viewer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let v = self.ui.mods.viewer.as_ref()?;
        let Shot { slot, path, size } = v.shots.get(v.at)?.clone();
        let (closing, turn, many) = (v.closing, v.turn, v.shots.len() > 1);
        let (zoomed, (fx, fy), zoom_turn) = (v.zoomed, v.focus, v.zoom_turn);
        let full = fitted(size, window.viewport_size());
        // A thumbnail that was never drawn (scrolled away before it was): grow from the middle.
        let thumb = thumb_bounds(slot).unwrap_or_else(|| {
            Bounds::centered_at(
                full.center(),
                Size::new(full.size.width * 0.6, full.size.height * 0.6),
            )
        });
        let (from, to) = match closing {
            true => (full, thumb),
            false => (thumb, full),
        };
        let fade = move |t: f32| if closing { 1. - t } else { t };
        let id = |part: &'static str| ElementId::NamedInteger(part.into(), turn as u64);

        let dark = div()
            .absolute()
            .size_full()
            .bg(gpui::black().opacity(0.85))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.viewer_close(cx);
                }),
            )
            .with_animation(
                id("alt-viewer-dark"),
                crate::motion::eased(GROW),
                move |d, t| d.opacity(fade(t)),
            );
        // Where the pointer is over the picture, 0 to 1 across and down.
        let over = move |at: Point<Pixels>| {
            let f = |at: Pixels, start: Pixels, len: Pixels| {
                (f32::from(at - start) / f32::from(len).max(1.)).clamp(0., 1.)
            };
            (
                f(at.x, full.origin.x, full.size.width),
                f(at.y, full.origin.y, full.size.height),
            )
        };
        // The picture inside its frame, at `scale` times the frame, moved so the focus point
        // stays where it shows: zoomed in, the point clicked stays under the pointer.
        let zoom = move |scale: f32| {
            img(path.clone())
                .absolute()
                .left(gpui::relative(-fx * (scale - 1.)))
                .top(gpui::relative(-fy * (scale - 1.)))
                .w(gpui::relative(scale))
                .h(gpui::relative(scale))
                .object_fit(ObjectFit::Cover)
        };
        let inner = div().size_full().with_animation(
            ElementId::NamedInteger("alt-viewer-zoom".into(), zoom_turn as u64),
            crate::motion::eased(ZOOMING),
            move |d, t| {
                let scale = match zoomed {
                    true => 1. + (ZOOM - 1.) * t,
                    false => ZOOM - (ZOOM - 1.) * t,
                };
                // The first open has nothing to zoom out of.
                let scale = if zoom_turn == 0 { 1. } else { scale };
                d.child(zoom(scale))
            },
        );
        let picture = div()
            .id("alt-viewer-picture-frame")
            .absolute()
            .rounded(px(tokens::R_CONTROL))
            .overflow_hidden()
            .occlude()
            .cursor_pointer()
            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                if let Some(v) = this.ui.mods.viewer.as_mut().filter(|v| !v.closing) {
                    v.zoomed = !v.zoomed;
                    v.focus = over(ev.position());
                    v.zoom_turn += 1;
                    cx.notify();
                }
            }))
            .child(inner)
            .with_animation(
                id("alt-viewer-picture"),
                crate::motion::eased(GROW),
                move |d, t| {
                    let r = lerp(from, to, t);
                    d.left(r.origin.x)
                        .top(r.origin.y)
                        .w(r.size.width)
                        .h(r.size.height)
                },
            );
        let mut overlay = div()
            .id("alt-viewer")
            .absolute()
            .size_full()
            .occlude()
            .child(dark)
            .child(picture);
        if !closing {
            let (count, at) = (
                self.ui.mods.viewer.as_ref()?.shots.len(),
                self.ui.mods.viewer.as_ref()?.at,
            );
            let mut controls = div().absolute().size_full().child(
                control("alt-viewer-close", "✕")
                    .absolute()
                    .top(px(ROOM.0 - 8.))
                    .right(px(16.))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.viewer_close(cx);
                    })),
            );
            if many {
                let mid = full.center().y - px(20.);
                controls = controls
                    .child(
                        control("alt-viewer-back", "‹")
                            .absolute()
                            .left(px(16.))
                            .top(mid)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.viewer_step(-1, cx);
                            })),
                    )
                    .child(
                        control("alt-viewer-next", "›")
                            .absolute()
                            .right(px(16.))
                            .top(mid)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.viewer_step(1, cx);
                            })),
                    )
                    .child(
                        div()
                            .absolute()
                            .w_full()
                            .top(full.bottom() + px(20.))
                            .flex()
                            .justify_center()
                            .text_size(px(tokens::LABEL))
                            .text_color(c(TEXT))
                            .child(format!("{} / {count}", at + 1)),
                    );
            }
            overlay = overlay.child(controls.with_animation(
                id("alt-viewer-controls"),
                crate::motion::eased(GROW),
                |d, t| d.opacity(t),
            ));
        }
        Some(overlay.into_any_element())
    }
}
