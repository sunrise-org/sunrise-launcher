//! Everything on the Mods screens that changes files. Each action is refused while the game or
//! a launch step is running. The launcher makes no backups of worlds itself: the game's own
//! Edit World › Make Backup is the way to keep one.
use super::model::{self, Included, Row};
use super::*;
use sunrise_launcher::browser::sunrise::auto;

#[derive(Clone)]
pub(super) enum Action {
    /// (mod id, title)
    TurnOff(Vec<(String, String)>),
    /// (mod id, project id, title)
    Remove(Vec<(String, String, String)>),
    Update(Vec<Included>),
}

impl Launcher {
    /// Opens a confirmation, remembering which folder it was reviewed for.
    fn confirm(&mut self, confirm: Confirm) {
        let home = self.ui.mods.snapshot.as_ref().map(|s| s.home.clone());
        self.ui.mods.reviewed_home = home;
        self.ui.mods.confirm = Some(confirm);
    }

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

    /// Turn one mod on, or check its worlds before turning it off. For files installed
    /// outside the registry, switching on records the choice; the native consent and build
    /// gates still decide whether the game can use it.
    pub(super) fn toggle(&mut self, row: &Row) {
        if !self.guard() {
            return;
        }
        if row.on {
            self.run(Action::TurnOff(vec![(row.id.clone(), row.title.clone())]));
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
            self.ui.mods.notice = match mod_list::set_enabled(&snapshot.home, &row.id, true) {
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
        m.notice = Some(match mod_list::set_all_enabled(&home, &ids, true) {
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
        let chosen: Vec<(String, String)> = rows
            .iter()
            .filter(|r| r.local.is_none() && r.on && self.ui.mods.selection.contains(&r.id))
            .map(|r| (r.id.clone(), r.title.clone()))
            .collect();
        if chosen.is_empty() {
            self.ui.mods.notice = Some(Err(
                tr("None of the selected mods is turned on.").to_string()
            ));
            return;
        }
        self.run(Action::TurnOff(chosen));
    }

    /// Asks for confirmation before removing the chosen mods.
    pub(super) fn ask_remove(&mut self) {
        let rows = self.rows();
        let chosen: Vec<(String, String, String)> = rows
            .iter()
            .filter(|r| r.local.is_none() && self.ui.mods.selection.contains(&r.id))
            .filter_map(|r| {
                Some((
                    r.id.clone(),
                    r.entry.as_ref()?.project.clone(),
                    r.title.clone(),
                ))
            })
            .collect();
        if !chosen.is_empty() {
            self.confirm(Confirm::Remove(chosen));
        }
    }

    /// Plans "Update N mods" for the whole pack, or for the selected rows only.
    pub(super) fn ask_update(&mut self, only_selected: bool) {
        let rows: Vec<Row> = self
            .rows()
            .into_iter()
            .filter(|r| !only_selected || self.ui.mods.selection.contains(&r.id))
            .collect();
        let plan = model::plan_updates(&rows, self.settings.native_mods_allowed_at.is_some());
        self.confirm(Confirm::Update(plan));
    }

    /// One mod's update: the same reviewed path as the batch, for one mod.
    pub(super) fn update_one(&mut self, row: &Row) {
        let Some(to) = row.update.clone().filter(|_| row.local.is_none()) else {
            return;
        };
        let refusal = if row.native() && self.settings.native_mods_allowed_at.is_none() {
            Some(tr(
                "It runs native code. Allow native mods first (Allow… on its row), then update it.",
            ))
        } else {
            None
        };
        if let Some(why) = refusal {
            self.ui.mods.notice = Some(Err(why.to_string()));
            return;
        }
        self.run(Action::Update(vec![Included {
            id: row.id.clone(),
            title: row.title.clone(),
            from: row.version.clone(),
            to,
            native: row.native(),
        }]));
    }

    /// Starts `action` on a worker.
    pub(super) fn run(&mut self, action: Action) {
        if !self.guard() {
            return;
        }
        let game = self.mods_game();
        let m = &mut self.ui.mods;
        let current = m
            .snapshot
            .as_ref()
            .map(|s| s.home.clone())
            .unwrap_or_else(backend::game_home);
        if !reviewed_here(m.confirm.is_some(), &m.reviewed_home, &current) {
            m.confirm = None;
            m.selection.clear();
            m.notice = Some(Err(tr(
                "The profile changed since you reviewed this. Nothing was changed; choose again.",
            )
            .to_string()));
            return;
        }
        m.confirm = None;
        m.notice = None;
        m.errors.clear();
        m.busy = Some((tr("Working…").to_string(), 0.));
        if matches!(action, Action::Remove(_)) {
            m.selection.clear();
        }
        let (tx, home) = (
            m.tx.clone(),
            m.snapshot
                .as_ref()
                .map(|s| s.home.clone())
                .unwrap_or_else(backend::game_home),
        );
        std::thread::spawn(move || run(action, &home, &game, &tx));
    }
}

/// Whether a confirmation (if one is open) was made for the folder the change would go into.
fn reviewed_here(confirming: bool, reviewed: &Option<PathBuf>, current: &std::path::Path) -> bool {
    !confirming || reviewed.as_deref() == Some(current)
}

/// `one` for a count of 1, else `many` with its `{count}` filled in.
fn counted(count: usize, one: &'static str, many: &'static str) -> String {
    match count {
        1 => tr(one).to_string(),
        n => trf(many, &[("count", &n.to_string())]),
    }
}

fn run(action: Action, home: &std::path::Path, game: &str, tx: &Sender<Event>) {
    let refuse = |why: String| {
        let _ = tx.send(Event::Refused(why));
    };
    let _lock = match sunrise_launcher::browser::lock_home(home) {
        Ok(lock) => lock,
        Err(e) => return refuse(e.to_string()),
    };
    let done = |said: String, failed: Vec<(String, String)>| {
        let _ = tx.send(Event::Done(said, failed));
    };
    match action {
        Action::TurnOff(items) => {
            let ids: Vec<&str> = items.iter().map(|(id, _)| id.as_str()).collect();
            match mod_list::set_all_enabled(home, &ids, false) {
                Ok(()) => done(
                    counted(
                        ids.len(),
                        "Turned off 1 mod. The game reads this when it starts.",
                        "Turned off {count} mods. The game reads this when it starts.",
                    ),
                    Vec::new(),
                ),
                Err(e) => refuse(e),
            }
        }
        Action::Remove(items) => {
            let (mut removed, mut failed) = (0, Vec::new());
            for (i, (id, project, title)) in items.iter().enumerate() {
                let _ = tx.send(Event::Progress(
                    trf("Removing {name}…", &[("name", title)]),
                    i as f32 / items.len() as f32,
                ));
                match installed::remove(home, project) {
                    Ok(_) => removed += 1,
                    Err(e) => failed.push((id.clone(), e)),
                }
            }
            done(
                counted(removed, "Removed 1 mod.", "Removed {count} mods."),
                failed,
            );
        }
        Action::Update(items) => {
            let index = match sunrise::index() {
                Ok(index) => index,
                Err(e) => return refuse(e),
            };
            let (mut updated, mut failed) = (0, Vec::new());
            let count = items.len();
            for (i, item) in items.iter().enumerate() {
                let step = |line: &str, f: f32| {
                    let _ = tx.send(Event::Progress(
                        trf(
                            "{n} of {count}: {line}",
                            &[
                                ("n", &(i + 1).to_string()),
                                ("count", &count.to_string()),
                                ("line", line),
                            ],
                        ),
                        (i as f32 + f) / count as f32,
                    ));
                };
                // Read now, not at planning: a mod that is off stays off.
                let was_on = mod_list::is_enabled(home, &item.id);
                let result = match item.native {
                    true => sunrise::Registry
                        .project(&item.id)
                        .and_then(|p| sunrise::update_native(&p, home, game, was_on, &step))
                        .map(|_| None),
                    false => auto::update_to(&index, &item.id, &item.to, home, &step),
                };
                match result {
                    Ok(None) => updated += 1,
                    Ok(Some(why)) => failed.push((item.id.clone(), why)),
                    Err(e) => failed.push((item.id.clone(), e)),
                }
            }
            done(
                counted(
                    updated,
                    "Updated 1 mod, checked against the registry.",
                    "Updated {count} mods, each checked against the registry.",
                ),
                failed,
            );
        }
    }
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

    fn off(ids: &[&str]) -> Action {
        Action::TurnOff(
            ids.iter()
                .map(|i| (i.to_string(), i.to_uppercase()))
                .collect(),
        )
    }

    #[test]
    fn turning_off_a_mod_a_world_was_saved_with_is_just_the_switch() {
        let home = home_with_world("bricks");
        let (tx, rx) = channel();
        run(off(&["bricks"]), &home, "26.3", &tx);
        assert!(matches!(rx.try_iter().last(), Some(Event::Done(..))));
        assert!(!mod_list::is_enabled(&home, "bricks"));
        assert!(mod_list::is_enabled(&home, "unrelated"));
        assert!(!home.join("backups").exists(), "no backup, no prompt");
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn old_backups_are_left_exactly_where_they_are() {
        let home = home_with_world("bricks");
        let old = home.join("backups");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("2026-01-01_00-00-00_Home.zip"), b"older backup").unwrap();
        let (tx, rx) = channel();
        run(off(&["bricks"]), &home, "26.3", &tx);
        run(
            Action::Remove(vec![("bricks".into(), "bricks".into(), "BRICKS".into())]),
            &home,
            "26.3",
            &tx,
        );
        assert!(rx
            .try_iter()
            .any(|e| matches!(e, Event::Done(..) | Event::Refused(_))));
        let kept: Vec<_> = std::fs::read_dir(&old)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(kept, ["2026-01-01_00-00-00_Home.zip"]);
        assert_eq!(std::fs::read(old.join(&kept[0])).unwrap(), b"older backup");
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
        run(off(&["unrelated"]), &home, "26.3", &tx);
        assert!(matches!(rx.try_iter().last(), Some(Event::Done(..))));
        assert!(!mod_list::is_enabled(&home, "unrelated"));
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn a_review_made_for_one_setup_cannot_change_another() {
        let a = PathBuf::from("/a/home");
        let b = PathBuf::from("/b/home");
        assert!(
            reviewed_here(false, &None, &a),
            "no confirmation open: nothing to match"
        );
        assert!(reviewed_here(true, &Some(a.clone()), &a));
        assert!(!reviewed_here(true, &Some(a.clone()), &b));
        assert!(!reviewed_here(true, &None, &a));
    }
}
