//! Read-only inspection and explicit update offers. Only the shared installer can publish.
use super::*;
use crate::mods_page::install::SourceTarget;
use std::collections::BTreeMap;
use sunrise_launcher::browser::sunrise::{PreparedSource, SourceError, SourceRequest};
use sunrise_launcher::github_mods::{Origin, SourceTrack};
use sunrise_launcher::native_mods::Cancellation;

pub(in crate::mods) enum Event {
    Catalogue(Catalogue),
    Detail(u64, Box<Inspection>),
    Image(sources::ReadmeImage, Option<PathBuf>),
    Updates(Box<CheckedUpdates>),
}

pub(in crate::mods) struct Open {
    pub(super) source: Source,
    pub(super) scope: OpenScope,
    pub(super) request: u64,
    pub(super) inspection: Option<Inspection>,
}

pub(in crate::mods) struct Inspection {
    pub(super) data: sources::Details,
    pub(super) candidate: Result<(SourceTarget, PreparedSource), SourceError>,
}

pub(in crate::mods) struct CheckedUpdates {
    scope: OpenScope,
    target: Option<SourceTarget>,
    candidates: BTreeMap<String, (Origin, PreparedSource)>,
    issues: BTreeMap<String, SourceError>,
    notice: Option<String>,
}

#[derive(Default)]
pub(in crate::mods) struct Work {
    pub(super) images: HashMap<sources::ReadmeImage, Option<Option<PathBuf>>>,
    checking: bool,
    updates: Option<CheckedUpdates>,
    pub(super) log: Option<(String, std::sync::Arc<str>)>,
}

impl Launcher {
    pub(in crate::mods) fn nightly_event(&mut self, event: Event) {
        let state = &mut self.ui.mods.nightly;
        match event {
            Event::Catalogue(catalogue) => {
                state.reading = false;
                if state
                    .catalogue
                    .as_ref()
                    .is_none_or(|old| catalogue.checked_at >= old.checked_at)
                {
                    state.catalogue = Some(catalogue);
                }
            }
            Event::Detail(request, detail) => {
                if let Some(open) = state.open.as_mut().filter(|open| open.request == request) {
                    open.inspection = Some(*detail);
                }
            }
            Event::Image(image, path) => {
                state.work.images.insert(image, Some(path));
            }
            Event::Updates(checked) => {
                state.work.checking = false;
                state.work.updates = Some(*checked);
            }
        }
    }

    pub(in crate::mods) fn refresh_nightly_scope(&mut self) {
        let state = &self.ui.mods.nightly;
        if state.open.is_none() && state.work.updates.is_none() {
            return;
        }
        let scope = self.mods_page_scope();
        let state = &mut self.ui.mods.nightly;
        if state.open.as_ref().is_some_and(|open| open.scope != scope) {
            state.open = None;
            if self.ui.page == Page::Project {
                self.ui.page = Page::Discover;
            }
        }
        if state
            .work
            .updates
            .as_ref()
            .is_some_and(|updates| updates.scope != scope)
        {
            state.work.updates = None;
        }
    }

    pub(in crate::mods) fn inspect_nightly(&mut self, source: Source, refresh: bool) {
        let scope = self.mods_page_scope();
        let target = self.source_target();
        self.ui.page = Page::Project;
        self.ui.mods.open = None;
        let state = &mut self.ui.mods.nightly;
        state.detail_request += 1;
        let request = state.detail_request;
        state.open = Some(Open {
            source: source.clone(),
            scope,
            request,
            inspection: None,
        });
        if refresh {
            self.read_nightly_catalogue(true);
        }
        let tx = self.ui.mods.tx.clone();
        std::thread::spawn(move || {
            let data = sources::details(&source, refresh);
            let candidate = (|| {
                if let Some(error) = &data.error {
                    return Err(SourceError::Github(error.clone()));
                }
                let pin = data
                    .commit
                    .clone()
                    .ok_or(SourceError::Github(sources::Failure::NoManifest))?;
                let target = target.map_err(SourceError::Unproved)?;
                let candidate = sunrise::prepare_source(
                    target.instance(),
                    target.identity(),
                    target.base(),
                    SourceRequest::Add(pin),
                    &Cancellation::default(),
                )?;
                Ok((target, candidate))
            })();
            if let Err(error) = &candidate {
                eprintln!("Nightly source preflight ({}): {error}", source.id());
            }
            let _ = tx.send(super::super::Event::Nightly(Event::Detail(
                request,
                Box::new(Inspection { data, candidate }),
            )));
        });
    }

