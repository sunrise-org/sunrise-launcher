//! A switch that will not be switched off by a pointer: the "Show the cat" easter egg.
//!
//! While the switch is on and the pointer comes near it, it flees: it is pushed away from the
//! pointer (with a sideways lean that is rolled again each time the pointer closes in), never
//! further than a leash from where it sits, and when it is cornered it hops across the pointer to
//! the far side. It follows its target on a spring, stepped every frame, so it speeds off,
//! overshoots a little and settles; frames are asked for only until it has settled. It is drawn
//! in a layer above the page (so the card's edges never clip it) while it is on or still moving;
//! the row keeps a placeholder of its size. The keyboard still toggles it, and under reduced
//! motion it does not move at all. Off, it behaves as any switch and glides home.
use super::*;
use crate::motion;
use gpui::{canvas, DispatchPhase, MouseMoveEvent, ScrollHandle};

/// How near the pointer is to the switch's centre before it flees.
const RADIUS: f32 = 80.;
/// The least and the most one flee step pushes it (the nearer the pointer, the more).
const PUSH: (f32, f32) = (26., 70.);
/// The farthest it strays from home, sideways and up or down.
const LEASH_X: f32 = 70.;
const LEASH_Y: f32 = 30.;
/// The pointer this far from home lets it drift back.
const RETURN: f32 = 200.;
/// The spring: its pull, and its drag (a damping ratio of 0.7, so it overshoots a touch).
const PULL: f32 = 640.;
const DRAG: f32 = 2. * 0.7 * 25.3;
/// The speed it bolts at when the pointer first comes near, in px per second.
const KICK: f32 = 180.;
/// How close a dodge may pass the pointer (centre to pointer) before it hops instead, in px.
const CLEAR: f32 = 30.;
/// How long a hop takes to pop in.
const HOP_POP: std::time::Duration = std::time::Duration::from_millis(140);

/// How close the segment from `a` to `b` comes to `p`.
fn segment_dist(a: Pt, b: Pt, p: Pt) -> f32 {
    let ab = (b.0 - a.0, b.1 - a.1);
    let len = ab.0 * ab.0 + ab.1 * ab.1;
    let t = if len <= f32::EPSILON {
        0.
    } else {
        (((p.0 - a.0) * ab.0 + (p.1 - a.1) * ab.1) / len).clamp(0., 1.)
    };
    dist((a.0 + ab.0 * t, a.1 + ab.1 * t), p)
}
/// The size of a switch (`check::switch`'s track), which the placeholder in the row keeps.
const SIZE: (f32, f32) = (36., 20.);

type Pt = (f32, f32);

struct Shy {
    pos: Pt,
    vel: Pt,
    target: Pt,
    /// When `pos` was last stepped.
    at: Instant,
    /// The pointer is within `RADIUS`, as of the last move.
    engaged: bool,
    /// Which way, from -1 to 1, it leans sideways of straight away from the pointer.
    bias: f32,
    seed: u32,
    /// When it last hopped (it pops in at its new place rather than slide past the pointer).
    hopped: Option<Instant>,
    /// Scrolls nothing: its bounds follow the home spot when a page scrolls (as the slider's).
    home: ScrollHandle,
}

