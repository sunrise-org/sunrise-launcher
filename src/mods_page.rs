//! The Mods page's state and work: Sunrise's registry mods, modpacks and shader packs, and
//! resource and data packs from Modrinth, installed with their checksum checked, Sunrise mods
//! turned on and off, and what was installed updated or removed (`sunrise_launcher::browser`).

use super::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use sunrise_launcher::browser::{self, installed, sunrise, Kind as PackKind, Project, SourceId};

mod queue;
/// An install that failed, as its card says it.
pub(crate) struct CardError {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) line: String,
    pub(crate) details: String,
    /// The mod's own build failed, not the install: the sheet says "couldn't be built".
    pub(crate) built: bool,
    /// Details is open.
    pub(crate) open: bool,
}

/// What an install completed, including dependencies it installed or updated.
pub(super) struct Finished {
    said: Option<String>,
    /// What came in with the mod (its dependencies), by title, for "Also installed".
    also: Vec<String>,
    /// The dependencies it moved to another version: title, before, after.
    updated: Vec<(String, String, String)>,
}

impl Finished {
    fn said(said: String) -> Self {
        Self {
            said: Some(said),
            also: Vec::new(),
            updated: Vec::new(),
        }
    }

    fn registry_install(
        done: &[installed::Entry],
        before: &[installed::Entry],
        asked: &str,
    ) -> Self {
        Self {
            said: None,
            also: done
                .iter()
                .filter(|e| e.project != asked)
                .filter(|e| !before.iter().any(|old| old.project == e.project))
                .map(|e| e.title.clone())
                .collect(),
            updated: done
                .iter()
                .filter_map(|e| {
                    let old = before.iter().find(|old| old.project == e.project)?;
                    (e.project != asked && old.number != e.number)
                        .then(|| (e.title.clone(), old.number.clone(), e.number.clone()))
                })
                .collect(),
        }
    }
}

pub(crate) struct PreparedInstall {
    target: queue::Target,
    label: String,
    proposal: InstallProposal,
}

enum InstallProposal {
    Registry(sunrise::RegistryInstall, Finished),
    Pack(sunrise::pack::PackInstall, Option<String>),
}

impl PreparedInstall {
    pub(crate) fn question(&self) -> Option<&sunrise_launcher::worlds::SwitchQuestion> {
        match &self.proposal {
            InstallProposal::Registry(plan, _) => Some(plan.question()),
            InstallProposal::Pack(plan, _) => plan.question(),
        }
    }

    fn finish_or_ask(self) -> Result<Finished, Stop> {
        if self
            .question()
            .is_some_and(sunrise_launcher::worlds::SwitchQuestion::needed)
        {
            return Err(Stop::ReadyInstall(Box::new(self)));
        }
        self.proposal.commit(false).map_err(Into::into)
    }
}

impl InstallProposal {
    fn commit(self, approved: bool) -> Result<Finished, installed::InstallError> {
        match self {
            Self::Registry(plan, finished) => {
                plan.commit(approved)?;
                Ok(finished)
            }
            Self::Pack(plan, icon) => {
                let pack = plan.commit(approved)?;
                if let Some(url) = icon {
                    sunrise_launcher::instances::adopt_icon(&sunrise::pack::home(&pack.work), &url);
                }
                Ok(Finished::said(trf(
                    "Installed {pack} {version} as its own profile with {mods}, each checked. Play it from the list above.",
                    &[("pack", &pack.name), ("version", &pack.version), ("mods", &pack.mods.join(", "))],
                )))
            }
        }
    }
}

pub(super) struct ModEditRequest {
    pub(super) instance: sunrise_launcher::instances::Instance,
    pub(super) edit: sunrise::txn::ModEdit,
    pub(super) name: String,
    pub(super) success: String,
}

pub(crate) struct PreparedEdit {
    name: String,
    success: String,
    pub(crate) plan: sunrise::txn::PreparedModEdit,
}

