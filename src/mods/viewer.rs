//! A page's picture enlarged over the window: it grows out of its tile, steps through the
//! gallery with ‹ › or the arrow keys, zooms in where it is clicked, and shrinks back into its
//! tile on Esc, ✕ or a click on the dark around it. Pictures load as they are shown, so a pack's
//! long gallery isn't fetched on opening its page.
use super::*;
use crate::frame::c;
use crate::paint::palette;
use crate::widgets::button::{self, icon_only, Look, Style};
use gpui::{
    img, AnimationExt, Bounds, ElementId, MouseButton, ObjectFit, Pixels, Point, ScrollHandle, Size,
};
use std::cell::RefCell;
use std::path::PathBuf;
use sunrise_launcher::browser::images;

/// How long growing and shrinking take, and on what curve. Not [`MOVE`]'s: its long-tailed
/// ease-out spends a quarter of a tile-to-window trip in the first frame and looks done by the
/// sixth, so the picture jumped ~150 px a frame and read as a low frame rate. Easing in and out
/// spreads the trip over every frame (at most ~85 px a frame at 60 Hz).
///
/// [`MOVE`]: sunrise_launcher_theme::MOVE
const GROW: sunrise_launcher_theme::Motion = sunrise_launcher_theme::Motion {
    name: "grow",
    ms: 320,
    curve: sunrise_launcher_theme::QUICK.curve,
};
/// How far a click on the enlarged picture zooms in, and how long that takes.
const ZOOM: f32 = 2.5;
const ZOOMING: sunrise_launcher_theme::Motion = sunrise_launcher_theme::Motion {
    name: "zoom",
    ms: 260,
    curve: sunrise_launcher_theme::QUICK.curve,
};
/// The room kept free around the picture: the title bar above, the arrows beside it, the count
/// below.
const ROOM: (f32, f32, f32) = (56., 72., 64.);

thread_local! {
    /// Each tile of the open page's gallery tracked by a handle, for the picture to grow out of
    /// and shrink back into. A scrolled page moves its drawn layer without drawing it again, so
    /// only a handle knows where a tile is now; nothing scrolls by these.
    static TILES: RefCell<Vec<ScrollHandle>> = const { RefCell::new(Vec::new()) };
    /// Each picture's size in pixels, read once from its file's header.
    static SIZES: RefCell<std::collections::HashMap<PathBuf, Option<(u32, u32)>>> =
        RefCell::default();
}

/// The handle for the gallery's tile `slot`, for its `track_scroll`.
pub(super) fn tile(slot: usize) -> ScrollHandle {
    TILES.with_borrow_mut(|t| {
        while t.len() <= slot {
            t.push(ScrollHandle::new());
        }
        t[slot].clone()
    })
}

/// Where tile `slot` is on the screen now, once it has been drawn.
fn tile_bounds(slot: usize) -> Option<Bounds<Pixels>> {
    let bounds = TILES.with_borrow(|t| t.get(slot).map(ScrollHandle::bounds))?;
    (bounds.size.width > px(0.)).then_some(bounds)
}

/// A picture's width and height in pixels, when its file says.
pub(super) fn picture_size(path: &std::path::Path) -> Option<(u32, u32)> {
    SIZES.with_borrow_mut(|sizes| {
        *sizes
            .entry(path.to_path_buf())
            .or_insert_with(|| image::image_dimensions(path).ok())
    })
}

/// A gallery picture: an address the image cache fetches or a content-addressed v2 registry image.
#[derive(Clone)]
pub(super) enum Picture {
    Url(String),
    V2 {
        project_id: String,
        relative_path: String,
        caption: String,
    },
}

pub(super) struct Viewer {
    urls: Rc<[Picture]>,
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

impl Viewer {
    /// Toggles zoom. Zooming in focuses on `at`; zooming out keeps the existing focus so the
    /// picture shrinks back from where it already sits instead of jumping to the pointer first.
    fn toggle_zoom(&mut self, at: (f32, f32)) {
        self.zoomed = !self.zoomed;
        if self.zoomed {
            self.focus = at;
        }
        self.zoom_turn += 1;
    }
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

/// A control over the dark: a ghost icon button, as the window's own. The dark around it
/// closes the viewer on a press, so the press is the control's.
fn control(id: &'static str, glyph: &'static str) -> Stateful<Div> {
    icon_only(
        id,
        Look::new(Style::Ghost, button::Size::Medium).icon(glyph),
    )
    .occlude()
}

impl Launcher {
    /// The picture's file once it has loaded, asking for it the first time.
    pub(super) fn picture(&mut self, picture: &Picture, size: u32) -> Option<PathBuf> {
        match picture {
            Picture::Url(url) => self.mods_image(url, size),
            Picture::V2 {
                project_id,
                relative_path,
                ..
            } => self.v2_mod_image(project_id, relative_path, size),
        }
    }

