//! The Mods tab's build card and the per-mod route chips (docs/modding/src/how-to/native-mods.md).
//! The card picks the build mode, shows where a build is (a percentage, one pill per stage) and
//! cancels it. A chip on each native mod says how it reaches the game; its menu gives the reason
//! and the actions.
use super::*;

use native_mods::Profile;
use native_status::State;

impl Launcher {
    /// A build stopped for worlds at stake: the choice is shown, and the mods that failed to
    /// compile stay marked past it.
    pub(super) fn keep_stop(&mut self, stake: &native_mods::AtStake) {
        self.native_worlds = stake.incompatible.then(|| stake.clone());
        self.mods.unbuilt = self
            .native_for
            .clone()
            .map(|instance| (instance, stake.unbuilt.clone()));
    }

    /// The mods the selected instance's native build left out, as the build recorded them.
    pub(super) fn native_left_out(&self) -> Vec<native_mods::LeftOut> {
        let Some(instance) = self.selected_instance() else {
            return Vec::new();
        };
        let Some(version) = self.version_of(instance) else {
            return Vec::new();
        };
        match self.native_build_of(version) {
            Some(build) => build.left_out,
            // A game that needed no build because every mod with native code was left out.
            None => native_mods::kept_left_out(&instance.home, version),
        }
    }

    /// Where the instance's native mods stand, read again only when what it depends on changes.
    pub(super) fn native_state_of(&self, instance: &instances::Instance) -> State {
        let version = self.version_of(instance).cloned();
        let home = instance.home.clone();
        let profile = Profile::Optimized;
        let key = format!(
            "{}{profile:?}",
            native_status::fingerprint(&home, version.as_deref()) + &native_mods::kept_stamp(&home)
        );
        let mut memo = self.mods.native_state.borrow_mut();
        match memo.as_ref() {
            Some((k, state, _)) if *k == key => *state,
            _ => {
                let built = version.as_deref().and_then(native_status::built);
                let enabled = native_status::enabled(&home);
                // No build, because every mod with native code was left out of the game as it
                // is: for these mods that plain game is what a build gives, so nothing waits.
                let plain = built.is_none()
                    && !enabled.is_empty()
                    && version
                        .as_deref()
                        .is_some_and(|v| native_mods::kept_current(&home, v));
                let state = if plain {
                    native_status::State::UpToDate
                } else {
                    native_status::in_mode(
                        native_status::state(&enabled, built.as_deref()),
                        version.as_deref().and_then(native_status::built_profile),
                        profile,
                    )
                };
                // A build in another mode rebuilds every mod; otherwise only the changed ones.
                let unbuilt = match state {
                    _ if plain => Vec::new(),
                    native_status::State::NotBuilt => enabled.into_iter().map(|m| m.0).collect(),
                    _ => native_status::unbuilt(&enabled, built.as_deref()),
                };
                *memo = Some((key, state, unbuilt));
                state
            }
        }
    }

    /// Whether native mod `id` of the selected instance waits for a build ([`Self::native_state_of`]).
    pub(super) fn native_mod_unbuilt(&self, id: &str) -> bool {
        if let Some(instance) = self.selected_instance() {
            self.native_state_of(instance);
        }
        self.mods
            .native_state
            .borrow()
            .as_ref()
            .is_some_and(|(_, _, unbuilt)| unbuilt.iter().any(|u| u == id))
    }
}