pub(super) enum Event {
    Progress(String, f32),
    ReadyEdit(Box<PreparedEdit>),
    EditFinished(Result<Option<String>, String>),
    Done(Result<Finished, installed::InstallError>),
    /// The install found native code the plan didn't know of before the player had allowed
    /// it: the question to ask, whose answer starts the same install again.
    NeedsConsent(Box<PendingConsent>, Box<queue::Request>),
    ReadyInstall(Box<PreparedInstall>),
    InstallCancelled,
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
    /// Whether the start's recovery ran: it waits for the release list.
    pub(super) recovered: bool,
    /// The game folder the browser installs into and lists: the selected instance's.
    home: ModsHome,
    /// Its worlds (folder, name), the last played first, and the one data packs go into.
    worlds: Vec<(PathBuf, String)>,
    world: Option<PathBuf>,
    /// The project being installed, and the line and fraction of its download.
    pub(super) busy: Option<(String, String, f32)>,
    pub(super) message: Option<Result<String, String>>,
    /// What waits for the install and build slot.
    queue: queue::Queue,
    /// Live batches wait here between publishing and their follow-up build, by profile.
    pub(super) install_owners: HashMap<String, Arc<sunrise::txn::Lease>>,
    /// The start's automatic update runs: Play and installs wait for it.
    /// The install that asked the native-code question, whole (its profile, what it replaces,
    /// its collection): the answer starts it again, and nothing behind it starts meanwhile.
    pub(super) consent_request: Option<queue::Request>,
    /// A build switch whose mods are being moved first: the profile, and the pin it will get.
    /// The profile the install that runs was asked for.
    running: Option<queue::Target>,
    /// The question a switch or update asks before it removes mods saved worlds use. Nothing
    /// is removed until it is answered, and the answer defaults to Stay.
    pub(crate) world_ask: Option<WorldAsk>,
    /// What came in with each mod installed this run, by id (the card's "Also installed").
    pub(crate) also: HashMap<String, Vec<String>>,
    /// The dependencies each moved to another version (the card's "Also updated").
    pub(crate) updated: HashMap<String, Vec<(String, String, String)>>,
    /// Why the last install failed, on its card: one plain line, the whole text behind Details.
    pub(crate) card_error: Option<CardError>,
    pub(super) updates: HashMap<(SourceId, String), String>,
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

