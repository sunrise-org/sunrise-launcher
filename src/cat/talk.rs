//! What the cat says when it is clicked, and how long the speech bubble over it lasts. A click
//! gets a random line, never the one before. Clicked again within `WINDOW` it escalates, line by
//! line through one of a few sequences, until it leaves; a pause longer than that starts it over. The state is plain data, so
//! the choice of line is tested without a window.
use crate::i18n::tr_key;

/// The lines it says at a first click, picked at random.
pub(super) const LINES: [&str; 10] = [
    tr_key("Go annoy another cat, fatzo."),
    tr_key("Touch me again and I'm telling the mods."),
    tr_key("Have you tried turning your profile off and on again?"),
    tr_key("I've crashed more times than your game. On purpose."),
    tr_key("Bold of you to assume I'm a decoration."),
    tr_key("I'm not lost. The world is."),
    tr_key("Wow. Clicking a cat. Peak gameplay."),
    tr_key("Don't you have a game to lose?"),
    tr_key("My nine lives, and you waste one of them."),
    tr_key("Go build a dirt house or something."),
];

/// What it says to clicks that come in a row, in four ways of the same arc (mild, annoyed,
/// warning, gone). One is picked at random when the pestering starts; the last line of each
/// sends the cat away.
pub(super) const ESCALATIONS: [[&str; 4]; 4] = [
    [
        tr_key("…what."),
        tr_key("Stop that."),
        tr_key("I'm warning you."),
        tr_key("That's it, I'm leaving."),
    ],
    [
        tr_key("Hm?"),
        tr_key("Seriously?"),
        tr_key("Last warning, human."),
        tr_key("Fine. I'm out."),
    ],
    [
        tr_key("Oh, you again."),
        tr_key("Cats are not touchscreens."),
        tr_key("One more poke and I file a report."),
        tr_key("Goodbye, menace."),
    ],
    [
        tr_key("Is this your whole hobby?"),
        tr_key("I'm trying to nap here."),
        tr_key("Touch me again, I dare you."),
        tr_key("Enough. I'm taking my nap elsewhere."),
    ],
];

/// A click this soon after the one before it counts as the same pestering, in ms.
pub(super) const WINDOW: f32 = 4000.;
const FADE_IN: f32 = 180.;
const FADE_OUT: f32 = 300.;

/// How long a bubble of a line `chars` long lives in ms, fades included: a base and a little
/// per character so a longer line can be read, but never more than the cap.
pub(super) fn lasts(chars: usize) -> f32 {
    (3500. + 60. * chars as f32).min(7000.)
}

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub(super) struct Talk {
    last_click: Option<f32>,
    /// How many lines of the escalation it has said.
    stage: usize,
    /// Which of the escalations it is saying, once the pestering has started.
    sequence: usize,
    last_line: Option<usize>,
}

/// One answer to a click: the line (an English key, for `tr`) and whether the cat leaves after it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct Said {
    pub(super) line: &'static str,
    pub(super) leaves: bool,
}

/// The cat's answer to a click at `now` (ms), and the state after it. `roll` is a number in
/// `0..1` that picks the random line, or which escalation starts.
pub(super) fn next_line(talk: Talk, now: f32, roll: f32) -> (Said, Talk) {
    let pestered = talk.last_click.is_some_and(|at| now - at <= WINDOW);
    if pestered {
        // The first quick click picks the escalation; the rest follow it.
        let sequence = if talk.stage == 0 {
            let k = (roll.clamp(0., 0.999_999) * ESCALATIONS.len() as f32) as usize;
            k.min(ESCALATIONS.len() - 1)
        } else {
            talk.sequence
        };
        let steps = &ESCALATIONS[sequence];
        let line = steps[talk.stage.min(steps.len() - 1)];
        let leaves = talk.stage + 1 >= steps.len();
        let next = if leaves {
            // It is gone; whatever comes back starts from nothing.
            Talk::default()
        } else {
            Talk {
                last_click: Some(now),
                stage: talk.stage + 1,
                sequence,
                last_line: talk.last_line,
            }
        };
        return (Said { line, leaves }, next);
    }
    // One of the others than the last, spread evenly over them.
    let count = LINES.len() - usize::from(talk.last_line.is_some());
    let k = ((roll.clamp(0., 0.999_999) * count as f32) as usize).min(count - 1);
    let pick = match talk.last_line {
        Some(last) if k >= last => k + 1,
        _ => k,
    };
    let said = Said {
        line: LINES[pick],
        leaves: false,
    };
    let next = Talk {
        last_click: Some(now),
        stage: 0,
        sequence: 0,
        last_line: Some(pick),
    };
    (said, next)
}

/// The bubble's opacity `t` ms after it began, when it lives `lasts` ms, or none once it is
/// over. Under reduced motion it is simply there and then gone.
pub(super) fn bubble_alpha(t: f32, lasts: f32, still: bool) -> Option<f32> {
    if !(0. ..lasts).contains(&t) {
        return None;
    }
    if still {
        return Some(1.);
    }
    Some((t / FADE_IN).min((lasts - t) / FADE_OUT).clamp(0., 1.))
}

/// A speech bubble: the line it shows, when it began on the cat's clock, and how long it lives.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct Bubble {
    pub(super) line: &'static str,
    pub(super) since: f32,
    pub(super) lasts: f32,
}
