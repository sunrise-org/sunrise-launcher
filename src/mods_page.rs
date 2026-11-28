//! The Mods page's state and work: Sunrise's registry mods, modpacks and shader packs, and
//! resource and data packs from Modrinth, installed with their checksum checked, Sunrise mods
//! turned on and off, and what was installed updated or removed (`sunrise_launcher::browser`).

use super::*;
use std::path::PathBuf;
use std::sync::Arc;
use sunrise_launcher::browser::{installed, sunrise};
use sunrise_launcher::native_status;

pub(crate) mod install;

pub(super) enum Event {
    Registry(install::Worker),
    /// What the automatic updates did (`sunrise::auto`).
    LegacyBackupNotice(String),
    /// An image loaded in the background (`mods_project`).
    Loaded(super::mods_project::Loaded),
}

pub(super) struct Mods {
    pub(super) discover_list: cull::Cull,
    pub(super) pack_discover_list: cull::Cull,
    /// The game folder the browser installs into and lists: the selected instance's.
    home: ModsHome,
    pub(crate) installs: install::Installs,
    pub(super) message: Option<Result<String, String>>,
    /// The selected instance's native build state, cached by `native_status::fingerprint`.
    pub(super) native_state: std::cell::RefCell<Option<(String, native_status::State)>>,
    /// The instance and the mods whose compile failed in the last stop: their chips say so until
    /// the next build attempt, whatever the player chose.
    pub(super) unbuilt: Option<(String, Vec<String>)>,
    /// The project images cache and fetch queue.
    pub(super) browse: super::mods_project::Browse,
    pub(super) tx: Sender<Event>,
    rx: Receiver<Event>,
}

/// The profile routed to the browser and its pending native-code actions. Kept separate from
/// the page's window state so both use the same binding even before the page has opened.
#[derive(Default)]
struct ModsHome(Option<PathBuf>);

impl ModsHome {
    fn follow_home(&mut self, home: Option<PathBuf>) {
        if home.is_some() && self.0 != home {
            self.0 = home;
        }
    }

    fn current(&self) -> PathBuf {
        self.0.clone().unwrap_or_else(backend::game_home)
    }
}

impl Mods {
    /// Points the folder installs and toggles go into at `home`, the selected instance's folder,
    /// without reading or writing anything: the Mods screens (`mods`) call it so
    /// that a native-code consent answered later continues in the folder they show.
    pub(super) fn follow_home(&mut self, home: Option<PathBuf>) {
        self.home.follow_home(home);
    }

    pub(super) fn home(&self) -> PathBuf {
        self.home.current()
    }

    pub(super) fn new() -> Self {
        let (tx, rx) = crate::wake::channel();
        Self {
            discover_list: cull::Cull::default(),
            pack_discover_list: cull::Cull::default(),
            home: ModsHome::default(),
            installs: install::Installs::default(),
            message: None,
            native_state: Default::default(),
            unbuilt: None,
            browse: Default::default(),
            tx,
            rx,
        }
    }
}

impl Launcher {
    /// Called with the launcher's other work: what the background sent.
    pub(super) fn mods_tick(&mut self) -> bool {
        let mut changed = false;
        let mut loaded = Vec::new();
        let mut github_feedback = None;
        let mut refused_here = None;
        let mut failed_here = None;
        let mut removed_here = None;
        let mut done = false;
        while let Ok(event) = self.mods.rx.try_recv() {
            changed = true;
            let m = &mut self.mods;
            match event {
                Event::Registry(install::Worker::Source(question, reply)) => {
                    self.receive_source_question(question, reply);
                }
                Event::Registry(event) => {
                    if let Some(feedback) = m.installs.github_feedback(&event) {
                        github_feedback = Some(feedback);
                    }
                    if let Some(refused) = m.installs.refused(&event) {
                        refused_here = Some(refused);
                    }
                    if let Some(error) = m.installs.mutation_failure(&event) {
                        m.message = Some(Err(error));
                    }
                    if let Some(failed) = m.installs.install_failure(&event) {
                        failed_here = Some(failed);
                    }
                    if let Some((home, ids)) = m.installs.removed(&event) {
                        removed_here = Some((home.to_owned(), ids.to_vec()));
                    }
                    if m.installs.handle(event) {
                        done = true;
                    } else if m.installs.question.is_some() || m.installs.world.is_some() {
                        self.reset_install_question_sheet();
                    }
                }
                Event::Loaded(l) => loaded.push(l),
                Event::LegacyBackupNotice(notice) => {
                    m.message = Some(Ok(notice));
                }
            }
        }
        if done {
            // A data pack put into a world shows in its list on Home.
            self.world_packs.reread();
        }
        for l in loaded {
            self.mods_loaded(l);
        }
        if done {
            if self.ui.sheet == Some(crate::sheets::Sheet::RegistryPlay) {
                self.ui.sheet = None;
            }
            if let Some((home, intent)) = self.mods.installs.take_play() {
                if self
                    .selected_instance()
                    .is_some_and(|i| i.home == home && i.id == intent.instance)
                {
                    self.resume_play(intent);
                }
            }
        }
        if let Some((home, ids)) = removed_here {
            self.mods_leave_removed(&home, &ids);
        }
        if let Some(feedback) = github_feedback {
            self.show_github_result(feedback);
        }
        // A failed install says why in a dialog, naming its profile when another one is shown.
        if let Some((profile, why)) = failed_here {
            let here = self.selected_instance().is_some_and(|i| {
                i.home == profile.home && i.id == profile.id && i.incarnation == profile.incarnation
            });
            let why = if here {
                why
            } else {
                format!(
                    "{}\n\n{why}",
                    trf(
                        "Installing to {profile} failed.",
                        &[("profile", &profile.name)]
                    )
                )
            };
            self.open_notice(why, None);
        }
        // The profile that asked hears why at once; another one finds it on the mod's page.
        if let Some((profile, refused)) = refused_here {
            if self.selected_instance().is_some_and(|i| {
                i.home == profile.home && i.id == profile.id && i.incarnation == profile.incarnation
            }) {
                self.open_refused(refused);
            }
        }
        self.start_registry_queue() || changed
    }

    /// Turns a Sunrise mod on after the worker admits its profile.
    pub(super) fn enable_registry_mod(&mut self, e: &installed::Entry, home: PathBuf) {
        let Some(id) = sunrise::mod_id(e) else {
            return;
        };
        let hash = e.checksum.trim_start_matches("SHA-256:");
        let catalog = sunrise::v2::known_index();
        let revoked = catalog.as_ref().and_then(|catalog| {
            catalog
                .index
                .revoked(&e.project, Some(hash))
                .or_else(|| catalog.index.revoked_by_sha256(hash))
        });
        if let Some(revoked) = revoked {
            self.mods.message = Some(Err(trf(
                "{mod} was taken down: {reason}",
                &[("mod", &e.title), ("reason", &revoked.reason)],
            )));
            return;
        }
        self.enable_one_in(home, id.into(), e.title.clone());
    }
}
