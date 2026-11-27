use super::*;
use crate::widgets::button::secondary;

impl Launcher {
    pub(super) fn home_servers(
        &mut self,
        instance: &instances::Instance,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> Content {
        self.load_servers(instance.home.clone());
        let count = self.multiplayer.list.len();
        let view = &self.ui.home.tabs.views[Tab::Servers.index()];
        let (built, place) = view.near.list(&view.scroll, count, 1, ROW, 0.);
        let start = built.start;
        let rows = self.multiplayer.list[built].to_vec();
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
        } else {
            for (offset, (server, saved)) in rows.into_iter().enumerate() {
                let index = start + offset;
                let status = match self.multiplayer.reach.get(&server.ip) {
                    Some(multiplayer_page::Reach::Online { players, max }) => Some(trf(
                        "{online} of {max}",
                        &[("online", &players.to_string()), ("max", &max.to_string())],
                    )),
                    Some(multiplayer_page::Reach::Offline) => Some(tr("No answer").into()),
                    _ => None,
                };
                let icon = saved.or_else(|| self.multiplayer.icons.get(&server.ip).cloned());
                let picture =
                    crate::mod_icons::tile(icon, &server.ip, &server.name, 30.).into_any_element();
                items = items.child(self.home_overview_row(
                    RowDisplay {
                        id: format!("home-server:{index}:{}:{}", server.ip, server.name).into(),
                        picture,
                        title: server.name.clone(),
                        detail: (!compact || status.is_none()).then(|| server.ip.clone()),
                        badge: None,
                        tail: status,
                    },
                    cx,
                    move |this, cx| {
                        this.navigate(Page::Join);
                        this.multiplayer.target = Some(multiplayer_page::Target {
                            index,
                            name: server.name.clone(),
                            address: server.ip.clone(),
                            focus: cx.focus_handle(),
                            revealed: Default::default(),
                        });
                    },
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
            turned_off: 0,
            items,
        }
    }
}
