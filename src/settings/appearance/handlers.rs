//! Settings › Appearance: the actions a change takes, saving it and reading the themes
//! folder again.
use super::state::Look;
use super::*;
use sunrise_launcher::theme_files::{self, Entry};

impl Launcher {
    /// Makes one change to the look and saves it. A change that is refused or can't be saved
    /// leaves the settings as they were and is said under the row.
    pub(super) fn appearance_change(
        &mut self,
        change: impl FnOnce(&mut backend::Settings) -> Result<(), String>,
        window: &Window,
    ) {
        let before = self.look();
        let done = change(&mut self.settings).and_then(|()| self.settings.save());
        let state = &mut self.ui.settings.appearance;
        state.note = None;
        state.error = done.err();
        if state.error.is_some() {
            let Look {
                theme,
                accent,
                accents,
                background,
                scenery,
                reduced_graphics,
            } = before;
            self.settings.theme = theme;
            self.settings.accent = accent;
            self.settings.theme_accents = accents;
            self.settings.background = background;
            self.settings.scenery_on = scenery;
            self.settings.reduced_graphics = reduced_graphics;
            return;
        }
        self.appearance_changed(window);
    }

    /// The look changed, by a choice or by Reload: what was worked out from the old one is
    /// worked out again.
    pub(super) fn appearance_changed(&mut self, window: &Window) {
        self.theme = self.settings_theme(window);
        self.ui.settings.appearance.behind = None;
    }

    /// Reads the themes folder again: a repaired theme is back, an edited one is as edited.
    pub(super) fn appearance_reload(&mut self, window: &Window) {
        let entries = theme_files::reload();
        let state = &mut self.ui.settings.appearance;
        // The same file may hold another picture now.
        if let Some((_, Some(old))) = state.photo.take() {
            state.retired.push(old.image);
        }
        let bad = entries
            .iter()
            .filter(|e| matches!(e, Entry::Bad { .. }))
            .count();
        state.error = None;
        state.note = Some(match (entries.len() - bad, bad) {
            (0, 0) => tr("No themes in the folder yet.").to_string(),
            (good, 0) => trf(
                "Read {count} of your themes.",
                &[("count", &good.to_string())],
            ),
            (good, bad) => trf(
                "Read {count} of your themes; {bad} can't be used.",
                &[("count", &good.to_string()), ("bad", &bad.to_string())],
            ),
        });
        self.appearance_changed(window);
    }
}
