//! A server's live status: its address, console and players.
use super::widgets::*;
use super::*;
use crate::widgets::feedback::{banner, Tone};

/// The LAN address a listener is reached at, when the address card shows a local one: the
/// listener's own IP, or this machine's LAN IP when it listens on every interface.
fn lan_ip(
    listener: Option<std::net::SocketAddr>,
    internet: bool,
    local_only: bool,
) -> Option<std::net::IpAddr> {
    listener
        .filter(|_| !internet && !local_only)
        .and_then(|address| {
            if address.ip().is_unspecified() {
                servers::lan_address().filter(|ip| ip.is_ipv4() == address.is_ipv4())
            } else {
                Some(address.ip())
            }
        })
}

impl Launcher {
    /// The address this server is shared at, as the address card shows it and its copy button
    /// copies it: the public one in internet mode once detected, else the local one.
    pub(super) fn share_text(&self, listener: Option<std::net::SocketAddr>) -> Option<String> {
        let s = &self.servers;
        let internet = s.internet_address;
        let local_only = listener.is_some_and(|address| address.ip().is_loopback());
        let public_port = listener
            .filter(|address| address.is_ipv4() && !address.ip().is_loopback())
            .map(|address| address.port());
        if internet {
            s.last_public_address
                .filter(|_| s.public_lookup == PublicLookup::Detected)
                .zip(public_port)
                .map(|(ip, port)| servers::network::address(ip.into(), port))
        } else {
            listener.filter(|_| !local_only).map(|listener| {
                lan_ip(Some(listener), internet, local_only).map_or_else(
                    || format!("localhost:{}", listener.port()),
                    |ip| servers::network::address(ip, listener.port()),
                )
            })
        }
    }

