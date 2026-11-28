//! The Servers page's actions: start, stop and restart, fix a problem, create a server and add a
//! folder, and the commands that drive them.

use super::*;

impl Launcher {
    /// `sunrise-server <args>` on the open server, as a job.
    pub(super) fn server_command(&mut self, label: &str, args: Vec<String>) {
        let Some(dir) = self.servers.open.clone() else {
            return;
        };
        let tx = self.tx.clone();
        self.servers.start_job(label.to_string(), move |log| {
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

    pub(super) fn open_server(&mut self, dir: PathBuf) {
        if self.servers.open.as_ref() == Some(&dir) {
            return;
        }
        let s = &mut self.servers;
        s.statuses.remove(&dir);
        s.open = Some(dir);
        s.internet_address = false;
        s.reset_public_lookup();
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
        s.texts.remove(&Field::Motd);
        s.texts.remove(&Field::Port);
        s.invalid = None;
        s.polled = None;
        self.load_details();
    }

    /// A command for the running game, through the control API; its answer shows in the console.
    pub(super) fn api_command(&mut self, line: String) {
        let Some(dir) = self.servers.open.clone() else {
            return;
        };
        let tx = self.servers.tx.clone();
        self.servers.log.push_back(format!("> {line}"));
        self.servers.console_scroll.handle.scroll_to_bottom();
        std::thread::spawn(move || {
            let result = servers::Api::open(&dir)
                .ok_or_else(|| tr("The server isn't running.").to_string())
                .and_then(|a| a.command(&line));
            if let Err(e) = result {
                let _ = tx.send(Event::Failed(e));
            }
        });
    }

    pub(super) fn start_server(&mut self) {
        if let Some(dir) = self.servers.open.clone() {
            self.start_server_in(dir);
        }
    }

    /// Starts the server in `dir`, open or not: its card on the list shows how it goes.
    pub(super) fn start_server_in(&mut self, dir: PathBuf) {
        if self.servers.busy.is_some() {
            return;
        }
        recover(&STARTED_HERE).push(dir.clone());
        if let Some(outcome) = self.servers.settings_outcomes.get_mut(&dir) {
            outcome.restart.clear();
        }
        let tx = self.tx.clone();
        let job = dir.clone();
        self.servers.start_job("Starting", move |log| {
            if let Some(api) = servers::Api::open(&job) {
                if let Ok(status) = api.call("GET", "/api/v1/status", None) {
                    if status["phase"] == "stopped"
                        && api.call("POST", "/api/v1/start", None).is_ok()
                    {
                        return Ok(String::new());
                    }
                }
            }
            log(tr("A new world takes a minute the first time").into());
            run_managed_server(&tx, &job, &["start"], log).map(|_| String::new())
        });
        self.servers.job_dir = Some(dir);
    }

    pub(super) fn stop_server(&mut self) {
        if let Some(dir) = self.servers.open.clone() {
            self.stop_server_in(dir);
        }
    }

    /// Stops the server in `dir`, open or not.
    pub(super) fn stop_server_in(&mut self, dir: PathBuf) {
        if self.servers.busy.is_some() {
            return;
        }
        let job = dir.clone();
        self.servers.start_job("Stopping", move |log| {
            // The running server's API first; when it can't be reached or refuses, the CLI,
            // whose error is the job's.
            let via_api = servers::Api::open(&job)
                .is_some_and(|api| api.call("POST", "/api/v1/stop", None).is_ok());
            if via_api {
                return Ok(String::new());
            }
            servers::run_streamed(&job, &["stop"], log).map(|_| String::new())
        });
        self.servers.job_dir = Some(dir);
    }

    pub(super) fn restart_server(&mut self) {
        let Some(dir) = self.servers.open.clone() else {
            return;
        };
        if let Some(outcome) = self.servers.settings_outcomes.get_mut(&dir) {
            outcome.restart.clear();
        }
        let tx = self.tx.clone();
        self.servers.start_job("Restarting", move |log| {
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

    pub(super) fn fix(&mut self, fix: Fix, cx: &mut Context<Self>) {
        self.servers.problem = None;
        match fix {
            Fix::UsePort(port) => {
                self.set_setting("server.port", port.to_string());
                self.servers.texts.remove(&Field::Port);
                self.servers.note = Some(trf(
                    "The server now uses port {port}. Press Start again.",
                    &[("port", &port.to_string())],
                ));
            }
            Fix::Prepare => match self.selected().cloned() {
                Some(tag) if backend::is_prepared(&tag) => {
                    self.servers.start_job(
                        crate::i18n::tr_key("Copying the game files"),
                        move |log| {
                            log(trf(
                                "Copying the game files prepared for Sunrise {tag}",
                                &[("tag", &tag)],
                            ));
                            servers::set_up(&tag)
                                .map(|_| {
                                    tr("The game files are ready. Press Start again.").to_string()
                                })
                                .map_err(Problem::plain)
                        },
                    );
                }
                _ => {
                    self.servers.create = None;
                    self.go(Page::Play);
                    if !self.preparing {
                        self.prepare();
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
                    crate::toast::open(cx, &dir.join("logs"));
                }
            }
        }
    }

    pub(super) fn open_create(&mut self, cx: &mut Context<Self>) {
        self.focus_handle(Field::Name, cx);
        self.servers.focus_soon = Some(Field::Name);
        let worlds = self.create_worlds();
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
            world: WorldChoice::New,
            worlds,
            menu: None,
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

    /// The worlds of every profile the dialog can copy from, the last played first. A profile's
    /// world is told by its name and the profile's.
    fn create_worlds(&self) -> Vec<CreateWorld> {
        let mut homes: Vec<(String, PathBuf)> = self
            .instances
            .list
            .iter()
            .map(|i| (i.name.clone(), i.home.clone()))
            .collect();
        if homes.is_empty() {
            homes.push((String::new(), backend::game_home()));
        }
        let mut seen: Vec<PathBuf> = Vec::new();
        let mut found = Vec::new();
        for (profile, home) in homes {
            if seen.contains(&home) {
                continue;
            }
            seen.push(home.clone());
            for w in servers::player_world_list(&home) {
                let hint = format!(
                    "{} · {}",
                    tr(w.mode),
                    crate::worlds_page::ago(w.last_played)
                );
                found.push((
                    w.last_played,
                    CreateWorld {
                        name: w.name,
                        profile: profile.clone(),
                        path: w.path,
                        hint,
                    },
                ));
            }
        }
        found.sort_by_key(|(played, _)| std::cmp::Reverse(*played));
        found.into_iter().map(|(_, world)| world).collect()
    }

    /// Asks for a world file or folder, and makes it the new server's world.
    pub(super) fn pick_create_world(&mut self, cx: &mut Context<Self>) {
        let pick = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: false,
            prompt: Some(tr("Add world").into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = pick.await {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update(cx, |this, cx| {
                        if let Some(c) = this.servers.create.as_mut() {
                            c.world = WorldChoice::Import(path);
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn create_server(&mut self) {
        if self.servers.creating {
            return;
        }
        let Some(c) = &mut self.servers.create else {
            return;
        };
        if !c.eula {
            c.eula_missing = true;
            // The box is the form's last line, under the fold in a small window: bring it up
            // to where the footer's message points.
            // (The dialog is a sheet, and `modal_frame` gives it its handle.)
            if let Some(body) = crate::frame::drawn_scroll("sheet") {
                body.scroll_to_bottom();
            }
            return;
        }
        let Some(c) = self.servers.create.as_ref() else {
            return;
        };
        let world = match &c.world {
            WorldChoice::Copy(dir) => World::Copy(dir.clone()),
            WorldChoice::Import(path) => World::Import(path.clone()),
            WorldChoice::New => World::New {
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

    pub(super) fn add_server_folder(&mut self, cx: &mut Context<Self>) {
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
                        tag.ok_or_else(|| Problem::plain(tr("Install a Sunrise version first.")))?;
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
}