impl Shy {
    fn new() -> Shy {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9e37_79b9, |d| d.subsec_nanos())
            | 1;
        Shy {
            pos: (0., 0.),
            vel: (0., 0.),
            target: (0., 0.),
            at: Instant::now(),
            engaged: false,
            bias: 0.,
            seed,
            hopped: None,
            home: ScrollHandle::new(),
        }
    }

    /// A number from -1 to 1.
    fn roll(&mut self) -> f32 {
        let mut x = self.seed;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        (x as f32 / u32::MAX as f32) * 2. - 1.
    }

    fn settled(&self) -> bool {
        self.pos == self.target && self.vel == (0., 0.)
    }

    fn go_home(&mut self) {
        self.pos = (0., 0.);
        self.vel = (0., 0.);
        self.target = (0., 0.);
        self.engaged = false;
    }

    /// Moves the spring on to `now`.
    fn step(&mut self, now: Instant) {
        let dt = now
            .saturating_duration_since(self.at)
            .as_secs_f32()
            .min(0.05);
        self.at = now;
        if self.settled() {
            return;
        }
        let n = (dt * 240.).ceil().max(1.) as usize;
        let h = dt / n as f32;
        for _ in 0..n {
            let ax = PULL * (self.target.0 - self.pos.0) - DRAG * self.vel.0;
            let ay = PULL * (self.target.1 - self.pos.1) - DRAG * self.vel.1;
            self.vel = (self.vel.0 + ax * h, self.vel.1 + ay * h);
            self.pos = (self.pos.0 + self.vel.0 * h, self.pos.1 + self.vel.1 * h);
        }
        let off = dist(self.pos, self.target);
        let speed = dist(self.vel, (0., 0.));
        if off < 0.15 && speed < 4. {
            self.pos = self.target;
            self.vel = (0., 0.);
        }
    }
}

