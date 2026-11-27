//! One visible registry install batch, and requests that arrived after it was sealed.
use super::*;
use crate::github_mods_page::{Feedback, GithubInput, GithubSelection, LocalInput};
use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
};
use sunrise_launcher::{
    browser::sunrise::{
        BuildNotice, ChangeImpact, InstallError, InstallHooks, InstallProgress, InstalledBatch,
        PreparedSource, ProviderChoiceNeeded, PublisherChangeQuestion, SourceDependency,
        SourceError, SourceHooks, V2Hooks, V2InstallOptions,
    },
    github_mods::{AddedFrom, Origin},
    managed_play::FirstBuild,
    native_mods::Cancellation,
    profile_txn::Identity,
};

use sunrise_launcher::browser::sunrise::sources::{self, Source, SourceCommit};

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum SourcePicks {
    One(Box<SourceCommit>),
    Collection {
        id: String,
        name: String,
        sources: Vec<Source>,
    },
}

impl SourcePicks {
    pub(crate) fn listings(&self) -> &[Source] {
        match self {
            Self::One(pin) => std::slice::from_ref(pin.source()),
            Self::Collection { sources, .. } => sources,
        }
    }

    pub(super) fn resolve(
        &self,
    ) -> impl Iterator<Item = Result<SourceCommit, sources::Failure>> + '_ {
        let (one, collection) = match self {
            Self::One(pin) => (Some(Ok((**pin).clone())), &[][..]),
            Self::Collection { sources, .. } => (None, sources.as_slice()),
        };
        one.into_iter().chain(sources::resolve_each(collection))
    }
}

#[derive(Default)]
pub(crate) struct Installs {
    running: Option<Running>,
    waiting: VecDeque<Request>,
    pub(crate) question: Option<(BuildNotice, Sender<bool>)>,
    pub(crate) world: Option<(WorldQuestion, Sender<bool>)>,
    source_question: Option<(SourceQuestion, Sender<Result<bool, String>>)>,
    source_outcomes: BTreeMap<(Identity, String), Result<(), SourceError>>,
    outcomes: HashMap<(PathBuf, String), Outcome>,
    play: Option<(PathBuf, launcher_play::PlayIntent)>,
}

struct Running {
    profile: instances::Instance,
    identity: Identity,
    target: Option<String>,
    requests: Vec<RequestView>,
    stage: InstallProgress,
    eta: crate::eta::StageEta,
    cancellation: Cancellation,
    github: Option<Arc<GithubInput>>,
    local: Option<Arc<LocalInput>>,
    source: Option<RunningSource>,
}

struct Request {
    profile: instances::Instance,
    identity: Identity,
    target: Option<String>,
    release: Option<backend::Release>,
    displayed_names: BTreeMap<String, String>,
    view: RequestView,
    github: Option<Arc<GithubInput>>,
    local: Option<Arc<LocalInput>>,
    sources: Vec<PreparedSource>,
    picks: Option<SourcePicks>,
}

impl Request {
    fn source_inputs(&self) -> &[PreparedSource] {
        match self.github.as_deref().map(|input| &input.selection) {
            Some(GithubSelection::Source(sources)) => sources,
            _ => &self.sources,
        }
    }
}

/// Bound before the link can start a network lookup, not when its answer comes back.
#[derive(Clone)]
pub(crate) struct GithubTarget {
    profile: instances::Instance,
    identity: Identity,
    target: Option<String>,
    release: Option<backend::Release>,
}

impl GithubTarget {
    pub(crate) fn same_destination(&self, current: &Self) -> bool {
        self.identity == current.identity
            && self.profile.home == current.profile.home
            && self.profile.id == current.profile.id
            && self.profile.incarnation == current.profile.incarnation
            && self.profile.native_profile == current.profile.native_profile
            && self.target == current.target
            && match (&self.release, &current.release) {
                (Some(a), Some(b)) => a.nightly == b.nightly && a.asset == b.asset,
                _ => true, // A successful exact cache-only download can satisfy the retry.
            }
    }

    pub(crate) fn profile_name(&self) -> &str {
        &self.profile.name
    }

    pub(crate) fn source(&self) -> Result<SourceTarget, String> {
        Ok(SourceTarget {
            profile: self.profile.clone(),
            identity: self.identity,
            base: self
                .target
                .clone()
                .ok_or_else(backend::no_version_installed)?,
        })
    }
}

#[cfg(test)]
impl GithubTarget {
    pub(crate) fn fixture(home: &Path) -> Self {
        let request = tests::request(home, "fixture", "test-base");
        Self {
            profile: request.profile,
            identity: request.identity,
            target: request.target,
            release: None,
        }
    }
}

/// The exact installed base and profile that a read-only source fit check covers.
#[derive(Clone)]
pub(crate) struct SourceTarget {
    profile: instances::Instance,
    identity: Identity,
    base: String,
}

#[cfg(test)]
impl SourceTarget {
    /// The profile in `home` (an existing folder) on build `base`.
    pub(crate) fn fixture(home: &Path, base: &str) -> Self {
        let request = tests::request(home, "fixture", base);
        Self {
            profile: request.profile,
            identity: request.identity,
            base: base.into(),
        }
    }
}

impl SourceTarget {
    pub(crate) fn instance(&self) -> &instances::Instance {
        &self.profile
    }
    pub(crate) fn identity(&self) -> Identity {
        self.identity
    }
    pub(crate) fn base(&self) -> &str {
        &self.base
    }
}

pub(crate) struct SourceQuestion {
    pub(crate) base: String,
    pub(crate) mods: Vec<(String, Origin)>,
    pub(crate) dependencies: Vec<SourceDependency>,
}

impl SourceQuestion {
    pub(crate) fn sources(&self) -> impl Iterator<Item = (&str, &Origin)> {
        self.mods
            .iter()
            .map(|(name, origin)| (name.as_str(), origin))
            .chain(
                self.dependencies
                    .iter()
                    .filter_map(|dep| Some((dep.name.as_str(), dep.origin.as_ref()?))),
            )
    }
}

pub(crate) struct SourceActivity<'a> {
    pub(crate) ids: &'a [String],
    pub(crate) names: &'a [String],
    pub(crate) progress: InstallProgress,
    pub(crate) stopping: bool,
}

struct RunningSource {
    ids: Vec<String>,
    names: Vec<String>,
    /// Each mod asked for: its id, the name people know, and its `owner/repo`.
    members: Vec<(String, String, String)>,
    /// It updates installed mods rather than adding new ones.
    update: bool,
    cancellation: Cancellation,
    rejected: BTreeMap<String, SourceError>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Projects from the signed v2 index, by project ID.
    InstallV2,
    Github,
    Local,
    Source,
    /// Update mods the signed v2 index installed: one asked for, or every one with an update.
    UpdateV2,
    UpdateAllV2,
    Remove,
}

#[derive(Clone)]
struct RequestView {
    kind: Kind,
    card: String,
    ids: Vec<String>,
}

pub(crate) struct WorldQuestion {
    pub(crate) removal: bool,
    pub(crate) cancel_default: bool,
    pub(crate) heading: String,
    pub(crate) detail: String,
}

impl WorldQuestion {
    pub(crate) fn new(impact: &ChangeImpact, displayed_names: &BTreeMap<String, String>) -> Self {
        let (heading, detail) = impact.gui_review(displayed_names);
        Self {
            removal: impact.is_removal(),
            cancel_default: impact.is_removal() || impact.risks_worlds(),
            heading,
            detail,
        }
    }
}

pub(crate) enum Worker {
    SourceRejected(String, SourceError),
    Progress(InstallProgress),
    Source(SourceQuestion, Sender<Result<bool, String>>),
    Decide(BuildNotice, Sender<bool>),
    World(WorldQuestion, Sender<bool>),
    Done(Result<InstalledBatch, InstallError>),
}

pub(crate) enum Card<'a> {
    Progress(InstallProgress),
    Waiting,
    Done(&'a Outcome),
}

#[derive(Clone, Default)]
pub(crate) struct Outcome {
    pub(crate) added: Vec<String>,
    pub(crate) updated: Vec<(String, String, String)>,
    pub(crate) failure: Option<(String, String)>,
}

impl Outcome {
    /// The sentence under a card. A failure says why on the line under it, so a card explains
    /// itself without opening the mod's page.
    fn note(self) -> Option<String> {
        if self.failure.is_some() {
            return None;
        }
        if !self.added.is_empty() {
            return Some(trf(
                "Also installed: {mods}",
                &[("mods", &crate::mods::names(&self.added))],
            ));
        }
        self.updated.first().map(|(name, from, to)| {
            trf(
                "Also updated: {mod} {from} → {to}",
                &[("mod", name), ("from", from), ("to", to)],
            )
        })
    }
}

