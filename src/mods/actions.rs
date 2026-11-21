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
    grant_native: bool,
    reply: impl FnOnce(Result<EnabledMods, String>, Option<u64>) + Send + 'static,
) {
    std::thread::spawn(move || {
        let mut allowed_at = None;
        let result = (|| {
            let profile = Profile::open_for(&home, None, Some(identity)).map_err(String::from)?;
            if grant_native {
                let mut settings = backend::Settings::default();
                settings.allow_native_mods()?;
                allowed_at = settings.native_mods_allowed_at;
            }
            let ids: Vec<_> = ids.iter().map(String::as_str).collect();
            let skipped = profile.enable_unheld(&ids)?;
            Ok(EnabledMods {
                count: ids.len() - skipped.len(),
                skipped: skipped
                    .into_iter()
                    .map(|(id, reason)| (id.into(), reason))
                    .collect(),
            })
        })();
        reply(result, allowed_at);
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
        } else if self
            .ui
            .mods
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
        if row.on {
            self.turn_off(vec![row.id.clone()]);
        } else if let Some(entry) = &row.entry {
            self.mods.message = None;
            self.enable_registry_mod(entry, self.mods_home(), None);
            if let Some(Err(e)) = self.mods.message.take() {
                self.ui.mods.notice = Some(Err(e));
            }
        } else if let Some(snapshot) = &self.ui.mods.snapshot {
            self.enable_one_in(snapshot.home.clone(), row.id.clone(), row.title.clone());
        }
    }

    pub(crate) fn enable_one_in(&mut self, home: PathBuf, id: String, title: String) {
        self.queue_enable(home, vec![id], EnableNotice::One(title), None);
    }

    pub(crate) fn enable_native_in(
        &mut self,
        home: PathBuf,
        identity: Identity,
        id: String,
        title: String,
    ) {
        self.queue_enable(home, vec![id], EnableNotice::One(title), Some(identity));
    }

    /// Turn the chosen registry mods on in one admitted rewrite. An already-known hold
    /// stays out of the plan; the worker also checks live holds before touching the file.
    pub(super) fn bulk_on(&mut self) {
        if !self.guard() {
            return;
        }
        let rows = self.rows();
        let Some(snapshot) = &self.ui.mods.snapshot else {
            return;
        };
        let mut held = Vec::new();
        let mut candidates = Vec::new();
        for row in rows
            .iter()
            .filter(|r| r.local.is_none() && self.ui.mods.selection.contains(&r.id))
        {
            if let Some(reason) = snapshot.switch_held.get(&row.id) {
                held.push((row.id.clone(), reason.clone()));
            } else {
                candidates.push(row);
            }
        }
        let (go, blocked) =
            model::turn_on_plan(&candidates, self.settings.native_mods_allowed_at.is_some());
        let blocked = blocked
            .into_iter()
            .map(|(row, reason)| (row.id.clone(), reason))
            .collect();
        let ids = go.into_iter().map(|row| row.id.clone()).collect();
        self.queue_enable(
            snapshot.home.clone(),
            ids,
            EnableNotice::Bulk { held, blocked },
            None,
        );
    }

    fn queue_enable(
        &mut self,
        home: PathBuf,
        ids: Vec<String>,
        notice: EnableNotice,
        approved: Option<Identity>,
    ) {
        if !self.guard() {
            return;
        }
        if home != self.mods_home() {
            self.ui.mods.notice = Some(Err(tr("The selected profile changed. Try again.").into()));
            return;
        }
        self.ui.mods.errors.clear();
        self.ui.mods.notice = None;
        if ids.is_empty() {
            self.enabled_mods(
                &home,
                notice,
                Ok(EnabledMods {
                    count: 0,
                    skipped: Vec::new(),
                }),
                None,
            );
            return;
        }
        let identity = match approved.map_or_else(|| Identity::of(&home), Ok) {
            Ok(identity) => identity,
            Err(error) => {
                self.ui.mods.notice = Some(Err(error));
                return;
            }
        };
        self.ui.mods.busy = Some((tr("Turning on mods…").into(), 0.));
        let tx = self.ui.mods.tx.clone();
        dispatch_enable(
            home.clone(),
            identity,
            ids,
            approved.is_some(),
            move |result, allowed_at| {
                let _ = tx.send(Event::Enabled(home, notice, result, allowed_at));
            },
        );
    }

    pub(super) fn enabled_mods(
        &mut self,
        home: &Path,
        notice: EnableNotice,
        result: Result<EnabledMods, String>,
        allowed_at: Option<u64>,
    ) {
        if allowed_at.is_some() {
            self.settings.native_mods_allowed_at = allowed_at;
        }
        self.ui.mods.busy = None;
        if self.mods_home() != home {
            return;
        }
        let m = &mut self.ui.mods;
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
            self.ui.mods.notice = Some(Err(tr("The selected profile changed. Try again.").into()));
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
                    && (!only_selected || self.ui.mods.selection.contains(&row.id))
            })
            .map(|row| row.id)
            .collect();
        self.submit_registry_change(Kind::UpdateAll, "update-all", ids, BTreeMap::new());
    }

    pub(super) fn update_one(&mut self, row: &Row) {
        if row.local.is_none() && row.update.is_some() && self.guard() {
            self.submit_registry_change(
                Kind::Update,
                &row.id,
                vec![row.id.clone()],
                BTreeMap::new(),
            );
        }
    }

    fn submit_registry_change(
        &mut self,
        kind: Kind,
        card: &str,
        ids: Vec<String>,
        displayed_names: BTreeMap<String, String>,
    ) {
        self.ui.mods.notice = None;
        self.mods.message = None;
        if let Err(why) = self.queue_registry_change(kind, card.to_owned(), ids, displayed_names) {
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
            dispatch_enable(
                selected,
                identity,
                vec!["light".into()],
                false,
                move |result, _| done.send(result).unwrap(),
            );
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

    #[test]
    fn toggle_consent_returns_while_manifest_and_settings_are_locked() {
        use crate::native_consent::{self, PendingConsent, Toggle};
        use std::{fs, time::Duration};
        use sunrise_launcher::browser::{installed::Entry, sunrise, Kind, SourceId};

        const ACTOR: &str = "NATIVE_TOGGLE_LOCK_ACTOR";
        let Some(root) = std::env::var_os(ACTOR).map(PathBuf::from) else {
            let root = tempfile::tempdir().unwrap();
            assert!(std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "mods::actions::tests::toggle_consent_returns_while_manifest_and_settings_are_locked"])
                .env(ACTOR, root.path())
                .env("SUNRISE_HOME", root.path().join("game"))
                .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
                .status().unwrap().success());
            return;
        };
        let home = root.join("game");
        let packages = home.join(sunrise_launcher_smod::mod_list::NATIVE_DIR);
        fs::create_dir_all(&packages).unwrap();
        let archive = sunrise_launcher_smod::Package::write_files(
            &format!(
                "id='light'\nname='Light'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n[native]\nsunrise='{}'\nmixins='{}'\ngame='26.3'\n",
                "a".repeat(40),
                "c".repeat(40),
            ),
            &[
                ("Cargo.toml", b"[package]\nname='light'\nversion='1.0.0'\n"),
                ("src/lib.rs", b""),
            ],
        )
        .unwrap();
        fs::write(packages.join("light.smod"), archive).unwrap();
        let manifest = sunrise_launcher::browser::lock_home(&home).unwrap();
        let identity = Identity::of(&home).unwrap();
        let data = root.join("launcher");
        fs::create_dir_all(&data).unwrap();
        let settings_path = data.join("settings.json");
        let settings_before = br#"{"native_consent_revision":0,"unrelated":1}"#;
        fs::write(&settings_path, settings_before).unwrap();
        let settings_gate = fs::File::options()
            .write(true)
            .read(true)
            .create_new(true)
            .open(data.join(".settings.json.lock"))
            .unwrap();
        settings_gate.lock().unwrap();
        let list = home.join("sunrise-mods.txt");
        let list_before = b"mod.light:false\nmod.other:true\n";
        fs::write(&list, list_before).unwrap();
        let (returned, wait_return) = channel();
        let (done, wait_done) = channel();
        let selected = home.clone();
        let caller = std::thread::spawn(move || {
            let question = || {
                Some(PendingConsent::Toggle(Box::new(Toggle {
                    entry: Entry {
                        source: SourceId::Sunrise,
                        project: "light".into(),
                        slug: "light".into(),
                        title: "Light".into(),
                        kind: Kind::Mod,
                        version: "1".into(),
                        number: "1".into(),
                        file: "sunrise-native-mods/light.smod".into(),
                        checksum: String::new(),
                        page: String::new(),
                        dependency: false,
                        verdict: None,
                        install_hold: None,
                    },
                    home: selected.clone(),
                    identity,
                })))
            };
            let mut settings = backend::Settings::load();
            for _ in ["Cancel", "Escape"] {
                let mut pending = question();
                assert!(native_consent::resolve(
                    &mut pending,
                    false,
                    true,
                    &mut settings,
                    &selected
                )
                .unwrap()
                .is_none());
                assert!(pending.is_none());
            }
            let mut pending = question();
            let action =
                native_consent::resolve(&mut pending, true, true, &mut settings, &selected)
                    .unwrap();
            assert!(pending.is_none());
            assert!(settings.native_mods_allowed_at.is_none());
            let Some(PendingConsent::Toggle(toggle)) = action else {
                panic!("Allow must dispatch this Toggle to the On worker");
            };
            let id = sunrise::mod_id(&toggle.entry).unwrap().to_owned();
            dispatch_enable(
                toggle.home,
                toggle.identity,
                vec![id],
                true,
                move |result, allowed_at| {
                    done.send((result, allowed_at)).unwrap();
                },
            );
            returned.send(()).unwrap();
        });
        wait_return
            .recv_timeout(Duration::from_secs(5))
            .expect("the consent handler must not wait on either lock");
        caller.join().unwrap();
        assert_eq!(fs::read(&list).unwrap(), list_before);
        assert_eq!(fs::read(&settings_path).unwrap(), settings_before);
        drop(manifest);
        assert!(matches!(
            wait_done.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        assert!(!mod_list::is_enabled(&home, "light"));
        drop(settings_gate);
        let (result, allowed_at) = wait_done.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(result.unwrap().count, 1);
        assert!(allowed_at.is_some());
        let written: serde_json::Value =
            serde_json::from_slice(&fs::read(settings_path).unwrap()).unwrap();
        assert_eq!(written["native_mods_allowed_at"].as_u64(), allowed_at);
        assert_eq!(written["native_consent_revision"], 1);
        assert_eq!(written["unrelated"], 1);
        assert!(mod_list::is_enabled(&home, "light"));
        assert!(mod_list::is_enabled(&home, "other"));
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
