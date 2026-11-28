//! The installed builds and import cards in Settings → Storage.
use crate::frame::button;
use crate::mods::switch;
use crate::settings::{danger, group, hint, off, row, warning};
use std::sync::Arc;
use sunrise_launcher::backend::{
    version_storage::{self, Build, InUse, SizeUnavailable},
    NightlyReceiptWarning,
};

use super::*;

#[derive(Default)]
pub(super) struct View {
    rows: Option<Vec<Build>>,
    generation: u64,
    /// Stops the worker still adding up sizes, when the page closes or a newer one starts.
    cancel: Option<storage::Cancel>,
    pub(super) confirm: Option<Confirm>,
    remove_error: Option<String>,
    import_notice: Option<Result<String, String>>,
    receipts: HashMap<String, ReceiptNotice>,
}

struct ReceiptNotice {
    warning: Arc<NightlyReceiptWarning>,
    retrying: bool,
}

pub(super) enum Confirm {
    One(String),
    Old(Vec<String>),
}

impl View {
    pub(crate) fn stop_count(&mut self) {
        self.generation += 1;
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel();
        }
    }

    pub(crate) fn start_count(
        &mut self,
        location_unavailable: bool,
    ) -> Option<(u64, storage::Cancel)> {
        self.stop_count();
        self.rows = None;
        if location_unavailable {
            return None;
        }
        let cancel = storage::Cancel::default();
        self.cancel = Some(cancel.clone());
        Some((self.generation, cancel))
    }

    pub(crate) fn accept_rows(&mut self, generation: u64, mut rows: Vec<Build>) {
        if self.generation == generation {
            rows.retain(|row| row.size_unavailable != Some(SizeUnavailable::Vanished));
            self.rows = Some(rows);
        }
    }

    #[cfg(test)]
    pub(crate) fn has_rows(&self) -> bool {
        self.rows.is_some()
    }
}

fn count_builds(releases: &[Release], cancel: &storage::Cancel, mut publish: impl FnMut(&[Build])) {
    let Some(mut rows) = version_storage::inventory_until(releases, cancel) else {
        return;
    };
    publish(&rows);
    version_storage::measure(&mut rows, cancel, publish);
}

fn build_size(build: &Build) -> String {
    match (build.bytes, build.size_unavailable) {
        (Some(bytes), _) => storage::format_bytes(bytes),
        (None, Some(_)) => tr("Not counted").into(),
        (None, None) => tr("Counting…").into(),
    }
}

/// Total bytes for the confirmed old-build tags only when every confirmed tag is present in
/// `rows` and has a measured size. Missing or unmeasured tags return `None` so the confirmation
/// never quotes a partial or zero total as complete.
fn bulk_remove_bytes(tags: &[String], rows: &[Build]) -> Option<u64> {
    if tags.is_empty() {
        return None;
    }
    let mut total = 0u64;
    for tag in tags {
        let bytes = rows.iter().find(|row| &row.tag == tag)?.bytes?;
        total = total.saturating_add(bytes);
    }
    Some(total)
}

