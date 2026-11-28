//! What the launcher just did, said for 4.2 s each (the mockup's `say`): a column of cards in the
//! window's top-right corner, newest on top, over every page and dialog, each drawn by
//! `widgets::feedback::toast_card`, its path or command in mono. A new card slides in from the
//! edge at the top and the others slide down a place; each keeps its own time, which holds while
//! the pointer is on that card, and slides back out when it is up or the card is clicked, the
//! ones below moving up. Four stay at most; a fifth sends the oldest out. Saying a line that is
//! showing already starts that card over at the top.
//!
//! Any handler can say something, even one with only `&mut App`: the line waits here, wakes the
//! launcher's drain (`wake.rs`), which draws it, then lets it go.

use super::*;

use crate::widgets::feedback::Tone;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

/// How long a line stays (`setTimeout(…, 4200)`).
const SHOWN: Duration = Duration::from_millis(4200);

/// How long the card takes to slide in, and out again.
const SLIDE: sunrise_launcher_theme::Motion = sunrise_launcher_theme::MOVE;

/// How far the card slides from, past its place.
const SLIDE_PX: f32 = 24.;

/// A line's pieces: the words, and a path or command (`true`) set in mono.
type Parts = Vec<(String, bool)>;

/// One line of the stack: its pieces, when its 4.2 s started (moved on by the time it was held
/// under the pointer), its number (it keys the card's animations), since when it's held, and
/// since when it's sliding out.
struct Line {
    parts: Parts,
    /// What the card's copy button copies (the full path it names), if it has one.
    copy: Option<String>,
    since: Instant,
    n: u64,
    held: Option<Instant>,
    leaving: Option<Instant>,
    /// Counts each hold and release, so the countdown bar's animation restarts from where it was.
    segment: usize,
    /// The share of the time left when the bar last started running: its animation goes from
    /// here to empty, so a frame drawn part way through never moves its start.
    segment_left: f32,
    /// The place in the column the card was last drawn at (0 is the top), and the one before,
    /// so it can slide from one to the other when others come or go.
    slot: usize,
    from: usize,
}

/// The lines showing, newest first.
static LINES: Mutex<Vec<Line>> = Mutex::new(Vec::new());

/// The number the next new line gets.
static NEXT: AtomicU64 = AtomicU64::new(1);

/// The most lines that stay at once: a fifth sends the oldest out.
const MOST: usize = 4;

/// A card's height (14 px of padding top and bottom is 12 + 12, plus its one line and border),
/// and the gap between cards. Each card sits in a slot this tall, so places are exact.
const CARD_H: f32 = 44.;
const GAP: f32 = 8.;

fn lines() -> MutexGuard<'static, Vec<Line>> {
    LINES.lock().unwrap_or_else(|e| e.into_inner())
}

fn say_parts(parts: Parts, copy: Option<String>) {
    {
        let mut lines = lines();
        let now = Instant::now();
        if let Some(at) = lines
            .iter()
            .position(|l| l.leaving.is_none() && l.parts == parts)
        {
            // Said again while still showing: the same card starts over, at the top.
            let mut l = lines.remove(at);
            l.copy = copy;
            l.since = now;
            l.held = l.held.map(|_| now);
            l.segment += 1;
            l.segment_left = 1.;
            lines.insert(0, l);
        } else {
            lines.insert(
                0,
                Line {
                    parts,
                    copy,
                    since: now,
                    n: NEXT.fetch_add(1, Ordering::Relaxed),
                    held: None,
                    leaving: None,
                    segment: 0,
                    segment_left: 1.,
                    slot: 0,
                    from: 0,
                },
            );
            // Past the most, the oldest ones start to leave.
            for l in lines.iter_mut().filter(|l| l.leaving.is_none()).skip(MOST) {
                l.leaving = Some(now);
                l.held = None;
            }
        }
    }
    crate::wake::wake();
}

