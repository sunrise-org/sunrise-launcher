//! The launcher's motion, as the website's launcher plays it (sunrise-site's `launcher.css` and
//! Tailwind's defaults): hovers ease over the design's quick curve (150 ms,
//! `cubic-bezier(0.4, 0, 0.2, 1)`); toasts and a new page arrive as `hzl-pop` does (a fade and a
//! 6 px rise over 200 ms); a switch's knob slides for 200 ms. Under reduced motion
//! ([`crate::sky::motion`]) nothing moves: everything is drawn where it ends. Menus open at once:
//! an animation inside a `deferred` menu kept its first frame (invisible) under GPUI Fast.
use gpui::{px, Animation, AnimationExt, AnyElement, Div, ElementId, IntoElement, Styled};
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Instant;
use sunrise_launcher_theme as tokens;

/// A new page: as [`POP`], a little longer and from further down, so the switch reads.
#[cfg(test)]
pub const PAGE: tokens::Motion = tokens::Motion {
    name: "page",
    ms: 260,
    curve: [0.2, 0.7, 0.2, 1.],
};

/// A switch's knob (`transition-[translate] duration-200`, Tailwind's default curve).
const SLIDE: tokens::Motion = tokens::Motion {
    name: "slide",
    ms: 200,
    curve: tokens::QUICK.curve,
};

/// Whether the player's system asks for reduced motion (or `SUNRISE_LAUNCHER_MOTION=off`).
pub fn still() -> bool {
    crate::clock::motion() == crate::clock::Motion::Still
}

thread_local! {
    /// The system's power-saver profile is on, as the frame being built saw it.
    static SAVING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Notes whether the system saves power, once a frame, for [`endless`].
pub fn note_power(cx: &gpui::App) {
    SAVING.set(cx.power_mode() == gpui::PowerMode::LowPower);
}

/// The most an endless animation (a shuttle, a pulse) draws a second while the system saves
/// power. The window itself draws at the display's rate even then, so a picture growing or a
/// page arriving stays smooth; only what would otherwise draw for as long as it is shown is
/// held back.
const SAVING_FPS: f32 = 30.;

/// A repeating animation, held to [`SAVING_FPS`] while the system saves power.
pub fn endless(animation: Animation) -> Animation {
    if SAVING.get() {
        animation.with_max_fps(SAVING_FPS)
    } else {
        animation
    }
}

/// `motion` as GPUI plays it: its length on its own curve; at its end from the first frame under
/// reduced motion.
pub fn eased(motion: tokens::Motion) -> Animation {
    eased_as(motion, still())
}

fn eased_as(motion: tokens::Motion, still: bool) -> Animation {
    let animation = Animation::new(motion.duration());
    if still {
        return animation.with_easing(|_| 1.);
    }
    animation.with_easing(move |t| motion.ease(t))
}

/// A hover easing on (to 1) or off (to 0) on its motion's curve, from wherever the last one
/// had got to: a pointer that leaves halfway turns it around there, instead of the element
/// jumping to fully lit first. The rest of the way takes its share of the motion's length.
#[derive(Clone, Copy, Debug)]
pub struct Fade {
    pub on: bool,
    from: f32,
    at: Instant,
    /// The motion it plays on, as it was last drawn with.
    pub motion: tokens::Motion,
}

impl Fade {
    /// Easing towards `on` from where `last` is now; from rest when there was none.
    pub fn toward(last: Option<Fade>, on: bool, now: Instant) -> Fade {
        Fade::toward_as(last, on, now, still())
    }

    /// [`Fade::toward`] with the reduced-motion preference given, not read.
    fn toward_as(last: Option<Fade>, on: bool, now: Instant, still: bool) -> Fade {
        let (from, motion) = match last {
            Some(last) => (last.value_as(now, still), last.motion),
            None => (if on { 0. } else { 1. }, tokens::QUICK),
        };
        Fade {
            on,
            from,
            at: now,
            motion,
        }
    }

    fn target(self) -> f32 {
        if self.on {
            1.
        } else {
            0.
        }
    }

    /// How long it plays: the motion's length, for the share of the way left.
    fn length(self) -> std::time::Duration {
        self.motion
            .duration()
            .mul_f32((self.target() - self.from).abs().min(1.))
    }

    /// The strength at `now`: 0 at rest, 1 lit (more, briefly, on an overshooting curve); `still`
    /// (reduced motion) holds it at its target.
    fn value_as(self, now: Instant, still: bool) -> f32 {
        let length = self.length().as_secs_f32();
        if length <= 0. || still {
            return self.target();
        }
        let t = (now.saturating_duration_since(self.at).as_secs_f32() / length).min(1.);
        self.from + (self.target() - self.from) * self.motion.ease(t)
    }

    /// Ended off: nothing is left to draw.
    pub fn rested(self, now: Instant) -> bool {
        !self.on && now.saturating_duration_since(self.at) >= self.length()
    }

    /// `el` drawn by `paint` at this fade's strength, which GPUI plays from here on. `key`
    /// names the element; each fade plays under its own animation id.
    pub fn play<E: IntoElement + 'static>(
        self,
        key: &gpui::SharedString,
        el: E,
        paint: impl Fn(E, f32) -> E + 'static,
    ) -> AnyElement {
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        let start = *START.get_or_init(Instant::now);
        let id = ElementId::named_usize(
            key.clone(),
            self.at.saturating_duration_since(start).as_micros() as usize,
        );
        let (from, to, motion) = (self.from, self.target(), self.motion);
        let length = self.length().max(std::time::Duration::from_millis(1));
        let animation = if still() {
            Animation::new(length).with_easing(|_| 1.)
        } else {
            Animation::new(length).with_easing(move |t| motion.ease(t))
        };
        el.with_animation(id, animation, move |d, p| paint(d, from + (to - from) * p))
            .into_any_element()
    }
}

