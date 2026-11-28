//! Read-only inspection and explicit update offers. Only the shared installer can publish.
use super::*;
use crate::mods_page::install::{SourcePicks, SourceTarget};
use std::collections::BTreeMap;
use sunrise_launcher::browser::sunrise::{PreparedSource, SourceError, SourceProof, SourceRequest};
use sunrise_launcher::github_mods::{Origin, SourceTrack};
use sunrise_launcher::native_mods::Cancellation;

pub(in crate::mods) enum Event {
    Catalogue(Catalogue),
    Detail(u64, Box<Inspection>),
    Icon(sources::SourceIcon, Option<PathBuf>),
    Updates(Box<CheckedUpdates>),
}

pub(in crate::mods) struct Open {
    pub(in crate::mods) source: Source,
    pub(super) from: Page,
    pub(super) scope: OpenScope,
    pub(super) request: u64,
    pub(super) inspection: Option<Inspection>,
    /// Add was pressed after a refusal: add the source once it has been read again.
    pub(super) add_when_read: bool,
}

pub(in crate::mods) struct Inspection {
    pub(super) data: sources::Details,
}

pub(in crate::mods) struct CheckedUpdates {
    scope: OpenScope,
    /// The installed mods' fingerprint the check ran against (`native_status::fingerprint`):
    /// a finding holds only while the mods are as they were.
    inputs: String,
    /// The installer's own proof (profile identity, base, active state, exact mod inputs hash
    /// and cached base build). Only offers proved against the exact same state carry over.
    proof: Option<SourceProof>,
    only: Option<String>,
    target: Option<SourceTarget>,
    candidates: BTreeMap<String, (Origin, PreparedSource)>,
    issues: BTreeMap<String, SourceError>,
    notice: Option<String>,
}

#[derive(Default)]
pub(in crate::mods) struct Work {
    pub(super) icons: HashMap<sources::SourceIcon, Option<PathBuf>>,
    icon_misses: sources::IconMisses,
    pub(super) release_pins: HashMap<String, sources::SourceCommit>,
    checking: bool,
    updates: Option<CheckedUpdates>,
    pub(super) log: Option<(String, std::sync::Arc<str>)>,
}

