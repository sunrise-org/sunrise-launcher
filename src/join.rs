//! Join: one box for servers (an address or a friend's invite code, Save, Join) over the saved
//! list.
//!
//! Everything shown is read from the player's real launcher data, and nothing is written until
//! the player acts (Save, Remove, Join). Join sets `quick_play` or `joining` and calls
//! `play_game`, which owns the sign-in, install, prepare, build and trust gates:
//! - servers: the game's own `servers.dat` (`server_list`) with the status ping the Multiplayer
//!   screen sends, Join, Save and Remove; an invite code joins through `join_friend`;
//! - featured servers: a bundled pair, Horizon Minecraft (joinable once its address is set)
//!   and Sunrise's own (inert, coming soon); no backend service.
use super::*;
use crate::frame::button;
use crate::frame::Reach as _;
use crate::home::muted;
use crate::paint::palette;
use crate::servers_page::collection;
use gpui::{rgb, rgba};

/// Horizon Minecraft's address, passed to the game as written. Empty: the row has no Join.
pub(crate) const HORIZON_ADDRESS: &str = "play.playhorizon.dev";

/// A server Sunrise features, bundled: there is no service to ask.
#[derive(Debug, PartialEq)]
pub(crate) struct Featured {
    pub name: &'static str,
    /// None: it can't be joined (yet).
    pub address: Option<&'static str>,
    pub note: &'static str,
    /// What an entry without an address says.
    pub inert: &'static str,
    /// The picture it wears when the server sends none of its own.
    pub badge: Badge,
}

/// A featured server's bundled picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Badge {
    /// Horizon's site icon.
    Horizon,
    /// The launcher's own mark.
    Sunrise,
}

/// Horizon's icon as its site draws it: an H on dark purple, 16 cells a side, painted as
/// rectangles so it stays sharp at any size.
fn horizon_mark(size: f32) -> Div {
    const CELLS: [(f32, f32, f32, f32, u32); 5] = [
        (4., 3., 2., 4., 0xffffffff),
        (10., 3., 2., 4., 0xffffffff),
        (4., 8., 2., 4., 0xffffff66),
        (10., 8., 2., 4., 0xffffff66),
        (1., 7., 14., 1., 0xa98fffff),
    ];
    let u = size / 16.;
    let mut tile = div()
        .relative()
        .flex_none()
        .size(px(size))
        .rounded(px(3. * u))
        .bg(rgb(0x100a1f));
    for (x, y, w, h, color) in CELLS {
        tile = tile.child(
            div()
                .absolute()
                .left(px(x * u))
                .top(px(y * u))
                .w(px(w * u))
                .h(px(h * u))
                .bg(rgba(color)),
        );
    }
    tile
}

pub(crate) fn featured() -> Vec<Featured> {
    featured_with(HORIZON_ADDRESS)
}

fn featured_with(horizon: &'static str) -> Vec<Featured> {
    vec![
        Featured {
            name: "Horizon Minecraft",
            address: Some(horizon).filter(|a| !a.is_empty()),
            note: "A Horizon community server",
            inert: "Address coming soon",
            badge: Badge::Horizon,
        },
        Featured {
            name: "Sunrise server",
            address: None,
            note: "Sunrise's own server",
            inert: "Coming soon",
            badge: Badge::Sunrise,
        },
    ]
}

/// What the address box holds.
#[derive(Debug, PartialEq)]
pub(crate) enum Entry {
    Empty,
    Invite,
    Address(String),
    /// Spaces or the like inside: neither.
    Unusable,
}

pub(crate) fn entry(text: &str) -> Entry {
    let text = text.trim();
    if text.is_empty() {
        Entry::Empty
    } else if sunrise_launcher_friends::invite::parse(text).is_some() {
        Entry::Invite
    } else if text
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '/')
        || !sunrise_launcher_telemetry::server_status::split_address(text)
            .is_some_and(|(host, port)| !host.is_empty() && port > 0)
    {
        Entry::Unusable
    } else {
        Entry::Address(text.to_string())
    }
}