    pub(super) fn request_nightly_image(&mut self, image: sources::ReadmeImage) {
        use std::collections::hash_map::Entry;
        let Entry::Vacant(entry) = self.ui.mods.nightly.work.images.entry(image.clone()) else {
            return;
        };
        entry.insert(None);
        let tx = self.ui.mods.tx.clone();
        std::thread::spawn(move || {
            let path = sources::readme_image(&image).ok();
            let _ = tx.send(super::super::Event::Nightly(Event::Image(image, path)));
        });
    }

    pub(super) fn build_nightly(&mut self) {
        if !self.nightly_catalogue_current() || !self.guard() {
            return;
        }
        let scope = self.mods_page_scope();
        let state = &mut self.ui.mods.nightly;
        let Some(open) = state.open.as_mut().filter(|open| open.scope == scope) else {
            return;
        };
        if !state
            .catalogue
            .as_ref()
            .is_some_and(|list| list.sources.contains(&open.source))
        {
            self.ui.mods.notice = Some(Err(tr(
                "This source listing changed. Open it again from Discover.",
            )
            .into()));
            return;
        }
        let Some(inspection) = open.inspection.as_mut() else {
            return;
        };
        if inspection.candidate.is_err() {
            return;
        }
        let candidate = std::mem::replace(&mut inspection.candidate, Err(SourceError::Cancelled));
        if let Ok((target, candidate)) = candidate {
            if let Err(error) = self.queue_sources(target, vec![candidate]) {
                self.ui.mods.notice = Some(Err(error));
            }
        }
    }

    pub(super) fn nightly_catalogue_current(&self) -> bool {
        self.ui.mods.nightly.catalogue.as_ref().is_some_and(|list| {
            list.saved_at.is_some()
                && list.error.is_none()
                && list.checked_at.elapsed().is_ok_and(|age| age < FRESH)
        })
    }

    pub(in crate::mods) fn check_source_updates(&mut self) {
        if self.ui.mods.nightly.work.checking || !self.guard() {
            return;
        }
        let scope = self.mods_page_scope();
        let target = match self.source_target() {
            Ok(target) => target,
            Err(error) => {
                self.ui.mods.notice = Some(Err(error));
                return;
            }
        };
        let origins: Vec<Origin> = self
            .ui
            .mods
            .snapshot
            .as_ref()
            .into_iter()
            .flat_map(|snapshot| &snapshot.rows.local)
            .filter_map(|local| local.github.as_ref())
            .filter(|origin| {
                origin
                    .source
                    .as_ref()
                    .is_some_and(|source| !matches!(source.track, SourceTrack::Pinned))
            })
            .cloned()
            .collect();
        self.ui.mods.nightly.work.checking = true;
        self.ui.mods.nightly.work.updates = None;
        let tx = self.ui.mods.tx.clone();
        std::thread::spawn(move || {
            let mut checked = CheckedUpdates {
                scope,
                target: None,
                candidates: BTreeMap::new(),
                issues: BTreeMap::new(),
                notice: None,
            };
            let catalogue = sources::catalogue(false);
            if catalogue.error.is_some() {
                checked.notice = Some(tr("No source updates were checked while offline.").into());
            } else {
                let cancellation = Cancellation::default();
                let mut requested = Vec::new();
                let mut predecessors = Vec::new();
                for origin in origins {
                    match source_for_update(&origin) {
                        Ok(Some(source)) => {
                            requested.push(source);
                            predecessors.push(origin);
                        }
                        Ok(None) => {}
                        Err(error) => {
                            checked.issues.insert(origin.id, SourceError::Github(error));
                        }
                    }
                }
                for (origin, pin) in predecessors
                    .into_iter()
                    .zip(sources::resolve_batch(&requested))
                {
                    let pin = match pin {
                        Ok(pin) if pin.sha() == origin.commit => continue,
                        Ok(pin) => pin,
                        Err(error) => {
                            checked.issues.insert(origin.id, SourceError::Github(error));
                            continue;
                        }
                    };
                    match sunrise::prepare_source(
                        target.instance(),
                        target.identity(),
                        target.base(),
                        SourceRequest::Update {
                            pin,
                            predecessor: Box::new(origin.clone()),
                        },
                        &cancellation,
                    ) {
                        Ok(prepared) => {
                            checked
                                .candidates
                                .insert(origin.id.clone(), (origin, prepared));
                        }
                        Err(error) => {
                            checked.issues.insert(origin.id, error);
                        }
                    }
                }
                checked.notice = Some(
                    if checked.candidates.is_empty() && checked.issues.is_empty() {
                        tr("No source updates available.").into()
                    } else {
                        tr("Finished checking for source updates.").into()
                    },
                );
                checked.target = Some(target);
            }
            for (id, error) in &checked.issues {
                eprintln!("Nightly source update preflight ({id}): {error}");
            }
            let _ = tx.send(super::super::Event::Nightly(Event::Catalogue(catalogue)));
            let _ = tx.send(super::super::Event::Nightly(Event::Updates(Box::new(
                checked,
            ))));
        });
    }