    pub(super) fn host_address_card(
        &mut self,
        listener: Option<std::net::SocketAddr>,
        saved: Option<u16>,
        status: &Status,
        cx: &mut Context<Self>,
    ) -> Div {
        let s = &self.servers;
        let internet = s.internet_address;
        let lookup = s.public_lookup;
        let last = s.last_public_address;
        let port = listener.map(|address| address.port());
        let local_only = listener.is_some_and(|address| address.ip().is_loopback());
        let public_port = listener
            .filter(|address| address.is_ipv4() && !address.ip().is_loopback())
            .map(|address| address.port());
        let lan = lan_ip(listener, internet, local_only);
        let address = self.share_text(listener);

        let modes = chip_group(
            "host-address-network",
            &[("lan", "Local network"), ("internet", "Internet")],
            if internet { "internet" } else { "lan" },
            cx,
            |this, value| {
                let internet = value == "internet";
                if internet != this.servers.internet_address {
                    this.servers.internet_address = internet;
                    this.servers.reset_public_lookup();
                }
            },
        );
        let shown = address.clone().unwrap_or_else(|| {
            tr(match (internet, lookup) {
                _ if local_only => "Local connections only",
                (true, _) if listener.is_some() && public_port.is_none() => {
                    "Public IPv4 unavailable for this listener"
                }
                (true, PublicLookup::Idle) => "Public IP not detected",
                (true, PublicLookup::Loading(_)) => "Finding public IP…",
                (true, PublicLookup::Failed) => "Public IP unavailable",
                _ => "Server port unavailable",
            })
            .to_owned()
        });
        let mut panel = card()
            .gap(px(8.))
            .p(px(16.))
            .child(label("Address to share"))
            .child(modes)
            .child(label(if internet {
                "Last detected public address"
            } else {
                "Address on this network"
            }))
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
                            .child(shown),
                    )
                    .child(match address {
                        Some(address) => crate::widgets::motion_bits::copy_button(
                            "host-address-copy",
                            tr("Copy"),
                            address,
                        )
                        .into_any_element(),
                        None => self
                            .host_button("host-address-copy", tr("Copy"), false, cx, |_, _| {})
                            .into_any_element(),
                    }),
            );

        if local_only {
            panel = panel.child(hint(tr("This server listens only on this computer.")));
        } else if internet && (listener.is_none() || public_port.is_some()) {
            if listener.is_none() {
                panel = panel.child(hint(tr("Server port unavailable")));
            }
            if let Some(ip) =
                last.filter(|_| lookup != PublicLookup::Detected || public_port.is_none())
            {
                panel = panel.child(hint(trf(
                    "Last detected public IP: {ip}.",
                    &[("ip", &ip.to_string())],
                )));
            }
            panel = panel
                .child(hint(tr(
                    "Detecting your public IPv4 sends a direct request to ipify outside the launcher proxy. ipify sees this connection's IP address. The request does not check whether your server is reachable.",
                )))
                .child(self.host_button(
                    "host-public-address-detect",
                    tr(if lookup == PublicLookup::Detected {
                        "Refresh public IP"
                    } else {
                        "Detect public IP"
                    }),
                    s.public_lookup_in_flight.is_none(),
                    cx,
                    |this, _| this.find_server_public_address(),
                ));
            if lookup == PublicLookup::Failed {
                panel = panel.child(banner(Tone::Danger, tr(
                    "Could not find your public IP. Check your internet connection and try again.",
                ), None));
            }
            if let Some(port) = public_port {
                panel = panel.child(hint(trf(
                    "Forward TCP port {port} on your router to this computer and allow it through your firewall. If your provider uses CGNAT, you need a public IP or a tunnel.",
                    &[("port", &port.to_string())],
                )));
            }
        } else if !internet && lan.is_some() {
            panel = panel.child(hint(tr(
                "Friends on the same Wi-Fi: Multiplayer, Add Server, then paste this address.",
            )));
        } else if !internet && listener.is_some() {
            panel = panel.child(hint(tr(
                "No local network address was found. localhost works only on this computer.",
            )));
        }

        if let Some(port) = public_port.filter(|_| !internet && lan.is_some()) {
            panel = panel.child(hint(trf(
                "Friends elsewhere can't reach it as it is: use Tailscale below, or forward TCP port {port} on your router to this computer.",
                &[("port", &port.to_string())],
            )));
        }
        if let Some((running, saved)) = port
            .zip(saved)
            .filter(|(running, saved)| status.phase == "running" && running != saved)
        {
            panel = panel.child(hint(trf(
                "The server is running on port {running}. Restart to use saved port {saved}.",
                &[
                    ("running", &running.to_string()),
                    ("saved", &saved.to_string()),
                ],
            )));
        }
        panel
    }

    pub(super) fn host_console(&mut self, running: bool, cx: &mut Context<Self>) -> Div {
        let lines: Vec<Div> = self
            .servers
            .log
            .iter()
            .enumerate()
            .map(|(n, l)| {
                div()
                    .when(l.starts_with("> "), |d| {
                        d.text_color(rgb(palette().accent_ink))
                    })
                    .child(crate::selectable::Selectable::styled(
                        ("host-console-line", n),
                        gpui::StyledText::new(l.clone()),
                    ))
            })
            .collect();
        let whole = self
            .servers
            .log
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        let log = div()
            .id("host-console")
            .track_scroll(&self.servers.console_scroll.handle)
            .h(px(220.))
            .pl(px(16.))
            // Room for the copy button.
            .pr(px(44.))
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
                crate::i18n::tr_key("Type a command: say Hello, time set day…"),
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
            .child(div().relative().child(log).when(!whole.is_empty(), |d| {
                d.child(crate::code_box::copy_button("host-console".into(), whole))
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(16.))
                    .border_t_1()
                    .border_color(palette().rule())
                    .child(div().text_color(rgb(palette().accent_ink)).child("›"))
                    .child(input),
            )
    }

    pub(super) fn host_players(
        &mut self,
        dir: &Path,
        status: &Status,
        cx: &mut Context<Self>,
    ) -> Div {
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
                        this.server_command(
                            crate::i18n::tr_key("Changing operators"),
                            vec![verb.into(), name.clone()],
                        );
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
            .shown("server", "whitelist")
            .parse::<bool>()
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
            .children(refused.map(|e| banner(Tone::Danger, tr(e), None)));
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
                            crate::i18n::tr_key("Updating the list"),
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
}
