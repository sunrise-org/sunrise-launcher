//! One visible registry install batch, and requests that arrived after it was sealed.
use super::*;
use crate::github_mods_page::{Feedback, GithubInput, GithubSelection};
use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
    sync::Mutex,
};
use sunrise_launcher::{
    browser::sunrise::{
        BuildNotice, ChangeImpact, CombinedUpdate, InstallError, InstallHooks, InstallProgress,
        InstalledBatch, NativeQuestion, PreparedSource, ResolveQuestion, SourceError, SourceHooks,
    },
    github_mods::Origin,
    managed_play::FirstBuild,
    native_mods::Cancellation,
    profile_txn::Identity,
};

#[derive(Default)]
pub(crate) struct Installs {
    running: Option<Running>,
    waiting: VecDeque<Request>,
    pub(crate) question: Option<(Decision, Sender<bool>)>,
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
    batch: Arc<Mutex<Option<Vec<String>>>>,
    requests: Vec<RequestView>,
    stage: InstallProgress,
    github: Option<Arc<GithubInput>>,
    source: Option<RunningSource>,
    mixed: bool,
}

struct Request {
    profile: instances::Instance,
    identity: Identity,
    target: Option<String>,
    release: Option<backend::Release>,
    displayed_names: BTreeMap<String, String>,
    view: RequestView,
    github: Option<Arc<GithubInput>>,
    sources: Vec<PreparedSource>,
    mixed_registry: Vec<String>,
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

/// The exact installed base and profile that a read-only source fit check covers.
#[derive(Clone)]
pub(crate) struct SourceTarget {
    profile: instances::Instance,
    identity: Identity,
    base: String,
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
}

pub(crate) struct SourceActivity<'a> {
    pub(crate) ids: &'a [String],
    pub(crate) names: &'a [String],
    pub(crate) progress: InstallProgress,
    pub(crate) stopping: bool,
}

struct RunningSource {
    ids: Vec<String>,
    visible: bool,
    names: Vec<String>,
    cancellation: Cancellation,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Install,
    Github,
    Source,
    Mixed,
    Update,
    UpdateAll,
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

pub(crate) enum Decision {
    Build(BuildNotice),
    Resolve(Arc<ResolveQuestion>),
}

pub(crate) enum Worker {
    SourcePhase,
    Mixed(Box<CombinedUpdate>),
    Progress(InstallProgress),
    Native(NativeQuestion, Sender<Result<bool, String>>),
    Source(SourceQuestion, Sender<Result<bool, String>>),
    Decide(Decision, Sender<bool>),
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

impl Installs {
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
        if let Some(run) = &mut self.running {
            if run.profile.id == request.profile.id
                && run.profile.home == request.profile.home
                && run.identity == request.identity
                && run.target == request.target
                && run.profile.native_profile == request.profile.native_profile
                && request.view.kind == Kind::Install
                && run.requests[0].kind == Kind::Install
            {
                if run
                    .requests
                    .iter()
                    .any(|work| work.card == request.view.card && work.ids == request.view.ids)
                {
                    return;
                }
                let mut batch = run.batch.lock().expect("registry batch");
                if let Some(ids) = batch.as_mut() {
                    ids.extend(request.view.ids.iter().cloned());
                    run.requests.push(request.view);
                    return;
                }
            }
        }
        if !self.waiting.iter().any(|q| {
            q.profile.id == request.profile.id
                && q.profile.home == request.profile.home
                && q.identity == request.identity
                && q.target == request.target
                && q.profile.native_profile == request.profile.native_profile
                && q.view.kind == request.view.kind
                && q.view.card == request.view.card
                && q.view.ids == request.view.ids
                && match (&q.github, &request.github) {
                    (None, None) => true,
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    _ => false,
                }
        }) {
            self.waiting.push_back(request);
        }
    }

    pub(super) fn github_feedback(&self, event: &Worker) -> Option<Feedback> {
        let run = self.running.as_ref()?;
        let input = run.github.as_ref()?;
        let Worker::Done(result) = event else {
            return None;
        };
        let update = matches!(input.selection, GithubSelection::Update(_));
        let retry = match result {
            Err(
                InstallError::Download(_)
                | InstallError::GithubBuild(_)
                | InstallError::Source(SourceError::Tools(_) | SourceError::Build { .. }),
            ) if !update => Some(input.clone()),
            _ => None,
        };
        let result = match result {
            Ok(_) => Ok(if update {
                tr("Updated the GitHub mod.")
            } else {
                tr("Installed the selected GitHub mods.")
            }
            .into()),
            Err(InstallError::Cancelled | InstallError::Source(SourceError::Cancelled)) => {
                return None
            }
            Err(error @ InstallError::Source(_)) => {
                Err((failure_line(error).into(), String::new()))
            }
            Err(error) => Err((failure_line(error).into(), error.to_string())),
        };
        Some(Feedback {
            profile: run.profile.clone(),
            link: input.link.clone(),
            result,
            retry,
        })
    }

