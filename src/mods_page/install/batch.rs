//! The batch's state machine: requests in, worker events, what each card says.
use super::*;

impl Installs {
    /// The build an install running on `profile` captured when it was queued, whatever its
    /// source: the profile keeps showing it while the install holds the profile.
    pub(super) fn base(&self, profile: &instances::Instance) -> Option<&str> {
        let run = self.running.as_ref()?;
        (run.profile.id == profile.id && run.profile.home == profile.home)
            .then_some(run.target.as_deref())
            .flatten()
    }

    pub(in crate::mods_page) fn stage(&self) -> Option<InstallProgress> {
        self.running.as_ref().map(|run| run.stage)
    }

    pub(in crate::mods_page) fn card(&self, home: &Path, id: &str) -> Option<Card<'_>> {
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

    pub(super) fn submit(&mut self, request: Request) {
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
    pub(in crate::mods_page) fn removed(&self, event: &Worker) -> Option<(&Path, &[String])> {
        let run = self.running.as_ref()?;
        let request = run.requests.first().filter(|r| r.kind == Kind::Remove)?;
        matches!(event, Worker::Done(Ok(_))).then_some((&run.profile.home, &request.ids))
    }

    pub(in crate::mods_page) fn github_feedback(&self, event: &Worker) -> Option<Feedback> {
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
    pub(in crate::mods_page) fn refused(
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
    pub(in crate::mods_page) fn install_failure(
        &self,
        event: &Worker,
    ) -> Option<(instances::Instance, String)> {
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

    pub(in crate::mods_page) fn mutation_failure(&self, event: &Worker) -> Option<String> {
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

    pub(in crate::mods_page) fn handle(&mut self, event: Worker) -> bool {
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
                                                    ("reason", &sunrise_launcher::browser::installed::hold_detail(&change.reason)),
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

    pub(super) fn runs_for(&self, instance: &instances::Instance) -> bool {
        self.running
            .as_ref()
            .is_some_and(|run| run.profile.id == instance.id && run.profile.home == instance.home)
    }

    pub(super) fn accepts_play(&self, instance: &instances::Instance) -> bool {
        self.play.is_none() && self.runs_for(instance)
    }

    pub(super) fn defer_play(
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

    pub(in crate::mods_page) fn take_play(
        &mut self,
    ) -> Option<(PathBuf, launcher_play::PlayIntent)> {
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
