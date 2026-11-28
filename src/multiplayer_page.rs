//! The selected instance's server list (`sunrise_launcher::server_list`, the game's own
//! `servers.dat`): what Home's Servers tab shows, with each server's
//! picture and the status ping the game's Multiplayer screen sends, and the edits the page makes
//! (add, edit, move, remove).
use super::*;
use crate::line_edit::LineEdit;
use crate::mods::selection::Selection;
use gpui::{Image, ImageFormat};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use sunrise_launcher::server_list::{self, Server};

#[derive(Default)]
pub(super) struct Multiplayer {
    /// The game folder the list is of, and the list as last read with each picture decoded.
    home: Option<PathBuf>,
    pub(super) list: Vec<(Server, Option<Arc<Image>>)>,
    fresh: bool,
    pub(super) error: Option<String>,
    /// What each address answered to the status ping, asked once per list.
    pub(super) reach: HashMap<String, Reach>,
    /// How long each answer took, in milliseconds.
    pub(super) latency: HashMap<String, u128>,
    /// The picture each address sent with that answer, for a server the game hasn't saved one
    /// for.
    pub(super) icons: HashMap<String, Arc<Image>>,
    /// The name field of the server dialog, and its keyboard focus (the address field is
    /// `Launcher::friend_code`).
    pub(super) name: LineEdit,
    pub(super) name_focus: Option<FocusHandle>,
    /// The saved servers ticked, by their place in the list.
    pub(super) picks: Selection,
    /// The dialog over the page, if one is up.
    pub(super) dialog: Option<ServerDialog>,
    /// The "…" menu of a saved server, while it is open.
    pub(super) menu: Option<ServerMenu>,
    /// The featured servers this profile took away (`join::hidden`).
    pub(super) hidden: Vec<String>,
}

/// What is open over the Servers tab.
pub(super) enum ServerDialog {
    /// The form of a new server, or of the saved server at this place.
    Form { edit: Option<usize> },
    /// The question before these saved servers (place and name) are removed.
    Remove(Vec<(usize, String)>),
    /// The question before a featured server (its key in `join::hidden`, and its name) is taken
    /// off the tab.
    RemoveFeatured { key: String, name: String },
}

/// The "…" menu of the saved server at `index` (or, `featured`, of the bundled one), hung from
/// its button (window coordinates).
pub(super) struct ServerMenu {
    pub(super) index: usize,
    pub(super) featured: bool,
    pub(super) button: gpui::Bounds<gpui::Pixels>,
}

/// A saved server as its status ping found it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Reach {
    Asking,
    Online { players: u64, max: u64 },
    Offline,
}

impl Multiplayer {
    /// Read again on the next frame: the game may have changed its list.
    pub(super) fn stale(&mut self) {
        self.fresh = false;
    }
}

impl Launcher {
    /// Asks each listed server for its players once, as the game's Multiplayer list does when it
    /// opens; the cards show the answer ("3 of 10 playing", or "No answer").
    pub(super) fn ping_servers(&mut self, cx: &mut Context<Self>) {
        let asks: Vec<String> = self
            .multiplayer
            .list
            .iter()
            .map(|(s, _)| s.ip.clone())
            .collect();
        for ip in asks {
            self.ask_server(ip, cx);
        }
    }

    /// Asks `address` for its players, unless it was asked already.
    pub(super) fn ask_server(&mut self, address: String, cx: &mut Context<Self>) {
        if self.multiplayer.reach.contains_key(&address) {
            return;
        }
        self.multiplayer
            .reach
            .insert(address.clone(), Reach::Asking);
        let asked = address.clone();
        let task = cx.background_executor().spawn(async move {
            let started = Instant::now();
            let answer = sunrise_launcher::telemetry::server_status(&asked);
            (answer, started.elapsed())
        });
        cx.spawn(async move |this, cx| {
            let (answer, took) = task.await;
            let _ = this.update(cx, |this, cx| {
                if answer.is_some() {
                    this.multiplayer
                        .latency
                        .insert(address.clone(), took.as_millis());
                }
                this.server_answered(address, answer);
                cx.notify();
            });
        })
        .detach();
    }

    /// Asks `address` again, whatever it answered before.
    pub(super) fn refresh_server(&mut self, address: &str, cx: &mut Context<Self>) {
        self.multiplayer.reach.remove(address);
        self.multiplayer.latency.remove(address);
        self.ask_server(address.to_string(), cx);
    }

