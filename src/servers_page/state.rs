//! The Servers page's state: the servers and their status, the settings writes in flight and the
//! details read back for the open one.

use super::*;

#[derive(Clone)]
pub(super) struct SettingsBatch {
    pub(super) dir: PathBuf,
    pub(super) changes: Vec<(String, String)>,
}

impl SettingsBatch {
    pub(super) fn changes_port(&self) -> bool {
        self.changes.iter().any(|(key, _)| key == "server.port")
    }
}

#[derive(Default)]
pub(super) struct SettingsOutcome {
    pub(super) restart: BTreeSet<String>,
    /// True when a restart-requiring save succeeded while `running: true` against a stale
    /// non-running cached status, so the next `"running"` poll is catch-up rather than a restart.
    restart_needs_stop: bool,
    pub(super) problem: Option<Problem>,
    /// Setting keys from the batch that produced `problem`; an unrelated later save keeps it.
    failed_keys: BTreeSet<String>,
    /// Recently saved values kept visible until a fresh settings read replaces them.
    pub(super) saved: HashMap<String, String>,
}

/// What is open over the Host page's list.
pub(super) enum HostDialog {
    /// The new name's field (its text is `Servers::rename`).
    Rename { dir: PathBuf, focus: FocusHandle },
    /// The question before the server is moved to the trash.
    Delete { dir: PathBuf, name: String },
}

/// The "…" menu of the server in `dir`, hung from its button (window coordinates).
pub(super) struct HostMenu {
    pub(super) dir: PathBuf,
    pub(super) button: gpui::Bounds<gpui::Pixels>,
}

pub(crate) struct Servers {
    pub(super) list: Vec<Entry>,
    pub(super) loaded: bool,
    pub(super) open: Option<PathBuf>,
    pub(super) tab: Tab,
    pub(super) statuses: HashMap<PathBuf, Status>,
    /// Each listed server's game mode from its `server.toml`, read once while it is listed.
    pub(super) modes: HashMap<PathBuf, Option<String>>,
    pub(super) polling: bool,
    pub(super) polled: Option<Instant>,
    pub(super) log: VecDeque<String>,
    pub(super) log_next: u64,
    pub(super) log_dir: Option<PathBuf>,
    pub(super) console_scroll: Scrollbar,
    pub(super) create_scroll: Scrollbar,
    /// The action under way, and what it printed.
    pub(super) busy: Option<String>,
    pub(super) job_log: VecDeque<String>,
    /// The server the action under way is on, when the list started it (its card says so).
    pub(super) job_dir: Option<PathBuf>,
    pub(super) menu: Option<HostMenu>,
    pub(super) dialog: Option<HostDialog>,
    pub(super) rename: LineEdit,
    pub(super) problem: Option<Problem>,
    pub(super) note: Option<String>,
    pub(crate) create: Option<Create>,
    pub(super) creating: bool,
    pub(super) config: Option<Value>,
    pub(super) details_request: u64,
    /// Settings waiting for `config set`, and whether one runs.
    pub(super) queued: VecDeque<SettingsBatch>,
    pub(super) applying: Option<SettingsBatch>,
    /// A result stays with the server even when its page closes.
    pub(super) settings_outcomes: HashMap<PathBuf, SettingsOutcome>,
    pub(super) backups: Vec<Backup>,
    pub(super) confirm_restore: Option<String>,
    pub(super) confirm_delete: bool,
    pub(super) service: Option<bool>,
    pub(super) update: Option<Value>,
    /// `update --check` is under way.
    pub(super) checking: bool,
    pub(super) pvp: Option<bool>,
    pub(super) texts: HashMap<Field, LineEdit>,
    pub(super) focus: HashMap<Field, FocusHandle>,
    pub(super) focused: Option<Field>,
    /// The field to put the keyboard in on the next frame.
    pub(crate) focus_soon: Option<Field>,
    /// A suggested value shown selected: the first key typed replaces it.
    pub(super) selected: Option<Field>,
    /// A field whose value was refused, and why: shown under it until it's edited.
    pub(super) invalid: Option<(Field, &'static str)>,
    pub(super) internet_address: bool,
    pub(super) last_public_address: Option<std::net::Ipv4Addr>,
    pub(super) public_lookup: PublicLookup,
    public_lookup_request: u64,
    pub(super) public_lookup_in_flight: Option<u64>,
    pub(super) faces: HashMap<String, Option<Vec<[u8; 4]>>>,
    pub(super) tx: Sender<Event>,
    pub(super) rx: Receiver<Event>,
}

impl Servers {
    /// Work still owned by this launcher, not the lifetime of a dedicated server.
    pub(crate) fn writing(&self) -> bool {
        self.busy.is_some() || self.creating || self.applying.is_some() || !self.queued.is_empty()
    }

