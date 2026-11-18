//! Home: the player's worlds as
//! pictures, one box for servers (an address or a friend's invite code, Save, Join) with the
//! saved list, and the registry's most downloaded mods.
//!
//! Everything shown is read from the player's real launcher data, and nothing is written until
//! the player acts (Save, Remove, Add, Play, Join). Play and Join set `quick_play` or `joining`
//! and call `play_game`, which owns the sign-in, install, prepare, build and trust gates:
//! - worlds: `worlds::list` of the selected setup's `saves/` (`level.dat` and `icon.png`, read
//!   only), Play starts the game straight into one (`quick_play`);
//! - a world without `icon.png` is drawn from `dirt.png` of the player's own prepared game
//!   files (`backend::resources`, nothing is shipped), else a plain charcoal tile, no letter;
//! - servers: the game's own `servers.dat` (`server_list`) with the status ping the Multiplayer
//!   screen sends, Join, Save and Remove; an invite code joins through `join_friend`;
//! - featured servers: a bundled pair, Horizon Minecraft (joinable once its address is set)
//!   and Sunrise's own (inert, coming soon); no backend service;
//! - popular mods: the registry index's most downloaded mods installable on this game, Add is
//!   the Mods page's own install (`mods_install`, with its consent), a row opens the project.
//!   Home never calls `mods_load`, which looks for updates and turns off taken-down mods: it
//!   reads the installed list itself and points installs at the selected setup (`follow_home`);
//!   a row opens the project page of the Mods screens (`mods_show_project`).
//! - signed out with no worlds, the top is the welcome banner (mock 10) in place of worlds.
use super::*;
use crate::frame::Reach as _;
use crate::frame::{button, BG, CARD, GOLD, LINE, MUTED, SOFT, TEXT};
use crate::worlds_page;
use gpui::{rgb, rgba};
use sunrise_launcher::browser::{self, Kind, Project, Query, SourceId};
use sunrise_launcher::worlds::World;

/// Worlds shown across the top.
const WORLDS: usize = 3;
/// Popular mods listed.
const POPULAR: usize = 6;
/// The picture band of a world card, in points.
const BAND: f32 = 150.;
/// An icon is drawn at a whole multiple of its pixels up to about this wide, then cropped.
const BAND_ART: f32 = 480.;
/// One texture tile, in points (16 pixels at 8×).
const TILE: f32 = 128.;
/// The icons of a world or a server are small: anything larger is not one.
const MAX_PICTURE: u64 = 4 << 20;

#[derive(Default)]
pub(crate) enum Popular {
    #[default]
    Unasked,
    Asking,
    Ready(String, Vec<Project>),
    Failed(String),
}

#[derive(Default)]
pub(crate) struct HomeState {
    popular: Popular,
    /// The saved server whose Remove button is showing, and whether it was asked once.
    menu: Option<(usize, bool)>,
    /// The registry projects installed in the selected setup, and which setup that was.
    installed: Vec<String>,
    installed_home: Option<std::path::PathBuf>,
    /// An install was under way last frame: the list is read again when it ends.
    was_busy: bool,
}

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

/// The popular mods to list: those that run on this game, not already installed, `limit` of them
/// in the order given (the registry's most downloaded first).
pub(crate) fn popular_rows(
    found: &[Project],
    game: &str,
    installed: impl Fn(&Project) -> bool,
    limit: usize,
) -> Vec<Project> {
    found
        .iter()
        .filter(|p| p.kind == Kind::Mod && p.support(game).installable() && !installed(p))
        .take(limit)
        .cloned()
        .collect()
}

pub(crate) fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn card() -> Div {
    div()
        .bg(rgb(CARD))
        .border_1()
        .border_color(rgb(SOFT))
        .rounded(px(tokens::R_CARD))
        .overflow_hidden()
}

fn heading(label: &str) -> Div {
    div()
        .text_size(px(tokens::HEADING))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(TEXT))
        .child(crate::selectable::selectable(label.to_string()))
}

