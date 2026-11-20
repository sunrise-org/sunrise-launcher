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
            Some(perf::Step::Go(page)) => {
                let pages: Vec<Page> = Page::ALL.iter().map(|p| p.0).collect();
                self.go(pages[page % pages.len()]);
                changed = true;
            }
            Some(perf::Step::Scroll) => {
                self.settings_open(settings::Section::Storage);
                self.navigate(frame::Page::Settings);
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
                    self.refresh_open_storage_builds();
                    // Once the list is in (or failed), each instance resolves to the build it plays.
                    if !self.mods.recovered {
                        self.mods.recovered = true;
                        self.mods_recover();
                    }
                }
                Msg::Links(links) => self.links = links,
                // The page reads is_prepared when drawn; `changed` redraws it.
                Msg::Reused(_) => {
                    self.installed = backend::installed();
                    self.refresh_open_storage_builds();
                }
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
                Msg::Session(session) => self.handle_game_session(session),
                Msg::JarConsent { jars, reply } => jar_launch::consent(window, jars, reply, cx),
                Msg::JarClient {
                    run,
                    account: accepted,
                    profile_owner,
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
                        backend::spawn(Job::Client, cmd, self.tx.clone(), Some(&profile_owner))
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
                    let for_instance = self.instance_install.take_if(|t| *t == tag).is_some();
                    let cache_only = self.switch_cache_download_finished(&tag);
                    if let (Ok(()), Some(started)) = (&result, self.install_started.take()) {
                        self.perf.record_since("launcher.install_ms", started);
                    }
                    let resume = self
                        .ui
                        .after_install
                        .take_if(|(_, wanted)| *wanted == tag)
                        .filter(|_| result.is_ok());
                    match result {
                        Ok(()) => {
                            settle_download_offer(&mut self.updates, &tag, cache_only);
                            // An instance's install does not change other profiles. A picker
                            // download is only a staged candidate, not a profile switch.
                            if resume.is_none() {
                                if !cache_only
                                    && (!for_instance || self.launcher_version().is_none())
                                {
                                    self.settings.selected = Some(tag.clone());
                                }
                                // Pruning reads the selection from disk. A failed save leaves
                                // everything in place, and the just-downloaded build always stays.
                                if (cache_only || self.save_settings())
                                    && self.settings.auto_prune()
                                {
                                    let tx = self.tx.clone();
                                    let releases = self.game_releases().to_vec();
                                    std::thread::spawn(move || {
                                        let errors = backend::version_storage::prune_old_versions(
                                            &releases, &tag, cache_only,
                                        );
                                        let _ = tx.send(Msg::Pruned(errors));
                                    });
                                }
                            }
                        }
                        Err(e) => self.install_error = Some((tag, e)),
                    }
                    self.refresh_open_storage_builds();
                    if let Some((intent, _)) = resume {
                        self.resume_play(intent);
                    }
                    if let Some(next) = self.install_next.take() {
                        self.install_for_instance(next);
                    }
                }
                Msg::Removed(errors) => self.storage_builds_removed(errors),
                Msg::BuildInventory(generation, rows) => {
                    self.storage_builds_arrived(generation, rows)
                }
                Msg::Storage(result) => self.storage_arrived(result),
                Msg::Imported(result) => self.storage_build_imported(result),
                Msg::Pruned(errors) => self.storage_builds_pruned(errors),
                Msg::Updates(updates) => {
                    self.checking_updates = false;
                    self.launcher_checked_now = None;
                    if let Some(error) = &updates.launcher_error {
                        settings::log_launcher_check_failure(error, |line| eprintln!("{line}"));
                    }
                    self.updates = *updates;
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
                Msg::PlayImage(result) => {
                    self.native_cancellation = None;
                    self.native_step = None;
                    self.native_progress = None;
                    let wanted = self.play_after_build.take();
                    match result {
                        Ok(tag) => {
                            self.installed = backend::installed();
                            self.reload_instances();
                            if let Some(intent) = wanted.filter(|intent| {
                                self.selected_instance()
                                    .is_some_and(|instance| instance.id == intent.instance)
                            }) {
                                self.play(intent, tag);
                            }
                        }
                        Err(error) => self.client_error = Some(error),
                    }
                }
                Msg::Native(result) => {
                    self.native_cancellation = None;
                    self.native_step = None;
                    self.native_progress = None;
                    self.first_build = None;
                    let cancelled = matches!(&result, Err(native_mods::Failed::Other(error)) if error == native_mods::CANCELLED);
                    self.installed = backend::installed();
                    self.refresh_open_storage_builds();
                    self.reload_instances();
                    let built = std::mem::take(&mut self.native_built_mods);
                    if let (Err(error), false) = (&result, cancelled) {
                        self.native_failed(error.to_string(), &built);
                        if let native_mods::Failed::Mods { left, .. } = error {
                            self.native_failed_mods =
                                left.iter().map(|left| left.id.clone()).collect();
                        }
                    }
                    let shown = self.ui.sheet == Some(crate::sheets::Sheet::ModBuild);
                    if let Some(run) = &mut self.ui.mods.build {
                        match (&result, cancelled) {
                            (Ok(_), _) => run.done(),
                            (Err(_), true) => run.cancelled(),
                            (Err(_), false) => run.stopped(
                                self.native_error.clone().unwrap_or_default(),
                                &self.native_failed_mods,
                            ),
                        }
                        if !shown {
                            run.said_at = Some(Instant::now());
                            std::thread::spawn(|| {
                                std::thread::sleep(crate::mods::build_run::SAID_FOR);
                                crate::wake::wake();
                            });
                        }
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
                            self.refresh_open_storage_builds();
                            // Play pressed before the version was prepared plays now.
                            if let Some((tag, intent)) = self.play_when_prepared.take() {
                                if error.is_none() && backend::check_prepared(&tag) {
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
                            self.refresh_open_storage_builds();
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

// Downloading a candidate is not the update. An abandoned picker must leave its offer intact.
fn settle_download_offer(updates: &mut backend::Updates, tag: &str, cache_only: bool) {
    if !cache_only
        && updates
            .sunrise
            .as_ref()
            .is_some_and(|release| release.tag == tag)
    {
        updates.sunrise = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cached_candidate_does_not_consume_the_offer_if_the_picker_is_cancelled() {
        let tag = "nightly-2026-11-20-aaaaaaa";
        let mut updates = backend::Updates {
            sunrise: Some(backend::Release {
                tag: tag.into(),
                name: String::new(),
                body: String::new(),
                date: String::new(),
                time: String::new(),
                asset: None,
                prerelease: false,
                nightly: None,
            }),
            ..Default::default()
        };
        settle_download_offer(&mut updates, tag, true);
        // Cancel leaves the candidate cached but does not commit a Switch.
        assert_eq!(updates.sunrise.as_ref().map(|r| r.tag.as_str()), Some(tag));
        settle_download_offer(&mut updates, tag, false);
        assert!(updates.sunrise.is_none());
    }
}