/// The pointer is on toast `n` (its time stops) or left it (its time goes on).
fn hold(n: u64, on: bool) {
    let mut lines = lines();
    if let Some(l) = lines.iter_mut().find(|l| l.n == n && l.leaving.is_none()) {
        match (on, l.held) {
            (true, None) => {
                l.held = Some(Instant::now());
                l.segment += 1;
            }
            (false, Some(from)) => {
                l.since += from.elapsed();
                l.held = None;
                l.segment += 1;
                l.segment_left = share_left(l.since, Instant::now());
            }
            _ => {}
        }
    }
}

/// Lets toast `n` go now: a click on it. It slides out from where it is.
fn dismiss(n: u64) {
    let mut lines = lines();
    if let Some(l) = lines.iter_mut().find(|l| l.n == n) {
        l.leaving.get_or_insert_with(Instant::now);
        l.held = None;
    }
}

/// Says `text` now, on top of whatever was said before.
pub(super) fn say(text: impl Into<String>) {
    say_parts(vec![(text.into(), false)], None);
}

/// Says `en` translated, its one `{hole}` filled with `value` in mono, wherever the language
/// puts it.
pub(super) fn say_with(en: &str, value: &str) {
    say_parts(parts_with(tr(en), value), None);
}

/// [`say_with`], with a copy button on the card that copies `full` (the whole path, where the
/// line shows it shortened).
pub(super) fn say_copyable(en: &str, value: &str, full: String) {
    say_parts(parts_with(tr(en), value), Some(full));
}

/// The longest start of a copy the toast shows, in characters, ellipsis included.
const COPIED_SHOWN: usize = 40;

/// Says what was copied: "Copied" and the start of `text` in mono.
pub(super) fn copied(text: &str) {
    say_with("Copied {text}", &copied_preview(text));
}

/// The start of a copied `text` for the toast: its first line, a path with `~` for home, cut
/// on a character boundary to about 40 characters, with an ellipsis where something was left out.
fn copied_preview(text: &str) -> String {
    let text = text.trim();
    let first = text.lines().next().unwrap_or("").trim_end();
    let more = text.lines().nth(1).is_some();
    let first = if first.starts_with('/') && !first.contains(char::is_whitespace) {
        tilde(Path::new(first))
    } else {
        first.to_string()
    };
    if first.chars().nth(COPIED_SHOWN).is_none() {
        return if more { format!("{first}…") } else { first };
    }
    let end = first
        .char_indices()
        .nth(COPIED_SHOWN - 1)
        .map_or(first.len(), |(i, _)| i);
    format!("{}…", first[..end].trim_end())
}

/// `said`'s one `{hole}` filled with `value` in mono.
fn parts_with(said: &str, value: &str) -> Parts {
    match (said.find('{'), said.find('}')) {
        (Some(a), Some(b)) if a < b => [
            (said[..a].to_string(), false),
            (value.to_string(), true),
            (said[b + 1..].to_string(), false),
        ]
        .into_iter()
        .filter(|(p, _)| !p.trim().is_empty())
        .collect(),
        _ => vec![(said.to_string(), false)],
    }
}

/// A card's countdown bar: the share of its time left, whether the pointer holds it, which
/// hold or release this is, and the share the bar's animation starts from.
type Countdown = (f32, bool, usize, f32);

/// One line as the frame draws it.
struct Card {
    parts: Parts,
    copy: Option<String>,
    n: u64,
    leaving: bool,
    slot: usize,
    from: usize,
    /// None once it slides out.
    countdown: Option<Countdown>,
}

/// The lines still showing, newest first. Each one whose time is up starts to slide out, and
/// each that has slid out is gone.
fn cards() -> Vec<Card> {
    let mut lines = lines();
    let now = Instant::now();
    for l in lines.iter_mut() {
        if l.leaving.is_none() && l.held.is_none() && l.since.elapsed() >= SHOWN {
            l.leaving = Some(now);
        }
    }
    lines.retain(|l| l.leaving.is_none_or(|at| at.elapsed() < SLIDE.duration()));
    lines
        .iter_mut()
        .enumerate()
        .map(|(i, l)| {
            if l.slot != i {
                l.from = l.slot;
                l.slot = i;
                // Moving rebuilds the card, its countdown bar with it: the bar goes on from the
                // time actually left, not from the start of its run.
                if l.held.is_none() {
                    l.segment += 1;
                    l.segment_left = share_left(l.since, now);
                }
            }
            let countdown = l.leaving.is_none().then(|| {
                let left = share_left(l.since, l.held.unwrap_or(now));
                (left, l.held.is_some(), l.segment, l.segment_left)
            });
            Card {
                parts: l.parts.clone(),
                copy: l.copy.clone(),
                n: l.n,
                leaving: l.leaving.is_some(),
                slot: l.slot,
                from: l.from,
                countdown,
            }
        })
        .collect()
}