impl Installs {
    /// The build an install running on `profile` captured when it was queued, whatever its
    /// source: the profile keeps showing it while the install holds the profile.
    fn base(&self, profile: &instances::Instance) -> Option<&str> {
        let run = self.running.as_ref()?;
        (run.profile.id == profile.id && run.profile.home == profile.home)
            .then_some(run.target.as_deref())
            .flatten()
    }

    pub(super) fn stage(&self) -> Option<InstallProgress> {
        self.running.as_ref().map(|run| run.stage)
    }

    pub(super) fn card(&self, home: &Path, id: &str) -> Option<Card<'_>> {
        let has = |view: &RequestView| view.card == id || view.ids.iter().any(|root| root == id);
        if let Some(run) = self
            .running
            .as_ref()
            .filter(|run| run.profile.home == home && run.requests.iter().any(has))
        {
            return Some(Card::Progress(run.stage));
        }
        if self
            .waiting
            .iter()
            .any(|request| request.profile.home == home && has(&request.view))
        {
            return Some(Card::Waiting);
        }
        self.outcomes
            .get(&(home.to_path_buf(), id.to_owned()))
            .map(Card::Done)
    }

    fn submit(&mut self, request: Request) {
        if !self.waiting.iter().any(|q| {
            q.profile.id == request.profile.id
                && q.profile.home == request.profile.home
                && q.identity == request.identity
                && q.target == request.target
                && q.profile.native_profile == request.profile.native_profile
                && q.view.kind == request.view.kind
                && q.view.card == request.view.card
                && q.view.ids == request.view.ids
                && q.picks == request.picks
                && match (&q.github, &request.github) {
                    (None, None) => true,
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    _ => false,
                }
                && match (&q.local, &request.local) {
                    (None, None) => true,
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    _ => false,
                }
        }) {
            self.waiting.push_back(request);
        }
    }

    /// The mods a Remove that just finished took out of its profile.
    pub(super) fn removed(&self, event: &Worker) -> Option<(&Path, &[String])> {
        let run = self.running.as_ref()?;
        let request = run.requests.first().filter(|r| r.kind == Kind::Remove)?;
        matches!(event, Worker::Done(Ok(_))).then_some((&run.profile.home, &request.ids))
    }

    pub(super) fn github_feedback(&self, event: &Worker) -> Option<Feedback> {
        let run = self.running.as_ref()?;
        let Worker::Done(result) = event else {
            return None;
        };
        if let Some(local) = run.local.as_ref() {
            let result = match result {
                Ok(_) => Ok(if local.packages.len() == 1 {
                    trf("Installed {mod}.", &[("mod", &local.packages[0].name)])
                } else {
                    trf(
                        "Installed {count} mods.",
                        &[("count", &local.packages.len().to_string())],
                    )
                }),
                Err(InstallError::Cancelled | InstallError::Source(SourceError::Cancelled)) => {
                    return None;
                }
                Err(error) => Err((failure_line(error).into(), error.to_string())),
            };
            return Some(Feedback {
                profile: run.profile.clone(),
                link: local.label.clone(),
                result,
                retry: None,
                local: true,
            });
        }
        let input = run.github.as_ref()?;
        let retry = match result {
            Err(
                InstallError::Download(_)
                | InstallError::GithubBuild(_)
                | InstallError::Source(SourceError::Tools(_) | SourceError::Build { .. }),
            ) => Some(input.clone()),
            _ => None,
        };
        let result = match result {
            Ok(_) => Ok(tr("Installed the selected GitHub mods.").into()),
            Err(InstallError::Cancelled | InstallError::Source(SourceError::Cancelled)) => {
                return None
            }
            Err(InstallError::Source(error)) => {
                let names: Vec<&str> = match &input.selection {
                    GithubSelection::Source(sources) => {
                        sources.iter().map(PreparedSource::name).collect()
                    }
                    GithubSelection::Add(packages) => {
                        packages.iter().map(|p| p.name.as_str()).collect()
                    }
                };
                let name = match names.as_slice() {
                    [one] => (*one).to_owned(),
                    _ => tr("One of these mods").to_owned(),
                };
                let refusal = crate::mods::refused::Refusal::of(&name, error)?;
                Err((refusal.said, refusal.details))
            }
            Err(error) => Err((failure_line(error).into(), error.to_string())),
        };
        Some(Feedback {
            profile: run.profile.clone(),
            link: input.link.clone(),
            result,
            retry,
            local: false,
        })
    }

    /// The modal a finished source Add or Update opens, for the profile that asked: every mod
    /// refused on its own, or, when the run failed as a whole and nothing names a mod, its one
    /// reason. GitHub Adds say theirs in the GitHub form (`github_feedback`).
    pub(super) fn refused(
        &self,
        event: &Worker,
    ) -> Option<(instances::Instance, crate::mods::refused::Refused)> {
        use crate::mods::refused::{Refusal, Refused, RefusedMod};
        let run = self.running.as_ref()?;
        let source = run.source.as_ref().filter(|_| run.github.is_none())?;
        let batch = match event {
            Worker::Done(Err(error)) => Some(match error {
                InstallError::Cancelled => SourceError::Cancelled,
                InstallError::NotForVersion(why) => SourceError::DoesNotFit(why.clone()),
                InstallError::Source(error) => error.clone(),
                error => SourceError::Unproved(error.to_string()),
            }),
            Worker::Done(Ok(_)) => None,
            _ => return None,
        };
        let mut mods: Vec<RefusedMod> = source
            .members
            .iter()
            .filter_map(|(id, name, repo)| {
                Some(RefusedMod {
                    name: name.clone(),
                    repo: Some(repo.clone()),
                    refusal: Refusal::of(name, source.rejected.get(id)?)?,
                })
            })
            .collect();
        if matches!(batch, Some(SourceError::Cancelled)) {
            return None;
        }
        let mut names: Vec<String> = mods.iter().map(|m| m.name.clone()).collect();
        // The rest failed too, for its own reason: said once, after the members refused alone.
        if let Some(error) = batch
            .as_ref()
            .filter(|error| !mods.is_empty() && !source.rejected.values().any(|e| e == *error))
        {
            let rest: Vec<&(String, String, String)> = source
                .members
                .iter()
                .filter(|(id, _, _)| !source.rejected.contains_key(id))
                .collect();
            let (name, repo) = match rest.as_slice() {
                [(_, name, repo)] => (name.clone(), Some(repo.clone())),
                _ => (tr("The other mods").to_owned(), None),
            };
            if let Some(refusal) = Refusal::of(&name, error) {
                mods.push(RefusedMod {
                    name,
                    repo,
                    refusal,
                });
                names = source.names.clone();
            }
        }
        if mods.is_empty() {
            let error = batch?;
            let (name, repo) = match source.members.as_slice() {
                [(_, name, repo)] => (name.clone(), Some(repo.clone())),
                _ => (tr("One of these mods").to_owned(), None),
            };
            mods.push(RefusedMod {
                refusal: Refusal::of(&name, &error)?,
                name,
                repo,
            });
            names = source.names.clone();
        }
        Some((
            run.profile.clone(),
            Refused {
                update: source.update,
                names,
                mods,
                details: false,
            },
        ))
    }

    /// Why a registry install that just ended failed, for a dialog: the card keeps no error.
    pub(super) fn install_failure(&self, event: &Worker) -> Option<(instances::Instance, String)> {
        let run = self.running.as_ref()?;
        if run.source.is_some() || run.requests[0].kind != Kind::InstallV2 {
            return None;
        }
        let (line, detail) = match event {
            Worker::Done(Ok(done)) => notice_failure(done.failure.as_ref()?, &run.requests[0].card),
            Worker::Done(Err(InstallError::Cancelled)) => return None,
            Worker::Done(Err(error @ InstallError::Compile(_))) => {
                (tr("Stayed. No mods changed.").into(), error.to_string())
            }
            Worker::Done(Err(error)) => (failure_line(error).into(), error.to_string()),
            _ => return None,
        };
        let text = if detail.trim().is_empty() || detail == line {
            line
        } else {
            format!("{line}\n\n{detail}")
        };
        Some((run.profile.clone(), text))
    }

    pub(super) fn mutation_failure(&self, event: &Worker) -> Option<String> {
        let run = self.running.as_ref()?;
        if matches!(
            run.requests[0].kind,
            Kind::InstallV2 | Kind::Github | Kind::Local | Kind::Source
        ) {
            return None;
        }
        match event {
            Worker::Done(Err(error)) if !matches!(error, InstallError::Cancelled) => {
                Some(error.to_string())
            }
            _ => None,
        }
    }

    pub(super) fn handle(&mut self, event: Worker) -> bool {
        match event {
            Worker::Progress(step) => {
                if let Some(run) = &mut self.running {
                    run.stage = step;
                    run.eta.update(
                        crate::eta::install_stage_key(step),
                        stage_fraction(step),
                        Instant::now(),
                    );
                }
            }
            Worker::SourceRejected(id, error) => {
                if let Some(source) = self.running.as_mut().and_then(|run| run.source.as_mut()) {
                    source.rejected.insert(id, error);
                }
            }
            Worker::Decide(notice, reply) => {
                if let Some((_, old)) = self.question.replace((notice, reply)) {
                    let _ = old.send(false);
                }
            }
            Worker::World(impact, reply) => {
                if self
                    .running
                    .as_ref()
                    .is_some_and(|run| run.cancellation.is_stopped())
                {
                    let _ = reply.send(false);
                    return false;
                }
                if let Some((_, old)) = self.world.replace((impact, reply)) {
                    let _ = old.send(false);
                }
            }
            Worker::Source(..) => {
                unreachable!("handled at the launcher consent boundary")
            }
            Worker::Done(result) => {
                let Some(run) = self.running.take() else {
                    return false;
                };
                let ready = result.is_ok();
                if let Some(mut source) = run.source {
                    self.source_question.take();
                    let outcome = result.map(|_| ()).map_err(SourceError::from);
                    if let Err(error) = &outcome {
                        if !matches!(error, SourceError::Cancelled) {
                            eprintln!("Nightly source edit: {error}");
                        }
                    }
                    let several = run.requests[0].ids.len() > 1;
                    let stopped = matches!(outcome, Err(SourceError::Cancelled));
                    for id in &run.requests[0].ids {
                        let key = (run.identity, id.clone());
                        let result = match source.rejected.remove(id) {
                            // Stopped: nothing it said before is pinned on the page.
                            Some(_) if stopped => Err(SourceError::Cancelled),
                            Some(error) => Err(error),
                            // Several that failed as a whole: said once, by the modal.
                            None if several && outcome.is_err() => {
                                self.source_outcomes.remove(&key);
                                continue;
                            }
                            None => outcome.clone(),
                        };
                        if matches!(result, Err(SourceError::Cancelled)) {
                            self.source_outcomes.remove(&key);
                        } else {
                            self.source_outcomes.insert(key, result);
                        }
                    }
                } else {
                    for request in run.requests {
                        let outcome = match &result {
                            Ok(done) => Outcome {
                                added: done
                                    .added
                                    .iter()
                                    .filter(|entry| {
                                        !request.ids.contains(&entry.project)
                                            && entry.install_hold.is_none()
                                    })
                                    .map(|entry| entry.title.clone())
                                    .collect(),
                                updated: done
                                    .updated
                                    .iter()
                                    .filter(|(_, entry)| !request.ids.contains(&entry.project))
                                    .map(|(old, entry)| {
                                        (entry.title.clone(), old.clone(), entry.version.clone())
                                    })
                                    .collect(),
                                failure: done
                                    .failure
                                    .as_ref()
                                    .map(|notice| notice_failure(notice, &request.card)),
                            },
                            Err(InstallError::Compile(notice)) => Outcome {
                                failure: Some((
                                    tr("Stayed. No mods changed.").into(),
                                    notice
                                        .changes
                                        .iter()
                                        .map(|change| {
                                            trf(
                                                "{mod}: {reason}",
                                                &[
                                                    ("mod", &change.title),
                                                    ("reason", &change.reason.detail()),
                                                ],
                                            )
                                        })
                                        .collect::<Vec<_>>()
                                        .join("\n\n"),
                                )),
                                ..Outcome::default()
                            },
                            Err(InstallError::Cancelled) => Outcome::default(),
                            Err(error) => Outcome {
                                failure: Some((failure_line(error).into(), error.to_string())),
                                ..Outcome::default()
                            },
                        };
                        if let Some((line, detail)) = &outcome.failure {
                            eprintln!("installing {}: {line}: {detail}", request.card);
                        }
                        self.outcomes
                            .insert((run.profile.home.clone(), request.card), outcome.clone());
                        for id in request.ids {
                            self.outcomes
                                .insert((run.profile.home.clone(), id), outcome.clone());
                        }
                    }
                }
                if !ready
                    && self.play.as_ref().is_some_and(|(home, intent)| {
                        *home == run.profile.home && intent.instance == run.profile.id
                    })
                {
                    self.play = None;
                }
                return true;
            }
        }
        false
    }

    fn runs_for(&self, instance: &instances::Instance) -> bool {
        self.running
            .as_ref()
            .is_some_and(|run| run.profile.id == instance.id && run.profile.home == instance.home)
    }

    fn accepts_play(&self, instance: &instances::Instance) -> bool {
        self.play.is_none() && self.runs_for(instance)
    }

    fn defer_play(
        &mut self,
        instance: &instances::Instance,
        intent: launcher_play::PlayIntent,
    ) -> bool {
        if !self.accepts_play(instance) {
            return false;
        }
        self.play = Some((instance.home.clone(), intent));
        true
    }

    pub(super) fn take_play(&mut self) -> Option<(PathBuf, launcher_play::PlayIntent)> {
        let (home, intent) = self.play.as_ref()?;
        if self
            .waiting
            .iter()
            .any(|request| request.profile.home == *home && request.profile.id == intent.instance)
        {
            return None;
        }
        self.play.take()
    }
}

