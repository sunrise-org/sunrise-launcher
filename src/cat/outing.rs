//! The play bar's scene: where the cat goes, how it gets there, and whether it may come out.
use super::talk::{lasts, next_line, Bubble, Talk};
use super::*;

/// The bar is this wide before the cat has room beside Play, and the window this tall.
pub(super) const ROOM: (f32, f32) = (1000., 560.);
/// Screen pixels per pixel of the cat, at a scale factor of 1.
pub(super) const SIZE: f32 = 2.;
/// How far the picture reaches under the cat's feet (its shadow's rows), in px: what stands it
/// on an edge is this much below the picture's bottom.
pub(crate) const BELOW_FEET: f32 = 3. * SIZE;
pub(super) const WIDTH: f32 = W as f32 * SIZE;
/// A cat acting in place is drawn 12 times a second, a cat on the move 25 times.
const FRAME: Duration = Duration::from_millis(83);
const STEP_FRAME: Duration = Duration::from_millis(40);
/// The cat's lane runs this far under Play's edge, so the cat comes out from behind the button.
pub(crate) const UNDER: f32 = 8.;
/// A peek shows the head and chest, and lasts this long.
pub(super) const PEEK_OUT: f32 = 44.;
const PEEK: f32 = 4200.;
/// Walking: pixels a millisecond, and the leg swing that keeps the paws from sliding at that pace.
const SPEED: f32 = 0.055;
const STRIDE: f32 = 0.012;
/// Sleeping on Play: the climb up from behind the button, curling up, and uncurling again, in
/// ms. A cat that has to leave at once drops back behind the button in `DROP`.
pub(super) const CLIMB: f32 = 800.;
pub(super) const CURL: f32 = 1300.;
pub(super) const DROP: f32 = 260.;

/// Where the cat is. Mostly it is away, behind Play; now and then it peeks out, or walks out
/// into the empty part of the bar, stays a while and walks back.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Scene {
    Away {
        until: f32,
    },
    Peek {
        since: f32,
    },
    /// `from` and `to` are how far out of hiding the cat is, in pixels.
    Walk {
        from: f32,
        to: f32,
        since: f32,
    },
    Visit {
        at: f32,
        until: f32,
    },
    /// Asleep on top of Play until `until`, then it uncurls and climbs back down.
    Perch {
        since: f32,
        until: f32,
    },
    /// Off Play at once, from the `pose` and height (`up`) it had.
    Drop {
        since: f32,
        pose: Pose,
        up: f32,
    },
}

/// What to draw of the cat: how far out it is, its pose, and whether it faces left.
pub(super) struct View {
    pub(super) out: f32,
    pub(super) pose: Pose,
    pub(super) left: bool,
    pub(super) pettable: bool,
    /// On Play instead of in the bar: how far it has climbed over the button's top edge, 0 to 1.
    pub(super) perch: Option<f32>,
}

pub(super) struct Outing {
    pub(super) cat: Cat,
    pub(super) scene: Scene,
    /// The first time out is a visit, so the cat introduces itself properly.
    been_out: bool,
    /// Its next time out is a sleep on Play, when Play is free.
    pub(super) perch_next: bool,
    /// How the cat answers clicks, and the bubble it is saying one in.
    talk: Talk,
    pub(super) bubble: Option<Bubble>,
    /// It is leaving for good (the last of its lines): how long it stays away, in ms.
    away_for: Option<[f32; 2]>,
}

fn ease(k: f32) -> f32 {
    let k = unit(k);
    k * k * (3. - 2. * k)
}

impl Outing {
    pub(super) fn new(ways: &'static Ways, seed: u64, first: f32) -> Self {
        Self {
            cat: Cat::new(ways, seed),
            scene: Scene::Away { until: first },
            been_out: false,
            perch_next: false,
            talk: Talk::default(),
            bubble: None,
            away_for: None,
        }
    }

    /// The cat on Play at `now`: its pose and how far up it is.
    fn perched(since: f32, until: f32, now: f32) -> (Pose, f32) {
        let (t, left) = (now - since, now - until);
        // Curled up once it is up, and uncurled again before it climbs down.
        let curl = ease((t - CLIMB) / CURL) * ease(-left / CURL);
        let (sit, sleep) = (with(SIT, &[(TURN, 0.35)]), rest(State::Sleep));
        let mut pose = sit;
        for (value, asleep) in pose.iter_mut().zip(sleep) {
            *value = lerp(*value, asleep, curl);
        }
        // Eyes are open or shut, never half.
        pose[EYES] = if curl > 0.6 { 0. } else { 1. };
        (pose, ease(t / CLIMB) * ease((CURL + CLIMB - left) / CLIMB))
    }

