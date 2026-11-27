//! Join: one box for servers (an address or a friend's invite code, Save, Join) over the saved
//! list.
//!
//! Everything shown is read from the player's real launcher data, and nothing is written until
//! the player acts (Save, Remove, Join). Join passes the destination to
//! `play_game`, which owns the sign-in, install, prepare, build and trust gates:
//! - servers: the game's own `servers.dat` (`server_list`) with the status ping the Multiplayer
//!   screen sends, Join, Save and Remove; an invite code joins through `join_friend`;
//! - featured servers: a bundled pair, Horizon Minecraft (joinable once its address is set)
//!   and Sunrise's own (inert, coming soon); no backend service.
use super::*;
use crate::frame::Reach as _;
use crate::home::{card, muted};
use crate::paint::palette;
use crate::widgets::button::{primary, secondary};
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
            name: crate::i18n::tr_key("Horizon Minecraft"),
            address: Some(horizon).filter(|a| !a.is_empty()),
            note: crate::i18n::tr_key("A Horizon community server"),
            inert: crate::i18n::tr_key("Address coming soon"),
            badge: Badge::Horizon,
        },
        Featured {
            name: crate::i18n::tr_key("Sunrise server"),
            address: None,
            note: crate::i18n::tr_key("Sunrise's own server"),
            inert: crate::i18n::tr_key("Coming soon"),
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
    } else if text.chars().any(|c| c.is_whitespace() || c.is_control()) {
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
            .size(px(36.))
            .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
            .into_any_element(),
        (None, Some(Badge::Horizon)) => horizon_mark(36.).into_any_element(),
        (None, Some(Badge::Sunrise)) => crate::mark(36.).into_any_element(),
        (None, None) => tile(name, 36.).into_any_element(),
    }
}

/// A saved server's status at the row's end: a dot and its words, both in `color`.
fn status_line(color: u32, words: String) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(tokens::SMALL))
        .text_color(rgb(color))
        .child(
            div()
                .flex_none()
                .size(px(8.))
                .rounded(tokens::r(tokens::R_SMALL))
                .bg(rgb(color)),
        )
        .child(words)
}

