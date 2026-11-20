//! The Servers page: create a server in one dialog, then start, stop and watch it, give friends its
//! address, manage who may join, change its settings, back it up and update it, all with buttons. Every
//! action runs `sunrise-server` or calls the running server's control API (`sunrise_launcher::servers`).
use super::line_edit::{self, Key, LineEdit};
use super::*;
use crate::frame::Reach as _;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use sunrise_launcher::heads;
use sunrise_launcher::servers::{
    self, Applied, Backup, Entry, Fix, NewServer, Problem, Status, World,
};
use sunrise_launcher_theme::glass::radius;

#[path = "glass_server_dialog.rs"]
mod glass_server_dialog;
#[path = "glass_servers.rs"]
mod glass_servers;

mod hosting;
#[path = "servers_format.rs"]
mod servers_format;
use servers_format::{backup_label, bytes, uptime};

/// One consent boundary for every launcher-managed start/restart route.
fn run_managed_server(
    tx: &std::sync::mpsc::Sender<backend::Msg>,
    dir: &Path,
    args: &[&str],
    log: &dyn Fn(String),
) -> Result<(), Problem> {
    crate::jar_launch::with_consent(tx, |permission| {
        servers::run_managed(dir, args, log, permission)
    })
}

/// The servers this launcher started in this session, stopped when it closes unless they run as a
/// service (`stop_on_quit`).
static STARTED_HERE: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// How often the open server's status and log are read, and the list's.
const POLL_OPEN: Duration = Duration::from_millis(1000);
const POLL_LIST: Duration = Duration::from_secs(3);

