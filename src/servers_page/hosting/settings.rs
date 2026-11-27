//! A server's settings, and deleting it.
use super::widgets::*;
use super::*;

impl Launcher {
    pub(super) fn host_settings(&mut self, status: &Status, cx: &mut Context<Self>) -> Div {
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
            .shown("server", "online_mode")
            .parse::<bool>()
            .unwrap_or(false);
        let service = self.servers.service.unwrap_or(false);
        let background = switch("host-background", service);
        let background = if self.servers.busy.is_none() && self.servers.service.is_some() {
            background.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.server_command(
                    if service {
                        crate::i18n::tr_key("Turning off running in the background")
                    } else {
                        crate::i18n::tr_key("Setting it to run in the background")
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
            && self.servers.applying.is_none()
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
            let delete = self.host_action(
                crate::widgets::button::Style::Danger,
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
            );
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
        let confirm = self.host_action(
            crate::widgets::button::Style::Danger,
            "host-delete-yes",
            tr("Move to trash"),
            ready && matches,
            cx,
            |this, _| {
                if let Some(dir) = this.servers.open.clone() {
                    this.servers.busy = Some(crate::i18n::tr_key("Moving server to trash").into());
                    let tx = this.servers.tx.clone();
                    std::thread::spawn(move || {
                        let result = servers::delete(&dir);
                        let _ = tx.send(Event::Deleted { dir, result });
                    });
                }
            },
        );
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
}
