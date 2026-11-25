//! Installing and removing builds, preparing and starting the game, and the accounts' sign-in.
use super::*;

/// Who and what Play was accepted for, before any install, preparation or build.
#[derive(Clone)]
pub(crate) struct PlayIntent {
    pub instance: String,
    pub incarnation: Option<instances::Incarnation>,
    pub account: Option<String>,
    pub destination: PlayDestination,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum PlayDestination {
    #[default]
    Home,
    Quick(&'static str, String),
    Invite(String),
}

impl PlayDestination {
    pub(crate) fn check_mod_recovery(&self) -> Result<(), &'static str> {
        if matches!(self, Self::Quick("--world", _)) {
            Err(tr(
                "This world was not opened. Turning off mods could remove content from it.",
            ))
        } else {
            Ok(())
        }
    }

    pub(super) fn apply(self, cmd: &mut std::process::Command) -> Result<(), String> {
        match self {
            Self::Home => (),
            Self::Quick(flag, value) => {
                cmd.arg(flag).arg(value);
            }
            Self::Invite(code) => backend::join_friend(cmd, &code)?,
        }
        Ok(())
    }
}

impl PlayIntent {
    pub fn new(
        instance: &instances::Instance,
        selected: Option<&account::Account>,
        destination: PlayDestination,
    ) -> Self {
        Self {
            instance: instance.id.clone(),
            incarnation: instance.incarnation,
            account: selected.map(|a| a.uuid.clone()),
            destination,
        }
    }

    pub(super) fn matches_profile(&self, current: &instances::Instance) -> bool {
        current.id == self.instance && current.incarnation == self.incarnation
    }
}

impl Launcher {
    pub(crate) fn retry_managed_play(
        &mut self,
        recovery: Option<sunrise_launcher::managed_play::ModRecovery>,
    ) {
        if self.client.is_some()
            || (self.launch_busy() && (recovery.is_some() || !self.registry_accepts_play()))
        {
            return;
        }
        let (Some(request), Some(current)) = (self.play_retry.clone(), self.selected_instance())
        else {
            return;
        };
        if !request.matches_profile(current) {
            return;
        }
        if recovery.is_some() {
            if let Err(reason) = request.destination.check_mod_recovery() {
                self.client_error = Some(reason.into());
                return;
            }
        }
        let current = current.clone();
        self.close_sheet();
        self.client_error = None;
        self.prepare_error = None;
        if recovery.is_some() {
            self.start_managed_client(current, request, recovery);
        } else {
            self.resume_play(request);
        }
    }
}

/// An older update ledger is advisory, never an authorization step or updater in Play.
fn continue_after_assets(
    tag: &str,
    accepted: Option<&str>,
    legacy_home: Option<&std::path::Path>,
    tx: &std::sync::mpsc::Sender<Msg>,
    mods_tx: &std::sync::mpsc::Sender<mods_page::Event>,
) {
    if let Some(home) = legacy_home {
        backend::play_step("checking the legacy mod backup ledger");
        let _ = tx.send(Msg::PlayStep(backend::PlayStep::Mods));
        if let Err(notice) = sunrise_launcher::browser::sunrise::auto::recover(home) {
            let _ = mods_tx.send(mods_page::Event::LegacyBackupNotice(notice));
        }
    }
    backend::play_step("reading the sign-in");
    let _ = tx.send(Msg::PlayStep(backend::PlayStep::SignIn));
    let session = if sunrise_launcher::developer_offline::enabled(tag) {
        Ok(None)
    } else {
        match accepted {
            Some(uuid) => account::session_for_game_accepted(&backend::game_home(), uuid),
            None => Err(account::NoOwner::default()),
        }
        .map(Some)
    };
    backend::play_step("starting the game");
    let _ = tx.send(Msg::Session(session));
}

impl Launcher {
    /// After a crash with no report: when the game's output ends while its Vulkan renderer
    /// started, a native crash in the graphics driver (#746), switches the game to
    /// `backend::vulkan_crash_fallback` (DirectX 12 on Windows, OpenGL on Linux; only where the
    /// player left the graphics API on Auto). The API's label when it did.
    pub(crate) fn switched_from_vulkan(&self) -> Option<&'static str> {
        let (api, label) = backend::vulkan_crash_fallback(std::env::consts::OS)?;
        if !backend::vulkan_start_crashed(&self.prepare_log) {
            return None;
        }
        // The GPU is the same for every profile: the game home's setting and the crashed run's.
        let home = backend::game_home();
        let mut homes = vec![home.clone()];
        homes.extend(self.client_home.clone().filter(|h| *h != home));
        let mut switched = false;
        for home in homes {
            match backend::switch_from_auto(&home, api) {
                Ok(changed) => switched |= changed,
                Err(e) => eprintln!("Sunrise launcher: could not switch to {label}: {e}"),
            }
        }
        switched.then_some(label)
    }

