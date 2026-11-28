//! The lines of the log, in a card: a gutter of line numbers, a band behind each warning and
//! error, the level of a line marked in its colour, the words searched for lit, and a pill that
//! jumps back to the newest line once Follow is off and the view was scrolled up. Only the lines
//! near the view are built.
use super::*;
use crate::widgets::focus::c;
use std::ops::Range;

/// A line of the log. Fixed, so a long log builds only the lines near the view.
pub(super) const LINE: f32 = 20.;

/// The room for line numbers, with a little air before the text.
const GUTTER: f32 = 56.;

/// The space above the first line and under the last.
const PAD: f32 = 8.;

/// How bad a log line is: 0 for information, 1 for a warning, 2 for an error.
pub(super) fn severity(line: &str) -> u8 {
    if line.contains("/ERROR]") || line.contains("ERROR:") || line.contains(" ERROR ") {
        2
    } else if line.contains("/WARN]") || line.contains(" WARN ") || line.contains("WARN:") {
        1
    } else {
        0
    }
}

/// The part of `line` that names its level ("[Render thread/ERROR]", "ERROR:"), when it has one.
fn level_mark(line: &str, severity: u8) -> Option<Range<usize>> {
    let marks: &[&str] = match severity {
        2 => &["/ERROR]", "ERROR:", " ERROR "],
        1 => &["/WARN]", "WARN:", " WARN "],
        _ => return None,
    };
    marks.iter().find_map(|mark| {
        let at = line.find(mark)?;
        // "[thread/LEVEL]" is marked from its bracket.
        let start = match mark.starts_with('/') {
            true => line[..at].rfind('[').unwrap_or(at),
            false => at,
        };
        Some(start..at + mark.len())
    })
}

/// Where each of `words` (lower case, ASCII case ignored) is in `line`.
fn found(line: &str, words: &[String]) -> Vec<Range<usize>> {
    let low = line.to_ascii_lowercase();
    let mut hits = Vec::new();
    for word in words {
        let mut from = 0;
        while let Some(at) = low[from..].find(word.as_str()) {
            let start = from + at;
            hits.push(start..start + word.len());
            from = start + word.len();
        }
    }
    hits
}

/// The highlights of one `line` (tabs already spaces), `base` bytes into the block of text: its
/// level in the level's colour, and the words searched for on a tint of the accent. A hit that
/// overlaps an earlier one is left out, so the runs never cross.
fn line_runs(
    line: &str,
    base: usize,
    words: &[String],
) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
    let p = palette();
    let level = severity(line);
    let mut runs: Vec<(Range<usize>, gpui::HighlightStyle)> = Vec::new();
    if let Some(mark) = level_mark(line, level) {
        let ink = if level == 2 { p.danger } else { p.warning };
        runs.push((
            mark,
            gpui::HighlightStyle {
                color: Some(c(ink)),
                font_weight: Some(gpui::FontWeight::SEMIBOLD),
                ..Default::default()
            },
        ));
    }
    for hit in found(line, words) {
        runs.push((
            hit,
            gpui::HighlightStyle {
                background_color: Some(c(p.accent).opacity(0.3)),
                ..Default::default()
            },
        ));
    }
    runs.sort_by_key(|(range, _)| range.start);
    let mut end = 0;
    runs.retain(|(range, _)| {
        let keep = range.start >= end;
        if keep {
            end = range.end;
        }
        keep
    });
    runs.into_iter()
        .map(|(range, style)| (range.start + base..range.end + base, style))
        .collect()
}