impl Launcher {
    pub(crate) fn source_target(&self) -> Result<SourceTarget, String> {
        self.github_target()?.source()
    }

    pub(crate) fn queue_sources(
        &mut self,
        target: SourceTarget,
        inputs: Vec<PreparedSource>,
    ) -> Result<(), String> {
        if inputs.is_empty() {
            return Ok(());
        }
        let current = self.source_target()?;
        if current.identity != target.identity
            || current.base != target.base
            || current.profile.home != target.profile.home
        {
            return Err(tr(
                "The profile or game build changed. Check the source again; nothing was changed.",
            )
            .into());
        }
        let ids = inputs
            .iter()
            .map(|source| source.origin().id.clone())
            .collect::<Vec<_>>();
        for id in &ids {
            self.mods
                .installs
                .source_outcomes
                .remove(&(target.identity, id.clone()));
        }
        self.mods.installs.submit(Request {
            profile: target.profile,
            identity: target.identity,
            target: Some(target.base),
            release: None,
            displayed_names: BTreeMap::new(),
            view: RequestView {
                kind: Kind::Source,
                card: ids[0].clone(),
                ids,
            },
            github: None,
            local: None,
            sources: inputs,
            picks: None,
        });
        self.start_registry_queue();
        Ok(())
    }

    pub(crate) fn source_activity(&self) -> Option<SourceActivity<'_>> {
        let run = self.mods.installs.running.as_ref()?;
        if let Some(source) = &run.source {
            return Some(SourceActivity {
                ids: &source.ids,
                names: &source.names,
                progress: run.stage,
                stopping: source.cancellation.is_stopped(),
            });
        }
        let local = run.local.as_ref()?;
        Some(SourceActivity {
            ids: &run.requests[0].ids,
            names: &local.names,
            progress: run.stage,
            stopping: run.cancellation.is_stopped(),
        })
    }

    pub(crate) fn stop_source(&mut self) {
        let Some(cancellation) = self
            .mods
            .installs
            .running
            .as_ref()
            .filter(|run| run.source.is_some() || run.local.is_some())
            .map(|run| run.cancellation.clone())
        else {
            return;
        };
        cancellation.stop();
        if cancellation.is_stopped() {
            self.answer_source_question(Ok(false));
            if let Some((_, reply)) = self.mods.installs.world.take() {
                let _ = reply.send(false);
            }
        }
    }

    pub(crate) fn stop_registry_install(&mut self) {
        let Some(cancellation) = self
            .mods
            .installs
            .running
            .as_ref()
            .map(|run| run.cancellation.clone())
        else {
            return;
        };
        cancellation.stop();
        if cancellation.is_stopped() {
            self.answer_source_question(Ok(false));
            if let Some((_, reply)) = self.mods.installs.world.take() {
                let _ = reply.send(false);
            }
            if let Some((_, reply)) = self.mods.installs.question.take() {
                let _ = reply.send(false);
            }
        }
    }

    pub(crate) fn registry_install_stopping(&self) -> bool {
        self.mods
            .installs
            .running
            .as_ref()
            .is_some_and(|run| run.cancellation.is_stopped())
    }

    pub(crate) fn source_outcome(&self, id: &str) -> Option<&Result<(), SourceError>> {
        let home = &self.selected_instance()?.home;
        self.mods
            .installs
            .source_outcomes
            .get(&(Identity::of(home).ok()?, id.to_owned()))
    }

    pub(crate) fn source_question(&self) -> Option<&SourceQuestion> {
        self.mods
            .installs
            .source_question
            .as_ref()
            .map(|(question, _)| question)
    }

    pub(crate) fn answer_source_question(&mut self, allow: Result<bool, String>) {
        if let Some((_, reply)) = self.mods.installs.source_question.take() {
            let _ = reply.send(allow);
        }
    }

    pub(super) fn receive_source_question(
        &mut self,
        question: SourceQuestion,
        reply: Sender<Result<bool, String>>,
    ) {
        if self
            .source_activity()
            .is_none_or(|activity| activity.stopping)
        {
            let _ = reply.send(Ok(false));
        } else if question.dependencies.is_empty() && self.source_already_allowed(&question) {
            let _ = reply.send(Ok(true));
        } else {
            self.reset_install_question_sheet();
            self.mods.installs.source_question = Some((question, reply));
        }
    }

    pub(crate) fn registry_card(&self, id: &str) -> Option<Card<'_>> {
        self.mods.installs.card(&self.mods_home(), id)
    }

    pub(crate) fn registry_card_control(
        &self,
        id: &str,
        button: impl FnOnce(String) -> AnyElement,
    ) -> Option<AnyElement> {
        match self.registry_card(id)? {
            Card::Progress(stage) => Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(button(stage_label(stage)))
                    .children(stage_fraction(stage).map(|fraction| {
                        crate::widgets::progress_bar(Some((fraction, fraction)), &self.theme)
                    }))
                    .into_any_element(),
            ),
            Card::Waiting => Some(button(tr("Waiting to install…").into())),
            Card::Done(_) => None,
        }
    }

    pub(crate) fn registry_install_label(&self) -> String {
        if let Some(instance) = self
            .selected_instance()
            .filter(|_| self.instances.list.len() > 1)
        {
            trf("Install to {profile}", &[("profile", &instance.name)])
        } else {
            tr("Install").into()
        }
    }

    pub(crate) fn registry_card_note(&self, id: &str) -> Option<String> {
        self.registry_outcome(id)?.note()
    }

    fn registry_outcome(&self, id: &str) -> Option<Outcome> {
        match self.registry_card(id) {
            Some(Card::Progress(_) | Card::Waiting) => None,
            card => {
                if let Some(hold) = self.registry_install_hold(id) {
                    return Some(Outcome {
                        failure: Some((hold.label(), hold.detail())),
                        ..Outcome::default()
                    });
                }
                match card {
                    Some(Card::Done(outcome)) => Some(outcome.clone()),
                    _ => None,
                }
            }
        }
    }

    pub(crate) fn registry_card_outcome(&self, id: &str, cx: &mut Context<Self>) -> Option<Div> {
        let done = self.registry_outcome(id)?;
        let mut lines = Vec::new();
        if !done.added.is_empty() {
            lines.push(trf(
                "Also installed: {mods}",
                &[("mods", &crate::mods::names(&done.added))],
            ));
        }
        for (name, from, to) in &done.updated {
            lines.push(trf(
                "Updated {mod} from {from} to {to}",
                &[("mod", name), ("from", from), ("to", to)],
            ));
        }
        // A failed install says why in a dialog as it ends (`install_failure`); only a hold,
        // which lasts, stays on the page.
        let failure = done
            .failure
            .clone()
            .filter(|_| self.registry_install_hold(id).is_some());
        if lines.is_empty() && failure.is_none() {
            return None;
        }
        let failure = failure.map(|(label, detail)| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .child(label)
                .child(
                    crate::frame::button("install-outcome-detail", tr("Show why")).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.open_notice(detail.clone(), None);
                            cx.notify();
                        }),
                    ),
                )
        });
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .text_size(px(tokens::LABEL))
                .children(lines.into_iter().map(|line| div().child(line)))
                .children(failure),
        )
    }

    pub(crate) fn registry_accepts_play(&self) -> bool {
        self.selected_instance()
            .is_some_and(|instance| self.mods.installs.accepts_play(instance))
    }

    pub(crate) fn defer_registry_play(
        &mut self,
        instance: &instances::Instance,
        intent: launcher_play::PlayIntent,
    ) {
        if self.mods.installs.defer_play(instance, intent) {
            self.open_sheet(crate::sheets::Sheet::RegistryPlay);
        }
    }

    pub(crate) fn registry_stage(&self) -> Option<InstallProgress> {
        self.mods.installs.stage()
    }

    pub(crate) fn registry_eta(&self) -> Option<Duration> {
        self.mods
            .installs
            .running
            .as_ref()
            .and_then(|run| run.eta.remaining(Instant::now()))
    }

    pub(crate) fn github_target(&self) -> Result<GithubTarget, String> {
        if self.accounts.selected().is_none() && !sunrise_launcher::developer_offline::selected() {
            return Err(tr("Installs after you sign in").into());
        }
        if let Some(why) = self.mods_locked() {
            return Err(why.into());
        }
        let profile = self
            .selected_instance()
            .cloned()
            .ok_or_else(backend::no_version_installed)?;
        if profile.kind == instances::Kind::Pack {
            return Err(tr("This pack controls which mods it installs.").into());
        }
        let target = self.profile_target(&profile);
        let release = target
            .as_deref()
            .and_then(|tag| self.release_to_install(tag));
        Ok(GithubTarget {
            identity: sunrise::install_target(&profile)?,
            profile,
            target,
            release,
        })
    }

    pub(crate) fn add_sources(&mut self, picks: SourcePicks) {
        let target = match self.source_target() {
            Ok(target) => target,
            Err(error) => {
                self.open_notice(error, None);
                return;
            }
        };
        let ids: Vec<String> = picks
            .listings()
            .iter()
            .map(|source| source.id().into())
            .collect();
        let Some(card) = ids.first().cloned() else {
            return;
        };
        for id in &ids {
            self.mods
                .installs
                .source_outcomes
                .remove(&(target.identity, id.clone()));
        }
        self.mods.installs.submit(Request {
            profile: target.profile,
            identity: target.identity,
            target: Some(target.base),
            release: None,
            displayed_names: BTreeMap::new(),
            view: RequestView {
                kind: Kind::Source,
                card,
                ids,
            },
            github: None,
            local: None,
            sources: Vec::new(),
            picks: Some(picks),
        });
        self.start_registry_queue();
    }

    pub(crate) fn install_base(&self, profile: &instances::Instance) -> Option<&str> {
        self.mods.installs.base(profile)
    }

    pub(crate) fn queue_github(&mut self, input: Arc<GithubInput>) -> Result<(), String> {
        let target = &input.target;
        if !target.same_destination(&self.github_target()?) {
            return Err(tr(
                "The profile or game build changed. Check the source again; nothing was changed.",
            )
            .into());
        }
        let (kind, ids): (Kind, Vec<String>) = match &input.selection {
            GithubSelection::Add(packages) => (
                Kind::Github,
                packages.iter().map(|p| p.id.clone()).collect(),
            ),
            GithubSelection::Source(packages) => (
                Kind::Source,
                packages.iter().map(|p| p.origin().id.clone()).collect(),
            ),
        };
        if kind == Kind::Source {
            for id in &ids {
                self.mods
                    .installs
                    .source_outcomes
                    .remove(&(target.identity, id.clone()));
            }
        }
        self.mods.installs.submit(Request {
            identity: target.identity,
            profile: target.profile.clone(),
            target: target.target.clone(),
            release: target.release.clone(),
            displayed_names: BTreeMap::new(),
            view: RequestView {
                kind,
                card: input.link.clone(),
                ids,
            },
            github: Some(input),
            local: None,
            sources: Vec::new(),
            picks: None,
        });
        self.start_registry_queue();
        Ok(())
    }

    pub(crate) fn queue_local(&mut self, input: Arc<LocalInput>) -> Result<(), String> {
        let target = &input.target;
        if !target.same_destination(&self.github_target()?) {
            return Err(tr(
                "The profile or game build changed. Check the source again; nothing was changed.",
            )
            .into());
        }
        let ids: Vec<String> = input.packages.iter().map(|p| p.id.clone()).collect();
        self.mods.installs.submit(Request {
            identity: target.identity,
            profile: target.profile.clone(),
            target: target.target.clone(),
            release: target.release.clone(),
            displayed_names: BTreeMap::new(),
            view: RequestView {
                kind: Kind::Local,
                card: input.label.clone(),
                ids,
            },
            github: None,
            local: Some(input),
            sources: Vec::new(),
            picks: None,
        });
        self.start_registry_queue();
        Ok(())
    }

    pub(crate) fn queue_registry_change(
        &mut self,
        kind: Kind,
        card: String,
        ids: Vec<String>,
        displayed_names: BTreeMap<String, String>,
    ) -> Result<(), String> {
        if ids.is_empty() {
            return Ok(());
        }
        if kind == Kind::InstallV2
            && self.accounts.selected().is_none()
            && !sunrise_launcher::developer_offline::selected()
        {
            return Err(tr("Installs after you sign in").into());
        }
        if let Some(why) = self.mods_locked() {
            return Err(why.into());
        }
        let instance = self
            .selected_instance()
            .cloned()
            .ok_or_else(backend::no_version_installed)?;
        if instance.kind == instances::Kind::Pack {
            return Err(tr("This pack controls which mods it installs.").into());
        }
        let target = self.profile_target(&instance);
        let release = target
            .as_deref()
            .and_then(|tag| self.release_to_install(tag));
        self.mods.installs.submit(Request {
            identity: sunrise::install_target(&instance)?,
            profile: instance,
            target,
            release,
            displayed_names,
            view: RequestView { kind, card, ids },
            github: None,
            local: None,
            sources: Vec::new(),
            picks: None,
        });
        self.start_registry_queue();
        Ok(())
    }

    pub(super) fn start_registry_queue(&mut self) -> bool {
        if self.mods_locked().is_some()
            || self.mutating()
            || self.mods.installs.question.is_some()
            || self.mods.installs.world.is_some()
        {
            return false;
        }
        let Some(request) = self.mods.installs.waiting.pop_front() else {
            return false;
        };
        // Pressing Add again reads the source again: its last refusal goes until this run says.
        let identity = request.identity;
        self.mods
            .installs
            .source_outcomes
            .retain(|(who, id), _| *who != identity || !request.view.ids.contains(id));
        let kind = request.view.kind;
        let ids = request.view.ids.clone();
        let cancellation = Cancellation::default();
        let listed = request
            .picks
            .as_ref()
            .is_none_or(|picks| self.source_picks_listed(picks.listings()));
        let source = (kind == Kind::Source).then(|| RunningSource {
            ids: ids.clone(),
            members: if let Some(picks) = &request.picks {
                picks
                    .listings()
                    .iter()
                    .map(|source| {
                        (
                            source.id().to_owned(),
                            source.name().to_owned(),
                            source.repo().to_owned(),
                        )
                    })
                    .collect()
            } else {
                request
                    .source_inputs()
                    .iter()
                    .map(|source| {
                        let origin = source.origin();
                        (
                            origin.id.clone(),
                            source.name().to_owned(),
                            format!("{}/{}", origin.owner, origin.repo),
                        )
                    })
                    .collect()
            },
            update: request
                .source_inputs()
                .iter()
                .any(PreparedSource::is_update),
            names: if let Some(picks) = &request.picks {
                picks
                    .listings()
                    .iter()
                    .map(|source| source.name().into())
                    .collect()
            } else {
                request
                    .source_inputs()
                    .iter()
                    .map(|source| source.name().into())
                    .collect()
            },
            cancellation: cancellation.clone(),
            rejected: BTreeMap::new(),
        });
        self.mods.installs.running = Some(Running {
            profile: request.profile.clone(),
            identity: request.identity,
            target: request.target.clone(),
            requests: vec![request.view.clone()],
            stage: InstallProgress::Checking,
            eta: crate::eta::StageEta::default(),
            cancellation: cancellation.clone(),
            source,
            github: request.github.clone(),
            local: request.local.clone(),
        });
        let tx = self.mods.tx.clone();
        let launcher = self.tx.clone();
        std::thread::spawn(move || {
            let post = |event| tx.send(Event::Registry(event)).is_ok();
            let choose = || {
                if let Some(tag) = request
                    .target
                    .as_deref()
                    .filter(|tag| backend::installed().iter().any(|local| local == tag))
                {
                    FirstBuild::installed(tag)
                } else if let Some(release) = &request.release {
                    FirstBuild::download(release)
                } else {
                    Err(backend::no_version_installed())
                }
            };
            let hooks = InstallHooks {
                progress: &|step| {
                    let _ = post(Worker::Progress(step));
                },
                continue_after_failure: &|notice| {
                    let (reply, answer) = std::sync::mpsc::channel();
                    post(Worker::Decide(notice.clone(), reply)) && answer.recv().unwrap_or(false)
                },
                cancellation: Some(&cancellation),
            };
            let approve = |impact: &ChangeImpact| {
                if cancellation.is_stopped() {
                    return false;
                }
                let (reply, answer) = std::sync::mpsc::channel();
                let question = WorldQuestion::new(impact, &request.displayed_names);
                post(Worker::World(question, reply)) && answer.recv().unwrap_or(false)
            };
            let authorize = |prepared: &[PreparedSource], dependencies: &[SourceDependency]| {
                let question = SourceQuestion {
                    base: prepared[0].base().into(),
                    mods: prepared
                        .iter()
                        .map(|item| (item.name().into(), item.origin().clone()))
                        .collect(),
                    dependencies: dependencies.to_vec(),
                };
                let (reply, answer) = std::sync::mpsc::channel();
                if !post(Worker::Source(question, reply)) {
                    return Ok(false);
                }
                answer.recv().unwrap_or(Ok(false))
            };
            let confirm_publisher_change = |question: &PublisherChangeQuestion| {
                let (reply, answer) = std::sync::mpsc::channel();
                launcher
                    .send(Msg::PublisherChangeQuestion(
                        Box::new(question.clone()),
                        reply,
                    ))
                    .is_ok()
                    && answer.recv().unwrap_or(false)
            };
            let choose_provider = |need: &ProviderChoiceNeeded| {
                let (reply, answer) = std::sync::mpsc::channel();
                launcher
                    .send(Msg::ProviderChoice(Box::new(need.clone()), reply))
                    .ok()
                    .and_then(|()| answer.recv().ok().flatten())
            };
            let v2_hooks = V2Hooks {
                install: hooks,
                choose_provider: Some(&choose_provider),
                confirm_publisher_change: Some(&confirm_publisher_change),
            };
            let result = match kind {
                Kind::InstallV2 => sunrise::install_registry_v2(
                    &request.profile,
                    request.identity,
                    ids.clone(),
                    V2InstallOptions::default(),
                    choose,
                    v2_hooks,
                ),
                Kind::UpdateV2 | Kind::UpdateAllV2 => sunrise::update_registry_v2(
                    &request.profile,
                    request.identity,
                    if kind == Kind::UpdateV2 {
                        sunrise::UpdateRequest::Strict(ids)
                    } else {
                        sunrise::UpdateRequest::Bulk(ids)
                    },
                    V2InstallOptions::default(),
                    choose,
                    v2_hooks,
                    &approve,
                ),
                Kind::Github => {
                    let input = request.github.as_ref().expect("a checked GitHub request");
                    match &input.selection {
                        GithubSelection::Add(packages) => sunrise::install_github_manual(
                            &request.profile,
                            request.identity,
                            sunrise::ManualRelease {
                                found: &input.source,
                                selected: packages,
                                base: request.target.as_deref(),
                            },
                            choose,
                            hooks,
                            &approve,
                        )
                        .map(|_| InstalledBatch::default()),
                        GithubSelection::Source(_) => {
                            unreachable!("source installs use the source worker")
                        }
                    }
                }
                Kind::Local => {
                    let input = request.local.as_ref().expect("a checked local request");
                    sunrise::install_local(
                        &request.profile,
                        request.identity,
                        &input.packages,
                        &cancellation,
                        &|step| {
                            let _ = post(Worker::Progress(step));
                        },
                        &approve,
                    )
                    .map(|_| InstalledBatch::default())
                }
                Kind::Source => {
                    let hooks = SourceHooks {
                        install: hooks,
                        cancellation: &cancellation,
                        authorize: &authorize,
                        approve_worlds: &approve,
                    };
                    let result = if let Some(picks) = &request.picks {
                        (|| {
                            if !listed {
                                return Err(SourceError::Changed(
                                    tr("This source listing changed. Open it again from Discover.")
                                        .into(),
                                ));
                            }
                            let mut rejected = None;
                            let mut pins = Vec::new();
                            let mut resolved = picks.resolve();
                            for listing in picks.listings() {
                                if cancellation.is_stopped() {
                                    return Err(SourceError::Cancelled);
                                }
                                let pin = resolved.next().expect("one resolution per listing");
                                if cancellation.is_stopped() {
                                    return Err(SourceError::Cancelled);
                                }
                                match pin {
                                    Ok(pin) => pins.push(pin),
                                    Err(error) => {
                                        let error = SourceError::Github(error);
                                        let _ = post(Worker::SourceRejected(
                                            listing.id().into(),
                                            error.clone(),
                                        ));
                                        rejected = Some(error);
                                    }
                                }
                            }
                            let added_from = match picks {
                                SourcePicks::One(_) => AddedFrom::Discover,
                                SourcePicks::Collection { id, name, .. } => AddedFrom::Collection {
                                    id: id.clone(),
                                    name: name.clone(),
                                },
                            };
                            let packed = sunrise::pack_sources(&pins, added_from, &cancellation);
                            if cancellation.is_stopped() {
                                return Err(SourceError::Cancelled);
                            }
                            let mut packages = Vec::new();
                            for (pin, result) in pins.iter().zip(packed) {
                                match result {
                                    Ok(package) => packages.push(package),
                                    Err(error) => {
                                        let _ = post(Worker::SourceRejected(
                                            pin.source().id().into(),
                                            error.clone(),
                                        ));
                                        rejected = Some(error);
                                    }
                                }
                            }
                            if cancellation.is_stopped() {
                                return Err(SourceError::Cancelled);
                            }
                            if packages.is_empty() {
                                return Err(rejected.unwrap_or(SourceError::Cancelled));
                            }
                            let result = sunrise::add_sources(
                                &request.profile,
                                request.identity,
                                request.target.as_deref().expect("source add has a target"),
                                packages,
                                hooks,
                            );
                            if cancellation.is_stopped() {
                                Err(SourceError::Cancelled)
                            } else {
                                result
                            }
                        })()
                    } else {
                        sunrise::apply_sources(
                            &request.profile,
                            request.identity,
                            request.source_inputs(),
                            choose,
                            hooks,
                        )
                    };
                    result
                        .map(|()| InstalledBatch::default())
                        .map_err(InstallError::Source)
                }
                Kind::Remove => sunrise::remove_mods(
                    &request.profile,
                    request.identity,
                    &ids,
                    choose,
                    hooks,
                    &approve,
                )
                .map(|_| InstalledBatch::default()),
            };
            let _ = post(Worker::Done(result));
        });
        true
    }
}

