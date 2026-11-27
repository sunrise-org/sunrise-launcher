//! Feedback: the banner (warning, info, danger, success), the progress bar, the empty state and the toast card.
#![allow(dead_code)] // Used by the shell and pages that the next redesign steps move over; removed in the final sweep.
use super::*;

use super::focus::{c, mix};
use crate::glass_ui::icon;
use crate::motion;
use crate::paint::{palette, Palette};
use crate::tokens::{r, BODY, HEADING, LABEL, R_CARD, R_CONTROL};
use gpui::{Animation, AnimationExt, FontWeight, Hsla};
use std::time::Duration;

/// What the feedback says, and so its colour and icon.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Tone {
    Warning,
    Info,
    Danger,
    Success,
}

impl Tone {
    /// The tone's colour role in the palette.
    fn colour(self, p: &Palette) -> u32 {
        match self {
            Tone::Warning => p.warning,
            Tone::Info => p.info,
            Tone::Danger => p.danger,
            Tone::Success => p.success,
        }
    }

    /// The tone's icon (a file under `crates/theme/icons/glass/`).
    fn icon(self) -> &'static str {
        match self {
            Tone::Warning | Tone::Danger => "alert-02",
            Tone::Info => "information-circle",
            Tone::Success => "tick-02",
        }
    }
}

/// One pass of the indeterminate bar's shuttle.
const SHUTTLE: Duration = Duration::from_millis(1600);

/// A bar's fill never drops below a sliver, so the start of a job still shows.
fn clamp_fraction(fraction: f32) -> f32 {
    fraction.clamp(0.02, 1.)
}

/// An empty state's line without its trailing full stop (`。`, `।` too): the line is set as a
/// label, not a sentence.
fn label_line(line: SharedString) -> SharedString {
    SharedString::from(line.trim_end_matches(['.', '。', '।']).to_owned())
}

/// A notice strip: a tone icon, a title in the tone's colour and an optional body line.
/// Its fill is the tone at 15 % over the card colour; its edge the tone at 50 %.
pub(crate) fn banner(
    tone: Tone,
    title: impl Into<SharedString>,
    body: Option<SharedString>,
) -> Div {
    let p = palette();
    let ink: Hsla = c(tone.colour(&p));
    let text = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(
            div()
                .text_size(px(BODY))
                .font_weight(FontWeight::MEDIUM)
                .text_color(ink)
                .child(title.into()),
        )
        .children(body.map(|b| div().text_size(px(LABEL)).text_color(c(p.text)).child(b)));
    div()
        .w_full()
        .flex()
        .items_start()
        .gap(px(12.))
        .px(px(16.))
        .py(px(12.))
        .rounded(r(R_CONTROL))
        .border_1()
        .border_color(ink.opacity(0.5))
        .bg(mix(c(p.card_color), ink, 0.15))
        .child(icon(tone.icon(), 20., ink))
        .child(text)
}

/// A thin track with a fill: `fraction` of the way (`None`: no known end, so a 30 % segment
/// shuttles along it, or sits still under reduced motion).
pub(crate) fn progress_bar(fraction: Option<f32>) -> Div {
    let p = palette();
    let track = div()
        .relative()
        .flex_none()
        .w_full()
        .h(px(6.))
        .rounded_full()
        .bg(c(p.divider))
        .overflow_hidden();
    let bar = div()
        .absolute()
        .top_0()
        .bottom_0()
        .rounded_full()
        .bg(c(p.accent));
    match fraction {
        Some(f) => track.child(bar.left_0().w(relative(clamp_fraction(f)))),
        // Reduced motion holds the segment still where it can be seen (GPUI freezes a repeating
        // animation at its start, off the track).
        None if motion::still() => track.child(bar.left(relative(0.35)).w(relative(0.3))),
        None => track.child(
            bar.w(relative(0.3)).with_animation(
                "feedback-shuttle",
                motion::endless(
                    Animation::new(SHUTTLE)
                        .repeat()
                        .with_easing(gpui::ease_in_out),
                ),
                |d, t| d.left(relative(-0.3 + 1.3 * t)),
            ),
        ),
    }
}

/// A progress line: `label` on the left, `value` (a figure or an ETA) on the right, the bar
/// under them both.
pub(crate) fn progress(
    label: impl Into<SharedString>,
    value: Option<SharedString>,
    fraction: Option<f32>,
) -> Div {
    let p = palette();
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .text_size(px(LABEL))
                .child(div().min_w_0().text_color(c(p.text)).child(label.into()))
                .children(value.map(|v| div().flex_none().text_color(c(p.muted)).child(v))),
        )
        .child(progress_bar(fraction))
}

/// A page or list with nothing in it: a title, a line of why and what to do, and an optional
/// action under them, centred in the space it's given.
pub(crate) fn empty_state(
    title: impl Into<SharedString>,
    body: impl Into<SharedString>,
    action: Option<AnyElement>,
) -> Div {
    let p = palette();
    div()
        .flex_1()
        .min_h(px(240.))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .px(px(24.))
        .py(px(32.))
        .child(
            div()
                .text_size(px(HEADING))
                .font_weight(FontWeight::MEDIUM)
                .text_color(c(p.text))
                .child(title.into()),
        )
        .child(
            div()
                .max_w(px(380.))
                .text_center()
                .text_size(px(BODY))
                .text_color(c(p.muted))
                .child(label_line(body.into())),
        )
        .children(action.map(|a| div().mt(px(4.)).child(a)))
}

/// The card a toast draws its line on: 320 to 420 px wide, a tone icon, the content beside it.
/// The content may wrap, and a path with no spaces breaks wherever the row runs out, so nothing
/// leaves the card. The toast owns its timing and slide; this is only the look.
pub(crate) fn toast_card(tone: Tone, content: AnyElement) -> Div {
    let p = palette();
    let ink: Hsla = c(tone.colour(&p));
    div()
        .min_w(px(320.))
        .max_w(px(420.))
        .px(px(14.))
        .py(px(12.))
        .flex()
        .items_start()
        .gap(px(10.))
        .rounded(r(R_CARD))
        .bg(c(p.raised))
        .border_1()
        .border_color(c(p.line))
        .shadow_lg()
        .text_color(c(p.text))
        .text_size(px(LABEL))
        .child(icon(tone.icon(), 18., ink))
        .child(div().flex_1().min_w_0().child(content))
}

#[cfg(test)]
mod tests;