/// The share of a toast's time left at `now`, for one that started at `since`.
fn share_left(since: Instant, now: Instant) -> f32 {
    let spent = now.saturating_duration_since(since);
    (1. - spent.as_secs_f32() / SHOWN.as_secs_f32()).clamp(0., 1.)
}

/// `/Users/me/x` as `~/x`, as the mockup writes folders.
pub(super) fn tilde(path: &Path) -> String {
    let shown = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && shown.starts_with(&home) => {
            format!("~{}", &shown[home.len()..])
        }
        _ => shown,
    }
}

/// Opens `path` in the system's file manager (or its app), and says so.
pub(super) fn open(cx: &mut App, path: &Path) {
    cx.open_with_system(path);
    say_copyable("Opened {path}", &tilde(path), path.display().to_string());
}

impl Launcher {
    /// From the drain: whether the stack changed (a new line, one sliding out, one gone, or
    /// the order).
    pub(super) fn toast_tick(&mut self) -> bool {
        let now: Vec<(u64, bool)> = cards().iter().map(|c| (c.n, c.leaving)).collect();
        let changed = now != self.toast_seen;
        self.toast_seen = now;
        changed
    }

    /// The column of lines in the window's top-right corner while they are said, over every
    /// page and dialog: under the title bar (`titlebar_h`, none when the system draws it) and
    /// the header, so it never covers the window's buttons, search or the account.
    pub(super) fn toast_float(&self, titlebar_h: f32) -> Option<Div> {
        toast_stack(titlebar_h + crate::tokens::FRAME_INSET + 8.)
    }
}

/// The top of slot `slot` in the column.
fn slot_top(slot: f32) -> f32 {
    slot * (CARD_H + GAP)
}

/// The column of cards, newest on top, none once they are all gone. Each card sits at its
/// slot's place; when its slot changed it slides there from the old one.
fn toast_stack(top: f32) -> Option<Div> {
    let cards = cards();
    if cards.is_empty() {
        return None;
    }
    let height = slot_top(cards.len() as f32) - GAP;
    let mut column = div()
        .absolute()
        .top(px(top))
        .right(px(crate::tokens::FRAME_INSET))
        .w(px(420.))
        .h(px(height));
    for card in cards {
        let (n, slot, from) = (card.n, card.slot, card.from);
        let place = div()
            .absolute()
            .right(px(0.))
            .top(px(slot_top(slot as f32)))
            .child(toast_card(card));
        column = column.child(if from != slot && crate::motion::arrivals() {
            place
                .with_animation(
                    ElementId::named_usize(format!("toast-move-{n}"), slot),
                    crate::motion::eased(SLIDE),
                    move |d, t| {
                        let (a, b) = (slot_top(from as f32), slot_top(slot as f32));
                        d.top(px(a + (b - a) * t))
                    },
                )
                .into_any_element()
        } else {
            place.into_any_element()
        });
    }
    Some(column)
}

