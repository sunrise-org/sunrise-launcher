use super::*;

impl Launcher {
    /// Records why a build failed. A mod's own compile failure reads as plain words with a way
    /// on; the compiler's text stays in the build log. `enabled` are the mods that were built.
    pub(super) fn native_failed(&mut self, error: String, enabled: &[String]) {
        let (text, failed) = failure_message(error, enabled);
        self.native_error = Some(text);
        if !failed.is_empty() {
            self.native_failed_mods = failed;
        }
    }

    /// Build on the profile's committed base without changing its selection or mod files.
    pub(super) fn build_native(&mut self) {
        if self.native_step.is_some()
            || self.mods.busy.is_some()
            || self.mods.world_ask.is_some()
            || self.pending_consent.is_some()
        {
            return;
        }
        self.native_error = None;
        self.native_failed_mods.clear();
        self.native_log_reset();
        let instance = instances::selected();
        self.native_for = Some(instance.id.clone());
        let build =
            match sunrise_launcher::browser::sunrise::txn::ManualBuild::admit(&instance.home, None)
            {
                Ok(build) => build,
                Err(error) => {
                    self.native_log_preflight(&error);
                    self.native_error = Some(error);
                    return;
                }
            };
        if self.settings.native_mods_allowed_at.is_none() {
            self.request_native_build_consent(build.home().to_owned());
            self.native_consent_build = Some((instance, build));
        } else {
            self.run_native_build(instance, build);
        }
    }

    pub(super) fn run_native_build(
        &mut self,
        instance: instances::Instance,
        build: sunrise_launcher::browser::sunrise::txn::ManualBuild,
    ) {
        self.native_built_mods = native_status::enabled(build.home())
            .into_iter()
            .map(|(id, ..)| id)
            .collect();
        // The attempt belongs to both the controls and the worker from the start. A stop
        // before the worker is scheduled is already visible when the worker first checks it.
        let cancellation = build.cancellation();
        self.native_cancellation = Some(cancellation.clone());
        self.native_step = Some(tr("Starting the build").into());
        self.native_progress = None;
        // The first build downloads the compiler: the card says so, with its size.
        self.first_build = sunrise_launcher::rust_toolchain::installed()
            .is_none()
            .then(|| sunrise_launcher::rust_toolchain::host().map(|h| h.download_bytes()))
            .flatten();
        let mut run = crate::mods::build_run::Run::start(
            instance.id.clone(),
            instance.name.clone(),
            crate::sheets::build_name(build.base()),
            self.mods_lines(),
            false,
        );
        run.first_build = self.first_build;
        self.ui.mods.build = Some(run);
        if std::mem::take(&mut self.ui.mods.build_wanted) {
            self.open_sheet(crate::sheets::Sheet::ModBuild);
        }
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = build.run(
                instance.native_profile,
                |step| {
                    let _ = tx.send(Msg::NativeProgress(step));
                },
                &|inventory| ask_registry(&tx, inventory),
            );
            let _ = tx.send(Msg::Native(result));
        });
    }
}

/// The SDK may ask before compiling added registry code. Losing the waiting window refuses it.
pub(super) fn ask_registry(tx: &Sender<Msg>, inventory: &native_mods::Consent) -> bool {
    let (reply, answer) = std::sync::mpsc::channel();
    tx.send(Msg::NativeRegistryConsent(inventory.clone(), reply))
        .is_ok()
        && answer.recv().unwrap_or(false)
}

/// What a failed build says, and the mods that stopped it: a mod's own compile failure, or its
/// build script or features the prebuilt game refuses, in plain words naming it, read against
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
    fn an_sdk_dependency_request_gets_only_its_own_answer() {
        let (tx, requests) = std::sync::mpsc::channel();
        let inventory = native_mods::Consent {
            added_crates: 2,
            build_scripts: vec!["builder".into()],
            proc_macros: vec!["macro".into()],
        };
        std::thread::scope(|scope| {
            for allowed in [false, true] {
                let worker = scope.spawn(|| ask_registry(&tx, &inventory));
                let Msg::NativeRegistryConsent(asked, reply) = requests.recv().unwrap() else {
                    panic!("expected a dependency-consent request");
                };
                assert_eq!(asked, inventory);
                if allowed {
                    reply.send(true).unwrap();
                } else {
                    drop(reply);
                }
                assert_eq!(worker.join().unwrap(), allowed);
            }
        });
        drop(requests);
        assert!(!ask_registry(&tx, &inventory));
    }

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
    fn a_refused_build_script_or_features_reads_like_a_mod_that_failed() {
        let built = vec!["maps".to_string(), "fast-ore".to_string()];
        for refusal in [
            native_mods::Refusal::BuildScript {
                mod_id: "fast-ore".into(),
            },
            native_mods::Refusal::Features {
                mod_id: "fast-ore".into(),
            },
        ] {
            // As the build ends: the log's line, after any mods left out before it.
            let error = format!("maps was left out\n\n{refusal}");
            let (text, failed) = failure_message(error, &built);
            assert_eq!(text, refusal.plain());
            assert_eq!(failed, ["fast-ore"]);
        }
    }
}
