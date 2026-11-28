//! The work under way, gathered from wherever it runs into one list the status strip draws on
//! every page: what it does, its step, how far it is, the time left and how to stop it.
//!
//! Nothing here starts or owns work. Each job is read from the state its own code keeps, so a
//! page switch never touches it, and the list only says what is already true.
use super::*;
use crate::eta::StageEta;
use crate::frame::Page;
use crate::launch_state::Main;
use crate::mods_page::install::{stage_fraction, stage_label};
use std::collections::HashMap;
use sunrise_launcher::browser::sunrise::InstallProgress;

/// One piece of work under way, as every page shows it.
pub(crate) struct Job {
    /// Tells the job apart from the others, for the time left measured for it.
    pub(super) key: &'static str,
    /// What the work does: "Installing Nov 17".
    pub(super) what: String,
    /// The step it is at, when it has steps: "Downloading Minecraft's files (210 of 3,480)".
    pub(super) step: Option<String>,
    /// How far it is, when that is known.
    pub(super) fraction: Option<f32>,
    pub(super) eta: Option<Duration>,
    /// The work keeps no time left of its own: the strip measures one from `fraction`.
    measure: bool,
    /// How it is stopped, when it can be.
    pub(super) stop: Option<JobAct>,
    /// It was asked to stop and is winding down.
    pub(super) stopping: bool,
    /// Where the work has more to show (its window, sheet or page).
    pub(super) show: Option<JobAct>,
}

impl Job {
    fn new(key: &'static str, what: impl Into<String>) -> Self {
        Self {
            key,
            what: what.into(),
            step: None,
            fraction: None,
            eta: None,
            measure: false,
            stop: None,
            stopping: false,
            show: None,
        }
    }

    fn step(mut self, step: impl Into<String>) -> Self {
        self.step = Some(step.into());
        self
    }

    /// A known fraction, and that the strip measures the time left from it.
    fn measured(mut self, fraction: f32) -> Self {
        self.fraction = Some(fraction.clamp(0., 1.));
        self.measure = true;
        self
    }

    fn stop(mut self, act: JobAct, stopping: bool) -> Self {
        self.stop = Some(act);
        self.stopping = stopping;
        self
    }

    fn show(mut self, act: JobAct) -> Self {
        self.show = Some(act);
        self
    }
}

/// What a job's Stop or Show does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum JobAct {
    StopGameDownload,
    /// Stops the preparation the player started.
    StopPrepare,
    /// Stops Play on its way to the game (its download, preparation and mod build).
    StopStart,
    StopLauncherUpdate,
    /// Stops copying another launcher's profile into a new one.
    StopImport,
    StopRegistryInstall,
    StopSourceBuild,
    StopNativeBuild,
    /// Cancels a build switch (`true`) or a provider switch.
    CancelSwitch(bool),
    ShowNativeBuild,
    ShowSwitch(bool),
    ShowLauncherUpdate,
    Go(Page),
}

/// The strip's own state: whether its list is open, and the time left it measures for work that
/// keeps none (an EMA of each job's progress, `eta::StageEta`).
#[derive(Default)]
pub(crate) struct Jobs {
    pub(super) open: bool,
    measured: HashMap<&'static str, StageEta>,
}

impl Launcher {
    /// Every piece of work under way: switches first, then what Play waits for, then the rest.
    pub(crate) fn jobs(&mut self) -> Vec<Job> {
        let mut jobs = Vec::new();
        self.switch_jobs(&mut jobs);
        self.game_jobs(&mut jobs);
        self.mod_jobs(&mut jobs);
        self.other_jobs(&mut jobs);
        let now = Instant::now();
        let measured = &mut self.jobs.measured;
        measured.retain(|key, _| jobs.iter().any(|job| job.measure && job.key == *key));
        for job in jobs.iter_mut().filter(|job| job.measure) {
            let eta = measured.entry(job.key).or_default();
            eta.update(0, job.fraction, now);
            job.eta = eta.remaining(now);
        }
        if jobs.len() < 2 {
            self.jobs.open = false;
        }
        jobs
    }