    /// Whatever it was doing on Play, it gets off now.
    pub(super) fn leave_play(&mut self, now: f32) {
        if let Scene::Perch { since, until } = self.scene {
            let (pose, up) = Self::perched(since, until, now);
            self.scene = Scene::Drop {
                since: now,
                pose: with(pose, &[(EYES, 1.), (EAR_NEAR, 0.), (EAR_FAR, 0.)]),
                up,
            };
        }
    }

    pub(super) fn hide(&mut self, now: f32) {
        let away = self.away_for.take().unwrap_or([60000., 200000.]);
        self.bubble = None;
        self.scene = Scene::Away {
            until: now + self.cat.random.span(away),
        };
    }

    /// A click on the cat at `now`: it stops where it is, sits and says a line. The last line of
    /// the escalation sends it home, and it stays away a good while.
    pub(super) fn speak(&mut self, now: f32) {
        if self.away_for.is_some() {
            return;
        }
        let at = match self.scene {
            Scene::Walk { from, to, since } => {
                let gone = ((now - since) * SPEED).min((to - from).abs());
                from + gone * (to - from).signum()
            }
            Scene::Visit { at, .. } => at,
            _ => return,
        };
        let roll = self.cat.random.next();
        let (said, talk) = next_line(self.talk, now, roll);
        self.talk = talk;
        let lasts = lasts(crate::i18n::tr(said.line).chars().count());
        self.bubble = Some(Bubble {
            line: said.line,
            since: now,
            lasts,
        });
        self.cat.state = State::Sit;
        self.cat.playing = None;
        self.cat.pet(now);
        self.cat.next_at = now + lasts + 400.;
        let until = if said.leaves {
            self.away_for = Some([20000., 40000.]);
            now + 1500.
        } else {
            now + lasts + 1000.
        };
        let until = match self.scene {
            Scene::Visit { until: old, .. } if !said.leaves => old.max(until),
            _ => until,
        };
        self.scene = Scene::Visit {
            at: (at / SIZE).round() * SIZE,
            until,
        };
    }

    /// Clears a bubble that has run its time.
    fn expire_bubble(&mut self, now: f32) {
        if self.bubble.is_some_and(|b| now - b.since >= b.lasts) {
            self.bubble = None;
        }
    }

    /// Moves the scene on to `now` (ms). `room` is the empty width beside Play, in pixels: the
    /// cat never goes further out than that, and stays hidden when there is none. `free` says
    /// Play is idle and the bar has nothing to say, so the cat may sleep on the button.
    pub(super) fn step(&mut self, now: f32, room: f32, free: bool) {
        self.expire_bubble(now);
        match self.scene {
            Scene::Perch { .. } if !free => self.leave_play(now),
            Scene::Perch { until, .. } if now >= until + CURL + CLIMB => self.hide(now),
            Scene::Drop { since, .. } if now - since >= DROP => self.hide(now),
            Scene::Away { until } if now >= until => {
                if free && (self.perch_next || self.been_out && self.cat.random.next() < 0.25) {
                    self.perch_next = false;
                    self.scene = Scene::Perch {
                        since: now,
                        until: now + self.cat.random.span([45000., 150000.]),
                    };
                } else if room < PEEK_OUT {
                    self.scene = Scene::Away { until: now + 5000. };
                } else if room >= WIDTH * 1.5 && (!self.been_out || self.cat.random.next() < 0.7) {
                    let far = room.min(WIDTH + 16. + 520.);
                    let to = self.cat.random.span([WIDTH + 16., far]);
                    self.scene = Scene::Walk {
                        from: 0.,
                        to: (to / SIZE).round() * SIZE,
                        since: now,
                    };
                    self.been_out = true;
                } else {
                    self.scene = Scene::Peek { since: now };
                }
            }
            Scene::Peek { since } if now - since >= PEEK => self.hide(now),
            Scene::Walk { from, to, since } => {
                let gone = (now - since) * SPEED;
                if gone >= (to - from).abs() {
                    if to <= 0. {
                        self.hide(now);
                    } else {
                        // It sits down fresh, and looks around before doing anything else.
                        self.cat.state = State::Sit;
                        self.cat.playing = None;
                        self.cat.next_at = now + self.cat.random.span([900., 2200.]);
                        self.scene = Scene::Visit {
                            at: to,
                            until: now + self.cat.random.span([20000., 60000.]),
                        };
                    }
                } else if to > from && from + gone > room {
                    // The room shrank ahead of it: it turns round.
                    self.scene = Scene::Walk {
                        from: from + gone,
                        to: 0.,
                        since: now,
                    };
                }
            }
            Scene::Visit { at, until } => {
                self.cat.advance(now);
                // It goes home when its time is up, but never mid-action and never asleep.
                if (now >= until || at > room)
                    && self.cat.playing.is_none()
                    && self.cat.state == State::Sit
                {
                    self.scene = Scene::Walk {
                        from: at,
                        to: 0.,
                        since: now,
                    };
                }
            }
            _ => {}
        }
    }