/// The first letter of `name`, upper case, for a plain tile.
pub(crate) fn initial(name: &str) -> String {
    name.trim()
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "?".into())
}

/// The host an address names, without its port, in lower case.
fn host(address: &str) -> Option<String> {
    sunrise_launcher_telemetry::server_status::split_address(address)
        .map(|(host, _)| host.to_ascii_lowercase())
}

/// The featured server at `address`'s host, whatever port or letter case it was saved with.
fn featured_at(address: &str) -> Option<Featured> {
    let host = host(address)?;
    featured()
        .into_iter()
        .find(|f| f.address.and_then(self::host).as_ref() == Some(&host))
}

/// A server's picture: its own, else the bundled one of a featured server, else its first
/// letter.
fn server_picture(
    icon: Option<std::sync::Arc<gpui::Image>>,
    badge: Option<Badge>,
    name: &str,
) -> AnyElement {
    match (icon, badge) {
        (Some(image), _) => gpui::img(image)
            .flex_none()
            .size(px(32.))
            .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
            .into_any_element(),
        (None, Some(Badge::Horizon)) => horizon_mark(32.).into_any_element(),
        (None, Some(Badge::Sunrise)) => crate::mark(32.).into_any_element(),
        (None, None) => tile(name, 32.).into_any_element(),
    }
}

pub(crate) fn tile(label: &str, side: f32) -> Div {
    // A colour of its own from its name, the same every time, kept quiet enough to sit on the
    // card: the letter reads as the server's badge rather than a missing picture.
    let hash = label.bytes().fold(0x811c_9dc5_u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });
    let hue = (hash % 360) as f32 / 360.;
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(side))
        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
        .bg(gpui::hsla(hue, 0.28, 0.24, 1.))
        .text_size(px(side * 0.45))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(gpui::hsla(hue, 0.45, 0.82, 1.))
        .child(initial(label))
}

fn reach_labels(reach: multiplayer_page::Reach) -> (String, Option<String>) {
    match reach {
        multiplayer_page::Reach::Online {
            players,
            max,
            latency,
        } => (
            trf(
                "{online}/{max} players",
                &[("online", &players.to_string()), ("max", &max.to_string())],
            ),
            Some(trf(
                "{ping} ms",
                &[("ping", &latency.as_millis().to_string())],
            )),
        ),
        multiplayer_page::Reach::Offline => (tr("No answer").into(), None),
        multiplayer_page::Reach::Asking => (tr("Checking…").into(), None),
    }
}