impl Launcher {
    pub(in crate::mods) fn nightly_event(&mut self, event: Event) {
        let mut add = false;
        let state = &mut self.mods_state.nightly;
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
                let Some(open) = state.open.as_mut().filter(|open| open.request == request) else {
                    return;
                };
                if matches!(open.source.tracker(), sources::Tracker::Release) {
                    let pins = &mut state.work.release_pins;
                    if let Some(pin) = &detail.data.commit {
                        pins.insert(open.source.id().to_owned(), pin.clone());
                    } else {
                        pins.remove(open.source.id());
                    }
                }
                open.inspection = Some(*detail);
                add = std::mem::take(&mut open.add_when_read);
            }
            Event::Icon(icon, path) => {
                state.work.icons.insert(icon, path);
            }
            Event::Updates(checked) => {
                state.work.checking = false;
                state.work.updates = Some(merge_checked(state.work.updates.take(), *checked));
            }
        }
        if add {
            self.build_nightly();
        }
    }

    /// Add after a refusal: the author may have fixed the source since, so it is read again
    /// first, and added at the commit that read finds.
    pub(in crate::mods) fn reread_and_add(&mut self, source: Source) {
        self.inspect_nightly(source, true);
        if let Some(open) = &mut self.mods_state.nightly.open {
            open.add_when_read = true;
        }
    }

    pub(in crate::mods) fn refresh_nightly_scope(&mut self) {
        let state = &self.mods_state.nightly;
        if state.open.is_none() && state.work.updates.is_none() {
            return;
        }
        let scope = self.mods_page_scope();
        let state = &mut self.mods_state.nightly;
        if state.open.as_ref().is_some_and(|open| open.scope != scope) {
            let from = state.open.take().map_or(Page::Discover, |open| open.from);
            if self.ui.page == Page::Project {
                self.ui.page = from;
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
        let from = self
            .mods_state
            .nightly
            .open
            .as_ref()
            .filter(|_| self.ui.page == Page::Project)
            .map_or(Page::Discover, |open| open.from);
        self.inspect_nightly_from(source, refresh, from, false);
    }

    pub(in crate::mods) fn inspect_nightly_from(
        &mut self,
        source: Source,
        refresh: bool,
        from: Page,
        installed: bool,
    ) {
        let scope = self.mods_page_scope();
        self.ui.page = Page::Project;
        // An installed mod's page keeps the open mod it was drawn from.
        if !installed {
            self.mods_state.open = None;
        }
        let state = &mut self.mods_state.nightly;
        if state.open.as_ref().is_none_or(|open| open.source != source) {
            crate::frame::scrolls("project").set_offset(gpui::point(px(0.), px(0.)));
        }
        state.detail_request += 1;
        let request = state.detail_request;
        state.collection = None;
        state.open = Some(Open {
            source: source.clone(),
            from,
            scope,
            request,
            inspection: None,
            add_when_read: false,
        });
        if refresh {
            self.read_nightly_catalogue(true);
        }
        let tx = self.mods_state.tx.clone();
        std::thread::spawn(move || {
            let data = sources::details(&source, refresh);
            let _ = tx.send(super::super::Event::Nightly(Event::Detail(
                request,
                Box::new(Inspection { data }),
            )));
        });
    }

    pub(super) fn request_nightly_icon(&mut self, icon: sources::SourceIcon) {
        use std::collections::hash_map::Entry;
        let Entry::Vacant(entry) = self.mods_state.nightly.work.icons.entry(icon.clone()) else {
            return;
        };
        entry.insert(None);
        let tx = self.mods_state.tx.clone();
        let misses = self.mods_state.nightly.work.icon_misses.clone();
        std::thread::spawn(move || {
            let path = icon.load(&misses);
            let _ = tx.send(super::super::Event::Nightly(Event::Icon(icon, path)));
        });
    }

    pub(super) fn nightly_busy(&self) -> bool {
        self.mods_locked().is_some()
            || (!self.mutating()
                && self
                    .mods_state
                    .snapshot
                    .as_ref()
                    .is_none_or(|snap| !snap.ready()))
    }

    pub(crate) fn source_picks_listed(&self, picks: &[Source]) -> bool {
        self.mods_state
            .nightly
            .catalogue
            .as_ref()
            .is_some_and(|list| picks.iter().all(|source| list.sources.contains(source)))
    }

    pub(super) fn build_nightly(&mut self) {
        if !self.nightly_catalogue_current() || self.nightly_busy() {
            return;
        }
        let scope = self.mods_page_scope();
        let Some(open) = self
            .mods_state
            .nightly
            .open
            .as_ref()
            .filter(|open| open.scope == scope)
        else {
            return;
        };
        if !self
            .discover_installed()
            .is_some_and(|rows| rows.source_presence(&open.source) == SourcePresence::Missing)
        {
            return;
        }
        if !self.source_picks_listed(std::slice::from_ref(&open.source)) {
            self.mods_state.notice = Some(Err(tr(
                "This source listing changed. Open it again from Discover.",
            )
            .into()));
            return;
        }
        if let Some(pin) = open
            .inspection
            .as_ref()
            .and_then(|inspection| inspection.data.checked_commit())
            .cloned()
        {
            self.add_sources(SourcePicks::One(Box::new(pin)));
        }
    }

    pub(crate) fn nightly_catalogue_current(&self) -> bool {
        self.mods_state
            .nightly
            .catalogue
            .as_ref()
            .is_some_and(|list| {
                list.saved_at.is_some()
                    && list.error.is_none()
                    && list.checked_at.elapsed().is_ok_and(|age| age < FRESH)
            })
    }

    pub(in crate::mods) fn check_source_updates(&mut self, only: Option<&Origin>) {
        if self.mods_state.nightly.work.checking || !self.guard() {
            return;
        }
        let scope = self.mods_page_scope();
        let target = match self.source_target() {
            Ok(target) => target,
            Err(error) => {
                self.mods_state.notice = Some(Err(error));
                return;
            }
        };
        let Some(rows) = self.discover_installed() else {
            return;
        };
        let Some(inputs) = self.mods_state.snapshot.as_ref().map(|s| s.key.clone()) else {
            return;
        };
        let origins = tracked_origins(&rows.local, only);
        if origins.is_empty() {
            return;
        }
        let only = only.map(|origin| origin.id.clone());
        // The last findings stay until this check answers (`merge_checked`).
        self.mods_state.nightly.work.checking = true;
        let refresh_catalogue = !self.nightly_catalogue_current();
        let tx = self.mods_state.tx.clone();
        std::thread::spawn(move || {
            let proof =
                SourceProof::capture(target.profile_target(), target.identity(), target.base())
                    .ok();
            let mut checked = CheckedUpdates {
                scope,
                inputs,
                proof,
                only,
                target: None,
                candidates: BTreeMap::new(),
                issues: BTreeMap::new(),
                notice: None,
            };
            let catalogue = sources::catalogue(refresh_catalogue);
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
                        target.profile_target(),
                        target.identity(),
                        target.base(),
                        SourceRequest::Update {
                            pin,
                            predecessor: Box::new(origin.clone()),
                        },
                        &cancellation,
                    ) {
                        Ok(prepared) => {
                            if checked.proof.as_ref() == Some(prepared.proof()) {
                                checked
                                    .candidates
                                    .insert(origin.id.clone(), (origin, prepared));
                            } else {
                                checked.issues.insert(
                                    origin.id,
                                    SourceError::Changed(
                                        tr("The profile or game build changed. Check the source again; nothing was changed.")
                                            .into(),
                                    ),
                                );
                            }
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
        self.mods_state.nightly.work.checking
    }

    pub(in crate::mods) fn source_update_version(&self, origin: &Origin) -> Option<&str> {
        let (predecessor, prepared) = self.source_updates_here()?.candidates.get(&origin.id)?;
        (predecessor == origin).then_some(prepared.origin().version.as_str())
    }

    pub(in crate::mods) fn source_update_issue(&self, id: &str) -> Option<&SourceError> {
        self.source_updates_here()?.issues.get(id)
    }

    pub(in crate::mods) fn source_update_notice(&self, only: Option<&str>) -> Option<&str> {
        // A check under way says so itself; the last one's outcome would contradict it.
        if self.mods_state.nightly.work.checking {
            return None;
        }
        let checked = self.source_updates_here()?;
        if checked.only.as_deref() == only {
            if let Some(notice) = checked.notice.as_deref() {
                return Some(notice);
            }
        }
        // Last-known offers stay visible while the catalogue is stale; say so beside them.
        if !self.nightly_catalogue_current()
            && only.map_or(!checked.candidates.is_empty(), |id| {
                checked.candidates.contains_key(id)
            })
        {
            return Some(tr("No source updates were checked while offline."));
        }
        None
    }

    /// The last findings, while the installed mods are still the ones they were proved against.
    fn source_updates_here(&self) -> Option<&CheckedUpdates> {
        let now = &self.mods_state.snapshot.as_ref()?.key;
        self.mods_state
            .nightly
            .work
            .updates
            .as_ref()
            .filter(|checked| checked.inputs == *now)
    }

    pub(in crate::mods) fn submit_source_update(&mut self, id: &str) {
        if !self.guard() {
            return;
        }
        if let Some((target, sources)) = self.take_source_updates(Some(id)) {
            if let Err(error) = self.queue_sources(target, sources) {
                self.mods_state.notice = Some(Err(error));
            }
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
            .mods_state
            .snapshot
            .as_ref()
            .into_iter()
            .flat_map(|snapshot| &snapshot.rows.local)
            .filter_map(|local| local.github.as_ref())
            .filter(|origin| !origin.is_manual());
        let checked = self
            .mods_state
            .nightly
            .work
            .updates
            .as_mut()
            .filter(|updates| updates.scope == scope)
            .filter(|updates| {
                self.mods_state.snapshot.as_ref().map(|s| &s.key) == Some(&updates.inputs)
            })?;
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

/// What a check found, laid over the last one. Only findings proved against the exact same
/// profile identity, active state, mod inputs and cached base build carry over. A profile-wide
/// check replaces them; one project's check replaces only that project's offer or issue. A check
/// that couldn't run (offline) keeps them and only says so.
fn merge_checked(previous: Option<CheckedUpdates>, checked: CheckedUpdates) -> CheckedUpdates {
    let same_target = |earlier: &Option<SourceTarget>| match (earlier, &checked.target) {
        (Some(earlier), Some(now)) => {
            earlier.identity() == now.identity() && earlier.base() == now.base()
        }
        (_, None) => true,
        (None, Some(_)) => false,
    };
    let Some(now_proof) = checked.proof.as_ref() else {
        return checked;
    };
    let Some(mut merged) = previous.filter(|earlier| {
        earlier.scope == checked.scope
            && earlier.inputs == checked.inputs
            && earlier.proof.as_ref() == Some(now_proof)
            && same_target(&earlier.target)
    }) else {
        return checked;
    };
    merged
        .candidates
        .retain(|_, (_, prepared)| prepared.proof() == now_proof);
    if checked.target.is_some() {
        let Some(id) = checked.only.clone() else {
            return checked;
        };
        merged.candidates.remove(&id);
        merged.issues.remove(&id);
        merged.target = checked.target;
        merged.proof = checked.proof;
        merged.candidates.extend(checked.candidates);
        merged.issues.extend(checked.issues);
    }
    merged.only = checked.only;
    merged.notice = checked.notice;
    merged
}

fn tracked_origins(local: &[model::Local], only: Option<&Origin>) -> Vec<Origin> {
    local
        .iter()
        .filter_map(|local| local.github.as_ref())
        .filter(|origin| !origin.is_manual())
        .filter(|origin| only.is_none_or(|only| only == *origin))
        .filter(|origin| {
            origin
                .source
                .as_ref()
                .is_some_and(|source| !matches!(source.track, SourceTrack::Pinned))
        })
        .cloned()
        .collect()
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

#[cfg(test)]
mod tests;