    pub(in crate::mods) fn source_updates_checking(&self) -> bool {
        self.ui.mods.nightly.work.checking
    }

    pub(in crate::mods) fn source_update_version(&self, origin: &Origin) -> Option<&str> {
        if !self.nightly_catalogue_current() {
            return None;
        }
        let (predecessor, prepared) = self.source_updates_here()?.candidates.get(&origin.id)?;
        (predecessor == origin).then_some(prepared.origin().version.as_str())
    }

    pub(in crate::mods) fn source_update_issue(&self, id: &str) -> Option<&SourceError> {
        self.source_updates_here()?.issues.get(id)
    }

    pub(in crate::mods) fn source_update_notice(&self) -> Option<&str> {
        self.source_updates_here()?.notice.as_deref()
    }

    fn source_updates_here(&self) -> Option<&CheckedUpdates> {
        self.ui.mods.nightly.work.updates.as_ref()
    }

    pub(in crate::mods) fn submit_source_update(&mut self, id: &str) {
        if !self.guard() {
            return;
        }
        if let Some((target, sources)) = self.take_source_updates(Some(id)) {
            if let Err(error) = self.queue_sources(target, sources) {
                self.ui.mods.notice = Some(Err(error));
            }
        }
    }

    pub(in crate::mods) fn submit_mod_update_all(
        &mut self,
        displayed_names: BTreeMap<String, String>,
    ) {
        if !self.guard() {
            return;
        }
        let registry: Vec<_> = self
            .rows()
            .into_iter()
            .filter(model::bulk_updatable)
            .map(|row| row.id)
            .collect();
        let Some((target, sources)) = self.take_source_updates(None) else {
            if !registry.is_empty() {
                self.ask_update(false);
            }
            return;
        };
        self.ui.mods.notice = None;
        self.mods.message = None;
        let result = if registry.is_empty() {
            self.queue_sources(target, sources)
        } else {
            self.queue_all_mod_updates(target, registry, displayed_names, sources)
        };
        if let Err(error) = result {
            self.ui.mods.notice = Some(Err(error));
        }
    }

    fn take_source_updates(
        &mut self,
        id: Option<&str>,
    ) -> Option<(SourceTarget, Vec<PreparedSource>)> {
        if !self.nightly_catalogue_current() {
            return None;
        }
        let scope = self.mods_page_scope();
        let installed = self
            .ui
            .mods
            .snapshot
            .as_ref()
            .into_iter()
            .flat_map(|snapshot| &snapshot.rows.local)
            .filter_map(|local| local.github.as_ref());
        let checked = self
            .ui
            .mods
            .nightly
            .work
            .updates
            .as_mut()
            .filter(|updates| updates.scope == scope)?;
        let target = checked.target.clone()?;
        let wanted: Vec<_> = installed
            .filter(|origin| id.is_none_or(|id| id == origin.id))
            .filter(|origin| {
                checked
                    .candidates
                    .get(&origin.id)
                    .is_some_and(|(predecessor, _)| predecessor == *origin)
            })
            .map(|origin| origin.id.clone())
            .collect();
        let inputs: Vec<_> = wanted
            .into_iter()
            .filter_map(|id| checked.candidates.remove(&id))
            .map(|(_, prepared)| prepared)
            .collect();
        (!inputs.is_empty()).then_some((target, inputs))
    }
}

fn source_for_update(predecessor: &Origin) -> Result<Option<Source>, sources::Failure> {
    let Some(source) = predecessor.source.as_ref() else {
        return Ok(None);
    };
    let tracker = match &source.track {
        SourceTrack::Branch(branch) => sources::Tracker::Branch(branch.clone()),
        SourceTrack::Release => sources::Tracker::Release,
        SourceTrack::Pinned => return Ok(None),
    };
    Source::for_origin(
        &predecessor.id,
        &format!("{}/{}", predecessor.owner, predecessor.repo),
        source.repo_id,
        &source.manifest,
        tracker,
    )
    .map(Some)
}
