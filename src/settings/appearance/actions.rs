//! Settings › Appearance: what the colour editor and the theme library do. Each action works
//! on the settings through [`Launcher::appearance_change`], so a failure puts everything back
//! and is said under the page, and a success is painted at once.
use super::state::Panel;
use super::*;
use sunrise_launcher::theme_files::{self, Reason, Roles};
use sunrise_launcher::themes::library::{self, SLOTS};
use sunrise_launcher::themes::share;

impl Launcher {
    /// The colours the editor shows: ones typed that can't be read on each other yet, else
    /// what is painted.
    pub(super) fn editor_roles(&self) -> Roles {
        let draft = self.ui.settings.appearance.draft;
        draft.unwrap_or_else(|| library::current(&self.settings))
    }

    /// Sets one colour of the editor and paints the result. Colours that can't be read on
    /// each other are kept in the editor and not painted, with why.
    pub(super) fn colour_set(&mut self, slot: usize, rgb: u32, window: &Window) {
        let mut roles = self.editor_roles();
        (SLOTS[slot].set)(&mut roles, rgb);
        self.ui.settings.appearance.draft = None;
        match roles.readable() {
            Ok(()) => self.appearance_change(|s| library::paint(s, &roles), window),
            Err(reason) => {
                let state = &mut self.ui.settings.appearance;
                state.draft = Some(roles);
                state.error = None;
                state.note = Some(reason.text());
            }
        }
    }

    /// Takes what is typed in the hex field, if a colour is being typed.
    pub(super) fn colour_commit(&mut self, window: &Window) {
        let state = &mut self.ui.settings.appearance;
        let Some(slot) = state.editing.take() else {
            return;
        };
        let Some(rgb) = library::parse_hex(state.hex.text()) else {
            let label = tr(SLOTS[slot].label).to_string();
            state.error = Some(Reason::Colour(label).text());
            return;
        };
        if rgb != (SLOTS[slot].get)(&self.editor_roles()) {
            self.colour_set(slot, rgb, window);
        }
    }

    /// Starts typing the colour in `slot`, taking what was typed in another first.
    pub(super) fn colour_start(
        &mut self,
        slot: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.colour_commit(window);
        let rgb = (SLOTS[slot].get)(&self.editor_roles());
        let state = &mut self.ui.settings.appearance;
        state.editing = Some(slot);
        state.hex.set(format!("#{rgb:06x}"));
        state.hex.select_all();
        let focus = state
            .hex_focus
            .get_or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone();
        window.focus(&focus, cx);
    }

    /// Enter in the hex field.
    pub(super) fn colour_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.colour_commit(window);
        window.blur(cx);
    }

    /// Escape in a text box: nothing typed is taken, and an open panel closes.
    pub(super) fn text_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = &mut self.ui.settings.appearance;
        state.editing = None;
        state.panel = None;
        window.blur(cx);
    }

    /// Chooses Sunrise's night or day, with its own colours.
    pub(super) fn look_pick(&mut self, light: bool, window: &Window) {
        self.ui.settings.appearance.draft = None;
        self.appearance_change(|s| library::reset(s, light), window);
    }

    /// Loads the colours of the library's theme `slug` into the editor and paints them.
    pub(super) fn theme_apply(&mut self, slug: &str, window: &Window) {
        let found = library::listed(&theme_files::loaded())
            .into_iter()
            .find(|local| local.slug == slug);
        self.ui.settings.appearance.draft = None;
        self.appearance_change(
            |s| match found {
                Some(local) => library::paint(s, &local.roles),
                None => Err(Reason::NoFile.text()),
            },
            window,
        );
    }

    /// Opens the library's panel for `panel`, with its text box ready where it has one.
    pub(super) fn panel_open(&mut self, panel: Panel, window: &mut Window, cx: &mut Context<Self>) {
        let named = match &panel {
            Panel::Rename(slug) => library::listed(&theme_files::loaded())
                .into_iter()
                .find(|local| &local.slug == slug)
                .map(|local| local.name),
            _ => None,
        };
        let state = &mut self.ui.settings.appearance;
        state.error = None;
        state.note = None;
        state.more = None;
        let ready = || cx.focus_handle().tab_index(0).tab_stop(true);
        let focus = match &panel {
            Panel::Save | Panel::Rename(_) => {
                state.name.set(named.unwrap_or_default());
                state.name.select_all();
                Some(state.name_focus.get_or_insert_with(ready).clone())
            }
            Panel::Import => {
                state.code.set(String::new());
                Some(state.code_focus.get_or_insert_with(ready).clone())
            }
            Panel::Delete(_) | Panel::Export(..) => None,
        };
        state.panel = Some(panel);
        if let Some(focus) = focus {
            window.focus(&focus, cx);
        }
    }

    /// The code for the themes in the folders `slugs`, in a panel to copy it from.
    pub(super) fn export_open(&mut self, slugs: &[String]) {
        let themes = library::listed(&theme_files::loaded());
        let chosen: Vec<_> = themes
            .into_iter()
            .filter(|local| slugs.contains(&local.slug))
            .map(|local| library::Custom {
                name: local.name,
                roles: local.roles,
            })
            .collect();
        if chosen.is_empty() {
            return;
        }
        let state = &mut self.ui.settings.appearance;
        state.error = None;
        state.note = None;
        state.more = None;
        state.panel = Some(Panel::Export(
            share::encode(&chosen),
            chosen.len().min(share::MOST),
        ));
    }

    /// What the open panel's main button does: saves, renames, deletes or imports. On success
    /// the panel closes; otherwise it stays, with why.
    pub(super) fn panel_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = &self.ui.settings.appearance;
        let Some(panel) = state.panel.clone() else {
            return;
        };
        let root = theme_files::dir();
        let typed = state.name.text().trim().to_string();
        let said = match &panel {
            Panel::Save => library::save_new(&root, &typed, &self.editor_roles())
                .map(|_| tr("Saved to your themes.").to_string()),
            Panel::Rename(slug) => library::rename(&root, slug, &typed).map(|()| String::new()),
            Panel::Delete(slugs) => slugs
                .iter()
                .try_for_each(|slug| library::remove(&root, slug))
                .map(|()| tr("Deleted.").to_string()),
            Panel::Import => share::decode(state.code.text())
                .map_err(|e| e.text())
                .and_then(|themes| library::add_all(&root, &themes))
                .map(|n| {
                    trf(
                        "Added {count} to your themes.",
                        &[("count", &n.to_string())],
                    )
                }),
            Panel::Export(..) => Ok(String::new()),
        };
        theme_files::reload();
        let note = said.as_ref().ok().cloned().filter(|note| !note.is_empty());
        let ok = said.is_ok();
        self.appearance_change(|_| said.map(|_| ()), window);
        if ok {
            let state = &mut self.ui.settings.appearance;
            state.note = note;
            state.panel = None;
            state.ticked.clear();
            window.blur(cx);
        }
    }

    /// One of a library row's "…" menu: Rename…, Duplicate, Export…, Delete….
    pub(super) fn theme_menu_pick(
        &mut self,
        slug: String,
        pick: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ui.settings.appearance.more = None;
        match pick {
            0 => self.panel_open(Panel::Rename(slug), window, cx),
            1 => {
                let done = library::duplicate(&theme_files::dir(), &slug).map(|_| ());
                theme_files::reload();
                self.appearance_change(|_| done, window);
            }
            2 => self.export_open(&[slug]),
            _ => self.panel_open(Panel::Delete(vec![slug]), window, cx),
        }
    }
}