    /// The hosting job under way, for the status strip: what it does, and its last log line.
    pub(crate) fn job(&self) -> Option<(String, Option<String>)> {
        let what = match &self.busy {
            Some(doing) => tr(doing).to_string(),
            None if self.creating => tr("Setting up the server").to_string(),
            None => return None,
        };
        Some((what, self.job_log.back().cloned()))
    }

    pub(crate) fn location_busy(&self) -> bool {
        self.writing()
            || self
                .statuses
                .values()
                .any(|s| matches!(s.phase.as_str(), "running" | "starting" | "stopping"))
    }

    /// The game mode a listed server is set to, as people read it. The open server's mode is
    /// the one its settings show now, once read; the others' (and the open one's until then)
    /// come from their files.
    pub(super) fn mode_label(&mut self, dir: &std::path::Path) -> Option<String> {
        let open = Some(self.shown("world", "game_mode"))
            .filter(|m| !m.is_empty() && self.open.as_deref() == Some(dir));
        let mode = open.or_else(|| {
            self.modes
                .entry(dir.to_path_buf())
                .or_insert_with(|| servers::game_mode(dir))
                .clone()
        })?;
        Some(
            MODES
                .iter()
                .find(|(id, _)| *id == mode)
                .map_or(mode.clone(), |(_, label)| tr(label).to_string()),
        )
    }
}

impl Servers {
    pub(crate) fn new() -> Self {
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
            job_dir: None,
            menu: None,
            dialog: None,
            rename: LineEdit::default(),
            problem: None,
            note: None,
            create: None,
            creating: false,
            config: None,
            details_request: 0,
            queued: VecDeque::new(),
            applying: None,
            settings_outcomes: HashMap::new(),
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
            internet_address: false,
            last_public_address: None,
            public_lookup: PublicLookup::Idle,
            public_lookup_request: 0,
            public_lookup_in_flight: None,
            faces: HashMap::new(),
            tx,
            rx,
        }
    }

    /// Shuts the Host page's menu and dialog, as the page is left.
    pub(crate) fn close_layers(&mut self) {
        self.menu = None;
        self.dialog = None;
    }

    pub(crate) fn text(&mut self, f: Field) -> &mut LineEdit {
        self.texts.entry(f).or_default()
    }

    pub(crate) fn value(&self, f: Field) -> &str {
        self.texts.get(&f).map_or("", LineEdit::text)
    }

    /// Whether this field has the keyboard, as of this frame.
    pub(crate) fn has_focus(&self, f: Field) -> bool {
        self.focused == Some(f)
    }

