//! The Servers tab: the profile's server list (`servers.dat`) and the servers this computer
//! hosts, as cards under small headings, as the Worlds tab draws its own. The header's actions
//! are a refresh of the answers, Host a server (the dialog that creates one) and Add server (a
//! dialog that saves an address, or joins it once without saving: quick join); the cards
//! (`crate::join`, `crate::servers_page`) have Join or Start and a "…" menu, and a saved
//! server's tick for the bar's Remove.
use super::*;
use crate::join::{featured, featured_at, section, Featured};
use crate::widgets::button::{button, icon_button, Look, Size, Style};
use std::rc::Rc;

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
        let saved = self.multiplayer.list.clone();
        // A featured server that is saved already, on any port, shows once: as the saved card.
        let unsaved: Vec<(usize, Featured)> = featured()
            .into_iter()
            .enumerate()
            .filter(|(_, server)| {
                !saved
                    .iter()
                    .any(|(s, _)| featured_at(&s.ip).as_ref() == Some(server))
            })
            .collect();
        let count = unsaved.len() + saved.len() + self.hosted_count();
        let none_saved = saved.is_empty();
        let shown: Vec<(usize, Featured)> = unsaved
            .into_iter()
            .filter(|(_, server)| {
                search::matches(&query, &[tr(server.name), server.address.unwrap_or("")])
            })
            .collect();
        // Each match keeps its place in the saved list: the menu and the ticks work by it.
        let found: Vec<_> = saved
            .into_iter()
            .enumerate()
            .filter(|(_, (server, _))| {
                search::matches(&query, &[server.name.as_str(), server.ip.as_str()])
            })
            .collect();
        let order: Rc<Vec<String>> = Rc::new(found.iter().map(|(i, _)| i.to_string()).collect());
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
                let mut cards = div().flex().flex_col().gap(px(GAP));
                for (place, entry) in found {
                    cards = cards.child(self.saved_server_card(place, entry, &order, cx));
                }
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

    /// The tab row's actions: refresh the answers, Host a server and Add server.
    fn servers_actions(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let refresh = icon_button(
            "home-servers-refresh",
            "refresh",
            tr("Refresh"),
            Style::Secondary,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.refresh_servers();
            this.refresh_hosted();
            cx.notify();
        }));
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
            .child(refresh)
            .child(host)
            .child(add)
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
