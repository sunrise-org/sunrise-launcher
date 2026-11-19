use super::*;
use sunrise_launcher::server_list::SavedKey;

pub(super) struct Target {
    profile: String,
    key: SavedKey,
    revealed: Rc<Cell<bool>>,
}

impl Launcher {
    pub(super) fn home_servers(
        &mut self,
        instance: &instances::Instance,
        compact: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Content {
        self.load_servers(instance.home.clone());
        let count = self.multiplayer.list.len();
        let view = &self.ui.home.tabs.views[Tab::Servers.index()];
        let (built, place) = view.near.list(&view.scroll, count, 1, ROW, 0.);
        let keys = self.multiplayer.keys[built.clone()].to_vec();
        let rows = self.multiplayer.list[built].to_vec();
        let mut items = div().flex().flex_col();
        if count == 0 {
            items = empty(
                tr("No saved servers"),
                button("home-no-servers", tr("Add server")).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.navigate(Page::Join);
                        this.open_add_server(cx);
                        cx.notify();
                    },
                )),
            );
        } else {
            for ((server, saved), key) in rows.into_iter().zip(keys) {
                let status = match self.multiplayer.reach.get(&server.ip) {
                    Some(multiplayer_page::Reach::Online { players, max, .. }) => Some(trf(
                        "{online} of {max}",
                        &[("online", &players.to_string()), ("max", &max.to_string())],
                    )),
                    Some(multiplayer_page::Reach::Offline) => Some(tr("No answer").into()),
                    _ => None,
                };
                let id = SharedString::from(format!("home-server:{key:?}"));
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
                    window,
                    cx,
                    move |this| {
                        this.ui.home.tabs.server = Some(Target {
                            profile: profile.clone(),
                            key: key.clone(),
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

    pub(crate) fn returning_to_home_server(&self) -> bool {
        self.ui.home.tabs.server.is_some()
    }

    pub(crate) fn clear_home_server_target(&mut self) {
        self.ui.home.tabs.server = None;
    }

    /// Attach inside the saved row in Join. An offline row can pass its own focus handle.
    pub(crate) fn home_server_target(
        &self,
        key: &SavedKey,
        focus: &FocusHandle,
    ) -> Option<impl IntoElement + use<>> {
        let target = self.ui.home.tabs.server.as_ref()?;
        if &target.key != key
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
