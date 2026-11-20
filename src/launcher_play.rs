//! Installing and removing builds, preparing and starting the game, and the accounts' sign-in.
use super::*;

/// Who and what Play was accepted for, before any install, preparation or build.
pub(crate) struct PlayIntent {
    pub instance: String,
    pub account: Option<String>,
    pub owner: Option<std::sync::Arc<sunrise_launcher::browser::sunrise::txn::Lease>>,
}

impl PlayIntent {
    pub fn new(instance: String, selected: Option<&account::Account>) -> Self {
        Self {
            instance,
            account: selected.map(|a| a.uuid.clone()),
            owner: None,
        }
    }
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
        if release.asset.is_none() || self.removing.contains(&release.tag) {
            return;
        }
        let release = release.clone();
        self.install_worker(release.tag.clone(), move |progress| {
            backend::install(&release, progress)
        });
    }

    pub(super) fn install_bootstrap(
        &mut self,
        owner: std::sync::Arc<sunrise_launcher::browser::sunrise::txn::Lease>,
        target: sunrise_launcher::browser::sunrise::txn::Target,
    ) {
        self.install_worker(target.base().into(), move |progress| {
            owner.install_bootstrap(&target, progress)
        });
    }

    fn install_worker(
        &mut self,
        tag: String,
        work: impl FnOnce(&dyn Fn(f32)) -> Result<(), String> + Send + 'static,
    ) {
        let tx = self.tx.clone();
        self.installing = Some((tag.clone(), 0.0));
        self.install_started = Some(Instant::now());
        self.bar = (0., 0.);
        self.install_error = None;
        std::thread::spawn(move || {
            let progress = |p| {
                let _ = tx.send(Msg::Progress(tag.clone(), p));
            };
            let result = work(&progress);
            let _ = tx.send(Msg::Installed(tag, result));
        });
    }

    /// Prepares the selected version, its Minecraft files from the player's install or, for an
    /// owner who asks, from Mojang.
    pub(crate) fn prepare(&mut self, from: backend::MinecraftFrom) {
        let Some(tag) = self.selected().cloned() else {
            return;
        };
        self.prepare_tag(tag, from, None);
    }

    pub(super) fn prepare_tag(
        &mut self,
        tag: String,
        from: backend::MinecraftFrom,
        owner: Option<std::sync::Arc<sunrise_launcher::browser::sunrise::txn::Lease>>,
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
            let lock = match backend::prepare_lock(&tag) {
                Ok(lock) => lock,
                Err(e) => {
                    let _ = tx.send(Msg::Exit(Job::Prepare, Err(e)));
                    return;
                }
            };
            let result = backend::preparer(&tag, &minecraft, from, &log)
                .inspect(|(_, minecraft)| backend::sounds(&tag, minecraft, &log))
                .map(|(p, _)| {
                    backend::set_prepare_step(backend::PrepareStep::Prepare);
                    p
                })
                .and_then(|cmd| {
                    backend::spawn_holding(
                        Job::Prepare,
                        cmd,
                        tx.clone(),
                        Some(lock),
                        owner.as_deref(),
                    )
                });
            // A running preparer sends its own Exit when it ends.
            if let Err(e) = result {
                let _ = tx.send(Msg::Exit(Job::Prepare, Err(e)));
            }
        });
        self.preparing = true;
        self.prepare_started = Some(Instant::now());
        self.download_seen = None;
    }

    pub(super) fn play(&mut self, intent: PlayIntent, tag: String) {
        let bound = intent
            .owner
            .as_ref()
            .is_some_and(|owner| owner.active_build().is_ok_and(|active| active.is_some()));
        let check =
            !bound || self.client.is_some() || self.starting_client.is_some() || self.preparing;
        if let Some(why) = check.then(|| self.why_not_now(&tag)).flatten() {
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

    /// Starts the game: the session first (a network round trip when the token needs
    /// renewing), then `drain` starts the process when it arrives.
    fn start_client(&mut self, tag: String, intent: PlayIntent) {
        self.client_error = None;
        self.crash = None;
        // Refuse revoked permission early as well as at spawn. The prepared command's
        // compiled inventory is checked off-thread once its actual profile has been applied.
        if let Err(error) = native_status::check_permission(&tag) {
            self.client_error = Some(error);
            self.starting_pack = None;
            self.joining = None;
            self.quick_play = None;
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
        self.starting_play = Some(intent);
        std::thread::spawn(move || {
            // One line per step: a terminal run shows where Play stops (#12179).
            backend::play_step("reading the sign-in");
            let _ = tx.send(Msg::PlayStep(backend::PlayStep::SignIn));
            let session = if sunrise_launcher::developer_offline::enabled(&tag) {
                Ok(None)
            } else {
                match &accepted {
                    Some(uuid) => account::session_for_game_accepted(&backend::game_home(), uuid),
                    None => Err(account::NoOwner::default()),
                }
                .map(Some)
            };
            backend::play_step("starting the game");
            let _ = tx.send(Msg::Session(session));
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
        crate::glass_toast::say(tr("Wait for the game to start before changing accounts."));
    }

    /// Generic Add account and targeted re-sign-in share the launcher-owned device-code flow.
    pub(crate) fn start_sign_in(&mut self) {
        if self.account_locked() {
            self.say_account_locked();
            return;
        }
        self.begin_sign_in(None);
    }

    pub(crate) fn re_sign_in(&mut self, uuid: &str) {
        if let Some(account) = self.accounts.find(uuid).cloned() {
            self.begin_sign_in(Some(account));
        }
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
        self.sign_in_target = target;
        self.sign_in_state = Some(SignInState::Starting);
    }

    pub(crate) fn cancel_sign_in(&mut self) {
        if self.ui.after_sign_in.take().is_some() {
            self.quick_play = None;
            self.joining = None;
        }
        if let Some(old) = self.sign_in.take() {
            old.cancel();
        }
        self.sign_in_state = None;
        self.sign_in_target = None;
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let intent = PlayIntent::new("sunrise".into(), accounts.selected());
        // A sign-out in flight when Play was pressed completes during preparation.
        accounts.accounts.remove(0);
        accounts.selected = Some("second".into());
        let accepted = intent.account.as_deref().unwrap();
        assert_eq!(accepted, "first");
        assert!(account::check_launch_account(&accounts, accepted).is_err());
        assert_eq!(accounts.selected().unwrap().uuid, "second");
    }
}