    pub(super) fn mutation_failure(&self, event: &Worker) -> Option<String> {
        let run = self.running.as_ref()?;
        if let Worker::Mixed(done) = event {
            let mut failures = Vec::new();
            match &done.registry {
                Ok(batch) => {
                    if let Some(notice) = &batch.failure {
                        failures.extend(notice.changes.iter().map(|change| change.title.clone()));
                    }
                }
                Err(InstallError::Cancelled) => (),
                Err(_) => failures.extend(run.requests[0].ids.iter().cloned()),
            }
            if let Err(error) = &done.sources {
                if !matches!(error, SourceError::Cancelled) || done.registry.is_ok() {
                    failures.extend(run.source.as_ref()?.names.iter().cloned());
                }
            }
            return (!failures.is_empty()).then(|| {
                trf(
                    "These mods didn't update: {mods}",
                    &[("mods", &crate::mods::names(&failures))],
                )
            });
        }
        if matches!(
            run.requests[0].kind,
            Kind::Install | Kind::Github | Kind::Source | Kind::Mixed
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
            Worker::SourcePhase => {
                if let Some(run) = &mut self.running {
                    if let Some(source) = &mut run.source {
                        source.visible = true;
                    }
                    run.stage = InstallProgress::Checking;
                }
            }
            Worker::Mixed(done) => {
                let Some(run) = &mut self.running else {
                    return false;
                };
                let Some(source) = run.source.take() else {
                    return false;
                };
                self.source_question.take();
                if let Err(error) = &done.sources {
                    if !matches!(error, SourceError::Cancelled) {
                        eprintln!("Nightly source update batch: {error}");
                    }
                }
                for id in source.ids {
                    self.source_outcomes
                        .insert((run.identity, id), done.sources.clone());
                }
                if done.sources.is_err()
                    && self.play.as_ref().is_some_and(|(home, intent)| {
                        *home == run.profile.home && intent.instance == run.profile.id
                    })
                {
                    self.play = None;
                }
                run.requests.truncate(1);
                return self.handle(Worker::Done(done.registry));
            }
            Worker::Progress(step) => {
                if let Some(run) = &mut self.running {
                    run.stage = step;
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
                    .and_then(|run| run.source.as_ref())
                    .is_some_and(|source| source.cancellation.is_stopped())
                {
                    let _ = reply.send(false);
                    return false;
                }
                if let Some((_, old)) = self.world.replace((impact, reply)) {
                    let _ = old.send(false);
                }
            }
            Worker::Native(..) | Worker::Source(..) => {
                unreachable!("handled at the launcher consent boundary")
            }
            Worker::Done(result) => {
                let Some(run) = self.running.take() else {
                    return false;
                };
                let ready = result.is_ok();
                if run.source.is_some() {
                    self.source_question.take();
                    let outcome = result.map(|_| ()).map_err(SourceError::from);
                    if let Err(error) = &outcome {
                        if !matches!(error, SourceError::Cancelled) {
                            eprintln!("Nightly source edit: {error}");
                        }
                    }
                    for id in &run.requests[0].ids {
                        self.source_outcomes
                            .insert((run.identity, id.clone()), outcome.clone());
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
                                failure: done.failure.as_ref().map(|notice| {
                                    let this = notice
                                        .changes
                                        .iter()
                                        .find(|change| change.project == request.card);
                                    let label = match this {
                                        Some(change) if change.predecessor.is_some() => {
                                            tr("Kept the working version").into()
                                        }
                                        Some(change) => change.reason.label(),
                                        None => {
                                            tr("Some mods stayed off or kept their old version")
                                                .into()
                                        }
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
                                }),
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

    pub(super) fn take_play(&mut self) -> Option<(PathBuf, launcher_play::PlayIntent)> {
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
        self.queue_all_mod_updates(target, Vec::new(), BTreeMap::new(), inputs)
    }

    pub(crate) fn queue_all_mod_updates(
        &mut self,
        target: SourceTarget,
        registry: Vec<String>,
        displayed_names: BTreeMap<String, String>,
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
            displayed_names,
            view: RequestView {
                kind: if registry.is_empty() {
                    Kind::Source
                } else {
                    Kind::Mixed
                },
                card: ids[0].clone(),
                ids,
            },
            github: None,
            sources: inputs,
            mixed_registry: registry,
        });
        self.start_registry_queue();
        Ok(())
    }

    pub(crate) fn mixed_update_active(&self) -> bool {
        self.mods
            .installs
            .running
            .as_ref()
            .is_some_and(|run| run.mixed)
    }

    pub(crate) fn source_activity(&self) -> Option<SourceActivity<'_>> {
        let run = self.mods.installs.running.as_ref()?;
        let source = run.source.as_ref().filter(|source| source.visible)?;
        Some(SourceActivity {
            ids: &source.ids,
            names: &source.names,
            progress: run.stage,
            stopping: source.cancellation.is_stopped(),
        })
    }

    pub(crate) fn stop_source(&mut self) {
        let Some(source) = self
            .mods
            .installs
            .running
            .as_ref()
            .and_then(|run| run.source.as_ref())
        else {
            return;
        };
        source.cancellation.stop();
        if source.cancellation.is_stopped() {
            self.answer_source_question(Ok(false));
            if let Some((_, reply)) = self.mods.installs.world.take() {
                let _ = reply.send(false);
            }
        }
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
        } else if self.source_already_allowed(&question) {
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
        let outcome = self.registry_outcome(id)?;
        if let Some((label, _)) = outcome.failure {
            return Some(label);
        }
        if !outcome.added.is_empty() {
            return Some(trf(
                "Also installed: {mods}",
                &[("mods", &crate::mods::names(&outcome.added))],
            ));
        }
        outcome.updated.first().map(|(name, from, to)| {
            trf(
                "Also updated: {mod} {from} → {to}",
                &[("mod", name), ("from", from), ("to", to)],
            )
        })
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
        if lines.is_empty() && done.failure.is_none() {
            return None;
        }
        let failure = done.failure.clone().map(|(label, detail)| {
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
            .is_some_and(|instance| self.mods.installs.runs_for(instance))
            && self.mods.installs.play.is_none()
    }

    pub(crate) fn defer_registry_play(
        &mut self,
        instance: &instances::Instance,
        intent: launcher_play::PlayIntent,
    ) {
        self.mods.installs.play = Some((instance.home.clone(), intent));
        self.open_sheet(crate::sheets::Sheet::RegistryPlay);
    }

    pub(crate) fn registry_stage(&self) -> Option<InstallProgress> {
        self.mods.installs.stage()
    }

    /// The clicked profile and initial build selection travel together into the worker, even
    /// if the user switches pages or selections while the request waits.
    pub(crate) fn queue_registry_install(
        &mut self,
        card: String,
        ids: Vec<String>,
    ) -> Result<(), String> {
        self.queue_registry_change(Kind::Install, card, ids, BTreeMap::new())
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

    pub(crate) fn queue_github(&mut self, input: Arc<GithubInput>) -> Result<(), String> {
        let target = &input.target;
        if !target.same_destination(&self.github_target()?) {
            return Err(tr(
                "The profile or game build changed. Check the source again; nothing was changed.",
            )
            .into());
        }
        let (kind, ids) = match &input.selection {
            GithubSelection::Add(packages) => (
                Kind::Github,
                packages.iter().map(|p| p.id.clone()).collect(),
            ),
            GithubSelection::Update(package) => (Kind::Github, vec![package.id.clone()]),
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
            sources: Vec::new(),
            mixed_registry: Vec::new(),
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
        if kind == Kind::Install
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
            sources: Vec::new(),
            mixed_registry: Vec::new(),
        });
        self.start_registry_queue();
        Ok(())
    }

    pub(super) fn start_registry_queue(&mut self) -> bool {
        if self.mods_locked().is_some()
            || self.mutating()
            || self.pending_consent.is_some()
            || self.mods.installs.question.is_some()
            || self.mods.installs.world.is_some()
        {
            return false;
        }
        let Some(mut request) = self.mods.installs.waiting.pop_front() else {
            return false;
        };
        let batch = Arc::new(Mutex::new(
            (request.view.kind == Kind::Install).then(|| request.view.ids.clone()),
        ));
        let kind = request.view.kind;
        let ids = request.view.ids.clone();
        let cancellation = Cancellation::default();
        let mixed = kind == Kind::Mixed;
        let source = matches!(kind, Kind::Source | Kind::Mixed).then(|| RunningSource {
            ids: ids.clone(),
            visible: !mixed,
            names: request
                .source_inputs()
                .iter()
                .map(|source| source.name().into())
                .collect(),
            cancellation: cancellation.clone(),
        });
        let mut views = Vec::new();
        if mixed {
            views.push(RequestView {
                kind: Kind::Mixed,
                card: "mixed-registry".into(),
                ids: request.mixed_registry.clone(),
            });
        }
        views.push(request.view.clone());
        self.mods.installs.running = Some(Running {
            profile: request.profile.clone(),
            identity: request.identity,
            target: request.target.clone(),
            batch: batch.clone(),
            requests: views,
            mixed,
            stage: InstallProgress::Checking,
            source,
            github: request.github.clone(),
        });
        let tx = self.mods.tx.clone();
        let launcher = self.tx.clone();
        std::thread::spawn(move || {
            let post = |event| tx.send(Event::Registry(event));
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
            let resolve = |question: &ResolveQuestion| {
                let (reply, answer) = std::sync::mpsc::channel();
                post(Worker::Decide(
                    Decision::Resolve(Arc::new(question.clone())),
                    reply,
                ))
                .is_ok()
                    && answer.recv().unwrap_or(false)
            };
            let hooks = InstallHooks {
                progress: &|step| {
                    let _ = post(Worker::Progress(step));
                },
                native: &|question| {
                    let (reply, answer) = std::sync::mpsc::channel();
                    if post(Worker::Native(question.clone(), reply)).is_err() {
                        return Ok(false);
                    }
                    answer.recv().unwrap_or(Ok(false))
                },
                registry_crates: Some(&|consent| {
                    let (reply, answer) = std::sync::mpsc::channel();
                    launcher
                        .send(Msg::NativeRegistryConsent(consent.clone(), reply))
                        .is_ok()
                        && answer.recv().unwrap_or(false)
                }),
                continue_after_failure: &|notice| {
                    let (reply, answer) = std::sync::mpsc::channel();
                    post(Worker::Decide(Decision::Build(notice.clone()), reply)).is_ok()
                        && answer.recv().unwrap_or(false)
                },
                resolve: (kind == Kind::Install)
                    .then_some(&resolve as &dyn Fn(&ResolveQuestion) -> bool),
            };
            let review = |impact: &ChangeImpact, sources: Option<&ChangeImpact>| {
                if cancellation.is_stopped() {
                    return false;
                }
                let (reply, answer) = std::sync::mpsc::channel();
                let mut question = WorldQuestion::new(impact, &request.displayed_names);
                if let Some(sources) = sources {
                    let next = WorldQuestion::new(sources, &request.displayed_names);
                    question.heading = tr("Review mod updates").into();
                    question.detail = format!(
                        "{}\n{}\n\n{}\n{}",
                        tr("Registry mods"),
                        question.detail,
                        tr("Source mods"),
                        next.detail
                    );
                    question.cancel_default |= next.cancel_default;
                }
                post(Worker::World(question, reply)).is_ok() && answer.recv().unwrap_or(false)
            };
            let approve = |impact: &ChangeImpact| review(impact, None);
            let authorize = |prepared: &[PreparedSource]| {
                let question = SourceQuestion {
                    base: prepared[0].base().into(),
                    mods: prepared
                        .iter()
                        .map(|item| (item.name().into(), item.origin().clone()))
                        .collect(),
                };
                let (reply, answer) = std::sync::mpsc::channel();
                if post(Worker::Source(question, reply)).is_err() {
                    return Ok(false);
                }
                answer.recv().unwrap_or(Ok(false))
            };
            let result = match kind {
                Kind::Install => sunrise::install_registry(
                    &request.profile,
                    request.identity,
                    || {
                        batch
                            .lock()
                            .expect("registry batch")
                            .take()
                            .expect("sealed once")
                    },
                    choose,
                    hooks,
                ),
                Kind::Github => {
                    let input = request.github.as_ref().expect("a checked GitHub request");
                    match &input.selection {
                        GithubSelection::Update(package) => sunrise::update_github(
                            &request.profile,
                            request.identity,
                            &input.source,
                            package,
                            choose,
                            hooks,
                            &approve,
                        )
                        .map(|_| InstalledBatch::default()),
                        GithubSelection::Add(packages) => sunrise::add_github(
                            &request.profile,
                            request.identity,
                            &input.source,
                            packages,
                            request.target.as_deref(),
                            choose,
                            hooks,
                        )
                        .map(|_| InstalledBatch::default()),
                        GithubSelection::Source(_) => {
                            unreachable!("source installs use the source worker")
                        }
                    }
                }
                Kind::Source => sunrise::apply_sources(
                    &request.profile,
                    request.identity,
                    request.source_inputs(),
                    choose,
                    SourceHooks {
                        install: hooks,
                        cancellation: &cancellation,
                        authorize: &authorize,
                        approve_worlds: &approve,
                    },
                )
                .map(|()| InstalledBatch::default())
                .map_err(InstallError::Source),
                Kind::Mixed => {
                    let done = sunrise::update_registry_and_sources(
                        &request.profile,
                        request.identity,
                        request.mixed_registry,
                        &mut request.sources,
                        choose,
                        hooks,
                        hooks,
                        &cancellation,
                        &authorize,
                        &review,
                        || {
                            let _ = post(Worker::SourcePhase);
                        },
                    );
                    let _ = post(Worker::Mixed(Box::new(done)));
                    return;
                }
                Kind::Update | Kind::UpdateAll => sunrise::update_registry(
                    &request.profile,
                    request.identity,
                    if kind == Kind::Update {
                        sunrise::UpdateRequest::Strict(ids)
                    } else {
                        sunrise::UpdateRequest::Bulk(ids)
                    },
                    choose,
                    hooks,
                    &approve,
                ),
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
        InstallProgress::Checking => tr("Checking the mod set…").into(),
        InstallProgress::Downloading(f) => trf(
            "Downloading {percent}%",
            &[("percent", &format!("{:.0}", f * 100.))],
        ),
        InstallProgress::Preparing => tr("Getting ready…").into(),
        InstallProgress::Rust(fraction) => trf(
            "Downloading build tools {percent}%",
            &[("percent", &format!("{:.0}", fraction * 100.0))],
        ),
        InstallProgress::Tools { total, .. } => trf(
            "Getting build tools ({size})",
            &[("size", &sunrise_launcher::storage::format_bytes(total))],
        ),
        InstallProgress::Building(_) => tr("Building").into(),
        InstallProgress::Publishing => tr("Finishing…").into(),
        InstallProgress::Reviewing => tr("Review changes…").into(),
    }
}

pub(crate) fn stage_fraction(step: InstallProgress) -> Option<f32> {
    match step {
        InstallProgress::Downloading(f) | InstallProgress::Rust(f) => Some(f),
        InstallProgress::Building(f) => f,
        InstallProgress::Tools { done, total } if total > 0 => Some(done as f32 / total as f32),
        _ => None,
    }
}

fn failure_line(error: &InstallError) -> &'static str {
    match error {
        InstallError::Refused(_) => tr("Can't install these mods together"),
        InstallError::Host(_) => tr("Native mods can't build on this computer"),
        InstallError::Unproved(_) => tr("Couldn't check that the mods fit"),
        InstallError::NotForVersion(_) => tr("Not available for your version"),
        InstallError::Source(SourceError::DoesNotFit(_)) => tr("Not available for your version"),
        InstallError::Source(SourceError::Dependency { .. }) => {
            tr("Can't install these mods together")
        }
        InstallError::Source(SourceError::Unproved(_) | SourceError::Github(_)) => {
            tr("Fit could not be verified")
        }
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
        InstallError::Cancelled => tr("Install cancelled."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(home: &Path, id: &str, target: &str) -> Request {
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
            sources: Vec::new(),
            mixed_registry: Vec::new(),
            view: RequestView {
                kind: Kind::Install,
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
                batch: Arc::new(Mutex::new(Some(first.view.ids.clone()))),
                requests: vec![first.view],
                stage: InstallProgress::Checking,
                github: None,
                source: None,
                mixed: false,
            }),
            ..Installs::default()
        }
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
    fn visible_matching_batch_joins_before_sealing_only_and_later_clicks_wait_fifo() {
        let root = homes();
        let a = root.path().join("A");
        let b = root.path().join("B");
        let mut installs = with_running(root.path());
        installs.submit(request(&b, "two", "nightly"));
        installs.submit(request(&a, "three", "nightly"));
        installs.submit(request(&a, "four", "stable"));
        let joined = installs
            .running
            .as_ref()
            .unwrap()
            .batch
            .lock()
            .unwrap()
            .take()
            .unwrap();
        assert_eq!(joined, ["one", "three"]);
        installs.handle(Worker::Progress(InstallProgress::Building(Some(0.5))));
        installs.submit(request(&a, "five", "nightly"));
        installs.submit(request(&a, "five", "nightly"));
        assert!(matches!(
            installs.card(&a, "three"),
            Some(Card::Progress(InstallProgress::Building(Some(0.5))))
        ));
        assert!(matches!(installs.card(&a, "five"), Some(Card::Waiting)));
        assert_eq!(
            installs
                .waiting
                .iter()
                .map(|q| q.view.card.as_str())
                .collect::<Vec<_>>(),
            ["two", "four", "five"]
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
}