pub(crate) fn stage_label(step: InstallProgress) -> String {
    match step {
        InstallProgress::CheckingBuild => tr("Checking the build…").into(),
        InstallProgress::UnpackingBuild => tr("Unpacking the build").into(),
        InstallProgress::FinishingBuild => tr("Putting the build in place").into(),
        InstallProgress::PreparingBuild => tr("Getting the build ready…").into(),
        InstallProgress::Checking => tr("Checking the mod set…").into(),
        InstallProgress::Waiting => tr("Waiting for another build to finish…").into(),
        InstallProgress::Downloading(f) => trf(
            "Downloading {percent}%",
            &[("percent", &format!("{:.0}", f * 100.))],
        ),
        InstallProgress::DownloadingBuild(None) => tr("Downloading the build…").into(),
        InstallProgress::DownloadingBuild(Some(f)) => trf(
            "Downloading the build {percent}%",
            &[("percent", &format!("{:.0}", f.clamp(0., 1.) * 100.))],
        ),
        InstallProgress::Minecraft(step, files) => {
            use sunrise_launcher::backend::PrepareStep;
            let count = files.map(|(done, total)| (done.to_string(), total.to_string()));
            match (step, count) {
                (PrepareStep::Files, None) => tr("Finding Minecraft's files…").into(),
                (PrepareStep::Files, Some((done, total))) => trf(
                    "Downloading Minecraft's files ({done} of {total})",
                    &[("done", &done), ("total", &total)],
                ),
                (PrepareStep::Java, None) => tr("Finding Java…").into(),
                (PrepareStep::Java, Some((done, total))) => trf(
                    "Downloading Java ({done} of {total} files)",
                    &[("done", &done), ("total", &total)],
                ),
                (PrepareStep::Sounds, _) => tr("Adding the sounds…").into(),
                (PrepareStep::Prepare, _) => tr("Preparing Minecraft's files…").into(),
            }
        }
        InstallProgress::Preparing => tr("Getting ready…").into(),
        InstallProgress::Rust(fraction) => trf(
            "Downloading build tools {percent}%",
            &[("percent", &format!("{:.0}", fraction * 100.0))],
        ),
        InstallProgress::Tools { total, .. } => trf(
            "Getting build tools ({size})",
            &[("size", &sunrise_launcher::storage::format_bytes(total))],
        ),
        InstallProgress::Building(Some(fraction)) => trf(
            "Building {percent}%",
            &[(
                "percent",
                &format!("{:.0}", (fraction * 100.0).clamp(0.0, 100.0)),
            )],
        ),
        InstallProgress::Building(None) => tr("Building").into(),
        InstallProgress::Publishing => tr("Finishing…").into(),
        InstallProgress::Reviewing => tr("Review changes…").into(),
    }
}

