//! The backend's messages, drained as they arrive (`Launcher::drain`, `wake.rs`).
use super::*;

impl Launcher {
    pub(crate) fn drain(&mut self, window: gpui::AnyWindowHandle, cx: &mut Context<Self>) {
        let mut changed = false;
        self.perf.tick();
        if self.perf.resting(Duration::from_millis(250)) {
            let _ = cx.update_window(window, |_, window, _| {
                language_page::warm_language_name(window)
            });
        }
        #[cfg(feature = "frame-stats")]
        if self.bench.is_some() {
            self.perf.draws = cx.update_window(window, |_, w, _| perf::draws(w)).ok();
        }
        match self.bench.as_mut().map(|b| b.tick(&self.perf)) {
            // Through the page the frame draws (`navigate`, as a click on a tab does): `go` sets
            // the older `page` field, which nothing on screen reads, so the benchmark measured a
            // window that never changed.
            Some(perf::Step::Go(page)) => {
                let pages = crate::frame::Page::BENCH;
                self.navigate(pages[page % pages.len()]);
                changed = true;
            }
            Some(perf::Step::Scroll) => {
                self.navigate(crate::frame::Page::Versions);
                changed = true;
            }
            Some(perf::Step::Quit) => cx.quit(),
            Some(perf::Step::Wait) | None => {}
        }
        while let Ok(msg) = self.rx.try_recv() {
            changed = true;
            match msg {
                Msg::Releases(r) => {
                    if let (Ok(_), Some(checked)) = (&r, self.checked) {
                        self.perf.record_since("launcher.release_list_ms", checked);
                    }
                    if let Ok(list) = &r {
                        sunrise_launcher::nightly::note_published(list);
                    }
                    self.releases = r.map_or_else(Load::Failed, Load::Ready);
                    // Once the list is in (or failed), each instance resolves to the build it plays.
                    if !self.mods.auto_updated {
                        self.mods.auto_updated = true;
                        self.mods_auto_update();
                    }
                }
                Msg::Links(links) => self.links = links,
                // The page reads is_prepared when drawn; `changed` redraws it.
                Msg::Reused(_) => self.installed = backend::installed(),
                Msg::Keychain(said) => self.keychain = said,
                Msg::Owner(owns) => self.owner = Some(owns),
                Msg::AccountHead(uuid, url, face, failed) => {
                    self.profile_heads.loaded(&uuid, &url, face, failed)
                }
                Msg::AccountProgress(uuid, generation, phase) => {
                    if self.auth_busy.get(&uuid) == Some(&generation) {
                        self.auth_progress.insert(uuid, phase);
                    }
                }
                Msg::AccountStatus(uuid, generation, status) => {
                    if self.auth_busy.get(&uuid) != Some(&generation) {
                        continue;
                    }
                    self.auth_busy.remove(&uuid);
                    self.auth_progress.remove(&uuid);
                    if self.accounts.find(&uuid).is_some() {
                        if status.state == account::session::State::Ready {
                            self.owner = Some(true);
                        }
                        self.auth_status.insert(uuid, status);
                        // A refresh may have saved a new name or skin: show it now.
                        let saved = Accounts::load();
                        if saved != self.accounts {
                            self.accounts = saved;
                            self.load_profile_heads();
                        }
                    }
                }
                Msg::AccountRemoved(uuid, generation, result) => {
                    if self.auth_busy.get(&uuid) != Some(&generation) {
                        continue;
                    }
                    self.auth_busy.remove(&uuid);
                    self.auth_progress.remove(&uuid);
                    match result {
                        Ok(owns) => {
                            self.accounts = Accounts::load();
                            self.auth_status.remove(&uuid);
                            self.owner = Some(owns);
                        }
                        Err(e) => self.sign_in_state = Some(SignInState::Done(Err(e))),
                    }
                }
                Msg::Playing(playing) => self.playing = playing,
                Msg::PlayStep(step) => {
                    if self.starting_client.is_some() {
                        self.play_step = Some(step);
                    }
                }
                Msg::PlayAdmission(home, result) => {
                    let Some(intent) = self.play_admission.take() else {
                        continue;
                    };
                    if self
                        .selected_instance()
                        .is_none_or(|i| i.home != home || i.id != intent.instance)
                    {
                        continue;
                    }
                    self.play_install_recovery = None;
                    match result {
                        Ok(()) => self.play_after_journal_check(intent, &home),
                        Err(problem) => {
                            self.play_install_recovery = Some((home, problem.details()))
                        }
                    }
                }
                Msg::PinnedBuild {
                    home,
                    result,
                    close_versions,
                } => self.finish_build_pin(&home, result, close_versions),
                Msg::Session(session) => self.handle_game_session(session),
                Msg::JarConsent { jars, reply } => jar_launch::consent(window, jars, reply, cx),
                Msg::JarClient {
                    run,
                    account: accepted,
                    session_file,
                    registry_notices,
                    prepared,
                } => {
                    let Some(tag) = self.starting_client.take() else {
                        account::discard_launch_session(session_file);
                        continue;
                    };
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
                        native_status::check_permission(&tag)?;
                        self.client_home = cmd.get_current_dir().map(|d| d.to_path_buf());
                        backend::spawn(Job::Client, cmd, self.tx.clone())
                    });
                    match started {
                        Ok(r) => {
                            self.client = Some(r);
                            self.client_run = run.map(|id| (id, Instant::now()));
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
                Msg::Progress(tag, p) => {
                    self.bar = (self.bar.1, p);
                    self.installing = Some((tag, p));
                }
                Msg::Installed(tag, result) => {
                    self.installing = None;
                    self.bar = (0., 0.);
                    self.installed = backend::installed();
                    let as_update = self.update_install.take().filter(|t| *t == tag);
                    let for_instance = self.instance_install.take_if(|t| *t == tag).is_some();
                    if let (Ok(()), Some(started)) = (&result, self.install_started.take()) {
                        let name = if as_update.is_some() {
                            "launcher.update_ms"
                        } else {
                            "launcher.install_ms"
                        };
                        self.perf.record_since(name, started);
                    }
                    let resume = self
                        .ui
                        .after_install
                        .take()
                        .filter(|(_, wanted)| result.is_ok() && *wanted == tag);
                    match result {
                        Ok(()) => {
                            if as_update.is_some()
                                && sunrise_launcher::nightly::date_of(&tag).is_some()
                            {
                                if let Err(e) = backend::mark_auto_installed(&tag) {
                                    eprintln!("Could not mark updated Nightly {tag}: {e}");
                                }
                            }
                            if self.updates.sunrise.as_ref().is_some_and(|r| r.tag == tag) {
                                self.updates.sunrise = None;
                            }
                            // An instance's own version leaves the other instances on theirs.
                            // Judged now, not when the download started: a version picked on
                            // Versions meanwhile stays picked.
                            let before: Vec<String> = self
                                .installed
                                .iter()
                                .filter(|t| **t != tag)
                                .cloned()
                                .collect();
                            let select = as_update.is_none_or(|_| {
                                backend::select_after_update(
                                    self.settings.selected.as_deref(),
                                    &before,
                                )
                            }) && (!for_instance || self.launcher_version().is_none());
                            if select {
                                self.settings.selected = Some(tag.clone());
                            }
                            // Pruning reads settings from disk: protect the selection only once
                            // it has been saved there. A failed save leaves all versions in place.
                            if self.save_settings() {
                                let tx = self.tx.clone();
                                std::thread::spawn(move || {
                                    let _ = tx.send(Msg::Pruned(backend::prune_old_versions()));
                                });
                            }
                        }
                        Err(e) => self.install_error = Some((tag, e)),
                    }
                    if let Some((intent, _)) =
                        resume.filter(|(_, wanted)| self.selected() == Some(wanted))
                    {
                        self.resume_play(intent);
                    }
                    if let Some(next) = self.install_next.take() {
                        self.install_for_instance(next);
                    }
                }
                Msg::Removed(tag, result) => {
                    self.removing.retain(|t| *t != tag);
                    self.installed = backend::installed();
                    let selected = if self.settings_save_error.is_some() {
                        self.settings.selected.take()
                    } else {
                        Settings::load().selected
                    };
                    self.settings.selected = selection_after_removal(&self.installed, selected);
                    if let Err(e) = result {
                        self.install_error = Some((tag, e));
                    }
                }
                Msg::DiskUse(disk) => self.disk = disk,
                Msg::Storage(result) => self.storage_arrived(result),
                Msg::Imported(result) => {
                    self.importing_build = false;
                    self.installed = backend::installed();
                    match result {
                        // `build_import::import` saved the selection; this keeps the window's copy.
                        Ok(tag) => self.settings.selected = Some(tag),
                        Err(e) => self.import_error = Some(e),
                    }
                }
                Msg::Pruned(gone) => {
                    if !gone.is_empty() {
                        self.installed = backend::installed();
                    }
                }
                Msg::Updates(updates) => {
                    self.checking_updates = false;
                    self.launcher_checked_now = None;
                    if let Some(error) = &updates.launcher_error {
                        settings::log_launcher_check_failure(error, |line| eprintln!("{line}"));
                    }
                    self.updates = *updates;
                    if self.settings.auto_install() {
                        self.install_update();
                    }
                    if let Some(release) = self.updates.launcher.clone() {
                        self.update_launcher(release);
                    }
                }
                Msg::LauncherChecked(result) => {
                    self.checking_updates = false;
                    self.launcher_checked_now = result.is_ok().then(Instant::now);
                    match *result {
                        Ok(release) => {
                            self.updates.launcher = release;
                            self.updates.launcher_error = None;
                        }
                        Err(error) => {
                            settings::log_launcher_check_failure(&error, |line| {
                                eprintln!("{line}")
                            });
                            self.updates.launcher_error = Some(error)
                        }
                    }
                }
                Msg::NativeConsentRead(read, inputs) => {
                    if let Some(pending) = &mut self.pending_consent {
                        pending.finish_read(&read, inputs);
                    }
                }
                Msg::NativeRegistryConsent(inventory, reply) => {
                    self.registry_consent(inventory, reply);
                }
                Msg::NativeProgress(step) => {
                    self.native_step = Some(step.shown());
                    self.native_progress = Some(step.progress());
                    if let Some(run) = &mut self.ui.mods.build {
                        run.step(&step);
                    }
                }
                Msg::Native(result) => {
                    self.native_cancellation = None;
                    self.native_step = None;
                    self.native_progress = None;
                    self.first_build = None;
                    // Build and play: the game starts once the build worked, and only for the
                    // instance it was pressed for, still on the build just made (another may
                    // have been picked, or its version changed, meanwhile).
                    let wanted = self.play_after_build.take();
                    let cancelled = matches!(&result, Err(native_mods::Failed::Other(error)) if error == native_mods::CANCELLED);
                    self.installed = backend::installed();
                    self.reload_instances();
                    // No permission: the question is the answer, not the refusal's words.
                    let no_consent = matches!(result, Err(native_mods::Failed::NoConsent));
                    if no_consent {
                        self.request_native_build_consent();
                    }
                    if let (Err(e), false) = (&result, cancelled || no_consent) {
                        if let native_mods::Failed::Worlds(stake) = e {
                            if let Some(run) = &mut self.ui.mods.build {
                                run.at_stake(&stake.mods, &stake.unbuilt);
                            }
                            self.keep_stop(stake);
                        }
                        // Read against the mods this build was given, not the instance now
                        // selected.
                        let built = std::mem::take(&mut self.native_built_mods);
                        self.native_failed(e.to_string(), &built);
                    }
                    // The build window's build ends as the build did. With its window closed,
                    // the strip says how it went for a moment.
                    let shown = self.ui.sheet == Some(crate::sheets::Sheet::ModBuild);
                    if no_consent {
                        self.ui.mods.build = None;
                    } else if let Some(run) = &mut self.ui.mods.build {
                        match (result.is_ok(), cancelled, &self.native_error) {
                            (true, ..) => run.done(),
                            (false, true, _) => run.cancelled(),
                            (false, false, error) => run.stopped(
                                error.clone().unwrap_or_default(),
                                &self.native_failed_mods,
                            ),
                        }
                        if let Err(native_mods::Failed::Mods { left, .. }) = &result {
                            for reason in left {
                                run.mark_left_out(reason);
                            }
                        }
                        if !shown {
                            run.said_at = Some(Instant::now());
                            std::thread::spawn(|| {
                                std::thread::sleep(crate::mods::build_run::SAID_FOR);
                                crate::wake::wake();
                            });
                        }
                    }
                    // The build the instance plays now: its own, or its channel's when every mod
                    // was left out and it follows one.
                    let plays = self
                        .selected_instance()
                        .map(|i| (i.id.as_str(), self.version_of(i).map(String::as_str)));
                    if let Some(play) =
                        crate::mods::build_run::play_on_finish(wanted, &result, plays)
                    {
                        // The game is what the player asked for: its window gets out of the way.
                        if shown {
                            self.close_sheet();
                        }
                        self.play(play);
                    }
                }
                Msg::LauncherProgress(p) => {
                    if let SelfUpdate::Downloading(_, f) = &mut self.self_update {
                        *f = p;
                    }
                }
                Msg::LauncherUpdated(result) => {
                    let version = match &self.self_update {
                        SelfUpdate::Downloading(v, _) => v.clone(),
                        _ => String::new(),
                    };
                    self.self_update = match result {
                        Ok(staged) => {
                            sunrise_launcher::update_handoff::set_pending(staged);
                            SelfUpdate::Ready(version)
                        }
                        Err(e) => SelfUpdate::Failed(e),
                    };
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
                    // The MCP server keeps its own server's log; the launcher's are the preparer's
                    // and the game's.
                    if job != Job::Server {
                        console_page::keep_line(
                            &mut self.prepare_log,
                            &mut self.log_revision,
                            line,
                        );
                    }
                }
                Msg::Exit(job, result) => {
                    // A preparation (or a build) ended: the disk may have changed under the pages.
                    backend::forget_prepared();
                    let error = result.err();
                    match job {
                        Job::Prepare => {
                            self.preparing = false;
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
                        }
                        Job::Client => {
                            self.client = None;
                            // Any session file the game didn't read (it crashed first).
                            account::discard_unread_sessions();
                            self.come_back(cx);
                            // The game saved: last played, sizes and pictures changed.
                            self.worlds.stale();
                            self.multiplayer.stale();
                            self.screenshots.stale();
                            self.game_packs.stale();
                            let vars = std::mem::take(&mut self.client_vars);
                            let mut other_profile = None;
                            if let Some((id, started)) = self.client_run.take() {
                                if let Err(e) = instances::record_play(&id, started.elapsed()) {
                                    eprintln!("Play time not saved: {e}");
                                }
                                if let Some(i) = instances::list().into_iter().find(|i| i.id == id)
                                {
                                    if self
                                        .selected_instance()
                                        .is_some_and(|selected| selected.id != id)
                                    {
                                        other_profile = Some(i.name);
                                    }
                                    sunrise_launcher::launch::run_after_exit(
                                        &i.launch, &i.home, &vars,
                                    );
                                }
                                self.reload_instances();
                            }
                            // The run's own game folder: an instance's or a pack's keeps its reports.
                            let home = self.client_home.clone().unwrap_or_else(backend::game_home);
                            let report = self
                                .client_started
                                .take()
                                .and_then(|since| backend::crash_report_since(&home, since));
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
                                        error.as_ref().map(|e| {
                                            tail.map_or(e.clone(), |t| format!("{e}: {t}"))
                                        })
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
            }
        }
        changed |= self.servers_tick();
        changed |= self.mods_tick();
        changed |= self.mods_drain();
        changed |= self.github_tick();
        changed |= self.toast_tick();
        changed |= self.artwork_tick();
        if let Some(sign_in) = &self.sign_in {
            if let Some(state) = sign_in_change(&mut self.sign_in_state, sign_in.state()) {
                if let SignInState::Done(Ok(ref account)) = state {
                    self.auth_busy.remove(&account.uuid);
                    self.sign_in_target = None;
                    self.accounts = Accounts::load();
                    self.load_profile_heads();
                    self.load_auth_statuses(false);
                    // Sign-in refuses accounts that don't own the game.
                    self.owner = Some(true);
                    self.owner_screen_open = false;
                    self.client_error = None;
                    if !self.settings.owner_notice_seen {
                        self.settings.owner_notice_seen = true;
                        self.save_settings();
                    }
                    if let Some(instance) = self.ui.after_sign_in.take().filter(|asked| {
                        account_chip::plays_after_sign_in(
                            Some(asked.clone()),
                            self.selected_instance().map(|i| i.id.as_str()),
                        )
                    }) {
                        self.resume_play(launcher_play::PlayIntent::new(instance, Some(account)));
                    }
                }
                changed = true;
            }
        }
        if !self.checking_updates
            && self
                .checked
                .is_some_and(|at| at.elapsed() > backend::CHECK_EVERY)
        {
            self.check_updates(UpdateCheck::Scheduled);
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }
}
