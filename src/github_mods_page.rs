//! "Add from GitHub" on the Mods page: a link to a repository, release, tag, branch or commit,
//! resolved to an exact release file or commit (`sunrise_launcher::github_mods`), shown with the
//! plain warning and a link to the exact source, and installed only after the player says they
//! trust it. Mods installed that way list where they came from, with "Check for updates".
use super::line_edit::LineEdit;
use super::*;
use crate::mods_page::install::WorldQuestion;
use std::path::{Path, PathBuf};
use sunrise_launcher::github_mods::{self, Packed, Resolved};
use sunrise_launcher::native_mods;
use sunrise_launcher::{browser::sunrise, managed_play::FirstBuild};

enum Event {
    /// The mods of a source, fetched and checked, when there is a choice to make.
    Fetched(Result<Vec<Packed>, String>),
    Review(WorldQuestion, Sender<bool>),
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
    message: Option<Result<String, String>>,
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
            message: None,
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

impl Launcher {
    pub(super) fn github_tick(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.github.rx.try_recv() {
            changed = true;
            match event {
                Event::Fetched(Ok(_)) => self.github.stage = Stage::Pick,
                Event::Fetched(Err(e)) => {
                    self.github.stage = Stage::Idle;
                    self.github.message = Some(Err(e));
                }
                Event::Review(question, reply) => {
                    if let Some((_, old)) = self.mods.installs.world.replace((question, reply)) {
                        let _ = old.send(false);
                    }
                }
                Event::Installed(result) => {
                    self.github.stage = Stage::Idle;
                    if result.is_ok() {
                        self.github.text.set(String::new());
                        self.github.newer.clear();
                    }
                    self.instance_mods_changed();
                    self.github.message = Some(result);
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
        if self.mutating() || self.mods_locked().is_some() {
            self.github.message = Some(Err(tr("Wait for the current operation to finish.").into()));
            return;
        }
        let Some(instance) = self.selected_instance().cloned() else {
            self.github.message = Some(Err(backend::no_version_installed()));
            return;
        };
        if instance.kind == instances::Kind::Pack {
            self.github.message =
                Some(Err(tr("This pack controls which mods it installs.").into()));
            return;
        }
        let identity = match sunrise::install_target(&instance) {
            Ok(identity) => identity,
            Err(error) => {
                self.github.message = Some(Err(error));
                return;
            }
        };
        let target = self.profile_target(&instance).cloned();
        let release = target
            .as_deref()
            .and_then(|tag| self.release_to_install(tag));
        self.github.stage = Stage::Working;
        let tx = self.github.tx.clone();
        let launcher = self.tx.clone();
        std::thread::spawn(move || {
            let fetched = github_mods::fetch(&found);
            let result = match fetched {
                Ok(mods) if mods.len() == 1 || only.is_some() => {
                    let done = (|| {
                        let mut installed = Vec::new();
                        let mut updated = Vec::new();
                        for pack in github_mods::choose(&mods, only.as_deref(), false)? {
                            let path = instance
                                .home
                                .join(native_mods::INSTALLED)
                                .join(format!("{}.smod", pack.id));
                            let replace = match std::fs::symlink_metadata(&path) {
                                Ok(_) => true,
                                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                                Err(error) => return Err(format!("{}: {error}", path.display())),
                            };
                            if !replace {
                                let origin = github_mods::install(&instance.home, &found, pack)?;
                                installed.push(format!("{} {}", origin.id, origin.version));
                                continue;
                            }
                            let origin = sunrise::update_github(
                                &instance,
                                identity,
                                &found,
                                pack,
                                || {
                                    if let Some(tag) = target.as_deref().filter(|tag| {
                                        backend::installed().iter().any(|local| local == tag)
                                    }) {
                                        FirstBuild::installed(tag)
                                    } else if let Some(release) = &release {
                                        FirstBuild::download(release)
                                    } else {
                                        Err(backend::no_version_installed())
                                    }
                                },
                                sunrise::InstallHooks {
                                    progress: &|_| {},
                                    native: &|_| {
                                        native_mods::require_consent()
                                            .map(|_| true)
                                            .map_err(String::from)
                                    },
                                    registry_crates: Some(&|consent| {
                                        let (reply, answer) = std::sync::mpsc::channel();
                                        launcher
                                            .send(Msg::NativeRegistryConsent(
                                                consent.clone(),
                                                reply,
                                            ))
                                            .is_ok()
                                            && answer.recv().unwrap_or(false)
                                    }),
                                    continue_after_failure: &|_| false,
                                },
                                &|impact| {
                                    let (reply, answer) = std::sync::mpsc::channel();
                                    let question = WorldQuestion {
                                        removal: false,
                                        detail: impact.detail(),
                                    };
                                    tx.send(Event::Review(question, reply)).is_ok()
                                        && answer.recv().unwrap_or(false)
                                },
                            )
                            .map_err(|error| match error {
                                sunrise::InstallError::Cancelled => {
                                    tr("Nothing was updated.").into()
                                }
                                e => e.to_string(),
                            })?;
                            updated.push(format!("{} {}", origin.id, origin.version));
                        }
                        let mut messages = Vec::new();
                        if !installed.is_empty() {
                            messages.push(trf(
                                "Installed {mods} from GitHub ({source}). Build it on the Mods page.",
                                &[("mods", &installed.join(", ")), ("source", &found.label())],
                            ));
                        }
                        if !updated.is_empty() {
                            messages.push(trf(
                                "Updated {mods} from GitHub ({source}).",
                                &[("mods", &updated.join(", ")), ("source", &found.label())],
                            ));
                        }
                        Ok(messages.join("\n"))
                    })();
                    github_mods::discard(&found);
                    Event::Installed(done)
                }
                other => Event::Fetched(other),
            };
            let _ = tx.send(result);
        });
    }
}