    pub(super) fn view(&self, now: f32) -> Option<View> {
        match self.scene {
            Scene::Away { .. } => None,
            Scene::Perch { since, until } => {
                let (pose, up) = Self::perched(since, until, now);
                Some(View {
                    out: 0.,
                    pose,
                    left: true,
                    pettable: false,
                    perch: Some(up),
                })
            }
            Scene::Drop { since, pose, up } => Some(View {
                out: 0.,
                pose,
                left: true,
                pettable: false,
                perch: Some(up * (1. - ease((now - since) / DROP))),
            }),
            Scene::Peek { since } => {
                let t = now - since;
                // Out, a look at you with a blink and a tilt of the head, and back in.
                let look = ease((t - 900.) / 300.) * ease((PEEK - 900. - t) / 300.);
                let tilt = ease((t - 2500.) / 250.) * ease((3400. - t) / 250.);
                let blink = (1900. ..2050.).contains(&t);
                Some(View {
                    out: PEEK_OUT * ease(t / 700.) * ease((PEEK - t) / 600.),
                    pose: with(
                        SIT,
                        &[
                            (TURN, look),
                            (TILT, -0.3 * tilt),
                            (EYES, if blink { 0. } else { 1. }),
                        ],
                    ),
                    left: true,
                    pettable: false,
                    perch: None,
                })
            }
            Scene::Walk { from, to, since } => {
                let t = now - since;
                let gone = (t * SPEED).min((to - from).abs());
                Some(View {
                    out: from + gone * (to - from).signum(),
                    pose: with(
                        SIT,
                        &[(WALK, t * STRIDE), (TAIL_SWAY, (t * STRIDE * 0.5).sin())],
                    ),
                    left: to > from,
                    pettable: self.away_for.is_none(),
                    perch: None,
                })
            }
            Scene::Visit { at, .. } => Some(View {
                out: at,
                pose: self.cat.pose(now),
                left: true,
                pettable: self.away_for.is_none(),
                perch: None,
            }),
        }
    }

    /// How long until the cat next needs a look.
    pub(super) fn next_change(&self, now: f32) -> Duration {
        let until = |at: f32| Duration::from_secs_f32((at - now).max(0.) / 1000.);
        let change = match self.scene {
            Scene::Away { until: at } => until(at),
            Scene::Peek { .. } | Scene::Walk { .. } | Scene::Drop { .. } => STEP_FRAME,
            // Asleep it is one still picture: the next look is when it wakes.
            Scene::Perch { since, until } => {
                if now - since < CLIMB + CURL || now >= until - CURL {
                    STEP_FRAME
                } else {
                    self::until_ms(until - CURL - now)
                }
            }
            Scene::Visit { until: at, .. } => self
                .cat
                .next_change()
                .map_or(FRAME, |change| until(change.min(at.max(now + 500.)))),
        };
        // A bubble fading in or out wants a frame at the walking pace.
        if self.bubble.is_some() {
            change.min(STEP_FRAME)
        } else {
            change
        }
    }
}

fn until_ms(ms: f32) -> Duration {
    Duration::from_secs_f32(ms.max(0.) / 1000.)
}

#[derive(Default)]
pub(crate) struct CatState {
    pub(super) cat: Option<Outing>,
    /// The cat's own time, in ms: it stands still while the window is in the background.
    pub(super) clock: f32,
    pub(super) seen: Option<Instant>,
    /// The width of the cat's lane as last laid out.
    pub(super) lane: std::rc::Rc<std::cell::Cell<f32>>,
    /// The picture on screen, and the pose, scale and facing it was drawn for.
    pub(super) shown: Option<(Pose, usize, bool, Arc<RenderImage>)>,
    /// When the pending wake fires: one timer at a time.
    pub(super) wake: Option<Instant>,
}

/// Whether the cat may come out at all: it is switched on, and motion is not reduced. Its
/// visits are motion and nothing else, so with less motion asked for there are none.
pub(super) fn cat_out(shown: bool, still: bool) -> bool {
    shown && !still
}

impl CatState {
    /// The cat is switched off: nothing of it is kept, no timer is due, and the picture it last
    /// drew is handed back to be dropped.
    pub(super) fn switch_off(&mut self) -> Option<Arc<RenderImage>> {
        self.cat = None;
        self.seen = None;
        self.wake = None;
        self.shown.take().map(|(_, _, _, image)| image)
    }
}
