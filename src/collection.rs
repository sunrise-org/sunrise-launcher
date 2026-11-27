//! Data collection in the window: the question asked once (and again when what is collected
//! changes), and its section in Settings. Everything goes through `sunrise_launcher::telemetry`.
use super::*;
use sunrise_launcher::telemetry;
use sunrise_launcher_telemetry::{Choices, Consent};

/// What the window keeps about data collection.
pub(crate) struct Collection {
    pub consent: Consent,
    /// "View what would be sent": a line on what it is, and the JSON.
    pub view: Option<(String, String)>,
    /// What "Delete my data" did, or why it could not.
    pub note: Option<String>,
    pub confirm_delete: bool,
    /// The switches (by name) whose "What's sent?" is open, on the question and in Settings.
    pub(crate) sent_open: Vec<&'static str>,
    /// The switch whose "?" the pointer is over, in the new look's Settings.
    pub(crate) sent_hover: Option<&'static str>,
}

impl Collection {
    pub fn load() -> Self {
        Self {
            consent: telemetry::consent(),
            view: None,
            note: None,
            confirm_delete: false,
            sent_open: Vec::new(),
            sent_hover: None,
        }
    }
}

impl Launcher {
    pub(crate) fn answer_consent(&mut self, choices: Choices) {
        match telemetry::answer(choices) {
            Ok(consent) => self.collection.consent = consent,
            Err(e) => {
                self.collection.note =
                    Some(trf("Could not save your choice: {error}", &[("error", &e)]))
            }
        }
        // Settings says what the "I'm playing" ping holds, beside the switches.
        if !self.settings.ping_notice_seen {
            self.settings.ping_notice_seen = true;
            self.save_settings();
        }
        // What would be sent may have changed with the answer: an open view shows it afresh and
        // stays open until Hide, or until the player leaves the page.
        if self.collection.view.is_some() {
            self.collection.view = Some(telemetry::preview());
        }
    }

    pub(crate) fn toggle_view(&mut self) {
        self.collection.view = match self.collection.view {
            Some(_) => None,
            None => Some(telemetry::preview()),
        };
    }
}
