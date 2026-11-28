//! Which log the Logs tab shows: this run's game output, the game's `latest.log`, its crash
//! reports, the launcher's own diagnostics and the native mods' build log. The lists of files
//! (crash reports, launcher diagnostics) are read off the UI thread, newest first.
use super::*;
use std::path::{Path, PathBuf};

/// How often a folder's list of logs is read again.
const RELIST: std::time::Duration = std::time::Duration::from_secs(3);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Source {
    /// This session's run, as it comes; the profile's `latest.log` while there is none.
    #[default]
    Game,
    /// The profile's `latest.log`, run or not.
    Latest,
    /// The profile's `crash-reports`, one file at a time.
    Crashes,
    /// What the launcher wrote about a game it could not start (`run_leases`), one file at a time.
    Launcher,
    /// The last native mods build (`native_mods::log_path`).
    Build,
}

impl Source {
    pub(super) const ALL: [Self; 5] = [
        Self::Game,
        Self::Latest,
        Self::Crashes,
        Self::Launcher,
        Self::Build,
    ];

    /// Whether the source is a folder of files, one of which is read.
    pub(super) fn is_list(self) -> bool {
        matches!(self, Self::Crashes | Self::Launcher)
    }

    pub(super) fn label(self) -> SharedString {
        match self {
            Self::Game => tr("Game output").into(),
            Self::Latest => "latest.log".into(),
            Self::Crashes => tr("Crash reports").into(),
            Self::Launcher => tr("Launcher").into(),
            Self::Build => tr("Build").into(),
        }
    }

    /// Whether a file called `name` belongs to the source's list.
    fn accepts(self, name: &str) -> bool {
        match self {
            Self::Crashes => name.starts_with("crash-") && name.ends_with(".txt"),
            Self::Launcher => name.starts_with("launch-failure-") && name.ends_with(".log"),
            _ => false,
        }
    }

    /// The folder Open logs folder opens and a list is read from, for the profile at `home`.
    pub(super) fn folder(self, home: &Path) -> PathBuf {
        match self {
            Self::Game | Self::Latest => home.join("logs"),
            Self::Crashes => home.join("crash-reports"),
            Self::Launcher => sunrise_launcher::backend::data_dir().join("logs"),
            Self::Build => sunrise_launcher::native_mods::log_path()
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default(),
        }
    }
}

/// The files of `source`'s list in `dir`, newest first (by modification time, then by name).
pub(super) fn list_files(dir: &Path, source: Source) -> Vec<PathBuf> {
    let mut found: Vec<(Option<std::time::SystemTime>, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|n| source.accepts(n)))
        .map(|e| (e.metadata().and_then(|m| m.modified()).ok(), e.path()))
        .collect();
    found.sort_by(|a, b| b.cmp(a));
    found.into_iter().map(|(_, path)| path).collect()
}

/// Which of `files` is read: the one `chosen` while it is still there, else the newest.
pub(super) fn pick_index(files: &[PathBuf], chosen: Option<&Path>) -> Option<usize> {
    let at = chosen.and_then(|c| files.iter().position(|f| f == c));
    at.or((!files.is_empty()).then_some(0))
}

/// What a file of a list is called on its choice: a crash report by its date, else by its place
/// in the list (the newest is 1).
pub(super) fn file_label(source: Source, index: usize, path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    match source {
        Source::Crashes => name
            .trim_start_matches("crash-")
            .trim_end_matches(".txt")
            .trim_end_matches("-client")
            .to_string(),
        _ => trf("Failure {n}", &[("n", &(index + 1).to_string())]).to_string(),
    }
}

impl Logs {
    /// Shows `source`, with nothing of the last one left over.
    pub(super) fn switch(&mut self, source: Source) {
        if self.source == source {
            return;
        }
        self.source = source;
        self.chosen = None;
        self.files.clear();
        self.times.clear();
        self.menu = None;
        self.listed = false;
        self.list_checked = None;
        self.file = None;
        self.checked = None;
        self.confirm = false;
        self.seen.set((u64::MAX, 0));
    }

    /// The file the source reads for the profile at `home`: `None` for a list with no file (yet).
    pub(super) fn target(&self, home: &Path) -> Option<PathBuf> {
        match self.source {
            Source::Game | Source::Latest => Some(home.join("logs/latest.log")),
            Source::Build => Some(sunrise_launcher::native_mods::log_path()),
            Source::Crashes | Source::Launcher => {
                pick_index(&self.files, self.chosen.as_deref()).map(|i| self.files[i].clone())
            }
        }
    }
}

impl Launcher {
    /// Keeps the file the source shows read: lists the folder when the source is one, then reads
    /// the file picked. Both run off the UI thread.
    pub(super) fn follow_source(&mut self, home: &Path, folder: &Path, cx: &mut Context<Self>) {
        let source = self.ui.home.tabs.logs.source;
        if source.is_list() {
            self.list_source(source, folder.to_path_buf(), cx);
        }
        match self.ui.home.tabs.logs.target(home) {
            Some(path) => self.read_file(path, cx),
            None => self.ui.home.tabs.logs.file = None,
        }
    }

    /// Lists `folder` for `source`, again once `RELIST` has passed.
    fn list_source(&mut self, source: Source, folder: PathBuf, cx: &mut Context<Self>) {
        let logs = &mut self.ui.home.tabs.logs;
        let due = logs.list_checked.is_none_or(|at| at.elapsed() > RELIST);
        if !due || logs.listing {
            return;
        }
        logs.listing = true;
        logs.list_checked = Some(std::time::Instant::now());
        let task = cx.background_executor().spawn(async move {
            let files = list_files(&folder, source);
            let times = files
                .iter()
                .map(|file| std::fs::metadata(file).and_then(|m| m.modified()).ok())
                .collect::<Vec<_>>();
            (files, times)
        });
        cx.spawn(async move |this, cx| {
            let (files, times) = task.await;
            let _ = this.update(cx, |this, cx| {
                let logs = &mut this.ui.home.tabs.logs;
                logs.listing = false;
                if logs.source == source {
                    logs.listed = true;
                    logs.times = times;
                    if logs.files != files {
                        logs.files = files;
                    } else {
                        return;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// The empty state of a source with nothing to show, with `open` (Open logs folder) under it.
pub(super) fn nothing_here(source: Source, open: AnyElement) -> Div {
    let (glyph, title, body) = match source {
        Source::Game => (
            "computer-terminal-01",
            tr("No game log yet"),
            tr("The game's output shows here once it starts."),
        ),
        Source::Latest => (
            "book",
            tr("No latest.log yet"),
            tr("The game writes it when it starts. Nothing has run in this profile yet."),
        ),
        Source::Crashes => (
            "shield",
            tr("No crash reports"),
            tr("That's good news. A report shows up here if the game ever crashes."),
        ),
        Source::Launcher => (
            "information-circle",
            tr("No launcher diagnostics"),
            tr(
                "The launcher writes one here when it cannot start the game. Nothing has gone wrong yet.",
            ),
        ),
        Source::Build => (
            "hammer",
            tr("No build log yet"),
            tr("Building native mods writes its output here."),
        ),
    };
    empty_state_icon(Some(glyph), title, body, Some(open))
}

/// The empty state of the whole tab: no source has anything, with `open` (Open logs folder) under it.
pub(super) fn no_logs_yet(open: AnyElement) -> Div {
    empty_state_icon(
        Some("computer-terminal-01"),
        tr("No logs yet"),
        tr("Logs from playing, building and crashes show up here."),
        Some(open),
    )
}
