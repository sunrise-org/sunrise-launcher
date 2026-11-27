//! What the launcher just did, said for 4.2 s (the mockup's `say`): one card in the window's
//! top-right corner, over every page and dialog, drawn by `widgets::feedback::toast_card`, its
//! path or command in mono. It slides in from the edge, lights up and holds its time while the pointer is
//! on it, and slides back out when its time is up or it's clicked.
//!
//! Any handler can say something, even one with only `&mut App`: the line waits here, wakes the
//! launcher's drain (`wake.rs`), which draws it, then lets it go.

use super::*;

use crate::widgets::feedback::Tone;
use std::path::Path;
use std::sync::Mutex;

/// How long a line stays (`setTimeout(…, 4200)`).
const SHOWN: Duration = Duration::from_millis(4200);

/// How long the card takes to slide in, and out again.
const SLIDE: sunrise_launcher_theme::Motion = sunrise_launcher_theme::MOVE;

/// How far the card slides from, past its place.
const SLIDE_PX: f32 = 24.;

/// A line's pieces: the words, and a path or command (`true`) set in mono.
type Parts = Vec<(String, bool)>;

/// The line showing: its pieces, when its 4.2 s started (moved on by the time it was held
/// under the pointer), its number (a new one restarts them), since when it's held, and since
/// when it's sliding out.
struct Line {
    parts: Parts,
    since: Instant,
    n: u64,
    held: Option<Instant>,
    leaving: Option<Instant>,
}

static LINE: Mutex<Option<Line>> = Mutex::new(None);

fn say_parts(parts: Parts) {
    let mut line = LINE.lock().unwrap_or_else(|e| e.into_inner());
    let n = line.as_ref().map_or(1, |l| l.n + 1);
    *line = Some(Line {
        parts,
        since: Instant::now(),
        n,
        held: None,
        leaving: None,
    });
    crate::wake::wake();
}

/// The pointer is on the toast (its time stops) or left it (its time goes on).
fn hold(on: bool) {
    let mut line = LINE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(l) = line.as_mut().filter(|l| l.leaving.is_none()) {
        match (on, l.held) {
            (true, None) => l.held = Some(Instant::now()),
            (false, Some(from)) => {
                l.since += from.elapsed();
                l.held = None;
            }
            _ => {}
        }
    }
}

/// Lets the line go now: a click on the toast. It slides out from where it is.
fn dismiss() {
    let mut line = LINE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(l) = line.as_mut() {
        l.leaving.get_or_insert_with(Instant::now);
        l.held = None;
    }
}

/// Says `text` now, in place of whatever was said before.
pub(super) fn say(text: impl Into<String>) {
    say_parts(vec![(text.into(), false)]);
}

/// Says `en` translated, its one `{hole}` filled with `value` in mono, wherever the language
/// puts it.
pub(super) fn say_with(en: &str, value: &str) {
    say_parts(parts_with(tr(en), value));
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

/// The line still showing, its number, and whether it's sliding out. Its time up, it starts
/// to; once it has, it's gone.
fn current_parts() -> Option<(Parts, u64, bool)> {
    let mut line = LINE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(l) = line.as_mut() {
        if l.leaving.is_none() && l.held.is_none() && l.since.elapsed() >= SHOWN {
            l.leaving = Some(Instant::now());
        }
        if l.leaving.is_some_and(|at| at.elapsed() >= SLIDE.duration()) {
            *line = None;
        }
    }
    line.as_ref()
        .map(|l| (l.parts.clone(), l.n, l.leaving.is_some()))
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
    say_with("Opened {path}", &tilde(path));
}

impl Launcher {
    /// From the drain: whether the line changed (a new one, one sliding out, or the last one
    /// gone).
    pub(super) fn toast_tick(&mut self) -> bool {
        let now = current_parts().map(|(_, n, leaving)| (n, leaving));
        let changed = now != self.toast_seen;
        self.toast_seen = now;
        changed
    }

    /// The line in the window's top-right corner while it is said, over every page and
    /// dialog: under the title bar (`titlebar_h`, none when the system draws it) and the
    /// header, so it never covers the window's buttons, search or the account.
    pub(super) fn toast_float(&self, titlebar_h: f32) -> Option<Div> {
        toast_card(titlebar_h + crate::tokens::FRAME_INSET + 8.)
    }
}

/// The card for the line still showing, none once it is gone: the shared toast card, with the
/// line's pieces inside it, sliding in and out and taking the pointer's click and hold.
fn toast_card(top: f32) -> Option<Div> {
    let (parts, n, leaving) = current_parts()?;
    let p = crate::paint::palette();
    let mut line = div()
        .flex()
        .flex_wrap()
        .items_baseline()
        .gap_x(px(6.))
        .min_w_0();
    for (part, mono) in parts {
        let part = part.trim().to_string();
        line = line.child(if mono {
            div()
                .font_family(crate::theme::MONO)
                .text_color(gpui::rgb(p.accent_ink))
                .child(part)
        } else {
            div().child(part)
        });
    }
    let card = crate::widgets::feedback::toast_card(Tone::Info, line.into_any_element())
        .id("toast")
        .relative()
        .occlude()
        .cursor_pointer()
        .on_hover(|on, window, _| {
            hold(*on);
            window.refresh();
        })
        .on_click(|_, window, _| {
            dismiss();
            window.refresh();
        })
        .hover(move |s| s.bg(gpui::rgb(p.hover)));
    // In from the right edge as it arrives, back out the same way as it goes; under reduced
    // motion it is simply there, then not.
    let card = if leaving {
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
    };
    Some(
        div()
            .absolute()
            .top(px(top))
            .right(px(crate::tokens::FRAME_INSET))
            .left(px(crate::tokens::FRAME_INSET))
            .flex()
            .justify_end()
            .child(card),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_shows_while_a_line_is_said_and_goes_when_it_expires_or_is_clicked() {
        // One test: the line is a process-wide static.
        say("Copied it");
        assert!(toast_card(0.).is_some());
        // Held under the pointer, its time does not run out.
        hold(true);
        LINE.lock().unwrap().as_mut().unwrap().since -= SHOWN * 2;
        assert!(toast_card(0.).is_some());
        hold(false);
        // Released, the 4.2 s are long past: it slides out, then it's gone.
        LINE.lock().unwrap().as_mut().unwrap().since -= SHOWN * 2;
        assert!(toast_card(0.).is_some());
        assert!(current_parts().is_some_and(|(_, _, leaving)| leaving));
        // The pointer can't catch it on the way out.
        hold(true);
        slid_out();
        assert!(toast_card(0.).is_none());
        say("Copied again");
        assert!(toast_card(0.).is_some());
        dismiss();
        assert!(current_parts().is_some_and(|(_, _, leaving)| leaving));
        slid_out();
        assert!(toast_card(0.).is_none());
    }

    /// The slide out is over.
    fn slid_out() {
        let mut line = LINE.lock().unwrap();
        let l = line.as_mut().unwrap();
        *l.leaving.as_mut().unwrap() -= SLIDE.duration() * 2;
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
}