    /// Opens the gallery `urls` at picture `at`, growing it out of its tile.
    pub(super) fn viewer_open(&mut self, urls: Rc<[Picture]>, at: usize) {
        let turn = self.mods_state.viewer.as_ref().map_or(0, |v| v.turn + 1);
        self.mods_state.viewer = Some(Viewer {
            urls,
            at,
            closing: false,
            turn,
            zoomed: false,
            focus: (0.5, 0.5),
            zoom_turn: 0,
        });
    }

    /// Shrinks the picture back into its thumbnail; the viewer goes once it has.
    pub(crate) fn viewer_close(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(v) = self.mods_state.viewer.as_mut().filter(|v| !v.closing) else {
            return false;
        };
        v.closing = true;
        v.zoomed = false;
        v.turn += 1;
        let turn = v.turn;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(GROW.duration()).await;
            let _ = this.update(cx, |this, cx| {
                if this
                    .mods_state
                    .viewer
                    .as_ref()
                    .is_some_and(|v| v.turn == turn)
                {
                    this.mods_state.viewer = None;
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
        let Some(v) = self.mods_state.viewer.as_mut().filter(|v| !v.closing) else {
            return false;
        };
        let n = v.urls.len() as isize;
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
        let v = self.mods_state.viewer.as_ref()?;
        let (at, count) = (v.at, v.urls.len());
        let picture = v.urls.get(at)?.clone();
        let caption = match &picture {
            Picture::V2 { caption, .. } if !caption.is_empty() => Some(caption.clone()),
            _ => None,
        };
        let (closing, turn, many) = (v.closing, v.turn, count > 1);
        let (zoomed, (fx, fy), zoom_turn) = (v.zoomed, v.focus, v.zoom_turn);
        let path = self.picture(&picture, images::GALLERY);
        let size = path.as_deref().and_then(picture_size);
        let full = fitted(size, window.viewport_size());
        // A picture past the tiles, or a tile never drawn: it grows from the middle.
        let thumb = tile_bounds(at).unwrap_or_else(|| {
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
            .bg(gpui::black().opacity(0.94))
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
            let Some(path) = path.clone() else {
                return div().into_any_element();
            };
            img(path)
                .absolute()
                .left(gpui::relative(-fx * (scale - 1.)))
                .top(gpui::relative(-fy * (scale - 1.)))
                .w(gpui::relative(scale))
                .h(gpui::relative(scale))
                .object_fit(ObjectFit::Cover)
                .into_any_element()
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
            .rounded(px(tokens::R_CARD))
            .overflow_hidden()
            // Its card's colour until the picture arrives.
            .bg(palette().card())
            .occlude()
            .cursor_pointer()
            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                if let Some(v) = this.mods_state.viewer.as_mut().filter(|v| !v.closing) {
                    v.toggle_zoom(over(ev.position()));
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
            let mut controls = div().absolute().size_full().child(
                control("alt-viewer-close", "cancel-01")
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
                        control("alt-viewer-back", "arrow-left-01")
                            .absolute()
                            .left(px(16.))
                            .top(mid)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.viewer_step(-1, cx);
                            })),
                    )
                    .child(
                        control("alt-viewer-next", "arrow-right-01")
                            .absolute()
                            .right(px(16.))
                            .top(mid)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.viewer_step(1, cx);
                            })),
                    );
            }
            if caption.is_some() || many {
                // One quiet line under the picture: its place in the gallery, then its caption.
                let place = many.then(|| {
                    trf(
                        "Picture {number} of {total}",
                        &[
                            ("number", &(at + 1).to_string()),
                            ("total", &count.to_string()),
                        ],
                    )
                });
                let line: Vec<String> = [place, caption].into_iter().flatten().collect();
                controls = controls.child(
                    div()
                        .absolute()
                        .w_full()
                        .top(full.bottom() + px(14.))
                        .px(px(24.))
                        .flex()
                        .justify_center()
                        .text_size(px(tokens::SMALL))
                        .text_color(c(palette().muted))
                        .child(line.join(" · ")),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_viewer() -> Viewer {
        Viewer {
            urls: Rc::from([]),
            at: 0,
            closing: false,
            turn: 0,
            zoomed: false,
            focus: (0.5, 0.5),
            zoom_turn: 0,
        }
    }

    #[test]
    fn zoom_out_keeps_zoom_in_focus() {
        let mut viewer = sample_viewer();

        viewer.toggle_zoom((0.2, 0.8));
        assert!(viewer.zoomed);
        assert_eq!(viewer.focus, (0.2, 0.8));
        assert_eq!(viewer.zoom_turn, 1);

        // Clicking somewhere else to zoom out must not jump the focus to the new pointer.
        viewer.toggle_zoom((0.9, 0.1));
        assert!(!viewer.zoomed);
        assert_eq!(viewer.focus, (0.2, 0.8));
        assert_eq!(viewer.zoom_turn, 2);

        // Zooming back in picks up the new click position.
        viewer.toggle_zoom((0.75, 0.25));
        assert!(viewer.zoomed);
        assert_eq!(viewer.focus, (0.75, 0.25));
        assert_eq!(viewer.zoom_turn, 3);
    }
}
