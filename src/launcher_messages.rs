//! The backend's messages, drained as they arrive (`Launcher::drain`, `wake.rs`).
use super::*;
use sunrise_launcher::managed_play::StartError;

/// A canceled Play is not a build failure. Otherwise keep the window open for the native
/// build diagnostic instead of restarting as soon as Play stops.
fn settle_managed_play_error(
    error: &StartError,
    active_play_build: bool,
    cancel_restart: impl FnOnce(),
) -> bool {
    let cancelled = matches!(error,
        StartError::Native(native_mods::Failed::Other(detail), _) | StartError::Other(detail)
            if detail == native_mods::CANCELLED);
    if !cancelled && (active_play_build || matches!(error, StartError::Native(..))) {
        cancel_restart();
    }
    cancelled
}

/// Once Play reaches verification, the native build and profile publication are complete.
fn finish_play_build_at_handoff(
    run: Option<&mut crate::mods::build_run::Run>,
    step: backend::PlayStep,
) -> bool {
    if matches!(step, backend::PlayStep::Assets | backend::PlayStep::SignIn) {
        if let Some(run) = run.filter(|run| run.for_play && run.running()) {
            run.done();
            return true;
        }
    }
    false
}

/// Route a Play failure that arrived while the Play build sheet was still open.
/// Returns `(captured_build_error, close_build_sheet)`: a cached-snapshot failure
/// belongs to Start, not Build, so the build run finishes and closes its sheet.
fn route_active_play_build_error(
    run: &mut crate::mods::build_run::Run,
    error: &StartError,
    cancelled: bool,
) -> (bool, bool) {
    match error {
        _ if cancelled => {
            run.cancelled();
            (false, false)
        }
        StartError::Native(error, build) => {
            run.failed_build = build.clone();
            run.native_failed(error, &[]);
            (false, false)
        }
        StartError::Snapshot(_) => {
            run.done();
            (false, true)
        }
        StartError::Other(error) => {
            run.stopped(
                tr("The build could not finish. No game was started.").into(),
                error.clone(),
            );
            (true, false)
        }
        StartError::Journal(error) => {
            run.stopped(error.summary().into(), error.details());
            (false, false)
        }
        _ => {
            run.stopped(
                tr("The build couldn't be saved. No game was started.").into(),
                String::new(),
            );
            (false, false)
        }
    }
}

impl Launcher {
    pub(super) fn refresh_installed(&mut self) {
        (self.installed, self.known_nightly) =
            backend::play_installations(self.settings.selected.as_deref());
        self.refresh_open_storage_builds();
    }

