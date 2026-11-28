//! The hosted servers on Home's Servers tab: each as a card built as a saved server's and a
//! world's are (`join::cards`): a picture, its name over a status line (Survival · Stopped, or
//! Running with its address and how many play), a Start or Stop icon button and a "…" menu
//! (`menu.rs`). Pressing a card opens the server's own page.
use super::*;
use crate::join::{more_button, no_tick, section, server_row, RowParts};
use crate::widgets::button::{button, icon_button, icon_only, Look, Size, Style};
use crate::widgets::label::name_over;

const ICON: f32 = 40.;

impl Launcher {
    /// Loads the list of hosted servers the first time it is wanted.
    pub(crate) fn load_hosted(&mut self) {
        if self.servers.loaded {
            return;
        }
        self.servers.list = servers::list();
        self.servers.modes.clear();
        self.servers.loaded = true;
    }

    /// How many servers this computer hosts.
    pub(crate) fn hosted_count(&mut self) -> usize {
        self.load_hosted();
        self.servers.list.len()
    }

    /// Reads every hosted server's status again at the next tick.
    pub(crate) fn refresh_hosted(&mut self) {
        self.servers.polled = None;
    }

    /// Opens the Create a server dialog.
    pub(crate) fn open_host_create(&mut self, cx: &mut Context<Self>) {
        self.close_menus();
        self.open_create(cx);
    }

    /// The "Hosted on this computer" group of the Servers tab: a card for each server `query`
    /// (lower case, as the tab's search holds it) finds, or, with nothing hosted yet, one quiet
    /// line with a Create button. None while a search finds none.
    pub(crate) fn hosted_section(&mut self, query: &str, cx: &mut Context<Self>) -> Option<Div> {
        self.load_hosted();
        let problem = self
            .host_problem(cx)
            .filter(|_| self.servers.create.is_none());
        let body = div().flex().flex_col().gap(px(8.)).children(problem);
        if self.servers.list.is_empty() {
            if !query.is_empty() {
                return None;
            }
            let create = button(
                "home-host-create",
                tr("Create"),
                Look::new(Style::Secondary, Size::Medium),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.open_host_create(cx);
                cx.notify();
            }));
            let hint = div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(crate::home::muted(tr(
                    "Host a world for friends on this computer.",
                )))
                .child(create);
            return Some(section(tr("Hosted on this computer"), body.child(hint)));
        }
        let mut cards = div().flex().flex_col().gap(px(crate::home::tabs::GAP));
        let mut shown = 0;
        for (i, entry) in self.servers.list.clone().into_iter().enumerate() {
            let name = entry.name.to_lowercase();
            if !query.split_whitespace().all(|word| name.contains(word)) {
                continue;
            }
            shown += 1;
            cards = cards.child(self.host_card(i, entry.dir, entry.name, cx));
        }
        if shown == 0 {
            return None;
        }
        Some(section(tr("Hosted on this computer"), body.child(cards)))
    }

    /// What a server's line under its name says, and in which ink: its mode and state, and while
    /// it runs its address and how many play. The state is the job's while the list is
    /// starting or stopping it.
    fn host_status_line(&mut self, dir: &Path, status: &Status) -> (String, u32) {
        let p = palette();
        let working = self.servers.job_dir.as_deref() == Some(dir);
        let job = self
            .servers
            .busy
            .clone()
            .filter(|_| working)
            .map(|doing| format!("{}…", tr(&doing)));
        let moving = job.is_some()
            || matches!(
                status.phase.as_str(),
                "starting" | "restarting" | "stopping"
            );
        let state = job.unwrap_or_else(|| phase_label(status));
        if status.running() && !moving {
            let mut parts = vec![state];
            parts.extend(self.share_text(status.listener));
            parts.push(trf(
                "{online} of {max} playing",
                &[
                    ("online", &status.players.len().to_string()),
                    ("max", &status.max_players.unwrap_or(0).to_string()),
                ],
            ));
            return (parts.join(" · "), p.success);
        }
        let line = match self.servers.mode_label(dir) {
            Some(mode) => trf("{mode} · {state}", &[("mode", &mode), ("state", &state)]),
            None => state,
        };
        (line, if moving { p.warning } else { p.muted })
    }

    /// Start or Stop of the card at `i`, an icon button as a world's Play is. Not allowed, it is
    /// dimmed, with the reason as its tooltip.
    fn host_power_button(
        &self,
        i: usize,
        dir: &Path,
        status: &Status,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let idle = self.servers.busy.is_none();
        let running = status.running();
        // Start only for a server the engine reported stopped: an unknown status may be a server
        // started elsewhere, which Start would claim as this launcher's.
        let (icon, label, enabled) = if running {
            ("stop", tr("Stop"), idle)
        } else {
            ("play", tr("Start"), idle && status.phase == "stopped")
        };
        let id = format!("host-card-power-{i}");
        if !enabled {
            let why = if idle {
                label
            } else {
                tr("Wait for the current operation to finish.")
            };
            return icon_only(id, Look::new(Style::Secondary, Size::Medium).icon(icon))
                .opacity(0.4)
                .cursor_default()
                .tab_stop(false)
                .tooltip(crate::glass_ui::tip(why));
        }
        let dir = dir.to_path_buf();
        icon_button(id, icon, label, Style::Secondary).on_click(cx.listener(
            move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                if running {
                    this.stop_server_in(dir.clone());
                } else {
                    this.start_server_in(dir.clone());
                }
                cx.notify();
            },
        ))
    }

    /// One server as a card. Start and Stop act on the server they sit beside, and its card
    /// says how it goes; pressing the card opens the server's own page.
    fn host_card(
        &mut self,
        i: usize,
        dir: PathBuf,
        name: String,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let status = self
            .servers
            .statuses
            .get(&dir)
            .cloned()
            .unwrap_or_else(Status::stopped);
        let (line, ink) = self.host_status_line(&dir, &status);
        let power = self.host_power_button(i, &dir, &status, cx);
        let open = self
            .servers
            .menu
            .as_ref()
            .is_some_and(|menu| menu.dir == dir);
        let menu_dir = dir.clone();
        let more = more_button(
            format!("host-card-{i}-more"),
            open,
            cx,
            move |this, button| {
                this.servers.menu = Some(crate::servers_page::state::HostMenu {
                    dir: menu_dir.clone(),
                    button,
                });
            },
        );
        let picture = crate::mod_icons::tile(
            None::<std::sync::Arc<gpui::Image>>,
            &dir.to_string_lossy(),
            &name,
            ICON,
        );
        server_row(
            SharedString::from(format!("host-card-{i}")),
            RowParts {
                check: no_tick(),
                picture: picture.into_any_element(),
                name: name_over(name, tokens::LABEL, Some((line.into(), ink))),
                note: None,
                buttons: vec![power.into_any_element(), more.into_any_element()],
            },
        )
        .reach()
        .cursor_pointer()
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.show_server(dir.clone(), Tab::Overview);
            cx.notify();
        }))
    }
}
