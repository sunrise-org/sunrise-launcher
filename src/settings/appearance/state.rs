//! Settings › Appearance: the section's state and defaults, and the look a change is saved
//! against.
use super::*;
use crate::widgets::colour_picker::{Areas, Part};
use gpui::RenderImage;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use sunrise_launcher::appearance::Behind;
use sunrise_launcher::themes::hsv::Hsv;
use sunrise_launcher::themes::library;
use sunrise_launcher::themes::Accent;

type BehindKey = (Option<String>, Option<String>, bool);

/// What the theme library's one open dialog (a sheet over the window) is for.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Dialog {
    /// Naming the colours being edited, to keep them.
    Save,
    /// Giving the theme in this folder another name.
    Rename(String),
    /// Taking a pasted code.
    Import,
    /// Showing the code for these themes.
    Export {
        code: String,
        themes: Vec<library::Custom>,
    },
}

pub(crate) struct State {
    /// The hex of the colour being picked, and which colour that is (an index into
    /// `themes::library::SLOTS`): the colour whose picker is open.
    pub(super) hex: line_edit::LineEdit,
    pub(super) hex_focus: Option<FocusHandle>,
    pub(super) editing: Option<usize>,
    /// The open picker: its colour as hue, saturation and value (a grey keeps its hue), the
    /// colour it was when it opened, where its square and bar were drawn, the part being
    /// dragged and where the keyboard is while it is open.
    pub(super) hsv: Hsv,
    pub(super) before: u32,
    pub(super) areas: Areas,
    pub(super) dragging: Option<Part>,
    /// A drag painted colours not written yet: they are written when it ends.
    pub(super) unsaved: bool,
    pub(super) picker_focus: Option<FocusHandle>,
    /// Whether the hex field had the keyboard when last drawn: leaving it takes what is typed.
    pub(super) hex_was_focused: bool,
    /// The name being typed for Save and Rename, and the code pasted for Import.
    pub(super) name: line_edit::LineEdit,
    pub(super) name_focus: Option<FocusHandle>,
    pub(super) code: line_edit::LineEdit,
    pub(super) code_focus: Option<FocusHandle>,
    pub(super) dialog: Option<Dialog>,
    /// Why the dialog's button failed, said in the dialog.
    pub(super) dialog_error: Option<String>,
    /// The dialog just opened: its text box takes the keyboard when it is first drawn.
    pub(super) dialog_focus: bool,
    /// The library's themes ticked for export, by folder.
    pub(super) ticked: BTreeSet<String>,
    /// Whether the colour editor is open under the current theme's bar, and when that last
    /// changed (for the open and close glide).
    pub(super) colours_open: bool,
    pub(super) colours_since: Option<std::time::Instant>,
    /// The built-in theme the colours being edited started from: what Reset goes back to.
    pub(super) origin: Option<&'static str>,
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
            hsv: Hsv {
                h: 0.,
                s: 0.,
                v: 0.,
            },
            before: 0,
            areas: Areas::default(),
            dragging: None,
            unsaved: false,
            picker_focus: None,
            hex_was_focused: false,
            name: line_edit::LineEdit::new(String::new(), 64, 40),
            name_focus: None,
            // A code for a few themes is a few hundred characters.
            code: line_edit::LineEdit::new(String::new(), 40_000, 60),
            code_focus: None,
            dialog: None,
            dialog_error: None,
            dialog_focus: false,
            ticked: BTreeSet::new(),
            colours_open: false,
            colours_since: None,
            origin: None,
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
