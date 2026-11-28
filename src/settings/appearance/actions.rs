//! Settings › Appearance: what the colour picker, the dialogs and the theme gallery do. Each action works
//! on the settings through [`Launcher::appearance_change`], so a failure puts everything back
//! and is said under the page, and a success is painted at once.
use super::state::Dialog;
use super::*;
use crate::sheets::Sheet;
use crate::widgets::colour_picker::Part;
use gpui::{MouseButton, MouseMoveEvent, Pixels, Point};
use sunrise_launcher::theme_files::{self, Reason, Roles};
use sunrise_launcher::themes::hsv::Hsv;
use sunrise_launcher::themes::library::{self, SLOTS};
use sunrise_launcher::themes::share;

impl Launcher {
    /// The colours the editor shows: what is painted.
    pub(super) fn editor_roles(&self) -> Roles {
        library::current(&self.settings)
    }

    /// Sets one colour of the theme being edited and paints it, however it reads on the others.
    pub(super) fn colour_set(&mut self, slot: usize, rgb: u32, window: &Window) {
        let mut roles = self.editor_roles();
        (SLOTS[slot].set)(&mut roles, rgb);
        self.edit_paint(&roles, window);
    }

    /// Paints `roles` as the theme being edited (nothing when no theme is). While a colour is
    /// dragged they are painted from memory: writing the theme and the settings at every step
    /// falls behind the pointer. The drag's end writes them (`picker_release`).
    fn edit_paint(&mut self, roles: &Roles, window: &Window) {
        let state = &self.ui.settings.appearance;
        let Some(slug) = state.edit.as_ref().map(|edit| edit.slug.clone()) else {
            return;
        };
        if state.dragging.is_some() {
            match library::preview(&mut self.settings, &slug, roles) {
                Ok(()) => {
                    self.ui.settings.appearance.unsaved = true;
                    self.appearance_changed(window);
                }
                Err(error) => self.ui.settings.appearance.error = Some(error),
            }
        } else {
            self.appearance_change(|s| library::paint(s, &slug, roles), window);
        }
    }

    /// Takes what is typed in the hex field, if a colour's picker is open. False when it isn't
    /// a colour. The picker stays open.
    pub(super) fn hex_apply(&mut self, window: &Window) -> bool {
        let state = &self.ui.settings.appearance;
        let Some(slot) = state.editing else {
            return true;
        };
        let Some(rgb) = library::parse_hex(state.hex.text()) else {
            return false;
        };
        if rgb != (SLOTS[slot].get)(&self.editor_roles()) {
            self.colour_set(slot, rgb, window);
        }
        true
    }

    /// Takes what is typed in the hex field and closes the picker. What the picker moved is
    /// already painted.
    pub(super) fn colour_commit(&mut self, window: &Window) {
        let Some(slot) = self.ui.settings.appearance.editing else {
            return;
        };
        self.picker_release(window);
        let typed = self.hex_apply(window);
        let state = &mut self.ui.settings.appearance;
        if !typed {
            let label = tr(SLOTS[slot].label).to_string();
            state.error = Some(Reason::Colour(label).text());
        }
        state.editing = None;
        state.dragging = None;
    }

    /// Opens the picker of the colour in `slot`, taking what was typed in another first, with
    /// its hex field ready.
    pub(super) fn colour_start(
        &mut self,
        slot: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.colour_commit(window);
        let rgb = (SLOTS[slot].get)(&self.editor_roles());
        let state = &mut self.ui.settings.appearance;
        state.more = None;
        state.header_open = false;
        state.editing = Some(slot);
        state.hsv = Hsv::from_rgb(rgb);
        state.before = rgb;
        state.dragging = None;
        state.hex.set(format!("#{rgb:06x}"));
        state.hex.select_all();
        let focus = state
            .hex_focus
            .get_or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone();
        window.focus(&focus, cx);
    }

    /// A press on the picker's square or bar: the colour goes to where it is, and moves with
    /// the pointer until the button is let go.
    pub(super) fn picker_press(&mut self, part: Part, at: Point<Pixels>, window: &Window) {
        self.ui.settings.appearance.dragging = Some(part);
        self.picker_to(at, window);
    }

    /// The pointer moved with the picker open: it drags what was pressed, while the left
    /// button is down.
    pub(super) fn picker_drag(&mut self, ev: &MouseMoveEvent, window: &Window) {
        if ev.pressed_button == Some(MouseButton::Left) {
            self.picker_to(ev.position, window);
        } else {
            self.picker_release(window);
        }
    }

    /// The drag ended: what it painted is written, theme and settings, once.
    pub(super) fn picker_release(&mut self, window: &Window) {
        let state = &mut self.ui.settings.appearance;
        state.dragging = None;
        if std::mem::take(&mut state.unsaved) {
            let roles = library::current(&self.settings);
            self.edit_paint_written(&roles, window);
        }
    }

    /// Writes `roles`, painted from memory by a drag, as the theme being edited.
    fn edit_paint_written(&mut self, roles: &Roles, window: &Window) {
        let slug = self
            .ui
            .settings
            .appearance
            .edit
            .as_ref()
            .map(|edit| edit.slug.clone());
        if let Some(slug) = slug {
            self.appearance_change(|s| library::paint(s, &slug, roles), window);
        }
    }

    /// Sets the colour being dragged to the one under `at`, as a typed hex is set: painted and
    /// saved, or kept unpainted until it can be read.
    fn picker_to(&mut self, at: Point<Pixels>, window: &Window) {
        let state = &self.ui.settings.appearance;
        let (Some(slot), Some(part)) = (state.editing, state.dragging) else {
            return;
        };
        let next = state.areas.pick(part, state.hsv, at);
        let rgb = next.to_rgb();
        let changed = rgb != (SLOTS[slot].get)(&self.editor_roles());
        let state = &mut self.ui.settings.appearance;
        state.hsv = next;
        if changed {
            state.hex.set(format!("#{rgb:06x}"));
            self.colour_set(slot, rgb, window);
        }
    }

