//! What opens over the hosted servers on Home's Servers tab: a card's "…" menu (Start or Stop,
//! Open, Open console, Settings, Open folder, Copy address, Rename…, Delete…), the name field
//! Rename… opens and the question before Delete… moves a server to the trash.
use super::*;
use crate::servers_page::state::{HostDialog, HostMenu};
use crate::sheets::{modal_frame, title};
use crate::widgets::button::{danger, primary, secondary};
use crate::widgets::menu::{self, Row};

fn close_host_menu(this: &mut Launcher) {
    this.servers.menu = None;
}

fn close_host_dialog(this: &mut Launcher) {
    this.servers.dialog = None;
}

impl Launcher {
    /// The layer over the page for the open menu or dialog, if any.
    pub(crate) fn host_sheet(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        match self.servers.dialog.as_ref() {
            Some(HostDialog::Rename { focus, .. }) => {
                Some(self.host_rename_sheet(focus, window, cx))
            }
            Some(HostDialog::Delete { dir, name }) => Some(self.host_delete_sheet(dir, name, cx)),
            None => self.host_menu_sheet(self.servers.menu.as_ref()?, cx),
        }
    }

    /// Opens the server in `dir` on its own page, at `tab`.
    pub(super) fn show_server(&mut self, dir: PathBuf, tab: Tab) {
        self.open_server(dir);
        self.servers.tab = tab;
        self.navigate(crate::frame::Page::Hosting);
    }

    /// The "…" menu of the card of the server in `open.dir`.
    fn host_menu_sheet(&self, open: &HostMenu, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let entry = self.servers.list.iter().find(|e| e.dir == open.dir)?;
        let status = self
            .servers
            .statuses
            .get(&open.dir)
            .cloned()
            .unwrap_or_else(Status::stopped);
        let mut rows = self.host_go_rows(&open.dir, &status, cx);
        rows.extend(self.host_manage_rows(entry, &status, cx));
        // Open, Open folder, Rename… and Delete… each start a group.
        let height = menu::height(rows.len(), 4);
        Some(menu::layer(
            "host-menu",
            rows,
            height,
            open.button,
            (self.window_width, self.window_height),
            cx,
            close_host_menu,
        ))
    }

    /// Start or Stop, then the ways into the server's own page.
    fn host_go_rows(&self, dir: &Path, status: &Status, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let running = status.running();
        let idle = self.servers.busy.is_none();
        // Start only for a server the engine reported stopped (see `host_power_button`).
        let off = if running {
            (!idle).then(|| SharedString::from(tr("Wait for the current operation to finish.")))
        } else if idle && status.phase == "stopped" {
            None
        } else if idle {
            Some(SharedString::default())
        } else {
            Some(tr("Wait for the current operation to finish.").into())
        };
        let power = if running {
            Row::new("host-menu-power", "stop", tr("Stop"))
        } else {
            Row::new("host-menu-power", "play", tr("Start"))
        };
        let (power_dir, open_dir, console_dir, settings_dir) = (
            dir.to_path_buf(),
            dir.to_path_buf(),
            dir.to_path_buf(),
            dir.to_path_buf(),
        );
        vec![
            menu::row(
                power.off(off),
                cx.listener(move |this, _, _, cx| {
                    this.servers.menu = None;
                    if running {
                        this.stop_server_in(power_dir.clone());
                    } else {
                        this.start_server_in(power_dir.clone());
                    }
                    cx.notify();
                }),
            ),
            menu::row(
                Row::new("host-menu-open", "arrow-right-01", tr("Open")).divided(),
                cx.listener(move |this, _, _, cx| {
                    this.servers.menu = None;
                    this.show_server(open_dir.clone(), Tab::Overview);
                    cx.notify();
                }),
            ),
            menu::row(
                Row::new(
                    "host-menu-console",
                    "computer-terminal-01",
                    tr("Open console"),
                ),
                cx.listener(move |this, _, _, cx| {
                    this.servers.menu = None;
                    this.show_server(console_dir.clone(), Tab::Overview);
                    cx.notify();
                }),
            ),
            menu::row(
                Row::new("host-menu-settings", "settings-01", tr("Settings")),
                cx.listener(move |this, _, _, cx| {
                    this.servers.menu = None;
                    this.show_server(settings_dir.clone(), Tab::Settings);
                    cx.notify();
                }),
            ),
        ]
    }

