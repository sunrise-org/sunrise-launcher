//! The blocky look's Settings page, the native build consent and the settings rows.
use super::*;

impl Launcher {
    /// Cancel and Escape share the transition; neither can execute a pending action.
    pub(crate) fn cancel_native_consent(&mut self) {
        self.play_after_build = None;
        self.native_consent_build = None;
        self.pending_consent = None;
        self.consent_error = None;
        self.instance_mods_changed();
    }

    /// Keeps the Proxy field in `settings.json`, and tells the parts that build their own agents
    /// (`sunrise-port-pack`) which proxy to use.
    pub(crate) fn save_proxy(&mut self) {
        let value = self.proxy_input.text().trim().to_owned();
        self.settings.proxy = (!value.is_empty()).then_some(value);
        self.save_settings();
        sunrise_launcher::proxy::announce(self.settings.proxy.clone());
    }
}