    /// The installed version Play starts; an uninstalled profile target can't start yet.
    pub(crate) fn selected(&self) -> Option<&String> {
        match self.selected_instance() {
            Some(instance) => self.version_of(instance),
            None => self.launcher_version(),
        }
    }

    /// The launcher's version: what instances without one of their own play.
    pub(crate) fn launcher_version(&self) -> Option<&String> {
        self.settings
            .selected
            .as_ref()
            .filter(|s| self.installed.contains(s))
    }

    pub(crate) fn on_install_message(&mut self, msg: Msg) {
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
            Msg::Progress(tag, p) => {
                self.bar = (self.bar.1, p);
                self.switch_download_progress(&tag, p);
                self.installing = Some((tag, p));
            }
            Msg::Installed(tag, result) => {
                self.installing = None;
                self.bar = (0., 0.);
                self.refresh_installed();
                if let Some(warning) = self.record_nightly_receipt_outcome(&tag, &result) {
                    toast::say(trf(
                        "{warning} Try again in Settings → Storage.",
                        &[("warning", &warning)],
                    ));
                }
                if self.finish_switch_download(&tag, &result) {
                    self.install_started = None;
                    if let Some(next) = self.install_next.take() {
                        self.install_for_instance(next);
                    }
                    return;
                }
                let receipt_only = matches!(&result, Ok(backend::InstallOutcome::ReceiptOnly(_)));
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
            Msg::ConfirmUnreviewed(found, reply) => {
                let pending = crate::github_mods_page::PendingUnreviewed::new(found, reply);
                if let Some(earlier) = self.ui.sheets.unreviewed.replace(pending) {
                    earlier.refuse();
                }
            }
            _ => unreachable!("not an install message"),
        }
    }

    /// Installs `release` for an instance made or edited to play it, after the install running
    /// now if there is one.
    pub(crate) fn install_for_instance(&mut self, release: Release) {
        match &self.installing {
            Some((now, _)) if *now == release.tag => {}
            Some(_) => self.install_next = Some(release),
            None => {
                self.instance_install = Some(release.tag.clone());
                self.install(&release);
            }
        }
    }

    pub(crate) fn install(&mut self, release: &Release) {
        self.install_cancellable(release, None);
    }

    pub(crate) fn install_cancellable(
        &mut self,
        release: &Release,
        stop: Option<native_mods::Cancellation>,
    ) {
        if release.asset.is_none() || self.removing.contains(&release.tag) {
            return;
        }
        let (release, tx) = (release.clone(), self.tx.clone());
        let tag = release.tag.clone();
        self.installing = Some((tag.clone(), 0.0));
        self.install_started = Some(Instant::now());
        self.bar = (0., 0.);
        self.install_error = None;
        std::thread::spawn(move || {
            let progress = |p| {
                let _ = tx.send(Msg::Progress(tag.clone(), p));
            };
            let result = backend::install_cancellable(&release, stop.as_ref(), progress);
            let _ = tx.send(Msg::Installed(tag, result));
        });
    }

