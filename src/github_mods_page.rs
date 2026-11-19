//! "Add from GitHub" on the Mods page: a link to a repository, release, tag, branch or commit,
//! resolved to an exact release file or commit (`sunrise_launcher::github_mods`), shown with the
//! plain warning and a link to the exact source, and installed only after the player says they
//! trust it. Mods installed that way list where they came from, with "Check for updates".
use super::line_edit::LineEdit;
use super::*;
use std::path::{Path, PathBuf};
use sunrise_launcher::github_mods::{self, Packed, Resolved};

enum Event {
    /// The mods of a source, fetched and checked, when there is a choice to make.
    Fetched(Result<Vec<Packed>, String>),
    Installed(Result<String, String>),
}

enum Stage {
    Idle,
    Working,
    /// The source holds several mods and nothing was asked for by name.
    Pick,
}

pub(super) struct Github {
    text: LineEdit,
    stage: Stage,
    /// The mods installed from GitHub in `home`, and what the last update check found for each.
    home: Option<PathBuf>,
    newer: std::collections::HashMap<String, Resolved>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
}

impl Github {
    pub(super) fn busy(&self) -> bool {
        matches!(self.stage, Stage::Working)
    }

    pub(super) fn new() -> Self {
        let (tx, rx) = crate::wake::channel();
        Self {
            text: LineEdit::new(String::new(), 300, 40),
            stage: Stage::Idle,
            home: None,
            newer: Default::default(),
            tx,
            rx,
        }
    }

    fn reread(&mut self, home: &Path) {
        self.home = Some(home.to_owned());
    }
}

/// Installs the chosen mods of `mods` and says what was installed.
fn finish(
    home: &Path,
    found: &Resolved,
    mods: &[Packed],
    only: Option<&str>,
    all: bool,
) -> Result<String, String> {
    let mut chosen = github_mods::choose(mods, only, all)?.into_iter();
    let mut installed = Vec::new();
    let mut failure = None;
    for p in chosen.by_ref() {
        match github_mods::install(home, found, p) {
            Ok(o) => installed.push(format!("{} {}", o.id, o.version)),
            Err(e) => {
                failure = Some(format!("{} {}: {e}", p.id, p.version));
                break;
            }
        }
    }
    let mut summary = match installed.len() {
        1 => tr("Installed 1 mod from GitHub").to_owned(),
        count => trf(
            "Installed {count} mods from GitHub",
            &[("count", &count.to_string())],
        ),
    };
    if failure.is_some() {
        summary = trf("{installed} · 1 failed", &[("installed", &summary)]);
        if !chosen.as_slice().is_empty() {
            summary = trf(
                "{result} · {count} not attempted",
                &[("result", &summary), ("count", &chosen.len().to_string())],
            );
        }
    }
    let mut details = vec![summary, String::new(), found.label()];
    if !installed.is_empty() {
        details.push(tr("Build installed mods on the Mods page.").into());
        details.extend(installed);
    }
    if let Some(error) = failure {
        details.extend([String::new(), tr("Failed:").into(), error]);
        if !chosen.as_slice().is_empty() {
            details.push(tr("Not attempted:").into());
            details.extend(chosen.map(|p| format!("{} {}", p.id, p.version)));
        }
        return Err(details.join("\n"));
    }
    Ok(details.join("\n"))
}

impl Launcher {
    pub(super) fn github_tick(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.github.rx.try_recv() {
            changed = true;
            match event {
                Event::Fetched(Ok(_)) => self.github.stage = Stage::Pick,
                Event::Fetched(Err(e)) => {
                    self.github.stage = Stage::Idle;
                    self.dismiss_notice();
                    self.mods.message = Some(Err(e));
                }
                Event::Installed(result) => {
                    self.github.stage = Stage::Idle;
                    if result.is_ok() {
                        self.github.text.set(String::new());
                        self.github.newer.clear();
                    }
                    self.instance_mods_changed();
                    self.dismiss_notice();
                    self.mods.message = Some(result);
                    let home = self.mods.home();
                    self.github.reread(&home);
                }
            }
        }
        changed
    }

    /// Downloads and checks what the player agreed to; installs it unless there is a choice.
    pub(super) fn github_install(&mut self, found: Resolved, only: Option<String>) {
        if self.settings.native_mods_allowed_at.is_none() {
            self.pending_consent = Some(PendingConsent::Github(found, only));
            return;
        }
        self.github.stage = Stage::Working;
        let tx = self.github.tx.clone();
        let home = self.mods.home();
        std::thread::spawn(move || {
            let fetched = github_mods::fetch(&found);
            let result = match fetched {
                Ok(mods) if mods.len() == 1 || only.is_some() => {
                    let done = finish(&home, &found, &mods, only.as_deref(), false);
                    github_mods::discard(&found);
                    Event::Installed(done)
                }
                other => Event::Fetched(other),
            };
            let _ = tx.send(result);
        });
    }
}
