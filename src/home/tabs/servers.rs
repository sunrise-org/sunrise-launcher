//! The Servers tab: the profile's server list (`servers.dat`) and the servers this computer
//! hosts, as cards under small headings, as the Worlds tab draws its own. The header's actions
//! are Host a server (the dialog that creates one) and Add server (a
//! dialog that saves an address, or joins it once without saving: quick join); the cards
//! (`crate::join`, `crate::servers_page`) have Join or Start and a "…" menu, and a saved
//! server's tick for the bar's Remove.
use super::*;
use crate::join::{featured, featured_at, glides, section, Featured};
use crate::widgets::button::{button, icon_only, Look, Size, Style};
use crate::widgets::glide::{arrive, glide};
use gpui::{AnimationExt, Hsla};
use std::{rc::Rc, sync::Arc, time::Duration};
use sunrise_launcher::server_list::Server;

/// A saved server shown in the list: its place in the saved list, the server and its picture.
type Listed = (usize, (Server, Option<Arc<gpui::Image>>));

impl Launcher {
    pub(super) fn home_servers(
        &mut self,
        instance: &instances::Instance,
        cx: &mut Context<Self>,
    ) -> Content {
        self.load_servers(instance.home.clone());
        self.ping_servers(cx);
        for address in featured().into_iter().filter_map(|f| f.address) {
            self.ask_server(address.to_string(), cx);
        }
        let query = self.ui.home.tabs.query_of(Tab::Servers);
        // Featured servers always show under Featured. A saved entry with a featured address
        // stays out of "Your servers", so no server shows twice and none moves across sections.
        let listed: Vec<Listed> = self
            .multiplayer
            .list
            .iter()
            .cloned()
            .enumerate()
            .filter(|(_, (server, _))| featured_at(&server.ip).is_none())
            .collect();
        let count = featured().len() + listed.len() + self.hosted_count();
        let none_saved = listed.is_empty();
        let shown: Vec<(usize, Featured)> = featured()
            .into_iter()
            .enumerate()
            .filter(|(_, server)| {
                search::matches(&query, &[tr(server.name), server.address.unwrap_or("")])
            })
            .collect();
        // Each match keeps its place in the saved list: the menu and the ticks work by it.
        let found: Vec<usize> = listed
            .iter()
            .filter(|(_, (server, _))| {
                search::matches(&query, &[server.name.as_str(), server.ip.as_str()])
            })
            .map(|(place, _)| *place)
            .collect();
        let order: Rc<Vec<String>> = Rc::new(found.iter().map(|i| i.to_string()).collect());
        // A server the search hides drops out of the ticks.
        self.multiplayer.picks.retain(&order);
        let hosted = self.hosted_section(&query, cx);
        let items = if shown.is_empty() && found.is_empty() && hosted.is_none() {
            search::no_matches()
        } else {
            let mut items = div().flex().flex_col().gap(px(18.));
            if !shown.is_empty() {
                let mut cards = div().flex().flex_col().gap(px(GAP));
                for (n, server) in shown {
                    cards = cards.child(self.featured_server_card(n, server, cx));
                }
                items = items.child(section(tr("Featured"), cards));
            }
            if !found.is_empty() {
                let cards = self.saved_server_list(listed, &query, &order, cx);
                items = items.child(section(tr("Your servers"), cards));
            } else if none_saved && query.is_empty() {
                items = items.child(section(
                    tr("Your servers"),
                    crate::home::muted(tr("Servers you join or add show up here.")),
                ));
            }
            items.children(hosted)
        };
        let pick_bar = self.server_pick_bar(&order, cx);
        self.home_set_bar(pick_bar);
        Content {
            count: Some(count),
            manage: Some(self.servers_actions(cx)),
            items,
        }
    }

