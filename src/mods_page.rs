//! The Mods page's state and work: Sunrise's registry mods, modpacks and shader packs, and
//! resource and data packs from Modrinth, installed with their checksum checked, Sunrise mods
//! turned on and off, and what was installed updated or removed (`sunrise_launcher::browser`).

use super::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use sunrise_launcher::browser::{
    self, installed, mod_list, sunrise, Kind as PackKind, Project, SourceId,
};
use sunrise_launcher::native_status;

pub(super) enum Event {
    Progress(String, f32),
    Done(Result<String, String>),
    /// What the automatic updates did (`sunrise::auto`).
    AutoUpdated(Result<String, String>),
    /// A project page's part, an image or a filter's values (`mods_project`).
    Loaded(super::mods_project::Loaded),
}

pub(super) struct Mods {
    focus: FocusHandle,
    pub(super) focused: bool,
    everything: bool,
    /// The Minecraft version Sunrise plays, read again with each search ([`browser::game`]).
    pub(super) game: String,
    results_list: cull::Cull,
    installed_list: cull::Cull,
    pub(super) report_list: cull::Cull,
    pub(super) installed: Vec<installed::Entry>,
    /// The registry native mods' versions the player picked on their Versions tab, by id
    /// (`sunrise::chosen`), read with `installed`.
    pub(super) chosen: std::collections::BTreeMap<String, String>,
    /// The build the page's Minecraft version was last worked out for (the selected instance's).
    game_for: Option<(Option<String>, Option<String>)>,
    /// Whether the start's auto-update ran: it waits for the release list.
    pub(super) auto_updated: bool,
    /// The game folder the browser installs into and lists: the selected instance's.
    home: ModsHome,
    /// Its worlds (folder, name), the last played first, and the one data packs go into.
    worlds: Vec<(PathBuf, String)>,
    world: Option<PathBuf>,
    /// The project being installed, and the line and fraction of its download.
    pub(super) busy: Option<(String, String, f32)>,
    pub(super) message: Option<Result<String, String>>,
    pub(super) updates: HashMap<(SourceId, String), String>,
    /// An install awaiting explicit native-code consent, before any download.
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
    /// The saved settings changed (a save, or its retry): the builds the search follows are worked out again.
    pub(super) fn settings_saved(&mut self) {
        self.game_for = None;
    }

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
            everything: false,
            game: browser::game(),
            results_list: cull::Cull::default(),
            installed_list: cull::Cull::default(),
            report_list: cull::Cull::default(),
            installed: Vec::new(),
            chosen: Default::default(),
            game_for: None,
            auto_updated: false,
            home: ModsHome::default(),
            worlds: Vec::new(),
            world: None,
            busy: None,
            message: None,
            native_state: Default::default(),
            unbuilt: None,
            updates: HashMap::new(),
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
                Event::Progress(line, f) => {
                    if let Some(busy) = &mut m.busy {
                        busy.1 = line;
                        busy.2 = f;
                    }
                }
                Event::Done(result) => {
                    done = true;
                    m.busy = None;
                    if let Ok(said) = &result {
                        crate::glass_toast::say(said.lines().next().unwrap_or_default().to_owned());
                    }
                    m.message = Some(result);
                    m.reload_installed();
                    m.installed_list.reset();
                }
                Event::Loaded(l) => loaded.push(l),
                Event::AutoUpdated(notice) => {
                    m.message = Some(notice);
                    m.reload_installed();
                    m.installed_list.reset();
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
        changed
    }

    /// At start: recover interrupted updates, then update every Sunrise instance's registry
    /// content when enabled ([`auto_update`]), reading the selected instance's list again after.
    pub(super) fn mods_auto_update(&mut self) {
        let enabled = self.settings.auto_updates_mods();
        let list: Vec<AutoTarget> = self
            .instances
            .list
            .iter()
            .map(|i| AutoTarget {
                instance: i.clone(),
                game: self.game_of(i),
            })
            .collect();
        let tx = self.mods.tx.clone();
        std::thread::spawn(move || {
            if let Some(notice) = auto_update(&list, enabled) {
                let _ = tx.send(Event::AutoUpdated(notice));
            }
        });
    }

    /// Turns a Sunrise mod the browser installed on or off, as the game's Options > Mods does.
    pub(super) fn mods_switch(&mut self, e: &installed::Entry) {
        let home = self.mods.home();
        // An update holds the folder: the window doesn't wait for it.
        let Some(_lock) = browser::try_lock_home(&home) else {
            self.mods.message = Some(Err(tr("Wait for the current operation to finish.").into()));
            return;
        };
        let Some(id) = sunrise::mod_id(e) else {
            return;
        };
        let on = !mod_list::is_enabled(&home, id);
        if on
            && native_status::is_native_file(&e.file)
            && self.settings.native_mods_allowed_at.is_none()
        {
            self.pending_consent = Some(PendingConsent::Toggle(e.clone()));
            return;
        }
        let index = sunrise::fetched_index();
        let title = index
            .as_ref()
            .and_then(|index| index.project(&e.project))
            .map_or(id, |project| &project.name);
        let revoked = index.as_ref().and_then(|index| {
            let hash = e.checksum.trim_start_matches("SHA-256:");
            index.revoked(&e.project, hash)
        });
        self.mods.message = Some(match revoked.filter(|_| on) {
            Some(revoked) => Err(format!("{title} was taken down: {}", revoked.reason)),
            None => mod_list::set_enabled(&home, id, on).map(|()| {
                format!(
                    "{title} is {}. The game reads this when it starts.",
                    if on { "on" } else { "off" }
                )
            }),
        });
    }