pub(crate) fn stage_fraction(step: InstallProgress) -> Option<f32> {
    match step {
        InstallProgress::DownloadingBuild(f) => f.map(|f| f.clamp(0., 1.)),
        InstallProgress::Downloading(f) | InstallProgress::Rust(f) => Some(f),
        InstallProgress::Minecraft(_, Some((done, total))) if total > 0 => {
            Some(done as f32 / total as f32)
        }
        InstallProgress::Building(f) => f,
        InstallProgress::Tools { done, total } if total > 0 => Some(done as f32 / total as f32),
        _ => None,
    }
}

/// What a registry install that finished but left mods off or on their old version says:
/// a line about `card`'s own mod, or the batch, and each change's reason.
fn notice_failure(notice: &BuildNotice, card: &str) -> (String, String) {
    let this = notice.changes.iter().find(|change| change.project == card);
    let label = match this {
        Some(change) if change.predecessor.is_some() => tr("Kept the working version").into(),
        Some(change) => change.reason.label(),
        None => tr("Some mods stayed off or kept their old version").into(),
    };
    let detail = notice
        .changes
        .iter()
        .map(|change| {
            trf(
                "{mod}: {disposition}. {reason}",
                &[
                    ("mod", &change.title),
                    ("disposition", &change.disposition()),
                    ("reason", &change.reason.detail()),
                ],
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    (label, detail)
}

fn failure_line(error: &InstallError) -> &'static str {
    match error {
        InstallError::Refused(_) => tr("Can't install these mods together"),
        InstallError::Host(_) => tr("Native mods can't build on this computer"),
        InstallError::Unproved(_) => tr("Couldn't check that the mods fit"),
        InstallError::NotForVersion(_) => tr("Not available for your version"),
        InstallError::Source(SourceError::DoesNotFit(_)) => tr("Not available for your version"),
        InstallError::Source(SourceError::Dependency { .. } | SourceError::Catalogue(_)) => {
            tr("Can't install these mods together")
        }
        InstallError::Source(
            SourceError::Unproved(_)
            | SourceError::Github(_)
            | SourceError::DependencyGithub { .. },
        ) => tr("Fit could not be verified"),
        InstallError::Source(SourceError::Tools(_)) => {
            tr("Couldn't download build tools. Check your connection and try again.")
        }
        InstallError::Source(SourceError::Changed(_)) => {
            tr("The profile or game build changed. Check the source again; nothing was changed.")
        }
        InstallError::Source(SourceError::Cancelled) => tr("Install cancelled."),
        InstallError::Source(SourceError::Build { .. }) => tr("Couldn't build on this computer."),
        InstallError::GithubBuild(_) => tr("Could not build the selected mods."),
        InstallError::Download(_) => tr("Download failed. Try again"),
        InstallError::Compile(_) => tr("Couldn't build on your computer"),
        InstallError::PublisherChanged(_) => {
            tr("This mod's publisher changed. Confirm before installing.")
        }
        InstallError::AmbiguousProvider(_) => {
            tr("Several mods provide a required dependency. Choose one.")
        }
        InstallError::ProviderConflict(_) => {
            tr("Another mod is already installed for that slot. Switch provider instead.")
        }
        InstallError::Cancelled => tr("Install cancelled."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_build_transfer_has_determinate_fill() {
        for stage in [
            InstallProgress::CheckingBuild,
            InstallProgress::UnpackingBuild,
            InstallProgress::FinishingBuild,
            InstallProgress::PreparingBuild,
        ] {
            assert_eq!(stage_fraction(stage), None, "{stage:?}");
            assert!(!stage_label(stage).contains('%'));
        }
        assert_eq!(
            stage_fraction(InstallProgress::DownloadingBuild(Some(0.25))),
            Some(0.25)
        );
        assert_eq!(
            stage_fraction(InstallProgress::DownloadingBuild(Some(-0.25))),
            Some(0.)
        );
        assert_eq!(
            stage_fraction(InstallProgress::DownloadingBuild(Some(1.25))),
            Some(1.)
        );
        let unknown = sunrise_launcher::backend::InstallPhase::DownloadingUnknownLength.into();
        assert_eq!(stage_label(unknown), "Downloading the build…");
        assert_eq!(stage_fraction(unknown), None);
        assert_ne!(InstallProgress::FinishingBuild, InstallProgress::Publishing);
    }

    #[test]
    fn a_build_preparation_names_its_step_and_counts_mojangs_files() {
        use sunrise_launcher::backend::PrepareStep;
        let downloading = InstallProgress::Minecraft(PrepareStep::Files, Some((30, 120)));
        assert_eq!(
            stage_label(downloading),
            "Downloading Minecraft's files (30 of 120)"
        );
        assert_eq!(stage_fraction(downloading), Some(0.25));
        let preparing = InstallProgress::Minecraft(PrepareStep::Prepare, None);
        assert_eq!(stage_label(preparing), "Preparing Minecraft's files…");
        assert_eq!(stage_fraction(preparing), None);
    }

    #[test]
    fn a_failed_install_says_why_in_a_dialog_not_on_the_card() {
        let root = homes();
        let installs = with_running(root.path());
        let error = || {
            InstallError::Unproved("The mod registry index signature could not be verified.".into())
        };
        let (profile, why) = installs
            .install_failure(&Worker::Done(Err(error())))
            .unwrap();
        assert_eq!(profile.id, "A");
        assert_eq!(
            why,
            "Couldn't check that the mods fit\n\nThe mod registry index signature could not be verified."
        );
        assert!(installs
            .install_failure(&Worker::Done(Err(InstallError::Cancelled)))
            .is_none());
        // Installed, but a dependency was left off: the dialog says so too.
        let left_off = InstalledBatch {
            added: Vec::new(),
            updated: Vec::new(),
            failure: Some(BuildNotice {
                changes: vec![sunrise_launcher::browser::sunrise::BuildChange {
                    project: "lib".into(),
                    title: "Lib".into(),
                    reason: sunrise_launcher::browser::installed::InstallHold::Compile,
                    predecessor: None,
                }],
                detail: String::new(),
                log: None,
            }),
        };
        let (_, why) = installs
            .install_failure(&Worker::Done(Ok(left_off)))
            .unwrap();
        assert_eq!(
            why,
            "Some mods stayed off or kept their old version\n\nLib: Keep installed, but off. Its code couldn't build on your computer."
        );
        let clean = InstalledBatch {
            added: Vec::new(),
            updated: Vec::new(),
            failure: None,
        };
        assert!(installs.install_failure(&Worker::Done(Ok(clean))).is_none());
        let failed = Outcome {
            failure: Some((failure_line(&error()).into(), error().to_string())),
            ..Outcome::default()
        };
        assert_eq!(failed.note(), None);
    }

    pub(super) fn request(home: &Path, id: &str, target: &str) -> Request {
        let slug = home.file_name().unwrap().to_str().unwrap();
        Request {
            identity: Identity::of(home).unwrap(),
            profile: instances::Instance {
                id: slug.into(),
                name: slug.into(),
                kind: instances::Kind::Own,
                home: home.into(),
                work: home.into(),
                incarnation: None,
                icon: None,
                group: None,
                version: None,
                channel: Default::default(),
                native_profile: Default::default(),
                played: Default::default(),
                last_played: None,
                launch: Default::default(),
            },
            target: Some(target.into()),
            release: None,
            displayed_names: BTreeMap::new(),
            github: None,
            local: None,
            sources: Vec::new(),
            picks: None,
            view: RequestView {
                kind: Kind::InstallV2,
                card: id.into(),
                ids: vec![id.into()],
            },
        }
    }

    fn with_running(root: &Path) -> Installs {
        let first = request(&root.join("A"), "one", "nightly");
        Installs {
            running: Some(Running {
                profile: first.profile,
                identity: first.identity,
                target: first.target,
                requests: vec![first.view],
                stage: InstallProgress::Checking,
                eta: crate::eta::StageEta::default(),
                cancellation: Cancellation::default(),
                github: None,
                local: None,
                source: None,
            }),
            ..Installs::default()
        }
    }

    #[test]
    fn a_registry_install_keeps_its_profile_build_while_it_runs() {
        let root = homes();
        let installs = with_running(root.path());
        let run = installs.running.as_ref().unwrap();
        assert!(run.source.is_none(), "a registry install, not a source one");
        assert_eq!(installs.base(&run.profile), Some("nightly"));
        let other = request(&root.path().join("B"), "two", "dev").profile;
        assert_eq!(installs.base(&other), None);
    }

    #[test]
    fn failed_github_add_retries_only_the_checked_subset_and_never_a_version_refusal() {
        let root = homes();
        let mut installs = with_running(root.path());
        let run = installs.running.as_mut().unwrap();
        let target = GithubTarget {
            profile: run.profile.clone(),
            identity: run.identity,
            target: run.target.clone(),
            release: None,
        };
        let exact = crate::github_mods_page::tests::checked_subset(target);
        run.github = Some(exact.clone());
        let failed = Worker::Done(Err(InstallError::GithubBuild(
            "fixture build stopped".into(),
        )));
        let retry = installs.github_feedback(&failed).unwrap().retry.unwrap();
        assert!(installs.handle(failed));
        // The retry survives losing the worker and owns the exact files and revision it used.
        assert!(Arc::ptr_eq(&retry, &exact));
        assert_eq!(retry.source.commit, "a".repeat(40));
        let GithubSelection::Add(selected) = &retry.selection else {
            panic!("expected a fresh Add");
        };
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].id, "two");
        assert!(selected[0].file.is_file());

        let mut installs = with_running(root.path());
        installs.running.as_mut().unwrap().github = Some(retry);
        let wrong = Worker::Done(Err(InstallError::NotForVersion(
            "fixture version refusal".into(),
        )));
        assert!(installs.github_feedback(&wrong).unwrap().retry.is_none());
    }

    #[test]
    fn a_refused_source_add_opens_one_modal_naming_each_refused_member() {
        let root = homes();
        let mut installs = with_running(root.path());
        let member =
            |id: &str, name: &str| (id.to_owned(), name.to_owned(), format!("someone/{id}"));
        installs.running.as_mut().unwrap().source = Some(RunningSource {
            ids: vec!["jade".into()],
            names: vec!["Jade".into()],
            members: vec![member("jade", "Jade")],
            update: false,
            cancellation: Default::default(),
            rejected: BTreeMap::new(),
        });
        let refused = Worker::Done(Err(InstallError::Source(SourceError::DoesNotFit(
            "jade pins build ed3ac19".into(),
        ))));
        let (_, modal) = installs.refused(&refused).unwrap();
        assert_eq!(modal.mods.len(), 1);
        assert_eq!(modal.mods[0].name, "Jade");
        assert_eq!(modal.mods[0].repo.as_deref(), Some("someone/jade"));
        assert!(modal.mods[0].refusal.said.starts_with("Jade isn't ready"));
        assert_eq!(modal.mods[0].refusal.details, "jade pins build ed3ac19");
        let cancelled = Worker::Done(Err(InstallError::Source(SourceError::Cancelled)));
        assert_eq!(installs.refused(&cancelled), None);

        // A collection that added the rest names only the member it skipped.
        let source = installs.running.as_mut().unwrap().source.as_mut().unwrap();
        source.ids = vec!["jade".into(), "minimap".into()];
        source.names = vec!["Jade".into(), "Minimap".into()];
        source.members = vec![member("jade", "Jade"), member("minimap", "Minimap")];
        source
            .rejected
            .insert("minimap".into(), SourceError::Changed(String::new()));
        let (_, modal) = installs
            .refused(&Worker::Done(Ok(InstalledBatch::default())))
            .unwrap();
        assert_eq!(modal.names, ["Minimap"]);
        assert_eq!(modal.mods[0].name, "Minimap");
        assert!(installs
            .refused(&Worker::Progress(InstallProgress::Checking))
            .is_none());

        // The rest then failing for its own reason is said once more, after the member.
        let failed = Worker::Done(Err(InstallError::Source(SourceError::DoesNotFit(
            "jade pins build ed3ac19".into(),
        ))));
        let (_, modal) = installs.refused(&failed).unwrap();
        let said: Vec<_> = modal.mods.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(said, ["Minimap", "Jade"]);
        assert!(modal.mods[1].refusal.said.starts_with("Jade isn't ready"));
        // Stopping it opens nothing, whatever a member said before.
        let stopped = Worker::Done(Err(InstallError::Source(SourceError::Cancelled)));
        assert_eq!(installs.refused(&stopped), None);
    }

    #[test]
    fn later_clicks_wait_fifo_and_a_repeated_click_queues_once() {
        let root = homes();
        let a = root.path().join("A");
        let b = root.path().join("B");
        let mut installs = with_running(root.path());
        installs.submit(request(&b, "two", "nightly"));
        installs.submit(request(&a, "three", "nightly"));
        installs.handle(Worker::Progress(InstallProgress::Building(Some(0.5))));
        installs.submit(request(&a, "four", "nightly"));
        installs.submit(request(&a, "four", "nightly"));
        assert!(matches!(
            installs.card(&a, "one"),
            Some(Card::Progress(InstallProgress::Building(Some(0.5))))
        ));
        assert!(matches!(installs.card(&a, "three"), Some(Card::Waiting)));
        assert_eq!(
            installs
                .waiting
                .iter()
                .map(|q| q.view.card.as_str())
                .collect::<Vec<_>>(),
            ["two", "three", "four"]
        );
    }

    fn homes() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for id in ["A", "B"] {
            std::fs::create_dir(root.path().join(id)).unwrap();
        }
        root
    }

    #[test]
    fn queued_target_cannot_open_the_replacement_home() {
        let root = homes();
        let a = root.path().join("A");
        let b = root.path().join("B");
        let mut installs = with_running(root.path());
        installs.submit(request(&b, "two", "nightly"));
        installs.submit(request(&a, "three", "stable"));
        assert!(matches!(installs.card(&b, "two"), Some(Card::Waiting)));

        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "mods_page::install::tests::replace_home_worker"])
            .env("FIXTURE_REPLACE_QUEUED_HOME", &b)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        assert_eq!(std::fs::read(b.join("replacement")).unwrap(), b"new owner");
        installs.handle(Worker::Done(Ok(InstalledBatch::default())));
        let queued = installs.waiting.pop_front().unwrap();
        let error = match sunrise_launcher::managed_play::Profile::open_for(
            &queued.profile.home,
            None,
            Some(queued.identity),
        ) {
            Ok(_) => panic!("admitted a replacement"),
            Err(error) => String::from(error),
        };
        assert!(
            error.contains("profile was replaced while the request was waiting"),
            "{error}"
        );
        assert!(!b.join(".install-txn.lock").exists());
        assert!(!b.join("mod-browser").exists());
        let next = installs.waiting.pop_front().unwrap();
        assert_eq!(next.view.card, "three");
        assert_eq!(next.identity, Identity::of(&next.profile.home).unwrap());
    }

    #[test]
    fn replace_home_worker() {
        let Some(home) = std::env::var_os("FIXTURE_REPLACE_QUEUED_HOME").map(PathBuf::from) else {
            return;
        };
        let owner = home.with_extension("owner.lock");
        let _admitted =
            sunrise_launcher::profile_txn::Admission::open(&home, Some(&owner)).unwrap();
        std::fs::rename(&home, home.with_extension("detached")).unwrap();
        std::fs::create_dir(&home).unwrap();
        std::fs::write(home.join("replacement"), b"new owner").unwrap();
    }

    #[test]
    fn a_later_click_cannot_change_the_destination_of_a_queued_retry() {
        use launcher_play::{PlayDestination, PlayIntent};
        let root = homes();
        let mut installs = with_running(root.path());
        let profile = installs.running.as_ref().unwrap().profile.clone();
        let retry = PlayIntent {
            instance: profile.id.clone(),
            incarnation: profile.incarnation,
            account: Some("accepted".into()),
            destination: PlayDestination::Quick("--world", "original world".into()),
        };
        assert!(installs.defer_play(&profile, retry.clone()));
        assert!(!installs.defer_play(
            &profile,
            PlayIntent {
                destination: PlayDestination::Quick("--world", "other world".into()),
                ..retry
            }
        ));
        installs.handle(Worker::Done(Ok(InstalledBatch::default())));
        let (_, accepted) = installs.take_play().unwrap();
        let mut cmd = std::process::Command::new("sunrise-client");
        accepted.destination.apply(&mut cmd).unwrap();
        assert_eq!(
            cmd.get_args().collect::<Vec<_>>(),
            ["--world", "original world"]
        );
        assert_eq!(accepted.account.as_deref(), Some("accepted"));
    }

    #[test]
    fn deferred_play_is_kept_only_for_success_and_failure_is_scoped_to_its_profile() {
        let root = homes();
        for (home, pass, kept) in [("A", true, true), ("A", false, false), ("B", false, true)] {
            let mut installs = with_running(root.path());
            installs.play = Some((
                root.path().join(home),
                launcher_play::PlayIntent {
                    instance: home.into(),
                    incarnation: None,
                    account: Some("accepted".into()),
                    destination: launcher_play::PlayDestination::Home,
                },
            ));
            installs.handle(Worker::Done(if pass {
                Ok(InstalledBatch::default())
            } else {
                Err(InstallError::Download("timeout".into()))
            }));
            assert_eq!(
                installs.take_play().and_then(|(_, intent)| intent.account),
                kept.then(|| "accepted".into())
            );
        }
    }

    #[test]
    fn local_install_stays_tracked_and_honours_cancellation() {
        let root = homes();
        let smod = root.path().join("local-tweaks.smod");
        let bytes = sunrise_launcher_smod::Package::write_files(
            "id = 'local_tweaks'\nname = 'Local Tweaks'\nversion = '0.1.0'\napi = '0.2'\nside = 'both'\n",
            &[],
        )
        .unwrap();
        std::fs::write(&smod, &bytes).unwrap();
        let pkg = sunrise_launcher::github_mods::read_local_smod(&smod).unwrap();

        let mut installs = with_running(root.path());
        let run = installs.running.as_mut().unwrap();
        let target = GithubTarget {
            profile: run.profile.clone(),
            identity: run.identity,
            target: run.target.clone(),
            release: None,
        };
        run.requests = vec![RequestView {
            kind: Kind::Local,
            card: smod.display().to_string(),
            ids: vec![pkg.id.clone()],
        }];
        run.local = Some(Arc::new(LocalInput {
            target,
            label: smod.display().to_string(),
            names: vec![pkg.name.clone()],
            packages: vec![pkg],
        }));

        // Stopping before publication rejects any subsequent world review prompt and suppresses error banners.
        run.cancellation.stop();
        let (reply, answer) = std::sync::mpsc::channel();
        let question = WorldQuestion {
            removal: false,
            cancel_default: true,
            heading: "Review".into(),
            detail: "Detail".into(),
        };
        assert!(!installs.handle(Worker::World(question, reply)));
        assert!(!answer.recv().unwrap());
        assert!(installs
            .github_feedback(&Worker::Done(Err(InstallError::Cancelled)))
            .is_none());

        // Successful completion reports the installed mod name and clears running state.
        let ok = installs
            .github_feedback(&Worker::Done(Ok(InstalledBatch::default())))
            .unwrap();
        assert!(ok.local);
        assert_eq!(ok.result.unwrap(), "Installed Local Tweaks.");
        assert!(installs.handle(Worker::Done(Ok(InstalledBatch::default()))));
        assert!(installs.running.is_none());
    }
}
