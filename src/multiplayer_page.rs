//! The selected profile's server list (`servers.dat`) and the answers to its status requests.
use super::*;
use gpui::{Image, ImageFormat};
use servers_page::Field;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use sunrise_launcher::server_list::{self, SavedKey, Server};

#[derive(Default)]
pub(super) struct Multiplayer {
    /// The game folder the list is of, and the list as last read with each picture decoded.
    home: Option<PathBuf>,
    pub(super) list: Vec<(Server, Option<Arc<Image>>)>,
    pub(super) keys: Arc<[SavedKey]>,
    fresh: bool,
    /// The profile bound when the Add form opened, not whichever profile is selected later.
    pub(super) adding: Option<PathBuf>,
    pub(super) error: Option<String>,
    pub(super) selected: BTreeSet<SavedKey>,
    pub(super) remove: Option<BTreeSet<SavedKey>>,
    pub(super) focuses: BTreeMap<SavedKey, FocusHandle>,
    pub(super) thumb: crate::scrollbar::Thumb,
    /// What each address answered to the status ping, asked once per list.
    pub(super) reach: HashMap<String, Reach>,
    /// The picture each address sent with that answer, for a server the game hasn't saved one
    /// for.
    pub(super) icons: HashMap<String, Arc<Image>>,
}

/// A saved server as its status ping found it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Reach {
    Asking,
    Online {
        players: u64,
        max: u64,
        latency: std::time::Duration,
    },
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
    /// opens; the rows show the answer (the mockup's dot and "3 of 10" or "Offline").
    pub(super) fn ping_servers(&mut self, cx: &mut Context<Self>) {
        let asks: Vec<String> = self
            .multiplayer
            .list
            .iter()
            .map(|(s, _)| s.ip.clone())
            .filter(|ip| !self.multiplayer.reach.contains_key(ip))
            .collect();
        for ip in asks {
            self.multiplayer.reach.insert(ip.clone(), Reach::Asking);
            let address = ip.clone();
            let task = cx
                .background_executor()
                .spawn(async move { sunrise_launcher::telemetry::server_status(&address) });
            cx.spawn(async move |this, cx| {
                let answer = task.await;
                let _ = this.update(cx, |this, cx| {
                    this.server_answered(ip, answer);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// Records what `address` answered to the status ping: its players and its picture.
    pub(super) fn server_answered(
        &mut self,
        address: String,
        answer: Option<sunrise_launcher::telemetry::ServerStatus>,
    ) {
        let reach = match answer {
            Some(sunrise_launcher::telemetry::ServerStatus {
                players,
                icon,
                latency,
            }) => {
                if let Some(png) = icon {
                    let image = Arc::new(Image::from_bytes(ImageFormat::Png, png));
                    self.multiplayer.icons.insert(address.clone(), image);
                }
                Reach::Online {
                    players: players.online,
                    max: players.max,
                    latency,
                }
            }
            None => Reach::Offline,
        };
        self.multiplayer.reach.insert(address, reach);
    }

    pub(super) fn load_servers(&mut self, home: PathBuf) {
        if self.multiplayer.home.as_ref() != Some(&home) {
            self.multiplayer = Multiplayer {
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
        let keys = SavedKey::of_list(self.multiplayer.list.iter().map(|(s, _)| s));
        self.multiplayer.selected.retain(|key| keys.contains(key));
        self.multiplayer.focuses.retain(|key, _| keys.contains(key));
        self.multiplayer.keys = keys.into();
        self.multiplayer.fresh = true;
    }

    /// Enter in the address field, or Add.
    pub(super) fn add_server(&mut self) {
        let Some(home) = &self.multiplayer.adding else {
            return;
        };
        if !self.selected_instance().is_some_and(|i| &i.home == home) {
            self.close_add_server();
            return;
        }
        let name = self.servers.value(Field::ServerName);
        let crate::join::Entry::Address(address) =
            crate::join::entry(self.servers.value(Field::ServerAddress))
        else {
            self.multiplayer.error = Some(tr("Enter a server address, not an invite code.").into());
            return;
        };
        self.multiplayer.error = server_list::List::add_saved(home, name, &address).err();
        if self.multiplayer.error.is_none() {
            self.close_add_server();
            self.multiplayer.stale();
        }
    }

    pub(super) fn remove_saved_servers(&mut self) {
        let Some(reviewed) = self.multiplayer.remove.take() else {
            return;
        };
        let Some(home) = &self.multiplayer.home else {
            return;
        };
        if !self.selected_instance().is_some_and(|i| &i.home == home) {
            return;
        }
        self.multiplayer.error = server_list::List::remove_saved(home, &reviewed).err();
        if self.multiplayer.error.is_none() {
            self.multiplayer
                .selected
                .retain(|key| !reviewed.contains(key));
        }
        self.multiplayer.stale();
    }
}
