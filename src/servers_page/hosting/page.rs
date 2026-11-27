//! The hosting page's layout: the list of servers, one server's page and its tabs.
use super::widgets::*;
use super::*;
use crate::widgets::button::secondary;
use crate::widgets::feedback::empty_state;
use crate::widgets::{chip, nav};
use gpui::SharedString;

impl Launcher {
    /// Escape closes the Create dialog, or takes back a delete that was being confirmed.
    pub(crate) fn hosting_escape(&mut self) -> bool {
        if self.servers.confirm_delete {
            self.servers.confirm_delete = false;
            return true;
        }
        if self.servers.create.is_none() || self.servers.creating {
            return false;
        }
        self.servers.create = None;
        self.servers.problem = None;
        true
    }

    pub(crate) fn hosting(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if !self.servers.loaded {
            self.servers.list = servers::list();
            self.servers.modes.clear();
            self.servers.loaded = true;
            if self.servers.open.is_none() && self.servers.list.len() == 1 {
                let dir = self.servers.list[0].dir.clone();
                self.open_server(dir);
            }
        }
        let open = self
            .servers
            .open
            .clone()
            .filter(|dir| self.servers.list.iter().any(|e| &e.dir == dir));
        // The list of servers fills the page, as Versions' does; one server's page keeps its
        // width.
        let listing = open.is_none();
        let body = match open {
            Some(dir) => self.host_server(&dir, cx),
            None => {
                if self.servers.open.take().is_some() {
                    self.servers.reset_public_lookup();
                }
                self.host_list(cx)
            }
        };
        crate::scrollbar::Scrollbar::page("host")
            .wrap(
                div()
                    .id("alt-hosting-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(24.))
                    .pt(px(18.))
                    .pb(px(24.))
                    .text_color(rgb(palette().text))
                    .child(
                        body.when(!listing, |b| b.max_w(px(1180.)))
                            .flex()
                            .flex_col()
                            .gap(px(18.)),
                    ),
                &self.theme,
            )
            .size_full()
            .into_any_element()
    }

    fn host_problem(&self, cx: &mut Context<Self>) -> Option<Div> {
        let (p, settings_for) = if let Some(problem) = &self.servers.problem {
            (problem.clone(), None)
        } else {
            let dir = self.servers.open.as_ref()?;
            let problem = self.servers.settings_outcomes.get(dir)?.problem.clone()?;
            (problem, Some(dir.clone()))
        };
        let mut actions = div();
        if let Some(fix) = p.fix.clone() {
            let name = match &fix {
                Fix::UsePort(port) => trf("Use port {port}", &[("port", &port.to_string())]),
                Fix::Prepare => tr("Prepare the game files").into(),
                Fix::AgreeEula => tr("I agree to the EULA").into(),
                Fix::ShowLog => tr("Open the logs").into(),
            };
            let settings_for = settings_for.clone();
            actions =
                actions.child(
                    self.host_button("host-fix", name, true, cx, move |this, cx| {
                        this.servers.dismiss_problem(settings_for.as_deref());
                        this.host_fix(fix.clone(), cx);
                    }),
                );
        }
        actions = actions.child(self.host_button(
            "host-problem-dismiss",
            tr("Dismiss"),
            true,
            cx,
            move |this, _| this.servers.dismiss_problem(settings_for.as_deref()),
        ));
        Some(notice(palette().danger, p.message, actions))
    }

    /// The Servers page's fixes; preparing the game files is Play's work in this look.
    pub(super) fn host_fix(&mut self, fix: Fix, cx: &mut Context<Self>) {
        match fix {
            Fix::Prepare if !self.selected().is_some_and(|tag| backend::is_prepared(tag)) => {
                self.servers.problem = None;
                self.servers.create = None;
                self.navigate(crate::frame::Page::Home);
                self.play_game(crate::launcher_play::PlayDestination::Home);
            }
            fix => self.fix(fix, cx),
        }
    }

    fn host_list(&mut self, cx: &mut Context<Self>) -> Div {
        let problem = self
            .host_problem(cx)
            .filter(|_| self.servers.create.is_none());
        if self.servers.list.is_empty() {
            return self.host_empty(problem, cx);
        }
        let mut rows = Vec::new();
        for (i, e) in self.servers.list.clone().into_iter().enumerate() {
            rows.push(self.host_row(i, e.dir, e.name, cx));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(self.host_list_head(cx))
            .children(problem)
            .child(group(rows))
    }

    /// The two ways to add a server: a folder that holds servers, and a new one.
    fn host_add_create(&mut self, cx: &mut Context<Self>) -> (Stateful<Div>, Stateful<Div>) {
        let creating = self.servers.creating;
        let add = self.host_button(
            "host-add",
            tr("Add a server folder…"),
            !creating,
            cx,
            |this, cx| this.add_server_folder(cx),
        );
        let create = self.host_primary(
            "host-create",
            tr("Create a server"),
            true,
            cx,
            |this, cx| this.open_create(cx),
        );
        (add, create)
    }

    /// The page's first line: what hosting is, and the two ways to add a server.
    fn host_list_head(&mut self, cx: &mut Context<Self>) -> Div {
        let (add, create) = self.host_add_create(cx);
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .child(hint(tr(
                "Host a world for your friends on this computer. Create a server, press Start, and give them the address.",
            )))
            .child(div().flex().items_center().gap(px(8.)).child(add).child(create))
    }

    /// No server yet: the empty state carries the two actions, so the page has no separate row
    /// of them.
    fn host_empty(&mut self, problem: Option<Div>, cx: &mut Context<Self>) -> Div {
        let (add, create) = self.host_add_create(cx);
        let actions = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(12.))
            .child(add)
            .child(create)
            .into_any_element();
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .children(problem)
            .child(empty_state(
                tr("No server yet.").trim_end_matches('.'),
                tr("Host a world for your friends on this computer. Create a server, press Start, and give them the address."),
                Some(actions),
            ))
    }

    /// One server of the list: its name and players, its status tag, Start or Stop, and Open.
    /// Start and Stop act on the server they sit beside: it opens first, so the job's progress
    /// shows on its page.
    fn host_row(&mut self, i: usize, dir: PathBuf, name: String, cx: &mut Context<Self>) -> Div {
        let idle = self.servers.busy.is_none();
        let status = self
            .servers
            .statuses
            .get(&dir)
            .cloned()
            .unwrap_or_else(Status::stopped);
        let running = status.running();
        let detail = if running {
            trf(
                "{online} of {max} players",
                &[
                    ("online", &status.players.len().to_string()),
                    ("max", &status.max_players.unwrap_or(0).to_string()),
                ],
            )
        } else {
            format!("{} · {}", self.servers.stopped_line(&dir), short(&dir))
        };
        let kind = if running {
            chip::Kind::Selected
        } else {
            chip::Kind::Neutral
        };
        let tag = chip::tag(phase_label(&status), kind);
        let (start_dir, stop_dir, open_dir) = (dir.clone(), dir.clone(), dir);
        let power = if running {
            self.host_button(
                ElementId::named_usize("host-row-stop", i),
                tr("Stop"),
                idle,
                cx,
                move |this, _| {
                    this.open_server(stop_dir.clone());
                    this.stop_server();
                },
            )
        } else {
            self.host_button(
                ElementId::named_usize("host-row-start", i),
                tr("Start"),
                idle && status.phase == "stopped",
                cx,
                move |this, _| {
                    this.open_server(start_dir.clone());
                    this.start_server();
                },
            )
        };
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
                    .child(hint(detail)),
            )
            .child(tag)
            .child(power)
            .child(self.host_button(
                ElementId::named_usize("host-open", i),
                tr("Open"),
                true,
                cx,
                move |this, _| this.open_server(open_dir.clone()),
            ))
    }