    /// Prepares the selected version, its Minecraft files from the player's install or, for an
    /// owner who asks, from Mojang.
    pub(crate) fn prepare(&mut self, from: backend::MinecraftFrom) {
        let Some(tag) = self.selected().cloned() else {
            return;
        };
        self.prepare_version(tag, from);
    }

    pub(crate) fn prepare_version(&mut self, tag: String, from: backend::MinecraftFrom) {
        self.prepare_version_cancellable(tag, from, None);
    }

    pub(crate) fn prepare_version_cancellable(
        &mut self,
        tag: String,
        from: backend::MinecraftFrom,
        stop: Option<native_mods::Cancellation>,
    ) {
        sunrise_launcher::mojang::CANCEL.store(false, Ordering::Relaxed);
        let minecraft = self
            .settings
            .minecraft_dir
            .clone()
            .unwrap_or_else(|| sunrise_launcher::preparer::minecraft_dir(None));
        self.prepare_log.clear();
        self.prepare_error = None;
        self.prepare_log
            .push_back("Checking Java and your Minecraft install".into());
        let tx = self.tx.clone();
        // The checks may download the server jar, so they run on the thread too.
        std::thread::spawn(move || {
            let log = |line| {
                let _ = tx.send(Msg::Line(Job::Prepare, line));
            };
            let result = (|| {
                let lock = backend::prepare_lock(&tag)?;
                let (cmd, minecraft) = match &stop {
                    Some(stop) => {
                        backend::preparer_cancellable(&tag, &minecraft, from, &log, &lock, stop)?
                    }
                    None => backend::preparer(&tag, &minecraft, from, &log, &lock)?,
                };
                match &stop {
                    Some(stop) => backend::sounds_cancellable(&tag, &minecraft, &log, &lock, stop),
                    None => backend::sounds(&tag, &minecraft, &log, &lock),
                }
                backend::set_prepare_step(backend::PrepareStep::Prepare);
                backend::spawn_holding_cancellable(
                    Job::Prepare,
                    cmd,
                    tx.clone(),
                    Some((tag, lock)),
                    stop,
                )
            })();
            // A running preparer sends its own Exit when it ends.
            if let Err(e) = result {
                let _ = tx.send(Msg::Exit(Job::Prepare, Err(e)));
            }
        });
        self.preparing = true;
        self.prepare_started = Some(Instant::now());
        self.download_seen = None;
    }

    pub(super) fn play(&mut self, intent: PlayIntent) {
        if !self
            .selected_instance()
            .is_some_and(|current| intent.matches_profile(current))
        {
            return;
        }
        // A press that starts nothing says why, where Play's errors show: the button can look
        // ready when it is not (a version prepared elsewhere and since removed, a debug preview).
        let Some(tag) = self.selected().cloned() else {
            self.client_error = Some(tr("Install a Sunrise game build first.").into());
            return;
        };
        if let Some(why) = self.why_not_now(&tag) {
            self.client_error = Some(why);
            return;
        }
        self.starting_pack = None;
        self.start_client(tag, intent);
    }

    /// Why `tag` cannot start now, as the player reads it; None when it can ([`Self::can_start`]).
    fn why_not_now(&self, tag: &str) -> Option<String> {
        if self.can_start(tag) {
            None
        } else if self.client.is_some() || self.starting_client.is_some() {
            Some(tr("The game is already running.").into())
        } else if self.preparing {
            Some(tr("Wait for the preparation to finish, then press Play.").into())
        } else {
            Some(trf(
                "{version} isn't prepared yet: choose Prepare in Play's menu.",
                &[("version", &self.version_label(tag))],
            ))
        }
    }

