//! What a mod's card on Home offers beyond ticking: the facts its "…" menu and version line are
//! made of, read from Installed's rows once per change, and the single-mod actions (switch, update,
//! remove) it starts through the same actions and locks the Mods pages use.
use super::super::*;
use super::ModOp;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// What one installed mod's card says and allows.
#[derive(Clone, Debug)]
pub(crate) struct CardFacts {
    pub(crate) version: String,
    /// The version an update would install, for a registry mod.
    pub(crate) update: Option<String>,
    /// Its page in the registry, when it has one the browser can open.
    pub(crate) page: Option<String>,
    /// Its file in the profile.
    pub(crate) file: Option<PathBuf>,
    /// Whether it has a switch that can act: an unreadable file has none.
    pub(crate) switchable: bool,
    pub(crate) removable: bool,
}

/// [`CardFacts`] of every mod, for the inventory and the unfit-updates answer they were made
/// from.
#[derive(Default)]
pub(crate) struct CardCache {
    inventory: u64,
    unfit: u64,
    rows: Option<Arc<HashMap<String, CardFacts>>>,
}

impl Launcher {
    /// The facts of every installed mod, by id. Empty until Installed has read the profile; it
    /// is asked again each frame until it has.
    pub(crate) fn home_card_facts(&mut self, inventory: u64) -> Arc<HashMap<String, CardFacts>> {
        let unfit = sunrise_launcher::browser::sunrise::unfit_updates_revision();
        let cache = &self.ui.home.tabs.facts;
        if let Some(rows) = &cache.rows {
            if cache.inventory == inventory && cache.unfit == unfit {
                return rows.clone();
            }
        }
        self.mods_refresh();
        let ready = self
            .mods_state
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.ready());
        if !ready {
            return Arc::default();
        }
        let home = self.mods_home();
        let mut facts = HashMap::new();
        for row in self.rows() {
            let page = row
                .entry
                .as_ref()
                .map(|entry| entry.page.clone())
                .filter(|page| page.starts_with("https://"));
            facts.insert(
                row.id.clone(),
                CardFacts {
                    version: row.version.clone(),
                    update: row.local.is_none().then(|| row.update.clone()).flatten(),
                    page,
                    file: row.archive_file().map(|file| home.join(file)),
                    switchable: row.selectable_archive(),
                    removable: row.selectable_archive(),
                },
            );
        }
        let rows = Arc::new(facts);
        self.ui.home.tabs.facts = CardCache {
            inventory,
            unfit,
            rows: Some(rows.clone()),
        };
        rows
    }

    /// Turns the mod `id` on or off as its row on the Mods page does. Its result says nothing
    /// (the switch shows it); a refusal or a failure is told in a toast.
    pub(crate) fn home_mod_toggle(&mut self, id: &str) {
        self.mods_refresh();
        let Some(row) = self.rows().into_iter().find(|row| row.id == id) else {
            return;
        };
        self.mods_state.bulk_toast = true;
        self.ui.home.tabs.quiet = true;
        self.toggle(&row);
    }

    /// Updates the mod `id`; the result is told in a toast.
    pub(crate) fn home_mod_update(&mut self, id: &str) {
        self.mods_refresh();
        self.mods_state.bulk_toast = true;
        self.update_mod(id);
    }

    /// Removes the mod `id` through the Remove question the Mods page asks.
    pub(crate) fn home_mod_remove(&mut self, id: &str) {
        let ids = std::iter::once(id.to_owned()).collect();
        self.run_mod_op(ModOp::Remove, &ids);
    }
}