/// The lines drawn now (each with its place in the log): their numbers, a band behind the warnings
/// and errors, and the text as one block of selectable text, so a drag selects across lines and
/// copies as any text does. Tabs (stack traces) become spaces; a line too long for the box is cut,
/// not wrapped, so every line keeps its `LINE` height.
fn log_lines(lines: &[(usize, &str)], search: &str) -> Div {
    let p = palette();
    let words: Vec<String> = search
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect();
    let mut text = String::new();
    let mut runs = Vec::new();
    let mut bands = Vec::new();
    for (row, (_, line)) in lines.iter().enumerate() {
        if row > 0 {
            text.push('\n');
        }
        let clean = line.replace('\t', "    ");
        runs.extend(line_runs(&clean, text.len(), &words));
        text.push_str(&clean);
        let tint = match severity(&clean) {
            2 => Some(c(p.danger).opacity(0.13)),
            1 => Some(c(p.warning).opacity(0.11)),
            _ => None,
        };
        bands.extend(tint.map(|tint| {
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(row as f32 * LINE))
                .h(px(LINE))
                .bg(tint)
        }));
    }
    let numbers = div()
        .flex_none()
        .w(px(GUTTER))
        .pr(px(14.))
        .flex()
        .flex_col()
        .items_end()
        .text_color(c(p.muted).opacity(0.7))
        .children(
            lines
                .iter()
                .map(|(n, _)| div().h(px(LINE)).child((n + 1).to_string())),
        );
    let styled = gpui::StyledText::new(text).with_highlights(runs);
    let text = div()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .line_height(px(LINE))
        .text_color(c(p.text))
        .child(
            crate::selectable::Selectable::styled("home-logs-text", styled).without_copy_preview(),
        );
    div()
        .relative()
        .flex()
        .children(bands)
        .child(numbers)
        .child(text)
}

/// Moves the log by a key: the arrows, Page Up and Down, Home and End. Whether the key was one.
fn scroll_key(scroll: &ScrollHandle, key: &str) -> bool {
    let (old, max) = (scroll.offset(), scroll.max_offset().y);
    let page = (scroll.bounds().size.height - px(LINE)).max(px(LINE));
    let y = match key {
        "pageup" => old.y + page,
        "pagedown" => old.y - page,
        "up" => old.y + px(LINE),
        "down" => old.y - px(LINE),
        "home" => px(0.),
        "end" => -max,
        _ => return false,
    };
    scroll.set_offset(gpui::point(old.x, y.clamp(-max, px(0.))));
    true
}

impl Launcher {
    /// The scrolling list of `shown` in a card, following the newest line when Follow is on.
    pub(super) fn log_list(
        &self,
        shown: &[(usize, &str)],
        revision: u64,
        cx: &mut Context<Self>,
    ) -> Div {
        let logs = &self.ui.home.tabs.logs;
        let view = &self.ui.home.tabs.views[Tab::Logs.index()];
        let scroll = view.scroll.clone();
        crate::frame::scrolls_with(&scroll);
        let now = (revision, shown.len());
        let height = scroll.bounds().size.height;
        if height > px(0.) && logs.seen.get() != now {
            if logs.follow {
                let below = (px(shown.len() as f32 * LINE + 2. * PAD) - height).max(px(0.));
                scroll.set_offset(gpui::point(px(0.), -below));
            }
            logs.seen.set(now);
        }
        let (built, place) = view.near.list(&view.scroll, shown.len(), 1, LINE, 0.);
        let rows = log_lines(&shown[built], logs.search.text().trim());
        let keys = scroll.clone();
        let list = div()
            .id("home-logs-lines")
            .reach()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .py(px(PAD))
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .on_scroll(cx.listener(|_, _, _, cx| cx.notify()))
            .on_key_down(cx.listener(move |_, ev: &KeyDownEvent, _, cx| {
                if scroll_key(&keys, ev.keystroke.key.as_str()) {
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(place(rows));
        let card = crate::home::card()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .font_family(crate::theme::MONO)
            .text_size(px(tokens::SMALL))
            .line_height(px(LINE))
            .child(list);
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(card)
            .child(view.scrollbar.draw(&view.scroll, cx))
            .children(self.jump_pill(&scroll, cx))
    }

    /// "Jump to latest", over the card's bottom right: there while Follow is off and the view is
    /// not at the end.
    fn jump_pill(&self, scroll: &ScrollHandle, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let max = scroll.max_offset().y;
        let away = max > px(0.) && -scroll.offset().y < max - px(LINE);
        if self.ui.home.tabs.logs.follow || !away {
            return None;
        }
        let scroll = scroll.clone();
        Some(
            button(
                "home-logs-jump",
                tr("Jump to latest"),
                Look::new(Style::Primary, Size::Small).icon("arrow-down-01"),
            )
            .absolute()
            .bottom(px(14.))
            .right(px(22.))
            .shadow_lg()
            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                scroll.set_offset(gpui::point(px(0.), -scroll.max_offset().y));
                cx.notify();
            })),
        )
    }
}
