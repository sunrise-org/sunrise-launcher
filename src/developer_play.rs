//! Applying account or unsigned-developer sessions to GUI Play.
use super::*;

impl Launcher {
    pub(super) fn on_game_exit_message(&mut self, msg: Msg, cx: &mut Context<Self>) {
        match msg {
            Msg::Exit(job, result) => {
                // A preparation (or a build) ended: the disk may have changed under the pages.
                backend::forget_prepared();
                let error = result.err();
                match job {
                    Job::Prepare => {
                        self.preparing = false;
                        if self.starting_client.is_none() && self.installing.is_none() {
                            self.play_detail = None;
                        }
                        self.refresh_open_storage_builds();
                        // Play pressed before the version was prepared plays now.
                        if let Some((tag, intent)) = self.play_when_prepared.take() {
                            if error.is_none()
                                && self.selected() == Some(&tag)
                                && backend::check_prepared(&tag)
                            {
                                self.resume_play(intent);
                            }
                        }
                        if let (None, Some(started)) = (&error, self.prepare_started.take()) {
                            self.perf.record_since("launcher.prepare_ms", started);
                            if let (backend::PrepareStep::Prepare, Some(step_started)) =
                                backend::prepare_step_state()
                            {
                                crate::eta::record_prepare_duration(step_started.elapsed());
                            }
                        }
                        // A preparer process's exit status says little, so its last line
                        // follows; the launcher's own errors are full sentences already.
                        self.prepare_error = error.map(|e| {
                            let tail = self
                                .prepare_log
                                .iter()
                                .rev()
                                .find(|l| !l.trim().is_empty())
                                .filter(|_| e.starts_with("Exited with"));
                            tail.map_or(e.clone(), |t| format!("{e}: {t}"))
                        });
                        self.finish_switch_preparation();
                    }
                    Job::Client => {
                        self.client = None;
                        self.refresh_open_storage_builds();
                        // Any session file the game didn't read (it crashed first).
                        account::discard_unread_sessions();
                        self.come_back(cx);
                        // The game saved: last played, sizes and pictures changed.
                        self.worlds.stale();
                        self.multiplayer.stale();
                        self.screenshots.stale();
                        self.game_packs.stale();
                        let mut other_profile = None;
                        if let Some((played, started)) = self.client_run.take() {
                            self.finishing_game += 1;
                            if self.selected_instance().is_some_and(|selected| {
                                selected.id != played.id
                                    || selected.incarnation != played.incarnation
                            }) {
                                other_profile = Some(played.name.clone());
                            }
                            let tx = self.tx.clone();
                            let elapsed = started.elapsed();
                            std::thread::spawn(move || {
                                // An own profile's game kept its reports in that profile.
                                sunrise_launcher::telemetry::gather();
                                if let Err(error) = instances::finish_play(&played, elapsed, || {
                                    let _ = tx.send(Msg::PlayRecorded);
                                    sunrise_launcher::launch::run_after_exit(
                                        &played.launch,
                                        &played.home,
                                        &played.vars(),
                                    );
                                }) {
                                    eprintln!(
                                        "{}",
                                        trf("Play time not saved: {error}", &[("error", &error)])
                                    );
                                    let _ = tx.send(Msg::PlayRecorded);
                                }
                                let _ = tx.send(Msg::GameExitSettled);
                            });
                        }
                        // The run's own game folder: an instance's or a pack's keeps its reports.
                        let home = self.client_home.clone().unwrap_or_else(backend::game_home);
                        let report = self
                            .client_started
                            .take()
                            .and_then(|since| backend::crash_report_since(&home, since));
                        // Don't restart over a crash report (even if the download is still running).
                        if error.is_some() || report.is_some() {
                            self.cancel_launcher_restart();
                        }
                        let switched = if error.is_some() && report.is_none() {
                            self.switched_from_vulkan()
                        } else {
                            None
                        };
                        if let Some(api) = switched {
                            self.client_error = Some(trf(
                                "Sunrise's Vulkan renderer crashed while starting on this \
                                 graphics card, so the game now uses {api}. Press Play to \
                                 start it again. Video Settings > Graphics API switches back.",
                                &[("api", api)],
                            ));
                        } else if error.is_some() || report.is_some() {
                            let summary = report
                                .as_ref()
                                .and_then(|p| std::fs::read_to_string(p).ok())
                                .and_then(|text| backend::crash_summary(&text))
                                .or_else(|| {
                                    let tail = self
                                        .prepare_log
                                        .iter()
                                        .rev()
                                        .find(|l| !l.trim().is_empty());
                                    error
                                        .as_ref()
                                        .map(|e| tail.map_or(e.clone(), |t| format!("{e}: {t}")))
                                })
                                .unwrap_or_default();
                            let summary = match other_profile {
                                Some(profile) => trf(
                                    "The game stopped in {profile}. {reason}",
                                    &[("profile", &profile), ("reason", &summary)],
                                ),
                                None => summary,
                            };
                            self.crash = Some(Crash { summary });
                        }
                    }
                    Job::Server => {}
                }
            }
            _ => unreachable!("not a game exit message"),
        }
    }
    pub(super) fn handle_game_session(
        &mut self,
        session: std::result::Result<Option<account::GameSession>, account::NoOwner>,
    ) {
        self.load_auth_statuses(false);
        if let (Some(tag), Some(intent)) = (self.starting_client.take(), self.starting_play.take())
        {
            let mut cmd = backend::client_command(&tag);
            let pass = match session {
                Ok(pass) => pass,
                Err(refused) => {
                    // A keyring that didn't answer isn't a missing owner (#12179).
                    if refused.keyring.is_none() {
                        self.owner = Some(false);
                        self.sign_in_screen.open = true;
                    }
                    self.client_error = Some(refused.in_window());
                    self.starting_pack = None;
                    return;
                }
            };
            let session_file = pass.as_ref().and_then(|session| session.file.clone());
            if let Some(session) = pass {
                self.owner = Some(true);
                // Still play, offline, and say why.
                if let Some(e) = session.apply(&mut cmd) {
                    self.client_error = Some(trf("Playing offline: {reason}", &[("reason", &e)]));
                } else if let Some(e) = account::take_unsaved() {
                    self.client_error = Some(e);
                }
            } else {
                if let Err(error) = sunrise_launcher::developer_offline::require_capability(&tag) {
                    self.client_error = Some(error);
                    self.starting_pack = None;
                    return;
                }
                sunrise_launcher::developer_offline::mark(&mut cmd);
                self.client_error = Some(tr(sunrise_launcher::developer_offline::NOTICE).into());
            }
            let (profile, run) = match self.starting_pack.take() {
                Some(_) => (
                    Err(sunrise_launcher::browser::sunrise::pack::play_unavailable()),
                    None,
                ),
                None => {
                    let Some(instance) = instances::list()
                        .into_iter()
                        .find(|i| intent.matches_profile(i))
                    else {
                        self.client_error = Some(
                            tr("This profile no longer exists. Choose another profile to play.")
                                .into(),
                        );
                        account::discard_launch_session(session_file);
                        return;
                    };
                    (instances::apply(&mut cmd, &instance), Some(instance))
                }
            };
            if let Err(e) = profile {
                self.client_error = Some(e);
                account::discard_launch_session(session_file);
                return;
            }
            if let Err(error) = intent.destination.apply(&mut cmd) {
                self.client_error = Some(error);
            }
            backend::append_game_args(&mut cmd);
            // Last, around every argument: the instance's pre-launch command,
            // wrapper and variables.
            let launch = if let Some(instance) = &run {
                cmd.envs(instance.vars());
                instance.launch.clone()
            } else {
                Default::default()
            };
            self.starting_client = Some(tag.clone());
            jar_launch::client(
                cmd,
                tag,
                launch,
                run.map(Box::new),
                intent.account,
                session_file,
                self.tx.clone(),
            );
        } else if let Ok(Some(session)) = session {
            account::discard_launch_session(session.file);
        }
    }

