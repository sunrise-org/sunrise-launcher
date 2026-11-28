//! The Servers page's background work: jobs, the status and log polls, and the events they send
//! back.

use super::state::SettingsBatch;
use super::*;

impl Servers {
    /// Runs `job` on a thread with the page busy; its lines and result arrive as events.
    pub(super) fn start_job(
        &mut self,
        label: impl Into<String>,
        job: impl FnOnce(&dyn Fn(String)) -> Result<String, Problem> + Send + 'static,
    ) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(label.into());
        self.job_log.clear();
        self.problem = None;
        self.note = None;
        let tx = self.tx.clone();
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

    pub(super) fn finish_job(&mut self, result: Result<String, Problem>) {
        self.busy = None;
        self.job_dir = None;
        self.polled = None;
        match result {
            Ok(note) if !note.is_empty() => self.note = Some(note),
            Ok(_) => {}
            Err(problem) => self.problem = Some(problem),
        }
    }
}

impl Launcher {
    /// Reads the open server's settings, backups and service state again.
    pub(super) fn load_details(&mut self) {
        let Some(dir) = self.servers.open.clone() else {
            return;
        };
        self.servers.details_request += 1;
        let request = self.servers.details_request;
        let tx = self.servers.tx.clone();
        std::thread::spawn(move || {
            let config = servers::config(&dir);
            let backups = servers::backups(&dir).unwrap_or_default();
            let service = servers::service_installed(&dir);
            let _ = tx.send(Event::Details {
                dir,
                request,
                config,
                backups,
                service,
            });
        });
    }

    /// Called with the launcher's other work: what the background sent, and the next status poll.
    pub(crate) fn servers_tick(&mut self) -> bool {
        // The Hosting page shows how friends reach the server beside it, whatever the tab.
        let showing = self.ui.page == crate::frame::Page::Hosting;
        let overview = showing && self.servers.open.is_some() && self.servers.tab == Tab::Overview;
        let mut changed = self.internet_tick(overview);
        if !overview && self.servers.public_lookup != PublicLookup::Idle {
            self.servers.reset_public_lookup();
            changed = true;
        }
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
                let previous = self.servers.statuses.get(&dir).cloned();
                std::thread::spawn(move || {
                    let (status, logs) = servers::poll_hosting(&dir, previous.as_ref(), since);
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
            Event::PublicAddress { request, result } => s.finish_public_lookup(request, result),
            Event::Polled { dir, status, logs } => {
                s.polling = false;
                let mut changed = false;
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
                let running = status.phase == "running";
                changed |= s.note_status(dir.clone(), *status);
                if running && s.open.as_ref() == Some(&dir) && s.pvp.is_none() {
                    self.read_pvp();
                    changed = true;
                }
                changed
            }
            Event::Statuses(all) => {
                s.polling = false;
                let mut changed = false;
                for (dir, status) in all {
                    if s.open.as_ref() != Some(&dir) {
                        changed |= s.note_status(dir, status);
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
                s.finish_job(result);
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
                        self.servers.note = Some(trf(
                            "{name} is ready. Press Start, then give your friends the address below.",
                            &[("name", &entry.name)],
                        ));
                    }
                    Err(p) => s.problem = Some(p),
                }
                true
            }
            Event::Details {
                dir,
                request,
                config,
                backups,
                service,
            } => {
                if s.open.as_ref() == Some(&dir) && s.details_request == request {
                    s.config = config;
                    let pending_for_dir = s.applying.as_ref().is_some_and(|b| b.dir == dir)
                        || s.queued.iter().any(|b| b.dir == dir);
                    if !pending_for_dir {
                        if let Some(outcome) = s.settings_outcomes.get_mut(&dir) {
                            outcome.saved.clear();
                        }
                    }
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
                    s.settings_outcomes.remove(&dir);
                    s.open = None;
                    s.reset_public_lookup();
                    self.side_servers = s.list.clone();
                }
                match result {
                    Ok(()) => s.note = Some(tr("Server moved to the trash.").into()),
                    Err(e) => s.problem = Some(Problem::plain(e)),
                }
                true
            }
            Event::Applied { dir, result } => {
                if result.is_ok()
                    && s.open.as_ref() == Some(&dir)
                    && s.applying.as_ref().is_some_and(SettingsBatch::changes_port)
                {
                    s.details_request += 1;
                    if let Some(server) = s
                        .config
                        .as_mut()
                        .and_then(|c| c.get_mut("server")?.as_object_mut())
                    {
                        server.remove("port");
                    }
                }
                let refresh = s.finish_settings(&dir, result);
                self.apply_settings();
                if refresh {
                    self.load_details();
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

    pub(super) fn find_server_public_address(&mut self) {
        if self.ui.page != crate::frame::Page::Hosting
            || self.servers.open.is_none()
            || self.servers.tab != Tab::Overview
        {
            return;
        }
        let Some(request) = self.servers.begin_public_lookup() else {
            return;
        };
        let tx = self.servers.tx.clone();
        std::thread::spawn(move || {
            let result = servers::network::public_ipv4();
            let _ = tx.send(Event::PublicAddress { request, result });
        });
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
}