impl Launcher {
    /// The Join page: the servers of the selected profile.
    pub(crate) fn join(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(home) = self.selected_instance().map(|i| i.home.clone()) else {
            return div().into_any_element();
        };
        self.load_servers(home);
        self.ping_servers(cx);
        self.ping_featured(cx);
        let narrow = self.servers_narrow();
        let saved = self.multiplayer.list.clone();
        let keys = self.multiplayer.keys.clone();
        let toolbar = self.join_toolbar(cx);
        let selection = if let Some(remove) = &self.multiplayer.remove {
            collection::confirmation(
                "join-remove-review",
                if remove.len() == 1 {
                    tr("Remove this saved server?").into()
                } else {
                    trf(
                        "Remove {count} saved servers?",
                        &[("count", &remove.len().to_string())],
                    )
                },
                tr("Remove"),
                |this| this.multiplayer.remove = None,
                Launcher::remove_saved_servers,
                cx,
            )
        } else {
            let selected = self.multiplayer.selected.len();
            let all = !keys.is_empty() && selected == keys.len();
            let check = collection::check("join-select-all", all, tr("Select all saved servers"));
            let check = if keys.is_empty() {
                check.opacity(0.45).tab_stop(false)
            } else {
                let keys = keys.clone();
                check.on_click(cx.listener(move |this, _, _, cx| {
                    if all {
                        this.multiplayer.selected.clear()
                    } else {
                        this.multiplayer.selected.extend(keys.iter().cloned());
                    }
                    cx.notify();
                }))
            };
            let label = match (selected, saved.len()) {
                (0, 1) => tr("1 saved server").to_owned(),
                (0, count) => trf("{count} saved servers", &[("count", &count.to_string())]),
                (count, _) => trf("{count} selected", &[("count", &count.to_string())]),
            };
            let actions = if selected == 0 {
                div()
            } else {
                collection::bulk_actions(
                    "join-selection",
                    tr("Remove selected"),
                    |this| this.multiplayer.selected.clear(),
                    |this| this.multiplayer.remove = Some(this.multiplayer.selected.clone()),
                    cx,
                )
            };
            collection::selection_bar(check, label, actions)
        };
        let controls = div()
            .flex_none()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(selection)
            .children(
                self.multiplayer
                    .error
                    .as_ref()
                    .filter(|_| self.multiplayer.adding.is_none())
                    .map(|e| {
                        div()
                            .id("join-notice")
                            .px(px(12.))
                            .pb(px(8.))
                            .text_size(px(tokens::LABEL))
                            .text_color(rgb(palette().warning))
                            .line_clamp(2)
                            .tooltip(crate::glass_ui::tip(e.clone()))
                            .child(e.clone())
                    }),
            );
        let mut list = collection::rows("join");
        if saved.is_empty() {
            list = list.child(
                collection::toolbar()
                    .border_t_1()
                    .border_color(rgb(palette().divider))
                    .child(muted(tr("No saved servers"))),
            );
        }
        for (i, ((server, icon), key)) in
            saved.iter().cloned().zip(keys.iter().cloned()).enumerate()
        {
            list = list.child(self.server_row(i, server, icon, key, narrow, cx));
        }
        for (i, server) in featured().into_iter().enumerate() {
            // A featured host saved already, on any port, shows just once as the saved row.
            if saved
                .iter()
                .any(|(s, _)| featured_at(&s.ip).as_ref() == Some(&server))
            {
                continue;
            }
            list = list.child(self.featured_row(i, server, narrow, cx));
        }
        collection::table(controls, list, "join", &self.multiplayer.thumb, cx).into_any_element()
    }

    fn join_toolbar(&mut self, cx: &mut Context<Self>) -> Div {
        let typed = entry(self.friend_code.text());
        let focused = self.friend_focused;
        let field = crate::frame::input(focused)
            .id("alt-address")
            .track_focus(&self.friend_focus)
            .seen()
            .flex_1()
            .min_w_0()
            .h(px(36.))
            .px(px(12.))
            .text_size(px(tokens::BODY))
            .text_color(rgb(palette().text))
            .cursor_text()
            .child(line_edit::content(
                &self.friend_code,
                &self.theme,
                focused,
                false,
                false,
                "Server address or a friend's invite code",
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.friend_focus, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match line_edit::handle(&mut this.friend_code, ev, cx) {
                    line_edit::Key::Pass => return,
                    line_edit::Key::Enter => this.join_typed(),
                    line_edit::Key::Escape => window.blur(cx),
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        let field = line_edit::mouse(field, cx, |this| &mut this.friend_code);
        let join = button("alt-server-join", tr("Join")).h(px(36.));
        let join = if matches!(typed, Entry::Address(_) | Entry::Invite) {
            join.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.join_typed();
                cx.notify();
            }))
        } else {
            join.opacity(0.4).cursor_default().tab_stop(false)
        };
        let add = crate::frame::primary("alt-server-add", "")
            .h(px(36.))
            .gap(px(6.))
            .child(crate::glass_ui::icon(
                "plus-sign",
                14.,
                rgb(palette().background).into(),
            ))
            .child(tr("Add server"))
            .on_click(cx.listener(|this, _, _, cx| {
                this.open_add_server(cx);
                cx.notify();
            }));
        div()
            .flex()
            .flex_col()
            .child(collection::toolbar().child(field).child(join).child(add))
            .children((typed == Entry::Unusable).then(|| {
                muted(tr("That isn't an address or an invite code."))
                    .px(px(12.))
                    .pb(px(8.))
            }))
    }

