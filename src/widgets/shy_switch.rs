//! A switch that will not be switched off by a pointer: the "Show the cat" easter egg.
//!
//! While the switch is on and the pointer comes near it, it slides 32 px away from the pointer,
//! never further than a leash from where it sits. The row keeps its layout (only a relative
//! offset moves), the keyboard still toggles it, and under reduced motion it does not move at
//! all. Off, it behaves as any switch; it drifts home when the pointer is far from it.
use super::*;
use crate::motion;
use gpui::AnimationExt as _;
use gpui::{canvas, DispatchPhase, MouseMoveEvent, ScrollHandle};
use std::sync::atomic::{AtomicUsize, Ordering};
use sunrise_launcher_theme::QUICK;

/// How near the pointer is to the switch's centre before it moves.
const TRIGGER: f32 = 44.;
/// How far one dodge slides it.
const STEP: f32 = 32.;
/// The farthest it strays from home, sideways and up or down.
const LEASH: f32 = 60.;
const LEASH_Y: f32 = 24.;
/// The pointer this far from home lets it drift back.
const RETURN: f32 = 160.;

type Pt = (f32, f32);

/// A slide from one offset to another, started at `at`.
#[derive(Clone, Copy)]
struct Dodge {
    from: Pt,
    to: Pt,
    at: Instant,
    serial: usize,
}

impl Dodge {
    fn home() -> Dodge {
        Dodge {
            from: (0., 0.),
            to: (0., 0.),
            at: Instant::now(),
            serial: 0,
        }
    }

    fn moving(self, now: Instant) -> bool {
        now.saturating_duration_since(self.at) < QUICK.duration()
    }

    /// Where the switch is, as an offset from home, at `now`.
    fn at(self, now: Instant) -> Pt {
        let t = (now.saturating_duration_since(self.at).as_secs_f32() / QUICK.secs()).min(1.);
        let e = QUICK.ease(t);
        (
            self.from.0 + (self.to.0 - self.from.0) * e,
            self.from.1 + (self.to.1 - self.from.1) * e,
        )
    }

    fn toward(self, to: Pt, now: Instant) -> Dodge {
        static SERIAL: AtomicUsize = AtomicUsize::new(1);
        Dodge {
            from: self.at(now),
            to,
            at: now,
            serial: SERIAL.fetch_add(1, Ordering::Relaxed),
        }
    }
}

struct Shy {
    dodge: Dodge,
    /// Scrolls nothing: its bounds follow the home spot when a page scrolls (as the slider's).
    home: ScrollHandle,
}

thread_local! {
    static SHY: std::cell::RefCell<std::collections::HashMap<String, Shy>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

fn keep_in_leash((x, y): Pt) -> Pt {
    let (x, y) = (x.clamp(-LEASH, LEASH), y.clamp(-LEASH_Y, LEASH_Y));
    let len = (x * x + y * y).sqrt();
    if len > LEASH {
        (x * LEASH / len, y * LEASH / len)
    } else {
        (x, y)
    }
}

fn unit((x, y): Pt) -> Option<Pt> {
    let len = (x * x + y * y).sqrt();
    (len > 0.5).then(|| (x / len, y / len))
}

/// The pointer moved to `at`: slides the switch `key` away when it is near. Whether it moved.
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
        let now_at = shy.dodge.at(now);
        let centre = (home.0 + now_at.0, home.1 + now_at.1);
        let away = (centre.0 - pointer.0, centre.1 - pointer.1);
        let dist = (away.0 * away.0 + away.1 * away.1).sqrt();
        if dist < TRIGGER {
            let dir = unit(away).unwrap_or((1., 0.));
            let mut to = keep_in_leash((now_at.0 + dir.0 * STEP, now_at.1 + dir.1 * STEP));
            let slid = ((to.0 - now_at.0).powi(2) + (to.1 - now_at.1).powi(2)).sqrt();
            if slid < STEP * 0.4 {
                // Cornered against the leash: it hops to the far side of home instead.
                let from_home = (pointer.0 - home.0, pointer.1 - home.1);
                let across = unit(from_home).map_or((1., 0.), |(x, y)| (-x, -y));
                to = keep_in_leash((across.0 * (LEASH - 5.), across.1 * (LEASH_Y - 4.)));
            }
            shy.dodge = shy.dodge.toward(to, now);
            true
        } else if shy.dodge.to != (0., 0.)
            && ((pointer.0 - home.0).powi(2) + (pointer.1 - home.1).powi(2)).sqrt() > RETURN
        {
            shy.dodge = shy.dodge.toward((0., 0.), now);
            true
        } else {
            false
        }
    })
}

/// `inner` (a [`check::switch`](super::check::switch) with its click added) that dodges the
/// pointer while `on`. `id` names the switch and must be unique. The returned element takes the
/// switch's place in the row and is as big as it.
pub(crate) fn wrap(id: &str, on: bool, inner: impl IntoElement) -> Div {
    let now = Instant::now();
    let still = motion::still();
    let key = id.to_string();
    let (dodge, home) = SHY.with_borrow_mut(|all| {
        let shy = all.entry(key.clone()).or_insert_with(|| Shy {
            dodge: Dodge::home(),
            home: ScrollHandle::new(),
        });
        // Off, or under reduced motion, it goes home (at once when still).
        if (!on || still) && shy.dodge.to != (0., 0.) {
            shy.dodge = if still {
                Dodge::home()
            } else {
                shy.dodge.toward((0., 0.), now)
            };
        }
        (shy.dodge, shy.home.clone())
    });
    let moving = dodge.moving(now);
    let (x, y) = dodge.at(now);
    let shifted = div().relative().flex_none().child(inner);
    let shifted = if moving {
        let (from, to) = (dodge.from, dodge.to);
        shifted
            .with_animation(
                ElementId::named_usize(format!("{key}-shy"), dodge.serial),
                motion::eased(QUICK),
                move |d, t| {
                    d.left(px(from.0 + (to.0 - from.0) * t))
                        .top(px(from.1 + (to.1 - from.1) * t))
                },
            )
            .into_any_element()
    } else {
        shifted.left(px(x)).top(px(y)).into_any_element()
    };
    let owner = Owner::default();
    let mut home_box = div()
        .id(SharedString::from(format!("{key}-home")))
        .relative()
        .flex_none()
        .track_scroll(&home)
        .child(shifted);
    if on && !still {
        let (noted, listen) = (owner.clone(), key);
        home_box = home_box.child(owner.noting()).child(
            canvas(
                |_, _, _| {},
                move |_, _, window: &mut Window, _: &mut App| {
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
