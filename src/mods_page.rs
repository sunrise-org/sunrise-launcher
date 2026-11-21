//! The Mods page's state and work: Sunrise's registry mods, modpacks and shader packs, and
//! resource and data packs from Modrinth, installed with their checksum checked, Sunrise mods
//! turned on and off, and what was installed updated or removed (`sunrise_launcher::browser`).

use super::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use sunrise_launcher::browser::{installed, sunrise};
use sunrise_launcher::{native_status, profile_txn::Identity};

pub(crate) mod install;

pub(super) enum Event {
    Registry(install::Worker),
    /// What the automatic updates did (`sunrise::auto`).
    AutoUpdated(Vec<String>),
    /// A project page's part, an image or a filter's values (`mods_project`).
    Loaded(super::mods_project::Loaded),
}

pub(super) struct Mods {
    focus: FocusHandle,
    pub(super) focused: bool,
    results_list: cull::Cull,
    installed_list: cull::Cull,
    pub(super) report_list: cull::Cull,
    pub(super) installed: Vec<installed::Entry>,
    /// The registry native mods' versions the player picked on their Versions tab, by id
    /// (`sunrise::chosen`), read with `installed`.
    pub(super) chosen: std::collections::BTreeMap<String, String>,
    /// The game folder the browser installs into and lists: the selected instance's.
    home: ModsHome,
    pub(crate) installs: install::Installs,
    pub(super) message: Option<Result<String, String>>,
    /// The selected instance's native build state, the mods it would build again
    /// (`native_status::unbuilt`), and the `native_status::fingerprint` they were read for: read
    /// again only when that changes.
    pub(super) native_state:
        std::cell::RefCell<Option<(String, native_status::State, Vec<String>)>>,
    /// The instance and the mods whose compile failed in the last stop: their chips say so until
    /// the next build attempt, whatever the player chose.
    pub(super) unbuilt: Option<(String, Vec<String>)>,
    /// The project page, filters and images.
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

    fn begin_build_consent(&mut self, selected: PathBuf) -> Arc<PathBuf> {
        self.follow_home(Some(selected));
        Arc::new(self.current())
    }
}

impl Mods {
    pub(super) fn reset_lists(&self) {
        self.results_list.reset();
        self.installed_list.reset();
        self.report_list.reset();
    }

    /// The game folder installs go into: the selected instance's, once the page has read it.
    /// Reads what the browser installed in the game folder again, with the versions picked.
    pub(super) fn reload_installed(&mut self) {
        let home = self.home();
        self.installed = installed::Installed::load(&home).entries;
        self.chosen = sunrise::chosen(&home);
    }

    /// Points the folder installs and toggles go into at `home`, the selected instance's folder,
    /// without reading or writing anything: the Mods screens (`mods`) call it so
    /// that a native-code consent answered later continues in the folder they show.
    pub(super) fn follow_home(&mut self, home: Option<PathBuf>) {
        self.home.follow_home(home);
    }

    pub(super) fn home(&self) -> PathBuf {
        self.home.current()
    }

    pub(super) fn begin_build_consent(&mut self, selected: PathBuf) -> Arc<PathBuf> {
        self.home.begin_build_consent(selected)
    }

    pub(super) fn new(cx: &mut App) -> Self {
        let (tx, rx) = crate::wake::channel();
        Self {
            focus: cx.focus_handle().tab_index(0).tab_stop(true),
            focused: false,
            results_list: cull::Cull::default(),
            installed_list: cull::Cull::default(),
            report_list: cull::Cull::default(),
            installed: Vec::new(),
            chosen: Default::default(),
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

    pub(super) fn note_focus(&mut self, window: &Window) {
        self.focused = self.focus.is_focused(window);
    }
}

impl Launcher {
    /// Called with the launcher's other work: what the background sent.
    pub(super) fn mods_tick(&mut self) -> bool {
        let mut changed = false;
        let mut loaded = Vec::new();
        let mut done = false;
        while let Ok(event) = self.mods.rx.try_recv() {
            changed = true;
            let m = &mut self.mods;
            match event {
                Event::Registry(install::Worker::Native(question, reply)) => {
                    self.reset_install_question_sheet();
                    self.pending_consent = Some(PendingConsent::Registry(question, reply));
                }
                Event::Registry(event) => {
                    if m.installs.handle(event) {
                        done = true;
                        m.reload_installed();
                        m.installed_list.reset();
                    } else if m.installs.question.is_some() {
                        self.reset_install_question_sheet();
                    }
                }
                Event::Loaded(l) => loaded.push(l),
                Event::AutoUpdated(lines) => {
                    if !lines.is_empty() {
                        m.message = Some(Ok(lines.join("\n")));
                        m.reload_installed();
                        m.installed_list.reset();
                    }
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
        self.start_registry_queue() || changed
    }

    /// Turns a Sunrise mod on after the worker admits its profile. A consent answer
    /// can only act on the profile (and incarnation) that asked the question.
    pub(super) fn enable_registry_mod(
        &mut self,
        e: &installed::Entry,
        home: PathBuf,
        approved: Option<Identity>,
    ) {
        let Some(id) = sunrise::mod_id(e) else {
            return;
        };
        let index = sunrise::fetched_index();
        let title = index
            .as_ref()
            .and_then(|index| index.project(&e.project))
            .map_or(id, |project| &project.name);
        if let Some(revoked) = index
            .as_ref()
            .and_then(|index| index.revoked(&e.project, e.checksum.trim_start_matches("SHA-256:")))
        {
            self.mods.message = Some(Err(format!("{title} was taken down: {}", revoked.reason)));
            return;
        }
        if let Some(identity) = approved {
            self.enable_native_in(home, identity, id.into(), title.into());
        } else if native_status::is_native_file(&e.file)
            && self.settings.native_mods_allowed_at.is_none()
        {
            match Identity::of(&home) {
                Ok(identity) => {
                    self.pending_consent = Some(PendingConsent::Toggle(native_consent::Toggle {
                        entry: e.clone(),
                        home,
                        identity,
                    }));
                }
                Err(error) => self.mods.message = Some(Err(error)),
            }
        } else {
            self.enable_one_in(home, id.into(), title.into());
        }
    }
}

/// An instance whose registry projects may update.
pub(super) struct AutoTarget {
    pub(super) instance: instances::Instance,
    /// The Minecraft version its own build plays ([`Launcher::game_of`]).
    pub(super) game: String,
}

/// Updates the registry projects of `targets`: Sunrise's own instances only, never a modpack's
/// profile, whose pack pins its mods. Returns what happened for the page's notice.
pub(super) fn auto_update(targets: &[AutoTarget], enabled: bool) -> Vec<String> {
    let mut lines = Vec::new();
    for t in targets {
        let i = &t.instance;
        if i.id.starts_with("pack:") {
            continue;
        }
        let ctx = sunrise::auto::Context {
            in_pack: sunrise::auto::in_pack(&i.home),
            game: &t.game,
        };
        if let Some(report) = sunrise::auto::run(&i.home, &ctx, enabled, &|_, _| {}) {
            lines.extend(report.lines());
        }
    }
    lines
}

#[cfg(test)]
mod tests;