    /// The game build's download, its preparation and the game starting.
    fn game_jobs(&self, jobs: &mut Vec<Job>) {
        let downloading = self.switch_download();
        if let Some((tag, fraction)) = self
            .installing
            .as_ref()
            .filter(|(tag, _)| downloading != Some(tag.as_str()))
        {
            let stopping = self.install_stop.as_ref().is_some_and(|s| s.is_stopped());
            let job = Job::new(
                "install",
                trf(
                    "Installing {build}",
                    &[("build", &crate::sheets::build_name(tag))],
                ),
            )
            .step(if stopping {
                tr("Stopping…")
            } else {
                tr("Downloading")
            })
            .measured(*fraction);
            jobs.push(match self.install_stop {
                Some(_) => job.stop(JobAct::StopGameDownload, stopping),
                None => job,
            });
        }
        // A build switch prepares its build itself, and its own row says each step.
        let switching = jobs.iter().any(|job| job.key == "switch");
        if self.preparing && !switching {
            let job = prepare_job(tr("Preparing the game"), ["prepare", "prepare-java"]);
            jobs.push(match &self.prepare_stop {
                Some(stop) => stoppable(job, JobAct::StopPrepare, stop),
                None => job,
            });
        } else if self.starting_client.is_some() || self.play_admission.is_some() {
            let job = self.start_job();
            // Only the admitted worker holds a stop; a pack's older start has none.
            jobs.push(
                match (&self.native_cancellation, self.starting_client.is_some()) {
                    (Some(stop), true) => stoppable(job, JobAct::StopStart, stop),
                    _ => job,
                },
            );
        }
    }

    /// Play on its way to the game. Its download says how far it is, and its preparation the
    /// same steps and counts as Prepare's (sunrise-org/sunrise-launcher#417): never a bare
    /// "Getting ready…" that doesn't move.
    fn start_job(&self) -> Job {
        use sunrise_launcher::backend::PlayStep;
        let what = tr("Starting the game");
        match self.play_step {
            Some(PlayStep::Prepare) => prepare_job(what, ["start-prepare", "start-java"]),
            Some(PlayStep::Download) => {
                let job = Job::new("start-download", what)
                    .step(Main::Step(PlayStep::Download).label().to_string());
                match self.play_download {
                    Some(fraction) => job.measured(fraction),
                    None => job,
                }
            }
            step => Job::new("start", what)
                .step(step.map_or(Main::Starting, Main::Step).label().to_string()),
        }
    }

    /// Registry installs and source builds, native builds and the Mods page's own changes.
    fn mod_jobs(&self, jobs: &mut Vec<Job>) {
        if let Some(activity) = self.source_activity() {
            let step = activity.progress;
            let publishing = step == InstallProgress::Publishing;
            let job = Job::new(
                "source",
                trf(
                    "Building {mods}",
                    &[("mods", &crate::mods::names(activity.names))],
                ),
            )
            .step(if activity.stopping {
                tr("Stopping…").into()
            } else {
                step_name(step)
            });
            let job = Job {
                fraction: stage_fraction(step),
                eta: (!publishing && !activity.stopping)
                    .then(|| self.registry_eta())
                    .flatten(),
                ..job
            };
            jobs.push(if publishing {
                job
            } else {
                job.stop(JobAct::StopSourceBuild, activity.stopping)
            });
        } else if let Some((stage, what)) = self.registry_stage().zip(self.registry_title()) {
            let stopping = self.registry_install_stopping();
            let job = Job {
                fraction: stage_fraction(stage),
                eta: self.registry_eta().filter(|_| !stopping),
                ..Job::new("registry", what).step(if stopping {
                    tr("Stopping…").into()
                } else {
                    step_name(stage)
                })
            };
            jobs.push(match stage {
                InstallProgress::Publishing => job,
                _ => job.stop(JobAct::StopRegistryInstall, stopping),
            });
        }
        if let Some(run) = (self.mods_state.build.as_ref())
            .filter(|run| run.running() && self.native_step.is_some())
        {
            let job = Job::new("native", tr("Building your mods"))
                .measured(run.fraction())
                .stop(JobAct::StopNativeBuild, false)
                .show(JobAct::ShowNativeBuild);
            jobs.push(match run.at() {
                Some((at, total)) => job.step(trf(
                    "{done} of {total}",
                    &[("done", &at.to_string()), ("total", &total.to_string())],
                )),
                None => job,
            });
        }
        if let Some((line, fraction)) = self.mods_change() {
            let job = Job::new("mods", line);
            // A change starts at 0 and says so until its worker reports a first step.
            jobs.push(if fraction > 0. {
                job.measured(fraction)
            } else {
                job
            });
        }
    }