thread_local! {
    /// Each switch's last state, and when it last changed.
    static SWITCHES: RefCell<HashMap<String, (bool, Option<Instant>)>> = RefCell::new(HashMap::new());
}

/// When switch `id`'s knob started sliding towards `on`, while it still slides: a switch seen
/// for the first time, or changed long ago, is already there.
fn slide(id: &str, on: bool, now: Instant) -> Option<Instant> {
    SWITCHES.with_borrow_mut(|switches| {
        let seen = switches.entry(id.to_string()).or_insert((on, None));
        if seen.0 != on {
            *seen = (on, Some(now));
        }
        seen.1
            .filter(|since| now.duration_since(*since) < SLIDE.duration())
    })
}

/// While switch `id` slides towards `on`: a number for this slide, to give each of its
/// animations its own id so they play once, together.
pub fn sliding(id: &str, on: bool) -> Option<usize> {
    let since = slide(id, on, Instant::now())?;
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = *START.get_or_init(Instant::now);
    Some(since.saturating_duration_since(start).as_micros() as usize)
}

/// How a switch slides: [`SLIDE`], or at its end at once under reduced motion.
pub fn slide_motion() -> Animation {
    eased(SLIDE)
}

/// A switch's knob at `left` px when off and `left + travel` when on, sliding there when the
/// switch `id` changed in the last 200 ms.
#[cfg_attr(not(windows), allow(dead_code))] // the Windows setup window
pub fn knob(id: &str, on: bool, knob: Div, left: f32, travel: f32) -> AnyElement {
    let at = move |p: f32| left + travel * if on { p } else { 1. - p };
    match slide(id, on, Instant::now()) {
        Some(since) => {
            // Each slide gets its own animation id, from when it started, so it plays once.
            static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
            let start = *START.get_or_init(Instant::now);
            let key = since.saturating_duration_since(start).as_micros() as usize;
            knob.with_animation(
                ElementId::named_usize(format!("{id}-knob"), key),
                eased(SLIDE),
                move |d, p| d.left(px(at(p))),
            )
            .into_any_element()
        }
        None => knob.left(px(at(1.))).into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_hover_left_halfway_turns_around_where_it_got_to() {
        let linear = tokens::Motion {
            name: "linear",
            ms: 100,
            curve: [0., 0., 1., 1.],
        };
        let at = Instant::now();
        let ms = |n: u64| at + std::time::Duration::from_millis(n);
        let on = Fade {
            motion: linear,
            ..Fade::toward_as(None, true, at, false)
        };
        assert!((on.value_as(ms(50), false) - 0.5).abs() < 1e-3);
        assert!(!on.rested(ms(1000)), "a lit element stays lit");
        // The pointer leaves halfway: from 0.5 back to rest, over half the length.
        let off = Fade::toward_as(Some(on), false, ms(50), false);
        assert!((off.value_as(ms(50), false) - 0.5).abs() < 1e-3);
        assert!((off.value_as(ms(75), false) - 0.25).abs() < 1e-3);
        assert!(!off.rested(ms(99)));
        assert!(off.rested(ms(100)));
        // Back on before it rested: from where it was, not from 0.
        let again = Fade::toward_as(Some(off), true, ms(75), false);
        assert!((again.value_as(ms(75), false) - 0.25).abs() < 1e-3);
        assert!((again.value_as(ms(150), false) - 1.).abs() < 1e-3);
    }

    #[test]
    fn reduced_motion_draws_the_end_from_the_first_frame() {
        let still = eased_as(PAGE, true);
        assert_eq!((still.easing)(0.), 1.);
        let moving = eased_as(PAGE, false);
        assert_eq!((moving.easing)(0.), 0.);
        assert!(((moving.easing)(1.) - 1.).abs() < 1e-3);
    }

    #[test]
    fn a_switch_slides_only_after_it_changed() {
        let t0 = Instant::now();
        assert_eq!(slide("test-a", true, t0), None, "first seen: already there");
        assert_eq!(slide("test-a", true, t0 + Duration::from_millis(50)), None);
        let flip = t0 + Duration::from_millis(100);
        assert_eq!(slide("test-a", false, flip), Some(flip));
        assert_eq!(
            slide("test-a", false, flip + Duration::from_millis(150)),
            Some(flip),
            "the same slide while it runs"
        );
        assert_eq!(
            slide("test-a", false, flip + Duration::from_millis(250)),
            None
        );
    }
}
