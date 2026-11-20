//! The installed builds and import cards in Settings → Storage.
use crate::frame::button;
use crate::mods::switch;
use crate::settings::{compact_group, danger, hint, off, row, warning};
use sunrise_launcher::backend::version_storage::{self, Build, InUse};

use super::*;

#[derive(Default)]
pub(super) struct View {
    rows: Option<Vec<Build>>,
    generation: u64,
    pub(super) confirm: Option<Confirm>,
    remove_error: Option<String>,
    import_notice: Option<Result<String, String>>,
}

pub(super) enum Confirm {
    One(String),
    Old(Vec<String>),
}

impl Launcher {
    pub(super) fn refresh_storage_builds(&mut self) {
        self.storage_builds.generation += 1;
        let generation = self.storage_builds.generation;
        let releases = self.game_releases().to_vec();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let rows = version_storage::inventory(&releases);
            let _ = tx.send(Msg::BuildInventory(generation, rows));
        });
    }

    pub(super) fn refresh_open_storage_builds(&mut self) {
        if self.ui.page == frame::Page::Settings
            && self.ui.settings.section == settings::Section::Storage
        {
            self.refresh_storage_builds();
        }
    }

    pub(super) fn storage_builds_arrived(&mut self, generation: u64, rows: Vec<Build>) {
        if self.storage_builds.generation == generation {
            self.storage_builds.rows = Some(rows);
        }
    }

    fn can_change_builds(&self) -> bool {
        !self.importing_build
            && self.removing.is_empty()
            && !self.launch_busy()
            && !self.location_unavailable
    }

    fn remove_storage_builds(&mut self, cx: &mut Context<Self>) {
        let confirm = self.storage_builds.confirm.take();
        if !self.can_change_builds() {
            cx.notify();
            return;
        }
        let (tags, old_only) = match confirm {
            Some(Confirm::One(tag)) => (vec![tag], false),
            Some(Confirm::Old(tags)) => (tags, true),
            None => return,
        };
        self.removing.clone_from(&tags);
        self.storage_builds.remove_error = None;
        let releases = self.game_releases().to_vec();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let errors = version_storage::remove_builds(&tags, &releases, old_only);
            let _ = tx.send(Msg::Removed(errors));
        });
        cx.notify();
    }

    pub(super) fn storage_builds_removed(&mut self, errors: Vec<(String, String)>) {
        self.removing.clear();
        self.storage_builds_pruned(errors);
        self.refresh_storage(None, false);
    }

    pub(super) fn storage_builds_pruned(&mut self, errors: Vec<(String, String)>) {
        self.installed = backend::installed();
        self.storage_builds.remove_error = (!errors.is_empty()).then(|| {
            errors
                .iter()
                .map(|(tag, error)| {
                    trf(
                        "Could not remove {build}: {error}. Try again in Settings → Storage.",
                        &[("build", &self.version_label(tag)), ("error", error)],
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        });
        self.refresh_open_storage_builds();
    }

    pub(super) fn storage_build_imported(&mut self, result: Result<String, String>) {
        self.importing_build = false;
        self.installed = backend::installed();
        self.storage_builds.import_notice = Some(
            result.map(|_| tr("Build imported. To play it, choose it beside Play on Home.").into()),
        );
        self.refresh_open_storage_builds();
    }

    fn choose_storage_import(&mut self, cx: &mut Context<Self>) {
        let picking = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr("Import a game build").into()),
        });
        cx.spawn(async move |this, cx| {
            let choice = picking.await;
            let _ = this.update(cx, |this, cx| {
                let path = match choice {
                    Ok(Ok(Some(paths))) => paths.into_iter().next(),
                    Ok(Ok(None)) | Err(_) => None,
                    Ok(Err(error)) => {
                        this.storage_builds.import_notice = Some(Err(trf(
                            "Could not open the file picker: {error}",
                            &[("error", &error.to_string())],
                        )));
                        cx.notify();
                        return;
                    }
                };
                let Some(path) = path else { return };
                let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned();
                if !this.can_change_builds()
                    || !build_import::can_import(&this.installed, this.settings.selected.as_deref())
                {
                    this.storage_builds.import_notice = Some(Err(trf(
                        "Could not import {build}. Wait for any game or build operation to finish, then make sure a game build is installed and try again.",
                        &[("build", &name)],
                    )));
                    cx.notify();
                    return;
                }
                this.importing_build = true;
                this.storage_builds.import_notice = None;
                let tx = this.tx.clone();
                std::thread::spawn(move || {
                    let result = build_import::install(&path, None).map_err(|error| trf(
                        "Could not import {build}: {error}. Check the file and try again.",
                        &[("build", &name), ("error", &error)],
                    ));
                    let _ = tx.send(Msg::Imported(result));
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn storage_confirmation(
        &self,
        title: String,
        about: String,
        action: &str,
        cx: &mut Context<Self>,
    ) -> Div {
        let remove = danger(button("storage-build-confirm", action.to_owned()));
        row(
            title,
            about,
            div()
                .child(
                    button("storage-build-cancel", tr("Cancel")).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.storage_builds.confirm = None;
                            cx.notify();
                        },
                    )),
                )
                .child(if self.can_change_builds() {
                    remove.on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.remove_storage_builds(cx)),
                    )
                } else {
                    off(remove)
                }),
        )
    }

    pub(super) fn storage_build_rows(&self, cx: &mut Context<Self>) -> Div {
        let rows = self.storage_builds.rows.as_deref().unwrap_or_default();
        let old: Vec<String> = rows
            .iter()
            .filter(|b| b.old_download && b.in_use.is_none())
            .map(|b| b.tag.clone())
            .collect();
        let bulk = matches!(self.storage_builds.confirm, Some(Confirm::Old(_)));
        let available = self.can_change_builds();
        let size = |bytes| storage::format_bytes(bytes);
        let on_disk = if self.storage_builds.rows.is_none() {
            tr("Counting…").to_string()
        } else if rows.is_empty() {
            tr("No game builds installed").to_string()
        } else {
            trf(
                "{size} on disk",
                &[("size", &size(rows.iter().map(|b| b.bytes).sum()))],
            )
        };
        let cleanup = (!bulk && !old.is_empty()).then(|| {
            let clean = button("storage-build-clean", tr("Clean up old builds…"));
            if available {
                clean.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.storage_builds.confirm = Some(Confirm::Old(old.clone()));
                    this.storage_builds.remove_error = None;
                    cx.notify();
                }))
            } else {
                off(clean)
            }
        });
        let mut card = vec![row(
            tr("Installed game builds"),
            on_disk,
            div().children(cleanup),
        )];
        if self.storage_builds.rows.as_ref().is_some_and(Vec::is_empty) {
            card.push(hint(tr(
                "Choose a version beside Play on Home to install a build.",
            )));
        }
        if let Some(Confirm::Old(tags)) = &self.storage_builds.confirm {
            let bytes = rows
                .iter()
                .filter(|b| tags.contains(&b.tag))
                .map(|b| b.bytes)
                .sum();
            card.push(self.storage_confirmation(
                if tags.len() == 1 {
                    tr("Remove 1 old build?").to_string()
                } else {
                    trf(
                        "Remove {count} old builds?",
                        &[("count", &tags.len().to_string())],
                    )
                },
                trf(
                    "Frees up to {size}. Builds in use and your worlds and mods stay.",
                    &[("size", &size(bytes))],
                ),
                tr(if tags.len() == 1 {
                    "Remove old build"
                } else {
                    "Remove old builds"
                }),
                cx,
            ));
        }
        for build in rows {
            let name = self.version_label(&build.tag);
            let confirming = matches!(&self.storage_builds.confirm, Some(Confirm::One(tag)) if tag == &build.tag);
            let detail = match &build.in_use {
                Some(InUse::Profile(name)) => trf("Used by {profile}", &[("profile", name)]),
                Some(InUse::Current) => tr("Current build").into(),
                Some(InUse::Running) => tr("Running").into(),
                Some(InUse::NativeBuild) => tr("Used by a build with mods").into(),
                Some(InUse::ProfileReference) => tr("Kept for a profile").into(),
                None if self.removing.contains(&build.tag) => tr("Removing…").into(),
                None if build_import::is_imported(&build.tag) => tr("Unsigned import").into(),
                None if sunrise_launcher::developer_install::is_developer(&build.tag) => {
                    tr("Unsigned developer build").into()
                }
                None if sunrise_launcher::nightly::date_of(&build.tag).is_some() => {
                    tr("Nightly").into()
                }
                None => String::new(),
            };
            let subtitle = if detail.is_empty() {
                size(build.bytes)
            } else {
                trf(
                    "{size} · {detail}",
                    &[("size", &size(build.bytes)), ("detail", &detail)],
                )
            };
            let remove =
                (build.in_use.is_none() && !confirming && !self.removing.contains(&build.tag))
                    .then(|| {
                        let tag = build.tag.clone();
                        let remove = button(
                            gpui::SharedString::from(format!("storage-remove-{tag}")),
                            tr("Remove…"),
                        );
                        if available {
                            remove.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.storage_builds.confirm = Some(Confirm::One(tag.clone()));
                                this.storage_builds.remove_error = None;
                                cx.notify();
                            }))
                        } else {
                            off(remove)
                        }
                    });
            card.push(row(name.clone(), subtitle, div().children(remove)));
            if confirming {
                card.push(self.storage_confirmation(
                    trf("Remove {build}?", &[("build", &name)]),
                    trf(
                        "Frees up to {size}. Your worlds and mods stay.",
                        &[("size", &size(build.bytes))],
                    ),
                    tr("Remove build"),
                    cx,
                ));
            }
        }
        card.extend(self.storage_builds.remove_error.clone().map(warning));
        compact_group(card)
    }

    pub(super) fn storage_build_options(&self, cx: &mut Context<Self>) -> Div {
        let base = build_import::can_import(&self.installed, self.settings.selected.as_deref());
        let choose = button("storage-build-import", tr("Choose file…"));
        let mut rows = vec![row(
            tr("Import a game build"),
            tr(if base {
                "Use a local executable or archive with files from an installed build."
            } else {
                "Install a game build first. An import reuses its game files."
            }),
            div().child(if self.importing_build {
                off(button("storage-build-importing", tr("Importing…")))
            } else if base && self.can_change_builds() {
                choose.on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.choose_storage_import(cx)),
                )
            } else {
                off(choose)
            }),
        )];
        if let Some(note) = &self.storage_builds.import_notice {
            rows.push(match note {
                Ok(note) => hint(note.clone()),
                Err(error) => warning(error.clone()),
            });
        }
        rows.push(row(
            tr("Remove old game builds automatically"),
            tr("After a download, remove only old automatic downloads that are not in use."),
            div().child(
                switch("alt-prune", self.settings.auto_prune()).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.settings.prune_old_versions = Some(!this.settings.auto_prune());
                        this.save_settings();
                        cx.notify();
                    },
                )),
            ),
        ));
        compact_group(rows)
    }
}