    /// Whether the install of mod `id` waits for its turn.
    pub(crate) fn waiting(&self, id: &str) -> bool {
        self.queue.waits(id)
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
            recovered: false,
            home: ModsHome::default(),
            worlds: Vec::new(),
            world: None,
            busy: None,
            message: None,
            queue: Default::default(),
            install_owners: HashMap::new(),
            running: None,
            consent_request: None,
            world_ask: None,
            also: HashMap::new(),
            updated: HashMap::new(),
            card_error: None,
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
                Event::NeedsConsent(ask, request) => {
                    m.busy = None;
                    m.running = None;
                    // The exact request waits, whole: the answer starts it again, and the
                    // queue behind it does not move until then (`consent_request`).
                    m.consent_request = Some(*request);
                    self.pending_consent = Some(*ask);
                    self.content_scroll
                        .handle
                        .set_offset(gpui::point(px(0.), px(0.)));
                }
                Event::ReadyInstall(prepared) => {
                    m.busy = None;
                    m.running = None;
                    m.world_ask = Some(WorldAsk {
                        then: Then::PreparedInstall(prepared),
                    });
                }
                Event::ReadyEdit(prepared) => {
                    m.busy = None;
                    m.world_ask = Some(WorldAsk {
                        then: Then::PreparedEdit(prepared),
                    });
                }
                Event::EditFinished(result) => {
                    m.busy = None;
                    m.message = result.transpose();
                    m.reload_installed();
                    m.installed_list.reset();
                }
                Event::InstallCancelled => {
                    m.busy = None;
                    if let Some(target) = m.running.take() {
                        m.install_owners.remove(&target.instance);
                    }
                }
                Event::Done(result) => {
                    done = true;
                    let id = m.busy.take().map(|(id, ..)| id);
                    if let Some(instance) = m.running.take().map(|t| t.instance) {
                        if m.install_owners
                            .get(&instance)
                            .is_some_and(|lease| !lease.outstanding())
                        {
                            m.install_owners.remove(&instance);
                        }
                    }
                    m.card_error = match (&result, &id) {
                        (Err(why), Some(id)) => Some(CardError {
                            title: sunrise::known_index()
                                .and_then(|index| index.project(id).map(|p| p.name.clone()))
                                .unwrap_or_else(|| id.clone()),
                            id: id.clone(),
                            line: crate::mods::install_state::failure_line(why),
                            details: why.to_string(),
                            built: false,
                            open: false,
                        }),
                        _ => None,
                    };
                    m.message = match result {
                        Ok(finished) => {
                            if let Some(id) = id {
                                m.also.insert(id.clone(), finished.also);
                                m.updated.insert(id, finished.updated);
                            }
                            finished.said.map(|said| {
                                crate::glass_toast::say(said.clone());
                                Ok(said)
                            })
                        }
                        Err(why) => Some(Err(why.to_string())),
                    };
                    m.reload_installed();
                    m.installed_list.reset();
                }
                Event::Loaded(l) => loaded.push(l),
            }
        }
        if done {
            // A data pack put into a world shows in its list on Home.
            self.world_packs.reread();
        }
        let locked = matches!(
            self.mods_lock(),
            Some(crate::mods::Lock::GameRunning | crate::mods::Lock::Preparing)
        );
        // A question that is about to start an install again holds the queue; one that was
        // closed or left (another profile picked) lets go of its request.
        if self.pending_consent.is_none() {
            if let Some(request) = self.mods.consent_request.take() {
                let instance = &request.target().instance;
                if self
                    .mods
                    .install_owners
                    .get(instance)
                    .is_some_and(|lease| !lease.outstanding())
                {
                    self.mods.install_owners.remove(instance);
                }
            }
        }
        if let Some(request) = self.mods.queue.next(
            self.mods.busy.is_some()
                || self.native_step.is_some()
                || locked
                || self.pending_consent.is_some()
                || self.mods.world_ask.is_some(),
        ) {
            self.start_install(request);
            changed = true;
        }
        for l in loaded {
            self.mods_loaded(l);
        }
        changed
    }

    /// At start: an install or build an earlier run left unfinished is undone, in every
    /// profile (the one record of each says how: `txn`). Mods never update by themselves.
    pub(super) fn mods_recover(&mut self) {
        let homes: Vec<_> = self.instances.list.iter().map(|i| i.home.clone()).collect();
        std::thread::spawn(move || {
            for home in &homes {
                sunrise::txn::recover(home);
            }
        });
    }

    /// Runs the exact action captured by the Installed row, including after native consent.
    pub(super) fn start_mod_edit(&mut self, request: ModEditRequest) {
        let ModEditRequest {
            instance,
            edit,
            name,
            success,
        } = request;
        self.mods.busy = Some((name.clone(), tr("Working…").into(), 0.));
        self.mods.message = None;
        let tx = self.mods.tx.clone();
        let launcher = self.tx.clone();
        std::thread::spawn(move || {
            let prepared = sunrise::txn::Lease::admit(&instance.home).and_then(|owner| {
                sunrise::txn::prepare_mod_edit(
                    Arc::new(owner),
                    &instance,
                    edit,
                    &|line, f| {
                        let _ = tx.send(Event::Progress(line.into(), f));
                    },
                    &|inventory| crate::native_build::ask_registry(&launcher, inventory),
                )
            });
            let event = match prepared {
                Ok(plan) if plan.question().needed() => Event::ReadyEdit(Box::new(PreparedEdit {
                    name,
                    success,
                    plan,
                })),
                Ok(plan) => Event::EditFinished(plan.commit(false).map(|()| Some(success))),
                Err(error) => Event::EditFinished(Err(error)),
            };
            let _ = tx.send(event);
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
        let request = queue::Request::One {
            target: self.queue_target(),
            source,
            id,
            known: known.map(Box::new),
            shown,
        };
        self.submit_install(request);
    }

    /// The native-code question answered the request already at the head of the queue.
    pub(crate) fn resume_install(&mut self) {
        if let Some(request) = self.mods.consent_request.take() {
            self.mods.queue.resume(request);
        }
    }

    /// The profile an install asked for now is bound to.
    fn queue_target(&self) -> queue::Target {
        queue::Target {
            instance: self
                .selected_instance()
                .map(|i| i.id.clone())
                .unwrap_or_default(),
            home: self.mods.home(),
            game: self.mods_game(),
        }
    }

    /// Behind an install or a build, and behind what already waits: one at a time, in order.
    /// What the queue hands out later starts through [`Self::start_install`] and is not asked
    /// again.
    fn submit_install(&mut self, request: queue::Request) {
        let busy = self.mods.busy.is_some()
            || self.native_step.is_some()
            || self.pending_consent.is_some()
            || self.mods.world_ask.is_some();
        if let Some(request) = self.mods.queue.submit(request, busy) {
            self.start_install(request);
        }
    }

    fn start_install(&mut self, request: queue::Request) {
        let target = request.target();
        let (lease, fresh) = match self.mods.install_owners.get(&target.instance) {
            Some(lease) => (lease.clone(), false),
            None => match sunrise::txn::Lease::acquire(&target.home) {
                Ok(lease) => {
                    let lease = Arc::new(lease);
                    self.mods
                        .install_owners
                        .insert(target.instance.clone(), lease.clone());
                    (lease, true)
                }
                Err(error) => {
                    self.mods.message = Some(Err(error));
                    return;
                }
            },
        };
        match request {
            queue::Request::One {
                target,
                source,
                id,
                known,
                shown,
            } => {
                self.start_install_one(target, (lease, fresh), source, id, known.map(|p| *p), shown)
            }
            queue::Request::Many(target, ids, shown) => {
                self.start_install_many(target, (lease, fresh), ids, shown)
            }
        }
    }

    fn start_install_one(
        &mut self,
        target: queue::Target,
        (lease, fresh): (Arc<sunrise::txn::Lease>, bool),
        source: SourceId,
        id: String,
        known: Option<Project>,
        shown: Option<String>,
    ) {
        self.mods.message = None;
        self.mods.card_error = None;
        self.mods.busy = Some((
            id.clone(),
            trf("Asking {source}…", &[("source", source.label())]),
            0.,
        ));
        self.mods.updates.remove(&(source, id.clone()));
        let everything =
            self.mods.everything || self.mods.browse.filters.other_version(&self.mods.game);
        let tx = self.mods.tx.clone();
        let launcher = self.tx.clone();
        let queue::Target { home, game, .. } = target.clone();
        self.mods.running = Some(target.clone());
        let retry = known.clone();
        let instance = self
            .instances
            .list
            .iter()
            .find(|i| i.id == target.instance)
            .cloned();
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
            let result = (|| -> Result<Finished, Stop> {
                if fresh {
                    lease.recover_abandoned()?;
                }
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
                    return Ok(Finished::said(trf(
                        "Put {pack} {version} in {world}'s data packs, checksum checked: the \
                             game turns it on when the world opens. {coverage}",
                        &[
                            ("pack", &p.title),
                            ("version", &done.number),
                            ("world", &name),
                            ("coverage", &done.coverage.summary()),
                        ],
                    )));
                }
                if p.kind == PackKind::Modpack && p.source == SourceId::Sunrise {
                    let plan = sunrise::pack::prepare(&p.id, &progress, &|inventory| {
                        crate::native_build::ask_registry(&launcher, inventory)
                    })?;
                    return PreparedInstall {
                        target: target.clone(),
                        label: id.clone(),
                        proposal: InstallProposal::Pack(plan, p.icon),
                    }
                    .finish_or_ask();
                }
                if p.source == SourceId::Sunrise && p.kind == PackKind::Mod {
                    let instance = instance
                        .as_ref()
                        .ok_or_else(backend::no_version_installed)?;
                    let before = installed::Installed::load(&home).entries;
                    let plan = sunrise::prepare_registry_install(
                        lease.clone(),
                        instance,
                        std::slice::from_ref(&p.id),
                        &progress,
                        &|inventory| crate::native_build::ask_registry(&launcher, inventory),
                    )?;
                    let finished = Finished::registry_install(plan.entries(), &before, &p.id);
                    return PreparedInstall {
                        target: target.clone(),
                        label: id.clone(),
                        proposal: InstallProposal::Registry(plan, finished),
                    }
                    .finish_or_ask();
                }
                let done = lease.install(&*source, &p, everything, &game, &progress)?;
                let names: Vec<String> = done
                    .iter()
                    .map(|e| format!("{} {}", e.title, e.number))
                    .collect();
                Ok(Finished::said(trf(
                    "Installed {mods}, checksums checked. Turn it on in the game: Options \
                         > Resource Packs.",
                    &[("mods", &names.join(", "))],
                )))
            })();
            drop(lease);
            let _ = tx.send(match result {
                Err(Stop::Consent) => Event::NeedsConsent(
                    Box::new(PendingConsent::Install(
                        shown
                            .clone()
                            .or_else(|| retry.as_ref().map(|p| p.title.clone()))
                            .unwrap_or_else(|| id.clone()),
                    )),
                    Box::new(queue::Request::One {
                        target,
                        source,
                        id,
                        known: retry.map(Box::new),
                        shown,
                    }),
                ),
                Err(Stop::ReadyInstall(prepared)) => Event::ReadyInstall(prepared),
                Err(Stop::Failed(why)) => Event::Done(Err(why)),
                Ok(finished) => Event::Done(Ok(finished)),
            });
        });
    }
}

