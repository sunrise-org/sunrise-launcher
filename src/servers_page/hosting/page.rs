//! One hosted server's page and its tabs (the list of servers is Home's Servers tab).
use super::widgets::*;
use super::*;
use crate::widgets::button::secondary;
use crate::widgets::{chip, nav};
use gpui::SharedString;

impl Launcher {
    /// Escape closes the card menu or its dialog, the Create dialog, or takes back a delete that
    /// was being confirmed.
    pub(crate) fn hosting_escape(&mut self) -> bool {
        if self.servers.menu.take().is_some() {
            crate::glass_ui::menus_changed();
            return true;
        }
        if self.servers.dialog.take().is_some() {
            return true;
        }
        if self.servers.confirm_delete {
            self.servers.confirm_delete = false;
            return true;
        }
        if self.servers.create.is_none() || self.servers.creating {
            return false;
        }
        // An open dropdown of the Create dialog goes first.
        if self
            .servers
            .create
            .as_mut()
            .is_some_and(|c| c.menu.take().is_some())
        {
            return true;
        }
        self.servers.create = None;
        self.servers.problem = None;
        true
    }

    /// Whether a hosted server's page has a server to show: it opens from a card on Home's
    /// Servers tab, and the page has nothing without one.
    pub(crate) fn hosted_open(&self) -> bool {
        self.servers
            .open
            .as_ref()
            .is_some_and(|dir| self.servers.list.iter().any(|e| &e.dir == dir))
    }

    /// The open server's name, for the page's title.
    pub(crate) fn hosted_title(&self) -> String {
        let open = self.servers.open.as_ref();
        self.servers
            .list
            .iter()
            .find(|e| Some(&e.dir) == open)
            .map(|e| e.name.clone())
            .unwrap_or_default()
    }

    /// Back to Home's Servers tab, where the server's card is.
    pub(crate) fn leave_hosted(&mut self) {
        self.servers.open = None;
        self.servers.reset_public_lookup();
        self.servers.polled = None;
        self.servers.problem = None;
        self.servers.note = None;
        self.show_saved_servers();
    }

    pub(crate) fn hosting(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.load_hosted();
        let open = self
            .servers
            .open
            .clone()
            .filter(|dir| self.servers.list.iter().any(|e| &e.dir == dir));
        let body = match open {
            Some(dir) => self.host_server(&dir, cx),
            None => div(),
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
                    .child(body.max_w(px(1180.)).flex().flex_col().gap(px(18.))),
                &self.theme,
            )
            .size_full()
            .into_any_element()
    }

    pub(super) fn host_problem(&self, cx: &mut Context<Self>) -> Option<Div> {
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