/// Lines of the server's log kept on the page.
const CONSOLE_KEEP: usize = 1000;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) enum Field {
    Name,
    Seed,
    Players,
    Command,
    AddPlayer,
    Motd,
    Port,
    Token,
    Email,
    /// The instance dialog's (`instances_page.rs`).
    InstanceName,
    InstanceGroup,
    /// Home's Multiplayer section's (`multiplayer_page.rs`).
    ServerName,
    ServerAddress,
    /// The Launch panel's (`launch_page.rs`).
    PreLaunch,
    Wrapper,
    LaunchEnv,
    PostExit,
    DeleteServer,
    /// Settings' accent as hex (`themes_page.rs`).
    AccentHex,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Tab {
    Overview,
    Players,
    Settings,
    Backups,
}

const TABS: [(Tab, &str); 4] = [
    (Tab::Overview, "Overview"),
    (Tab::Players, "Players"),
    (Tab::Settings, "Settings"),
    (Tab::Backups, "Backups"),
];

const MODES: [(&str, &str); 3] = [
    ("survival", "Survival"),
    ("creative", "Creative"),
    ("adventure", "Adventure"),
];

const DIFFICULTIES: [(&str, &str); 4] = [
    ("peaceful", "Peaceful"),
    ("easy", "Easy"),
    ("normal", "Normal"),
    ("hard", "Hard"),
];

/// What the background work reports.
pub(super) enum Event {
    Polled {
        dir: PathBuf,
        status: Box<Status>,
        logs: Option<(u64, Vec<String>)>,
    },
    Statuses(Vec<(PathBuf, Status)>),
    Line(String),
    Done(Result<String, Problem>),
    Created(Result<Entry, Problem>),
    Deleted {
        dir: PathBuf,
        result: Result<(), String>,
    },
    Details {
        dir: PathBuf,
        config: Option<Value>,
        backups: Vec<Backup>,
        service: bool,
    },
    Applied(Result<Applied, Problem>),
    Pvp(Option<bool>),
    UpdateCheck(Result<Value, Problem>),
    Failed(String),
}

/// The Create dialog's answers.
pub(super) struct Create {
    new_world: bool,
    copy: Option<PathBuf>,
    worlds: Vec<(String, PathBuf)>,
    mode: &'static str,
    difficulty: &'static str,
    max_players: u32,
    friends_only: bool,
    check_accounts: bool,
    eula: bool,
    /// Create was pressed without the EULA ticked.
    eula_missing: bool,
    /// The glass dialog's friends let in, the first an operator (the light one types them in
    /// `Field::Players`).
    friends: Vec<String>,
}

/// What the sidebar shows of the Servers page: its list once the page has read it, and whether
/// a server answers.
impl Servers {
    pub(super) fn location_busy(&self) -> bool {
        self.busy.is_some()
            || self.creating
            || self.applying
            || !self.queued.is_empty()
            || self
                .statuses
                .values()
                .any(|s| matches!(s.phase.as_str(), "running" | "starting" | "stopping"))
    }

    /// A stopped server's line in the list: "Survival · stopped", as the mockup's. The open
    /// server's mode is the one its settings show now, once read; the others' (and the open one's
    /// until then) come from their files.
    pub(super) fn stopped_line(&mut self, dir: &std::path::Path) -> String {
        let open = Some(self.shown("world", "game_mode"))
            .filter(|m| !m.is_empty() && self.open.as_deref() == Some(dir));
        let mode = open.or_else(|| {
            self.modes
                .entry(dir.to_path_buf())
                .or_insert_with(|| servers::game_mode(dir))
                .clone()
        });
        match mode {
            Some(mode) => {
                let label = MODES
                    .iter()
                    .find(|(id, _)| *id == mode)
                    .map_or(mode.clone(), |(_, label)| tr(label).to_string());
                trf("{mode} · stopped", &[("mode", &label)])
            }
            None => tr("Stopped").to_string(),
        }
    }
}

pub(super) struct Servers {
    list: Vec<Entry>,
    loaded: bool,
    pub(super) open: Option<PathBuf>,
    tab: Tab,
    statuses: HashMap<PathBuf, Status>,
    /// Each listed server's game mode from its `server.toml`, read once while it is listed.
    modes: HashMap<PathBuf, Option<String>>,
    polling: bool,
    polled: Option<Instant>,
    log: VecDeque<String>,
    log_next: u64,
    log_dir: Option<PathBuf>,
    console_scroll: Scrollbar,
    create_scroll: Scrollbar,
    /// The action under way, and what it printed.
    busy: Option<String>,
    job_log: VecDeque<String>,
    problem: Option<Problem>,
    note: Option<String>,
    pub(super) create: Option<Create>,
    creating: bool,
    config: Option<Value>,
    /// Settings waiting for `config set`, and whether one runs.
    queued: Vec<(String, String)>,
    applying: bool,
    /// Settings saved while the server ran that wait for its next start.
    restart_needed: BTreeSet<String>,
    backups: Vec<Backup>,
    confirm_restore: Option<String>,
    confirm_delete: bool,
    service: Option<bool>,
    update: Option<Value>,
    /// `update --check` is under way.
    checking: bool,
    pvp: Option<bool>,
    texts: HashMap<Field, LineEdit>,
    focus: HashMap<Field, FocusHandle>,
    focused: Option<Field>,
    /// The field to put the keyboard in on the next frame.
    pub(super) focus_soon: Option<Field>,
    /// A suggested value shown selected: the first key typed replaces it.
    selected: Option<Field>,
    /// A field whose value was refused, and why: shown under it until it's edited.
    invalid: Option<(Field, &'static str)>,
    copied: Option<Instant>,
    faces: HashMap<String, Option<Vec<[u8; 4]>>>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
}

impl Servers {
    pub(super) fn new() -> Self {
        let (tx, rx) = crate::wake::channel();
        Self {
            list: Vec::new(),
            loaded: false,
            open: None,
            tab: Tab::Overview,
            statuses: HashMap::new(),
            modes: HashMap::new(),
            polling: false,
            polled: None,
            log: VecDeque::new(),
            log_next: 0,
            log_dir: None,
            console_scroll: Scrollbar::default(),
            create_scroll: Scrollbar::default(),
            busy: None,
            job_log: VecDeque::new(),
            problem: None,
            note: None,
            create: None,
            creating: false,
            config: None,
            queued: Vec::new(),
            applying: false,
            restart_needed: BTreeSet::new(),
            backups: Vec::new(),
            confirm_restore: None,
            confirm_delete: false,
            service: None,
            update: None,
            checking: false,
            pvp: None,
            texts: HashMap::new(),
            focus: HashMap::new(),
            focused: None,
            focus_soon: None,
            selected: None,
            invalid: None,
            copied: None,
            faces: HashMap::new(),
            tx,
            rx,
        }
    }

    pub(super) fn text(&mut self, f: Field) -> &mut LineEdit {
        self.texts.entry(f).or_default()
    }

    pub(super) fn value(&self, f: Field) -> &str {
        self.texts.get(&f).map_or("", LineEdit::text)
    }

    /// Whether this field has the keyboard, as of this frame.
    pub(super) fn has_focus(&self, f: Field) -> bool {
        self.focused == Some(f)
    }

    /// Which field has the keyboard, read every frame.
    pub(super) fn note_focus(&mut self, window: &mut Window, cx: &mut App) {
        if let Some(h) = self.focus_soon.take().and_then(|f| self.focus.get(&f)) {
            window.focus(h, cx);
        }
        self.focused = self
            .focus
            .iter()
            .find(|(_, h)| h.is_focused(window))
            .map(|(f, _)| *f);
        if self.focused.is_some() && self.focused != self.selected {
            self.selected = None;
        }
    }

    fn status(&self) -> Status {
        self.open
            .as_ref()
            .and_then(|d| self.statuses.get(d).cloned())
            .unwrap_or_else(Status::stopped)
    }

    /// A setting from server.toml (as `config --json` gave it).
    fn setting(&self, section: &str, key: &str) -> Option<&Value> {
        self.config
            .as_ref()
            .map(|c| &c[section][key])
            .filter(|v| !v.is_null())
    }

    /// A setting as shown: what was just asked for, else what server.toml holds.
    fn shown(&self, section: &str, key: &str) -> String {
        let full = format!("{section}.{key}");
        if let Some((_, v)) = self.queued.iter().rev().find(|(k, _)| *k == full) {
            return v.clone();
        }
        match self.setting(section, key) {
            Some(Value::String(s)) => s.clone(),
            None => String::new(),
            Some(v) => v.to_string(),
        }
    }
}

/// Stops the servers this launcher started, unless they run in the background as a service.
pub(super) fn stop_on_quit() {
    for dir in STARTED_HERE.lock().unwrap().drain(..) {
        if servers::service_installed(&dir) {
            continue;
        }
        if let Some(api) = servers::Api::open(&dir) {
            let _ = api.call("POST", "/api/v1/stop", None);
        }
    }
}

/// Update closes the window to open the new launcher at once: a server is its own process and the
/// new window finds it running, so an update doesn't end anyone's game. It stays up when that new
/// window closes too, until it is stopped on the Servers page.
pub(super) fn keep_running_through_update() {
    STARTED_HERE.lock().unwrap().clear();
}

/// The player's own name, to let them in on a friends-only server: the Microsoft account that plays.
fn own_name(accounts: &Accounts) -> Option<String> {
    accounts.selected().map(|a| a.name.clone())
}

impl Launcher {
    /// Whether a server this launcher started still runs and would stop with it
    /// ([`stop_on_quit`]): the launcher doesn't close itself under it while the game plays.
    pub(super) fn hosting_here(&self) -> bool {
        STARTED_HERE.lock().unwrap().iter().any(|dir| {
            !servers::service_installed(dir)
                && self.servers.statuses.get(dir).is_some_and(Status::running)
        })
    }

    // ----- Background work ------------------------------------------------------------------

    /// Runs `job` on a thread with the page busy; its lines and result arrive as events.
    fn server_job(
        &mut self,
        label: impl Into<String>,
        job: impl FnOnce(&dyn Fn(String)) -> Result<String, Problem> + Send + 'static,
    ) {
        if self.servers.busy.is_some() {
            return;
        }
        self.servers.busy = Some(label.into());
        self.servers.job_log.clear();
        self.servers.problem = None;
        self.servers.note = None;
        let tx = self.servers.tx.clone();
        std::thread::spawn(move || {
            let lines = tx.clone();
            let log = move |l: String| {
                let _ = lines.send(Event::Line(l));
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&log)))
                .unwrap_or_else(|panic| {
                    let error = (panic.downcast_ref::<&str>().map(|s| s.to_string()))
                        .or_else(|| panic.downcast_ref::<String>().cloned())
                        .unwrap_or_default();
                    Err(Problem::plain(trf(
                        "Could not run {program}: {error}",
                        &[("program", "sunrise-server"), ("error", &error)],
                    )))
                });
            let _ = tx.send(Event::Done(result));
        });
    }

    /// `sunrise-server <args>` on the open server, as a job.
    fn server_command(&mut self, label: &str, args: Vec<String>) {
        let Some(dir) = self.servers.open.clone() else {
            return;
        };
        let tx = self.tx.clone();
        self.server_job(label.to_string(), move |log| {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let result = match args.first().copied() {
                Some("start" | "restart") => {
                    if args[0] == "start" {
                        log(tr("A new world takes a minute the first time").into());
                    }
                    run_managed_server(&tx, &dir, &args, log)
                }
                _ => servers::run_streamed(&dir, &args, log),
            };
            result.map(|_| String::new())
        });
    }

    /// Reads the open server's settings, backups and service state again.
    fn load_details(&self) {
        let Some(dir) = self.servers.open.clone() else {
            return;
        };
        let tx = self.servers.tx.clone();
        std::thread::spawn(move || {
            let config = servers::config(&dir);
            let backups = servers::backups(&dir).unwrap_or_default();
            let service = servers::service_installed(&dir);
            let _ = tx.send(Event::Details {
                dir,
                config,
                backups,
                service,
            });
        });
    }

    pub(super) fn open_server(&mut self, dir: PathBuf) {
        if self.servers.open.as_ref() == Some(&dir) {
            return;
        }
        let s = &mut self.servers;
        s.open = Some(dir);
        // Its settings may have changed while it was open.
        s.modes.clear();
        s.tab = Tab::Overview;
        s.config = None;
        s.backups.clear();
        s.service = None;
        s.update = None;
        s.pvp = None;
        s.problem = None;
        s.note = None;
        s.confirm_restore = None;
        s.confirm_delete = false;
        s.texts.remove(&Field::DeleteServer);
        s.restart_needed.clear();
        s.texts.remove(&Field::Motd);
        s.texts.remove(&Field::Port);
        s.invalid = None;
        s.polled = None;
        self.load_details();
    }

    /// Called with the launcher's other work: what the background sent, and the next status poll.
    pub(super) fn servers_tick(&mut self) -> bool {
        // The Hosting page shows how friends reach the server beside it, whatever the tab.
        let showing = self.ui.page == crate::frame::Page::Hosting;
        let overview = showing && self.servers.open.is_some() && self.servers.tab == Tab::Overview;
        let mut changed = self.internet_tick(overview);
        while let Ok(event) = self.servers.rx.try_recv() {
            changed |= self.servers_event(event);
        }
        if !showing || self.servers.polling || !self.servers.loaded {
            return changed;
        }
        let every = if self.servers.open.is_some() {
            POLL_OPEN
        } else {
            POLL_LIST
        };
        if self.servers.polled.is_some_and(|at| at.elapsed() < every) {
            return changed;
        }
        self.servers.polled = Some(Instant::now());
        self.servers.polling = true;
        let tx = self.servers.tx.clone();
        match self.servers.open.clone() {
            Some(dir) => {
                let since = if self.servers.log_dir.as_ref() == Some(&dir) {
                    self.servers.log_next
                } else {
                    0
                };
                std::thread::spawn(move || {
                    let api = servers::Api::open(&dir);
                    let status = api
                        .as_ref()
                        .and_then(|a| a.status())
                        .unwrap_or_else(Status::stopped);
                    let logs = api.and_then(|a| a.logs(since));
                    let _ = tx.send(Event::Polled {
                        dir,
                        status: Box::new(status),
                        logs,
                    });
                });
            }
            None => {
                let dirs: Vec<PathBuf> = self.servers.list.iter().map(|e| e.dir.clone()).collect();
                std::thread::spawn(move || {
                    let all = dirs
                        .into_iter()
                        .map(|d| {
                            let s = servers::status(&d);
                            (d, s)
                        })
                        .collect();
                    let _ = tx.send(Event::Statuses(all));
                });
            }
        }
        changed
    }

    fn servers_event(&mut self, event: Event) -> bool {
        let s = &mut self.servers;
        match event {
            Event::Polled { dir, status, logs } => {
                s.polling = false;
                let mut changed = false;
                let was = s.statuses.get(&dir).map(|st| st.phase.clone());
                if was.as_deref() != Some("running") && status.phase == "running" {
                    // A fresh start took every saved setting.
                    s.restart_needed.clear();
                    s.pvp = None;
                    changed = true;
                }
                if let Some((next, lines)) = logs {
                    if s.log_dir.as_ref() != Some(&dir) {
                        s.log.clear();
                        s.log_dir = Some(dir.clone());
                        changed = true;
                    }
                    if !lines.is_empty() {
                        s.log.extend(lines);
                        while s.log.len() > CONSOLE_KEEP {
                            s.log.pop_front();
                        }
                        s.console_scroll.handle.scroll_to_bottom();
                        changed = true;
                    }
                    s.log_next = next;
                }
                let read_pvp = status.phase == "running" && s.pvp.is_none();
                if s.statuses.get(&dir) != Some(&status) {
                    s.statuses.insert(dir, *status);
                    changed = true;
                }
                if read_pvp {
                    self.read_pvp();
                    changed = true;
                }
                changed
            }
            Event::Statuses(all) => {
                s.polling = false;
                let mut changed = false;
                for (dir, status) in all {
                    if s.statuses.get(&dir) != Some(&status) {
                        s.statuses.insert(dir, status);
                        changed = true;
                    }
                }
                changed
            }
            Event::Line(line) => {
                s.job_log.push_back(line);
                while s.job_log.len() > 6 {
                    s.job_log.pop_front();
                }
                true
            }
            Event::Done(result) => {
                s.busy = None;
                s.polled = None;
                match result {
                    Ok(note) if !note.is_empty() => s.note = Some(note),
                    Ok(_) => {}
                    Err(p) => s.problem = Some(p),
                }
                self.load_details();
                true
            }
            Event::Created(result) => {
                s.creating = false;
                match result {
                    Ok(entry) => {
                        s.create = None;
                        s.list = servers::list();
                        s.modes.clear();
                        self.open_server(entry.dir.clone());
                        self.servers.note = Some(format!(
                            "{} is ready. Press Start, then give your friends the address below.",
                            entry.name
                        ));
                    }
                    Err(p) => s.problem = Some(p),
                }
                true
            }
            Event::Details {
                dir,
                config,
                backups,
                service,
            } => {
                if s.open.as_ref() == Some(&dir) {
                    s.config = config;
                    s.backups = backups;
                    s.service = Some(service);
                    true
                } else {
                    false
                }
            }
            Event::Deleted { dir, result } => {
                s.busy = None;
                s.confirm_delete = false;
                s.list = servers::list();
                s.modes.clear();
                s.statuses.remove(&dir);
                STARTED_HERE
                    .lock()
                    .unwrap()
                    .retain(|started| started != &dir);
                if !s.list.iter().any(|entry| entry.dir == dir) {
                    s.open = None;
                    self.side_servers = s.list.clone();
                }
                match result {
                    Ok(()) => s.note = Some("Server moved to the trash.".into()),
                    Err(e) => s.problem = Some(Problem::plain(e)),
                }
                true
            }
            Event::Applied(result) => {
                s.applying = false;
                match result {
                    Ok(applied) if applied.running => s.restart_needed.extend(applied.restart),
                    Ok(_) => {}
                    Err(p) => s.problem = Some(p),
                }
                if s.queued.is_empty() {
                    self.load_details();
                } else {
                    self.apply_settings();
                }
                true
            }
            Event::Pvp(on) => {
                s.pvp = on;
                true
            }
            Event::UpdateCheck(result) => {
                s.checking = false;
                match result {
                    Ok(v) => s.update = Some(v),
                    Err(p) => s.problem = Some(p),
                }
                true
            }
            Event::Failed(e) => {
                s.problem = Some(Problem::plain(e));
                true
            }
        }
    }

    /// The `pvp` game rule, asked of the running game (it's a rule of the world, not a setting).
    fn read_pvp(&self) {
        let Some(dir) = self.servers.open.clone() else {
            return;
        };
        let tx = self.servers.tx.clone();
        std::thread::spawn(move || {
            let on = servers::Api::open(&dir)
                .and_then(|a| a.command("gamerule pvp").ok())
                .and_then(|out| {
                    let text = out.join(" ");
                    let text = text.trim_end();
                    if text.ends_with("true") {
                        Some(true)
                    } else if text.ends_with("false") {
                        Some(false)
                    } else {
                        None
                    }
                });
            let _ = tx.send(Event::Pvp(on));
        });
    }

    /// Queues a setting for `sunrise-server config set`; they go one batch at a time.
    fn set_setting(&mut self, key: &str, value: String) {
        self.servers.queued.retain(|(k, _)| k != key);
        self.servers.queued.push((key.to_string(), value));
        self.servers.note = None;
        if !self.servers.applying {
            self.apply_settings();
        }
    }

    fn apply_settings(&mut self) {
        let Some(dir) = self.servers.open.clone() else {
            return;
        };
        let batch = std::mem::take(&mut self.servers.queued);
        if batch.is_empty() {
            return;
        }
        self.servers.applying = true;
        // Shown as asked until the file is read back.
        if let Some(config) = &mut self.servers.config {
            for (key, value) in &batch {
                if let Some((section, field)) = key.split_once('.') {
                    let new = match &config[section][field] {
                        Value::Bool(_) => Value::Bool(value == "true"),
                        Value::Number(_) => value.parse::<i64>().map_or(Value::Null, Value::from),
                        _ => Value::String(value.clone()),
                    };
                    config[section][field] = new;
                }
            }
        }
        let tx = self.servers.tx.clone();
        std::thread::spawn(move || {
            let pairs: Vec<(&str, String)> =
                batch.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
            let _ = tx.send(Event::Applied(servers::set_config(&dir, &pairs)));
        });
    }

    /// A command for the running game, through the control API; its answer shows in the console.
    fn api_command(&mut self, line: String) {
        let Some(dir) = self.servers.open.clone() else {
            return;
        };
        let tx = self.servers.tx.clone();
        self.servers.log.push_back(format!("> {line}"));
        self.servers.console_scroll.handle.scroll_to_bottom();
        std::thread::spawn(move || {
            let result = servers::Api::open(&dir)
                .ok_or_else(|| "The server isn't running.".to_string())
                .and_then(|a| a.command(&line));
            if let Err(e) = result {
                let _ = tx.send(Event::Failed(e));
            }
        });
    }

    fn start_server(&mut self) {
        let Some(dir) = self.servers.open.clone() else {
            return;
        };
        STARTED_HERE.lock().unwrap().push(dir.clone());
        self.servers.restart_needed.clear();
        let tx = self.tx.clone();
        self.server_job("Starting", move |log| {
            if let Some(api) = servers::Api::open(&dir) {
                if let Ok(status) = api.call("GET", "/api/v1/status", None) {
                    if status["phase"] == "stopped"
                        && api.call("POST", "/api/v1/start", None).is_ok()
                    {
                        return Ok(String::new());
                    }
                }
            }
            log("A new world takes a minute the first time".into());
            run_managed_server(&tx, &dir, &["start"], log).map(|_| String::new())
        });
    }

    fn stop_server(&mut self) {
        let Some(dir) = self.servers.open.clone() else {
            return;
        };
        self.server_job("Stopping", move |log| {
            // The running server's API first; when it can't be reached or refuses, the CLI,
            // whose error is the job's.
            let via_api = servers::Api::open(&dir)
                .is_some_and(|api| api.call("POST", "/api/v1/stop", None).is_ok());
            if via_api {
                return Ok(String::new());
            }
            servers::run_streamed(&dir, &["stop"], log).map(|_| String::new())
        });
    }

    fn restart_server(&mut self) {
        let Some(dir) = self.servers.open.clone() else {
            return;
        };
        let tx = self.tx.clone();
        self.server_job("Restarting", move |log| {
            // The running server's API first; when it can't be reached or refuses, the CLI,
            // whose error is the job's.
            let via_api = servers::Api::open(&dir)
                .is_some_and(|api| api.call("POST", "/api/v1/restart", None).is_ok());
            if via_api {
                return Ok(String::new());
            }
            run_managed_server(&tx, &dir, &["restart"], log).map(|_| String::new())
        });
    }

    fn fix(&mut self, fix: Fix, cx: &mut Context<Self>) {
        self.servers.problem = None;
        match fix {
            Fix::UsePort(port) => {
                self.set_setting("server.port", port.to_string());
                self.servers.texts.remove(&Field::Port);
                self.servers.note = Some(format!(
                    "The server now uses port {port}. Press Start again."
                ));
            }
            Fix::Prepare => match self.selected().cloned() {
                Some(tag) if backend::is_prepared(&tag) => {
                    self.server_job("Copying the game files", move |log| {
                        log(format!("Copying the game files prepared for Sunrise {tag}"));
                        servers::set_up(&tag)
                            .map(|_| "The game files are ready. Press Start again.".to_string())
                            .map_err(Problem::plain)
                    });
                }
                _ => {
                    self.servers.create = None;
                    self.go(Page::Play);
                    if !self.preparing {
                        self.prepare(backend::MinecraftFrom::Install);
                    }
                }
            },
            Fix::AgreeEula => {
                if let Some(c) = &mut self.servers.create {
                    c.eula = true;
                    c.eula_missing = false;
                } else if let Some(dir) = &self.servers.open {
                    if let Err(e) = backend::agree_eula(dir) {
                        self.servers.problem = Some(Problem::plain(e));
                    }
                }
            }
            Fix::ShowLog => {
                self.servers.tab = Tab::Overview;
                if let Some(dir) = &self.servers.open {
                    crate::glass_toast::open(cx, &dir.join("logs"));
                }
            }
        }
    }

    // ----- Creating -------------------------------------------------------------------------

    fn open_create(&mut self, cx: &mut Context<Self>) {
        self.focus_handle(Field::Name, cx);
        self.servers.focus_soon = Some(Field::Name);
        let worlds = servers::player_worlds(&backend::game_home());
        let name = if self.servers.list.is_empty() {
            "My server".to_string()
        } else {
            format!("Server {}", self.servers.list.len() + 1)
        };
        // The glass dialog starts with an empty name (its placeholder says the default) and the
        // player as the first friend.
        let glass = self.theme.glass;
        let own = own_name(&self.accounts).unwrap_or_default();
        self.servers
            .text(Field::Name)
            .set(if glass { String::new() } else { name });
        self.servers.selected = (!glass).then_some(Field::Name);
        self.servers.text(Field::Seed).set(String::new());
        self.servers
            .text(Field::Players)
            .set(if glass { String::new() } else { own.clone() });
        let friends = if glass && !own.is_empty() {
            vec![own]
        } else {
            Vec::new()
        };
        // Nothing to run it with yet: said as the form opens, not after it is filled in.
        self.servers.problem = servers::create_blocked(self.selected().map(String::as_str));
        self.servers.create_scroll = Scrollbar::default();
        self.servers.create = Some(Create {
            new_world: true,
            copy: worlds.first().map(|w| w.1.clone()),
            worlds,
            mode: "survival",
            difficulty: "normal",
            max_players: 10,
            friends_only: true,
            check_accounts: true,
            eula: false,
            eula_missing: false,
            friends,
        });
    }

    fn create_server(&mut self) {
        if self.servers.creating {
            return;
        }
        let Some(c) = &mut self.servers.create else {
            return;
        };
        if !c.eula {
            c.eula_missing = true;
            return;
        }
        let Some(c) = self.servers.create.as_ref() else {
            return;
        };
        let world = match (&c.copy, c.new_world) {
            (Some(dir), false) => World::Copy(dir.clone()),
            _ => World::New {
                seed: self.servers.value(Field::Seed).to_string(),
            },
        };
        let mut players = c.friends.clone();
        for p in self.servers.value(Field::Players).split([',', ' ']) {
            if !p.is_empty() && !players.iter().any(|q| q == p) {
                players.push(p.to_string());
            }
        }
        let name = self.servers.value(Field::Name).trim().to_string();
        let spec = NewServer {
            name: if !name.is_empty() {
                name
            } else if self.servers.list.is_empty() {
                "My server".into()
            } else {
                format!("Server {}", self.servers.list.len() + 1)
            },
            world,
            game_mode: c.mode.into(),
            difficulty: c.difficulty.into(),
            max_players: c.max_players,
            friends_only: c.friends_only,
            check_accounts: c.check_accounts,
            players,
            eula: c.eula,
        };
        let tag = self.selected().cloned();
        let tx = self.servers.tx.clone();
        self.servers.creating = true;
        self.servers.problem = None;
        self.servers.job_log.clear();
        std::thread::spawn(move || {
            let lines = tx.clone();
            let log = move |l: String| {
                let _ = lines.send(Event::Line(l));
            };
            let _ = tx.send(Event::Created(servers::create(&spec, tag.as_deref(), &log)));
        });
    }

    // ----- Building blocks ------------------------------------------------------------------

    pub(super) fn focus_handle(&mut self, f: Field, cx: &mut Context<Self>) -> FocusHandle {
        self.servers
            .focus
            .entry(f)
            .or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone()
    }

    /// A one-line text field: typing, paste, Backspace, Enter to confirm, Escape, Tab to the next.
    pub(super) fn field(
        &mut self,
        f: Field,
        placeholder: &'static str,
        width: f32,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = self.theme;
        let handle = self.focus_handle(f, cx);
        let focused = self.servers.focused == Some(f);
        let selected = focused && self.servers.selected == Some(f);
        let invalid = self.servers.invalid.is_some_and(|(g, _)| g == f);
        self.servers.text(f).show(((width - 24.) / 7.5) as usize);
        // Home's Add server line is in the glass look already; the other fields follow with
        // their pages.
        // The dark appearance draws every field in the glass look.
        let glass = t.glass;
        let base = if glass {
            // The mods filter and the log search sit by buttons: the mockup's small `FIELD`, as
            // tall as they are.
            crate::glass_ui::field(focused, invalid)
                .when(
                    matches!(
                        f,
                        Field::Motd
                            | Field::Port
                            | Field::Token
                            | Field::AddPlayer
                            | Field::DeleteServer
                    ),
                    |d| {
                        d.h(px(36.))
                            .px(px(12.))
                            .rounded(px(crate::glass_ui::rad(radius::CONTROL)))
                    },
                )
                // The instance dialog's are the large `FIELD_LG`: 48 px, 15 px text, white at 6 %,
                // 10 % while typed in.
                .when(
                    !invalid
                        && matches!(
                            f,
                            Field::InstanceName | Field::InstanceGroup | Field::Name | Field::Seed
                        ),
                    |d| {
                        d.h(px(48.))
                            .text_size(px(15.))
                            .bg(crate::glass_ui::white(if focused { 0.1 } else { 0.06 }))
                    },
                )
                // The accent's hex: `h-9 w-[96px] rounded-[10px] px-3 text-[13px] font-medium`.
                .when(f == Field::AccentHex, |d| {
                    d.h(px(36.))
                        .px(px(12.))
                        .rounded(px(crate::glass_ui::rad(radius::CONTROL)))
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                })
                // The friend field sits in its chip, on the chip's fill.
                .when(matches!(f, Field::Players), |d| {
                    d.h_full().px(px(14.)).bg(gpui::transparent_black())
                })
                // The server's command line: the field is the line's middle, on its fill, in mono.
                .when(f == Field::Command, |d| {
                    d.h_full()
                        .px_0()
                        .rounded_none()
                        .bg(gpui::transparent_black())
                        .font_family(theme::MONO)
                        .text_size(px(13.))
                })
        } else {
            div()
                .flex()
                .items_center()
                .h(px(theme::FIELD))
                .px(px(12.))
                .overflow_hidden()
                .rounded(px(theme::RADIUS))
                .border_1()
                .border_color(if invalid {
                    t.danger
                } else if focused {
                    t.accent
                } else {
                    t.rule
                })
                .bg(t.surface)
                .text_size(px(theme::UI.px))
        };
        let field = base
            .id(SharedString::from(format!("field-{f:?}")))
            .track_focus(&handle)
            .seen()
            .w(px(width))
            .cursor_text()
            .child(line_edit::content(
                self.servers.text(f),
                &t,
                focused,
                selected,
                f == Field::Token,
                placeholder,
            ))
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                // A click puts the caret in: the suggestion is no longer selected.
                if matches!(ev, ClickEvent::Mouse(_)) {
                    this.servers.selected = None;
                }
                if let Some(h) = this.servers.focus.get(&f) {
                    window.focus(h, cx);
                }
                cx.notify();
            }))
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key != "enter" && this.servers.invalid.is_some_and(|(g, _)| g == f)
                {
                    this.servers.invalid = None;
                }
                // A suggestion is selected: what's typed or pasted replaces it.
                let selected = this.servers.selected.take() == Some(f);
                if selected {
                    this.servers.text(f).select_all();
                }
                match line_edit::handle(this.servers.text(f), ev, cx) {
                    Key::Enter if matches!(f, Field::InstanceName | Field::InstanceGroup) => {
                        this.save_instance(cx)
                    }
                    Key::Enter => this.field_enter(f),
                    Key::Escape => {
                        this.servers.confirm_delete = false;
                        if this.servers.create.is_some() && !this.servers.creating {
                            this.servers.create = None;
                        }
                        this.instance_escape();
                        window.blur(cx);
                    }
                    // Tab and the rest go on to the window: Tab moves the focus.
                    Key::Pass => return,
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        line_edit::mouse(field, cx, move |this| this.servers.text(f))
    }

    fn field_enter(&mut self, f: Field) {
        match f {
            // The glass dialog's friend field adds the name typed.
            Field::Players if self.theme.glass => self.add_create_friend(),
            Field::Name | Field::Seed | Field::Players => self.create_server(),
            Field::Command => {
                let line = self.servers.text(Field::Command).take();
                let line = line.trim().trim_start_matches('/').to_string();
                if !line.is_empty() {
                    self.api_command(line);
                }
            }
            Field::AddPlayer => self.add_player(),
            Field::Motd => {
                let motd = self.servers.value(Field::Motd).trim().to_string();
                if motd.is_empty() {
                    self.servers.problem =
                        Some(Problem::plain("Type the line players see under the name."));
                } else {
                    self.set_setting("server.motd", motd);
                }
            }
            Field::Port => self.save_port(),
            Field::Token => self.save_token(),
            Field::Email => self.invite_friend(),
            Field::InstanceName | Field::InstanceGroup | Field::DeleteServer | Field::AccentHex => {
            }
            Field::PreLaunch | Field::Wrapper | Field::LaunchEnv | Field::PostExit => {
                self.save_launch()
            }
            Field::ServerName | Field::ServerAddress => self.add_server(),
        }
    }

    fn save_port(&mut self) {
        match self.servers.value(Field::Port).trim().parse::<u16>() {
            Ok(port) if port >= 1024 => self.set_setting("server.port", port.to_string()),
            _ => {
                self.servers.invalid = Some((
                    Field::Port,
                    "Use a number from 1024 to 65535 (25565 is Minecraft's own).",
                ))
            }
        }
    }

    fn add_player(&mut self) {
        let name = self.servers.value(Field::AddPlayer).trim().to_string();
        let valid = (3..=16).contains(&name.len())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid {
            self.servers.invalid = Some((
                Field::AddPlayer,
                "A Minecraft name has 3 to 16 letters, digits or underscores.",
            ));
            return;
        }
        self.servers.text(Field::AddPlayer).set(String::new());
        self.server_command(
            &format!("Letting {name} in"),
            vec!["whitelist".into(), "add".into(), name],
        );
    }

    /// A player's face, from the default skin their UUID picks (the server knows no other).
    pub(super) fn head(&mut self, uuid: &str, size: f32) -> Div {
        let resources = servers::game_files(&servers::runtime_root(), "26.3");
        let face = self
            .servers
            .faces
            .entry(uuid.to_string())
            .or_insert_with(|| heads::default_face(&resources, uuid))
            .clone();
        let cell = size / 8.;
        let mut d = div()
            .flex_none()
            .flex()
            .flex_col()
            .size(px(size))
            .rounded(px(if self.theme.glass {
                crate::glass_ui::rad(size / 4.).min(size / 2.)
            } else {
                3.
            }))
            .overflow_hidden()
            .bg(self.theme.plate);
        if let Some(pixels) = face {
            for row in pixels.chunks(8) {
                d = d.child(div().flex().children(row.iter().map(|p| {
                    div().size(px(cell)).bg(Hsla::from(gpui::Rgba {
                        r: p[0] as f32 / 255.,
                        g: p[1] as f32 / 255.,
                        b: p[2] as f32 / 255.,
                        a: 1.,
                    }))
                })));
            }
        }
        d
    }

    // ----- The page -------------------------------------------------------------------------

    fn add_server_folder(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = paths.await {
                if let Some(dir) = paths.pop() {
                    let _ = this.update(cx, |this, cx| {
                        this.add_existing(dir);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// A folder with a server.toml joins the list; a vanilla server folder is set up first (its
    /// server.properties becomes server.toml; its world, lists and EULA stay).
    fn add_existing(&mut self, dir: PathBuf) {
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Server".into());
        let tag = self.selected().cloned();
        let tx = self.servers.tx.clone();
        self.servers.creating = true;
        self.servers.problem = None;
        std::thread::spawn(move || {
            let result = (|| {
                if servers::program().is_none() {
                    let tag =
                        tag.ok_or_else(|| Problem::plain("Install a Sunrise version first."))?;
                    servers::set_up(&tag).map_err(Problem::plain)?;
                }
                if !dir.join("server.toml").is_file() {
                    servers::run(&dir, &["init", "--yes"])?;
                }
                let entry = Entry {
                    name,
                    dir,
                    host: None,
                };
                servers::add(entry.clone()).map_err(Problem::plain)?;
                Ok(entry)
            })();
            let _ = tx.send(Event::Created(result));
        });
    }

    /// The server's details came back without its settings: its sunrise-server predates
    /// `config` (and `backups --json`).
    fn older_sunrise(&self) -> bool {
        self.servers.service.is_some() && self.servers.config.is_none()
    }

    // ----- The Create dialog ----------------------------------------------------------------
}

/// The state, in a few words.
fn phase_label(status: &Status) -> String {
    if let Some(busy) = &status.update_busy {
        return trf("Updating: {step}", &[("step", busy)]);
    }
    match status.phase.as_str() {
        "running" if status.crashes > 0 => tr("Running · restarted after a crash").into(),
        "running" if status.paused => tr("Running · paused while nobody plays").into(),
        "running" => tr("Running").into(),
        "starting" => tr("Starting").into(),
        "restarting" => tr("Restarting").into(),
        "stopping" => tr("Stopping").into(),
        _ => tr("Stopped").into(),
    }
}

/// A setting's name for people.
fn setting_name(key: &str) -> &str {
    match key {
        "server.motd" => "the message in the server list",
        "server.port" => "the port",
        "server.max_players" => "most players",
        "server.view_distance" => "view distance",
        "server.simulation_distance" => "simulation distance",
        "server.online_mode" => "account checks",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn managed_start_and_restart_require_consent_before_running_the_supervisor() {
        const CHILD: &str = "SUNRISE_MANAGED_CONSENT_TEST";
        if std::env::var_os(CHILD).is_none() {
            let root = tempfile::tempdir().unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "servers_page::tests::managed_start_and_restart_require_consent_before_running_the_supervisor", "--nocapture"])
                .env(CHILD, "1")
                .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
                .env("SUNRISE_HOME", root.path().join("cache"))
                .status().unwrap();
            assert!(status.success());
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let bin = servers::runtime_root().join("current");
        std::fs::create_dir_all(&bin).unwrap();
        let supervisor = bin.join("sunrise-server");
        let engine = bin.join("sunrise-engine");
        // Answers `engine-command --json` as the real supervisor does; anything else would start the server.
        let reply = serde_json::json!({
            "format": 1,
            "program": engine,
            "args": ["--resources", root.path(), "--control-socket", root.path().join(".sunrise/engine.json"), "--dedicated", "--nogui"],
            "cwd": root.path(),
            "env": {"SUNRISE_NO_AUDIO": "1"},
            "resources": root.path(),
        });
        std::fs::write(
            &supervisor,
            format!("#!/bin/sh\nif [ \"$3\" = engine-command ]; then printf '%s' '{reply}'; exit 0; fi\nprintf ran > \"$2/ran\"\n"),
        )
        .unwrap();
        std::fs::set_permissions(&supervisor, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(&engine, "never executed").unwrap();
        std::fs::write(root.path().join("sunrise-resources.json"), "{}").unwrap();
        let config = format!(
            "[engine]\nresources = {:?}\n",
            root.path().to_str().unwrap()
        );
        std::fs::write(root.path().join("server.toml"), config).unwrap();
        let package = root
            .path()
            .join(sunrise_launcher_smod::mod_list::DIR)
            .join("needs_jar");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(package.join("sunrise-mod.toml"), format!(
            "id='needs_jar'\nname='Needs jar'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n[[jar]]\nid='fixture'\nversion='1'\nsha512='{}'\n", "a".repeat(128)
        )).unwrap();
        std::fs::write(
            root.path().join(sunrise_launcher_smod::mod_list::FILE),
            "mod.needs_jar:true\n",
        )
        .unwrap();
        for action in ["start", "restart"] {
            let (tx, rx) = std::sync::mpsc::channel();
            let dir = root.path().to_path_buf();
            let job = std::thread::spawn(move || run_managed_server(&tx, &dir, &[action], &|_| {}));
            let message = rx.recv_timeout(Duration::from_secs(10));
            let backend::Msg::JarConsent { jars, reply } = message.unwrap() else {
                panic!("expected jar consent")
            };
            assert_eq!(jars.len(), 1);
            assert_eq!(jars[0].id(), "fixture");
            reply.send(false).unwrap();
            assert!(job.join().unwrap().is_err());
            assert!(!root.path().join("ran").exists());
        }
    }

    #[test]
    fn backups_read_for_people() {
        let b = |name: &str| Backup {
            name: name.into(),
            bytes: 1,
            modified: None,
        };
        assert_eq!(
            backup_label(&b("2026-10-19_11-28-57-manual.zip")),
            ("2026-10-19 11:28:57 UTC".into(), "Made by hand".into())
        );
        assert_eq!(
            backup_label(&b("2026-10-19_11-28-57-before-v2026.10.20.zip")).1,
            "Before updating to v2026.10.20"
        );
        assert_eq!(
            backup_label(&b("2026-10-19_13-35-36-before-v2026-10-19-10.zip")).1,
            "Before updating to v2026.10.19.10"
        );
        assert_eq!(
            backup_label(&b("2026-10-14_17-46-40-before-update.zip")).1,
            "Before an update"
        );
        assert_eq!(
            backup_label(&b("2026-10-19_11-28-57-auto.zip")).1,
            "Automatic"
        );
        assert_eq!(backup_label(&b("odd.zip")), ("odd".into(), String::new()));
        assert_eq!(bytes(5_292_542), "5.3 MB");
        assert_eq!(uptime(3725), "1 h 02 min");
    }
}