fn muted(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(tokens::SMALL))
        .text_color(rgb(MUTED))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(text.into())
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
            .rounded(px(tokens::R_CONTROL))
            .into_any_element(),
        (None, Some(Badge::Horizon)) => horizon_mark(36.).into_any_element(),
        (None, Some(Badge::Sunrise)) => crate::mark(36.).into_any_element(),
        (None, None) => tile(name, 36.).into_any_element(),
    }
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
        .rounded(px(tokens::R_CONTROL))
        .bg(gpui::hsla(hue, 0.28, 0.24, 1.))
        .text_size(px(side * 0.45))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(gpui::hsla(hue, 0.45, 0.82, 1.))
        .child(initial(label))
}

impl Launcher {
    pub(crate) fn home(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(home) = self.selected_instance().map(|i| i.home.clone()) else {
            return div()
                .p(px(32.))
                .text_color(rgb(MUTED))
                .child(tr("Getting ready…"))
                .into_any_element();
        };
        self.load_worlds(home.clone(), cx);
        self.load_servers(home.clone());
        self.ping_servers(cx);
        self.mods.follow_home(Some(home.clone()));
        self.refresh_installed(&home);
        self.ping_featured(cx);
        self.popular_load(cx);
        let first_start = self.accounts.selected().is_none() && self.worlds.list.is_empty();
        let worlds = if first_start {
            self.welcome(cx)
        } else {
            self.worlds(cx)
        };
        let servers = self.servers(cx);
        let popular = self.popular(cx);
        div()
            .id("alt-home")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&crate::frame::scrolls("home"))
            .bg(rgb(BG))
            .flex()
            .flex_col()
            .gap(px(28.))
            .px(px(32.))
            .py(px(28.))
            // Nothing here gives up height to fit: the page scrolls instead.
            .child(worlds.flex_none())
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap(px(28.))
                    .items_start()
                    .child(div().flex_1().min_w(px(0.)).child(servers))
                    .child(div().flex_1().min_w(px(0.)).child(popular)),
            )
            .into_any_element()
    }

    fn worlds(&mut self, cx: &mut Context<Self>) -> Div {
        let list: Vec<World> = self.worlds.list.iter().take(WORLDS).cloned().collect();
        let mut row = div().flex().gap(px(16.));
        for world in list.iter() {
            let art = self.world_art(world);
            row = row.child(self.world_card(world, art, cx));
        }
        let body = if list.is_empty() {
            let saves = self.worlds.home.clone().map(|h| h.join("saves"));
            card()
                .p(px(20.))
                .flex()
                .items_center()
                .gap(px(12.))
                .child(muted(tr("No worlds yet. Make one in the game.")))
                .children(saves.map(|saves| {
                    button("alt-worlds-folder", tr("Open folder")).on_click(move |_, _, cx| {
                        let _ = std::fs::create_dir_all(&saves);
                        crate::glass_toast::open(cx, &saves);
                    })
                }))
        } else {
            row
        };
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(heading(tr("Your worlds")))
            .child(body)
    }

    /// A world's picture: its own `icon.png`, drawn big and sharp; else `dirt.png` of the
    /// player's own game files, darkened; else plain charcoal.
    fn world_art(&mut self, world: &World) -> Div {
        let scale = self.scale;
        let band = div()
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .w_full()
            .h(px(BAND))
            .overflow_hidden()
            .bg(rgb(BG));
        let stamp = world
            .last_played
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        if let Some(icon) = world.icon.clone() {
            let key = format!("alt-world:{}:{stamp}", icon.display());
            let read = icon.clone();
            let picture = self.icon_image(key, BAND_ART, true, move || {
                std::fs::metadata(&read)
                    .ok()
                    .filter(|m| m.len() <= MAX_PICTURE)
                    .and_then(|_| std::fs::read(&read).ok())
            });
            if let Some(image) = picture {
                let dim = image.size(0);
                return band.child(
                    gpui::img(image)
                        .flex_none()
                        .w(px(dim.width.0 as f32 / scale))
                        .h(px(dim.height.0 as f32 / scale)),
                );
            }
        }
        // Always dirt, from the player's own prepared files; none shipped.
        let file = self
            .game_resources()
            .join("assets/minecraft/textures/block/dirt.png");
        let key = format!("alt-dirt:{}", file.display());
        let texture = self.icon_image(key, TILE, true, move || std::fs::read(file).ok());
        match texture {
            Some(image) => {
                let dim = image.size(0);
                let (w, h) = (dim.width.0 as f32 / scale, dim.height.0 as f32 / scale);
                let mut tiles = div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .flex()
                    .flex_wrap()
                    .w(px(w * 8.));
                for _ in 0..16 {
                    tiles = tiles.child(gpui::img(image.clone()).flex_none().w(px(w)).h(px(h)));
                }
                band.child(tiles)
                    .child(div().absolute().inset_0().bg(rgba(0x0000_0073)))
            }
            // No dirt: plain charcoal a step above the card, no letter.
            None => band.bg(rgb(0x2b2627)),
        }
    }

    fn world_card(&mut self, world: &World, art: Div, cx: &mut Context<Self>) -> Div {
        let folder = world.folder.clone();
        let packs = {
            let path = world.path.clone();
            Some(
                button(format!("alt-world-packs-{}", world.folder), "")
                    .size(px(36.))
                    .px_0()
                    .child(blocks::icon("archive", 17., rgb(TEXT).into()))
                    .tooltip(crate::glass_ui::tip(tr("Data packs")))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.show_world_packs(path.clone());
                        cx.notify();
                    })),
            )
        };
        // Icons, named by their tooltips: the card keeps its room for the world's name.
        let play = button(format!("alt-world-{}", world.folder), "")
            .size(px(36.))
            .px_0()
            .bg(rgb(TEXT))
            .border_color(rgb(TEXT))
            .hover(|s| s.bg(rgb(0xffffff)))
            .child(blocks::icon("play", 17., rgb(BG).into()))
            .tooltip(crate::glass_ui::tip(tr("Play this world")))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.play_to("--world", folder.clone());
                cx.notify();
            }));
        let facts = format!(
            "{} · {}",
            tr(world.mode),
            sentence(&worlds_page::ago(world.last_played))
        );
        card()
            .flex_1()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .child(art)
            .child(
                div()
                    .flex()
                    // A narrow card puts its buttons under the name rather than squeezing it.
                    .flex_wrap()
                    .items_center()
                    .gap(px(10.))
                    .p(px(14.))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(120.))
                            .child(
                                div()
                                    .text_size(px(tokens::BODY))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(TEXT))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(world.name.clone()),
                            )
                            .child(muted(facts)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .gap(px(8.))
                            .children(packs)
                            .child(play),
                    ),
            )
    }

    fn servers(&mut self, cx: &mut Context<Self>) -> Div {
        let typed = entry(self.friend_code.text());
        let focused = self.friend_focused;
        let field = div()
            .id("alt-address")
            .track_focus(&self.friend_focus)
            .seen()
            .flex_1()
            .min_w(px(0.))
            .h(px(tokens::FIELD))
            .px(px(12.))
            .flex()
            .items_center()
            .overflow_hidden()
            .rounded(px(tokens::R_CONTROL))
            .bg(rgb(BG))
            .border_1()
            .border_color(rgb(if focused { GOLD } else { LINE }))
            .text_size(px(tokens::BODY))
            .text_color(rgb(TEXT))
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
        let save = button("alt-server-save", tr("Save"));
        let save = if address {
            save.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.save_typed();
                cx.notify();
            }))
        } else {
            save.opacity(0.4).cursor_default().tab_stop(false)
        };
        let joinable = matches!(typed, Entry::Address(_) | Entry::Invite);
        let join = button("alt-server-join", tr("Join"));
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
                    .text_color(rgb(GOLD))
                    .child(e),
            );
        }
        if let Entry::Unusable = typed {
            list = list.child(
                div()
                    .px(px(14.))
                    .py(px(8.))
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(MUTED))
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
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(heading(tr("Servers")))
            .child(
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
                let join = button(format!("alt-featured-{i}"), tr("Join")).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.play_to("--server", address.to_string());
                        cx.notify();
                    },
                ));
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
            .border_color(rgb(SOFT))
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
                            .text_color(rgb(TEXT))
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
    ) -> Div {
        use multiplayer_page::Reach;
        let reach = self
            .multiplayer
            .reach
            .get(&server.ip)
            .copied()
            .unwrap_or(Reach::Asking);
        let line = match reach {
            Reach::Online { players, max } => trf(
                "{online} of {max}",
                &[("online", &players.to_string()), ("max", &max.to_string())],
            ),
            Reach::Offline => tr("No answer").to_string(),
            Reach::Asking => server.ip.clone(),
        };
        let offline = reach == Reach::Offline;
        let ip = server.ip.clone();
        let join = button(format!("alt-server-{i}-join"), tr("Join"));
        let join = if offline {
            join.opacity(0.4)
                .cursor_default()
                .tooltip(crate::glass_ui::tip(tr(
                    "This server doesn't answer right now",
                )))
        } else {
            join.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.play_to("--server", ip.clone());
                cx.notify();
            }))
        };
        let open = self.ui.home.menu.filter(|(row, _)| *row == i);
        let more = match open {
            Some((_, asked)) => button(
                format!("alt-server-{i}-remove"),
                if asked {
                    tr("Click again to remove")
                } else {
                    tr("Remove")
                },
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if asked {
                    this.ui.home.menu = None;
                    this.edit_servers(|list| list.remove(i).map(|_| ()));
                } else {
                    this.ui.home.menu = Some((i, true));
                }
                cx.notify();
            })),
            None => button(format!("alt-server-{i}-more"), "…").on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    this.ui.home.menu = Some((i, false));
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
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(14.))
            .py(px(10.))
            .border_t_1()
            .border_color(rgb(SOFT))
            .child(picture)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .child(
                        div()
                            .text_size(px(tokens::BODY))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(TEXT))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(server.name),
                    )
                    .child(muted(line)),
            )
            .child(join)
            .child(more)
    }

    /// Every destination goes through `play_game`, which runs the sign-in, install, prepare,
    /// build and trust gates before the game starts.
    fn play_to(&mut self, flag: &'static str, value: String) {
        self.quick_play = Some((flag, value));
        self.play_game();
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

    /// Reads what the registry installed in this setup (a file read, nothing else), when the
    /// setup changes or an install just ended.
    fn refresh_installed(&mut self, home: &std::path::Path) {
        let busy = self.mods.busy.is_some();
        let state = &mut self.ui.home;
        let ended = state.was_busy && !busy;
        state.was_busy = busy;
        if state.installed_home.as_deref() == Some(home) && !ended {
            return;
        }
        state.installed_home = Some(home.to_path_buf());
        state.installed = browser::installed::Installed::load(home)
            .entries
            .into_iter()
            .filter(|e| e.source == SourceId::Sunrise)
            .map(|e| e.project)
            .collect();
    }

    /// The first start: signed out, nothing to play yet (mock 10). Nothing is read from outside
    /// the launcher's own data.
    fn welcome(&mut self, cx: &mut Context<Self>) -> Div {
        card()
            .flex()
            .flex_col()
            .gap(px(10.))
            .px(px(34.))
            .py(px(40.))
                    .child(
                        div()
                            .text_size(px(tokens::DISPLAY))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(TEXT))
                            .child(tr("Welcome to Sunrise")),
                    )
                    .child(
                        div().text_size(px(tokens::BODY)).text_color(rgb(TEXT)).child(crate::selectable::selectable(tr(
                            "Sign in with the Microsoft account that owns Minecraft: Java Edition. Sunrise then installs itself and you can play.",
                        ))),
                    )
                    .child(div().text_size(px(tokens::SMALL)).text_color(rgb(MUTED)).child(crate::selectable::selectable(tr(
                        "While you play, Sunrise counts you as playing once a minute. Nothing else is sent.",
                    ))))
                    .child(
                        div()
                            .id("alt-privacy")
.reach()
                            .cursor_pointer()
                            .text_size(px(tokens::LABEL))
                            .text_color(rgb(GOLD))
                            .child(tr("Privacy choices"))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.settings_open(crate::settings::Section::Privacy);
                                this.ui.page = crate::frame::Page::Settings;
                                cx.notify();
                            })),
                    )
    }

    /// Asks the registry for the most downloaded mods once per game version.
    fn popular_load(&mut self, cx: &mut Context<Self>) {
        let game = self.mods_game();
        match &self.ui.home.popular {
            Popular::Asking | Popular::Failed(_) => return,
            Popular::Ready(known, _) if *known == game => return,
            _ => {}
        }
        self.ui.home.popular = Popular::Asking;
        let query = Query {
            game: game.clone(),
            // A few more than listed: some may be installed already.
            limit: POPULAR * 4,
            ..Query::new("", Kind::Mod)
        };
        let task = cx.background_executor().spawn(async move {
            browser::source(SourceId::Sunrise)
                .ok_or_else(|| tr("The mod registry isn't set up.").to_string())
                .and_then(|source| source.search(&query))
        });
        cx.spawn(async move |this, cx| {
            let found = task.await;
            let _ = this.update(cx, |this, cx| {
                this.ui.home.popular = match found {
                    Ok(found) => Popular::Ready(game, found),
                    Err(e) => Popular::Failed(e),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn popular(&mut self, cx: &mut Context<Self>) -> Div {
        let head = div()
            .flex()
            .items_center()
            .justify_between()
            .child(heading(tr("Popular mods")))
            .child(
                div()
                    .id("alt-browse-mods")
                    .reach()
                    .cursor_pointer()
                    .text_size(px(tokens::LABEL))
                    .text_color(rgb(GOLD))
                    .child(tr("Browse all mods"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.ui.page = crate::frame::Page::Discover;
                        cx.notify();
                    })),
            );
        let game = self.mods_game();
        let body = match &self.ui.home.popular {
            Popular::Ready(_, found) => {
                let rows = popular_rows(
                    found,
                    &game,
                    |p| self.ui.home.installed.contains(&p.id),
                    POPULAR,
                );
                if rows.is_empty() {
                    card()
                        .p(px(14.))
                        .child(muted(tr("Nothing to suggest right now.")))
                } else {
                    let mut list = card();
                    for (i, project) in rows.into_iter().enumerate() {
                        list = list.child(self.popular_row(i, project, cx));
                    }
                    list
                }
            }
            Popular::Failed(why) => card()
                .p(px(14.))
                .flex()
                .items_center()
                .gap(px(12.))
                .child(muted(trf(
                    "The mod registry didn't answer: {why}",
                    &[("why", why)],
                )))
                .child(
                    button("alt-popular-retry", tr("Try again")).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.ui.home.popular = Popular::Unasked;
                            cx.notify();
                        },
                    )),
                ),
            Popular::Unasked | Popular::Asking => card()
                .p(px(14.))
                .child(muted(tr("Asking the mod registry…"))),
        };
        let note = self.mods.message.clone().map(|m| match m {
            Ok(text) => muted(text),
            Err(text) => muted(text).text_color(rgb(GOLD)),
        });
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(head)
            .child(body)
            .children(note)
    }

    fn popular_row(&mut self, i: usize, project: Project, cx: &mut Context<Self>) -> Div {
        let busy = self
            .mods
            .busy
            .as_ref()
            .map(|(id, line, _)| (id.clone(), line.clone()));
        let this_busy = busy.as_ref().is_some_and(|(id, _)| *id == project.id);
        let icon = self.mods_icon(&project, 36.);
        let add = button(
            format!("alt-popular-{i}-add"),
            tr(if this_busy { "Adding…" } else { "Add" }),
        );
        let add = if !self.launch_busy() && self.mods_locked().is_none() {
            let p = project.clone();
            add.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if !this.launch_busy() && this.mods_locked().is_none() {
                    this.mods_install(p.source, p.id.clone(), Some(p.clone()));
                }
                cx.notify();
            }))
        } else {
            add.opacity(0.4).cursor_default().tab_stop(false)
        };
        let open = project.id.clone();
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(14.))
            .py(px(10.))
            .when(i > 0, |d| d.border_t_1().border_color(rgb(SOFT)))
            .child(icon)
            .child(
                div()
                    .id(SharedString::from(format!("alt-popular-{i}-open")))
                    .reach()
                    .flex_1()
                    .min_w(px(0.))
                    .cursor_pointer()
                    .child(
                        div()
                            .text_size(px(tokens::BODY))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(TEXT))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(project.title.clone()),
                    )
                    .child(muted(match busy.filter(|_| this_busy) {
                        Some((_, line)) => line,
                        None => project.summary.clone(),
                    }))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.mods_show_project(open.clone());
                        cx.notify();
                    })),
            )
            .child(add)
    }
}

#[cfg(test)]
#[path = "home/tests.rs"]
mod tests;