    /// Open folder, Copy address, Rename… and Delete….
    fn host_manage_rows(
        &self,
        entry: &Entry,
        status: &Status,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let running = status.running();
        let address = running.then(|| self.share_text(status.listener)).flatten();
        let blocked: Option<SharedString> = if running {
            Some(tr("Stop the server before deleting it.").into())
        } else if self.servers.busy.is_some() {
            Some(tr("Wait for the current operation to finish.").into())
        } else {
            None
        };
        let (folder, renamed, doomed) = (entry.dir.clone(), entry.clone(), entry.clone());
        vec![
            menu::row(
                Row::new("host-menu-folder", "folder-01", tr("Open folder")).divided(),
                cx.listener(move |this, _, _, cx| {
                    this.servers.menu = None;
                    crate::toast::open(cx, &folder);
                    cx.notify();
                }),
            ),
            menu::row(
                Row::new("host-menu-copy", "copy-01", tr("Copy address")).off(
                    address
                        .is_none()
                        .then(|| tr("Start the server to get its address.").into()),
                ),
                cx.listener(move |this, _, _, cx| {
                    this.servers.menu = None;
                    if let Some(address) = &address {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(address.clone()));
                        crate::toast::say(trf("Copied {address}", &[("address", address)]));
                    }
                    cx.notify();
                }),
            ),
            menu::row(
                Row::new("host-menu-rename", "pencil-edit-02", tr("Rename…")).divided(),
                cx.listener(move |this, _, window, cx| {
                    this.start_host_rename(renamed.clone(), window, cx);
                    cx.notify();
                }),
            ),
            menu::row(
                Row::new("host-menu-delete", "delete-02", tr("Delete…"))
                    .danger()
                    .off(blocked),
                cx.listener(move |this, _, _, cx| {
                    this.servers.menu = None;
                    this.servers.dialog = Some(HostDialog::Delete {
                        dir: doomed.dir.clone(),
                        name: doomed.name.clone(),
                    });
                    cx.notify();
                }),
            ),
        ]
    }

    /// Opens the name field for `entry`, with its name in it, selected.
    fn start_host_rename(&mut self, entry: Entry, window: &mut Window, cx: &mut Context<Self>) {
        let focus = cx.focus_handle().tab_index(0).tab_stop(true);
        self.servers.rename.set(entry.name);
        self.servers.rename.select_all();
        window.focus(&focus, cx);
        self.servers.menu = None;
        self.servers.dialog = Some(HostDialog::Rename {
            dir: entry.dir,
            focus,
        });
    }

    /// Gives the server the typed name, in the list (its folder keeps its own).
    fn apply_host_rename(&mut self) {
        let Some(HostDialog::Rename { dir, .. }) = self.servers.dialog.take() else {
            return;
        };
        let name = self.servers.rename.text().trim().to_string();
        if name.is_empty() {
            return;
        }
        match servers::rename(&dir, &name) {
            Ok(()) => {
                self.servers.list = servers::list();
                self.side_servers = self.servers.list.clone();
            }
            Err(e) => self.servers.problem = Some(Problem::plain(e)),
        }
    }

    fn host_rename_sheet(
        &self,
        focus: &FocusHandle,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let focused = focus.is_focused(window);
        let field = crate::widgets::field::field_box(focused, false)
            .id("host-rename-field")
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
                &self.servers.rename,
                &self.theme,
                focused,
                false,
                false,
                "Server name",
            ))
            .on_click({
                let focus = focus.clone();
                cx.listener(move |_, _: &ClickEvent, window, cx| {
                    window.focus(&focus, cx);
                    cx.notify();
                })
            })
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                match line_edit::handle(&mut this.servers.rename, ev, cx) {
                    Key::Pass => return,
                    Key::Enter => this.apply_host_rename(),
                    Key::Escape => this.servers.dialog = None,
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        let field = line_edit::mouse(field, cx, |this: &mut Launcher| &mut this.servers.rename);
        let body = div()
            .flex()
            .flex_col()
            .child(title(tr("Rename server").to_string()))
            .child(div().flex().child(field))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        secondary("host-rename-cancel", tr("Cancel")).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.servers.dialog = None;
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        primary("host-rename-confirm", tr("Rename")).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.apply_host_rename();
                                cx.notify();
                            },
                        )),
                    ),
            );
        modal_frame("host-rename-sheet", 450., body, cx, close_host_dialog)
    }

    /// The question before the server in `dir` moves to the trash, with its worlds, settings
    /// and backups.
    fn host_delete_sheet(&self, dir: &Path, name: &str, cx: &mut Context<Self>) -> Stateful<Div> {
        let dir = dir.to_path_buf();
        let body = div()
            .flex()
            .flex_col()
            .child(title(trf("Delete {name}?", &[("name", name)])))
            .child(div().text_color(rgb(palette().muted)).child(tr(
                "Remove this server from Sunrise and move its worlds, settings and backups to the trash.",
            )))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(secondary("host-delete-cancel", tr("Cancel")).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.servers.dialog = None;
                            cx.notify();
                        }),
                    ))
                    .child(danger("host-delete-confirm", tr("Move to trash")).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.servers.dialog = None;
                            this.servers.busy =
                                Some(crate::i18n::tr_key("Moving server to trash").into());
                            let (tx, dir) = (this.servers.tx.clone(), dir.clone());
                            std::thread::spawn(move || {
                                let result = servers::delete(&dir);
                                let _ = tx.send(Event::Deleted { dir, result });
                            });
                            cx.notify();
                        }),
                    )),
            );
        modal_frame("host-delete-sheet", 450., body, cx, close_host_dialog)
    }
}
