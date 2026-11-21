//! Everything on the Mods screens that changes files. Each action is refused while the game or
//! a launch step is running. The launcher makes no backups of worlds itself: the game's own
//! Edit World › Make Backup is the way to keep one.
use super::model::{self, Row};
use super::*;
use crate::mods_page::install::Kind;

impl Launcher {
    /// Why nothing may change now: the game, a launch step or a build is under way.
    pub(crate) fn mods_locked(&self) -> Option<&'static str> {
        if self.client.is_some()
            || self.starting_client.is_some()
            || self.starting_play.is_some()
            || self.play_admission.is_some()
        {
            Some(tr(
                "The game is running or starting. Close it before changing mods.",
            ))
        } else if self.preparing
            || self.installing.is_some()
            || self.play_when_prepared.is_some()
            || self.play_after_build.is_some()
            || self.pinning_build
        {
            Some(tr(
                "Sunrise is getting ready to play. Wait for it to finish before changing mods.",
            ))
        } else if self.native_step.is_some() {
            Some(tr(
                "A native build is running. Wait for it to finish before changing mods.",
            ))
        } else {
            None
        }
    }

    /// The gate every change passes: not locked, nothing else running.
    pub(super) fn guard(&mut self) -> bool {
        let refusal = if let Some(why) = self.mods_locked() {
            Some(why.to_string())
        } else if let Some(InstalledReadError::GameInstall(problem)) = self
            .ui
            .mods
            .snapshot
            .as_ref()
            .filter(|s| s.home == self.mods_home())
            .and_then(|s| s.error.as_ref())
        {
            Some(problem.summary().into())
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

    pub(crate) fn add(&mut self, id: String) {
        self.ui.mods.notice = None;
        self.mods.message = None;
        if let Err(why) = self.queue_registry_install(id.clone(), vec![id]) {
            self.ui.mods.notice = Some(Err(why));
        }
    }

    /// A direct switch has no world-loss question. Enabling still requires the dependencies;
    /// the native consent and build gates decide whether the game can use native code.
    pub(super) fn toggle(&mut self, row: &Row) {
        if !self.guard() {
            return;
        }
        let managed = self
            .selected_instance()
            .is_some_and(|i| i.kind != instances::Kind::Pack);
        if row.on {
            self.turn_off(vec![row.id.clone()]);
        } else if let Some(entry) = &row.entry {
            self.mods.message = None;
            self.mods_switch(entry);
            if let Some(Err(e)) = self.mods.message.take() {
                self.ui.mods.notice = Some(Err(e));
            }
        } else if let Some(snapshot) = &mut self.ui.mods.snapshot {
            let Some(_lock) = sunrise_launcher::browser::try_lock_home(&snapshot.home) else {
                self.ui.mods.notice =
                    Some(Err(tr("Wait for the current operation to finish.").into()));
                return;
            };
            let change = (if managed {
                sunrise::check_activation(&snapshot.home, &[&row.id])
            } else {
                Ok(())
            })
            .and_then(|()| mod_list::set_enabled(&snapshot.home, &row.id, true));
            self.ui.mods.notice = match change {
                Ok(()) => {
                    snapshot.key.clear();
                    None
                }
                Err(e) => Some(Err(e)),
            };
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
        let managed = self
            .selected_instance()
            .is_some_and(|i| i.kind != instances::Kind::Pack);
        let m = &mut self.ui.mods;
        m.errors = held
            .iter()
            .map(|(r, why)| (r.id.clone(), why.clone()))
            .collect();
        let Some(home) = m.snapshot.as_ref().map(|s| s.home.clone()) else {
            return;
        };
        if go.is_empty() {
            m.notice = Some(Err(tr("Nothing selected can be turned on yet.").to_string()));
            return;
        }
        let Some(_lock) = sunrise_launcher::browser::try_lock_home(&home) else {
            m.notice = Some(Err(
                tr("Wait for the current operation to finish.").to_string()
            ));
            return;
        };
        let ids: Vec<&str> = go.iter().map(|r| r.id.as_str()).collect();
        let count = ids.len().to_string();
        let change = (if managed {
            sunrise::check_activation(&home, &ids)
        } else {
            Ok(())
        })
        .and_then(|()| mod_list::set_all_enabled(&home, &ids, true));
        m.notice = Some(match change {
            Ok(()) if held.is_empty() => Ok(trf(
                "Turned on {count} mods. The game reads this when it starts.",
                &[("count", &count)],
            )),
            Ok(()) => Err(trf(
                "Turned on {done}. {held} stayed off: see their rows.",
                &[("done", &count), ("held", &held.len().to_string())],
            )),
            Err(e) => Err(e),
        });
    }

    pub(super) fn bulk_off(&mut self) {
        let rows = self.rows();
        let chosen: Vec<String> = rows
            .iter()
            .filter(|r| r.local.is_none() && r.on && self.ui.mods.selection.contains(&r.id))
            .map(|r| r.id.clone())
            .collect();
        if chosen.is_empty() {
            self.ui.mods.notice = Some(Err(
                tr("None of the selected mods is turned on.").to_string()
            ));
            return;
        }
        self.turn_off(chosen);
    }

    /// Submit the approved scope, not old catalogue dependents. The worker reads the exact
    /// installed package manifests and offers one combined Remove decision.
    pub(super) fn ask_remove(&mut self) {
        if !self.guard() {
            return;
        }
        let chosen = self
            .rows()
            .into_iter()
            .filter(|row| row.selectable_archive() && self.ui.mods.selection.contains(&row.id))
            .map(|row| row.id)
            .collect();
        self.submit_registry_change(Kind::Remove, "remove", chosen);
    }

    pub(super) fn ask_update(&mut self, only_selected: bool) {
        if !self.guard() {
            return;
        }
        let ids = self
            .rows()
            .into_iter()
            .filter(|row| {
                model::bulk_updatable(row)
                    && (!only_selected || self.ui.mods.selection.contains(&row.id))
            })
            .map(|row| row.id)
            .collect();
        self.submit_registry_change(Kind::UpdateAll, "update-all", ids);
    }

    pub(super) fn update_one(&mut self, row: &Row) {
        if row.local.is_none() && row.update.is_some() && self.guard() {
            self.submit_registry_change(Kind::Update, &row.id, vec![row.id.clone()]);
        }
    }

    fn submit_registry_change(&mut self, kind: Kind, card: &str, ids: Vec<String>) {
        self.ui.mods.notice = None;
        self.mods.message = None;
        if let Err(why) = self.queue_registry_change(kind, card.to_owned(), ids) {
            self.ui.mods.notice = Some(Err(why));
        }
    }

    fn turn_off(&mut self, ids: Vec<String>) {
        if !self.guard() {
            return;
        }
        let home = self.mods_home();
        let m = &mut self.ui.mods;
        m.notice = None;
        m.errors.clear();
        m.busy = Some((tr("Working…").to_string(), 0.));
        let tx = m.tx.clone();
        std::thread::spawn(move || turn_off(&ids, &home, &tx));
    }
}

/// `one` for a count of 1, else `many` with its `{count}` filled in.
fn counted(count: usize, one: &'static str, many: &'static str) -> String {
    match count {
        1 => tr(one).to_string(),
        n => trf(many, &[("count", &n.to_string())]),
    }
}

fn turn_off(ids: &[String], home: &std::path::Path, tx: &Sender<Event>) {
    let result = (|| {
        let _lock = sunrise_launcher::browser::lock_home(home).map_err(|e| e.to_string())?;
        let ids = ids.iter().map(String::as_str).collect::<Vec<_>>();
        mod_list::set_all_enabled(home, &ids, false)
    })();
    let _ = tx.send(match result {
        Ok(()) => Event::Done(
            counted(
                ids.len(),
                "Turned off 1 mod. The game reads this when it starts.",
                "Turned off {count} mods. The game reads this when it starts.",
            ),
            Vec::new(),
        ),
        Err(error) => Event::Refused(error),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use sunrise_launcher_nbt::{self as nbt, Limits, Tag};

    fn home_with_world(mod_id: &str) -> PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let home = std::env::temp_dir().join(format!(
            "alt-mods-actions-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&home);
        let dir = home.join("saves/Home");
        std::fs::create_dir_all(dir.join("region")).unwrap();
        let record = Tag::compound([
            ("ModId", Tag::String(mod_id.into())),
            ("ModVersion", Tag::String("1".into())),
        ]);
        let root = Tag::compound([
            (
                "Data",
                Tag::compound([("LevelName", Tag::String("Home".into()))]),
            ),
            (
                "sunrise",
                Tag::compound([("ContentModList", Tag::List(vec![record]))]),
            ),
        ]);
        let bytes = nbt::write_named(
            &nbt::NamedTag {
                name: "".into(),
                tag: root,
            },
            Limits::default(),
        )
        .unwrap();
        std::fs::write(dir.join("level.dat"), bytes).unwrap();
        std::fs::write(dir.join("region/r.0.0.mca"), [7u8; 64]).unwrap();
        mod_list::set_enabled(&home, mod_id, true).unwrap();
        mod_list::set_enabled(&home, "unrelated", true).unwrap();
        home
    }

    #[test]
    fn turning_off_a_mod_a_world_was_saved_with_is_just_the_switch() {
        let home = home_with_world("bricks");
        let (tx, rx) = channel();
        turn_off(&["bricks".into()], &home, &tx);
        assert!(matches!(rx.try_iter().last(), Some(Event::Done(..))));
        assert!(!mod_list::is_enabled(&home, "bricks"));
        assert!(mod_list::is_enabled(&home, "unrelated"));
        assert!(!home.join("backups").exists(), "no backup, no prompt");
        std::fs::remove_dir_all(home).ok();
    }

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

    #[test]
    fn a_mod_no_world_uses_changes_at_once() {
        let home = home_with_world("bricks");
        let (tx, rx) = channel();
        turn_off(&["unrelated".into()], &home, &tx);
        assert!(matches!(rx.try_iter().last(), Some(Event::Done(..))));
        assert!(!mod_list::is_enabled(&home, "unrelated"));
        std::fs::remove_dir_all(home).ok();
    }
}