fn tile(label: &str, side: f32) -> Div {
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

impl Launcher {
    /// The Join page: the servers of the selected profile.
    pub(crate) fn join(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(home) = self.selected_instance().map(|i| i.home.clone()) else {
            return div().into_any_element();
        };
        self.load_servers(home);
        self.ping_servers(cx);
        self.ping_featured(cx);
        let servers = self.servers(cx);
        crate::scrollbar::Scrollbar::page("join")
            .wrap(
                div()
                    .id("alt-join")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(24.))
                    .pt(px(18.))
                    .pb(px(24.))
                    // The list fills the page at every width, as Versions' does.
                    .child(servers),
                &self.theme,
            )
            .size_full()
            .into_any_element()
    }

    fn servers(&mut self, cx: &mut Context<Self>) -> Div {
        let typed = entry(self.friend_code.text());
        let focused = self.friend_focused;
        let field = crate::widgets::field::field_box(focused, false)
            .id("alt-address")
            .track_focus(&self.friend_focus)
            .seen()
            .flex_1()
            .min_w(px(0.))
            .h(px(tokens::FIELD))
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
        let address = matches!(typed, Entry::Address(_));
        // As tall as the address field they stand beside.
        let save = secondary("alt-server-save", tr("Save")).h(px(tokens::FIELD));
        let save = if address {
            save.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.save_typed();
                cx.notify();
            }))
        } else {
            save.opacity(0.4).cursor_default().tab_stop(false)
        };
        let joinable = matches!(typed, Entry::Address(_) | Entry::Invite);
        let join = primary("alt-server-join", tr("Join")).h(px(tokens::FIELD));
        let join = if joinable {
            join.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.join_typed();
                cx.notify();
            }))
        } else {
            join.opacity(0.4).cursor_default().tab_stop(false)
        };
        let mut list = div().flex().flex_col();
        if let Some(e) = self.multiplayer.error.clone() {
            list = list.child(
                div()
                    .px(px(14.))
                    .py(px(8.))
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(palette().warning))
                    .child(e),
            );
        }
        if let Entry::Unusable = typed {
            list = list.child(
                div()
                    .px(px(14.))
                    .py(px(8.))
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(palette().muted))
                    .child(tr("That isn't an address or an invite code.")),
            );
        }
        let saved = self.multiplayer.list.clone();
        let mut shown = 0;
        for server in featured() {
            // Saved already, on any port: it shows once, as the saved row.
            if saved
                .iter()
                .any(|(s, _)| featured_at(&s.ip).as_ref() == Some(&server))
            {
                continue;
            }
            list = list.child(self.featured_row(shown, server, cx));
            shown += 1;
        }
        for (i, (server, icon)) in saved.into_iter().enumerate() {
            list = list.child(self.server_row(i, server, icon, cx));
            shown += 1;
        }
        if shown == 0 {
            list = list.child(
                div()
                    .px(px(14.))
                    .py(px(14.))
                    .child(muted(tr("No saved servers."))),
            );
        }
        div().flex().flex_col().gap(px(12.)).child(
            card()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .p(px(12.))
                        .child(field)
                        .child(save)
                        .child(join),
                )
                .child(list),
        )
    }

    fn featured_row(&mut self, i: usize, server: Featured, cx: &mut Context<Self>) -> Div {
        use multiplayer_page::Reach;
        let (end, line) = match server.address {
            Some(address) => {
                let line = match self.multiplayer.reach.get(address) {
                    Some(Reach::Online { players, .. }) => {
                        trf("{count} playing", &[("count", &players.to_string())])
                    }
                    Some(Reach::Offline) => tr("No answer").to_string(),
                    _ => tr(server.note).to_string(),
                };
                let join = secondary(format!("alt-featured-{i}"), tr("Join")).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.play_to("--server", address.to_string());
                        cx.notify();
                    }),
                );
                (join.into_any_element(), line)
            }
            None => (
                muted(tr(server.inert)).into_any_element(),
                tr(server.note).to_string(),
            ),
        };
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(14.))
            .py(px(10.))
            .border_t_1()
            .border_color(palette().rule())
            // The picture the server sends wins; until it does, the bundled one.
            .child(server_picture(
                server
                    .address
                    .and_then(|a| self.multiplayer.icons.get(a).cloned()),
                Some(server.badge),
                server.name,
            ))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .child(
                        div()
                            .text_size(px(tokens::BODY))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette().text))
                            .child(tr(server.name)),
                    )
                    .child(muted(line)),
            )
            .child(end)
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
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        use multiplayer_page::Reach;
        let target = self
            .multiplayer
            .target
            .as_ref()
            .filter(|target| target.matches(i, &server));
        let reach = self
            .multiplayer
            .reach
            .get(&server.ip)
            .copied()
            .unwrap_or(Reach::Asking);
        // The status the ping gave; nothing while it is still asking.
        let status = match reach {
            Reach::Online { players, max } => Some(status_line(
                palette().success,
                trf(
                    "{online} of {max}",
                    &[("online", &players.to_string()), ("max", &max.to_string())],
                ),
            )),
            Reach::Offline => Some(status_line(palette().muted, tr("No answer").to_string())),
            Reach::Asking => None,
        };
        let address = server.ip.clone();
        let offline = reach == Reach::Offline;
        let ip = server.ip.clone();
        let join = secondary(format!("alt-server-{i}-join"), tr("Join"));
        let join = if offline {
            join.opacity(0.4)
                .cursor_default()
                .tab_stop(false)
                .tooltip(crate::glass_ui::tip(tr(
                    "This server doesn't answer right now",
                )))
        } else {
            join.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.play_to("--server", ip.clone());
                cx.notify();
            }))
        };
        let open = self.multiplayer.menu.filter(|(row, _)| *row == i);
        let more = match open {
            Some((_, asked)) => secondary(
                format!("alt-server-{i}-remove"),
                if asked {
                    tr("Click again to remove")
                } else {
                    tr("Remove")
                },
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if asked {
                    this.multiplayer.menu = None;
                    this.edit_servers(|list| list.remove(i).map(|_| ()));
                } else {
                    this.multiplayer.menu = Some((i, true));
                }
                cx.notify();
            })),
            None => secondary(format!("alt-server-{i}-more"), "…").on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    this.close_menus();
                    this.multiplayer.menu = Some((i, false));
                    cx.notify();
                },
            )),
        };
        // The picture the game saved, else the one the server just sent.
        let picture = server_picture(
            icon.or_else(|| self.multiplayer.icons.get(&server.ip).cloned()),
            featured_at(&server.ip).map(|f| f.badge),
            &server.name,
        );
        div()
            .id(("alt-server-row", i))
            .relative()
            .when_some(target, |row, target| {
                row.track_focus(&target.focus)
                    .bg(rgb(palette().hover))
                    .focus(|s| s.border_color(rgb(palette().focus)))
                    .child(crate::mods::reveal(
                        crate::frame::scrolls("join"),
                        target.revealed.clone(),
                        Some(target.focus.clone()),
                    ))
            })
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(14.))
            .py(px(10.))
            .border_t_1()
            .border_color(palette().rule())
            .child(picture)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .child(
                        div()
                            .text_size(px(tokens::BODY))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette().text))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(server.name),
                    )
                    .child(
                        div()
                            .font_family(theme::MONO)
                            .text_size(px(tokens::SMALL))
                            .text_color(rgb(palette().muted))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(address),
                    ),
            )
            .children(status)
            .child(join)
            .child(more)
    }

    /// Join with what is typed: an invite code joins the friend, an address the server.
    fn join_typed(&mut self) {
        match entry(self.friend_code.text()) {
            Entry::Invite => {
                self.play_game(launcher_play::PlayDestination::Invite(
                    self.friend_code.text().to_string(),
                ));
            }
            Entry::Address(address) => self.play_to("--server", address),
            Entry::Empty | Entry::Unusable => {}
        }
    }

    /// Save with what is typed: the address goes into the game's own server list.
    fn save_typed(&mut self) {
        let Entry::Address(address) = entry(self.friend_code.text()) else {
            return;
        };
        self.edit_servers(|list| list.add(&address, &address));
        if self.multiplayer.error.is_none() {
            self.friend_code.set(String::new());
        }
    }
}

#[cfg(test)]
mod tests;
