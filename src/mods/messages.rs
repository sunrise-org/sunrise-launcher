//! The Mods page's worker messages and the pump that applies them to the page state.

use super::*;

pub(super) enum Event {
    /// Installed rows read for the requested folder and fingerprint, off the window thread.
    Local(String, Box<InstalledRead>),
    Enabled(
        PathBuf,
        actions::EnableNotice,
        Result<actions::EnabledMods, String>,
    ),
    V2Catalog(
        Option<String>,
        Result<V2CatalogPayload, sunrise::v2::LoadError>,
    ),
    /// The verified snapshot on disk was read, ahead of the network read: redraw from it.
    V2Saved,
    /// This computer's fit check of a build finished.
    LocalFit(String, Result<(), String>),
    V2Page(u64, Result<Box<model::discover_v2::V2Page>, String>),
    Nightly(nightly::Event),
    Progress(String, f32),
    /// A pack search's answer, by the number of the search that asked.
    Packs(u64, Result<Vec<sunrise_launcher::browser::Project>, String>),
    /// The categories Modrinth offers for a pack scope; empty when they could not be read.
    PackCategories(pack_discover::Scope, Vec<String>),
    /// A pack's page, by its source and id.
    PackPage((SourceId, String), pack_project::PackPage),
    /// A resource or data pack install ended: what it said.
    PackDone(Result<String, String>),
    /// The batch was refused before changing anything.
    Refused(String),
    /// A batch ended: what it said, and which mods failed and why.
    Done(String, Vec<(String, String)>),
    /// "Check for update" asked the registry again: the mod's name, and the newer version found.
    Checked(String, Result<Option<String>, String>),
    /// The registry answered about ratings.
    Rating(rating::Answer),
}

impl Launcher {
    /// What the workers said. Nothing here touches a file.
    pub(crate) fn mods_drain(&mut self) -> bool {
        let mut changed = false;
        let mut local_fit_due = false;
        while let Ok(event) = self.mods_state.rx.try_recv() {
            changed = true;
            let m = &mut self.mods_state;
            match event {
                Event::V2Catalog(build, result) => {
                    if m.v2_catalog_build.as_ref() == Some(&build) {
                        m.v2_failure = result.as_ref().err().map(|error| match error {
                            sunrise::v2::LoadError::Fetch(_) => trf(
                                "Registry not reachable: {error}",
                                &[("error", &error.to_string())],
                            ),
                            _ => trf(
                                "Registry refused: {error}",
                                &[("error", &error.to_string())],
                            ),
                        });
                        if result.is_ok() {
                            m.home_mod_labels = None;
                        }
                        m.v2_catalog = match result {
                            Ok(payload) => V2Catalog::Ready {
                                cards: payload.cards.into(),
                                collections: payload.collections.into(),
                                categories: payload.categories.into(),
                                build_listed: payload.build_listed,
                                stale_warning: payload.stale_warning,
                                from_cache: payload.from_cache,
                            },
                            Err(error) => V2Catalog::Failed(error.to_string()),
                        };
                        local_fit_due = true;
                    }
                }
                Event::LocalFit(build, result) => {
                    if m.local_fit.build.as_ref() == Some(&build) {
                        m.local_fit.running = false;
                        m.local_fit.failed = result.err();
                    }
                    self.v2_cards_again();
                    local_fit_due = true;
                }
                Event::V2Saved => m.home_mod_labels = None,
                Event::Local(key, result) => {
                    if let Some(snap) = m.snapshot.as_mut().filter(|s| s.key == key) {
                        let ready = snap.finish(*result);
                        m.home_mod_labels = None;
                        if ready {
                            let rows: HashSet<String> =
                                self.rows().into_iter().map(|r| r.id).collect();
                            self.mods_state.selection.retain(|id| rows.contains(id));
                        }
                    }
                }
                Event::Enabled(home, notice, result) => {
                    self.enabled_mods(&home, notice, result);
                }
                Event::V2Page(request, page) => {
                    if page.is_ok() {
                        self.mods_state.v2_failure = None;
                    }
                    let scope = self.mods_page_scope();
                    if let Some(open) = self.mods_state.open.as_mut() {
                        open.finish_v2(request, &scope, page);
                    }
                }
                Event::Nightly(event) => self.nightly_event(event),
                Event::Progress(line, f) => m.busy = Some((line, f)),
                Event::Packs(seq, result) => self.pack_answer(seq, result),
                Event::PackCategories(scope, found) => self.pack_categories(scope, found),
                Event::PackPage(key, page) => self.pack_page_answer(key, page),
                Event::PackDone(result) => self.pack_installed(result),
                Event::Checked(title, found) => self.checked(title, found),
                Event::Rating(answer) => self.rating_answer(answer),
                Event::Refused(why) => {
                    m.busy = None;
                    m.notice = Some(Err(why));
                }
                Event::Done(said, failed) => {
                    m.busy = None;
                    m.errors.clear();
                    m.errors.extend(failed.iter().cloned());
                    m.notice = Some(match failed.is_empty() {
                        true => Ok(said),
                        false => Err(format!(
                            "{said} {}",
                            trf(
                                "Not done: {list}",
                                &[(
                                    "list",
                                    &failed
                                        .iter()
                                        .map(|(id, why)| format!("{id}: {why}"))
                                        .collect::<Vec<_>>()
                                        .join("; ")
                                )]
                            )
                        )),
                    });
                    // The list is read again by the next refresh (the files' times moved).
                }
            }
        }
        // A change the bar started from Home has nowhere on the page to say its result.
        changed |= self.say_bulk_result();
        if local_fit_due {
            // A catalog read while a check runs drew its cards without it: draw them again.
            if self.mods_state.local_fit.running {
                self.v2_cards_again();
            }
            self.start_local_fit();
        }
        changed
    }
}