/// Why an install's thread stopped: it failed, or it needs the native-code question first.
enum Stop {
    Failed(installed::InstallError),
    ReadyInstall(Box<PreparedInstall>),
    Consent,
}

impl From<String> for Stop {
    fn from(why: String) -> Self {
        Self::Failed(why.into())
    }
}

impl From<installed::InstallError> for Stop {
    fn from(error: installed::InstallError) -> Self {
        match error {
            installed::InstallError::NeedsNativeConsent { .. } => Self::Consent,
            other => Self::Failed(other),
        }
    }
}

impl Launcher {
    /// Accept or drop the exact prepared candidate without rerunning the operation.
    pub(crate) fn answer_world_question(&mut self, answer: sunrise_launcher::worlds::Answer) {
        let Some(ask) = self.mods.world_ask.take() else {
            return;
        };
        let tx = self.mods.tx.clone();
        let stay = answer == sunrise_launcher::worlds::Answer::Stay;
        match ask.then {
            Then::PreparedInstall(prepared) => {
                let PreparedInstall {
                    target,
                    label,
                    proposal,
                } = *prepared;
                self.mods.busy = Some((label, tr("Installing…").into(), 0.99));
                self.mods.running = Some(target);
                std::thread::spawn(move || {
                    let event = if stay {
                        drop(proposal);
                        Event::InstallCancelled
                    } else {
                        Event::Done(proposal.commit(true))
                    };
                    let _ = tx.send(event);
                });
            }
            Then::PreparedEdit(prepared) => {
                let PreparedEdit {
                    name,
                    success,
                    plan,
                } = *prepared;
                self.mods.busy = Some((name, tr("Working…").into(), 0.99));
                std::thread::spawn(move || {
                    let result = if stay {
                        drop(plan);
                        Ok(None)
                    } else {
                        plan.commit(true).map(|()| Some(success))
                    };
                    let _ = tx.send(Event::EditFinished(result));
                });
            }
        }
    }

