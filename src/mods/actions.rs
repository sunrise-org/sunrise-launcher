//! Everything on the Mods screens that changes files. Each action is refused while the game or
//! a launch step is running. The launcher makes no backups of worlds itself: the game's own
//! Edit World › Make Backup is the way to keep one.
use super::model::{self, Row};
use super::*;
use crate::mods_page::install::Kind;
use std::collections::BTreeMap;

use sunrise_launcher::{
    managed_play::{HeldReason, Profile},
    profile_txn::Identity,
};

pub(super) enum EnableNotice {
    One(String),
    Bulk {
        held: Vec<(String, HeldReason)>,
        blocked: Vec<(String, String)>,
    },
}

pub(super) struct EnabledMods {
    count: usize,
    skipped: Vec<(String, HeldReason)>,
}

fn dispatch_enable(
    home: PathBuf,
    identity: Identity,
    ids: Vec<String>,
    reply: impl FnOnce(Result<EnabledMods, String>) + Send + 'static,
) {
    std::thread::spawn(move || {
        let result = (|| {
            let profile = Profile::open_for(&home, None, Some(identity)).map_err(String::from)?;
            let ids: Vec<_> = ids.iter().map(String::as_str).collect();
            let skipped =
                profile.enable_unheld(&ids, sunrise_launcher::browser::sunrise::activate)?;
            Ok(EnabledMods {
                count: ids.len() - skipped.len(),
                skipped: skipped
                    .into_iter()
                    .map(|(id, reason)| (id.into(), reason))
                    .collect(),
            })
        })();
        reply(result);
    });
}

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
            || self.switch_busy()
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
            .mods_state
            .snapshot
            .as_ref()
            .filter(|s| s.home == self.mods_home())
            .and_then(|s| s.error.as_ref())
        {
            Some(problem.summary().into())
        } else if self
            .mods_state
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| !snapshot.ready())
        {
            Some(
                tr("Installed mods aren't settled yet. Wait or retry before changing them.").into(),
            )
        } else if self.mutating() {
            Some(tr("Wait for the current operation to finish.").to_string())
        } else {
            None
        };
        if let Some(why) = refusal {
            self.mods_state.notice = Some(Err(why));
            return false;
        }
        true
    }

    /// Install a project from the signed v2 index (Discover, its pages and port rows).
    pub(crate) fn add_v2(&mut self, project_id: String) {
        let ids = vec![project_id.clone()];
        self.submit_registry_change(Kind::InstallV2, &project_id, ids, BTreeMap::new());
    }

    /// Install a collection's picked projects from the signed v2 index, planned together.
    pub(super) fn add_v2_collection(&mut self, collection: &str, project_ids: Vec<String>) {
        self.submit_registry_change(Kind::InstallV2, collection, project_ids, BTreeMap::new());
    }

    /// A direct switch has no world-loss question. Enabling still requires the dependencies;
    /// the native consent and build gates decide whether the game can use native code.
    pub(super) fn toggle(&mut self, row: &Row) {
        if !self.guard() {
            return;
        }
        if row.on {
            self.turn_off(vec![row.id.clone()]);
        } else if row.held_for.as_ref().is_some_and(|hold| hold.retry) {
            self.restore_held(row);
        } else if let Some(entry) = &row.entry {
            self.mods.message = None;
            self.enable_registry_mod(entry, self.mods_home());
            if let Some(Err(e)) = self.mods.message.take() {
                self.mods_state.notice = Some(Err(e));
            }
        } else if let Some(snapshot) = &self.mods_state.snapshot {
            self.enable_one_in(snapshot.home.clone(), row.id.clone(), row.title.clone());
        }
    }

    /// A held local or source mod turns on only once its exact bytes pass the fit check and
    /// build for this profile's build; otherwise the notice says what the check found.
    fn restore_held(&mut self, row: &Row) {
        let home = self.mods_home();
        let Some(instance) = self
            .selected_instance()
            .filter(|instance| instance.home == home)
            .cloned()
        else {
            self.mods_state.notice =
                Some(Err(tr("The selected profile changed. Try again.").into()));
            return;
        };
        let target = match sunrise_launcher::browser::sunrise::Target::of(&instance) {
            Ok(target) => target,
            Err(error) => {
                self.mods_state.notice = Some(Err(error.into()));
                return;
            }
        };
        self.mods_state.errors.clear();
        self.mods_state.notice = None;
        self.mods_state.busy = Some((
            trf("Checking {mod} for this build…", &[("mod", &row.title)]),
            0.,
        ));
        let tx = self.mods_state.tx.clone();
        let (id, title) = (row.id.clone(), row.title.clone());
        std::thread::spawn(move || {
            let result = sunrise_launcher::browser::sunrise::restore_held(
                &target,
                target.identity(),
                &id,
                &sunrise_launcher::native_mods::Cancellation::default(),
            )
            .map(|()| EnabledMods {
                count: 1,
                skipped: Vec::new(),
            });
            let _ = tx.send(Event::Enabled(home, EnableNotice::One(title), result));
        });
    }

    pub(crate) fn enable_one_in(&mut self, home: PathBuf, id: String, title: String) {
        self.queue_enable(home, vec![id], EnableNotice::One(title));
    }

    /// Turn the chosen registry mods on in one admitted rewrite. An already-known hold
    /// stays out of the plan; the worker also checks live holds before touching the file.
    pub(super) fn bulk_on(&mut self) {
        if !self.guard() {
            return;
        }
        let rows = self.rows();
        let Some(snapshot) = &self.mods_state.snapshot else {
            return;
        };
        let mut held = Vec::new();
        let mut candidates = Vec::new();
        for row in rows
            .iter()
            .filter(|r| r.local.is_none() && self.mods_state.selection.contains(&r.id))
        {
            if let Some(reason) = snapshot.switch_held.get(&row.id) {
                held.push((row.id.clone(), reason.clone()));
            } else {
                candidates.push(row);
            }
        }
        let (go, blocked) = model::turn_on_plan(&candidates);
        let blocked = blocked
            .into_iter()
            .map(|(row, reason)| (row.id.clone(), reason))
            .collect();
        let ids = go.into_iter().map(|row| row.id.clone()).collect();
        self.queue_enable(
            snapshot.home.clone(),
            ids,
            EnableNotice::Bulk { held, blocked },
        );
    }

    fn queue_enable(&mut self, home: PathBuf, ids: Vec<String>, notice: EnableNotice) {
        if !self.guard() {
            return;
        }
        if home != self.mods_home() {
            self.mods_state.notice =
                Some(Err(tr("The selected profile changed. Try again.").into()));
            return;
        }
        self.mods_state.errors.clear();
        self.mods_state.notice = None;
        if ids.is_empty() {
            self.enabled_mods(
                &home,
                notice,
                Ok(EnabledMods {
                    count: 0,
                    skipped: Vec::new(),
                }),
            );
            return;
        }
        let identity = match Identity::of(&home) {
            Ok(identity) => identity,
            Err(error) => {
                self.mods_state.notice = Some(Err(error));
                return;
            }
        };
        self.mods_state.busy = Some((tr("Turning on mods…").into(), 0.));
        let tx = self.mods_state.tx.clone();
        dispatch_enable(home.clone(), identity, ids, move |result| {
            let _ = tx.send(Event::Enabled(home, notice, result));
        });
    }

    pub(super) fn enabled_mods(
        &mut self,
        home: &Path,
        notice: EnableNotice,
        result: Result<EnabledMods, String>,
    ) {
        self.mods_state.busy = None;
        if self.mods_home() != home {
            return;
        }
        let m = &mut self.mods_state;
        m.notice = Some(result.and_then(|result| match notice {
            EnableNotice::One(title) => {
                if let Some((_, reason)) = result.skipped.first() {
                    Err(reason.label().into())
                } else {
                    Ok(trf(
                        "{mod} is on. The game reads this when it starts.",
                        &[("mod", &title)],
                    ))
                }
            }
            EnableNotice::Bulk { mut held, blocked } => {
                held.extend(result.skipped);
                let unknown = held.iter().filter(|(_, reason)| reason.unproved()).count();
                let needs_update = held.len() - unknown;
                m.errors.extend(
                    held.into_iter()
                        .map(|(id, reason)| (id, reason.label().into())),
                );
                let other = blocked.len();
                m.errors.extend(blocked);
                if needs_update == 0 && unknown == 0 && other == 0 {
                    return Ok(trf(
                        "Turned on {count} mods. The game reads this when it starts.",
                        &[("count", &result.count.to_string())],
                    ));
                }
                let mut lines = vec![trf(
                    "Turned on {count}.",
                    &[("count", &result.count.to_string())],
                )];
                if needs_update != 0 {
                    lines.push(trf(
                        "Needs an update from its author: {count}.",
                        &[("count", &needs_update.to_string())],
                    ));
                }
                if unknown != 0 {
                    lines.push(trf(
                        "Fit could not be verified: {count}.",
                        &[("count", &unknown.to_string())],
                    ));
                }
                if other != 0 {
                    lines.push(trf(
                        "{count} stayed off: see their rows.",
                        &[("count", &other.to_string())],
                    ));
                }
                Err(lines.join(" "))
            }
        }));
    }

    pub(super) fn bulk_off(&mut self) {
        let rows = self.rows();
        let chosen: Vec<String> = rows
            .iter()
            .filter(|r| r.local.is_none() && r.on && self.mods_state.selection.contains(&r.id))
            .map(|r| r.id.clone())
            .collect();
        if chosen.is_empty() {
            self.mods_state.notice = Some(Err(
                tr("None of the selected mods is turned on.").to_string()
            ));
            return;
        }
        self.turn_off(chosen);
    }

    pub(super) fn remove_mod_button(
        &self,
        id: &str,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let button = crate::frame::button("alt-project-remove", tr("Remove"))
            .text_color(ui::c(palette().danger));
        if busy {
            ui::dim(button)
        } else {
            let home = self.mods_home();
            let id = id.to_owned();
            button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                let (rows, labels) = this.labeled_installed_rows();
                let names = super::installed_view::remove_names(&rows, &labels);
                if let Some(open) = this.mods_state.open.as_mut() {
                    open.removing = vec![id.clone()];
                }
                this.ask_remove(&home, vec![id.clone()], names);
                cx.notify();
            }))
        }
    }

    /// Submit the approved scope, not old catalogue dependents. The worker reads the exact
    /// installed package manifests and offers one combined Remove decision.
    pub(super) fn ask_remove(
        &mut self,
        home: &Path,
        chosen: Vec<String>,
        displayed_names: BTreeMap<String, String>,
    ) {
        if !self.guard() {
            return;
        }
        if self.mods_home().as_path() != home {
            self.mods_state.notice =
                Some(Err(tr("The selected profile changed. Try again.").into()));
            return;
        }
        self.submit_registry_change(Kind::Remove, "remove", chosen, displayed_names);
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
                    && (!only_selected || self.mods_state.selection.contains(&row.id))
            })
            .map(|row| row.id)
            .collect();
        self.submit_registry_change(Kind::UpdateAllV2, "update-all", ids, BTreeMap::new());
    }

    pub(super) fn update_one(&mut self, row: &Row) {
        if row.local.is_none() && row.update.is_some() && self.guard() {
            let ids = vec![row.id.clone()];
            self.submit_registry_change(Kind::UpdateV2, &row.id, ids, BTreeMap::new());
        }
    }

    pub(in crate::mods) fn submit_registry_change(
        &mut self,
        kind: Kind,
        card: &str,
        ids: Vec<String>,
        displayed_names: BTreeMap<String, String>,
    ) {
        self.mods_state.notice = None;
        self.mods.message = None;
        if let Err(why) = self.queue_registry_change(kind, card.to_owned(), ids, displayed_names) {
            self.mods_state.notice = Some(Err(why));
        }
    }

    fn turn_off(&mut self, ids: Vec<String>) {
        if !self.guard() {
            return;
        }
        let home = self.mods_home();
        let m = &mut self.mods_state;
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

    #[test]
    fn enabling_dispatch_returns_before_a_held_manifest_lock_releases() {
        use std::time::Duration;
        const ACTOR: &str = "ENABLE_UNHELD_LOCK_ACTOR";
        let Some(root) = std::env::var_os(ACTOR).map(PathBuf::from) else {
            let root = tempfile::tempdir().unwrap();
            assert!(std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "mods::actions::tests::enabling_dispatch_returns_before_a_held_manifest_lock_releases"])
                .env(ACTOR, root.path())
                .env("SUNRISE_HOME", root.path().join("game"))
                .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
                .status().unwrap().success());
            return;
        };
        let home = root.join("game");
        let packages = home.join(sunrise_launcher_smod::mod_list::DIR);
        std::fs::create_dir_all(&packages).unwrap();
        let archive = sunrise_launcher_smod::Package::write_files(
            "id='light'\nname='Light'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n",
            &[],
        )
        .unwrap();
        std::fs::write(packages.join("light.smod"), archive).unwrap();
        let manifest = sunrise_launcher::browser::lock_home(&home).unwrap();
        let identity = Identity::of(&home).unwrap();
        let (returned, wait_return) = channel();
        let (done, wait_done) = channel();
        let selected = home.clone();
        std::thread::spawn(move || {
            dispatch_enable(selected, identity, vec!["light".into()], move |result| {
                done.send(result).unwrap()
            });
            returned.send(()).unwrap();
        });
        wait_return
            .recv_timeout(Duration::from_secs(5))
            .expect("don't block the caller on profile or manifest admission");
        assert!(matches!(
            wait_done.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        assert!(!mod_list::is_enabled(&home, "light"));
        drop(manifest);
        let enabled = wait_done
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!(enabled.count, 1);
        assert!(enabled.skipped.is_empty());
        assert!(mod_list::is_enabled(&home, "light"));
    }

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