    pub(super) fn on_session_message(
        &mut self,
        msg: Msg,
        window: gpui::AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        match msg {
            Msg::Session(session) => self.handle_game_session(session),
            Msg::JarConsent { jars, reply } => jar_launch::consent(window, jars, reply, cx),
            Msg::JarClient {
                run,
                account: accepted,
                session_file,
                registry_notices,
                prepared,
            } => {
                if self.starting_client.take().is_none() {
                    account::discard_launch_session(session_file);
                    return;
                }
                for notice in &registry_notices {
                    console_page::keep_line(
                        &mut self.prepare_log,
                        &mut self.log_revision,
                        notice.clone(),
                    );
                }
                self.play_registry_notices = registry_notices;
                let started = prepared.and_then(|cmd| {
                    self.accounts = Accounts::load();
                    if let Some(uuid) = accepted.as_deref() {
                        account::check_launch_account(&self.accounts, uuid)
                            .map_err(|refused| refused.in_window())?;
                    }
                    self.client_home = cmd.get_current_dir().map(|d| d.to_path_buf());
                    backend::spawn(Job::Client, cmd, self.tx.clone())
                });
                match started {
                    Ok(r) => {
                        self.client = Some(r);
                        self.client_run = run.map(|instance| (*instance, Instant::now()));
                        // Do not hide or minimize the window when the registry has
                        // something the player needs to see about this run.
                        if self.play_registry_notices.is_empty() {
                            self.step_aside(cx);
                        }
                    }
                    // A start that does not happen leaves no session file behind.
                    Err(e) => {
                        self.client_error = Some(e);
                        account::discard_launch_session(session_file);
                    }
                }
            }
            // Old versions log their session; it never reaches the window.
            Msg::Line(_, line) if line.contains("Session ID is") => {}
            Msg::Line(job, line) => {
                if job == Job::Client && line.starts_with(perf::GAME_FIRST_FRAME) {
                    if let Some(pressed) = self.play_pressed.take() {
                        self.perf
                            .record_since("launcher.play_to_game_frame_ms", pressed);
                    }
                }
                if job == Job::Prepare {
                    self.follow_switch_preparation();
                }
                // The MCP server keeps its own server's log; the launcher's are the preparer's
                // and the game's.
                if job != Job::Server {
                    console_page::keep_line(&mut self.prepare_log, &mut self.log_revision, line);
                }
            }
            Msg::ClientObservationLost(error) => {
                self.cancel_launcher_restart();
                self.client = None;
                self.client_run = None;
                self.client_started = None;
                self.come_back(cx);
                self.client_error = Some(error);
            }
            _ => unreachable!("not a session message"),
        }
    }
}