    /// Which field has the keyboard, read every frame.
    pub(crate) fn note_focus(&mut self, window: &mut Window, cx: &mut App) {
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

    pub(super) fn status(&self) -> Status {
        self.open
            .as_ref()
            .and_then(|d| self.statuses.get(d).cloned())
            .unwrap_or_else(Status::unknown)
    }

    /// A setting from server.toml (as `config --json` gave it).
    pub(super) fn setting(&self, section: &str, key: &str) -> Option<&Value> {
        self.config
            .as_ref()
            .and_then(|c| c.get(section)?.get(key))
            .filter(|v| !v.is_null())
    }

    pub(super) fn share_address(
        &self,
        status: &Status,
    ) -> (Option<std::net::SocketAddr>, Option<u16>) {
        let saved = self
            .setting("server", "port")
            .and_then(Value::as_u64)
            .and_then(|port| u16::try_from(port).ok())
            .filter(|port| *port != 0);
        let saving_port = self
            .applying
            .iter()
            .chain(&self.queued)
            .any(|batch| self.open.as_ref() == Some(&batch.dir) && batch.changes_port());
        let address = match status.phase.as_str() {
            "running" => status.listener,
            "stopped" if !saving_port => saved.and_then(|port| {
                let bind = match self.setting("server", "bind") {
                    None => "",
                    Some(value) => value.as_str()?,
                };
                let ip: std::net::IpAddr = if bind.is_empty() {
                    std::net::Ipv4Addr::UNSPECIFIED.into()
                } else {
                    bind.parse().ok()?
                };
                Some(std::net::SocketAddr::new(ip.to_canonical(), port))
            }),
            _ => None,
        };
        (address, saved)
    }

    pub(crate) fn reset_public_lookup(&mut self) {
        self.public_lookup = PublicLookup::Idle;
    }

    pub(super) fn begin_public_lookup(&mut self) -> Option<u64> {
        if !self.internet_address || self.public_lookup_in_flight.is_some() {
            return None;
        }
        self.public_lookup_request += 1;
        let request = self.public_lookup_request;
        self.public_lookup_in_flight = Some(request);
        self.public_lookup = PublicLookup::Loading(request);
        Some(request)
    }

    pub(super) fn finish_public_lookup(
        &mut self,
        request: u64,
        result: Result<std::net::Ipv4Addr, String>,
    ) -> bool {
        if self.public_lookup_in_flight != Some(request) {
            return false;
        }
        self.public_lookup_in_flight = None;
        if self.public_lookup == PublicLookup::Loading(request) {
            self.public_lookup = match result {
                Ok(ip) => {
                    self.last_public_address = Some(ip);
                    PublicLookup::Detected
                }
                Err(_) => PublicLookup::Failed,
            };
        }
        true
    }

    /// A setting as shown: what was just asked for, else what was recently saved awaiting a fresh
    /// read, else what server.toml holds.
    pub(super) fn shown(&self, section: &str, key: &str) -> String {
        let full = format!("{section}.{key}");
        if let Some((_, value)) = self
            .queued
            .iter()
            .chain(self.applying.iter())
            .filter(|batch| self.open.as_ref() == Some(&batch.dir))
            .find_map(|batch| batch.changes.iter().find(|(key, _)| *key == full))
        {
            return value.clone();
        }
        if let Some(value) = self
            .open
            .as_ref()
            .and_then(|dir| self.settings_outcomes.get(dir))
            .and_then(|outcome| outcome.saved.get(&full))
        {
            return value.clone();
        }
        match self.setting(section, key) {
            Some(Value::String(s)) => s.clone(),
            None => String::new(),
            Some(v) => v.to_string(),
        }
    }

    pub(super) fn queue_setting(&mut self, dir: PathBuf, key: &str, value: String) {
        if let Some(batch) = self.queued.iter_mut().find(|batch| batch.dir == dir) {
            batch.changes.retain(|(queued, _)| queued != key);
            batch.changes.push((key.to_owned(), value));
        } else {
            self.queued.push_back(SettingsBatch {
                dir,
                changes: vec![(key.to_owned(), value)],
            });
        }
    }

    /// Starts the oldest batch, even if its server is no longer open on the page.
    pub(super) fn next_settings_batch(&mut self) -> Option<SettingsBatch> {
        if self.applying.is_some() {
            return None;
        }
        let batch = self.queued.pop_front()?;
        self.applying = Some(batch.clone());
        Some(batch)
    }

    /// Whether the displayed settings need re-reading now that this write has finished.
    pub(super) fn finish_settings(&mut self, dir: &Path, result: Result<Applied, Problem>) -> bool {
        let finished = self.applying.take();
        self.modes.remove(dir);
        let cached_running = self
            .statuses
            .get(dir)
            .is_some_and(|status| status.phase == "running");
        let outcome = self.settings_outcomes.entry(dir.to_path_buf()).or_default();
        match result {
            Ok(applied) => {
                if let Some(batch) = finished {
                    for (key, value) in batch.changes {
                        outcome.failed_keys.remove(&key);
                        outcome.saved.insert(key, value);
                    }
                }
                if outcome.failed_keys.is_empty() {
                    outcome.problem = None;
                }
                if applied.running && !applied.restart.is_empty() {
                    outcome.restart.extend(applied.restart);
                    if !cached_running {
                        outcome.restart_needs_stop = true;
                    }
                }
            }
            Err(problem) => {
                outcome.problem = Some(problem);
                outcome.failed_keys = finished
                    .map(|batch| batch.changes.into_iter().map(|(key, _)| key).collect())
                    .unwrap_or_default();
            }
        }
        self.open.as_deref() == Some(dir) && !self.queued.iter().any(|batch| batch.dir == dir)
    }

    pub(super) fn note_status(&mut self, dir: PathBuf, status: Status) -> bool {
        if self.statuses.get(&dir) == Some(&status) {
            return false;
        }
        if status.phase == "running" {
            let was_stopped = self
                .statuses
                .get(&dir)
                .is_some_and(|old| old.phase != "running");
            if let Some(outcome) = self.settings_outcomes.get_mut(&dir) {
                if outcome.restart_needs_stop {
                    // The status poll just caught up to the server that was already running
                    // when the save happened; keep the restart notice until a real restart.
                    outcome.restart_needs_stop = false;
                } else if was_stopped {
                    // A fresh start took that server's saved settings, not another server's.
                    outcome.restart.clear();
                }
            }
            if was_stopped && self.open.as_ref() == Some(&dir) {
                self.pvp = None;
            }
        } else if let Some(outcome) = self.settings_outcomes.get_mut(&dir) {
            outcome.restart_needs_stop = false;
        }
        self.statuses.insert(dir, status);
        true
    }

    pub(super) fn dismiss_problem(&mut self, settings_for: Option<&Path>) {
        if let Some(dir) = settings_for {
            if let Some(outcome) = self.settings_outcomes.get_mut(dir) {
                outcome.problem = None;
                outcome.failed_keys.clear();
            }
        } else {
            self.problem = None;
        }
    }
}

impl Launcher {
    /// Queues a setting for `sunrise-server config set`; they go one batch at a time.
    pub(super) fn set_setting(&mut self, key: &str, value: String) {
        let Some(dir) = self.servers.open.clone() else {
            return;
        };
        self.servers.queue_setting(dir, key, value);
        self.servers.note = None;
        self.apply_settings();
    }

    pub(super) fn apply_settings(&mut self) {
        let Some(SettingsBatch { dir, changes }) = self.servers.next_settings_batch() else {
            return;
        };
        let tx = self.servers.tx.clone();
        std::thread::spawn(move || {
            let pairs: Vec<_> = changes
                .iter()
                .map(|(key, value)| (key.as_str(), value.clone()))
                .collect();
            let result = servers::set_config(&dir, &pairs);
            let _ = tx.send(Event::Applied { dir, result });
        });
    }

    /// The server's details came back without its settings: its sunrise-server predates
    /// `config` (and `backups --json`).
    pub(super) fn older_sunrise(&self) -> bool {
        self.servers.service.is_some() && self.servers.config.is_none()
    }
}

impl Servers {
    /// A Host dialog (rename, delete) is up.
    pub(crate) fn dialog_open(&self) -> bool {
        self.dialog.is_some()
    }

    /// Closes a server card's "…" menu, as opening any other menu does.
    pub(crate) fn close_menu(&mut self) {
        self.menu = None;
    }
}
