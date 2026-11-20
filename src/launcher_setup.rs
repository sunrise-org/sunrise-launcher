//! The launcher's state set up (`Launcher::new`), its settings saved, page changes, the
//! launcher's own updates and the release list.
use super::*;

impl Launcher {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        sunrise_launcher::storage::eviction::cleanup_after_start();
        // A press on something that then leaves the page (a row that opens its page, a tab)
        // would leave the keyboard on nothing, and Tab dead until the next click.
        cx.on_focus_lost(window, |this, window, cx| {
            window.focus(&this.root_focus, cx)
        })
        .detach();
        let (tx, rx) = wake::channel();
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            this.theme = this.settings_theme(window);
            cx.notify();
        });
        let window_handle = window.window_handle();
        #[cfg(feature = "frame-stats")]
        perf::frame_overlay(window);
        // The pump: on its tick, or at once when a worker says something (`wake.rs`).
        cx.spawn(async move |this, cx| loop {
            let tick = cx.background_executor().timer(wake::TICK);
            futures_util::future::select(std::pin::pin!(tick), std::pin::pin!(wake::woken())).await;
            if this
                .update(cx, |this, cx| this.drain(window_handle, cx))
                .is_err()
            {
                break;
            }
            cx.background_executor().timer(wake::SETTLE).await;
        })
        .detach();
        let mut settings = Settings::load();
        i18n::set(settings.language());
        if settings.minecraft_dir.is_none() {
            settings.minecraft_dir = backend::default_minecraft_dir();
        }
        // Release builds on Linux put their entry back in the applications menu when there is
        // none or it starts a program that is gone (a moved or deleted binary), never over one
        // that works; `SUNRISE_LAUNCHER_DESKTOP_ENTRY=1` does it in a development build.
        if cfg!(target_os = "linux")
            && settings.desktop_entry()
            && (!cfg!(debug_assertions)
                || std::env::var_os("SUNRISE_LAUNCHER_DESKTOP_ENTRY").is_some())
        {
            std::thread::spawn(|| {
                let _ = sunrise_launcher::desktop::repair(backend::exe_path());
            });
        }
        let (installed, known_nightly) = backend::play_installations(settings.selected.as_deref());
        if settings
            .selected
            .as_ref()
            .is_none_or(|s| !installed.contains(s))
        {
            settings.selected = known_nightly.clone().or_else(|| {
                installed
                    .iter()
                    .find(|tag| sunrise_launcher::nightly::date_of(tag).is_none())
                    .cloned()
            });
        }
        // Pin the window's chrome (macOS title bar) to the choice before the theme reads
        // `window.appearance()`, so both draw the same look.
        titlebar::apply_appearance(window, settings.appearance);
        let (location_error, location_unavailable) = location_page::startup_location();
        let mut this = Self {
            ui: Default::default(),
            theme: crate::theme::from_settings(&settings, window.appearance()),
            page: Page::from_env(),
            behind: Page::Play,
            switches: 0,
            content_scroll: Scrollbar::default(),
            settings_pane: gpui::ScrollHandle::new(),
            settings_query: line_edit::LineEdit::new(String::new(), 80, 40),
            settings_nav_open: false,
            mods_sticky: sticky::Sticky::default(),
            settings_sticky: sticky::Sticky::default(),
            long_list: cull::Cull::default(),
            updates: Updates::default(),
            checked: None,
            checking_updates: false,
            launcher_checked_now: None,
            launcher_notice_hidden: false,
            // The installer the last Update started could not put the new launcher in place.
            self_update: sunrise_launcher::update_handoff::take_failure()
                .map_or(SelfUpdate::Idle, |e| SelfUpdate::Stuck(Some(e))),
            update_install: None,
            architecture_reinstall: None,
            instance_install: None,
            install_next: None,
            bar: (0., 0.),
            releases: Load::Loading,
            installed,
            known_nightly,
            installing: None,
            removing: Vec::new(),
            disk: Vec::new(),
            storage_report: None,
            storage_pending: false,
            storage_confirm: None,
            storage_error: None,
            storage_revoke: None,
            storage_counting: None,
            storage_visit: 0,
            storage_revoke_error: None,
            native_consent_build: None,
            install_error: None,
            importing_build: false,
            import_error: None,
            native_step: None,
            native_progress: None,
            native_cancellation: None,
            first_build: None,
            play_after_build: None,
            native_error: None,
            native_failed_mods: Vec::new(),
            native_worlds: None,
            pending_consent: None,
            consent_error: None,
            native_for: None,
            native_built_mods: Vec::new(),
            native_builds: Default::default(),
            native_log: Default::default(),
            // The first draw takes the launcher theme from Settings.
            location_error,
            location_unavailable,
            location_note: location_page::startup_note(),
            location_target: None,
            args_input: line_edit::LineEdit::new(settings.game_args.clone(), 1024, 38),
            args_focused: false,
            args_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            proxy_input: line_edit::LineEdit::new(
                settings.proxy.clone().unwrap_or_default(),
                256,
                44,
            ),
            proxy_focused: false,
            corners_input: line_edit::LineEdit::new(settings.corner_radius().to_string(), 2, 3),
            corners_focused: false,
            corners_slider: crate::slider::Slider::default(),
            corners_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            cache_input: line_edit::LineEdit::new(
                crate::settings::cache_limit_text(settings.cache_limit_gib),
                5,
                6,
            ),
            cache_focused: false,
            cache_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            proxy_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            settings,
            settings_save_error: None,
            links: backend::Links::default(),
            preparing: false,
            prepare_log: VecDeque::new(),
            log_revision: 0,
            prepare_error: None,
            client: None,
            client_error: None,
            play_admission: None,
            play_install_recovery: None,
            pinning_build: false,
            play_registry_notices: Vec::new(),
            client_started: None,
            client_home: None,
            crash: None,
            servers: servers_page::Servers::new(),
            internet: internet::Internet::new(),
            root_focus: cx.focus_handle(),
            toast_seen: None,
            accounts: Accounts::load(),
            auth_status: HashMap::new(),
            auth_progress: HashMap::new(),
            profile_heads: profile_heads::Heads::default(),
            account_menu: account_chip::Menu::default(),
            search: command_search::Search::default(),
            auth_checked: None,
            sign_in_target: None,
            auth_busy: HashMap::new(),
            auth_next: 0,
            sign_in: None,
            sign_in_state: None,
            keychain: None,
            owner: None,
            owner_screen_open: false,
            starting_client: None,
            play_step: None,
            starting_play: None,
            play_when_prepared: None,
            friend_code: line_edit::LineEdit::new(String::new(), 256, 32),
            friend_focused: false,
            window_width: 0.,
            window_height: 0.,
            window_long_side: 0.,
            scale_factor: 1.,
            friend_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            joining: None,
            quick_play: None,
            client_run: None,
            client_vars: Vec::new(),
            collection: collection::Collection::load(),
            playing: None,
            tx,
            rx,
            _appearance: appearance,
            _activation: cx.observe_window_activation(window, |_, _, cx| cx.notify()),
            perf: perf::Perf::new("session"),
            install_started: None,
            prepare_started: None,
            download_seen: None,
            instance_mods: Default::default(),
            play_pressed: None,
            bench: perf::Bench::from_env(),
            starting_pack: None,
            mods: mods_page::Mods::new(cx),
            github: github_mods_page::Github::new(),
            instances: instances_page::Instances::default(),
            worlds: worlds_page::Worlds::default(),
            multiplayer: multiplayer_page::Multiplayer::default(),
            screenshots: screenshots_page::Screenshots::default(),
            pictures: Default::default(),
            game_packs: packs_page::Packs::default(),
            world_packs: datapacks_page::WorldPacks::default(),
            launch: None,
            aside: while_playing::Aside::new(window.window_handle()),
            side_servers: sunrise_launcher::servers::list(),
            scale: window.scale_factor(),
        };
        // A data folder that did not open, or a move that failed or finished, is dealt with in
        // Settings › Storage, where its message and the recoveries are.
        if let Some(section) =
            settings::startup_section(this.location_error.is_some(), this.location_note.is_some())
        {
            this.ui.page = frame::Page::Settings;
            this.ui.settings.section = section;
        }
        this.reload_instances();
        if this.bench.is_some() {
            this.perf.keep_local_log(false);
        }
        let tx = this.tx.clone();
        let keychain_tx = this.tx.clone();
        // Asking the keychain can block (D-Bus), so not on this thread.
        let accounts = this.accounts.clone();
        std::thread::spawn(move || {
            let store = account::store();
            let _ = keychain_tx.send(Msg::Keychain(account::no_keychain()));
            let _ = keychain_tx.send(Msg::Owner(accounts.has_owner(store)));
        });
        std::thread::spawn(move || {
            let reused = backend::reuse_prepared_everywhere();
            if !reused.is_empty() {
                let _ = tx.send(Msg::Reused(reused));
            }
        });
        this.load_profile_heads();
        this.load_auth_statuses(true);
        this.check_updates(UpdateCheck::Startup);
        this.fetch_links();
        telemetry::send_in_background();
        telemetry::watch_playing(this.tx.clone());
        this
    }

    /// Keeps the current choices in memory if writing fails; every page offers the same retry.
    pub(crate) fn save_settings(&mut self) -> bool {
        self.settings_save_error = self.settings.save().err();
        if self.settings_save_error.is_none() {
            self.mods.settings_saved();
        }
        self.settings_save_error.is_none()
    }

    pub(crate) fn settings_save_notice(&self, cx: &mut Context<Self>) -> Option<Div> {
        let error = self.settings_save_error.as_ref()?;
        let t = self.theme;
        Some(
            div()
                .flex_none()
                .px(px(theme::PAGE_GUTTER))
                .py(px(12.))
                .child(
                    crate::plate::plate(&t.night)
                        .flex()
                        .items_center()
                        .gap(px(16.))
                        .p(px(16.))
                        .child(
                            div()
                                .relative()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    shell::text(theme::night::UI, t.danger)
                                        .child(tr("Settings were not saved.")),
                                )
                                .child(
                                    shell::text(theme::night::CAPTION, t.ink_soft).child(crate::selectable::selectable(trf(
                                        "Your changes are still in this window. Retry to save them. {error}",
                                        &[("error", error)],
                                    ))),
                                ),
                        )
                        .child(
                            button("settings-save-retry", "Retry", Kind::Secondary, &t)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.save_settings();
                                    cx.notify();
                                })),
                        ),
                ),
        )
    }

    pub(crate) fn go(&mut self, page: Page) {
        // The CLI or another window may have changed the accounts: read them again (a small
        // file, once a switch).
        self.accounts = Accounts::load();
        // Each check reads the keychain (a prompt on macOS): page switches check every account
        // at most once a minute, and an account added meanwhile at once.
        if self
            .auth_checked
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(60))
            || (self.accounts.accounts.iter()).any(|a| !self.auth_status.contains_key(&a.uuid))
        {
            self.load_auth_statuses(false);
        }
        self.load_profile_heads();
        // A Turn off question left open isn't still asked the next time Settings opens.
        self.storage_revoke = None;
        // A Turn off error is read on the visit it shows in: leaving Settings settles it.
        if self.page == Page::Settings && page != Page::Settings {
            self.storage_revoke_error = None;
        }
        // Settings opens on its first page, its list folded and its search empty.
        if page == Page::Settings && self.page != Page::Settings {
            self.settings_nav_open = false;
            self.settings_pane.scroll_to_item(0);
            self.settings_query.set(String::new());
        }
        self.storage_counting = None;
        self.storage_visit += 1;
        // The glass look's Settings is a dialog over the page: opening or closing it keeps the
        // page under it as it was (its scroll, no fade).
        let dialog = self.theme.glass
            && (page == Page::Settings || (self.page == Page::Settings && page == self.behind));
        if page != Page::Settings {
            self.behind = page;
        }
        if page != self.page && dialog {
            self.page = page;
        } else if page != self.page {
            self.page = page;
            self.switches += 1;
            self.content_scroll = Scrollbar::default();
            self.long_list = cull::Cull::default();
            self.mods_sticky.reset();
            self.settings_sticky.reset();
            if page == Page::Mods {
                self.mods.reset_lists();
            }
        }
    }

    pub(crate) fn check_updates(&mut self, check: UpdateCheck) {
        if self.checking_updates {
            return;
        }
        self.checking_updates = true;
        if !matches!(check, UpdateCheck::ManualLauncher) {
            self.checked = Some(Instant::now());
        }
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let message = match check {
                UpdateCheck::Startup => {
                    let updates = backend::releases_and_updates(|releases| {
                        if let Ok(list) = &releases {
                            if let Err(error) = backend::note_play_catalogue(list) {
                                eprintln!(
                                    "Could not remember the current installed Nightly: {error}"
                                );
                            }
                        }
                        let _ = tx.send(Msg::Releases(releases));
                    });
                    Msg::Updates(Box::new(updates))
                }
                UpdateCheck::Scheduled => Msg::Updates(Box::new(backend::check_updates())),
                UpdateCheck::ManualLauncher => {
                    Msg::LauncherChecked(Box::new(backend::refresh_launcher_update()))
                }
            };
            let _ = tx.send(message);
        });
    }

    /// Recovery text for a launcher update that can't go in: why, or where the copy has to be.
    pub(crate) fn launcher_update_help(&self, error: Option<&str>) -> String {
        if let Some(error) = error {
            return trf("the update failed: {error}", &[("error", error)]);
        }
        if backend::package_managed_launcher() {
            return tr("Update this launcher with your package manager.").to_string();
        }
        let folder = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| std::path::PathBuf::from(tr("Downloads")));
        trf(
            "Close this launcher, move it to {folder}, and open it there to enable updates.",
            &[("folder", &folder.display().to_string())],
        )
    }

    pub(crate) fn launcher_download_available(&self) -> bool {
        !backend::package_managed_launcher()
    }

    pub(crate) fn update_launcher(&mut self, release: Release) {
        if !backend::self_update_allowed()
            || matches!(
                self.self_update,
                SelfUpdate::Downloading(..) | SelfUpdate::Ready(_) | SelfUpdate::Stuck(_)
            )
        {
            return;
        }
        if backend::cannot_replace_itself() {
            self.self_update = SelfUpdate::Stuck(None);
            return;
        }
        let version = backend::launcher_version(&release.tag)
            .unwrap_or_default()
            .to_string();
        self.self_update = SelfUpdate::Downloading(version, 0.);
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let progress = |p| {
                let _ = tx.send(Msg::LauncherProgress(p));
            };
            let _ = tx.send(Msg::LauncherUpdated(backend::stage_update(
                &release, progress,
            )));
        });
    }

    /// Update: the window closes, the installer puts the new launcher in place and starts it.
    /// When another launcher already put it there, this one only restarts.
    pub(crate) fn apply_launcher_update(&mut self, cx: &mut App) {
        match sunrise_launcher::update_handoff::start_now() {
            Ok(true) => {
                servers_page::keep_running_through_update();
                shut_down(cx)
            }
            Ok(false) => self.self_update = SelfUpdate::Failed(backend::restart()),
            Err(e) => self.self_update = SelfUpdate::Failed(e),
        }
    }

    /// Installs the Sunrise version the last check found, keeping the player's choice of
    /// version unless they were on the newest one.
    pub(crate) fn install_update(&mut self, release: Release) {
        if self.installing.is_some() || self.removing.contains(&release.tag) {
            return;
        }
        self.update_install = Some(release.tag.clone());
        self.install(&release);
    }

    /// What installed native build `version` was made from (`native_status::Build`), read once.
    pub(crate) fn native_build_of(&self, version: &str) -> Option<native_status::Build> {
        if !native_mods::is_native(version) {
            return None;
        }
        self.native_builds
            .borrow_mut()
            .entry(version.to_owned())
            .or_insert_with(|| native_status::Build::read(version))
            .clone()
    }

    pub(crate) fn fetch_links(&self) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Links(backend::links()));
        });
    }
}