/// The card for one line: the shared toast card, with the line's pieces inside it, sliding in
/// and out and taking the pointer's click and hold.
fn toast_card(card: Card) -> impl IntoElement {
    let Card {
        parts,
        copy,
        n,
        leaving,
        countdown,
        ..
    } = card;
    let p = crate::paint::palette();
    // One line, never taller: a part too long for the card is cut off with an ellipsis.
    let mut line = div()
        .flex()
        .items_baseline()
        .gap_x(px(6.))
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap();
    for (part, mono) in parts {
        let part = part.trim().to_string();
        line = line.child(if mono {
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_family(crate::theme::MONO)
                .text_color(gpui::rgb(p.accent_ink))
                .child(part)
        } else {
            div().min_w_0().truncate().child(part)
        });
    }
    // The bar shrinks over the time left, and waits while the pointer holds the card. Under
    // reduced motion there is none: the card is simply there, then not.
    let timer = countdown
        .filter(|_| !crate::motion::still())
        .map(|(left, held, segment, from)| {
            // Running, the bar animates from where this run started; held, it shows the time
            // actually left, which is where that animation had got to.
            let running = (!held).then(|| {
                (
                    ElementId::named_usize(format!("toast-timer-{n}"), segment),
                    SHOWN.mul_f32(from),
                )
            });
            crate::widgets::feedback::toast_timer(
                Tone::Info,
                if held { left } else { from },
                running,
            )
        });
    // The copy button at the right copies what the line names, and keeps the toast up: only a
    // click on the toast itself lets it go.
    let copy = copy.map(|text| crate::widgets::motion_bits::copy_icon("toast-copy", text));
    let content = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .min_w_0()
        .child(line.flex_1())
        .children(copy);
    let card = crate::widgets::feedback::toast_card(Tone::Info, content.into_any_element())
        .children(timer)
        .id(ElementId::named_usize("toast", n as usize))
        .h(px(CARD_H))
        .relative()
        .occlude()
        .cursor_pointer()
        .on_hover(move |on, window, _| {
            hold(n, *on);
            window.refresh();
        })
        .on_click(move |_, window, _| {
            dismiss(n);
            window.refresh();
        })
        .hover(move |s| s.bg(gpui::rgb(p.hover)));
    // In from the right edge as it arrives, back out the same way as it goes; under reduced
    // motion it is simply there, then not.
    if leaving {
        card.with_animation(
            ElementId::named_usize("toast-out", n as usize),
            crate::motion::eased(SLIDE),
            |d, t| d.opacity(1. - t).left(px(SLIDE_PX * t)),
        )
    } else {
        card.with_animation(
            ElementId::named_usize("toast", n as usize),
            crate::motion::eased(SLIDE),
            |d, t| d.opacity(t).left(px(SLIDE_PX * (1. - t))),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lines are a process-wide static, so each test takes this and starts from none.
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

    fn fresh() -> MutexGuard<'static, ()> {
        let guard = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        lines().clear();
        guard
    }

    /// The number of the line that says `text`.
    fn n_of(text: &str) -> u64 {
        let lines = lines();
        let line = lines
            .iter()
            .find(|l| l.parts.first().is_some_and(|(t, _)| t == text))
            .expect("the line was said");
        line.n
    }

    /// Ages the line `n` by `by`, as if it had been showing that much longer.
    fn age(n: u64, by: Duration) {
        let mut lines = lines();
        let line = lines
            .iter_mut()
            .find(|l| l.n == n)
            .expect("the line exists");
        line.since -= by;
    }

    /// The slide out of line `n` is over.
    fn slid_out(n: u64) {
        let mut lines = lines();
        let line = lines
            .iter_mut()
            .find(|l| l.n == n)
            .expect("the line exists");
        *line.leaving.as_mut().expect("it is leaving") -= SLIDE.duration() * 2;
    }

    fn texts() -> Vec<String> {
        cards()
            .into_iter()
            .map(|c| c.parts.into_iter().map(|(t, _)| t).collect())
            .collect()
    }

    #[test]
    fn the_card_shows_while_a_line_is_said_and_goes_when_it_expires_or_is_clicked() {
        let _one = fresh();
        say("Copied it");
        let n = n_of("Copied it");
        assert!(toast_stack(0.).is_some());
        // Held under the pointer, its time does not run out.
        hold(n, true);
        age(n, SHOWN * 2);
        assert!(toast_stack(0.).is_some());
        assert!(cards().iter().all(|c| !c.leaving));
        hold(n, false);
        // Released, the 4.2 s are long past: it slides out, then it's gone.
        age(n, SHOWN * 2);
        assert!(cards().iter().all(|c| c.leaving));
        // The pointer can't catch it on the way out.
        hold(n, true);
        slid_out(n);
        assert!(toast_stack(0.).is_none());
        say("Copied again");
        assert!(toast_stack(0.).is_some());
        let again = n_of("Copied again");
        dismiss(again);
        assert!(cards().iter().all(|c| c.leaving));
        slid_out(again);
        assert!(toast_stack(0.).is_none());
    }

    #[test]
    fn two_lines_show_together_newest_first() {
        let _one = fresh();
        say("first");
        say("second");
        assert_eq!(texts(), ["second", "first"]);
        let showing = cards();
        assert_eq!([showing[0].slot, showing[1].slot], [0, 1]);
        // The older one moved down from the top.
        assert_eq!(showing[1].from, 0);
    }

    #[test]
    fn a_new_line_does_not_restart_the_older_ones_time() {
        let _one = fresh();
        say("first");
        cards();
        let n = n_of("first");
        age(n, SHOWN / 2);
        say("second");
        let first = cards().into_iter().find(|c| c.n == n).expect("still shown");
        let (left, _, _, from) = first.countdown.expect("counting down");
        assert!((0.4..0.6).contains(&left), "half its time is left: {left}");
        // The bar, rebuilt in its new place, goes on from there, not from full.
        assert!((from - left).abs() < 0.05, "the bar starts at {from}");
    }

    #[test]
    fn the_same_line_said_again_starts_over_at_the_top() {
        let _one = fresh();
        say("a");
        say("b");
        let a = n_of("a");
        age(a, Duration::from_millis(3000));
        say("a");
        assert_eq!(texts(), ["a", "b"]);
        assert_eq!(n_of("a"), a);
        // Its 4.2 s start over.
        assert!(lines()[0].since.elapsed() < Duration::from_millis(1000));
    }

    #[test]
    fn one_line_expiring_leaves_the_other_showing() {
        let _one = fresh();
        say("old");
        say("new");
        let old = n_of("old");
        age(old, SHOWN * 2);
        let showing = cards();
        assert!(showing[1].leaving && !showing[0].leaving);
        slid_out(old);
        assert_eq!(texts(), ["new"]);
    }

    #[test]
    fn holding_one_line_does_not_hold_the_other() {
        let _one = fresh();
        say("held");
        say("free");
        let (held, free) = (n_of("held"), n_of("free"));
        hold(held, true);
        age(held, SHOWN * 2);
        age(free, SHOWN * 2);
        let showing = cards();
        assert!(!showing[1].leaving, "the held one stays");
        assert!(showing[0].leaving, "the other runs out");
    }

    #[test]
    fn a_fifth_line_sends_the_oldest_out() {
        let _one = fresh();
        for text in ["1", "2", "3", "4"] {
            say(text);
        }
        assert!(cards().iter().all(|c| !c.leaving));
        say("5");
        let showing = cards();
        assert_eq!(showing.len(), 5);
        assert!(showing[4].leaving);
        assert!(showing[..4].iter().all(|c| !c.leaving));
        slid_out(n_of("1"));
        assert_eq!(texts(), ["5", "4", "3", "2"]);
    }

    #[test]
    fn the_hole_is_set_in_mono_between_the_words() {
        assert_eq!(
            parts_with("Copied {text}", "AB12-CD"),
            vec![
                ("Copied ".to_string(), false),
                ("AB12-CD".to_string(), true)
            ]
        );
        assert_eq!(
            parts_with("Opened {path} now", "~/x"),
            vec![
                ("Opened ".to_string(), false),
                ("~/x".to_string(), true),
                (" now".to_string(), false)
            ]
        );
    }

    #[test]
    fn a_copy_shows_its_first_line_cut_to_forty_characters() {
        assert_eq!(copied_preview("AB12-CD\n"), "AB12-CD");
        assert_eq!(
            copied_preview(&"x".repeat(200)),
            format!("{}…", "x".repeat(39))
        );
        assert_eq!(copied_preview(&"é".repeat(40)), "é".repeat(40));
        assert_eq!(
            copied_preview(&"é".repeat(41)),
            format!("{}…", "é".repeat(39))
        );
        assert_eq!(copied_preview("error: one\n  two\nthree\n"), "error: one…");
        assert_eq!(copied_preview("\n\nerror\n"), "error");
        assert_eq!(copied_preview(""), "");
        assert_eq!(copied_preview("/srv/a b/c"), "/srv/a b/c");
    }
}