    fn switch_jobs(&self, jobs: &mut Vec<Job>) {
        for switch in self.switch_activities() {
            let publishing = switch.progress == InstallProgress::Publishing;
            let step = if switch.stopping {
                tr("Cancelling…").into()
            } else if switch.asking {
                tr("Waiting for your answer").into()
            } else {
                step_name(switch.progress)
            };
            let job = Job {
                fraction: stage_fraction(switch.progress),
                eta: switch.eta.filter(|_| !switch.stopping && !switch.asking),
                ..Job::new(
                    if switch.build { "switch" } else { "provider" },
                    switch.what,
                )
                .step(step)
                .show(JobAct::ShowSwitch(switch.build))
            };
            jobs.push(if publishing {
                job
            } else {
                job.stop(JobAct::CancelSwitch(switch.build), switch.stopping)
            });
        }
    }

    /// The launcher's own update, Storage's work and the servers this launcher hosts.
    fn other_jobs(&self, jobs: &mut Vec<Job>) {
        if let SelfUpdate::Downloading(release, fraction) = &self.self_update {
            let version = backend::launcher_version(&release.tag).unwrap_or(release.tag.as_str());
            jobs.push(
                Job::new(
                    "update",
                    trf(
                        "Updating the launcher to {version}",
                        &[("version", version)],
                    ),
                )
                .step(tr("Downloading"))
                .measured(*fraction)
                .show(JobAct::ShowLauncherUpdate),
            );
            if let (Some(job), Some(stop)) = (jobs.last_mut(), &self.update_stop) {
                job.stop = Some(JobAct::StopLauncherUpdate);
                job.stopping = stop.is_stopped();
                if job.stopping {
                    job.step = Some(tr("Stopping…").into());
                }
            }
        }
        if let Some(import) = &self.instances.importing {
            let stopping = import.cancel.is_stopped();
            let job = Job::new(
                "import-profile",
                trf("Importing {name}", &[("name", &import.name)]),
            )
            .step(if stopping {
                tr("Stopping…")
            } else {
                tr("Copying")
            });
            let job = match import.progress.fraction() {
                Some(fraction) => job.measured(fraction),
                None => job,
            };
            jobs.push(job.stop(JobAct::StopImport, stopping));
        }
        for tag in &self.removing {
            jobs.push(
                Job::new(
                    "remove",
                    trf(
                        "Removing {name}…",
                        &[("name", &crate::sheets::build_name(tag))],
                    ),
                )
                .show(JobAct::Go(Page::Settings)),
            );
        }
        if self.importing_build {
            jobs.push(
                Job::new("import", tr("Importing a game build")).show(JobAct::Go(Page::Settings)),
            );
        }
        if self.storage_pending && self.storage_clearing {
            jobs.push(Job::new("clear", tr("Clearing downloads")).show(JobAct::Go(Page::Settings)));
        }
        if let Some((what, line)) = self.servers.job() {
            let job = Job::new("server", what).show(JobAct::Go(Page::Hosting));
            jobs.push(match line {
                Some(line) => job.step(line),
                None => job,
            });
        }
    }