thread_local! {
    static SHY: std::cell::RefCell<std::collections::HashMap<String, Shy>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

fn dist(a: Pt, b: Pt) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// `p` pulled back into the leash, an ellipse of `LEASH_X` by `LEASH_Y` around home.
fn keep_in_leash((x, y): Pt) -> Pt {
    let r = ((x / LEASH_X).powi(2) + (y / LEASH_Y).powi(2)).sqrt();
    if r > 1. {
        (x / r, y / r)
    } else {
        (x, y)
    }
}

fn unit((x, y): Pt) -> Option<Pt> {
    let len = (x * x + y * y).sqrt();
    (len > 0.5).then(|| (x / len, y / len))
}

/// The pointer moved to `at`: points the switch `key` away from it when it is near. Whether
/// anything changed (the view is then drawn again, and steps the spring).
fn nudge(key: &str, at: gpui::Point<gpui::Pixels>) -> bool {
    let now = Instant::now();
    SHY.with_borrow_mut(|all| {
        let Some(shy) = all.get_mut(key) else {
            return false;
        };
        let bounds = shy.home.bounds();
        if f32::from(bounds.size.width) <= 0. {
            return false;
        }
        let home = (f32::from(bounds.center().x), f32::from(bounds.center().y));
        let pointer = (f32::from(at.x), f32::from(at.y));
        let centre = (home.0 + shy.pos.0, home.1 + shy.pos.1);
        let away = (centre.0 - pointer.0, centre.1 - pointer.1);
        let near = dist(centre, pointer);
        let was_settled = shy.settled();
        let before = shy.target;
        if near < RADIUS {
            let fresh = !shy.engaged;
            if fresh {
                shy.engaged = true;
                shy.bias = shy.roll();
            }
            let straight = unit(away).unwrap_or_else(|| {
                let a = shy.roll() * std::f32::consts::PI;
                (a.cos(), a.sin())
            });
            // Away from the pointer, leaning to one side so it is not simply pinned to a wall.
            let side = (-straight.1, straight.0);
            let lean = shy.bias * 0.7;
            let dir =
                unit((straight.0 + side.0 * lean, straight.1 + side.1 * lean)).unwrap_or(straight);
            let push = PUSH.0 + (PUSH.1 - PUSH.0) * (1. - near / RADIUS);
            let to = keep_in_leash((shy.pos.0 + dir.0 * push, shy.pos.1 + dir.1 * push));
            let cornered = dist(to, shy.pos) < push * 0.45 && near < RADIUS * 0.7;
            // A path that would brush the pointer is no path: it hops there instead.
            let pointer_at = (pointer.0 - home.0, pointer.1 - home.1);
            let brushes = |to: Pt| segment_dist(shy.pos, to, pointer_at) < CLEAR;
            if cornered || brushes(to) {
                // Cornered against the leash: it hops across the pointer, to the far side.
                shy.bias = shy.roll();
                let mirror = (
                    2. * pointer.0 - centre.0 - home.0,
                    2. * pointer.1 - centre.1 - home.1,
                );
                let mut hop = keep_in_leash(mirror);
                if dist((home.0 + hop.0, home.1 + hop.1), pointer) < RADIUS * 0.6 {
                    let across = unit((home.0 - pointer.0, home.1 - pointer.1))
                        .unwrap_or((if shy.bias < 0. { -1. } else { 1. }, 0.));
                    hop = keep_in_leash((across.0 * LEASH_X, across.1 * LEASH_Y));
                }
                // It vanishes and pops in over there, so it never crosses the pointer.
                shy.target = hop;
                shy.pos = hop;
                shy.vel = (0., 0.);
                shy.hopped = Some(now);
            } else {
                shy.target = to;
                if fresh {
                    shy.vel = (shy.vel.0 + dir.0 * KICK, shy.vel.1 + dir.1 * KICK);
                }
            }
        } else {
            shy.engaged = false;
            if shy.target != (0., 0.) && dist(pointer, home) > RETURN {
                shy.target = (0., 0.);
            }
        }
        let changed = shy.target != before;
        if changed && was_settled {
            shy.at = now;
        }
        changed
    })
}

/// `inner` (a [`check::switch`](super::check::switch) with its click added) that dodges the
/// pointer while `on`. `id` names the switch and must be unique. The returned element takes the
/// switch's place in the row and is as big as it.
pub(crate) fn wrap(id: &str, on: bool, inner: impl IntoElement) -> Div {
    let now = Instant::now();
    let still = motion::still();
    let key = id.to_string();
    let (pos, home, animating, pop) = SHY.with_borrow_mut(|all| {
        let shy = all.entry(key.clone()).or_insert_with(Shy::new);
        if still {
            shy.go_home();
        } else {
            if !on {
                // Off, it goes home, smoothly.
                shy.engaged = false;
                if shy.target != (0., 0.) {
                    if shy.settled() {
                        shy.at = now;
                    }
                    shy.target = (0., 0.);
                }
            }
            shy.step(now);
        }
        // A hop pops in: it fades up at its new place over HOP_POP.
        let pop = shy.hopped.map_or(1., |at| {
            (now.saturating_duration_since(at).as_secs_f32() / HOP_POP.as_secs_f32()).min(1.)
        });
        if pop >= 1. {
            shy.hopped = None;
        }
        (shy.pos, shy.home.clone(), !shy.settled() || pop < 1., pop)
    });
    let floating = (on && !still) || pos != (0., 0.);
    let placed = div()
        .absolute()
        .left(px(pos.0))
        .top(px(pos.1))
        .opacity(pop)
        .child(inner);
    let placed = if floating {
        // Above the card's edges and everything near it, below open menus (priority 9 and up).
        gpui::deferred(placed).with_priority(6).into_any_element()
    } else {
        placed.into_any_element()
    };
    let owner = Owner::default();
    let mut home_box = div()
        .id(SharedString::from(format!("{key}-home")))
        .relative()
        .flex_none()
        .w(px(SIZE.0))
        .h(px(SIZE.1))
        .track_scroll(&home)
        .child(placed);
    if (on && !still) || animating {
        let (noted, listen) = (owner.clone(), key);
        home_box = home_box.child(owner.noting()).child(
            canvas(
                |_, _, _| {},
                move |_, _, window: &mut Window, _: &mut App| {
                    // Asks for the next frame only while the spring still moves.
                    if animating {
                        window.request_animation_frame();
                    }
                    if !on {
                        return;
                    }
                    let (owner, key) = (noted.clone(), listen.clone());
                    window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture && nudge(&key, ev.position) {
                            owner.redraw(window, cx);
                        }
                    });
                },
            )
            .absolute()
            .size_0(),
        );
    }
    div().flex_none().child(home_box)
}