    /// The tab row's actions: Host a server and Add server.
    fn servers_actions(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let host = button(
            "home-servers-host",
            tr("Host a server"),
            Look::new(Style::Secondary, Size::Medium),
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.open_host_create(cx);
            cx.notify();
        }));
        let add = button(
            "home-servers-add",
            tr("Add server"),
            Look::new(Style::Secondary, Size::Medium).icon("plus-sign"),
        )
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.open_server_form(None, window, cx);
            cx.notify();
        }));
        div()
            .id("home-servers-actions")
            .flex()
            .items_center()
            .gap(px(8.))
            .child(host)
            .child(add)
    }

    /// Refresh, an icon beside the search field as Check for updates is on the Mods tab: it asks
    /// every server again, and its icon turns while an answer is still on its way (still under
    /// reduced motion).
    pub(super) fn servers_refresh_button(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let look = Look::new(Style::Secondary, Size::Medium);
        let asking = self
            .multiplayer
            .reach
            .values()
            .any(|reach| *reach == crate::multiplayer_page::Reach::Asking);
        if asking {
            let ink = Hsla::from(gpui::rgb(crate::paint::palette().text));
            let icon = crate::glass_ui::icon("refresh", 18., ink);
            let spinning = if crate::motion::still() {
                icon.into_any_element()
            } else {
                icon.with_animation(
                    "home-servers-refresh-spin",
                    crate::motion::endless(
                        gpui::Animation::new(Duration::from_millis(900)).repeat(),
                    ),
                    |icon, t| {
                        icon.with_transformation(gpui::Transformation::rotate(gpui::radians(
                            t * std::f32::consts::TAU,
                        )))
                    },
                )
                .into_any_element()
            };
            return icon_only("home-servers-refresh", look.disabled(true))
                .child(spinning)
                .tooltip(crate::glass_ui::tip(tr("Refreshing…")));
        }
        icon_only("home-servers-refresh", look.icon("refresh"))
            .tooltip(crate::glass_ui::tip(tr("Refresh")))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.refresh_servers();
                this.refresh_hosted();
                cx.notify();
            }))
    }

    /// The places of the saved servers "Your servers" lists (those that aren't a featured
    /// server's), in order: what Move up and Move down step through.
    pub(crate) fn listed_places(&self) -> Vec<usize> {
        self.multiplayer
            .list
            .iter()
            .enumerate()
            .filter(|(_, (server, _))| featured_at(&server.ip).is_none())
            .map(|(place, _)| place)
            .collect()
    }

    /// The places Ctrl+A ticks: the listed servers the search leaves.
    pub(crate) fn home_server_ids(&self) -> Vec<String> {
        let query = self.ui.home.tabs.query_of(Tab::Servers);
        let listed = self.listed_places();
        listed
            .into_iter()
            .filter(|place| {
                self.multiplayer
                    .list
                    .get(*place)
                    .is_some_and(|(server, _)| {
                        search::matches(&query, &[server.name.as_str(), server.ip.as_str()])
                    })
            })
            .map(|place| place.to_string())
            .collect()
    }

    /// Ctrl+A on the Servers tab: ticks every saved server shown. True when there was one.
    pub(crate) fn servers_tick_all(&mut self) -> bool {
        let ids = self.home_server_ids();
        self.multiplayer.picks.select_all(&ids);
        !ids.is_empty()
    }

    /// "Your servers": the saved servers `listed` (place and entry) that match `query`, as cards
    /// that glide when one is moved, fade in when added, and leave a ghost that folds shut when
    /// removed (`join::glides`).
    fn saved_server_list(
        &mut self,
        listed: Vec<Listed>,
        query: &str,
        order: &Rc<Vec<String>>,
        cx: &mut Context<Self>,
    ) -> Div {
        let names: Vec<(String, String)> = listed
            .iter()
            .map(|(_, (server, _))| (server.name.clone(), server.ip.clone()))
            .collect();
        let keys = glides::keys(&names);
        let seen = listed
            .iter()
            .zip(&keys)
            .map(|((_, (server, icon)), key)| {
                (
                    key.clone(),
                    glides::Seen {
                        name: server.name.clone(),
                        address: server.ip.clone(),
                        icon: icon.clone(),
                    },
                )
            })
            .collect();
        let step = crate::join::ROW + GAP;
        let playing = glides::note(seen, step);
        let mut cards = div().flex().flex_col().gap(px(GAP));
        for ghost in playing.after(None) {
            cards = cards.children(crate::join::ghost_card(ghost, query));
        }
        for ((place, entry), key) in listed.into_iter().zip(keys) {
            let shown = search::matches(query, &[entry.0.name.as_str(), entry.0.ip.as_str()]);
            if shown {
                let card = self.saved_server_card(place, entry, order, cx);
                cards = cards.child(match (playing.glides.get(&key), playing.born.get(&key)) {
                    (Some(started), _) => glide(card, "server-glide", &key, *started),
                    (None, Some(born)) => arrive(card, "server-glide", &key, *born, 10.),
                    (None, None) => card.into_any_element(),
                });
            }
            for ghost in playing.after(Some(key.as_str())) {
                cards = cards.children(crate::join::ghost_card(ghost, query));
            }
        }
        cards
    }

    /// Goes to Home with the Servers tab showing: where the servers are joined from and hosted.
    pub(crate) fn show_saved_servers(&mut self) {
        self.ui.home.tabs.selected = Tab::Servers;
        self.navigate(Page::Home);
    }

    /// Whether Home's Servers tab is the one showing.
    pub(crate) fn home_on_servers(&self) -> bool {
        self.ui.home.tabs.selected == Tab::Servers
    }

    /// Escape on the Servers tab: closes a menu or dialog (the Create dialog, a hosted server's
    /// menu), or clears its ticks. True when it did.
    pub(crate) fn servers_escape(&mut self) -> bool {
        self.ui.page == Page::Home
            && self.home_on_servers()
            && (self.hosting_escape() || self.join_escape())
    }
}