    fn featured_row(
        &mut self,
        i: usize,
        server: Featured,
        narrow: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let (line, end) = match server.address {
            Some(address) => {
                let (first, second) = reach_labels(
                    self.multiplayer
                        .reach
                        .get(address)
                        .copied()
                        .unwrap_or(multiplayer_page::Reach::Asking),
                );
                let join = button(("featured-join", i), tr("Join")).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.play_to("--server", address.to_string());
                        cx.notify();
                    },
                ));
                (
                    collection::status(narrow, first, second),
                    join.into_any_element(),
                )
            }
            None => (
                collection::status(narrow, tr(server.inert).into(), None),
                div().w(px(43.)).into_any_element(),
            ),
        };
        collection::row(narrow)
            .child(div().flex_none().w(px(18.)))
            .child(server_picture(
                server
                    .address
                    .and_then(|a| self.multiplayer.icons.get(a).cloned()),
                Some(server.badge),
                server.name,
            ))
            .child(collection::identity(
                ("featured-server", i),
                tr(server.name).into(),
                server.address.unwrap_or_else(|| tr(server.note)).into(),
            ))
            .child(line)
            .child(end)
            .child(div().flex_none().w(px(32.)))
    }

    /// Asks the featured servers that can be joined for their players, once each.
    fn ping_featured(&mut self, cx: &mut Context<Self>) {
        for address in featured().into_iter().filter_map(|f| f.address) {
            if self.multiplayer.reach.contains_key(address) {
                continue;
            }
            self.multiplayer
                .reach
                .insert(address.to_string(), multiplayer_page::Reach::Asking);
            let task = cx
                .background_executor()
                .spawn(async move { sunrise_launcher::telemetry::server_status(address) });
            cx.spawn(async move |this, cx| {
                let answer = task.await;
                let _ = this.update(cx, |this, cx| {
                    this.server_answered(address.to_string(), answer);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    fn server_row(
        &mut self,
        i: usize,
        server: sunrise_launcher::server_list::Server,
        icon: Option<std::sync::Arc<gpui::Image>>,
        key: sunrise_launcher::server_list::SavedKey,
        narrow: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        use multiplayer_page::Reach;
        let reach = self
            .multiplayer
            .reach
            .get(&server.ip)
            .copied()
            .unwrap_or(Reach::Asking);
        let (first, second) = reach_labels(reach);
        let checked = self.multiplayer.selected.contains(&key);
        let join_focus = self
            .multiplayer
            .focuses
            .entry(key.clone())
            .or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone();
        let reveal = self.home_server_target(&key, &join_focus);
        let select = collection::check(
            ("join-select", i),
            checked,
            trf(
                "Select {name} · {address}",
                &[("name", &server.name), ("address", &server.ip)],
            ),
        )
        .on_click(cx.listener({
            let key = key.clone();
            move |this, _, _, cx| {
                this.multiplayer.remove = None;
                if !this.multiplayer.selected.remove(&key) {
                    this.multiplayer.selected.insert(key.clone());
                }
                cx.notify();
            }
        }));
        let remove = collection::remove(("join-remove", i), tr("Remove server")).on_click(
            cx.listener(move |this, _, _, cx| {
                this.multiplayer.remove = Some([key.clone()].into());
                cx.notify();
            }),
        );
        let action = button(("join-server", i), tr("Join"));
        let join = if reach == Reach::Offline {
            action
                .opacity(0.4)
                .cursor_default()
                .tab_stop(false)
                .tooltip(crate::glass_ui::tip(tr(
                    "This server doesn't answer right now",
                )))
        } else {
            let ip = server.ip.clone();
            action
                .track_focus(&join_focus)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.play_to("--server", ip.clone());
                    cx.notify();
                }))
        };
        let picture = server_picture(
            icon.or_else(|| self.multiplayer.icons.get(&server.ip).cloned()),
            featured_at(&server.ip).map(|f| f.badge),
            &server.name,
        );
        collection::row(narrow)
            .id(("join-row", i))
            .when(checked, |d| d.bg(rgb(palette().selected)))
            .when(reach == Reach::Offline, |d| {
                d.track_focus(&join_focus)
                    .seen()
                    .aria_label(trf(
                        "{name} · {address} · No answer",
                        &[("name", &server.name), ("address", &server.ip)],
                    ))
                    .focus_visible(|s| s.bg(rgb(palette().hover)))
            })
            .children(reveal)
            .child(select)
            .child(picture)
            .child(collection::identity(
                ("saved-server", i),
                server.name,
                server.ip,
            ))
            .child(collection::status(narrow, first, second))
            .child(join)
            .child(remove)
    }

    /// Join with what is typed: an invite code joins the friend, an address the server.
    fn join_typed(&mut self) {
        match entry(self.friend_code.text()) {
            Entry::Invite => {
                self.joining = Some(self.friend_code.text().to_string());
                self.play_game();
            }
            Entry::Address(address) => self.play_to("--server", address),
            Entry::Empty | Entry::Unusable => {}
        }
    }

    pub(crate) fn open_add_server(&mut self, cx: &mut Context<Self>) {
        use crate::servers_page::Field;
        let Some(home) = self.selected_instance().map(|i| i.home.clone()) else {
            return;
        };
        self.load_servers(home.clone());
        let address = match entry(self.friend_code.text()) {
            Entry::Address(address) => address,
            _ => String::new(),
        };
        self.multiplayer.adding = Some(home);
        self.multiplayer.error = None;
        self.servers.text(Field::ServerName).set(String::new());
        self.servers.text(Field::ServerAddress).set(address);
        self.focus_handle(Field::ServerName, cx);
        self.servers.focus_soon = Some(Field::ServerName);
    }

    pub(crate) fn close_add_server(&mut self) {
        self.multiplayer.adding = None;
        self.multiplayer.error = None;
    }

    pub(crate) fn add_server_dialog(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        use crate::servers_page::Field;
        self.multiplayer.adding.as_ref()?;
        let field = |this: &mut Self, f, label, example, cx: &mut Context<Self>| {
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(muted(tr(label)))
                .child(
                    this.field(f, example, 352., cx)
                        .w_full()
                        .h(px(36.))
                        .aria_label(tr(label)),
                )
        };
        let name = field(self, Field::ServerName, "Name", "Minecraft Server", cx);
        let address = field(
            self,
            Field::ServerAddress,
            "Address",
            "play.example.net:25565",
            cx,
        );
        let body = div()
            .flex()
            .flex_col()
            .child(crate::sheets::title(tr("Add server").into()))
            .child(name)
            .child(address)
            .children(self.multiplayer.error.as_ref().map(|e| {
                div()
                    .id("join-add-error")
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(palette().warning))
                    .line_clamp(2)
                    .tooltip(crate::glass_ui::tip(e.clone()))
                    .child(e.clone())
            }))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        button("join-add-cancel", tr("Cancel")).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.close_add_server();
                                cx.notify();
                            },
                        )),
                    )
                    .child(crate::frame::primary("join-add-save", tr("Add")).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.add_server();
                            cx.notify();
                        }),
                    )),
            );
        Some(crate::sheets::modal_frame(
            "join-add-server",
            410.,
            body,
            cx,
            Launcher::close_add_server,
        ))
    }
}

#[cfg(test)]
#[path = "join/tests.rs"]
mod tests;