    /// Installs (or updates) a project on a thread: a pack into its folder, a modpack into
    /// Modpacks. `known` is the project when the page has it; else it is asked for first. `shown`
    /// is the name the page or card that asks shows for it, which the native-code question
    /// repeats; the id stays what is installed.
    pub(super) fn mods_install(
        &mut self,
        source: SourceId,
        id: String,
        known: Option<Project>,
        shown: Option<String>,
    ) {
        if self.mods.busy.is_some() {
            return;
        }
        // The first native mod: the trust notice, before anything is installed.
        let first_native = source == SourceId::Sunrise
            && self.settings.native_mods_allowed_at.is_none()
            && sunrise::is_native_mod(&id);
        if first_native {
            self.pending_consent = Some(PendingConsent::Install(source, id, known, shown));
            self.content_scroll
                .handle
                .set_offset(gpui::point(px(0.), px(0.)));
            return;
        }
        self.mods.message = None;
        self.mods.busy = Some((id.clone(), format!("Asking {}…", source.label()), 0.));
        self.mods.updates.remove(&(source, id.clone()));
        let everything =
            self.mods.everything || self.mods.browse.filters.other_version(&self.mods.game);
        let tx = self.mods.tx.clone();
        let home = self.mods.home();
        let game = self.mods_game();
        let instance = self
            .selected_instance()
            .map_or_else(|| "Sunrise".to_string(), |i| i.name.clone());
        let world = self.mods.world.clone().and_then(|path| {
            let (_, name) = self.mods.worlds.iter().find(|(p, _)| *p == path)?;
            Some((path, name.clone()))
        });
        std::thread::spawn(move || {
            let last = std::cell::Cell::new(-1);
            let progress = |line: &str, f: f32| {
                let pct = (f * 100.) as i32;
                if last.replace(pct) != pct {
                    let _ = tx.send(Event::Progress(line.to_string(), f));
                }
            };
            let result = (|| -> Result<String, String> {
                let source = browser::source(source);
                let p = match known {
                    Some(p) => p,
                    None => source.project(&id)?,
                };
                if p.kind == PackKind::DataPack {
                    let (world, name) = world.ok_or_else(|| {
                        tr("Pick the world the data pack goes into first.").to_string()
                    })?;
                    let done = installed::install_datapack(
                        &*source,
                        &p,
                        everything,
                        &world,
                        &game,
                        &|f| progress(&p.title, f),
                    )?;
                    return Ok(trf(
                        "Put {pack} {version} in {world}'s data packs, checksum checked: the \
                             game turns it on when the world opens. {coverage}",
                        &[
                            ("pack", &p.title),
                            ("version", &done.number),
                            ("world", &name),
                            ("coverage", &done.coverage.summary()),
                        ],
                    ));
                }
                if p.kind == PackKind::Modpack && p.source == SourceId::Sunrise {
                    let pack = sunrise::pack::install(&p.id, &progress)?;
                    if let Some(url) = &p.icon {
                        sunrise_launcher::instances::adopt_icon(
                            &sunrise::pack::home(&pack.work),
                            url,
                        );
                    }
                    return Ok(trf(
                        "Installed {pack} {version} as its own profile with {mods}, each \
                             checked. Play it from the list above.",
                        &[
                            ("pack", &pack.name),
                            ("version", &pack.version),
                            ("mods", &pack.mods.join(", ")),
                        ],
                    ));
                }
                let done = installed::install(&*source, &p, everything, &home, &game, &progress)?;
                let summary = match (p.kind, done.len()) {
                    (PackKind::Mod, 1) => tr("Installed 1 mod").to_owned(),
                    (PackKind::Mod, count) => {
                        trf("Installed {count} mods", &[("count", &count.to_string())])
                    }
                    (_, 1) => tr("Installed 1 pack").to_owned(),
                    (_, count) => trf("Installed {count} packs", &[("count", &count.to_string())]),
                };
                let mut details = vec![summary, String::new()];
                if done.iter().any(|e| native_status::is_native_file(&e.file)) {
                    details.push(trf(
                        "Installed into {instance}. Build to play with these mods.",
                        &[("instance", &instance)],
                    ));
                } else if !(p.source == SourceId::Sunrise && p.kind == PackKind::Mod) {
                    details.push(tr("Checksums checked. Turn packs on in the game: Options > Resource Packs.").into());
                }
                details.extend(done.iter().map(|e| format!("{} {}", e.title, e.number)));
                Ok(details.join("\n"))
            })();
            let _ = tx.send(Event::Done(result));
        });
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
pub(super) fn auto_update(targets: &[AutoTarget], enabled: bool) -> Option<Result<String, String>> {
    let mut reports = sunrise::auto::Report::default();
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
            reports.updated.extend(report.updated);
            reports.held.extend(report.held);
            reports.failed.extend(report.failed);
        }
    }
    reports.notice()
}

#[cfg(test)]
mod tests;