    pub(super) fn mods_install_many(&mut self, ids: Vec<String>, shown: String) {
        if ids.is_empty() {
            return;
        }
        let request = queue::Request::Many(self.queue_target(), ids, shown);
        self.submit_install(request);
    }

    fn start_install_many(
        &mut self,
        target: queue::Target,
        (lease, fresh): (Arc<sunrise::txn::Lease>, bool),
        ids: Vec<String>,
        shown: String,
    ) {
        self.mods.message = None;
        self.mods.busy = Some((
            shown.clone(),
            tr("Asking the Sunrise registry…").to_string(),
            0.,
        ));
        let tx = self.mods.tx.clone();
        let launcher = self.tx.clone();
        let home = target.home.clone();
        self.mods.running = Some(target.clone());
        let instance = self
            .instances
            .list
            .iter()
            .find(|i| i.id == target.instance)
            .cloned();
        std::thread::spawn(move || {
            let result = (|| -> Result<Finished, Stop> {
                if fresh {
                    lease.recover_abandoned()?;
                }
                let progress = |line: &str, f: f32| {
                    let _ = tx.send(Event::Progress(line.to_string(), f));
                };
                let instance = instance
                    .as_ref()
                    .ok_or_else(backend::no_version_installed)?;
                let before = installed::Installed::load(&home).entries;
                let plan = sunrise::prepare_registry_install(
                    lease.clone(),
                    instance,
                    &ids,
                    &progress,
                    &|inventory| crate::native_build::ask_registry(&launcher, inventory),
                )?;
                let finished = Finished::registry_install(plan.entries(), &before, "");
                PreparedInstall {
                    target: target.clone(),
                    label: shown.clone(),
                    proposal: InstallProposal::Registry(plan, finished),
                }
                .finish_or_ask()
            })();
            drop(lease);
            let _ = tx.send(match result {
                Err(Stop::ReadyInstall(prepared)) => Event::ReadyInstall(prepared),
                Err(Stop::Consent) => Event::NeedsConsent(
                    Box::new(PendingConsent::Install(shown.clone())),
                    Box::new(queue::Request::Many(target, ids, shown)),
                ),
                Err(Stop::Failed(why)) => Event::Done(Err(why)),
                Ok(finished) => Event::Done(Ok(finished)),
            });
        });
    }
}

/// What goes on once the player answers Switch anyway to a world question.
pub(crate) enum Then {
    /// The exact prepared set, image and locked world evidence to approve once.
    PreparedInstall(Box<PreparedInstall>),
    PreparedEdit(Box<PreparedEdit>),
}

/// A switch, update or install that waits for the player's answer: it would remove mods that
/// saved worlds use. Staying is the default; Switch anyway goes on with those mods allowed.
pub(crate) struct WorldAsk {
    pub(crate) then: Then,
}

#[cfg(test)]
mod tests;
