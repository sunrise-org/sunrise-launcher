use super::*;

pub(super) struct Target {
    profile: String,
    address: String,
    revealed: Rc<Cell<bool>>,
}

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
        let rows = self.multiplayer.list[built].to_vec();
        let mut items = div().flex().flex_col();
        if count == 0 {
            items = empty(
                "server",
                tr("No saved servers"),
                tr("Servers you join or add show up here."),
                Some(
                    button("home-no-servers", tr("Open Servers")).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.navigate(Page::Join);
                            cx.notify();
                        },
                    )),
                ),
            );
        } else {
            for (server, saved) in rows {
                let status = match self.multiplayer.reach.get(&server.ip) {
                    Some(multiplayer_page::Reach::Online { players, max }) => Some(trf(
                        "{online} of {max}",
                        &[("online", &players.to_string()), ("max", &max.to_string())],
                    )),
                    Some(multiplayer_page::Reach::Offline) => Some(tr("No answer").into()),
                    _ => None,
                };
                let id = SharedString::from(format!("home-server:{}:{}", server.ip, server.name));
                let icon = saved.or_else(|| self.multiplayer.icons.get(&server.ip).cloned());
                let picture =
                    crate::mod_icons::tile(icon, &server.ip, &server.name, 30.).into_any_element();
                let address = server.ip;
                let profile = instance.id.clone();
                items = items.child(self.home_overview_row(
                    id,
                    picture,
                    server.name,
                    (!compact || status.is_none()).then(|| address.clone()),
                    status,
                    cx,
                    move |this| {
                        this.ui.home.tabs.server = Some(Target {
                            profile: profile.clone(),
                            address: address.clone(),
                            revealed: Default::default(),
                        });
                        this.navigate(Page::Join);
                    },
                ));
            }
            items = place(items);
        }
        let manage = button("home-open-servers", tr("Open Servers")).on_click(cx.listener(
            |this, _, _, cx| {
                this.navigate(Page::Join);
                cx.notify();
            },
        ));
        Content {
            count: Some(count),
            manage: (count > 0).then_some(manage),
            extra: None,
            items,
        }
    }

    /// Attach inside the saved row in Join. An offline row can pass its own focus handle.
    #[allow(dead_code)] // The Servers rollup wires the canonical Join row to this handoff.
    pub(crate) fn home_server_target(
        &self,
        address: &str,
        focus: &FocusHandle,
    ) -> Option<impl IntoElement + use<>> {
        let target = self.ui.home.tabs.server.as_ref()?;
        if target.address != address
            || self.selected_instance()?.id != target.profile
            || target.revealed.get()
        {
            return None;
        }
        Some(crate::mods::reveal(
            crate::frame::scrolls("join"),
            target.revealed.clone(),
            Some(focus.clone()),
        ))
    }
}
