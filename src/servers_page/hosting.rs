//! Hosting in the new look: the servers this computer hosts, and one of them at a time with its
//! address, players, settings, backups and log. Every action is the Servers page's own
//! (`servers_page.rs`); only the drawing is new.
use super::*;
use crate::frame::button;
use crate::mods::{choices, switch, Choose};
use crate::paint::palette;
use crate::settings::{danger, group, hint, off, row, warning};
use gpui::rgb;
use std::rc::Rc;

fn label(text: &'static str) -> Div {
    caption(tr(text))
}

/// A small heading in capitals; `text` is already in the player's language.
fn caption(text: &str) -> Div {
    div()
        .text_size(px(tokens::CAPTION))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(palette().muted))
        .child(crate::selectable::selectable(text.to_uppercase()))
}

fn card() -> Div {
    div()
        .flex()
        .flex_col()
        .rounded(crate::tokens::r(crate::tokens::R_CARD))
        .border_1()
        .border_color(rgb(palette().divider))
        .bg(palette().card())
}

/// The main action of the page, in cream.
fn primary(b: Stateful<Div>) -> Stateful<Div> {
    b.bg(rgb(palette().text))
        .border_color(rgb(palette().text))
        .text_color(rgb(palette().background))
}

fn dot(color: u32) -> Div {
    div()
        .flex_none()
        .size(px(8.))
        .rounded(crate::tokens::r(crate::tokens::R_SMALL))
        .bg(rgb(color))
}

/// A line that says something happened, with what can be done about it.
fn notice(color: u32, text: impl Into<SharedString>, actions: Div) -> Div {
    card()
        .flex_row()
        .items_center()
        .gap(px(12.))
        .px(px(16.))
        .py(px(12.))
        .child(dot(color))
        .child(div().flex_1().min_w_0().line_clamp(2).child(text.into()))
        .child(actions.flex_none().flex().items_center().gap(px(6.)))
}