    pub(crate) fn drain(&mut self, window: gpui::AnyWindowHandle, cx: &mut Context<Self>) {
        let mut changed = self.pull_logs();
        changed |= self.pull_switch_log();
        self.perf.tick();
        if self.bench.as_ref().is_some_and(perf::Bench::ready_for_idle) {
            // Finish one-time fallback font loading before the benchmark starts its idle sample.
            let _ = cx.update_window(window, |_, window, _| {
                language_page::warm_all_language_names(window)
            });
        } else if self.bench.is_none() && self.perf.resting(Duration::from_millis(250)) {
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
                // The build picker's list, the longest the launcher has, or Installed, scrolled
                // as a wheel does it rather than from inside a frame.
                match self.bench.as_ref().map(perf::Bench::list) {
                    Some(perf::List::Installed) => self.show_installed_mods(),
                    _ => self.open_versions(),
                }
                let launcher = cx.weak_entity();
                let _ = cx.update_window(window, |_, w, _| crate::sheets::bench_wheel(launcher, w));
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
                    let loaded = r.is_ok();
                    self.releases = r.map_or_else(Load::Failed, Load::Ready);
                    if loaded {
                        self.refresh_installed();
                    } else {
                        self.refresh_open_storage_builds();
                    }
                }
                Msg::Links(links) => self.links = links,
                // The page reads is_prepared when drawn; `changed` redraws it.
                Msg::Reused(_) => self.refresh_installed(),
                Msg::Keychain(said) => self.keychain = said,
                Msg::Owner(owns) => self.owner = Some(owns),
                Msg::AccountHead(uuid, url, face, failed) => {
                    self.profile_heads.loaded(&uuid, &url, face, failed)
                }
                Msg::SkinsProfile(uuid, id, result) => self.skins_profile(uuid, id, result),
                Msg::Picture(url, png) => self.skins_picture(url, png),
                Msg::LookSaved(uuid, id, outcome) => self.look_saved(uuid, id, *outcome),
                Msg::SkinsAdded(uuid, added, listing) => self.skins_added(uuid, added, listing),
                Msg::SkinsListed(uuid, listing) => self.skins_listed(uuid, listing),
                Msg::LookRecorded(uuid, record, recorded) => {
                    self.look_recorded(uuid, record, recorded)
                }
                Msg::SkinRemoved(uuid, id, removed, listing) => {
                    self.skin_removed(uuid, id, removed, listing)
                }
                Msg::SkinFile(key, id, png) => self.skins_file(key, id, png),
                Msg::SkinKept(uuid, url, kept, listing) => {
                    self.skins_kept(uuid, url, kept, listing)
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
                        if self.play_step != Some(step) {
                            self.play_detail = None;
                        }
                        self.play_step = Some(step);
                    }
                    if finish_play_build_at_handoff(self.ui.mods.build.as_mut(), step) {
                        self.native_step = None;
                        self.native_progress = None;
                        if self.ui.sheet == Some(crate::sheets::Sheet::ModBuild) {
                            self.close_sheet();
                        }
                        self.refresh_installed();
                    }
                }
                Msg::PlayDetail(detail) => {
                    // A switch's steps come as SwitchDetail, from its own operation.
                    if self.starting_client.is_some() || self.preparing || self.installing.is_some()
                    {
                        self.play_detail = Some(detail);
                    }
                }
                Msg::ManagedBuild {
                    instance,
                    profile,
                    base,
                    mods,
                } => {
                    let selected = self.selected_instance().is_some_and(|i| i.id == instance);
                    let lines = self.build_lines(mods, selected);
                    self.native_for = Some(instance.clone());
                    self.native_failed_mods.clear();
                    self.ui.mods.build = Some(crate::mods::build_run::Run::start(
                        instance,
                        profile,
                        crate::sheets::build_name(&base),
                        lines,
                        true,
                    ));
                    self.open_sheet(crate::sheets::Sheet::ModBuild);
                }
                Msg::ManagedBuildLog(output) => {
                    if let Some(run) = self.ui.mods.build.as_mut().filter(|run| run.for_play) {
                        run.log = Some(output);
                    }
                }
                Msg::ManagedClient { instance, result } => {
                    self.native_cancellation = None;
                    self.native_step = None;
                    self.native_progress = None;
                    let active_play_build = self
                        .ui
                        .mods
                        .build
                        .as_ref()
                        .is_some_and(|run| run.for_play && run.running());
                    let cancelled = result.as_ref().as_ref().is_err_and(|error| {
                        settle_managed_play_error(error, active_play_build, || {
                            self.cancel_launcher_restart()
                        })
                    });
                    let mut captured_build_error = false;
                    let mut close_build_sheet = false;
                    if let (Some(run), Err(error)) = (&mut self.ui.mods.build, result.as_ref()) {
                        if active_play_build {
                            (captured_build_error, close_build_sheet) =
                                route_active_play_build_error(run, error, cancelled);
                        }
                    }
                    if close_build_sheet && self.ui.sheet == Some(crate::sheets::Sheet::ModBuild) {
                        self.close_sheet();
                    }
                    if !self.ui.mods.build.as_ref().is_some_and(|run| {
                        run.for_play
                            && matches!(run.end, Some(crate::mods::build_run::End::Stopped(_)))
                    }) {
                        self.play_retry = None;
                    }
                    self.starting_client = None;
                    self.play_step = None;
                    self.play_detail = None;
                    self.accounts = Accounts::load();
                    self.load_auth_statuses(false);
                    match *result {
                        Ok(game) => {
                            self.client_error = game.notice;
                            self.play_registry_notices = game.registry_notices;
                            for notice in &self.play_registry_notices {
                                console_page::keep_line(
                                    &mut self.prepare_log,
                                    &mut self.log_revision,
                                    notice.clone(),
                                );
                            }
                            if game.owner {
                                self.owner = Some(true);
                            }
                            self.refresh_installed();
                            sunrise_launcher::launcher_log::line(format!(
                                "Starting the game: {}",
                                self.profile_target(&game.instance)
                                    .unwrap_or_else(|| "an unknown version".into())
                            ));
                            self.client_home = Some(game.instance.home.clone());
                            self.client_run = Some((game.instance, Instant::now()));
                            self.client = Some(backend::monitor_game(
                                game.child,
                                self.tx.clone(),
                                game.runtime,
                            ));
                            if self.play_registry_notices.is_empty() {
                                self.step_aside(cx);
                            }
                        }
                        Err(StartError::Journal(problem)) => {
                            self.play_install_recovery = Some((instance.home, problem.details()));
                        }
                        Err(StartError::Account(refused)) => {
                            if refused.keyring.is_none() {
                                self.owner = Some(false);
                                self.sign_in_screen.open = true;
                            }
                            self.client_error = Some(refused.in_window());
                        }
                        Err(_) if cancelled => (),
                        Err(StartError::Native(error, _)) => {
                            self.native_for = Some(instance.id);
                            if !active_play_build {
                                self.native_failed(error.to_string(), &[]);
                            }
                        }
                        Err(StartError::Hook(error)) => self.client_error = Some(error.to_string()),
                        Err(StartError::Other(_)) if captured_build_error => (),
                        Err(
                            StartError::UnknownSpawn(error)
                            | StartError::Snapshot(error)
                            | StartError::Other(error),
                        ) => {
                            self.client_error = Some(error);
                        }
                    }
                }
                Msg::PlayRecorded => self.reload_instances(),
                Msg::GameExitSettled => self.finishing_game -= 1,
                Msg::PlayAdmission(home, result) => {
                    let Some(intent) = self.play_admission.take() else {
                        continue;
                    };
                    if self
                        .selected_instance()
                        .is_none_or(|i| i.home != home || !intent.matches_profile(i))
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
                Msg::SwitchProgress(stage) => self.switch_progress(stage),
                Msg::SwitchDetail(text) => self.switch_detail(text),
                Msg::SwitchQuestion(question, reply) => self.switch_question(question, reply),
                Msg::Switched(home, result) => self.switch_finished(&home, *result),
                Msg::ProviderSwitchProgress(stage) => self.provider_switch_progress(stage),
                Msg::ProviderSwitchQuestion(question, reply) => {
                    self.provider_switch_question(question, reply)
                }
                Msg::ProviderSwitched(home, result) => self.provider_switch_finished(&home, result),
                Msg::ProviderChoice(need, reply) => self.provider_choice_question(need, reply),
                Msg::PublisherChangeQuestion(question, reply) => {
                    self.publisher_change_question(question, reply)
                }
                Msg::ReceiptAcknowledged(home, result) => self.receipt_acknowledged(&home, result),
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
                        continue;
                    }
                    self.play_step = None;
                    self.play_detail = None;
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
                            sunrise_launcher::launcher_log::line(format!(
                                "Starting the game: {}",
                                run.as_deref()
                                    .and_then(|instance| self.profile_target(instance))
                                    .unwrap_or_else(|| "an unknown version".into())
                            ));
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
                Msg::Progress(tag, phase) => {
                    let Some((_, current)) = self
                        .installing
                        .as_mut()
                        .filter(|(active, _)| *active == tag)
                    else {
                        continue;
                    };
                    let key = crate::eta::install_stage_key;
                    if key((*current).into()) != key(phase.into()) {
                        self.play_detail = None;
                    }
                    *current = phase;
                    self.switch_download_progress(&tag, phase);
                }
                Msg::Installed(tag, result) => {
                    if self
                        .installing
                        .take_if(|(active, _)| *active == tag)
                        .is_none()
                    {
                        continue;
                    }
                    if self.starting_client.is_none() && !self.preparing {
                        self.play_detail = None;
                    }
                    self.refresh_installed();
                    if let Some(warning) = self.record_nightly_receipt_outcome(&tag, &result) {
                        glass_toast::say(trf(
                            "{warning} Try again in Settings → Storage.",
                            &[("warning", &warning)],
                        ));
                    }
                    if self.finish_switch_download(&tag, &result) {
                        self.install_started = None;
                        if let Some(next) = self.install_next.take() {
                            self.install_for_instance(next);
                        }
                        continue;
                    }
                    let receipt_only =
                        matches!(&result, Ok(backend::InstallOutcome::ReceiptOnly(_)));
                    let result = result.map(|_| ());
                    let repair = self.architecture_reinstall.take().is_some_and(|t| t == tag);
                    let for_instance = self.instance_install.take_if(|t| *t == tag).is_some();
                    if let (Ok(()), Some(started)) = (&result, self.install_started.take()) {
                        self.perf.record_since("launcher.install_ms", started);
                    }
                    let resume = self
                        .ui
                        .after_install
                        .take()
                        .filter(|(_, wanted)| result.is_ok() && *wanted == tag);
                    match result {
                        Ok(()) if repair || receipt_only => {}
                        Ok(()) => {
                            if self.updates.sunrise.as_ref().is_some_and(|r| r.tag == tag) {
                                self.updates.sunrise = None;
                            }
                            // An instance's own download leaves other selections alone.
                            let select = !for_instance || self.launcher_version().is_none();
                            if select {
                                self.settings.selected = Some(tag.clone());
                            }
                            // Pruning reads settings from disk: protect the selection only once
                            // it has been saved there. A failed save leaves all versions in place.
                            if self.save_settings() && self.settings.auto_prune() {
                                let tx = self.tx.clone();
                                let releases = self.game_releases().to_vec();
                                let installed = tag.clone();
                                std::thread::spawn(move || {
                                    let errors = backend::version_storage::prune_old_versions(
                                        &releases, &installed,
                                    );
                                    let _ = tx.send(Msg::Pruned(errors));
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
                Msg::NightlyReceiptRetried(warning, result) => {
                    if result.is_ok() {
                        self.refresh_installed();
                    }
                    self.finish_nightly_receipt_retry(&warning, result);
                }
                Msg::Removed(errors) => self.storage_builds_removed(errors),
                Msg::BuildInventory(generation, rows) => {
                    self.storage_builds_arrived(generation, rows)
                }
                Msg::Storage(token, result) => self.storage_arrived(token, result),
                Msg::Imported(result) => self.storage_build_imported(result),
                Msg::Pruned(errors) => self.storage_builds_pruned(errors),
                Msg::Updates(updates) => {
                    self.checking_updates = false;
                    self.launcher_checked_now = None;
                    let updates = *updates;
                    self.updates.sunrise = updates.sunrise;
                    self.updates.sunrise_error = updates.sunrise_error;
                    self.note_launcher_check(
                        updates.launcher_error.map_or(Ok(updates.launcher), Err),
                    );
                }
                Msg::LauncherChecked(result) => {
                    self.checking_updates = false;
                    self.launcher_checked_now = result.is_ok().then(Instant::now);
                    self.note_launcher_check(*result);
                }
                Msg::ConfirmUnreviewed(found, reply) => {
                    let pending = crate::github_mods_page::PendingUnreviewed::new(found, reply);
                    if let Some(earlier) = self.ui.sheets.unreviewed.replace(pending) {
                        earlier.refuse();
                    }
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
                    // Keep the window and its diagnostic when a build needs the player's attention.
                    if result.is_err() && !cancelled {
                        self.cancel_launcher_restart();
                    }
                    self.refresh_installed();
                    self.reload_instances();
                    if let (Err(e), false) = (&result, cancelled) {
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
                    if let Some(run) = &mut self.ui.mods.build {
                        match &result {
                            Ok(_) => run.done(),
                            Err(_) if cancelled => run.cancelled(),
                            Err(error) => run.native_failed(error, &self.native_failed_mods),
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
                Msg::LauncherUpdated(result) => self.launcher_update_finished(result),
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
                        console_page::keep_line(
                            &mut self.prepare_log,
                            &mut self.log_revision,
                            line,
                        );
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
                Msg::Exit(job, result) => {
                    // A preparation (or a build) ended: the disk may have changed under the pages.
                    backend::forget_prepared();
                    let error = result.err();
                    match job {
                        Job::Prepare => {
                            self.preparing = false;
                            self.play_detail = None;
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
                                    if let Err(error) =
                                        instances::finish_play(&played, elapsed, || {
                                            let _ = tx.send(Msg::PlayRecorded);
                                            sunrise_launcher::launch::run_after_exit(
                                                &played.launch,
                                                &played.home,
                                                &played.vars(),
                                            );
                                        })
                                    {
                                        sunrise_launcher::elog!(
                                            "{}",
                                            trf(
                                                "Play time not saved: {error}",
                                                &[("error", &error)]
                                            )
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
                                        error.as_ref().map(|e| {
                                            tail.map_or(e.clone(), |t| format!("{e}: {t}"))
                                        })
                                    })
                                    .unwrap_or_default();
                                sunrise_launcher::launcher_log::line(format!(
                                    "The game crashed: {summary}"
                                ));
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
                    self.sign_in_screen.target = None;
                    self.accounts = Accounts::load();
                    self.load_profile_heads();
                    self.load_auth_statuses(false);
                    // Sign-in refuses accounts that don't own the game.
                    self.owner = Some(true);
                    self.sign_in_screen.open = false;
                    self.client_error = None;
                    if !self.settings.owner_notice_seen {
                        self.settings.owner_notice_seen = true;
                        self.save_settings();
                    }
                    if let Some(mut intent) = self.ui.after_sign_in.take().filter(|asked| {
                        account_chip::plays_after_sign_in(
                            Some(asked.instance.clone()),
                            self.selected_instance().map(|i| i.id.as_str()),
                        )
                    }) {
                        intent.account = Some(account.uuid.clone());
                        self.resume_play(intent);
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
        self.restart_launcher_if_ready(cx);
        // A mod rewritten where it lies, seen while the page sits still.
        changed |= self.mods_check_due(std::time::Instant::now());
        // Repaint once a second while a loading sheet or status-strip stage is active so ETAs
        // tick down and stall/expiry hides take effect even if the worker goes quiet.
        if !changed && (self.switch_busy() || self.registry_stage().is_some()) {
            static LAST_ETA_TICK: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
            let now = Instant::now();
            if let Ok(mut last) = LAST_ETA_TICK.lock() {
                if last.is_none_or(|t| now.saturating_duration_since(t) >= Duration::from_secs(1)) {
                    *last = Some(now);
                    changed = true;
                }
            }
        }
        if changed {
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_handoff_leaves_start_errors_out_of_the_build_sheet() {
        use backend::PlayStep;
        let mut run = crate::mods::build_run::Run::start(
            "fixture".into(),
            "fixture profile".into(),
            "fixture".into(),
            Vec::new(),
            true,
        );
        assert!(!finish_play_build_at_handoff(
            Some(&mut run),
            PlayStep::Native
        ));
        assert!(run.running());
        assert!(finish_play_build_at_handoff(
            Some(&mut run),
            PlayStep::Assets
        ));
        assert!(
            !run.running(),
            "a subsequent Other error belongs to Start, not Build"
        );
        assert!(matches!(run.end, Some(crate::mods::build_run::End::Done)));

        // A cached snapshot failure arriving before the Verify step is processed must not be
        // captured as a Build failure; it finishes the build sheet and surfaces on Start.
        let mut pre_verify = crate::mods::build_run::Run::start(
            "fixture".into(),
            "fixture profile".into(),
            "fixture".into(),
            Vec::new(),
            true,
        );
        let detail =
            "Sunrise couldn't verify the profile's saved game build (managed_play/resources.rs:1)."
                .to_owned();
        let (captured, close_sheet) = route_active_play_build_error(
            &mut pre_verify,
            &StartError::Snapshot(detail.clone()),
            false,
        );
        assert!(!captured);
        assert!(close_sheet);
        assert!(!pre_verify.running());
        assert!(matches!(
            pre_verify.end,
            Some(crate::mods::build_run::End::Done)
        ));

        use crate::launch_state::{Act, Facts, Main};
        let view = Facts {
            client_error: Some(detail.clone()),
            signed_in: true,
            installed: true,
            located: true,
            ..Default::default()
        }
        .view();
        assert_eq!(view.main, Main::Play);
        assert!(view.enabled && view.failed);
        assert_eq!(view.said.as_deref(), Some(tr("The game couldn't start.")));
        assert_eq!(
            view.act,
            Some(Act::Notice {
                text: detail,
                way_out: None
            })
        );
    }

    #[test]
    fn a_managed_play_build_failure_disarms_the_update_but_a_cancellation_does_not() {
        let native = StartError::Native(native_mods::Failed::Other("compiler failed".into()), None);
        let publication = StartError::Other("could not write the profile pointer".into());
        for (error, build_running) in [(&native, false), (&publication, true)] {
            let mut restart_armed = true;
            assert!(!settle_managed_play_error(error, build_running, || {
                restart_armed = false
            }));
            assert!(!restart_armed);
        }
        assert!(!settle_managed_play_error(&publication, false, || panic!(
            "no active Play build"
        )));

        for error in [
            StartError::Native(
                native_mods::Failed::Other(native_mods::CANCELLED.into()),
                None,
            ),
            StartError::Other(native_mods::CANCELLED.into()),
        ] {
            assert!(settle_managed_play_error(&error, true, || panic!(
                "Play was canceled"
            )));
        }
    }
}
