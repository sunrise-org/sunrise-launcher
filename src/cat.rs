//! The website's cat, hiding behind Play: now and then it peeks out, or walks out into the empty
//! part of the play bar and stays a while, or climbs up and sleeps on Play. In the bar a click
//! pets it, awake or dozing. On Play it can't be petted: it is a picture with no handler and no
//! hitbox of its own, so a click there goes to whatever is under it, as if no cat were there.
//!
//! Its look and its ways are the Sunrise site's, ported from sunrise-org/sunny
//! (`frontend/site/art/`: `cat.ts` for the poses and coats, `cat-actions.ts` for what it does and
//! how it takes being petted, `core.ts` for the pixel helpers). The site's cat breathes all the
//! time; this one holds still between actions, so a resting cat costs no frames.
mod actions;
mod art;
mod behaviour;
mod drawing;
mod outing;
mod talk;
#[cfg(test)]
mod tests;

use super::*;
use gpui::RenderImage;
use std::sync::Arc;
use std::time::{Duration, Instant};

use actions::*;
use art::*;
use behaviour::*;
use drawing::*;
pub(crate) use outing::*;
use talk::bubble_alpha;

impl Launcher {
    /// The empty stretch of the play bar between its text and Play, and the cat when it is out.
    /// The cat is drawn inside this stretch and cut at its edges, under Play, so it comes out
    /// from behind the button and can never lie over text or a control. `beside` says Play is
    /// what follows; with other buttons in between the cat stays in.
    ///
    /// The second element is the cat asleep on Play, for Play's own box: it is drawn only above
    /// the button's top edge and takes no clicks. `free` says Play is idle and the bar has
    /// nothing to say; the cat climbs up only then, and drops off the moment that ends.
    pub(super) fn cat(
        &mut self,
        beside: bool,
        free: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Div, Option<Div>) {
        let measured = self.ui.cat.lane.clone();
        // The lane's box is `lane_box`; this is the part of it that cuts the cat off at its edges,
        // so the speech bubble over the cat's head can be drawn beside it, uncut.
        let lane = div().absolute().size_full().overflow_hidden().child(
            gpui::canvas(
                move |bounds, _, _| measured.set(f32::from(bounds.size.width)),
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        );
        // Switched off, or the player asked for less motion: the cat stays away. Nothing of it
        // is kept and no wake is scheduled, so an idle window draws nothing for it.
        if !cat_out(self.settings.show_cat(), crate::motion::still()) {
            if let Some(image) = self.ui.cat.switch_off() {
                let _ = window.drop_image(image);
            }
            return (lane_box(beside, lane, None), None);
        }
        let roomy = beside && self.window_width >= ROOM.0 && self.window_height >= ROOM.1;
        let room = if roomy {
            self.ui.cat.lane.get() - UNDER - 6.
        } else {
            0.
        };
        let free = free && roomy;
        // A window in the background keeps its cat still: nothing is scheduled until it is back.
        let awake = window.is_window_active();
        if awake {
            if let Some(seen) = self.ui.cat.seen {
                self.ui.cat.clock += seen.elapsed().as_secs_f32() * 1000.;
            }
        }
        self.ui.cat.seen = awake.then(Instant::now);
        let now = self.ui.cat.clock;
        let outing = self.ui.cat.cat.get_or_insert_with(|| {
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(4, |d| d.as_millis() as u64);
            // One of the site's two cats, a different one from launch to launch.
            let ways = if seed.is_multiple_of(2) {
                &SIAMESE_WAYS
            } else {
                &TABBY_WAYS
            };
            let mut outing = Outing::new(ways, seed, 0.);
            outing.scene = Scene::Away {
                until: outing.cat.random.span([15000., 40000.]),
            };
            outing
        });
        if awake {
            if room <= 0.
                && matches!(
                    outing.scene,
                    Scene::Peek { .. } | Scene::Walk { .. } | Scene::Visit { .. }
                )
            {
                outing.hide(now);
            }
            outing.step(now, room, free);
        }
        let after = outing.next_change(now);
        let outing_bubble = outing.bubble;
        let coat = outing.cat.ways.coat;
        let view = outing
            .view(now)
            .filter(|view| room > 0. || view.perch.is_some());
        if awake {
            self.cat_wake(after, cx);
        }
        let Some(view) = view else {
            return (lane_box(beside, lane, None), None);
        };
        let scale = (SIZE * self.scale).round().max(1.) as usize;
        let image = match &self.ui.cat.shown {
            Some((drawn, at, left, image))
                if *drawn == view.pose && *at == scale && *left == view.left =>
            {
                image.clone()
            }
            _ => {
                let small = picture(coat, &view.pose);
                let mut big = vec![0u8; W * scale * H * scale * 4];
                for (y, row) in big.chunks_exact_mut(W * scale * 4).enumerate() {
                    for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                        // The cat is drawn facing right; facing left is its mirror image.
                        let x = if view.left {
                            W - 1 - x / scale
                        } else {
                            x / scale
                        };
                        let from = (y / scale * W + x) * 4;
                        px.copy_from_slice(&small.0[from..from + 4]);
                    }
                }
                let frame = image::RgbaImage::from_raw((W * scale) as u32, (H * scale) as u32, big)
                    .expect("the buffer is the picture's size");
                let image = Arc::new(RenderImage::new([image::Frame::new(frame)]));
                if let Some((_, _, _, old)) =
                    self.ui
                        .cat
                        .shown
                        .replace((view.pose, scale, view.left, image.clone()))
                {
                    let _ = window.drop_image(old);
                }
                image
            }
        };
        let tall = H as f32 * SIZE;
        // The cat's highest opaque row in this very picture, for the bubble to sit above.
        let spot = outing_bubble.and_then(|_| head_spot(&image, scale));
        let top = spot.map(|(top, _)| top);
        if let Some(up) = view.perch {
            // A window above Play's top edge, as tall as the cat: it climbs into it from below,
            // its feet on the edge, and nothing of it is ever drawn over the button. Keep it
            // without an id, a listener or `occlude`: any of those would take a click from the
            // page or the bar under it.
            let feet = (H as f32 - GY - 1.) * SIZE;
            let down = ((1. - up) * (tall - feet) / SIZE).round() * SIZE;
            let perch = div()
                .absolute()
                .top(px(feet - tall))
                .right(px(18.))
                .w(px(WIDTH))
                .h(px(tall - feet))
                .overflow_hidden()
                .child(
                    gpui::img(image)
                        .absolute()
                        .top(px(down))
                        .left(px(0.))
                        .w(px(WIDTH))
                        .h(px(tall)),
                );
            return (lane_box(beside, lane, None), Some(perch));
        }
        // Whole cat pixels, so the picture stays crisp as it moves.
        let out = (view.out / SIZE).round() * SIZE;
        let cat = div()
            .id("alt-cat")
            .absolute()
            .top(px(0.))
            .right(px(out + UNDER - WIDTH))
            .child(gpui::img(image).w(px(WIDTH)).h(px(tall)));
        let lane = lane.child(if view.pettable {
            cat.cursor_pointer()
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    let now = this.ui.cat.clock;
                    if let Some(outing) = &mut this.ui.cat.cat {
                        outing.speak(now);
                    }
                    cx.notify();
                }))
        } else {
            cat
        });
        let bubble = outing_bubble.and_then(|b| {
            let alpha = bubble_alpha(now - b.since, b.lasts, crate::motion::still())?;
            // The cat's head, from the right edge the cat itself is placed from: the sprite
            // stands `out + UNDER - WIDTH` in from it, and the head is `head` sprite pixels in
            // from the sprite's left, read off the picture drawn.
            let head = spot.map_or(W as f32 / 2., |(_, x)| x);
            Some(speech_bubble(
                tr(b.line),
                out + UNDER - head * SIZE,
                self.ui.cat.lane.get(),
                alpha,
                tall,
                top.unwrap_or(0) as f32 * SIZE,
            ))
        });
        (lane_box(beside, lane, bubble), None)
    }

    /// Play was pressed: a cat asleep on it hops off.
    pub(super) fn cat_off_play(&mut self) {
        let now = self.ui.cat.clock;
        if let Some(outing) = &mut self.ui.cat.cat {
            outing.leave_play(now);
        }
    }

    /// Draws the launcher again `after` this, unless a wake is already due by then.
    fn cat_wake(&mut self, after: Duration, cx: &mut Context<Self>) {
        let at = Instant::now() + after;
        if self
            .ui
            .cat
            .wake
            .is_some_and(|due| due <= at + Duration::from_millis(5))
        {
            return;
        }
        self.ui.cat.wake = Some(at);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(after).await;
            let _ = this.update(cx, |this, cx| {
                if this.ui.cat.wake.take_if(|due| *due == at).is_some()
                    && cat_out(
                        this.settings.show_cat(),
                        this.settings.reduced_graphics() || crate::motion::still(),
                    )
                {
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

/// How far the bubble's tail tip stays above the cat's highest pixel, in px.
const TIP_GAP: f32 = 4.;

/// Where the cat's head is in the picture actually drawn (already turned the way the cat faces):
/// its highest opaque row, and the middle of the widest stretch of the rows under it, in sprite
/// pixels. The head is the bulk up there; a raised tail is thin, so it never wins.
fn head_spot(image: &RenderImage, scale: usize) -> Option<(usize, f32)> {
    const BAND: usize = 14;
    const HEAD: usize = 12;
    let bytes = image.as_bytes(0)?;
    let row = W * scale * 4;
    let opaque = |y: usize, x: usize| {
        let at = (y * scale) * row + (x * scale) * 4 + 3;
        bytes.get(at).is_some_and(|a| *a > 0)
    };
    let rows = bytes.len() / row / scale;
    let top = (0..rows).find(|y| (0..W).any(|x| opaque(*y, x)))?;
    let mass: Vec<usize> = (0..W)
        .map(|x| {
            (top..(top + BAND).min(rows))
                .filter(|y| opaque(*y, x))
                .count()
        })
        .collect();
    let width = HEAD.min(W);
    let start = (0..=W - width)
        .max_by_key(|x| mass[*x..*x + width].iter().sum::<usize>())
        .unwrap_or(0);
    Some((top, start as f32 + width as f32 / 2.))
}

/// The lane's box: as wide as the bar's gap leaves, as tall as the cat. It does not cut what is
/// drawn in it; `clipped` (the cat's own lane, filling it) does that, and `bubble` sits above.
fn lane_box(beside: bool, clipped: Div, bubble: Option<Div>) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .h(px(H as f32 * SIZE))
        .relative()
        // Over the bar's gap and a little under Play.
        .when(beside, |d| d.mr(px(-(14. + UNDER))))
        .child(clipped)
        .children(bubble)
}

/// A speech bubble over the cat, whose head is `head` px from the lane's left edge (`lane`
/// wide): a small pixel box in the page's colours, its tail pointing down at the cat. It is kept
/// inside the lane, so the banner never cuts it, and the tail still points at the cat. `above`
/// is how far down the lane's box the cat's highest pixel is: the tail's tip stops `TIP_GAP` short of it.
/// `head` is how far the cat's head is in from the right edge both are placed from.
fn speech_bubble(text: &str, head: f32, lane: f32, alpha: f32, tall: f32, above: f32) -> Div {
    use crate::tokens::{r, R_SMALL, SMALL};
    const MAX: f32 = 220.;
    const PAD: f32 = 8.;
    const TAIL: f32 = 10.;
    let p = crate::paint::palette();
    // A guess at the text's width, so the box hugs a short line and wraps a long one at MAX.
    let wide = (text.chars().count() as f32 * SMALL * 0.56 + 2. * PAD + 4.).clamp(48., MAX);
    // Centred over the head where it fits, kept inside the lane.
    let right = (head - wide / 2.).min(lane - wide).max(0.);
    // The tail's left edge, so its tip is over the head, kept clear of the bubble's rounded corners.
    let corner = crate::tokens::R_SMALL;
    let tip_from_left = wide - (head - right);
    let tail_x = (tip_from_left - TAIL / 2.).clamp(corner, (wide - corner - TAIL).max(corner));
    let line = gpui::rgb(p.line);
    let fill = gpui::rgb(p.raised);
    // The tail in three steps, 10, 6 and 2 px wide, each 2 px tall: a pixel arrow.
    let step = |w: f32, solid: bool| {
        div()
            .w(px(w))
            .h(px(2.))
            .bg(line)
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(if solid { 0. } else { w - 4. }))
                    .h(px(2.))
                    .bg(fill),
            )
    };
    div()
        .absolute()
        .right(px(right))
        .bottom(px(tall - above + TIP_GAP))
        .w(px(wide))
        .opacity(alpha)
        .flex()
        .flex_col()
        .child(
            div()
                .px(px(PAD))
                .py(px(5.))
                .rounded(r(R_SMALL))
                .bg(fill)
                .border_1()
                .border_color(line)
                .text_color(gpui::rgb(p.text))
                .text_size(px(SMALL))
                .child(text.to_string()),
        )
        .child(
            div()
                .ml(px(tail_x))
                .mt(px(-1.))
                .flex()
                .flex_col()
                .items_center()
                .child(step(TAIL, false))
                .child(step(6., false))
                .child(step(2., true)),
        )
}