    fn host_server(&mut self, dir: &Path, cx: &mut Context<Self>) -> Div {
        let status = self.servers.status();
        let name = self
            .servers
            .list
            .iter()
            .find(|e| e.dir == dir)
            .map(|e| e.name.clone())
            .unwrap_or_default();
        let running = status.running();
        let idle = self.servers.busy.is_none();
        let mut said = phase_label(&status);
        if let Some(up) = status.uptime.filter(|_| running) {
            said = trf(
                "{phase} · up {time}",
                &[("phase", &said), ("time", &uptime(up))],
            );
        }
        let start = if running {
            self.host_button("host-stop", tr("Stop"), idle, cx, |this, _| {
                this.stop_server()
            })
        } else {
            // Only a server the engine reported stopped: an unknown status (not polled yet, or a
            // failed poll) may be a server started elsewhere, which Start would claim as this
            // launcher's and stop when it quits.
            let stopped = status.phase == "stopped";
            self.host_primary("host-start", tr("Start"), idle && stopped, cx, |this, _| {
                this.start_server()
            })
        };
        let restart = self.host_button(
            "host-restart",
            tr("Restart"),
            idle && running,
            cx,
            |this, _| this.restart_server(),
        );
        let panel_dir = dir.to_path_buf();
        let panel = self.host_button(
            "host-panel",
            tr("Web panel"),
            running,
            cx,
            move |this, cx| match servers::Api::open(&panel_dir).map(|a| a.panel_link()) {
                Some(Ok(url)) => cx.open_url(&url),
                Some(Err(e)) => this.servers.problem = Some(Problem::plain(e)),
                None => {}
            },
        );
        let folder = dir.to_path_buf();
        let (listener, _) = self.servers.share_address(&status);
        let address = self.share_text(listener);
        let head = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(title(name))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(10.))
                            .child(chip::tag(
                                said,
                                if running {
                                    chip::Kind::Selected
                                } else {
                                    chip::Kind::Neutral
                                },
                            ))
                            .children(address.map(|address| {
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(
                                        div()
                                            .font_family(theme::MONO)
                                            .text_size(px(tokens::LABEL))
                                            .text_color(rgb(palette().muted))
                                            .child(address.clone()),
                                    )
                                    .child(crate::widgets::motion_bits::copy_button(
                                        "host-header-copy",
                                        tr("Copy"),
                                        address,
                                    ))
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    // One height along the row: Start is the field's height, so are the rest.
                    .child(
                        secondary("host-folder", tr("Open folder"))
                            .h(px(tokens::FIELD))
                            .on_click(move |_, _, cx| crate::toast::open(cx, &folder)),
                    )
                    .child(panel.h(px(tokens::FIELD)))
                    .child(restart.h(px(tokens::FIELD)))
                    .child(
                        start
                            .h(px(tokens::FIELD))
                            .min_w(px(96.))
                            .text_size(px(tokens::BODY)),
                    ),
            );
        let mut page = div();
        if self.servers.list.len() > 1 {
            page = page.child(
                div().flex().child(
                    div()
                        .id("host-back")
                        .reach()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(px(tokens::LABEL))
                        .text_color(rgb(palette().muted))
                        .cursor_pointer()
                        .child("‹")
                        .child(tr("All servers"))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.servers.open = None;
                            this.servers.reset_public_lookup();
                            this.servers.polled = None;
                            this.servers.problem = None;
                            this.servers.note = None;
                            cx.notify();
                        })),
                ),
            );
        }
        page = page.child(head);
        if status.crashes > 0 {
            let last = status
                .last_crash
                .as_ref()
                .map(|c| trf(" Last: {crash}.", &[("crash", c)]))
                .unwrap_or_default();
            page = page.child(hint(format!(
                "{}{last}",
                trf(
                    "The game crashed {count} times and was restarted by itself.",
                    &[("count", &status.crashes.to_string())],
                )
            )));
        }
        if let Some(doing) = &self.servers.busy {
            let last = self.servers.job_log.back().cloned().unwrap_or_default();
            page = page.child(notice(
                palette().warning,
                format!("{}… {last}", tr(doing)),
                div(),
            ));
        }
        page = page.children(self.host_problem(cx));
        if let Some(note) = self.servers.note.clone() {
            page = page.child(notice(palette().success, note, div()));
        }
        if let Some(outcome) = self
            .servers
            .settings_outcomes
            .get(dir)
            .filter(|outcome| running && !outcome.restart.is_empty())
        {
            let names: Vec<String> = outcome
                .restart
                .iter()
                .map(|k| tr(setting_name(k)).to_string())
                .collect();
            page = page.child(notice(
                palette().warning,
                trf(
                    "Saved. Restart to apply: {settings}.",
                    &[("settings", &names.join(", "))],
                ),
                div().child(self.host_button(
                    "host-restart-apply",
                    tr("Restart now"),
                    idle,
                    cx,
                    |this, _| this.restart_server(),
                )),
            ));
        }
        let chosen = self.servers.tab;
        let tabs: Vec<(Tab, SharedString)> = TABS
            .into_iter()
            .map(|(tab, name)| (tab, tr(name).into()))
            .collect();
        let this = cx.entity();
        let tabs = div()
            .flex()
            .child(nav::tabs("host-tab", &tabs, chosen, move |tab, _, cx| {
                this.update(cx, |this, cx| {
                    if this.servers.tab == Tab::Overview && tab != Tab::Overview {
                        this.servers.reset_public_lookup();
                    }
                    this.servers.tab = tab;
                    this.servers.confirm_restore = None;
                    cx.notify();
                })
            }));
        let body = match self.servers.tab {
            Tab::Overview => self.host_overview(&status, cx),
            Tab::Players => self.host_players(dir, &status, cx),
            Tab::Settings => self.host_settings(&status, cx),
            Tab::Backups => self.host_backups(&status, cx),
        };
        page.child(tabs).child(body)
    }

    fn host_overview(&mut self, status: &Status, cx: &mut Context<Self>) -> Div {
        let (listener, saved) = self.servers.share_address(status);
        let here = self.host_address_card(listener, saved, status, cx);
        let on = status.running();
        let dash = || "–".to_string();
        let stat = |value: String, name: &str| {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(tokens::HEADING))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(value),
                )
                .child(caption(name))
        };
        let stats = card()
            .flex_row()
            .gap(px(12.))
            .p(px(16.))
            .child(stat(
                if on {
                    format!(
                        "{} / {}",
                        status.players.len(),
                        status.max_players.unwrap_or(0)
                    )
                } else {
                    dash()
                },
                tr("Players"),
            ))
            .child(stat(
                status
                    .tps
                    .filter(|_| on)
                    .map_or_else(dash, |v| format!("{v:.1}")),
                tr("TPS of 20"),
            ))
            .child(stat(
                status.mspt.filter(|_| on).map_or_else(dash, |v| {
                    trf("{value} ms", &[("value", &format!("{v:.1}"))])
                }),
                tr("Tick time"),
            ))
            .child(stat(
                status.ram_bytes.filter(|_| on).map_or_else(dash, |b| {
                    trf(
                        "{value} GB",
                        &[("value", &format!("{:.2}", b as f64 / 1e9))],
                    )
                }),
                tr("Memory"),
            ));
        let mut right = div().flex_1().min_w_0().flex().flex_col().gap(px(14.));
        if !status.players.is_empty() {
            let mut who = div().flex().flex_wrap().gap(px(14.));
            for p in status.players.clone() {
                who = who.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(self.head(&p.uuid, 20.))
                        .child(p.name),
                );
            }
            right = right.child(
                card()
                    .gap(px(10.))
                    .p(px(16.))
                    .child(label("Online now"))
                    .child(who),
            );
        }
        right = right.child(self.host_console(on, cx));
        let sharing = self.hosting_share(listener, cx);
        div().flex().flex_col().gap(px(14.)).child(stats).child(
            div()
                .flex()
                .items_start()
                .gap(px(14.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(14.))
                        .child(here)
                        .child(sharing),
                )
                .child(right),
        )
    }

    fn host_backups(&mut self, status: &Status, cx: &mut Context<Self>) -> Div {
        let idle = self.servers.busy.is_none();
        let every = self.servers.shown("backups", "every");
        let auto = !every.is_empty() && every != "0";
        let keep = self.servers.shown("backups", "keep");
        let now = row(
            tr("Back up now"),
            tr(if status.running() {
                "Players stay online: the world is saved, then zipped."
            } else {
                "Zips the world and the server's settings into its backups folder."
            }),
            div().child(self.host_primary(
                "host-backup-now",
                tr("Back up now"),
                idle,
                cx,
                |this, _| {
                    this.server_command(crate::i18n::tr_key("Backing up"), vec!["backup".into()])
                },
            )),
        );
        if self.older_sunrise() {
            return div()
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(group(vec![now]))
                .child(hint(tr(
                    "This server's Sunrise is too old to list its backups here: they're in its backups folder. Update it below.",
                )))
                .child(self.host_updates(status, cx));
        }
        let automatic = row(
            tr("Back up automatically"),
            if auto {
                trf(
                    "On: every {every} while the server runs, keeping the last {keep}. Backups you make yourself are never deleted.",
                    &[("every", &every), ("keep", &keep)],
                )
            } else {
                tr("Off: only the backups you make.").to_string()
            },
            div().child(switch("host-auto-backup", auto).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    this.set_setting("backups.every", if auto { "0".into() } else { "6h".into() });
                    cx.notify();
                },
            ))),
        );
        let mut list = Vec::new();
        for (i, b) in self.servers.backups.clone().into_iter().enumerate() {
            let (when, kind) = backup_label(&b);
            let confirming = self.servers.confirm_restore.as_deref() == Some(b.name.as_str());
            let name = b.name.clone();
            let restored = trf(
                "The world is back as it was on {when}. The one it replaced is in the backups folder.",
                &[("when", &when)],
            );
            let restore = self.host_action(
                if confirming {
                    crate::widgets::button::Style::Danger
                } else {
                    crate::widgets::button::Style::Secondary
                },
                ElementId::named_usize("host-restore", i),
                tr(if confirming {
                    "Replace the world"
                } else {
                    "Restore…"
                }),
                idle,
                cx,
                move |this, _| {
                    if this.servers.confirm_restore.as_ref() != Some(&name) {
                        this.servers.confirm_restore = Some(name.clone());
                        return;
                    }
                    this.servers.confirm_restore = None;
                    if let Some(dir) = this.servers.open.clone() {
                        let (name, restored) = (name.clone(), restored.clone());
                        this.servers
                            .start_job(crate::i18n::tr_key("Restoring"), move |log| {
                                servers::run_streamed(&dir, &["restore", &name, "--yes"], log)
                                    .map(|_| restored)
                            });
                    }
                },
            );
            let mut actions = div();
            if confirming {
                actions = actions.child(self.host_button(
                    "host-restore-no",
                    tr("Keep the world"),
                    true,
                    cx,
                    |this, _| this.servers.confirm_restore = None,
                ));
            }
            let detail = if kind.is_empty() {
                bytes(b.bytes)
            } else {
                format!("{kind} · {}", bytes(b.bytes))
            };
            list.push(row(
                when,
                if confirming {
                    tr("Restoring replaces the world with this backup. The world as it is now isn't deleted: it's moved into the backups folder. A running server stops and starts again.").to_string()
                } else {
                    detail
                },
                actions.child(restore),
            ));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(group(vec![now, automatic]))
            .child(if list.is_empty() {
                card().p(px(16.)).child(hint(tr("No backup yet.")))
            } else {
                group(list)
            })
            .child(self.host_updates(status, cx))
    }

    fn host_updates(&mut self, status: &Status, cx: &mut Context<Self>) -> Div {
        let idle = self.servers.busy.is_none();
        let check = self.servers.update.clone();
        let release = status
            .release
            .clone()
            .or_else(|| {
                check
                    .as_ref()
                    .and_then(|u| u["installed"].as_str().map(str::to_string))
            })
            .or_else(servers::runtime_release)
            .unwrap_or_else(|| "?".into());
        let available = status.update_available.clone().or_else(|| {
            check
                .as_ref()
                .filter(|u| u["available"] == true)
                .and_then(|u| u["newest"].as_str().map(str::to_string))
        });
        let about = match (&available, &check, &status.update_last) {
            (Some(tag), _, _) => trf(
                "Update available: Sunrise {tag} (this server runs {release}). Updating backs up the world first, and goes back by itself if the new version doesn't start.",
                &[("tag", tag), ("release", &release)],
            ),
            (None, Some(_), _) => trf("Up to date: Sunrise {release}.", &[("release", &release)]),
            (None, None, Some((last, _))) => {
                trf("Sunrise {release}. {last}", &[("release", &release), ("last", last)])
            }
            (None, None, None) => trf("Sunrise {release}.", &[("release", &release)]),
        };
        let control = match available {
            Some(tag) => self.host_primary(
                "host-update-now",
                tr("Update now"),
                idle,
                cx,
                move |this, _| {
                    this.servers.update = None;
                    if let Some(dir) = this.servers.open.clone() {
                        let done = trf(
                            "Updated to Sunrise {tag}. The world was backed up first (Backups).",
                            &[("tag", &tag)],
                        );
                        this.servers
                            .start_job(trf("Updating to {tag}", &[("tag", &tag)]), move |log| {
                                servers::run_streamed(&dir, &["update"], log).map(|_| done)
                            });
                    }
                },
            ),
            None => {
                let checking = self.servers.checking;
                self.host_button(
                    "host-update-check",
                    tr(if checking {
                        "Checking…"
                    } else {
                        "Check for updates"
                    }),
                    idle && !checking,
                    cx,
                    |this, _| {
                        let Some(dir) = this.servers.open.clone() else {
                            return;
                        };
                        this.servers.checking = true;
                        let tx = this.servers.tx.clone();
                        std::thread::spawn(move || {
                            let result = servers::run(&dir, &["update", "--check", "--json"])
                                .and_then(|out| {
                                    serde_json::from_str(out.trim())
                                        .map_err(|e| Problem::plain(e.to_string()))
                                });
                            let _ = tx.send(Event::UpdateCheck(result));
                        });
                    },
                )
            }
        };
        let mut rows = vec![row(
            tr("This server's Sunrise"),
            about,
            div().child(control),
        )];
        if self.servers.setting("updates", "keep").is_some() {
            rows.push(row(
                tr("Older versions kept"),
                tr("Downloaded versions kept besides the one in use and the one an update can go back to."),
                self.host_stepper("updates.keep", 0..=5, cx),
            ));
        }
        group(rows)
    }
}