impl Launcher {
    pub(super) fn refresh_storage_builds(&mut self) {
        let Some((generation, cancel)) = self.storage_builds.start_count(self.location_unavailable)
        else {
            return;
        };
        let releases = self.game_releases().to_vec();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            // The builds show at once, each still counting, and take their sizes one by one.
            count_builds(&releases, &cancel, |rows| {
                let _ = tx.send(Msg::BuildInventory(generation, rows.to_vec()));
            });
        });
    }

    pub(super) fn refresh_open_storage_builds(&mut self) {
        if self.ui.page == frame::Page::Settings
            && self.ui.settings.section == settings::Section::Storage
        {
            self.refresh_storage_builds();
            self.resume_storage_count();
        } else {
            self.stop_storage_work();
        }
    }

    /// Storage and installed-builds messages routed from `launcher_messages::drain`.
    pub(super) fn on_storage_message(&mut self, msg: Msg) {
        match msg {
            Msg::Removed(errors) => self.storage_builds_removed(errors),
            Msg::BuildInventory(generation, rows) => self.storage_builds_arrived(generation, rows),
            Msg::Storage(cancel, result) => self.storage_arrived(cancel, result),
            Msg::Imported(result) => self.storage_build_imported(result),
            Msg::Pruned(errors) => self.storage_builds_pruned(errors),
            _ => unreachable!("launcher_messages::drain only routes storage messages here"),
        }
    }

    pub(super) fn storage_builds_arrived(&mut self, generation: u64, rows: Vec<Build>) {
        self.storage_builds.accept_rows(generation, rows);
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
        if self.ui.page == frame::Page::Settings
            && self.ui.settings.section == settings::Section::Storage
        {
            self.refresh_storage(None, false);
        }
    }

    pub(super) fn storage_builds_pruned(&mut self, errors: Vec<(String, String)>) {
        self.refresh_installed();
        self.storage_builds
            .receipts
            .retain(|tag, _| self.installed.contains(tag));
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
    }

    pub(super) fn storage_build_imported(&mut self, result: Result<String, String>) {
        self.importing_build = false;
        self.refresh_installed();
        self.storage_builds.import_notice = Some(
            result.map(|_| tr("Build imported. To play it, choose it beside Play on Home.").into()),
        );
    }

    pub(crate) fn record_nightly_receipt_outcome(
        &mut self,
        tag: &str,
        result: &Result<backend::InstallOutcome, String>,
    ) -> Option<String> {
        let outcome = result.as_ref().ok()?;
        let receipts = &mut self.storage_builds.receipts;
        if let Some(warning) = outcome.receipt_warning() {
            sunrise_launcher::elog!("{}", warning.detail());
            receipts.insert(
                tag.to_owned(),
                ReceiptNotice {
                    warning: Arc::clone(warning),
                    retrying: false,
                },
            );
            Some(warning.message())
        } else {
            receipts.remove(tag);
            None
        }
    }

    pub(crate) fn finish_nightly_receipt_retry(
        &mut self,
        warning: &Arc<NightlyReceiptWarning>,
        result: Result<(), String>,
    ) {
        let receipts = &mut self.storage_builds.receipts;
        let Some(notice) = receipts
            .get_mut(warning.tag())
            .filter(|notice| Arc::ptr_eq(&notice.warning, warning))
        else {
            return;
        };
        if let Err(error) = result {
            sunrise_launcher::elog!(
                "Could not retry the rolling receipt for {}: {error}",
                warning.tag()
            );
            notice.retrying = false;
        } else {
            receipts.remove(warning.tag());
        }
    }

    fn nightly_receipt_notice(&self, tag: &str, cx: &mut Context<Self>) -> Option<Div> {
        let notice = self.storage_builds.receipts.get(tag)?;
        let warning = Arc::clone(&notice.warning);
        let retry = button(
            SharedString::from(format!("retry-nightly-receipt-{tag}")),
            if notice.retrying {
                tr("Trying again…")
            } else {
                tr("Try again")
            },
        )
        .flex_none();
        let retry = if !notice.retrying && self.can_repair_now(warning.tag()) {
            retry.on_click(cx.listener(move |this, _, _, cx| {
                if !this.can_repair_now(warning.tag()) {
                    return;
                }
                let Some(notice) = this
                    .storage_builds
                    .receipts
                    .get_mut(warning.tag())
                    .filter(|notice| !notice.retrying && Arc::ptr_eq(&notice.warning, &warning))
                else {
                    return;
                };
                notice.retrying = true;
                let (tx, warning) = (this.tx.clone(), Arc::clone(&warning));
                std::thread::spawn(move || {
                    let result = warning.retry();
                    let _ = tx.send(Msg::NightlyReceiptRetried(warning, result));
                });
                cx.notify();
            }))
        } else {
            retry.opacity(0.45).cursor_default().tab_stop(false)
        };
        Some(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .min_w_0()
                        .text_color(gpui::rgb(crate::paint::palette().danger))
                        .text_size(px(tokens::SMALL))
                        .child(notice.warning.message()),
                )
                .child(retry),
        )
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
            tr(if self.location_unavailable {
                "Game storage is unavailable"
            } else {
                "Counting…"
            })
            .to_string()
        } else if rows.is_empty() {
            tr("No game builds installed").to_string()
        } else if let Some(bytes) = rows.iter().map(|b| b.bytes).sum::<Option<u64>>() {
            trf("{size} on disk", &[("size", &size(bytes))])
        } else if rows.iter().any(|b| b.size_unavailable.is_some()) {
            tr("Not counted").to_string()
        } else {
            tr("Counting…").to_string()
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
            // The sizes are offered once every confirmed tag is present and counted; Remove never
            // waits for them.
            let bytes = bulk_remove_bytes(tags, rows);
            card.push(self.storage_confirmation(
                if tags.len() == 1 {
                    tr("Remove 1 old build?").to_string()
                } else {
                    trf(
                        "Remove {count} old builds?",
                        &[("count", &tags.len().to_string())],
                    )
                },
                match bytes {
                    Some(bytes) => trf(
                        "Frees up to {size}. Builds in use and your worlds and mods stay.",
                        &[("size", &size(bytes))],
                    ),
                    None => {
                        tr("Frees space. Builds in use and your worlds and mods stay.").to_string()
                    }
                },
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
                None if backend::dev_date(&build.tag).is_some() => tr("Dev").into(),
                None => String::new(),
            };
            let counted = build_size(build);
            let subtitle = if detail.is_empty() {
                counted
            } else {
                trf(
                    "{size} · {detail}",
                    &[("size", &counted), ("detail", &detail)],
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
            let line = row(name.clone(), subtitle, div().children(remove));
            card.push(match self.nightly_receipt_notice(&build.tag, cx) {
                Some(notice) => div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(line)
                    .child(notice),
                None => line,
            });
            if confirming {
                card.push(self.storage_confirmation(
                    trf("Remove {build}?", &[("build", &name)]),
                    match build.bytes {
                        Some(bytes) => trf(
                            "Frees up to {size}. Your worlds and mods stay.",
                            &[("size", &size(bytes))],
                        ),
                        None => tr("Frees space. Your worlds and mods stay.").to_string(),
                    },
                    tr("Remove build"),
                    cx,
                ));
            }
        }
        card.extend(self.storage_builds.remove_error.clone().map(warning));
        group(card)
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
        group(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_build_inventory_rejects_queued_rows() {
        let mut view = View::default();
        let (generation, cancel) = view.start_count(false).unwrap();
        view.stop_count();
        assert!(cancel.is_cancelled());
        view.accept_rows(generation, Vec::new());
        assert!(view.rows.is_none());
    }

    #[test]
    fn reopening_unavailable_build_inventory_discards_previous_rows() {
        let mut view = View::default();
        let (generation, _) = view.start_count(false).unwrap();
        view.accept_rows(generation, Vec::new());
        assert!(view.rows.is_some());
        assert!(view.start_count(true).is_none());
        assert!(view.rows.is_none());
        view.accept_rows(generation, Vec::new());
        assert!(view.rows.is_none());
    }

    #[test]
    fn cancelled_build_discovery_publishes_nothing() {
        let cancel = storage::Cancel::default();
        cancel.cancel();
        count_builds(&[], &cancel, |_| {
            panic!("cancelled discovery published rows")
        });
    }

    #[test]
    fn unavailable_build_sizes_are_not_zero_and_vanished_builds_disappear() {
        let mut view = View::default();
        let (generation, _) = view.start_count(false).unwrap();
        let mut build = Build {
            tag: "unreadable".into(),
            bytes: None,
            size_unavailable: None,
            in_use: None,
            old_download: false,
        };
        assert_eq!(build_size(&build), "Counting…");
        build.size_unavailable = Some(SizeUnavailable::Unreadable);
        let vanished = Build {
            tag: "vanished".into(),
            size_unavailable: Some(SizeUnavailable::Vanished),
            ..build.clone()
        };
        view.accept_rows(generation, vec![build, vanished]);
        let rows = view.rows.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].tag, "unreadable");
        assert_eq!(build_size(&rows[0]), "Not counted");
    }

    #[test]
    fn bulk_remove_confirmation_requires_every_confirmed_tag_to_be_present_and_measured() {
        let row = |tag: &str, bytes: Option<u64>, size_unavailable| Build {
            tag: tag.into(),
            bytes,
            size_unavailable,
            in_use: None,
            old_download: true,
        };
        let tags = vec!["26.3-0.1.1".to_string(), "26.3-0.1.2".to_string()];
        // Empty rows (for example while reloading) must not quote 0 bytes.
        assert_eq!(bulk_remove_bytes(&tags, &[]), None);
        assert_eq!(
            bulk_remove_bytes(&[], &[row("26.3-0.1.1", Some(100), None)]),
            None
        );
        // A missing confirmed tag must not quote only the remaining measured rows.
        assert_eq!(
            bulk_remove_bytes(&tags, &[row("26.3-0.1.1", Some(100), None)]),
            None
        );
        // An unmeasured or unreadable confirmed tag omits the total.
        assert_eq!(
            bulk_remove_bytes(
                &tags,
                &[
                    row("26.3-0.1.1", Some(100), None),
                    row("26.3-0.1.2", None, None),
                ]
            ),
            None
        );
        assert_eq!(
            bulk_remove_bytes(
                &tags,
                &[
                    row("26.3-0.1.1", Some(100), None),
                    row("26.3-0.1.2", None, Some(SizeUnavailable::Unreadable)),
                ]
            ),
            None
        );
        // Once every confirmed tag is present and measured, sum only those tags.
        assert_eq!(
            bulk_remove_bytes(
                &tags,
                &[
                    row("26.3-0.1.1", Some(100), None),
                    row("26.3-0.1.2", Some(250), None),
                    row("26.3-0.1.3", None, None),
                ]
            ),
            Some(350)
        );
    }
}