    /// Forgets every answer, so the next frame asks each server again.
    pub(super) fn refresh_servers(&mut self) {
        self.multiplayer.reach.clear();
        self.multiplayer.latency.clear();
    }

    /// Records what `address` answered to the status ping: its players and its picture.
    pub(super) fn server_answered(
        &mut self,
        address: String,
        answer: Option<(
            sunrise_launcher_telemetry::server_status::Players,
            Option<Vec<u8>>,
        )>,
    ) {
        let reach = match answer {
            Some((players, icon)) => {
                if let Some(png) = icon {
                    let image = Arc::new(Image::from_bytes(ImageFormat::Png, png));
                    self.multiplayer.icons.insert(address.clone(), image);
                }
                Reach::Online {
                    players: players.online,
                    max: players.max,
                }
            }
            None => Reach::Offline,
        };
        self.multiplayer.reach.insert(address, reach);
    }

    pub(super) fn load_servers(&mut self, home: PathBuf) {
        if self.multiplayer.home.as_ref() != Some(&home) {
            self.multiplayer = Multiplayer {
                hidden: crate::join::hidden::load(&home),
                home: Some(home.clone()),
                ..Multiplayer::default()
            };
        }
        if self.multiplayer.fresh {
            return;
        }
        // A few servers and their 64×64 pictures: small enough to read while drawing.
        self.multiplayer.list = server_list::List::load(&home)
            .servers()
            .into_iter()
            .map(|s| {
                let icon = s
                    .icon
                    .clone()
                    .map(|png| Arc::new(Image::from_bytes(ImageFormat::Png, png)));
                (s, icon)
            })
            .collect();
        // A server the game removed drops out of the ticks.
        let every: Vec<String> = (0..self.multiplayer.list.len())
            .map(|i| i.to_string())
            .collect();
        self.multiplayer.picks.retain(&every);
        self.multiplayer.fresh = true;
    }

    pub(super) fn edit_servers(
        &mut self,
        edit: impl FnOnce(&mut server_list::List) -> Result<(), String>,
    ) {
        let Some(home) = self.multiplayer.home.clone() else {
            return;
        };
        let mut list = server_list::List::load(&home);
        self.multiplayer.error = edit(&mut list).and_then(|()| list.save(&home)).err();
        // The places the ticks hold have moved.
        self.multiplayer.picks.clear();
        self.multiplayer.stale();
    }

    /// Removes the saved servers at `places`, whatever order they are given in.
    pub(super) fn remove_servers(&mut self, mut places: Vec<usize>) {
        places.sort_unstable_by(|a, b| b.cmp(a));
        self.edit_servers(|list| {
            for place in places {
                list.remove(place)?;
            }
            Ok(())
        });
    }

    /// Takes the featured server `key` off this profile's Servers tab.
    pub(super) fn hide_featured(&mut self, key: String) {
        if !self.multiplayer.hidden.contains(&key) {
            self.multiplayer.hidden.push(key);
        }
        self.save_hidden();
    }

    /// Puts every featured server back on this profile's Servers tab.
    pub(super) fn show_featured_again(&mut self) {
        self.multiplayer.hidden.clear();
        self.save_hidden();
    }

    fn save_hidden(&mut self) {
        let Some(home) = self.multiplayer.home.clone() else {
            return;
        };
        self.multiplayer.error = crate::join::hidden::save(&home, &self.multiplayer.hidden).err();
    }

    /// Closes the dialog over the Servers tab, and what was typed in it.
    pub(super) fn close_server_dialog(&mut self) {
        self.multiplayer.dialog = None;
        self.multiplayer.error = None;
    }

    /// Shuts the menus and dialogs of the Servers pages, as a page is left.
    pub(super) fn close_server_layers(&mut self) {
        self.multiplayer.menu = None;
        self.multiplayer.dialog = None;
        self.servers.close_layers();
    }

    /// Escape on the Servers tab: the "…" menu or the dialog goes first, then the ticks.
    /// True when it closed or cleared something.
    pub(crate) fn join_escape(&mut self) -> bool {
        if self.multiplayer.menu.take().is_some() {
            crate::glass_ui::menus_changed();
            return true;
        }
        if self.multiplayer.dialog.is_some() {
            self.close_server_dialog();
            return true;
        }
        let any = !self.multiplayer.picks.is_empty();
        self.multiplayer.picks.clear();
        any
    }
}
