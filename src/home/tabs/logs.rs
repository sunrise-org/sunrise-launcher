//! The profile's Logs tab: a source (`sources`: the game's output of this run as it comes, its
//! `latest.log`, crash reports, the launcher's diagnostics, the native build log) and its lines.
//! A search, Follow (keep the newest line in view), Copy, Share log (`share`), Open file and
//! Open logs folder.
use super::*;
use crate::line_edit::{self, Key, LineEdit};
use crate::widgets::button::{button, primary, secondary, Look, Size, Style};
use crate::widgets::check::switch;
use crate::widgets::feedback::{banner, empty_state, empty_state_icon, Tone};
use crate::widgets::focus::c;
use crate::widgets::motion_bits::copy_button;
use sources::{nothing_here, Source};
use sunrise_launcher::game_log::{self, Shared};
use tail::{FileLog, Tail};

mod share;
mod sources;
mod tail;
#[cfg(test)]
mod tests;

/// A line of the log. Fixed, so a long log builds only the lines near the view.
const LINE: f32 = 20.;

pub(super) struct Logs {
    search: LineEdit,
    focus: Option<FocusHandle>,
    follow: bool,
    level: Level,
    source: Source,
    /// The file of a list source (crash report, launcher log) the player picked.
    chosen: Option<std::path::PathBuf>,
    /// The files of a list source, newest first, and whether they were read yet.
    files: Vec<std::path::PathBuf>,
    listed: bool,
    list_checked: Option<std::time::Instant>,
    /// The folder is being listed off the UI thread.
    listing: bool,
    /// The file of the source, shown while this session has no run lines.
    file: Option<FileLog>,
    /// A read of `file` is running off the UI thread.
    reading: bool,
    /// When `file` was last checked for a change.
    checked: Option<std::time::Instant>,
    /// Revision and lines shown at the last draw: a change means scroll down when following.
    seen: Cell<(u64, usize)>,
    confirm: bool,
    sharing: bool,
    shared: Option<Result<Shared, String>>,
}

impl Default for Logs {
    fn default() -> Self {
        Self {
            search: LineEdit::new(String::new(), 128, 24),
            focus: None,
            follow: true,
            level: Level::All,
            source: Source::default(),
            chosen: None,
            files: Vec::new(),
            listed: false,
            list_checked: None,
            listing: false,
            file: None,
            reading: false,
            checked: None,
            seen: Cell::new((0, 0)),
            confirm: false,
            sharing: false,
            shared: None,
        }
    }
}

/// How bad a log line is: 0 for information, 1 for a warning, 2 for an error.
fn severity(line: &str) -> u8 {
    if line.contains("/ERROR]") || line.contains("ERROR:") || line.contains(" ERROR ") {
        2
    } else if line.contains("/WARN]") || line.contains(" WARN ") || line.contains("WARN:") {
        1
    } else {
        0
    }
}

/// The lines drawn now as one block of selectable text, each line coloured by how bad it is, so
/// a drag selects across lines and copies as any text does. Tabs (stack traces) become spaces;
/// a line too long for the box is cut, not wrapped, so every line keeps its `LINE` height.
fn log_lines(lines: &[&str]) -> Div {
    let p = palette();
    let mut text = String::new();
    let mut runs = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        if n > 0 {
            text.push('\n');
        }
        let ink = match severity(line) {
            2 => p.danger,
            1 => p.warning,
            _ => p.muted,
        };
        let start = text.len();
        text.push_str(&line.replace('\t', "    "));
        runs.push((
            start..text.len(),
            gpui::HighlightStyle {
                color: Some(c(ink)),
                ..Default::default()
            },
        ));
    }
    let styled = gpui::StyledText::new(text).with_highlights(runs);
    div()
        .flex_none()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .line_height(px(LINE))
        .text_color(c(p.muted))
        .child(
            crate::selectable::Selectable::styled("home-logs-text", styled).without_copy_preview(),
        )
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

/// Which lines show: every one, warnings and errors, or errors only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Level {
    All,
    Warnings,
    Errors,
}

impl Level {
    /// The least `severity` a line has to have.
    fn floor(self) -> u8 {
        match self {
            Self::All => 0,
            Self::Warnings => 1,
            Self::Errors => 2,
        }
    }
}

