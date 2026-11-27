//! Settings › Appearance: the section's state and defaults, the look a change is saved
//! against, and the themes it lists.
use super::*;
use crate::paint::Palette;
use gpui::RenderImage;
use std::path::PathBuf;
use std::sync::Arc;
use sunrise_launcher::appearance::{self, Behind};
use sunrise_launcher::theme_files::{self, Entry};
use sunrise_launcher::themes::{self, Accent};

pub(super) struct Choice {
    /// What `select_theme` takes.
    pub(super) id: String,
    pub(super) name: SharedString,
    pub(super) about: SharedString,
    /// Its colours with the accent it would start with, for the preview.
    pub(super) palette: Palette,
}

type BehindKey = (Option<String>, Option<String>, bool);

pub(crate) struct State {
    /// The themes to choose from, made again when the folder is read or a choice changes.
    pub(super) choices: Option<Vec<Choice>>,
    pub(super) accent: line_edit::LineEdit,
    pub(super) accent_focus: Option<FocusHandle>,
    /// Whether the accent field had the keyboard when last drawn: leaving it takes what is typed.
    pub(super) accent_editing: bool,
    /// The keyboard's place on "Worlds on Home", and the search's request to be taken to it:
    /// whether that was done yet.
    pub(super) worlds_focus: Option<FocusHandle>,
    pub(super) worlds_asked: Option<std::rc::Rc<std::cell::Cell<bool>>>,
    /// What the last action could not do.
    pub(super) error: Option<String>,
    /// What Reload found.
    pub(super) note: Option<String>,
    /// What is behind the pages for the settings it was worked out from (theme, picture, scenery).
    pub(super) behind: Option<(BehindKey, Behind)>,
    /// The picture behind the pages: its file, and the picture once it is decoded (None: it could
    /// not be).
    pub(super) photo: Option<(PathBuf, Option<HomePhoto>)>,
    pub(super) decoding: bool,
    /// The kept copy of the player's own picture that could not be decoded: Home shows what
    /// comes after it, and Appearance says so.
    pub(super) unshown: Option<PathBuf>,
    /// Pictures no longer shown, for the window to let go of.
    pub(super) retired: Vec<Arc<RenderImage>>,
}

impl Default for State {
    fn default() -> Self {
        State {
            choices: None,
            accent: line_edit::LineEdit::new(String::new(), 32, 18),
            accent_focus: None,
            accent_editing: false,
            worlds_focus: None,
            worlds_asked: None,
            error: None,
            note: None,
            behind: None,
            photo: None,
            decoding: false,
            unshown: None,
            retired: Vec::new(),
        }
    }
}

impl State {
    /// The search opened "Worlds on Home": go to its control when the section is next drawn.
    pub(crate) fn ask_for_worlds(&mut self) {
        self.worlds_asked = Some(Default::default());
    }
}

pub(super) struct Look {
    pub(super) theme: Option<String>,
    pub(super) accent: Accent,
    pub(super) accents: std::collections::BTreeMap<String, Accent>,
    pub(super) background: Option<String>,
    pub(super) scenery: Option<bool>,
    pub(super) reduced_graphics: Option<bool>,
}

impl Launcher {
    pub(super) fn look(&self) -> Look {
        Look {
            theme: self.settings.theme.clone(),
            accent: self.settings.accent,
            accents: self.settings.theme_accents.clone(),
            background: self.settings.background.clone(),
            scenery: self.settings.scenery_on,
            reduced_graphics: self.settings.reduced_graphics,
        }
    }

    /// The themes to choose from: Dawn first, Sunrise's own violet, the other built-in ones,
    /// then the player's.
    pub(super) fn appearance_choices(&mut self) {
        if self.ui.settings.appearance.choices.is_some() {
            return;
        }
        let in_use = appearance::effective_id(&self.settings);
        let choice = |id: String, name: SharedString, about: SharedString, night| {
            // Each preview wears its theme's own accent; the one in use, the accent in use.
            let accent = if id == in_use {
                appearance::accent(&self.settings).rgb()
            } else {
                themes::own_accent(Some(&id)).rgb()
            };
            Choice {
                palette: Palette::plain(night, accent),
                id,
                name,
                about,
            }
        };
        let builtin = |t: &themes::Theme| {
            choice(
                t.id.into(),
                tr(t.name).into(),
                tr(t.description).into(),
                t.palette.copied().unwrap_or(theme::night::NIGHT),
            )
        };
        let mut all = vec![
            builtin(&themes::DAWN_THEME),
            choice(
                themes::TWILIGHT.into(),
                tr(themes::TWILIGHT_NAME).into(),
                tr(themes::TWILIGHT_DESCRIPTION).into(),
                theme::night::NIGHT,
            ),
        ];
        all.extend(
            themes::THEMES
                .iter()
                .filter(|t| t.id != themes::DAWN_THEME.id)
                .map(builtin),
        );
        for entry in theme_files::loaded().iter() {
            if let Entry::Good(local) = entry {
                all.push(choice(
                    local.id(),
                    local.name.clone().into(),
                    trf("Your theme · {folder}", &[("folder", &local.slug)]).into(),
                    local.roles.night(),
                ));
            }
        }
        self.ui.settings.appearance.choices = Some(all);
    }
}