    /// Stop or Show on a job's row.
    pub(crate) fn job_act(&mut self, act: JobAct) {
        match act {
            JobAct::StopGameDownload => {
                if let Some(stop) = &self.install_stop {
                    stop.stop();
                }
            }
            JobAct::StopPrepare => self.halt(crate::launch_state::Halt::Prepare),
            JobAct::StopStart => self.halt(crate::launch_state::Halt::Start),
            JobAct::StopLauncherUpdate => {
                if let Some(stop) = &self.update_stop {
                    stop.stop();
                }
            }
            JobAct::StopImport => {
                if let Some(job) = &self.instances.importing {
                    job.cancel.stop();
                }
            }
            JobAct::StopRegistryInstall => self.stop_registry_install(),
            JobAct::StopSourceBuild => self.stop_source(),
            JobAct::StopNativeBuild => self.stop_build(),
            JobAct::CancelSwitch(true) => self.cancel_switch(),
            JobAct::CancelSwitch(false) => self.cancel_provider_switch(),
            JobAct::ShowNativeBuild => self.build_all(),
            JobAct::ShowSwitch(build) => self.show_switch(build),
            JobAct::ShowLauncherUpdate => self.open_sheet(crate::sheets::Sheet::LauncherUpdate),
            JobAct::Go(page) => self.navigate(page),
        }
    }
}

/// `job` with its Stop, which says "Stopping…" once `stop` has been asked.
fn stoppable(job: Job, act: JobAct, stop: &native_mods::Cancellation) -> Job {
    let stopping = stop.is_stopped();
    let job = job.stop(act, stopping);
    if stopping {
        job.step(tr("Stopping…"))
    } else {
        job
    }
}

/// Getting the game's files ready: the step the preparer is at, with Minecraft's file count
/// while it downloads them. Its last step has no count; its time left is how long it took
/// before on this machine.
fn prepare_job(what: &str, keys: [&'static str; 2]) -> Job {
    use sunrise_launcher::backend::{prepare_step_state, PrepareStep};
    let (step, started) = prepare_step_state();
    let files = match step {
        PrepareStep::Files | PrepareStep::Java => sunrise_launcher::mojang::progress(),
        _ => None,
    };
    let stage = InstallProgress::Minecraft(step, files);
    let job = Job::new(keys[0], what).step(stage_label(stage));
    match (step, stage_fraction(stage), started) {
        (PrepareStep::Prepare, _, Some(started)) => Job {
            eta: crate::eta::expected_prepare_duration().and_then(|expected| {
                crate::eta::remaining_from_expected(started, expected, Instant::now())
            }),
            ..job
        },
        // Each step's count starts again at 0: a key of its own, so its time left does too.
        (PrepareStep::Java, Some(fraction), _) => Job {
            key: keys[1],
            ..job.measured(fraction)
        },
        (_, Some(fraction), _) => job.measured(fraction),
        _ => job,
    }
}

/// A step's name without the percentage the row already shows beside it.
fn step_name(step: InstallProgress) -> String {
    match step {
        InstallProgress::Downloading(_) => tr("Downloading").into(),
        InstallProgress::DownloadingBuild(_) => tr("Downloading the build").into(),
        InstallProgress::Rust(_) => tr("Downloading build tools").into(),
        InstallProgress::Building(_) => tr("Building").into(),
        InstallProgress::Tools { done, total } if total > 0 => trf(
            "Getting build tools · {done} of {total}",
            &[
                ("done", &sunrise_launcher::storage::format_bytes(done)),
                ("total", &sunrise_launcher::storage::format_bytes(total)),
            ],
        ),
        other => stage_label(other),
    }
}