/// What the log area has to say when it has no line to show.
enum Nothing {
    Loading,
    NoLog,
    Failed(String),
}

/// The log area, the text to copy and whether there are lines.
struct LogBox {
    body: Div,
    copy: String,
    has_lines: bool,
}

impl Launcher {
    /// Whether the lines to show are this session's run of the profile (else a file).
    fn log_is_run(&self, home: &std::path::Path) -> bool {
        self.ui.home.tabs.logs.source == Source::Game
            && (!self.prepare_log.is_empty() || self.client.is_some())
            && self.client_home.as_deref().is_none_or(|run| run == home)
    }

    /// Reads the end of `latest` off the UI thread, and again only when it changed.
    fn read_file(&mut self, latest: std::path::PathBuf, cx: &mut Context<Self>) {
        let logs = &mut self.ui.home.tabs.logs;
        if logs.file.as_ref().is_none_or(|f| f.path != latest) {
            logs.file = Some(FileLog {
                path: latest.clone(),
                stamp: None,
                tail: Tail::Loading,
            });
            logs.checked = None;
        }
        let due = logs
            .checked
            .is_none_or(|at| at.elapsed() > std::time::Duration::from_secs(2));
        if !due || logs.reading {
            return;
        }
        logs.reading = true;
        logs.checked = Some(std::time::Instant::now());
        let known = logs.file.as_ref().and_then(|f| f.stamp);
        let path = latest;
        let task = cx.background_executor().spawn({
            let path = path.clone();
            async move { tail::load_tail(&path, known) }
        });
        cx.spawn(async move |this, cx| {
            let loaded = task.await;
            let _ = this.update(cx, |this, cx| {
                let logs = &mut this.ui.home.tabs.logs;
                logs.reading = false;
                match (logs.file.as_mut(), loaded) {
                    (Some(file), Some((stamp, tail))) if file.path == path => {
                        file.stamp = stamp;
                        file.tail = tail;
                    }
                    (Some(file), None) if file.path == path => return,
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The Logs tab, in the panel the other tabs stand in: the tabs and Open logs folder, the
    /// toolbar, and the log under it.
    pub(super) fn home_logs(
        &mut self,
        strip: Stateful<Div>,
        instance: &instances::Instance,
        inset: f32,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let source = self.ui.home.tabs.logs.source;
        let folder = source.folder(&instance.home);
        if !self.log_is_run(&instance.home) {
            self.follow_source(&instance.home, &folder, cx);
        }
        let focus = self
            .ui
            .home
            .tabs
            .logs
            .focus
            .get_or_insert_with(|| cx.focus_handle().tab_stop(true))
            .clone();
        let LogBox {
            body,
            copy,
            has_lines,
        } = self.log_box(&instance.home, &folder, cx);
        let open = {
            let folder = folder.clone();
            folder_button("home-logs-folder").on_click(move |_, _, cx| open_logs(cx, &folder))
        };
        let latest = self
            .ui
            .home
            .tabs
            .logs
            .target(&instance.home)
            .filter(|path| path.is_file());
        let open_file = latest.map(|latest| {
            secondary("home-logs-open-file", tr("Open file"))
                .on_click(move |_, _, cx| crate::toast::open(cx, &latest))
        });
        let toolbar = has_lines.then(|| self.log_toolbar(&focus, copy, window, cx));
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(12.))
            .overflow_hidden()
            .px(px(inset))
            .pt(px(10.))
            .pb(px(20.))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .gap(px(10.))
                    .child(strip)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .children(open_file)
                            .child(open),
                    ),
            )
            .child(self.source_row(cx))
            .children(toolbar)
            .children(self.share_panel(&instance.home, cx))
            .child(body)
    }

    /// The search and Follow on the left, Copy and Share log on the right; it wraps in a narrow
    /// window.
    fn log_toolbar(
        &self,
        focus: &FocusHandle,
        copy: String,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let logs = &self.ui.home.tabs.logs;
        let toggle = |this: &mut Self, cx: &mut Context<Self>| {
            let logs = &mut this.ui.home.tabs.logs;
            logs.follow = !logs.follow;
            // Switching it on goes to the newest line at once.
            logs.seen.set((u64::MAX, 0));
            cx.notify();
        };
        let follow = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                switch("home-logs-follow", logs.follow)
                    .on_click(cx.listener(move |this, _, _, cx| toggle(this, cx))),
            )
            .child(
                div()
                    .id("home-logs-follow-label")
                    .cursor_pointer()
                    .text_size(px(tokens::LABEL))
                    .text_color(c(palette().text))
                    .child(tr("Follow"))
                    .on_click(cx.listener(move |this, _, _, cx| toggle(this, cx))),
            );
        let share = button(
            "home-logs-share",
            tr("Share log…"),
            Look::new(Style::Secondary, Size::Medium)
                .icon("link-01")
                .disabled(logs.sharing),
        );
        let share = match logs.sharing {
            true => share,
            false => share
                .tooltip(crate::glass_ui::tip(tr(
                    "Uploads the log to mclo.gs, after you confirm",
                )))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.ui.home.tabs.logs.confirm = true;
                    cx.notify();
                })),
        };
        div()
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .child(self.log_search(focus, window, cx))
                    .child(self.level_choice(cx))
                    .child(follow),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(copy_button("home-logs-copy", tr("Copy"), copy))
                    .child(share),
            )
    }

    /// All, Warnings or Errors: which lines the log shows.
    fn level_choice(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let this = cx.entity().downgrade();
        let options = [
            (Level::All, SharedString::from(tr("All"))),
            (Level::Warnings, SharedString::from(tr("Warnings"))),
            (Level::Errors, SharedString::from(tr("Errors"))),
        ];
        crate::widgets::choice::choices(
            "home-logs-level",
            &options,
            self.ui.home.tabs.logs.level,
            move |level, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    let logs = &mut this.ui.home.tabs.logs;
                    logs.level = level;
                    logs.seen.set((u64::MAX, 0));
                    cx.notify();
                });
            },
        )
    }

    /// The search field: the text lines must contain, Escape to leave it.
    fn log_search(
        &self,
        focus: &FocusHandle,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = crate::mods::field_theme(self.theme);
        let focused = focus.is_focused(window);
        let field = crate::widgets::field::search_box(
            focused,
            None,
            line_edit::content(
                &self.ui.home.tabs.logs.search,
                &t,
                focused,
                false,
                false,
                "Search the log",
            ),
        )
        .id("home-logs-search")
        .track_focus(focus)
        .seen()
        .w(px(240.))
        .min_w(px(140.))
        .cursor_text()
        .on_click({
            let focus = focus.clone();
            cx.listener(move |_, _: &ClickEvent, window, cx| {
                window.focus(&focus, cx);
                cx.notify();
            })
        })
        .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
            match line_edit::handle(&mut this.ui.home.tabs.logs.search, ev, cx) {
                Key::Escape => window.blur(cx),
                Key::Pass => return,
                _ => {}
            }
            cx.stop_propagation();
            cx.notify();
        }));
        line_edit::mouse(field, cx, |this: &mut Launcher| {
            &mut this.ui.home.tabs.logs.search
        })
    }

    /// The sunken box with the lines in it (only those near the view are built), or why there
    /// are none.
    fn log_box(
        &self,
        home: &std::path::Path,
        folder: &std::path::Path,
        cx: &mut Context<Self>,
    ) -> LogBox {
        let logs = &self.ui.home.tabs.logs;
        let search = logs.search.text().trim();
        let (all, revision, nothing): (Vec<&str>, u64, Option<Nothing>) = if self.log_is_run(home) {
            let lines = self.prepare_log.iter().map(String::as_str).collect();
            (lines, self.log_revision, None)
        } else {
            match logs.file.as_ref().map(|f| (&f.tail, f.stamp)) {
                Some((Tail::Lines(lines), stamp)) => (
                    lines.iter().map(String::as_str).collect(),
                    stamp.map_or(0, |(len, _)| len),
                    None,
                ),
                Some((Tail::Absent, _)) => (vec![], 0, Some(Nothing::NoLog)),
                Some((Tail::Failed(e), _)) => (vec![], 0, Some(Nothing::Failed(e.clone()))),
                Some((Tail::Loading, _)) => (vec![], 0, Some(Nothing::Loading)),
                // A list with no file is empty, once it has been read.
                None if logs.source.is_list() && logs.listed => (vec![], 0, Some(Nothing::NoLog)),
                None => (vec![], 0, Some(Nothing::Loading)),
            }
        };
        let nothing = nothing.or_else(|| all.is_empty().then_some(Nothing::NoLog));
        let total = all.len();
        let shown: Vec<&str> = all
            .into_iter()
            .filter(|line| severity(line) >= logs.level.floor())
            .filter(|line| search.is_empty() || game_log::matches(line, search))
            .collect();
        let sunken = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .rounded(tokens::r(tokens::R_CARD))
            .bg(c(palette().field))
            .border_1()
            .border_color(c(palette().line));
        if let Some(nothing) = nothing {
            return LogBox {
                body: sunken.child(self.nothing_to_show(nothing, folder, cx)),
                copy: String::new(),
                has_lines: false,
            };
        }
        let copy = shown.join("\n");
        let body = if shown.is_empty() {
            sunken.child(empty_state(
                tr("No line matches"),
                tr("Clear the search or pick All to see the whole log."),
                None,
            ))
        } else {
            self.log_list(sunken, &shown, revision, cx)
        };
        LogBox {
            body,
            copy,
            has_lines: total > 0,
        }
    }

    /// The scrolling list of `shown`, in `sunken`, following the newest line when Follow is on.
    fn log_list(&self, sunken: Div, shown: &[&str], revision: u64, cx: &mut Context<Self>) -> Div {
        let logs = &self.ui.home.tabs.logs;
        let view = &self.ui.home.tabs.views[Tab::Logs.index()];
        let scroll = view.scroll.clone();
        crate::frame::scrolls_with(&scroll);
        let now = (revision, shown.len());
        let height = scroll.bounds().size.height;
        if height > px(0.) && logs.seen.get() != now {
            if logs.follow {
                let below = (px(shown.len() as f32 * LINE) - height).max(px(0.));
                scroll.set_offset(gpui::point(px(0.), -below));
            }
            logs.seen.set(now);
        }
        let (built, place) = view.near.list(&view.scroll, shown.len(), 1, LINE, 0.);
        let rows = log_lines(&shown[built]);
        let keys = scroll.clone();
        let list = div()
            .id("home-logs-lines")
            .reach()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
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
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                sunken
                    .p(px(12.))
                    .font_family(crate::theme::MONO)
                    .text_size(px(tokens::SMALL))
                    .line_height(px(LINE))
                    .child(list),
            )
            .child(view.scrollbar.draw(&view.scroll, cx))
    }

    /// The empty state of the log area: no log yet, one that could not be read, or still reading.
    fn nothing_to_show(
        &self,
        nothing: Nothing,
        folder: &std::path::Path,
        cx: &mut Context<Self>,
    ) -> Div {
        let folder = folder.to_path_buf();
        let open = folder_button("home-logs-folder-empty")
            .on_click(move |_, _, cx| open_logs(cx, &folder))
            .into_any_element();
        match nothing {
            Nothing::Loading => div().flex_1(),
            Nothing::NoLog => nothing_here(self.ui.home.tabs.logs.source, open),
            Nothing::Failed(error) => empty_state(
                tr("Could not read this log"),
                error,
                Some(
                    secondary("home-logs-retry", tr("Retry"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.ui.home.tabs.logs.file = None;
                            cx.notify();
                        }))
                        .into_any_element(),
                ),
            ),
        }
    }
}

/// The Open logs folder button: a folder icon and its label.
fn folder_button(id: &'static str) -> Stateful<Div> {
    button(
        id,
        tr("Open logs folder"),
        Look::new(Style::Secondary, Size::Medium).icon("folder-01"),
    )
}

/// Opens the profile's `logs` folder, making it first so it opens before a first run.
fn open_logs(cx: &mut App, folder: &std::path::Path) {
    let _ = std::fs::create_dir_all(folder);
    crate::toast::open(cx, folder);
}
