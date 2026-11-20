//! Everything on the Mods screens that changes files. Each action is refused while the game or
//! a launch step is running. The launcher makes no backups of worlds itself: the game's own
//! Edit World › Make Backup is the way to keep one.
use super::model::{self, Row};
use super::*;

use sunrise::txn::{ModEdit, ModRemoval};

/// Why mods can't change now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Lock {
    /// The game is running or starting.
    GameRunning,
    /// Sunrise is getting ready to play.
    Preparing,
    /// A native build is running.
    Building,
}

impl Lock {
    /// The first reason that applies, the most pressing first.
    fn of(game_running: bool, preparing: bool, building: bool) -> Option<Lock> {
        if game_running {
            Some(Lock::GameRunning)
        } else if preparing {
            Some(Lock::Preparing)
        } else if building {
            Some(Lock::Building)
        } else {
            None
        }
    }

    fn why(self) -> &'static str {
        match self {
            Lock::GameRunning => {
                tr("The game is running or starting. Close it before changing mods.")
            }
            Lock::Preparing => {
                tr("Sunrise is getting ready to play. Wait for it to finish before changing mods.")
            }
            Lock::Building => {
                tr("A native build is running. Wait for it to finish before changing mods.")
            }
        }
    }
}

impl Launcher {
    /// Opens a confirmation, remembering which folder it was reviewed for.
    fn confirm(&mut self, confirm: Confirm) {
        let home = self.ui.mods.snapshot.as_ref().map(|s| s.home.clone());
        self.ui.mods.reviewed_home = home;
        self.ui.mods.confirm = Some(confirm);
    }

    /// Whether the selected profile's game runs or is starting.
    pub(crate) fn game_running(&self) -> bool {
        self.client.is_some() || self.starting_client.is_some() || self.starting_play.is_some()
    }

    /// Why nothing may change now: the game, a launch step or a build is under way.
    pub(crate) fn mods_lock(&self) -> Option<Lock> {
        Lock::of(
            self.game_running(),
            self.preparing
                || self.installing.is_some()
                || self.play_when_prepared.is_some()
                || self.play_after_build.is_some(),
            self.native_step.is_some(),
        )
    }

