//! Settings › Appearance: the section's state and defaults, and the look a change is saved
//! against.
use super::*;
use gpui::RenderImage;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use sunrise_launcher::appearance::Behind;
use sunrise_launcher::theme_files::Roles;
use sunrise_launcher::themes::Accent;

type BehindKey = (Option<String>, Option<String>, bool);

/// What the theme library's one open panel is doing.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Panel {
    /// Naming the colours being edited, to keep them.
    Save,
    /// Giving the theme in this folder another name.
    Rename(String),
    /// Asking before the themes in these folders are deleted.
    Delete(Vec<String>),
    /// Showing the code for this many themes.
    Export(String, usize),
    /// Taking a pasted code.
    Import,
}

pub(crate) struct State {
    /// The hex of the colour being typed, and which colour that is (an index into
    /// `themes::library::SLOTS`).
    pub(super) hex: line_edit::LineEdit,
    pub(super) hex_focus: Option<FocusHandle>,
    pub(super) editing: Option<usize>,
    /// Whether the hex field had the keyboard when last drawn: leaving it takes what is typed.
    pub(super) hex_was_focused: bool,
    /// Colours typed that can't be read on each other yet: shown, not painted, until they can.
    pub(super) draft: Option<Roles>,
    /// The name being typed for Save and Rename, and the code pasted for Import.
    pub(super) name: line_edit::LineEdit,
    pub(super) name_focus: Option<FocusHandle>,
    pub(super) code: line_edit::LineEdit,
    pub(super) code_focus: Option<FocusHandle>,
    pub(super) panel: Option<Panel>,
    /// The library's themes ticked for export, by folder.
    pub(super) ticked: BTreeSet<String>,
    /// The folder whose "…" menu is open, and whether Home header's menu is.
    pub(super) more: Option<String>,
    pub(super) header_open: bool,
    /// The keyboard's place on "Worlds on Home", and the search's request to be taken to it:
    /// whether that was done yet.
    pub(super) worlds_focus: Option<FocusHandle>,
    pub(super) worlds_asked: Option<std::rc::Rc<std::cell::Cell<bool>>>,
    /// What the last action could not do.
    pub(super) error: Option<String>,
    /// What the last action did, or why a colour isn't painted yet.
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
            hex: line_edit::LineEdit::new(String::new(), 8, 9),
            hex_focus: None,
            editing: None,
            hex_was_focused: false,
            draft: None,
            name: line_edit::LineEdit::new(String::new(), 64, 40),
            name_focus: None,
            // A code for a few themes is a few hundred characters.
            code: line_edit::LineEdit::new(String::new(), 40_000, 60),
            code_focus: None,
            panel: None,
            ticked: BTreeSet::new(),
            more: None,
            header_open: false,
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
}
