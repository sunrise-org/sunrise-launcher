//! One visible registry install batch, and requests that arrived after it was sealed.
use super::*;
use std::{collections::VecDeque, path::Path, sync::Mutex};
use sunrise_launcher::{
    browser::sunrise::{
        BuildNotice, InstallError, InstallHooks, InstallProgress, InstalledBatch, NativeQuestion,
    },
    managed_play::FirstBuild,
    profile_txn::Identity,
};

#[derive(Default)]
pub(crate) struct Installs {
    running: Option<Running>,
    waiting: VecDeque<Request>,
    pub(crate) question: Option<(BuildNotice, Sender<bool>)>,
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
}

struct Request {
    profile: instances::Instance,
    identity: Identity,
    target: Option<String>,
    release: Option<backend::Release>,
    view: RequestView,
}

struct RequestView {
    card: String,
    ids: Vec<String>,
}

pub(crate) enum Worker {
    Progress(InstallProgress),
    Native(NativeQuestion, Sender<Result<bool, String>>),
    Build(BuildNotice, Sender<bool>),
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
                && q.view.card == request.view.card
                && q.view.ids == request.view.ids
        }) {
            self.waiting.push_back(request);
        }
    }

    pub(super) fn handle(&mut self, event: Worker) -> bool {
        match event {
            Worker::Progress(step) => {
                if let Some(run) = &mut self.running {
                    run.stage = step;
                }
            }
            Worker::Build(notice, reply) => {
                if let Some((_, old)) = self.question.replace((notice, reply)) {
                    let _ = old.send(false);
                }
            }
            Worker::Native(..) => unreachable!("native questions use the native consent sheet"),
            Worker::Done(result) => {
                let Some(run) = self.running.take() else {
                    return false;
                };
                let ready = result.is_ok();
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
                                        tr("Some mods stayed off or kept their old version").into()
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
        if ids.is_empty() {
            return Ok(());
        }
        if self.accounts.selected().is_none() && !sunrise_launcher::developer_offline::selected() {
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
        let target = self.profile_target(&instance).cloned();
        let release = target
            .as_deref()
            .and_then(|tag| self.release_to_install(tag));
        self.mods.installs.submit(Request {
            identity: sunrise::install_target(&instance)?,
            profile: instance,
            target,
            release,
            view: RequestView { card, ids },
        });
        self.start_registry_queue();
        Ok(())
    }

    pub(super) fn start_registry_queue(&mut self) -> bool {
        if self.mods_locked().is_some()
            || self.mutating()
            || self.pending_consent.is_some()
            || self.mods.installs.question.is_some()
        {
            return false;
        }
        let Some(request) = self.mods.installs.waiting.pop_front() else {
            return false;
        };
        let batch = Arc::new(Mutex::new(Some(request.view.ids.clone())));
        self.mods.installs.running = Some(Running {
            profile: request.profile.clone(),
            identity: request.identity,
            target: request.target.clone(),
            batch: batch.clone(),
            requests: vec![request.view],
            stage: InstallProgress::Checking,
        });
        let tx = self.mods.tx.clone();
        let launcher = self.tx.clone();
        std::thread::spawn(move || {
            let post = |event| tx.send(Event::Registry(event));
            let result = sunrise::install_registry(
                &request.profile,
                request.identity,
                || {
                    batch
                        .lock()
                        .expect("registry batch")
                        .take()
                        .expect("sealed once")
                },
                || {
                    if let Some(tag) = request
                        .target
                        .as_deref()
                        .filter(|tag| backend::installed().iter().any(|local| local == tag))
                    {
                        FirstBuild::installed(tag)
                    } else if let Some(release) = request.release {
                        FirstBuild::download(&release)
                    } else {
                        Err(backend::no_version_installed())
                    }
                },
                InstallHooks {
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
                        post(Worker::Build(notice.clone(), reply)).is_ok()
                            && answer.recv().unwrap_or(false)
                    },
                },
            );
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
        InstallProgress::Tools { total, .. } => trf(
            "Getting build tools ({size})",
            &[("size", &sunrise_launcher::storage::format_bytes(total))],
        ),
        InstallProgress::Building(_) => tr("Building").into(),
        InstallProgress::Publishing => tr("Finishing install…").into(),
    }
}

pub(crate) fn stage_fraction(step: InstallProgress) -> Option<f32> {
    match step {
        InstallProgress::Downloading(f) => Some(f),
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
            view: RequestView {
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
            }),
            ..Installs::default()
        }
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
            error.contains("profile was replaced while Install was waiting"),
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
