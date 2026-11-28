//! What the launcher's search (`command_search`) reads of the installed mods and packs, and how
//! it shows one of them on its page. Reads what is in memory or a few small files; writes
//! nothing.
use super::*;
use std::sync::Arc;
use sunrise_launcher::packs::Kind;

impl Launcher {
    /// The selected profile's installed mods: each one's name and its icon's address.
    pub(crate) fn mods_for_search(&mut self) -> Vec<(String, String, Option<String>)> {
        self.mods_refresh();
        self.rows()
            .into_iter()
            .map(|row| (row.id, row.title, row.icon))
            .collect()
    }

    /// Its resource packs, then its shader packs: each one's kind, name and picture.
    pub(crate) fn packs_for_search(
        &mut self,
    ) -> Vec<(Kind, String, String, Option<Arc<gpui::Image>>)> {
        self.load_packs_list(self.mods_home());
        let packs = &self.game_packs;
        packs
            .resource
            .iter()
            .map(|(pack, icon)| {
                (
                    pack.kind,
                    pack.file.clone(),
                    pack.title.clone(),
                    icon.clone(),
                )
            })
            .chain(
                packs
                    .shader
                    .iter()
                    .map(|pack| (pack.kind, pack.file.clone(), pack.title.clone(), None)),
            )
            .collect()
    }

    /// The page of the installed mod called `title` (or with that id): its own page, as Home's
    /// card opens it. False when the profile has no such mod.
    pub(crate) fn open_mod_named(&mut self, title: &str) -> bool {
        self.mods_refresh();
        let Some(row) = self
            .rows()
            .into_iter()
            .find(|row| row.title == title || row.id == title)
        else {
            return false;
        };
        self.mods_show_installed_project(&row);
        true
    }
}
