//! The profile's Logs tab: a source (`sources`: the game's output of this run as it comes, its
//! `latest.log`, crash reports, the launcher's diagnostics, the native build log) and its lines.
//! The row under the tabs (`menus`) picks the source and its file, searches, filters by level,
//! follows the newest line and holds Copy all, Share log (`share`), Open file and Open logs
//! folder; the lines are in a card (`view`), or an empty state in the page when there are none.
use super::*;
use crate::line_edit::{self, Key, LineEdit};
use crate::widgets::button::{button, primary, secondary, Look, Size, Style};
use crate::widgets::feedback::{banner, empty_state, empty_state_icon, Tone};
use menus::Menu;
use sources::nothing_here;
pub(crate) use sources::Source;
use sunrise_launcher::game_log::{self, Shared};
use tail::{FileLog, Tail};
use view::severity;

mod menus;
mod share;
mod sources;
mod tail;
#[cfg(test)]
mod tests;
mod view;

/// The log an error on Home came from, if it left one: what its "View logs" opens. Mirrors the
/// order `Facts::view` picks the problem it says in.
///
/// - the game's preparer: this session's game output (`Game`), which holds the preparer's own
///   lines while it runs
/// - a native mods build: the build log (`Build`)
/// - a game that stopped: the crash reports, newest first (`Crashes`)
/// - a start that failed: the launcher's diagnostics (`Launcher`)
/// - a download or install: none
pub(crate) fn error_log(facts: &crate::launch_state::Facts) -> Option<Source> {
    if facts.install_recovery.is_some() || facts.building {
        return None;
    }
    if facts.client_error.is_some() {
        Some(Source::Launcher)
    } else if facts.prepare_error.is_some() {
        Some(Source::Game)
    } else if facts.build_result.is_some() || (facts.native_error.is_some() && facts.held.is_none())
    {
        Some(Source::Build)
    } else if facts.install_error.is_some() {
        None
    } else if facts.crash.is_some() {
        Some(Source::Crashes)
    } else {
        None
    }
}

impl Launcher {
    /// Shows Home's Logs tab with `source` selected, from any page: the "View logs" of an error.
    /// A list source (crash reports, launcher diagnostics) opens on its newest file.
    pub(crate) fn show_logs(&mut self, source: Source) {
        self.navigate(crate::frame::Page::Home);
        let tabs = &mut self.ui.home.tabs;
        tabs.selected = super::Tab::Logs;
        tabs.logs.switch(source);
        tabs.logs.chosen = None;
        tabs.logs.seen.set((u64::MAX, 0));
    }
}

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
    /// When each of `files` was last written, for their names in the picker.
    times: Vec<Option<std::time::SystemTime>>,
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
    /// Which of the row's menus is open.
    menu: Option<Menu>,
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
            times: Vec::new(),
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
            menu: None,
        }
    }
}

/// Which lines show: every one, warnings and errors, or errors only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Level {
    All,
    Warnings,
    Errors,
}

impl Level {
    const ALL: [Self; 3] = [Self::All, Self::Warnings, Self::Errors];

    fn label(self) -> SharedString {
        match self {
            Self::All => tr("All").into(),
            Self::Warnings => tr("Warnings").into(),
            Self::Errors => tr("Errors").into(),
        }
    }

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

/// The log area and whether there are lines.
struct LogBox {
    body: Div,
    has_lines: bool,
}

/// The lines the source has, those the level and search leave, and why there are none.
struct Shown<'a> {
    /// Each line with its place in the log, for the numbers in the gutter.
    lines: Vec<(usize, &'a str)>,
    revision: u64,
    nothing: Option<Nothing>,
    /// How many lines the source has, before the level and search.
    total: usize,
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

    /// The Logs tab, in the panel the other tabs stand in: the tabs, the row under them, and the
    /// log (or why there is none) below.
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
        let LogBox { body, has_lines } = self.log_box(&instance.home, &folder, cx);
        let toolbar = self.log_toolbar(&focus, &instance.home, &folder, has_lines, window, cx);
        // Open logs folder is in the "…" menu and the empty state, so the tabs' row has no action.
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .px(px(inset))
            .pt(px(10.))
            .pb(px(20.))
            .child(div().flex().flex_none().items_center().child(strip))
            .child(toolbar)
            .children(
                self.share_panel(&instance.home, cx)
                    .map(|panel| panel.mb(px(12.))),
            )
            .child(body)
    }

    /// The lines the level and search leave of what the source has.
    fn shown_lines(&self, home: &std::path::Path) -> Shown<'_> {
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
        let lines = all
            .into_iter()
            .enumerate()
            .filter(|(_, line)| severity(line) >= logs.level.floor())
            .filter(|(_, line)| search.is_empty() || game_log::matches(line, search))
            .collect();
        Shown {
            lines,
            revision,
            nothing,
            total,
        }
    }

    /// What Copy all copies: the lines shown, one to a line.
    pub(super) fn copy_text(&self, home: &std::path::Path) -> String {
        let shown = self.shown_lines(home);
        let lines: Vec<&str> = shown.lines.iter().map(|(_, line)| *line).collect();
        lines.join("\n")
    }

    /// The card with the lines in it (only those near the view are built), or why there are
    /// none, in the page.
    fn log_box(
        &self,
        home: &std::path::Path,
        folder: &std::path::Path,
        cx: &mut Context<Self>,
    ) -> LogBox {
        let shown = self.shown_lines(home);
        if let Some(nothing) = shown.nothing {
            return LogBox {
                body: self.nothing_to_show(nothing, folder, cx),
                has_lines: false,
            };
        }
        let body = if shown.lines.is_empty() {
            empty_state(
                tr("No line matches"),
                tr("Clear the search or pick All to see the whole log."),
                None,
            )
        } else {
            self.log_list(&shown.lines, shown.revision, cx)
        };
        LogBox {
            body,
            has_lines: shown.total > 0,
        }
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