    /// [`Self::mods_lock`] in words.
    pub(crate) fn mods_locked(&self) -> Option<&'static str> {
        self.mods_lock().map(Lock::why)
    }

    /// The gate every change passes: not locked, nothing else running.
    pub(super) fn guard(&mut self) -> bool {
        let refusal = if let Some(why) = self.mods_locked() {
            Some(why.to_string())
        } else if self.mutating() {
            Some(tr("Wait for the current operation to finish.").to_string())
        } else {
            None
        };
        if let Some(why) = refusal {
            self.ui.mods.notice = Some(Err(why));
            return false;
        }
        true
    }

    /// Add `id` to the game: the existing install path, which asks first for native code, checks
    /// the file against the index and its signature, and refuses what was taken down. Only a mod
    /// the registry says fits this build. `shown` is the name the page or card that asks shows.
    pub(crate) fn install_registry_mod(&mut self, id: String, shown: String) -> Result<(), String> {
        let fit = match &self.ui.mods.catalog {
            Catalog::Ready(cards, _) => cards.iter().find(|c| c.id == id).map(|c| c.fit.clone()),
            _ => None,
        };
        let Some(fit) = fit else {
            return Err(tr("The registry hasn't answered about this mod yet.").to_string());
        };
        if let Some(why) = fit.reason() {
            return Err(why);
        }
        // The card says the same before the click (`InstallState::CloseGame`).
        if self.game_running() {
            return Err(tr("Close the game to install").to_string());
        }
        // A build or another install doesn't refuse: this one waits its turn.
        if let Some(lock) = self.mods_lock().filter(|lock| *lock != Lock::Building) {
            return Err(lock.why().to_string());
        }
        if self.ui.mods.busy.is_some() {
            return Err(tr("Wait for the current operation to finish.").to_string());
        }
        self.ui.mods.notice = None;
        self.mods.message = None;
        self.mods_install(SourceId::Sunrise, id, None, Some(shown));
        Ok(())
    }

    /// Turn on the registry mod `id` the card says is installed but off: the same switch the
    /// Installed list's, which asks the one native question first when it must.
    pub(crate) fn turn_on_registry_mod(&mut self, id: &str) {
        // The same gate as every change: not while the game runs, or another change is under way.
        if !self.guard() {
            return;
        }
        let entry = self
            .ui
            .mods
            .snapshot
            .as_ref()
            .and_then(|s| s.entries.iter().find(|e| e.project == id))
            .cloned();
        if let Some((id, entry)) = entry
            .as_ref()
            .and_then(|entry| Some((sunrise::mod_id(entry)?, entry)))
        {
            self.toggle_one(
                id,
                &entry.title,
                true,
                native_status::is_native_file(&entry.file),
            );
        }
    }

    pub(crate) fn add(&mut self, id: String, shown: String) {
        if let Err(why) = self.install_registry_mod(id, shown) {
            self.ui.mods.notice = Some(Err(why));
        }
    }

    /// Turn the selected bit on or off. A native activation binds this exact action before asking.
    pub(super) fn toggle(&mut self, row: &Row) {
        if self.guard() {
            self.toggle_one(&row.id, &row.title, !row.on, row.native());
        }
    }

    fn toggle_one(&mut self, id: &str, title: &str, enabled: bool, native: bool) {
        let success = trf(
            if enabled {
                "Turned on {mod}."
            } else {
                "Turned off {mod}."
            },
            &[("mod", title)],
        );
        self.request_mod_edit(
            ModEdit::SetEnabled {
                ids: vec![id.into()],
                enabled,
            },
            title.into(),
            success,
            enabled && native,
        );
    }

    fn request_mod_edit(&mut self, edit: ModEdit, name: String, success: String, native: bool) {
        let Some(instance) = self
            .selected_instance()
            .filter(|i| {
                self.ui
                    .mods
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.home == i.home)
            })
            .cloned()
        else {
            self.ui.mods.notice = Some(Err(tr(
                "The profile changed since you reviewed this. Nothing was changed; choose again.",
            )
            .into()));
            return;
        };
        self.ui.mods.notice = None;
        let request = crate::mods_page::ModEditRequest {
            instance,
            edit,
            name,
            success,
        };
        if native && self.settings.native_mods_allowed_at.is_none() {
            self.pending_consent = Some(PendingConsent::Edit(Box::new(request)));
        } else {
            self.start_mod_edit(request);
        }
    }

    /// Turn the chosen registry mods on in one rewrite of the list. Local and GitHub mods
    /// have individual switches only: they aren't selectable for registry bulk actions.
    /// A mod that can't be turned on stays as it is and its row says why.
    pub(super) fn bulk_on(&mut self) {
        if !self.guard() {
            return;
        }
        let rows = self.rows();
        let chosen: Vec<&Row> = rows
            .iter()
            .filter(|r| r.local.is_none() && self.ui.mods.selection.contains(&r.id))
            .collect();
        let (go, held) =
            model::turn_on_plan(&chosen, self.settings.native_mods_allowed_at.is_some());
        let m = &mut self.ui.mods;
        m.errors = held
            .iter()
            .map(|(r, why)| (r.id.clone(), why.clone()))
            .collect();
        if go.is_empty() {
            m.notice = Some(Err(tr("Nothing selected can be turned on yet.").into()));
            return;
        }
        let ids: Vec<String> = go.iter().map(|r| r.id.clone()).collect();
        let count = ids.len().to_string();
        let success = if held.is_empty() {
            trf(
                "Turned on {count} mods. The game reads this when it starts.",
                &[("count", &count)],
            )
        } else {
            trf(
                "Turned on {done}. {held} stayed off: see their rows.",
                &[("done", &count), ("held", &held.len().to_string())],
            )
        };
        self.request_mod_edit(
            ModEdit::SetEnabled { ids, enabled: true },
            tr("Turn on").into(),
            success,
            false,
        );
    }

    pub(super) fn bulk_off(&mut self) {
        if !self.guard() {
            return;
        }
        let ids: Vec<_> = self
            .rows()
            .into_iter()
            .filter(|r| r.local.is_none() && r.on && self.ui.mods.selection.contains(&r.id))
            .map(|r| r.id)
            .collect();
        if ids.is_empty() {
            self.ui.mods.notice = Some(Err(tr("None of the selected mods is turned on.").into()));
            return;
        }
        let success = counted(
            ids.len(),
            "Turned off 1 mod. The game reads this when it starts.",
            "Turned off {count} mods. The game reads this when it starts.",
        );
        self.request_mod_edit(
            ModEdit::SetEnabled {
                ids,
                enabled: false,
            },
            tr("Turn off").into(),
            success,
            false,
        );
    }

    /// Asks for confirmation before removing the chosen mods.
    pub(super) fn ask_remove(&mut self) {
        let rows = self.rows();
        let chosen = rows
            .iter()
            .filter(|r| r.managed() && self.ui.mods.selection.contains(&r.id))
            .map(|r| {
                let receipt = match &r.entry {
                    Some(entry) => ModRemoval::registry(entry)?,
                    None => ModRemoval::github(r.origin().ok_or_else(|| {
                        tr("Installed mods changed. Refresh the list and try again.").to_owned()
                    })?),
                };
                Ok((r.id.clone(), receipt, r.title.clone()))
            })
            .collect::<Result<Vec<_>, String>>();
        match chosen {
            Ok(chosen) if !chosen.is_empty() => self.confirm(Confirm::Remove(chosen)),
            Ok(_) => {}
            Err(error) => self.ui.mods.notice = Some(Err(error)),
        }
    }

    /// One mod's update: its install again, which picks the newest version that fits.
    pub(super) fn update_one(&mut self, row: &Row) {
        self.update_rows(vec![row.id.clone()], row.title.clone());
    }

    /// "Update all": every mod with a newer version, as one install.
    pub(super) fn update_all(&mut self) {
        let ids = model::updatable(&self.rows())
            .iter()
            .map(|r| r.id.clone())
            .collect();
        self.update_rows(ids, tr("Updates").to_string());
    }

    /// The selected rows' updates, as one install.
    pub(super) fn update_selected(&mut self) {
        let rows = self.rows();
        let ids = model::updatable(&rows)
            .iter()
            .filter(|r| self.ui.mods.selection.contains(&r.id))
            .map(|r| r.id.clone())
            .collect();
        self.update_rows(ids, tr("Updates").to_string());
    }

    /// Updating is installing: the same path as the Install on Discover, so the native-code
    /// question, the plan and the world question, the queue and the transaction are the same.
    fn update_rows(&mut self, ids: Vec<String>, shown: String) {
        if self.game_running() {
            self.ui.mods.notice = Some(Err(tr("Close the game to update").to_string()));
            return;
        }
        if let Some(lock) = self.mods_lock().filter(|lock| *lock != Lock::Building) {
            self.ui.mods.notice = Some(Err(lock.why().to_string()));
            return;
        }
        self.ui.mods.notice = None;
        self.mods.message = None;
        match <[String; 1]>::try_from(ids) {
            Ok([id]) => self.mods_install(SourceId::Sunrise, id, None, Some(shown)),
            Err(ids) => self.mods_install_many(ids, shown),
        }
    }

    /// Use the exact rows the confirmation displayed, not newer rows with the same IDs.
    pub(super) fn remove_confirmed(&mut self) {
        if !self.guard() {
            return;
        }
        let m = &mut self.ui.mods;
        let Some(Confirm::Remove(items)) = m.confirm.take() else {
            return;
        };
        let current = m.snapshot.as_ref().map(|s| s.home.as_path());
        if current.is_none() || m.reviewed_home.take().as_deref() != current {
            m.selection.clear();
            m.notice = Some(Err(tr(
                "The profile changed since you reviewed this. Nothing was changed; choose again.",
            )
            .into()));
            return;
        }
        m.errors.clear();
        let count = items.len();
        let removals = items.into_iter().map(|(_, receipt, _)| receipt).collect();
        m.selection.clear();
        self.request_mod_edit(
            ModEdit::Remove(removals),
            tr("Remove").into(),
            counted(count, "Removed 1 mod.", "Removed {count} mods."),
            false,
        );
    }
}

/// `one` for a count of 1, else `many` with its `{count}` filled in.
fn counted(count: usize, one: &'static str, many: &'static str) -> String {
    match count {
        1 => tr(one).to_string(),
        n => trf(many, &[("count", &n.to_string())]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_of_mods_say_mod_once_and_mods_otherwise() {
        assert_eq!(
            counted(1, "Removed 1 mod.", "Removed {count} mods."),
            "Removed 1 mod."
        );
        assert_eq!(
            counted(3, "Removed 1 mod.", "Removed {count} mods."),
            "Removed 3 mods."
        );
        assert_eq!(
            counted(0, "Removed 1 mod.", "Removed {count} mods."),
            "Removed 0 mods."
        );
    }
}

#[cfg(test)]
mod lock_tests {
    use super::*;

    #[test]
    fn the_updater_of_play_and_of_the_start_holds_installs_like_a_build_does() {
        assert_eq!(Lock::of(false, false, false), None);
        assert_eq!(Lock::of(false, true, true), Some(Lock::Preparing));
        assert_eq!(Lock::of(true, true, true), Some(Lock::GameRunning));
        assert_eq!(Lock::of(false, false, true), Some(Lock::Building));
    }
}