/// One choice among `options`, as chips. The group is one Tab stop with the ring around it, and
/// the arrow keys move the choice within it.
fn chip_group(
    id: &'static str,
    options: &[(&'static str, &'static str)],
    current: &str,
    cx: &mut Context<Launcher>,
    set: impl Fn(&mut Launcher, &'static str) + Clone + 'static,
) -> Div {
    let chips = options
        .iter()
        .map(|&(value, name)| {
            let set = set.clone();
            let pick: Choose = Rc::new(move |this, _| set(this, value));
            (tr(name).into(), value == current, pick)
        })
        .collect();
    div().flex().child(choices(id, chips, cx).gap(px(6.)))
}

fn phase_color(status: &Status) -> u32 {
    match status.phase.as_str() {
        "running" if status.crashes > 0 => palette().warning,
        "running" => palette().success,
        "stopped" => palette().line,
        _ => palette().warning,
    }
}

impl Launcher {
    /// A text field of the Servers page, drawn in this look.
    fn host_field(
        &mut self,
        f: Field,
        placeholder: &'static str,
        width: f32,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let focused = self.servers.has_focus(f);
        let invalid = self.servers.invalid.is_some_and(|(g, _)| g == f);
        self.field(f, placeholder, width, cx)
            .h(px(tokens::FIELD))
            .px(px(12.))
            .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
            .border_1()
            .border_color(rgb(if invalid {
                palette().danger
            } else if focused {
                palette().focus
            } else {
                palette().line
            }))
            .bg(rgb(palette().field))
            .text_size(px(tokens::LABEL))
            .text_color(rgb(palette().text))
    }

    fn host_button(
        &self,
        id: impl Into<ElementId>,
        text: impl Into<SharedString>,
        enabled: bool,
        cx: &mut Context<Self>,
        action: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        let b = button(id, text);
        if enabled {
            b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                action(this, cx);
                cx.notify();
            }))
        } else {
            off(b)
        }
    }

    /// − value + for a number setting within `range`.
    fn host_stepper(
        &self,
        key: &'static str,
        range: std::ops::RangeInclusive<i64>,
        cx: &mut Context<Self>,
    ) -> Div {
        let (section, field) = key.split_once('.').unwrap_or_default();
        let value: i64 = self.servers.shown(section, field).parse().unwrap_or(0);
        let step = |id: &'static str, sign: &'static str, to: i64, cx: &mut Context<Self>| {
            self.host_button(
                SharedString::from(format!("{key}-{id}")),
                sign,
                range.contains(&to),
                cx,
                move |this, _| this.set_setting(key, to.to_string()),
            )
            .w(px(32.))
            .px(px(0.))
        };
        div()
            .child(step("less", "−", value - 1, cx))
            .child(
                div()
                    .w(px(36.))
                    .flex()
                    .justify_center()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(value.to_string()),
            )
            .child(step("more", "+", value + 1, cx))
    }

    fn host_choice(
        &self,
        key: &'static str,
        options: &[(&'static str, &'static str)],
        cx: &mut Context<Self>,
    ) -> Div {
        let (section, field) = key.split_once('.').unwrap_or_default();
        let current = self.servers.shown(section, field);
        chip_group(key, options, &current, cx, move |this, value| {
            this.set_setting(key, value.to_string())
        })
    }

    /// Escape closes Create or cancels a pending removal.
    pub(crate) fn hosting_escape(&mut self) -> bool {
        if self.servers.review_unregister.take().is_some() {
            return true;
        }
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

    // ----- The page -------------------------------------------------------------------------

    pub(crate) fn hosting(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if !self.servers.loaded {
            self.servers.list = servers::list();
            self.servers
                .selected_hosts
                .retain(|entry| self.servers.list.contains(entry));
            self.servers.loaded = true;
        }
        let open = self
            .servers
            .open
            .clone()
            .filter(|dir| self.servers.list.iter().any(|e| &e.dir == dir));
        let Some(dir) = open else {
            self.servers.open = None;
            return self.host_list(cx).into_any_element();
        };
        let body = self.host_server(&dir, cx);
        div()
            .id("alt-hosting-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&crate::frame::scrolls("host"))
            .px(px(24.))
            .pt(px(18.))
            .pb(px(24.))
            .text_color(rgb(palette().text))
            .child(body.max_w(px(1180.)).flex().flex_col().gap(px(18.)))
            .into_any_element()
    }

    fn host_problem(&self, cx: &mut Context<Self>) -> Option<Div> {
        let p = self.servers.problem.clone()?;
        let mut actions = div();
        if let Some(fix) = p.fix.clone() {
            let name = match &fix {
                Fix::UsePort(port) => trf("Use port {port}", &[("port", &port.to_string())]),
                Fix::Prepare => tr("Prepare the game files").into(),
                Fix::AgreeEula => tr("I agree to the EULA").into(),
                Fix::ShowLog => tr("Open the logs").into(),
            };
            actions =
                actions.child(
                    self.host_button("host-fix", name, true, cx, move |this, cx| {
                        this.host_fix(fix.clone(), cx)
                    }),
                );
        }
        let summary = p.message.lines().next().unwrap_or_default().to_owned();
        if summary != p.message || summary.chars().count() > 60 {
            actions = actions.child(self.host_button(
                "host-problem-details",
                tr("Details…"),
                true,
                cx,
                move |this, _| {
                    this.open_notice(p.message.clone(), None);
                },
            ));
        }
        actions = actions.child(self.host_button(
            "host-problem-dismiss",
            tr("Dismiss"),
            true,
            cx,
            |this, _| this.servers.problem = None,
        ));
        Some(notice(palette().danger, summary, actions))
    }

    /// The Servers page's fixes; preparing the game files is Play's work in this look.
    fn host_fix(&mut self, fix: Fix, cx: &mut Context<Self>) {
        match fix {
            Fix::Prepare if !self.selected().is_some_and(|tag| backend::is_prepared(tag)) => {
                self.servers.problem = None;
                self.servers.create = None;
                self.ui.page = crate::frame::Page::Home;
                self.play_game();
            }
            fix => self.fix(fix, cx),
        }
    }

    fn host_list(&mut self, cx: &mut Context<Self>) -> Div {
        let narrow = self.servers_narrow();
        let query = self.servers.value(Field::HostSearch).trim().to_lowercase();
        let visible: Vec<_> = self
            .servers
            .list
            .iter()
            .filter(|entry| {
                entry.name.to_lowercase().contains(&query)
                    || entry.dir.to_string_lossy().to_lowercase().contains(&query)
                    || entry
                        .host
                        .as_ref()
                        .is_some_and(|host| host.to_lowercase().contains(&query))
            })
            .cloned()
            .collect();
        let idle = !self.servers.creating && self.servers.busy.is_none();
        let toolbar = collection::toolbar()
            .child(
                self.host_field(
                    Field::HostSearch,
                    tr("Search hosted servers"),
                    if narrow { 195. } else { 420. },
                    cx,
                )
                .flex_1()
                .min_w_0()
                .h(px(36.)),
            )
            .child(
                self.host_button("host-add", tr("Add folder…"), idle, cx, |this, cx| {
                    this.add_server_folder(cx)
                })
                .h(px(36.)),
            )
            .child(
                primary(
                    self.host_button("host-create", "", idle, cx, |this, cx| this.open_create(cx)),
                )
                .h(px(36.))
                .gap(px(6.))
                .child(crate::glass_ui::icon(
                    "plus-sign",
                    14.,
                    rgb(palette().background).into(),
                ))
                .child(tr("Create server")),
            );
        let selection = if let Some(remove) = &self.servers.review_unregister {
            collection::confirmation(
                "host-unregister-review",
                if remove.len() == 1 {
                    tr("Unregister this server? Its files stay.").into()
                } else {
                    trf(
                        "Unregister {count} servers? Their files stay.",
                        &[("count", &remove.len().to_string())],
                    )
                },
                tr("Unregister"),
                |this| this.servers.review_unregister = None,
                Launcher::unregister_hosted,
                cx,
            )
        } else {
            let selected = self.servers.selected_hosts.len();
            let all = !visible.is_empty()
                && visible
                    .iter()
                    .all(|entry| self.servers.selected_hosts.contains(entry));
            let check = collection::check("host-select-all", all, tr("Select all shown servers"));
            let check = if idle && !visible.is_empty() {
                let visible = visible.clone();
                check.on_click(cx.listener(move |this, _, _, cx| {
                    if all {
                        for entry in &visible {
                            this.servers.selected_hosts.remove(entry);
                        }
                    } else {
                        this.servers.selected_hosts.extend(visible.iter().cloned());
                    }
                    cx.notify();
                }))
            } else {
                check.opacity(0.45).tab_stop(false)
            };
            let label = if selected > 0 {
                trf("{count} selected", &[("count", &selected.to_string())])
            } else if visible.len() != self.servers.list.len() {
                trf(
                    "{shown} of {total} hosted servers",
                    &[
                        ("shown", &visible.len().to_string()),
                        ("total", &self.servers.list.len().to_string()),
                    ],
                )
            } else if visible.len() == 1 {
                tr("1 hosted server").into()
            } else {
                trf(
                    "{count} hosted servers",
                    &[("count", &visible.len().to_string())],
                )
            };
            let actions = if selected > 0 && idle {
                collection::bulk_actions(
                    "host-selection",
                    tr("Unregister selected"),
                    |this| this.servers.selected_hosts.clear(),
                    |this| {
                        this.servers.review_unregister =
                            Some(this.servers.selected_hosts.iter().cloned().collect())
                    },
                    cx,
                )
            } else {
                div()
            };
            collection::selection_bar(check, label, actions)
        };
        let mut controls = div()
            .flex_none()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(selection);
        if let Some(problem) = self
            .host_problem(cx)
            .filter(|_| self.servers.create.is_none())
        {
            controls = controls.child(div().px(px(12.)).pb(px(8.)).child(problem));
        } else if let Some(note) = self.servers.busy.as_ref().or(self.servers.note.as_ref()) {
            controls = controls.child(hint(tr(note)).truncate().px(px(12.)).pb(px(8.)));
        }
        let mut list = collection::rows("host-list");
        if visible.is_empty() {
            list = list.child(
                collection::toolbar()
                    .border_t_1()
                    .border_color(rgb(palette().divider))
                    .child(hint(if query.is_empty() {
                        tr("No hosted servers")
                    } else {
                        tr("No servers match this search")
                    }))
                    .children((!query.is_empty()).then(|| {
                        button("host-clear-search", tr("Clear search")).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.servers.text(Field::HostSearch).set(String::new());
                                cx.notify();
                            },
                        ))
                    })),
            );
        }
        for (i, entry) in visible.into_iter().enumerate() {
            let status = self.servers.statuses.get(&entry.dir);
            let (first, second) = match (entry.host.is_some(), status) {
                (true, _) => (tr("Remote").to_owned(), None),
                (_, None) => (tr("Checking…").to_owned(), None),
                (_, Some(status)) => {
                    let players = (status.phase == "running").then(|| {
                        let online = status.players.len().to_string();
                        match status.max_players {
                            Some(max) => trf(
                                "{online}/{max} players",
                                &[("online", &online), ("max", &max.to_string())],
                            ),
                            None => trf("{count} playing", &[("count", &online)]),
                        }
                    });
                    (
                        tr(match status.phase.as_str() {
                            "running" => "Running",
                            "starting" => "Starting",
                            "restarting" => "Restarting",
                            "stopping" => "Stopping",
                            _ => "Stopped",
                        })
                        .to_owned(),
                        players,
                    )
                }
            };
            let checked = self.servers.selected_hosts.contains(&entry);
            let choose = collection::check(
                ("host-select", i),
                checked,
                trf("Select {name}", &[("name", &entry.name)]),
            );
            let choose = if idle {
                let entry = entry.clone();
                choose.on_click(cx.listener(move |this, _, _, cx| {
                    this.servers.review_unregister = None;
                    if !this.servers.selected_hosts.remove(&entry) {
                        this.servers.selected_hosts.insert(entry.clone());
                    }
                    cx.notify();
                }))
            } else {
                choose.opacity(0.45).tab_stop(false)
            };
            let manage = self.host_button(
                ("host-manage", i),
                tr("Manage"),
                entry.host.is_none(),
                cx,
                {
                    let dir = entry.dir.clone();
                    move |this, _| this.open_server(dir.clone())
                },
            );
            let power = status.filter(|_| !narrow && entry.host.is_none()).map(|s| {
                let running = s.running();
                let label = if running { tr("Stop") } else { tr("Start") };
                let can = idle && matches!(s.phase.as_str(), "running" | "stopped");
                let dir = entry.dir.clone();
                self.host_button(("host-power", i), label, can, cx, move |this, _| {
                    if running {
                        this.stop_hosted(dir.clone());
                    } else {
                        this.start_hosted(dir.clone());
                    }
                })
                .min_w(px(58.))
            });
            let unregister = collection::remove(("host-unregister", i), tr("Unregister server"));
            let unregister = if idle {
                let entry = entry.clone();
                unregister.on_click(cx.listener(move |this, _, _, cx| {
                    this.servers.review_unregister = Some(vec![entry.clone()]);
                    cx.notify();
                }))
            } else {
                unregister.opacity(0.45).tab_stop(false)
            };
            let location = entry.host.as_ref().map_or_else(
                || entry.dir.display().to_string(),
                |host| format!("{host}:{}", entry.dir.display()),
            );
            list = list.child(
                collection::row(narrow)
                    .when(checked, |d| d.bg(rgb(palette().selected)))
                    .child(choose)
                    .child(crate::join::tile(&entry.name, 32.))
                    .child(collection::identity(
                        ("host-identity", i),
                        entry.name,
                        location,
                    ))
                    .child(collection::status(narrow, first, second))
                    .children(power)
                    .child(manage)
                    .child(unregister),
            );
        }
        collection::table(controls, list, "host-list", &self.servers.host_thumb, cx)
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
            primary(
                self.host_button("host-start", tr("Start"), idle, cx, |this, _| {
                    this.start_server()
                }),
            )
        };
        let restart = self.host_button(
            "host-restart",
            tr("Restart"),
            idle && running,
            cx,
            |this, _| {
                this.servers.restart_needed.clear();
                this.restart_server();
            },
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
        let narrow = self.servers_narrow();
        let head = div()
            .flex()
            .when(narrow, |d| d.flex_col().items_start())
            .when(!narrow, |d| d.items_center())
            .gap(px(12.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .max_w_full()
                    .child(
                        div()
                            .id("host-title")
                            .truncate()
                            .text_size(px(tokens::DISPLAY))
                            .font_weight(FontWeight::BOLD)
                            .tooltip(crate::glass_ui::tip(name.clone()))
                            .child(name),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(dot(phase_color(&status)))
                            .child(hint(said)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    // One height along the row: Start is the field's height, so are the rest.
                    .child(
                        button("host-folder", tr("Open folder"))
                            .h(px(tokens::FIELD))
                            .on_click(move |_, _, cx| crate::glass_toast::open(cx, &folder)),
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
        let mut page = div()
            .child(
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
                            this.servers.polled = None;
                            this.servers.problem = None;
                            this.servers.note = None;
                            cx.notify();
                        })),
                ),
            )
            .child(head);
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
        if running && !self.servers.restart_needed.is_empty() {
            let names: Vec<_> = self
                .servers
                .restart_needed
                .iter()
                .map(|key| tr(setting_name(key)))
                .collect();
            let summary = match &names[..] {
                [name] => trf("Restart to apply: {setting}", &[("setting", name)]),
                _ => trf(
                    "Restart to apply {count} settings.",
                    &[("count", &names.len().to_string())],
                ),
            };
            let detail = format!("{summary}\n\n{}", names.join("\n"));
            page = page.child(notice(
                palette().warning,
                summary,
                div()
                    .child(self.host_button(
                        "host-restart-details",
                        tr("Details…"),
                        true,
                        cx,
                        move |this, _| {
                            this.open_details(detail.clone());
                        },
                    ))
                    .child(self.host_button(
                        "host-restart-apply",
                        tr("Restart now"),
                        idle,
                        cx,
                        |this, _| {
                            this.servers.restart_needed.clear();
                            this.restart_server();
                        },
                    )),
            ));
        }
        let chosen = self.servers.tab;
        let tabs = TABS
            .into_iter()
            .map(|(tab, name)| {
                let pick: Choose = Rc::new(move |this, _| {
                    this.servers.tab = tab;
                    this.servers.confirm_restore = None;
                });
                (tr(name).into(), tab == chosen, pick)
            })
            .collect();
        let tabs = div()
            .flex()
            .child(choices("host-tab", tabs, cx).gap(px(6.)));
        let body = match self.servers.tab {
            Tab::Overview => self.host_overview(&status, cx),
            Tab::Players => self.host_players(dir, &status, cx),
            Tab::Settings => self.host_settings(&status, cx),
            Tab::Backups => self.host_backups(&status, cx),
        };
        page.child(tabs).child(body)
    }

    // ----- Overview -------------------------------------------------------------------------

    fn host_overview(&mut self, status: &Status, cx: &mut Context<Self>) -> Div {
        let port = self
            .servers
            .setting("server", "port")
            .and_then(Value::as_u64)
            .unwrap_or(25565);
        let address = match servers::lan_address() {
            Some(ip) if port == 25565 => ip.to_string(),
            Some(ip) => format!("{ip}:{port}"),
            None => format!("localhost:{port}"),
        };
        let copied = self
            .servers
            .copied
            .is_some_and(|at| at.elapsed() < Duration::from_secs(2));
        let copy = address.clone();
        let here = card()
            .gap(px(8.))
            .p(px(16.))
            .child(label("Address on this network"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(theme::MONO)
                            .text_size(px(tokens::HEADING))
                            .child(address),
                    )
                    .child(self.host_button(
                        "host-address-copy",
                        tr(if copied { "Copied" } else { "Copy" }),
                        true,
                        cx,
                        move |this, cx| {
                            copy_to_clipboard(cx, gpui::ClipboardItem::new_string(copy.clone()));
                            this.servers.copied = Some(Instant::now());
                        },
                    )),
            )
            .child(hint(tr(
                "Friends on the same Wi-Fi: Multiplayer, Add Server, then paste this address.",
            )))
            .child(hint(trf(
                "Friends elsewhere can't reach it as it is: use Tailscale below, or forward TCP port {port} on your router to this computer.",
                &[("port", &port.to_string())],
            )));
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
        let mut right = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(stats);
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
        let sharing = self.hosting_share(port as u16, cx);
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
            .child(right)
    }

    fn host_console(&mut self, running: bool, cx: &mut Context<Self>) -> Div {
        let lines: Vec<Div> = self
            .servers
            .log
            .iter()
            .map(|l| {
                div()
                    .when(l.starts_with("> "), |d| {
                        d.text_color(rgb(palette().accent_ink))
                    })
                    .child(l.clone())
            })
            .collect();
        let log = div()
            .id("host-console")
            .track_scroll(&self.servers.console_scroll.handle)
            .h(px(220.))
            .px(px(16.))
            .pb(px(10.))
            .overflow_y_scroll()
            .font_family(theme::MONO)
            .text_size(px(tokens::SMALL))
            .line_height(px(18.))
            .text_color(rgb(palette().muted))
            .when(lines.is_empty(), |d| {
                d.child(tr(if running {
                    "Waiting for the server's log…"
                } else {
                    "The server's log shows here while it runs."
                }))
            })
            .children(lines);
        let input = if running {
            self.field(
                Field::Command,
                "Type a command: say Hello, time set day…",
                400.,
                cx,
            )
            .flex_1()
            .h(px(tokens::FIELD))
            .px(px(0.))
            .border_0()
            .bg(palette().card())
            .font_family(theme::MONO)
            .text_size(px(tokens::SMALL))
            .text_color(rgb(palette().text))
            .into_any_element()
        } else {
            div()
                .h(px(tokens::FIELD))
                .flex()
                .items_center()
                .text_size(px(tokens::SMALL))
                .text_color(rgb(palette().muted))
                .child(tr("Start the server to type commands"))
                .into_any_element()
        };
        card()
            .overflow_hidden()
            .child(
                div()
                    .px(px(16.))
                    .pt(px(14.))
                    .pb(px(10.))
                    .child(label("Console")),
            )
            .child(log)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(16.))
                    .border_t_1()
                    .border_color(rgb(palette().divider))
                    .child(div().text_color(rgb(palette().accent_ink)).child("›"))
                    .child(input),
            )
    }

    // ----- Players --------------------------------------------------------------------------

    fn host_person(name: String, about: Option<String>, face: Div, actions: Div) -> Div {
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(face)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
                    .children(about.map(hint)),
            )
            .child(actions.flex_none().flex().items_center().gap(px(6.)))
    }

    fn host_players(&mut self, dir: &Path, status: &Status, cx: &mut Context<Self>) -> Div {
        let running = status.running();
        let idle = self.servers.busy.is_none();
        let ops = servers::names(dir, "ops.json");
        let whitelist = servers::entries(dir, "whitelist.json");
        let banned = servers::names(dir, "banned-players.json");
        let is_op = |n: &str| ops.iter().any(|o| o.eq_ignore_ascii_case(n));
        let op_button =
            |this: &Self, id: ElementId, name: String, op: bool, cx: &mut Context<Self>| {
                this.host_button(
                    id,
                    tr(if op {
                        "Remove operator"
                    } else {
                        "Make operator"
                    }),
                    idle,
                    cx,
                    move |this, _| {
                        let verb = if op { "deop" } else { "op" };
                        this.server_command("Changing operators", vec![verb.into(), name.clone()]);
                    },
                )
            };

        let mut online = Vec::new();
        for (i, p) in status.players.clone().into_iter().enumerate() {
            let op = is_op(&p.name);
            let (kick, ban) = (p.name.clone(), p.name.clone());
            let actions = div()
                .child(op_button(
                    self,
                    ElementId::named_usize("host-online-op", i),
                    p.name.clone(),
                    op,
                    cx,
                ))
                .child(self.host_button(
                    ElementId::named_usize("host-kick", i),
                    tr("Kick"),
                    running,
                    cx,
                    move |this, _| this.api_command(format!("kick {kick}")),
                ))
                .child(danger(self.host_button(
                    ElementId::named_usize("host-ban", i),
                    tr("Ban"),
                    running,
                    cx,
                    move |this, _| this.api_command(format!("ban {ban}")),
                )));
            let ping = p
                .ping
                .map(|ms| trf("{ms} ms ping", &[("ms", &ms.to_string())]));
            let about = match (op, ping) {
                (true, Some(ping)) => Some(trf("Operator · {ping}", &[("ping", &ping)])),
                (true, None) => Some(tr("Operator").to_string()),
                (false, ping) => ping,
            };
            let face = self.head(&p.uuid, 28.);
            online.push(Self::host_person(p.name, about, face, actions));
        }
        let online = if online.is_empty() {
            card().p(px(16.)).child(hint(tr(if running {
                "Nobody is playing right now."
            } else {
                "The server is stopped."
            })))
        } else {
            group(online)
        };

        let friends_only = self
            .servers
            .setting("server", "whitelist")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let mut allowed = vec![row(
            tr("Friends only"),
            tr(if friends_only {
                "On: only the players below can join."
            } else {
                "Off: anyone who has the address can join."
            }),
            div().child(
                switch("host-friends-only", friends_only).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.set_setting("server.whitelist", (!friends_only).to_string());
                        cx.notify();
                    },
                )),
            ),
        )];
        let refused = self
            .servers
            .invalid
            .and_then(|(f, e)| (f == Field::AddPlayer).then_some(e));
        let add = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(self.host_field(Field::AddPlayer, tr("A player's name"), 240., cx))
            .child(
                // As tall as the name field beside it.
                self.host_button("host-let-in", tr("Let in"), idle, cx, |this, _| {
                    this.add_player()
                })
                .h(px(tokens::FIELD)),
            )
            .children(refused.map(|e| warning(tr(e))));
        allowed.push(add);
        let mut people: Vec<(String, String)> = whitelist.clone();
        for op in &ops {
            if !people.iter().any(|(n, _)| n.eq_ignore_ascii_case(op)) {
                people.push((op.clone(), String::new()));
            }
        }
        for (i, (name, uuid)) in people.into_iter().enumerate() {
            let op = is_op(&name);
            let listed = whitelist.iter().any(|(n, _)| n == &name);
            let mut actions = div().child(op_button(
                self,
                ElementId::named_usize("host-allowed-op", i),
                name.clone(),
                op,
                cx,
            ));
            if listed {
                let gone = name.clone();
                actions = actions.child(self.host_button(
                    ElementId::named_usize("host-allowed-remove", i),
                    tr("Remove"),
                    idle,
                    cx,
                    move |this, _| {
                        this.server_command(
                            "Updating the list",
                            vec!["whitelist".into(), "remove".into(), gone.clone()],
                        );
                    },
                ));
            }
            let face = if uuid.is_empty() {
                div().flex_none().size(px(28.))
            } else {
                self.head(&uuid, 28.)
            };
            let about = match (op, listed) {
                (true, true) => Some(tr("Operator: can run every command")),
                (true, false) => Some(tr("Operator (not on the list: can join anyway)")),
                _ => None,
            };
            allowed.push(Self::host_person(
                name,
                about.map(str::to_string),
                face,
                actions,
            ));
        }

        let mut tab = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(label("Online now"))
            .child(online)
            .child(div().pt(px(8.)).child(label("Who may join")))
            .child(group(allowed));
        if !banned.is_empty() {
            let rows = banned
                .into_iter()
                .enumerate()
                .map(|(i, name)| {
                    let pardon = name.clone();
                    let unban = self.host_button(
                        ElementId::named_usize("host-unban", i),
                        tr("Unban"),
                        running,
                        cx,
                        move |this, _| this.api_command(format!("pardon {pardon}")),
                    );
                    row(name, "", div().child(unban))
                })
                .collect();
            tab = tab
                .child(div().pt(px(8.)).child(label("Banned")))
                .children((!running).then(|| hint(tr("Start the server to unban someone."))))
                .child(group(rows));
        }
        tab
    }

    // ----- Settings -------------------------------------------------------------------------

    fn host_settings(&mut self, status: &Status, cx: &mut Context<Self>) -> Div {
        let running = status.running();
        if self.servers.config.is_none() {
            return card().p(px(16.)).child(hint(tr(if self.older_sunrise() {
                "This server's Sunrise is too old to show its settings here. Update it on the Backups tab, then they appear."
            } else {
                "Reading the settings…"
            })));
        }
        for (field, key) in [(Field::Motd, "motd"), (Field::Port, "port")] {
            if !self.servers.has_focus(field) && !self.servers.texts.contains_key(&field) {
                let value = self.servers.shown("server", key);
                self.servers.text(field).set(value);
            }
        }
        let live = tr(if running {
            "Applies at once."
        } else {
            "Used from the next start."
        });
        let later = tr(if running {
            "Saved now, used after a restart."
        } else {
            "Used from the next start."
        });
        let saved = |this: &mut Self,
                     f: Field,
                     placeholder: &'static str,
                     width: f32,
                     cx: &mut Context<Self>| {
            div()
                .child(this.host_field(f, placeholder, width, cx))
                .child(this.host_button(
                    SharedString::from(format!("host-save-{f:?}")),
                    tr("Save"),
                    true,
                    cx,
                    move |this, _| this.field_enter(f),
                ))
        };
        let motd = saved(self, Field::Motd, tr("A Minecraft Server"), 260., cx);
        let port = saved(self, Field::Port, "25565", 100., cx);
        let port_error = self
            .servers
            .invalid
            .and_then(|(f, e)| (f == Field::Port).then_some(e));
        let pvp = match (running, self.servers.pvp) {
            (true, Some(on)) => {
                switch("host-pvp", on).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.servers.pvp = Some(!on);
                    this.api_command(format!("gamerule pvp {}", !on));
                    cx.notify();
                }))
            }
            _ => off(switch("host-pvp", true)),
        };
        let accounts = self
            .servers
            .setting("server", "online_mode")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let service = self.servers.service.unwrap_or(false);
        let background = switch("host-background", service);
        let background = if self.servers.busy.is_none() && self.servers.service.is_some() {
            background.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.server_command(
                    if service {
                        "Turning off running in the background"
                    } else {
                        "Setting it to run in the background"
                    },
                    vec![
                        "service".into(),
                        if service { "uninstall" } else { "install" }.into(),
                    ],
                );
                cx.notify();
            }))
        } else {
            off(background)
        };
        let world = group(vec![
            row(
                tr("Game mode"),
                format!("{} {live}", tr("For players who join for the first time.")),
                self.host_choice("world.game_mode", &MODES, cx),
            ),
            row(
                tr("Difficulty"),
                live,
                self.host_choice("world.difficulty", &DIFFICULTIES, cx),
            ),
            row(
                tr("Players can hurt each other (PvP)"),
                tr(if running {
                    "A rule of the world. Applies at once."
                } else {
                    "A rule of the world: start the server to change it."
                }),
                div().child(pvp),
            ),
        ]);
        let server = group(vec![
            row(
                tr("Message in the server list"),
                format!(
                    "{} {later}",
                    tr("What players see under the server's name (the MOTD).")
                ),
                motd,
            ),
            row(
                tr("Most players at once"),
                format!("{} {later}", tr("Operators can always join.")),
                self.host_stepper("server.max_players", 1..=100, cx),
            ),
            row(
                tr("View distance"),
                format!(
                    "{} {later}",
                    tr("How far players see, in chunks (2 to 32). Lower is lighter.")
                ),
                self.host_stepper("server.view_distance", 2..=32, cx),
            ),
            row(
                tr("Simulation distance"),
                format!(
                    "{} {later}",
                    tr("How far from players crops grow and mobs move, in chunks.")
                ),
                self.host_stepper("server.simulation_distance", 2..=32, cx),
            ),
            row(
                tr("Port"),
                match port_error {
                    Some(e) => tr(e).to_string(),
                    None => format!(
                        "{} {later}",
                        tr("1024 to 65535. Friends type it after the address unless it's 25565.")
                    ),
                },
                port,
            ),
            row(
                tr("Check Minecraft accounts"),
                format!(
                    "{} {later}",
                    tr(if accounts {
                        "On: players must own Minecraft and sign in with Microsoft."
                    } else {
                        "Off: anyone can play under any name, so keep Friends only on."
                    })
                ),
                div().child(switch("host-online-mode", accounts).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.set_setting("server.online_mode", (!accounts).to_string());
                        cx.notify();
                    },
                ))),
            ),
            row(
                tr("Run in the background and at login"),
                tr(if service {
                    "On: starts when you log in to this computer, keeps running when the launcher closes, and comes back by itself if the computer restarts."
                } else {
                    "Off: the server stops when you close the launcher."
                }),
                div().child(background),
            ),
        ]);
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(hint(tr(
                "Changes save as you make them. Those the game can't take while it runs wait for a restart, and the page says so.",
            )))
            .child(world)
            .child(server)
            .child(group(vec![self.host_delete(status, cx)]))
    }

    fn host_delete(&mut self, status: &Status, cx: &mut Context<Self>) -> Div {
        let name = self
            .servers
            .list
            .iter()
            .find(|entry| Some(&entry.dir) == self.servers.open.as_ref())
            .map(|entry| entry.name.clone())
            .unwrap_or_default();
        let ready = !status.running()
            && self.servers.busy.is_none()
            && !self.servers.applying
            && self.servers.queued.is_empty()
            && self.servers.service == Some(false);
        let wait = if status.running() {
            Some("Stop the server before deleting it.")
        } else if self.servers.service == Some(true) {
            Some("Turn off running in the background before deleting it.")
        } else if !ready {
            Some("Wait for the current operation to finish.")
        } else {
            None
        };
        let about = tr("Remove this server from Sunrise and move its worlds, settings and backups to the trash.");
        if !self.servers.confirm_delete {
            let delete = danger(self.host_button(
                "host-delete",
                tr("Delete server…"),
                ready,
                cx,
                |this, cx| {
                    this.servers.confirm_delete = true;
                    this.servers.text(Field::DeleteServer).set(String::new());
                    let _ = this.focus_handle(Field::DeleteServer, cx);
                    this.servers.focus_soon = Some(Field::DeleteServer);
                },
            ));
            return row(
                tr("Delete server"),
                match wait {
                    Some(why) => format!("{about} {}", tr(why)),
                    None => about.to_string(),
                },
                div().child(delete),
            );
        }
        let matches = self.servers.value(Field::DeleteServer) == name && !name.is_empty();
        let field = self.host_field(Field::DeleteServer, tr("Server name"), 220., cx);
        let confirm = danger(self.host_button(
            "host-delete-yes",
            tr("Move to trash"),
            ready && matches,
            cx,
            |this, _| {
                if let Some(dir) = this.servers.open.clone() {
                    this.servers.busy = Some("Moving server to trash".into());
                    let tx = this.servers.tx.clone();
                    std::thread::spawn(move || {
                        let result = servers::delete(&dir);
                        let _ = tx.send(Event::Deleted { dir, result });
                    });
                }
            },
        ));
        let cancel = self.host_button("host-delete-no", tr("Keep it"), true, cx, |this, _| {
            this.servers.confirm_delete = false;
        });
        row(
            tr("Delete server"),
            trf(
                "{about} Type {name} to confirm.",
                &[("about", about), ("name", &name)],
            ),
            div().child(field).child(cancel).child(confirm),
        )
    }

    // ----- Backups and updates --------------------------------------------------------------

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
            div().child(primary(self.host_button(
                "host-backup-now",
                tr("Back up now"),
                idle,
                cx,
                |this, _| this.server_command("Backing up", vec!["backup".into()]),
            ))),
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
            let restore = self.host_button(
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
                        this.server_job("Restoring", move |log| {
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
                actions.child(if confirming { danger(restore) } else { restore }),
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
            Some(tag) => primary(self.host_button(
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
                        this.server_job(trf("Updating to {tag}", &[("tag", &tag)]), move |log| {
                            servers::run_streamed(&dir, &["update"], log).map(|_| done)
                        });
                    }
                },
            )),
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

    // ----- Creating -------------------------------------------------------------------------

    /// The one dialog a server is created in, over the page.
    pub(crate) fn hosting_create(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let c = self.servers.create.as_ref()?;
        let (new_world, copy, worlds) = (c.new_world, c.copy.clone(), c.worlds.clone());
        let (mode, difficulty, max_players) = (c.mode, c.difficulty, c.max_players);
        let (friends_only, eula, eula_missing) = (c.friends_only, c.eula, c.eula_missing);
        let friends = c.friends.clone();
        let creating = self.servers.creating;
        let choice = |id: &'static str,
                      options: &[(&'static str, &'static str)],
                      current: &str,
                      f: fn(&mut Create, &'static str),
                      cx: &mut Context<Self>| {
            chip_group(id, options, current, cx, move |this, value| {
                if let Some(c) = &mut this.servers.create {
                    f(c, value);
                }
            })
        };
        let part = |name: &'static str, control: Div| {
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(label(name))
                .child(control)
        };
        let world_kind = choice(
            "host-create-world",
            &[("new", "New world"), ("copy", "Copy one of my worlds")],
            if new_world { "new" } else { "copy" },
            |c, v| c.new_world = v == "new",
            cx,
        );
        let world_detail = if new_world {
            div().child(self.host_field(
                Field::Seed,
                tr("Seed (optional: leave empty for a random world)"),
                420.,
                cx,
            ))
        } else if worlds.is_empty() {
            hint(tr("No single-player world yet. Choose New world."))
        } else {
            let count = worlds.len();
            let mut list = div()
                .id("host-create-worlds")
                .flex_none()
                .flex()
                .flex_col()
                .min_h_0()
                .max_h(px(144.))
                .overflow_y_scroll()
                .track_scroll(&crate::frame::scrolls("host-create-worlds"))
                .on_scroll(|_, _, cx| cx.stop_propagation());
            for (i, (name, path)) in worlds.into_iter().enumerate() {
                let chosen = copy.as_ref() == Some(&path);
                list = list.child(
                    div()
                        .id(ElementId::named_usize("host-create-copy", i))
                        .flex_none()
                        .tab_index(0)
                        .seen()
                        .aria_label(name.clone())
                        .tooltip(crate::glass_ui::tip(name.clone()))
                        .border_1()
                        .border_color(gpui::transparent_black())
                        .focus_visible(|s| s.border_color(rgb(palette().focus)))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .h(px(32.))
                        .px(px(10.))
                        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                        .cursor_pointer()
                        .when(chosen, |d| d.bg(rgb(palette().selected)))
                        .child(
                            div()
                                .flex_none()
                                .size(px(12.))
                                .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                                .border_1()
                                .border_color(rgb(if chosen {
                                    palette().text
                                } else {
                                    palette().muted
                                }))
                                .when(chosen, |d| d.bg(rgb(palette().text))),
                        )
                        .child(div().flex_1().min_w_0().truncate().child(name))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if let Some(c) = &mut this.servers.create {
                                c.copy = Some(path.clone());
                            }
                            cx.notify();
                        })),
                );
            }
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(caption(&trf(
                    "Your worlds · {count}",
                    &[("count", &count.to_string())],
                )))
                .child(list)
                .child(hint(tr(
                    "The server gets a copy: your single-player world stays as it is.",
                )))
        };
        let step = |id: &'static str, sign: &'static str, delta: i32, cx: &mut Context<Self>| {
            button(id, sign).w(px(32.)).px(px(0.)).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    if let Some(c) = &mut this.servers.create {
                        c.max_players = (c.max_players as i32 + delta).clamp(1, 100) as u32;
                    }
                    cx.notify();
                },
            ))
        };
        let players = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(step("host-create-fewer", "−", -1, cx))
            .child(
                div()
                    .w(px(36.))
                    .flex()
                    .justify_center()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(max_players.to_string()),
            )
            .child(step("host-create-more", "+", 1, cx));
        let mut who = div().flex().flex_col().gap(px(10.)).child(choice(
            "host-create-who",
            &[
                ("friends", "Friends only"),
                ("anyone", "Anyone with the address"),
            ],
            if friends_only { "friends" } else { "anyone" },
            |c, v| c.friends_only = v == "friends",
            cx,
        ));
        if friends_only {
            let count = friends.len();
            let mut names = div().flex().flex_wrap().items_center().gap(px(6.));
            for (i, friend) in friends.into_iter().enumerate() {
                names = names.child(
                    div()
                        .id(ElementId::named_usize("host-create-friend", i))
                        .reach_edge()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .h(px(tokens::CONTROL))
                        .px(px(12.))
                        .rounded(crate::tokens::r(crate::tokens::R_PANEL))
                        .border_1()
                        .border_color(rgb(palette().line))
                        .text_size(px(tokens::LABEL))
                        .cursor_pointer()
                        .max_w_full()
                        .tooltip(crate::glass_ui::tip(friend.clone()))
                        .child(div().min_w_0().truncate().child(friend))
                        .child(
                            div()
                                .flex_none()
                                .text_color(rgb(palette().muted))
                                .child("×"),
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if let Some(c) = &mut this.servers.create {
                                if i < c.friends.len() {
                                    c.friends.remove(i);
                                }
                            }
                            cx.notify();
                        })),
                );
            }
            who = who
                .child(caption(&trf(
                    "Invited players · {count}",
                    &[("count", &count.to_string())],
                )))
                .children((count > 0).then(|| {
                    div()
                        .id("host-create-invites")
                        .min_h_0()
                        .max_h(px(104.))
                        .overflow_y_scroll()
                        .track_scroll(&crate::frame::scrolls("host-create-invites"))
                        .on_scroll(|_, _, cx| cx.stop_propagation())
                        .child(names)
                }))
                .child(self.host_field(Field::Players, tr("A player's name, then Enter"), 240., cx))
                .child(hint(tr(
                    "The first one becomes an operator. Add more later under Players.",
                )));
        }
        let eula_line = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                crate::mods::checkbox("host-create-eula", eula).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        if let Some(c) = &mut this.servers.create {
                            c.eula = !c.eula;
                            c.eula_missing = false;
                        }
                        cx.notify();
                    },
                )),
            )
            .child(
                div()
                    .when(eula_missing, |d| d.text_color(rgb(palette().danger)))
                    .child(tr("I agree to the Minecraft EULA")),
            )
            .child(
                div()
                    .id("host-create-eula-read")
                    .tab_index(0)
                    .seen()
                    .px(px(4.))
                    .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .focus_visible(|s| s.border_color(rgb(palette().focus)))
                    .text_color(rgb(palette().accent_ink))
                    .cursor_pointer()
                    .child(tr("Read it"))
                    .on_click(|_, _, cx| cx.open_url(servers::EULA_URL)),
            );
        let body = div()
            .id("host-create-body")
            .flex()
            .flex_col()
            .min_h_0()
            .gap(px(18.))
            .p(px(28.))
            .overflow_y_scroll().track_scroll(&crate::frame::scrolls("host-create"))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(tokens::TITLE))
                            .font_weight(FontWeight::BOLD)
                            .child(tr("Create a server")),
                    )
                    .child(hint(tr(
                        "Everything has a default: accept Minecraft's terms and Create makes a friends-only survival world. You can change all of it later.",
                    ))),
            )
            .child(part(
                "Name",
                div().child(self.host_field(Field::Name, tr("My server"), 420., cx)),
            ))
            .child(part(
                "World",
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(world_kind)
                    .child(world_detail),
            ))
            .child(part(
                "Game mode",
                choice("host-create-mode", &MODES, mode, |c, v| c.mode = v, cx),
            ))
            .child(part(
                "Difficulty",
                choice(
                    "host-create-difficulty",
                    &DIFFICULTIES,
                    difficulty,
                    |c, v| c.difficulty = v,
                    cx,
                ),
            ))
            .child(part("Most players at once", players))
            .child(part("Who can join", who))
            .child(eula_line)
            .children(self.servers.problem.as_ref().filter(|_| !creating).map(|p| {
                div()
                    .id("host-create-problem")
                    .flex_none()
                    .min_h_0()
                    .max_h(px(96.))
                    .overflow_y_scroll()
                    .track_scroll(&crate::frame::scrolls("host-create-problem"))
                    .on_scroll(|_, _, cx| cx.stop_propagation())
                    .child(warning(p.message.clone()))
            }));
        let said = if creating {
            let last = self.servers.job_log.back().cloned().unwrap_or_default();
            div()
                .text_size(px(tokens::LABEL))
                .text_color(rgb(palette().muted))
                .line_clamp(2)
                .child(format!("{} {last}", tr("Creating…")))
        } else if let Some(p) = &self.servers.problem {
            div()
                .text_size(px(tokens::LABEL))
                .text_color(rgb(palette().danger))
                .line_clamp(2)
                .child(p.message.clone())
        } else if eula_missing {
            warning(tr("Tick the EULA box to create the server."))
        } else {
            div()
        };
        let prepare = self
            .servers
            .problem
            .as_ref()
            .and_then(|p| p.fix.clone())
            .filter(|fix| !creating && matches!(fix, Fix::Prepare))
            .map(|fix| {
                self.host_button(
                    "host-create-fix",
                    tr("Prepare the game files"),
                    true,
                    cx,
                    move |this, cx| this.host_fix(fix.clone(), cx),
                )
            });
        let foot = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .px(px(28.))
            .py(px(16.))
            .border_t_1()
            .border_color(rgb(palette().divider))
            .child(said.flex_1().min_w_0())
            .children(prepare)
            .child(self.host_button(
                "host-create-cancel",
                tr("Cancel"),
                !creating,
                cx,
                |this, _| {
                    this.servers.create = None;
                    this.servers.problem = None;
                },
            ))
            .child(primary(self.host_button(
                "host-create-go",
                tr("Create"),
                !creating,
                cx,
                |this, _| this.create_server(),
            )));
        Some(
            div()
                .id("host-create")
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .p(px(24.))
                .bg(palette().scrim())
                .text_color(rgb(palette().text))
                .on_scroll(|_, _, cx| cx.stop_propagation())
                .child(
                    // Solid, like every other dialog: a card's fill is see-through, made to lie on
                    // the page, and the page showed through the form.
                    card()
                        .w(px(620.))
                        .max_w_full()
                        .max_h_full()
                        .rounded(crate::tokens::r(crate::tokens::R_PANEL))
                        .border_color(rgb(palette().line))
                        .bg(rgb(palette().raised))
                        .overflow_hidden()
                        .child(body)
                        .child(foot),
                ),
        )
    }
}
