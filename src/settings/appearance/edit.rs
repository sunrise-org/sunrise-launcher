//! Settings › Appearance: the life of the Edit theme dialog. It opens on a saved theme (one the
//! player picked, or a copy made for it by "+ New theme" or a built-in's Customize), paints
//! the colours as they change, and ends with Done, which keeps them, or with anything else,
//! which puts the theme and the look back as they were.
use super::state::{Dialog, Edit};
use super::*;
use sunrise_launcher::appearance;
use sunrise_launcher::theme_files::{self, Reason, Roles};
use sunrise_launcher::themes::library;

impl Launcher {
    /// Opens the Edit dialog on the saved theme in the folder `slug`.
    pub(super) fn theme_edit(&mut self, slug: &str, window: &Window) {
        self.edit_start(slug, false, window);
    }

    /// "+ New theme": a copy of the theme now painted, opened to edit.
    pub(super) fn theme_new(&mut self, window: &Window) {
        let (name, roles) = (self.current_name(), self.editor_roles());
        self.theme_copy(&name, &roles, window);
    }

    /// Makes a new saved theme, "{source} copy" in the colours `roles`, chooses it and opens it
    /// in the Edit dialog. Cancel deletes it again.
    pub(super) fn theme_copy(&mut self, source: &str, roles: &Roles, window: &Window) {
        let made = library::make_copy(&theme_files::dir(), source, roles);
        theme_files::reload();
        match made {
            Ok((slug, _)) => self.edit_start(&slug, true, window),
            Err(why) => self.ui.settings.appearance.error = Some(why),
        }
    }

    /// Chooses the saved theme `slug`, so that what is edited is painted, and opens the dialog
    /// on it. `created` says it was made for this dialog.
    fn edit_start(&mut self, slug: &str, created: bool, window: &Window) {
        let root = theme_files::dir();
        let found = library::listed(&theme_files::loaded())
            .into_iter()
            .find(|local| local.slug == slug);
        let Some(local) = found else {
            self.ui.settings.appearance.error = Some(Reason::NoFile.text());
            return;
        };
        let before = self.look();
        let id = local.id();
        self.appearance_change(
            |s| {
                s.theme_accents.remove(&id);
                appearance::select_theme(s, &id)
            },
            window,
        );
        if self.ui.settings.appearance.error.is_some() {
            if created {
                let _ = library::remove(&root, slug);
                theme_files::reload();
            }
            return;
        }
        self.ui.settings.appearance.edit = Some(Edit {
            slug: local.slug,
            start: library::Custom {
                name: local.name,
                roles: local.roles,
            },
            created,
            before,
        });
        self.dialog_open(Dialog::Edit);
    }

    /// Done: what was typed in the picker is taken, and the name typed is kept. The colours
    /// were written as they were painted.
    pub(super) fn edit_done(&mut self, window: &Window) {
        self.colour_commit(window);
        let state = &self.ui.settings.appearance;
        let Some(edit) = state.edit.as_ref() else {
            return;
        };
        let name = state.name.text().trim().to_string();
        if name.is_empty() {
            return;
        }
        let root = theme_files::dir();
        let renamed = if name == edit.start.name {
            Ok(())
        } else {
            library::rename(&root, &edit.slug, &name)
        };
        theme_files::reload();
        match renamed {
            Ok(()) => {
                self.appearance_change(|_| Ok(()), window);
                let state = &mut self.ui.settings.appearance;
                state.edit = None;
                state.dialog = None;
                self.close_sheet();
            }
            Err(why) => self.ui.settings.appearance.dialog_error = Some(why),
        }
    }

    /// A theme dialog was closed, by Done, Cancel, Escape or a press outside it. What the Edit
    /// dialog was working on is kept for [`Self::edit_revert`], which has the window.
    pub(crate) fn theme_dialog_closed(&mut self) {
        let state = &mut self.ui.settings.appearance;
        state.reverting = state.edit.take();
        state.dialog = None;
        state.editing = None;
        state.dragging = None;
    }

    /// Puts back what an Edit dialog closed without Done had changed: a theme made for it is
    /// deleted, another has its name and colours back, and the look is the one from before.
    /// True when it did (the window is drawn again).
    pub(super) fn edit_revert(&mut self, window: &Window) -> bool {
        let state = &mut self.ui.settings.appearance;
        let Some(edit) = state.reverting.take() else {
            return false;
        };
        state.unsaved = false;
        let root = theme_files::dir();
        let undone = if edit.created {
            library::remove(&root, &edit.slug)
        } else {
            library::restore(&root, &edit.slug, &edit.start)
        };
        theme_files::reload();
        let look = edit.before;
        self.appearance_change(
            |s| {
                look.restore(s);
                Ok(())
            },
            window,
        );
        if let Err(why) = undone {
            self.ui.settings.appearance.error = Some(why);
        }
        true
    }
}
