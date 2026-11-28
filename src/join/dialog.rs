//! What opens over Home's Servers tab: a card's "…" menu, the form that adds or edits a server
//! (and joins one without saving it: quick join), and the question before a removal. The hosted
//! servers' own menu and dialogs are `servers_page::hosting::menu`.
use super::*;
use crate::frame::{Page, Reach as _};
use crate::line_edit::{self, Key, LineEdit};
use crate::multiplayer_page::{Reach as Answer, ServerDialog, ServerMenu};
use crate::paint::palette;
use crate::sheets::{modal_frame, title};
use crate::widgets::button::{danger, primary, secondary};
use crate::widgets::menu::{self, Row};

fn name_edit(this: &mut Launcher) -> &mut LineEdit {
    &mut this.multiplayer.name
}

fn address_edit(this: &mut Launcher) -> &mut LineEdit {
    &mut this.friend_code
}

fn close_server_menu(this: &mut Launcher) {
    this.multiplayer.menu = None;
}

/// A button that does nothing now: dimmed, and out of the Tab order.
fn dimmed(button: Stateful<Div>) -> Stateful<Div> {
    button.opacity(0.4).cursor_default().tab_stop(false)
}

impl Launcher {
    /// The layer over the page for what is open on Home's Servers tab: a hosted server's menu or
    /// dialog, or a saved or featured one's.
    pub(crate) fn server_action_sheet(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        match self.ui.page {
            Page::Home if self.home_on_servers() => self
                .host_sheet(window, cx)
                .or_else(|| self.saved_servers_sheet(window, cx)),
            _ => None,
        }
    }

    fn saved_servers_sheet(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        match self.multiplayer.dialog.as_ref() {
            Some(ServerDialog::Form { edit }) => Some(self.server_form_sheet(*edit, window, cx)),
            Some(ServerDialog::Remove(servers)) => Some(self.server_remove_sheet(servers, cx)),
            None => self.server_menu_sheet(self.multiplayer.menu.as_ref()?, cx),
        }
    }

