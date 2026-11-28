use super::*;
use crate::widgets::button::secondary;
use crate::widgets::focus::c;
use gpui::Image;
use std::sync::Arc;
use sunrise_launcher::server_list::Server;

/// A saved server and its picture, as the list keeps them.
type Saved = (Server, Option<Arc<Image>>);

impl Launcher {
    pub(super) fn home_servers(
        &mut self,
        instance: &instances::Instance,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> Content {
        self.load_servers(instance.home.clone());
        let count = self.multiplayer.list.len();
        let query = self.ui.home.tabs.query_of(Tab::Servers);
        // Each match keeps its place in the saved list: the Join page finds the server by it.
        let found: Vec<(usize, Saved)> = self
            .multiplayer
            .list
            .iter()
            .enumerate()
            .filter(|(_, (server, _))| {
                search::matches(&query, &[server.name.as_str(), server.ip.as_str()])
            })
            .map(|(index, entry)| (index, entry.clone()))
            .collect();
        let view = &self.ui.home.tabs.views[Tab::Servers.index()];
        let (built, place) = view.near.list(&view.scroll, found.len(), 1, ROW, 0.);
        let rows = found[built].to_vec();
        let mut items = div().flex().flex_col();
        if count == 0 {
            items = empty(
                tr("No saved servers"),
                tr("Servers you join or add show up here."),
                Some(
                    secondary("home-no-servers", tr("Open Servers")).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.navigate(Page::Join);
                            cx.notify();
                        },
                    )),
                ),
            );
        } else if found.is_empty() {
            items = search::no_matches();
        } else {
            for (index, (server, saved)) in rows {
                let reach = self.multiplayer.reach.get(&server.ip);
                let offline = matches!(reach, Some(multiplayer_page::Reach::Offline));
                let status = match reach {
                    Some(multiplayer_page::Reach::Online { players, max }) => Some(trf(
                        "{online} of {max}",
                        &[("online", &players.to_string()), ("max", &max.to_string())],
                    )),
                    Some(multiplayer_page::Reach::Offline) => Some(tr("No answer").into()),
                    _ => None,
                };
                let icon = saved.or_else(|| self.multiplayer.icons.get(&server.ip).cloned());
                let picture =
                    crate::mod_icons::tile(icon, &server.ip, &server.name, 32.).into_any_element();
                items = items.child(self.saved_server_row(
                    index,
                    (server.name.clone(), server.ip.clone()),
                    picture,
                    (status, offline),
                    compact,
                    cx,
                ));
            }
            items = place(items);
        }
        let manage = secondary("home-open-servers", tr("Open Servers")).on_click(cx.listener(
            |this, _, _, cx| {
                this.navigate(Page::Join);
                cx.notify();
            },
        ));
        Content {
            count: Some(count),
            manage: (count > 0).then_some(manage),
            items,
        }
    }

    /// One saved server as a row, in fixed columns: its picture, its name over its address (mono),
    /// how many are playing (or that it does not answer), then Join. The row opens the server on
    /// the Join page; Join joins it, and is off while the server does not answer.
    fn saved_server_row(
        &mut self,
        index: usize,
        (name, ip): (String, String),
        picture: AnyElement,
        (status, offline): (Option<String>, bool),
        compact: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id: SharedString = format!("home-server:{index}:{ip}:{name}").into();
        let focus = self.ui.home.tabs.focus_for(id.clone(), cx);
        let join = row_join(&ip, offline, cx);
        let tail = status.map(|text| {
            div()
                .flex_none()
                .text_size(px(tokens::LABEL))
                .text_color(c(if offline {
                    palette().muted
                } else {
                    palette().success
                }))
                .child(text)
        });
        let address = div()
            .text_size(px(tokens::SMALL))
            .font_family(crate::theme::MONO)
            .text_color(c(palette().muted))
            .truncate()
            .child(ip.clone());
        let info = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(row_name(name.clone()))
            .children((!compact || tail.is_none()).then_some(address));
        div()
            .id(id)
            .relative()
            .flex_none()
            .min_h(px(52.))
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(12.))
            .border_b_1()
            .border_color(palette().rule())
            .hover(|s| s.bg(c(palette().chrome_raised)))
            .track_focus(&focus)
            .reach()
            .cursor_pointer()
            .child(div().flex_none().child(picture))
            .child(info)
            .children(tail)
            .child(join)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.ui.home.tabs.remember(Some(focus.clone()));
                this.navigate(Page::Join);
                this.multiplayer.target = Some(multiplayer_page::Target {
                    index,
                    name: name.clone(),
                    address: ip.clone(),
                    focus: cx.focus_handle(),
                    revealed: Default::default(),
                });
                cx.notify();
            }))
    }
}

/// Join: joins the server, as the Join page's button does. Off while the server does not answer.
fn row_join(ip: &str, offline: bool, cx: &mut Context<Launcher>) -> Stateful<Div> {
    let button = secondary("home-server-join", tr("Join"));
    if offline {
        return button
            .opacity(0.4)
            .cursor_default()
            .tab_stop(false)
            .tooltip(crate::glass_ui::tip(tr(
                "This server doesn't answer right now",
            )));
    }
    let ip = ip.to_owned();
    button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
        cx.stop_propagation();
        this.play_to("--server", ip.clone());
        cx.notify();
    }))
}
