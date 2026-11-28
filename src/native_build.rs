use super::*;

impl Launcher {
    pub(crate) fn on_native_message(&mut self, msg: Msg) {
        match msg {
            Msg::NightlyReceiptRetried(warning, result) => {
                if result.is_ok() {
                    self.refresh_installed();
                }
                self.finish_nightly_receipt_retry(&warning, result);
            }
            Msg::NativeProgress(step) => {
                self.native_step = Some(step.shown());
                self.native_progress = Some(step.progress());
                if let Some(run) = &mut self.mods_state.build {
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
                        if let Some(run) = &mut self.mods_state.build {
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
                if let Some(run) = &mut self.mods_state.build {
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
                if let Some(play) = crate::mods::build_run::play_on_finish(wanted, &result, plays) {
                    // The game is what the player asked for: its window gets out of the way.
                    if shown {
                        self.close_sheet();
                    }
                    self.play(play);
                }
            }
            _ => unreachable!("not a native build message"),
        }
    }

    /// Records why a build failed. A mod's own compile failure reads as plain words with a way
    /// on; the compiler's text stays in the build log. `enabled` are the mods that were built.
    pub(super) fn native_failed(&mut self, error: String, enabled: &[String]) {
        let (text, failed) = failure_message(error, enabled);
        self.native_error = Some(text);
        if !failed.is_empty() {
            self.native_failed_mods = failed;
        }
    }

    /// Builds the selected instance's native mods, once the player accepted the warning. The
    /// build stays on the Sunrise the instance plays (`native_mods::base_for`), unless the
    /// warning was Update's: then it moves to that newer Sunrise.
    pub(super) fn build_native(&mut self) {
        self.build_native_on(None, false)
    }

    /// [`Self::build_native`] on `base` when given (an update's newer release or Nightly) instead
    /// of the Sunrise the instance plays; with `leave_out`, the player chose to build without the mods
    /// that worlds were saved with.
    pub(super) fn build_native_on(&mut self, base: Option<String>, leave_out: bool) {
        if self.native_step.is_some() {
            return;
        }
        self.native_error = None;
        self.native_failed_mods.clear();
        self.native_worlds = None;
        self.mods.unbuilt = None;
        // This attempt's own log, starting empty: an open panel shows it rather than the last
        // attempt's (`native_log_page`).
        self.native_log_reset();
        let instance = instances::selected();
        self.native_for = Some(instance.id.clone());
        let plays = self.version_of(&instance).cloned();
        let base = base.or_else(|| native_mods::base_for(plays.as_deref(), &self.installed, false));
        let Some(base) = base else {
            let error = tr("Install Sunrise first.").to_string();
            self.native_log_preflight(&error);
            self.native_error = Some(error);
            self.play_after_build = None;
            return;
        };
        // The mods this build is given: its failure is read against them, even if another
        // instance is selected by the time it ends.
        self.native_built_mods = native_status::enabled(&self.mods.home())
            .into_iter()
            .map(|(id, ..)| id)
            .collect();
        // The attempt belongs to both the controls and the worker from the start. A stop
        // before the worker is scheduled is already visible when the worker first checks it.
        let cancellation = native_mods::Cancellation::default();
        self.native_cancellation = Some(cancellation.clone());
        self.native_step = Some(tr("Starting the build").into());
        self.native_progress = None;
        // The first build downloads the compiler: the card says so, with its size.
        self.first_build = sunrise_launcher::rust_toolchain::installed()
            .is_none()
            .then(|| sunrise_launcher::rust_toolchain::host().map(|h| h.download_bytes()))
            .flatten();
        // The build window's build: a line per mod the build is given. Play's own build opens
        // the window; Build opens it itself.
        let for_play = self
            .play_after_build
            .as_ref()
            .is_some_and(|play| play.instance == instance.id);
        let mut run = crate::mods::build_run::Run::start(
            instance.id.clone(),
            instance.name.clone(),
            crate::sheets::build_name(&base),
            self.mods_lines(),
            for_play,
        );
        run.first_build = self.first_build;
        self.mods_state.build = Some(run);
        if for_play || std::mem::take(&mut self.mods_state.build_wanted) {
            self.open_sheet(crate::sheets::Sheet::ModBuild);
        }
        let tx = self.tx.clone();
        // The release list the window already has: the build reads it and asks GitHub for none.
        let catalogue = match &self.releases {
            Load::Ready(releases) => Some(releases.clone()),
            _ => None,
        };
        std::thread::spawn(move || {
            let result = native_mods::installed_mods(&instance.home)
                .and_then(|folder| {
                    native_mods::bundles(&folder)
                        .map(|_| folder)
                        .map_err(native_mods::Failed::Other)
                })
                .and_then(|folder| {
                    native_mods::build(
                        &base,
                        &folder,
                        &instance,
                        None,
                        leave_out,
                        &cancellation,
                        |step| {
                            let _ = tx.send(Msg::NativeProgress(step));
                        },
                        move || catalogue,
                    )
                });
            let _ = tx.send(Msg::Native(
                result.map(native_mods::ClientBuild::into_version),
            ));
        });
    }
}

/// What a failed build says, and the mods that stopped it: a mod's own compile failure, or the
/// features the prebuilt game refuses, in plain words naming it, read against
/// `built`, the mods the build was given (never the instance selected when it ends); any other
/// failure as it is.
fn failure_message(error: String, built: &[String]) -> (String, Vec<String>) {
    if let Some(id) = native_mods::compile_failed_mod(&error, built) {
        return (native_mods::fit::compile_failed(id), vec![id.clone()]);
    }
    match native_mods::refused_mod(&error, built) {
        Some((id, refusal)) => (refusal.plain(), vec![id.clone()]),
        None => (error, Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_after_switching_instances_is_read_against_the_mods_built() {
        let error = native_mods::failure(
            &native_mods::compiling_mod("fast-ore"),
            "exit status: 101",
            &["error[E0425]: cannot find value `speed` in this scope".into()],
            &["error: could not compile `fast-ore` (lib) due to 1 previous error".into()],
        );
        // The build was given fast-ore: its failure names it, whatever is selected now.
        let built = vec!["fast-ore".to_string(), "greeter".to_string()];
        let (text, failed) = failure_message(error.clone(), &built);
        assert_eq!(text, native_mods::fit::compile_failed(&built[0]));
        assert_eq!(failed, ["fast-ore"]);
        // Read against another instance's mods it could not be attributed.
        let (text, failed) = failure_message(error.clone(), &["maps".to_string()]);
        assert_eq!(text, error);
        assert!(failed.is_empty());
    }

    #[test]
    fn refused_features_read_like_a_mod_that_failed() {
        let built = vec!["maps".to_string(), "fast-ore".to_string()];
        let refusal = native_mods::Refusal::Features {
            mod_id: "fast-ore".into(),
        };
        // As the build ends: the log's line, after any mods left out before it.
        let error = format!("maps was left out\n\n{refusal}");
        let (text, failed) = failure_message(error, &built);
        assert_eq!(text, refusal.plain());
        assert_eq!(failed, ["fast-ore"]);
    }
}