    /// Enter in the hex field: the colour is taken and the keyboard goes to the picker, which
    /// stays open.
    pub(super) fn colour_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.hex_apply(window) {
            let slot = self.ui.settings.appearance.editing.unwrap_or(0);
            let label = tr(SLOTS[slot].label).to_string();
            self.ui.settings.appearance.error = Some(Reason::Colour(label).text());
        }
        if let Some(focus) = self.ui.settings.appearance.picker_focus.clone() {
            window.focus(&focus, cx);
        }
    }

    /// Escape in a text box: nothing typed is taken, and an open picker or dialog closes. In
    /// the Edit dialog the picker closes first, and the dialog on the next Escape.
    pub(super) fn text_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = &mut self.ui.settings.appearance;
        let picker = state.editing.take().is_some();
        state.dragging = None;
        let editing = state.edit.is_some();
        if self.ui.sheet == Some(Sheet::ThemeDialog) && !(picker && editing) {
            self.close_sheet();
        }
        window.blur(cx);
    }

    /// The code for the colours now painted, in a dialog to copy it from.
    pub(super) fn share_current(&mut self) {
        let theme = library::Custom {
            name: self.current_name(),
            roles: self.editor_roles(),
        };
        let code = share::encode(std::slice::from_ref(&theme));
        self.dialog_open(Dialog::Export {
            code,
            themes: vec![theme],
        });
    }

    /// Opens the library's dialog for `dialog`, a sheet over the window. Its text box gets the
    /// keyboard when the sheet is first drawn.
    pub(super) fn dialog_open(&mut self, dialog: Dialog) {
        self.open_sheet(Sheet::ThemeDialog);
        let state = &mut self.ui.settings.appearance;
        if dialog != Dialog::Edit && state.edit.is_some() {
            // A dialog left open by a path that doesn't close the sheet: put its theme back.
            state.reverting = state.edit.take();
        }
        state.error = None;
        state.note = None;
        state.more = None;
        state.editing = None;
        state.dragging = None;
        match &dialog {
            Dialog::Edit => {
                let named = state.edit.as_ref().map(|edit| edit.start.name.clone());
                state.name.set(named.unwrap_or_default());
                state.name.select_all();
            }
            Dialog::Import => state.code.set(String::new()),
            Dialog::Export { .. } => {}
        }
        state.dialog = Some(dialog);
        state.dialog_error = None;
        state.dialog_focus = true;
    }

    /// The code for the themes in the folders `slugs`, in a dialog to copy it from.
    pub(super) fn export_open(&mut self, slugs: &[String]) {
        let themes = library::take(&theme_files::dir(), slugs);
        if themes.is_empty() {
            return;
        }
        let code = share::encode(&themes);
        self.dialog_open(Dialog::Export { code, themes });
    }

    /// What the open dialog's main button does: keeps the edited theme or imports. On success
    /// the sheet closes; otherwise it stays, with why.
    pub(super) fn dialog_confirm(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        let state = &self.ui.settings.appearance;
        let Some(dialog) = state.dialog.clone() else {
            return;
        };
        let root = theme_files::dir();
        let said = match &dialog {
            Dialog::Edit => return self.edit_done(window),
            Dialog::Import => share::decode(state.code.text())
                .map_err(|e| e.text())
                .and_then(|themes| library::add_all(&root, &themes))
                .map(|n| {
                    trf(
                        "Added {count} to your themes.",
                        &[("count", &n.to_string())],
                    )
                }),
            Dialog::Export { .. } => Ok(String::new()),
        };
        theme_files::reload();
        match said {
            Ok(note) => {
                self.appearance_change(|_| Ok(()), window);
                let state = &mut self.ui.settings.appearance;
                state.note = Some(note).filter(|note| !note.is_empty());
                state.dialog = None;
                state.ticked.clear();
                self.close_sheet();
            }
            Err(why) => self.ui.settings.appearance.dialog_error = Some(why),
        }
    }

    /// Deletes the themes in the folders `slugs` at once: their folders go to the system's
    /// trash, where they can be got back, and a toast says so.
    pub(super) fn theme_delete(&mut self, slugs: &[String], window: &Window) {
        let root = theme_files::dir();
        let names: Vec<String> = library::listed(&theme_files::loaded())
            .into_iter()
            .filter(|local| slugs.contains(&local.slug))
            .map(|local| local.name)
            .collect();
        let done = slugs
            .iter()
            .try_for_each(|slug| library::trash(&root, slug));
        theme_files::reload();
        self.appearance_change(|_| done, window);
        let state = &mut self.ui.settings.appearance;
        if state.error.is_some() {
            return;
        }
        state.ticked.retain(|slug| !slugs.contains(slug));
        crate::toast::say(match names.as_slice() {
            [one] => trf("Deleted {name}", &[("name", one.as_str())]),
            _ => trf(
                "Deleted {count} themes",
                &[("count", &slugs.len().to_string())],
            ),
        });
    }

    /// One of a saved tile's "…" menu: Edit…, Duplicate, Share…, Delete.
    pub(super) fn theme_menu_pick(&mut self, slug: String, pick: usize, window: &Window) {
        self.ui.settings.appearance.more = None;
        match pick {
            0 => self.theme_edit(&slug, window),
            1 => {
                let done = library::duplicate(&theme_files::dir(), &slug).map(|_| ());
                theme_files::reload();
                self.appearance_change(|_| done, window);
            }
            2 => self.export_open(&[slug]),
            _ => self.theme_delete(&[slug], window),
        }
    }
}