    /// Opens the form of a new server, or of the saved server at `edit`. The address field has
    /// the keyboard: a pasted address or invite code is one Join away.
    pub(crate) fn open_server_form(
        &mut self,
        edit: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_menus();
        let (name, address) = match edit.and_then(|place| self.multiplayer.list.get(place)) {
            Some((server, _)) => (server.name.clone(), server.ip.clone()),
            None => (String::new(), String::new()),
        };
        self.multiplayer.name.set(name);
        self.friend_code.set(address);
        self.multiplayer.error = None;
        self.multiplayer
            .name_focus
            .get_or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true));
        let focus = self.friend_focus.clone();
        window.focus(&focus, cx);
        self.multiplayer.dialog = Some(ServerDialog::Form { edit });
    }

    /// Join with what is typed: an invite code joins the friend, an address the server.
    pub(crate) fn join_typed(&mut self) {
        let text = self.friend_code.text().to_string();
        match entry(&text) {
            Entry::Invite => {
                self.close_server_dialog();
                self.play_game(launcher_play::PlayDestination::Invite(text));
            }
            Entry::Address(address) => {
                self.close_server_dialog();
                self.play_to("--server", address);
            }
            Entry::Empty | Entry::Unusable => {}
        }
    }

    /// Save with what is typed: the address goes into the game's own server list, as a new
    /// server or as the one being edited.
    fn save_server_form(&mut self) {
        let edit = match &self.multiplayer.dialog {
            Some(ServerDialog::Form { edit }) => *edit,
            _ => return,
        };
        let Entry::Address(address) = entry(self.friend_code.text()) else {
            return;
        };
        let name = self.multiplayer.name.text().trim().to_string();
        let name = if name.is_empty() {
            address.clone()
        } else {
            name
        };
        match edit {
            Some(place) => self.edit_servers(|list| list.edit(place, &name, &address)),
            None => self.edit_servers(|list| list.add(&name, &address)),
        }
        if self.multiplayer.error.is_none() {
            self.multiplayer.dialog = None;
            self.friend_code.set(String::new());
            self.multiplayer.name.set(String::new());
        }
    }

    /// Enter in either field: Save for an address, and join for an invite code (a new server's
    /// form only).
    fn submit_server_form(&mut self) {
        let editing = matches!(
            self.multiplayer.dialog,
            Some(ServerDialog::Form { edit: Some(_) })
        );
        match entry(self.friend_code.text()) {
            Entry::Address(_) => self.save_server_form(),
            Entry::Invite if !editing => self.join_typed(),
            _ => {}
        }
    }

    /// One text field of the form, in the plain field frame.
    fn form_field(
        &self,
        (id, placeholder): (&'static str, &'static str),
        (focus, edit): (&FocusHandle, fn(&mut Launcher) -> &mut LineEdit),
        text: &LineEdit,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let focused = focus.is_focused(window);
        let own = focus.clone();
        let field = crate::widgets::field::field_box(focused, false)
            .id(id)
            .track_focus(focus)
            .seen()
            .flex_1()
            .min_w(px(0.))
            .h(px(tokens::FIELD))
            .px(px(12.))
            .text_size(px(tokens::BODY))
            .text_color(rgb(palette().text))
            .cursor_text()
            .child(line_edit::content(
                text,
                &self.theme,
                focused,
                false,
                false,
                placeholder,
            ))
            .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
                window.focus(&own, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
                match line_edit::handle(edit(this), ev, cx) {
                    Key::Pass => return,
                    Key::Enter => this.submit_server_form(),
                    Key::Escape => this.close_server_dialog(),
                    Key::Edited => this.multiplayer.error = None,
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        div().flex().child(line_edit::mouse(field, cx, edit))
    }

    /// The form: a name and an address (or an invite code), Save, and Join now for a one-off
    /// join of a new server.
    fn server_form_sheet(
        &self,
        edit: Option<usize>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let typed = entry(self.friend_code.text());
        let saves = matches!(typed, Entry::Address(_));
        let joins = matches!(typed, Entry::Address(_) | Entry::Invite);
        let hint = match (&self.multiplayer.error, &typed) {
            (Some(error), _) => Some((error.clone(), palette().warning)),
            (None, Entry::Unusable) => Some((
                tr("That isn't an address or an invite code.").to_string(),
                palette().muted,
            )),
            (None, Entry::Invite) => Some((
                tr("An invite code joins your friend. It isn't saved.").to_string(),
                palette().muted,
            )),
            _ => None,
        };
        let name_focus = self.multiplayer.name_focus.clone();
        let name = name_focus.as_ref().map(|focus| {
            self.form_field(
                ("server-name", "Server name"),
                (focus, name_edit),
                &self.multiplayer.name,
                window,
                cx,
            )
        });
        let address = self.form_field(
            ("server-address", "Server address or a friend's invite code"),
            (&self.friend_focus, address_edit),
            &self.friend_code,
            window,
            cx,
        );
        let save = primary("server-form-save", tr("Save"));
        let save = if saves {
            save.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.save_server_form();
                cx.notify();
            }))
        } else {
            dimmed(save)
        };
        let cancel = secondary("server-form-cancel", tr("Cancel")).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.close_server_dialog();
                cx.notify();
            },
        ));
        // A one-off join belongs to a new server: an edit has nothing to join.
        let join_now = edit.is_none().then(|| {
            let join = secondary("server-form-join", tr("Join now"));
            if joins {
                join.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.join_typed();
                    cx.notify();
                }))
            } else {
                dimmed(join)
            }
        });
        let heading = if edit.is_some() {
            tr("Edit server")
        } else {
            tr("Add server")
        };
        let body = div()
            .flex()
            .flex_col()
            .child(title(heading.to_string()))
            .children(name)
            .child(address)
            .children(hint.map(|(text, ink)| {
                div()
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(ink))
                    .child(text)
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .child(div().children(join_now))
                    .child(div().flex().gap(px(8.)).child(cancel).child(save)),
            );
        modal_frame("server-form", 450., body, cx, Self::close_server_dialog)
    }

    /// The question before `servers` (place and name) leave the list.
    fn server_remove_sheet(
        &self,
        servers: &[(usize, String)],
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let places: Vec<usize> = servers.iter().map(|(place, _)| *place).collect();
        let (heading, line) = match servers {
            [(_, name)] => (
                trf("Remove {name}?", &[("name", name)]),
                tr("It leaves this profile's server list.").to_string(),
            ),
            _ => (
                trf(
                    "Remove {count} servers?",
                    &[("count", &servers.len().to_string())],
                ),
                tr("They leave this profile's server list.").to_string(),
            ),
        };
        let body =
            div()
                .flex()
                .flex_col()
                .child(title(heading))
                .child(div().text_color(rgb(palette().muted)).child(line))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(secondary("server-remove-cancel", tr("Cancel")).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.close_server_dialog();
                                cx.notify();
                            }),
                        ))
                        .child(danger("server-remove-confirm", tr("Remove")).on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.multiplayer.dialog = None;
                                this.remove_servers(places.clone());
                                cx.notify();
                            }),
                        )),
                );
        modal_frame("server-remove", 450., body, cx, Self::close_server_dialog)
    }

    /// The "…" menu of a saved server (Join, Edit, Copy address, Refresh status, Move up and
    /// down, Remove) or of a featured one (Join, Copy address, Refresh
    /// status), its groups set apart by hairlines.
    fn server_menu_sheet(
        &self,
        open: &ServerMenu,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        let (rows, hairlines) = if open.featured {
            self.featured_menu_rows(open.index, cx)?
        } else {
            self.saved_menu_rows(open.index, cx)?
        };
        let height = menu::height(rows.len(), hairlines);
        Some(menu::layer(
            "server-menu",
            rows,
            height,
            open.button,
            (self.window_width, self.window_height),
            cx,
            close_server_menu,
        ))
    }

    /// The rows of a featured server's menu, and how many hairlines they hold.
    fn featured_menu_rows(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Option<(Vec<AnyElement>, usize)> {
        let server = featured().into_iter().nth(index)?;
        let address = server.address?;
        // A featured server stays under Featured: there is no adding it to the saved list.
        let rows = vec![
            self.join_row(address.to_string(), cx),
            self.copy_address_row(address.to_string(), true, cx),
            self.refresh_row(address.to_string(), false, cx),
        ];
        Some((rows, 1))
    }

    /// The rows of the saved server's menu at `index`, and how many hairlines they hold.
    fn saved_menu_rows(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Option<(Vec<AnyElement>, usize)> {
        let (server, _) = self.multiplayer.list.get(index)?;
        let name = server.name.clone();
        // Move up and down step through the listed servers, never past a featured one's entry.
        let places = self.listed_places();
        let at = places.iter().position(|place| *place == index)?;
        let up = at.checked_sub(1).map(|n| places[n]);
        let down = places.get(at + 1).copied();
        let rows = vec![
            self.join_row(server.ip.clone(), cx),
            menu::row(
                Row::new("server-menu-edit", "pencil-edit-02", tr("Edit…")).divided(),
                cx.listener(move |this, _, window, cx| {
                    this.open_server_form(Some(index), window, cx);
                    cx.notify();
                }),
            ),
            self.copy_address_row(server.ip.clone(), false, cx),
            self.refresh_row(server.ip.clone(), false, cx),
            menu::row(
                Row::new("server-menu-up", "arrow-up-01", tr("Move up"))
                    .divided()
                    .off(up.is_none().then(SharedString::default)),
                cx.listener(move |this, _, _, cx| {
                    this.multiplayer.menu = None;
                    if let Some(other) = up {
                        this.edit_servers(|list| list.swap(index, other));
                    }
                    cx.notify();
                }),
            ),
            menu::row(
                Row::new("server-menu-down", "arrow-down-01", tr("Move down"))
                    .off(down.is_none().then(SharedString::default)),
                cx.listener(move |this, _, _, cx| {
                    this.multiplayer.menu = None;
                    if let Some(other) = down {
                        this.edit_servers(|list| list.swap(index, other));
                    }
                    cx.notify();
                }),
            ),
            menu::row(
                Row::new("server-menu-remove", "delete-02", tr("Remove…")).danger(),
                cx.listener(move |this, _, _, cx| {
                    this.multiplayer.menu = None;
                    this.multiplayer.dialog =
                        Some(ServerDialog::Remove(vec![(index, name.clone())]));
                    cx.notify();
                }),
            ),
        ];
        Some((rows, 3))
    }

    /// "Join": off with the reason while the server doesn't answer.
    fn join_row(&self, address: String, cx: &mut Context<Self>) -> AnyElement {
        let offline = self.multiplayer.reach.get(&address).copied() == Some(Answer::Offline);
        menu::row(
            Row::new("server-menu-join", "play", tr("Join")).off(
                offline.then(|| SharedString::from(tr("This server doesn't answer right now"))),
            ),
            cx.listener(move |this, _, _, cx| {
                this.multiplayer.menu = None;
                this.play_to("--server", address.clone());
                cx.notify();
            }),
        )
    }

    /// "Refresh status": asks the server for its players again.
    fn refresh_row(&self, address: String, apart: bool, cx: &mut Context<Self>) -> AnyElement {
        let row = Row::new("server-menu-refresh", "refresh", tr("Refresh status"));
        menu::row(
            if apart { row.divided() } else { row },
            cx.listener(move |this, _, _, cx| {
                this.multiplayer.menu = None;
                this.refresh_server(&address, cx);
                cx.notify();
            }),
        )
    }

    /// "Copy address", which says it did in a toast: the menu closes and nothing else would.
    fn copy_address_row(&self, address: String, apart: bool, cx: &mut Context<Self>) -> AnyElement {
        let row = Row::new("server-menu-copy", "copy-01", tr("Copy address"));
        menu::row(
            if apart { row.divided() } else { row },
            cx.listener(move |this, _, _, cx| {
                this.multiplayer.menu = None;
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(address.clone()));
                crate::toast::say(trf("Copied {address}", &[("address", &address)]));
                cx.notify();
            }),
        )
    }
}
