//! The backend's messages, drained as they arrive (`Launcher::drain`, `wake.rs`).
use super::*;
use sunrise_launcher::managed_play::StartError;

/// A canceled Play is not a build failure. Otherwise keep the window open for the native
/// build diagnostic instead of restarting as soon as Play stops.
pub(crate) fn settle_managed_play_error(
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
pub(crate) fn finish_play_build_at_handoff(
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
pub(crate) fn route_active_play_build_error(
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
        StartError::Account(refused) => {
            run.stopped(refused.in_window(), String::new());
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
            Some(perf::Step::Install) => {
                // A local `.smod` read and installed into the benchmark's scratch profile
                // folder. Both paths come from `perf::benchmark`; nothing here touches the
                // network or the player's profile.
                let started = self.bench.as_mut().and_then(perf::Bench::start_install);
                let result = match started {
                    Some((smod, home)) => sunrise_launcher::github_mods::read_local_smod(&smod)
                        .and_then(|package| {
                            sunrise_launcher::github_mods::install_local_packages(
                                &home,
                                std::slice::from_ref(&package),
                            )
                            .map(|_| ())
                        }),
                    None => Err("the benchmark's install step had no artifact".to_owned()),
                };
                if let Some(bench) = self.bench.as_mut() {
                    bench.installed(result.err());
                }
                changed = true;
            }
            Some(perf::Step::Quit) => cx.quit(),
            Some(perf::Step::Wait) | None => {}
        }
        while let Ok(msg) = self.rx.try_recv() {
            changed = true;
            match msg {
                Msg::Keychain(_)
                | Msg::Owner(_)
                | Msg::AccountHead(_, _, _, _)
                | Msg::SkinsProfile(_, _, _)
                | Msg::Picture(_, _)
                | Msg::LookSaved(_, _, _)
                | Msg::SkinsAdded(_, _, _)
                | Msg::SkinsListed(_, _)
                | Msg::LookRecorded(_, _, _)
                | Msg::SkinRemoved(_, _, _, _)
                | Msg::SkinFile(_, _, _)
                | Msg::SkinKept(_, _, _, _)
                | Msg::AccountProgress(_, _, _)
                | Msg::AccountStatus(_, _, _)
                | Msg::AccountRemoved(_, _, _) => self.on_account_message(msg),
                Msg::Releases(_)
                | Msg::Links(_)
                | Msg::Reused(_)
                | Msg::Progress(_, _)
                | Msg::Installed(_, _)
                | Msg::ConfirmUnreviewed(_, _) => self.on_install_message(msg),
                Msg::Playing(_)
                | Msg::PlayStep(_)
                | Msg::PlayDownload(_)
                | Msg::PlayDetail(_)
                | Msg::ManagedBuild { .. }
                | Msg::ManagedBuildLog(_)
                | Msg::ManagedClient { .. }
                | Msg::PlayRecorded
                | Msg::GameExitSettled
                | Msg::PlayAdmission(_, _) => self.on_play_message(msg, cx),
                Msg::Session(_)
                | Msg::JarConsent { .. }
                | Msg::JarClient { .. }
                | Msg::Line(_, _)
                | Msg::ClientObservationLost(_) => self.on_session_message(msg, window, cx),
                Msg::SwitchProgress(_)
                | Msg::SwitchDetail(_)
                | Msg::SwitchQuestion(_, _)
                | Msg::Switched(_, _) => self.on_switch_message(msg),
                Msg::ProviderSwitchProgress(_)
                | Msg::ProviderSwitchQuestion(_, _)
                | Msg::ProviderSwitched(_, _)
                | Msg::ProviderChoice(_, _)
                | Msg::PublisherChangeQuestion(_, _)
                | Msg::ReceiptAcknowledged(_, _) => self.on_provider_message(msg),
                Msg::NightlyReceiptRetried(_, _) | Msg::NativeProgress(_) | Msg::Native(_) => {
                    self.on_native_message(msg)
                }
                Msg::Removed(_)
                | Msg::BuildInventory(_, _)
                | Msg::Storage(_, _)
                | Msg::Imported(_)
                | Msg::Pruned(_) => self.on_storage_message(msg),
                Msg::Updates(_)
                | Msg::LauncherChecked(_)
                | Msg::LauncherProgress(_)
                | Msg::LauncherUpdated(_) => self.on_update_message(msg),
                Msg::Exit(_, _) => self.on_game_exit_message(msg, cx),
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