    /// Whether `tag` can start now ([`starts_now`]): prepared, no preparation under way, no game
    /// running or starting.
    pub(crate) fn can_start(&self, tag: &str) -> bool {
        let running = self.client.is_some() || self.starting_client.is_some();
        starts_now(backend::is_prepared(tag), self.preparing, running)
    }

    pub(super) fn start_managed_client(
        &mut self,
        instance: instances::Instance,
        intent: PlayIntent,
        recovery: Option<sunrise_launcher::managed_play::ModRecovery>,
    ) {
        use sunrise_launcher::managed_play::{Client, FirstBuild, Progress};
        let target = self.profile_target(&instance);
        let release = target
            .as_deref()
            .and_then(|tag| self.release_to_install(tag));
        let absent = match &self.releases {
            Load::Loading => {
                tr("Checking available builds. Press Play when the list is ready.").into()
            }
            Load::Failed(error) => error.clone(),
            Load::Ready(_) => {
                tr("No downloadable build matches this profile. Choose a game build.").into()
            }
        };
        self.play_retry = Some(intent.clone());
        if self
            .mods_state
            .build
            .as_ref()
            .is_some_and(|run| run.for_play)
        {
            self.mods_state.build = None;
        }
        self.starting_pack = None;
        self.play_install_recovery = None;
        self.native_error = None;
        self.prepare_log.clear();
        self.client_started =
            Some(std::time::SystemTime::now() - std::time::Duration::from_secs(2));
        self.play_pressed = Some(Instant::now());
        self.starting_client = Some(target.clone().unwrap_or_default());
        self.play_step = Some(backend::PlayStep::Assets);
        let cancellation = native_mods::Cancellation::default();
        self.native_cancellation = Some(cancellation.clone());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = Client {
                instance: &instance,
                accepted_account: intent.account.as_deref(),
                explicit_account: false,
                piped: true,
                registry: sunrise_launcher::browser::sunrise::v2::current_index,
            }
            .start_with_recovery(
                recovery,
                || {
                    if let Some(tag) = target
                        .as_deref()
                        .filter(|tag| backend::installed().iter().any(|found| found == tag))
                    {
                        FirstBuild::installed(tag)
                    } else if let Some(release) = release {
                        FirstBuild::download(&release)
                    } else {
                        Err(absent)
                    }
                },
                |cmd| {
                    intent.destination.apply(cmd)?;
                    backend::append_game_args(cmd);
                    Ok(())
                },
                |cmd| {
                    jar_launch::with_consent(&tx, |permission| {
                        sunrise_launcher::jars::prepare_for_command(cmd, permission)
                    })
                    .map_err(|error| error.to_string())
                },
                &cancellation,
                |progress| {
                    let msg = match progress {
                        Progress::Download(_) => Msg::PlayStep(backend::PlayStep::Download),
                        Progress::Prepare => Msg::PlayStep(backend::PlayStep::Prepare),
                        Progress::NativeBuild { base, mods } => Msg::ManagedBuild {
                            instance: instance.id.clone(),
                            profile: instance.name.clone(),
                            base,
                            mods,
                        },
                        Progress::Native(step) => Msg::NativeProgress(step),
                        Progress::NativeLog(output) => Msg::ManagedBuildLog(output),
                        Progress::SignIn => Msg::PlayStep(backend::PlayStep::SignIn),
                        Progress::Line(line) | Progress::HookLine(line) => {
                            Msg::Line(Job::Prepare, line)
                        }
                    };
                    let _ = tx.send(msg);
                },
            );
            if let Err(std::sync::mpsc::SendError(Msg::ManagedClient { result, .. })) =
                tx.send(Msg::ManagedClient {
                    instance: Box::new(instance),
                    result: Box::new(result),
                })
            {
                if let Ok(game) = *result {
                    // The admitted child owns its claims now. Closing the window does
                    // not stop it; drain its streams until this launcher exits.
                    backend::monitor_game(game.child, tx, game.runtime);
                }
            }
        });
    }

    /// Starts a pack's game through its legacy route. Ordinary profiles use the admitted worker.
    fn start_client(&mut self, tag: String, intent: PlayIntent) {
        self.client_error = None;
        self.crash = None;
        let refusal =
            backend::game_architecture_launch_error_with_catalogue(&tag, self.game_releases());
        if let Some(error) = refusal {
            self.client_error = Some(error);
            self.starting_pack = None;
            return;
        }
        self.prepare_log.clear();
        // A little earlier than the spawn: file times can be coarse.
        self.client_started =
            Some(std::time::SystemTime::now() - std::time::Duration::from_secs(2));
        // What the resources lack (built-in packs, panorama, sounds) and the session (a network
        // round trip when the token needs renewing) come first; `drain` starts the game when the
        // session arrives.
        self.starting_client = Some(tag.clone());
        self.play_step = None;
        let accepted = intent.account.clone();
        self.play_pressed = Some(Instant::now());
        let tx = self.tx.clone();
        let legacy_home = self
            .instances
            .list
            .iter()
            .find(|i| i.id == intent.instance && i.kind != instances::Kind::Pack)
            .filter(|_| self.starting_pack.is_none())
            .map(|i| i.home.clone());
        let mods_tx = self.mods.tx.clone();
        self.starting_play = Some(intent);
        std::thread::spawn(move || {
            // One line per step: a terminal run shows where Play stops (#12179).
            backend::play_step("checking the game's assets");
            let _ = tx.send(Msg::PlayStep(backend::PlayStep::Assets));
            backend::complete_assets(&tag);
            continue_after_assets(
                &tag,
                accepted.as_deref(),
                legacy_home.as_deref(),
                &tx,
                &mods_tx,
            );
        });
    }

    /// The prepared game files of the selected version (else the first installed): where the
    /// default skins are.
    pub(crate) fn game_resources(&self) -> std::path::PathBuf {
        self.selected()
            .or_else(|| self.installed.first())
            .map(|tag| backend::resources(tag))
            .unwrap_or_default()
    }

    pub(crate) fn load_profile_heads(&mut self) {
        let resources = self.game_resources();
        self.profile_heads
            .request(&self.accounts.accounts, resources, &self.tx);
    }

    pub(crate) fn begin_auth_job(&mut self, uuid: &str) -> Option<u64> {
        if self.auth_busy.contains_key(uuid) {
            return None;
        }
        self.auth_next += 1;
        self.auth_busy.insert(uuid.into(), self.auth_next);
        self.auth_progress
            .insert(uuid.into(), account::session::Phase::Queued);
        Some(self.auth_next)
    }

    /// Only workers read the keychain or contact authentication services. Cached public status
    /// keeps drawing independent of keychain prompts, network timeouts and refresh operations.
    pub(crate) fn load_auth_statuses(&mut self, renew_due: bool) {
        self.auth_checked = Some(Instant::now());
        for account in self.accounts.accounts.clone() {
            let Some(generation) = self.begin_auth_job(&account.uuid) else {
                continue;
            };
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let progress = |phase| {
                    let _ = tx.send(Msg::AccountProgress(
                        account.uuid.clone(),
                        generation,
                        phase,
                    ));
                };
                progress(account::session::Phase::Keychain);
                let store = account::store();
                let mut status =
                    account::session::status_with_progress(store, &account.uuid, &progress);
                if renew_due && status.state == account::session::State::NeedsRefresh {
                    if let Ok(renewed) =
                        account::session::refresh_with_progress(&account.uuid, &progress)
                    {
                        status = renewed;
                    }
                }
                let _ = tx.send(Msg::AccountStatus(account.uuid, generation, status));
            });
        }
    }

    /// Whether who plays is settled for now: from an accepted Play until the game has started
    /// (installing, preparing, building, signing in), the account can't be switched, signed out
    /// or joined by a new one. The start only reads the account in its Play intent; this keeps
    /// the chip showing the account that is about to play.
    pub(crate) fn account_locked(&self) -> bool {
        self.launch_busy()
            || self.ui.after_install.is_some()
            || self.play_when_prepared.is_some()
            || self.play_after_build.is_some()
    }

    /// Says why a locked account action did nothing.
    pub(crate) fn say_account_locked(&self) {
        crate::toast::say(tr("Wait for the game to start before changing accounts."));
    }

    /// Generic Add account and targeted re-sign-in share the launcher-owned device-code flow.
    pub(crate) fn start_sign_in(&mut self) {
        if self.account_locked() {
            self.say_account_locked();
            return;
        }
        self.begin_sign_in(None);
    }

    /// Signs an account in again on the sign-in screen, where its code is shown (#110).
    pub(crate) fn re_sign_in(&mut self, uuid: &str) {
        sign_in_click(self, Some(uuid.to_owned()));
    }

    /// Retry the original account when the current screen has a target.
    pub(crate) fn retry_sign_in(&mut self) {
        retry_click(self);
    }

    pub(crate) fn begin_sign_in(&mut self, target: Option<account::Account>) {
        self.cancel_sign_in();
        // The code page offers a private window: find the default browser meanwhile.
        std::thread::spawn(|| {
            sunrise_launcher::private_window::detected();
        });
        self.sign_in = Some(match &target {
            Some(account) => SignIn::start_for(account.clone()),
            None => SignIn::start(),
        });
        self.sign_in_screen.target = target;
        self.sign_in_state = Some(SignInState::Starting);
    }

    pub(crate) fn cancel_sign_in(&mut self) {
        self.ui.after_sign_in = None;
        if let Some(old) = self.sign_in.take() {
            old.cancel();
        }
        self.sign_in_state = None;
        self.sign_in_screen.target = None;
        self.sign_in_copies.clear();
    }

    pub(crate) fn choose_folder(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = paths.await {
                if let Some(path) = paths.pop() {
                    let _ = this.update(cx, |this, cx| {
                        this.settings.minecraft_dir = Some(path);
                        this.save_settings();
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }
}

/// Sign-in screen state owned by [`Launcher`] and shared directly with tests so only starting
/// the background worker is faked.
#[derive(Default)]
pub(crate) struct SignInScreen {
    /// The account being re-signed-in, or `None` when adding a new account.
    pub(crate) target: Option<account::Account>,
    /// Whether the sign-in screen is open.
    pub(crate) open: bool,
}

impl SignInScreen {
    fn retry_target(&self) -> Option<String> {
        self.target.as_ref().map(|account| account.uuid.clone())
    }
}

#[derive(Debug, PartialEq, Eq)]
enum SignInRefusal {
    Busy,
    AccountRemoved,
}

/// What the sign-in buttons need from the launcher, so their real handler runs in tests too.
trait SignInHost {
    /// A game is starting or the account can't change now.
    fn locked(&self) -> bool;
    fn account(&self, uuid: &str) -> Option<account::Account>;
    fn screen(&mut self) -> &mut SignInScreen;
    /// Starts the sign-in worker for `target`.
    fn start_worker(&mut self, target: Option<account::Account>);
    fn refuse(&mut self, refusal: SignInRefusal);
}

impl SignInHost for Launcher {
    fn locked(&self) -> bool {
        self.account_locked()
    }

    fn account(&self, uuid: &str) -> Option<account::Account> {
        self.accounts.find(uuid).cloned()
    }

    fn screen(&mut self) -> &mut SignInScreen {
        &mut self.sign_in_screen
    }

    fn start_worker(&mut self, target: Option<account::Account>) {
        self.begin_sign_in(target);
    }

    fn refuse(&mut self, refusal: SignInRefusal) {
        match refusal {
            SignInRefusal::Busy => self.say_account_locked(),
            SignInRefusal::AccountRemoved => {
                crate::toast::say(tr("This account was signed out. Add it again to sign in."))
            }
        }
    }
}

/// Sign `uuid` in again, or a new account when there is none: refused while the account is
/// locked, and when that account was removed meanwhile.
fn sign_in_click(host: &mut impl SignInHost, uuid: Option<String>) {
    if host.locked() {
        return host.refuse(SignInRefusal::Busy);
    }
    let target = match uuid {
        None => None,
        Some(uuid) => match host.account(&uuid) {
            Some(account) => Some(account),
            None => return host.refuse(SignInRefusal::AccountRemoved),
        },
    };
    host.screen().target = target.clone();
    host.start_worker(target);
    host.screen().open = true;
}

/// Retry on the sign-in screen: the account that screen was for, or a new one.
fn retry_click(host: &mut impl SignInHost) {
    let target = host.screen().retry_target();
    sign_in_click(host, target);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeSignIn {
        locked: bool,
        accounts: Vec<account::Account>,
        screen: SignInScreen,
        started: Vec<Option<String>>,
        refused: Vec<SignInRefusal>,
    }

    impl SignInHost for FakeSignIn {
        fn locked(&self) -> bool {
            self.locked
        }
        fn account(&self, uuid: &str) -> Option<account::Account> {
            self.accounts.iter().find(|a| a.uuid == uuid).cloned()
        }
        fn screen(&mut self) -> &mut SignInScreen {
            &mut self.screen
        }
        fn start_worker(&mut self, target: Option<account::Account>) {
            self.started
                .push(target.as_ref().map(|account| account.uuid.clone()));
        }
        fn refuse(&mut self, refusal: SignInRefusal) {
            self.refused.push(refusal);
        }
    }

    #[test]
    fn a_world_tile_does_not_authorize_turning_mods_off_to_play() {
        let world = PlayDestination::Quick("--world", "My world".into());
        assert_eq!(
            world.check_mod_recovery(),
            Err("This world was not opened. Turning off mods could remove content from it.")
        );
        assert_eq!(PlayDestination::Home.check_mod_recovery(), Ok(()));
        assert_eq!(
            PlayDestination::Quick("--server", "example.invalid".into()).check_mod_recovery(),
            Ok(())
        );
    }

    #[test]
    fn renewing_and_retrying_start_only_the_requested_account() {
        let original = account::Account {
            uuid: "a1".into(),
            name: "Original account".into(),
            skin: None,
            cape: None,
        };
        let mut host = FakeSignIn {
            accounts: vec![original],
            ..FakeSignIn::default()
        };
        // Sign in again opens the screen for that account; a retry after an error reuses it.
        sign_in_click(&mut host, Some("a1".into()));
        assert!(host.screen.open);
        assert_eq!(host.screen.retry_target().as_deref(), Some("a1"));
        retry_click(&mut host);
        assert_eq!(host.started, [Some("a1".into()), Some("a1".into())]);

        // The account was removed meanwhile: retry starts nothing.
        host.accounts.clear();
        host.screen.open = false;
        retry_click(&mut host);
        assert_eq!(host.started.len(), 2);
        assert!(!host.screen.open);
        assert_eq!(host.refused, [SignInRefusal::AccountRemoved]);

        // While a game starts, neither button opens the screen or starts a worker.
        let mut starting = FakeSignIn {
            locked: true,
            screen: SignInScreen {
                target: Some(account::Account {
                    uuid: "a1".into(),
                    name: "Original account".into(),
                    skin: None,
                    cape: None,
                }),
                open: false,
            },
            ..FakeSignIn::default()
        };
        sign_in_click(&mut starting, Some("a1".into()));
        retry_click(&mut starting);
        assert!(starting.started.is_empty() && !starting.screen.open);
        assert_eq!(starting.refused, [SignInRefusal::Busy, SignInRefusal::Busy]);

        // A screen with no account behind it signs a new account in.
        let mut fresh = FakeSignIn::default();
        retry_click(&mut fresh);
        assert_eq!(fresh.started, [None]);
        assert!(fresh.screen.open);
    }

    #[test]
    fn a_retained_legacy_copy_does_not_stop_play_at_the_mod_check() {
        let scratch = tempfile::tempdir().unwrap();
        let home = scratch.path();
        let mods = home.join("sunrise-mods");
        std::fs::create_dir(&mods).unwrap();
        let active = mods.join("ruby-1.0.0.smod");
        let bytes = sunrise_launcher_smod::Package::write_files(
            &format!(
                "id='ruby'\nname='Ruby'\nversion='1.0.0'\napi='{}'\ncapabilities=['registries']\n",
                sunrise_launcher_smod::manifest::API
            ),
            &[],
        )
        .unwrap();
        std::fs::write(&active, &bytes).unwrap();
        let ledger = home.join("mod-browser/replaced.json");
        let unverified = home.join("mod-browser/replaced/ruby");
        std::fs::create_dir_all(&unverified).unwrap();
        let old = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/replaced-v1.1.2.json"
        ))
        .replace(
            "PACKAGE_DIGEST",
            &sunrise_launcher_smod::index::sha256_file(&active).unwrap(),
        )
        .replace("\"pid\": 0", &format!("\"pid\": {}", std::process::id()));
        std::fs::write(&ledger, &old).unwrap();
        let entry: serde_json::Value = serde_json::from_str(&old).unwrap();
        std::fs::write(
            home.join("mod-browser.json"),
            serde_json::json!({"entries": [entry[0]["entry"]]}).to_string(),
        )
        .unwrap();
        let game_journal = home.join("mod-browser/install-journal.json");
        std::fs::write(&game_journal, b"separate journal").unwrap();
        let (tx, events) = std::sync::mpsc::channel();
        let (mods_tx, notices) = std::sync::mpsc::channel();
        continue_after_assets("v0.0.0-legacy-fixture", None, Some(home), &tx, &mods_tx);
        assert!(
            matches!(notices.try_recv().unwrap(), mods_page::Event::LegacyBackupNotice(notice) if notice.contains("You can still play"))
        );
        assert!(matches!(
            events.try_recv().unwrap(),
            Msg::PlayStep(backend::PlayStep::Mods)
        ));
        assert!(matches!(
            events.try_recv().unwrap(),
            Msg::PlayStep(backend::PlayStep::SignIn)
        ));
        assert!(matches!(events.try_recv().unwrap(), Msg::Session(Err(_)))); // Only sign-in stops this fixture.
        assert_eq!(std::fs::read_to_string(ledger).unwrap(), old);
        assert_eq!(std::fs::read(active).unwrap(), bytes);
        assert_eq!(std::fs::read(game_journal).unwrap(), b"separate journal");
        assert!(unverified.is_dir());
    }

    #[test]
    fn a_deferred_play_keeps_the_uuid_captured_before_preparation() {
        let mut accounts = Accounts::default();
        for uuid in ["first", "second"] {
            accounts.accounts.push(account::Account {
                uuid: uuid.into(),
                name: uuid.into(),
                skin: None,
                cape: None,
            });
        }
        accounts.selected = Some("first".into());
        let intent = PlayIntent {
            instance: "sunrise".into(),
            incarnation: None,
            account: accounts.selected().map(|a| a.uuid.clone()),
            destination: PlayDestination::Home,
        };
        // A sign-out in flight when Play was pressed completes during preparation.
        accounts.accounts.remove(0);
        accounts.selected = Some("second".into());
        let accepted = intent.account.as_deref().unwrap();
        assert_eq!(accepted, "first");
        assert!(account::check_launch_account(&accounts, accepted).is_err());
        assert_eq!(accounts.selected().unwrap().uuid, "second");
    }
}
