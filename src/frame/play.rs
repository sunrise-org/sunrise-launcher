//! Play: the one entry point every Play button, shortcut and destination goes through, and its journal check.

use super::*;

impl Launcher {
    /// Also called for world and server destinations. Never send a destination through the
    /// legacy shortcuts, which skip some of the checks on the main Play action.
    pub(crate) fn play_game(&mut self, destination: launcher_play::PlayDestination) {
        if self.client.is_some() || (self.launch_busy() && !self.registry_accepts_play()) {
            return;
        }
        self.prepare_error = None;
        self.client_error = None;
        self.play_registry_notices.clear();
        self.crash = None;
        self.install_error = None;
        let Some(instance) = self.selected_instance() else {
            return;
        };
        let kind = instance.kind;
        let intent =
            launcher_play::PlayIntent::new(instance, self.accounts.selected(), destination);
        if self.refuse_pack_play(kind) {
            return;
        }
        if !self.account_ready() {
            self.sign_in_to_play();
            self.ui.after_sign_in = Some(intent);
            return;
        }
        self.resume_play(intent);
    }

    /// Whether Play has an account to play with: one is chosen and its sign-in still holds (the
    /// chip says "Signed out" otherwise), or the developer's offline Play is on.
    pub(crate) fn account_ready(&self) -> bool {
        if sunrise_launcher::developer_offline::selected() {
            return true;
        }
        self.accounts.selected().is_some_and(|account| {
            !self.auth_status.get(&account.uuid).is_some_and(|status| {
                status.state == sunrise_launcher::account::session::State::SignInRequired
            })
        })
    }

    /// Continue the same accepted Play after a launch step; never recapture the account.
    pub(crate) fn resume_play(&mut self, intent: launcher_play::PlayIntent) {
        let Some(instance) = self
            .selected_instance()
            .filter(|i| intent.matches_profile(i))
            .cloned()
        else {
            return;
        };
        if self.refuse_pack_play(instance.kind) {
            return;
        }
        if self.registry_accepts_play() {
            self.defer_registry_play(&instance, intent);
            return;
        }
        if self.client.is_some() || self.launch_busy() {
            return;
        }
        self.start_managed_client(instance, intent, None);
    }

    fn refuse_pack_play(&mut self, kind: instances::Kind) -> bool {
        if kind != instances::Kind::Pack {
            return false;
        }
        self.client_error = Some(sunrise_launcher::browser::sunrise::pack::play_unavailable());
        true
    }

    pub(crate) fn play_after_journal_check(
        &mut self,
        intent: launcher_play::PlayIntent,
        home: &std::path::Path,
    ) {
        if self.client.is_some() || self.launch_busy() {
            return;
        }
        let Some(instance) = self
            .selected_instance()
            .filter(|i| intent.matches_profile(i) && i.home == home)
            .cloned()
        else {
            return;
        };
        let target = self.profile_target(&instance);
        let Some(tag) = target
            .as_ref()
            .filter(|tag| self.installed.contains(tag))
            .cloned()
        else {
            let release = target
                .as_deref()
                .and_then(|tag| self.release_to_install(tag));
            if let Some(release) = release {
                self.ui.after_install = Some((intent, release.tag.clone()));
                self.install(&release);
            } else {
                self.client_error = Some(match &self.releases {
                    Load::Loading => {
                        tr("Checking available builds. Press Play when the list is ready.").into()
                    }
                    Load::Failed(e) => e.clone(),
                    Load::Ready(_) => {
                        tr("No downloadable build matches this profile. Choose a game build.")
                            .into()
                    }
                });
                if matches!(self.releases, Load::Failed(_)) {
                    self.check_updates(UpdateCheck::Startup);
                }
            }
            return;
        };
        if !backend::check_prepared(&tag) {
            self.play_when_prepared = Some((tag, intent));
            self.prepare();
            return;
        }
        self.native_error = None;
        if matches!(
            self.native_state_of(&instance),
            native_status::State::NotBuilt | native_status::State::Outdated
        ) {
            self.play_after_build = Some(intent);
            self.build_native();
        } else {
            self.play(intent);
        }
    }

    pub(crate) fn launch_busy(&self) -> bool {
        self.launch_activity().busy()
    }
}
