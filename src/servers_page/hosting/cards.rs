//! The Host page's list: each server as a card, as Home's tabs draw theirs: a picture, its name
//! over a status line (Survival · Stopped, or Running with its address and how many play),
//! Start or Stop, Open, and a "…" menu (`menu.rs`). The tab's actions are Create server and
//! Add folder….
use super::*;
use crate::widgets::button::{button, icon_button_opening, secondary, Look, Size, Style};
use crate::widgets::feedback::empty_state_icon;
use std::{cell::Cell, rc::Rc};

const ROW: f32 = 52.;
const ICON: f32 = 40.;

impl Launcher {
    /// The page's list: a problem to show, then a card for each server, or the empty state.
    pub(super) fn host_list(&mut self, cx: &mut Context<Self>) -> Div {
        let problem = self
            .host_problem(cx)
            .filter(|_| self.servers.create.is_none());
        if self.servers.list.is_empty() {
            return self.host_empty(problem, cx);
        }
        let mut cards = div().flex().flex_col().gap(px(crate::home::tabs::GAP));
        for (i, entry) in self.servers.list.clone().into_iter().enumerate() {
            cards = cards.child(self.host_card(i, entry.dir, entry.name, cx));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .children(problem)
            .child(cards)
    }

    /// Add folder… and Create server, as the page's header and the empty state show them.
    fn host_create_buttons(&mut self, cx: &mut Context<Self>) -> (Stateful<Div>, Stateful<Div>) {
        let creating = self.servers.creating;
        let add = self.host_button(
            "host-add",
            tr("Add folder…"),
            !creating,
            cx,
            |this, cx| this.add_server_folder(cx),
        );
        let create = button(
            "host-create",
            tr("Create server"),
            Look::new(Style::Primary, Size::Medium).icon("plus-sign"),
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.open_create(cx);
            cx.notify();
        }));
        (add, create)
    }

    /// The page header's actions: none while a server's own page is open, nor in the empty
    /// state, which carries them itself.
    pub(crate) fn host_actions(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.servers.open.is_some() || self.servers.list.is_empty() {
            return None;
        }
        let (add, create) = self.host_create_buttons(cx);
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(add)
                .child(create)
                .into_any_element(),
        )
    }

    /// No server yet: the empty state carries the two actions.
    fn host_empty(&mut self, problem: Option<Div>, cx: &mut Context<Self>) -> Div {
        let (add, create) = self.host_create_buttons(cx);
        let actions = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(create)
            .child(add)
            .into_any_element();
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .children(problem)
            .child(empty_state_icon(
                Some("server-stack"),
                tr("No server yet.").trim_end_matches('.'),
                tr("Host a world for your friends on this computer. Create a server, press Start, and give them the address."),
                Some(actions),
            ))
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

    /// A card's text button: `action` on a press (the card under it doesn't take the press).
    /// Not `enabled`, it is dimmed, with `why` as its tooltip when there is one.
    fn host_card_button(
        &self,
        (id, label): (String, SharedString),
        (enabled, why): (bool, Option<&'static str>),
        cx: &mut Context<Self>,
        action: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        let button = secondary(id, label);
        if !enabled {
            let button = button.opacity(0.4).cursor_default().tab_stop(false);
            return match why {
                Some(why) => button.tooltip(crate::glass_ui::tip(tr(why))),
                None => button,
            };
        }
        button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            cx.stop_propagation();
            action(this, cx);
            cx.notify();
        }))
    }

    /// The "…" button of the card of `dir`: opens its menu, hung from where it was drawn.
    fn host_card_more(&self, i: usize, dir: &Path, cx: &mut Context<Self>) -> Stateful<Div> {
        let drawn = Rc::new(Cell::new(gpui::Bounds::<gpui::Pixels>::default()));
        let (write, read) = (drawn.clone(), drawn);
        let open = self
            .servers
            .menu
            .as_ref()
            .is_some_and(|menu| menu.dir == dir);
        let dir = dir.to_path_buf();
        icon_button_opening(
            ElementId::named_usize("host-card-more", i),
            "more-horizontal",
            tr("More actions"),
            Style::Ghost,
            open,
        )
        // A step brighter than the card's own light, so it shows on a lit card.
        .hover(|s| s.bg(rgb(palette().selected)))
        .child(
            gpui::canvas(move |bounds, _, _| write.set(bounds), |_, _, _, _| {})
                .absolute()
                .inset_0(),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            cx.stop_propagation();
            this.close_menus();
            this.servers.menu = Some(crate::servers_page::state::HostMenu {
                dir: dir.clone(),
                button: read.get(),
            });
            cx.notify();
        }))
    }

    /// One server as a card. Start and Stop act on the server they sit beside, and its card
    /// says how it goes; pressing the card, or Open, opens the server's own page.
    fn host_card(
        &mut self,
        i: usize,
        dir: PathBuf,
        name: String,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let idle = self.servers.busy.is_none();
        let status = self
            .servers
            .statuses
            .get(&dir)
            .cloned()
            .unwrap_or_else(Status::stopped);
        let (line, ink) = self.host_status_line(&dir, &status);
        let wait = (!idle).then_some("Wait for the current operation to finish.");
        let (start_dir, stop_dir, open_dir, card_dir) =
            (dir.clone(), dir.clone(), dir.clone(), dir.clone());
        let power = if status.running() {
            self.host_card_button(
                (format!("host-card-stop-{i}"), tr("Stop").into()),
                (idle, wait),
                cx,
                move |this, _| this.stop_server_in(stop_dir.clone()),
            )
        } else {
            // Only a server the engine reported stopped: an unknown status may be a server
            // started elsewhere, which Start would claim as this launcher's.
            self.host_card_button(
                (format!("host-card-start-{i}"), tr("Start").into()),
                (idle && status.phase == "stopped", wait),
                cx,
                move |this, _| this.start_server_in(start_dir.clone()),
            )
        };
        let open = self.host_card_button(
            (format!("host-card-open-{i}"), tr("Open").into()),
            (true, None),
            cx,
            move |this, _| this.open_server(open_dir.clone()),
        );
        let more = self.host_card_more(i, &dir, cx);
        let picture = crate::mod_icons::tile(
            None::<std::sync::Arc<gpui::Image>>,
            &dir.to_string_lossy(),
            &name,
            ICON,
        );
        crate::home::card()
            .id(ElementId::named_usize("host-card", i))
            .reach()
            .cursor_pointer()
            .flex_none()
            .h(px(ROW))
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(12.))
            .hover(|s| s.bg(rgb(palette().hover)))
            .child(div().flex_none().size(px(ICON)).child(picture))
            .child(crate::widgets::label::name_over(
                name,
                tokens::LABEL,
                Some((line.into(), ink)),
            ))
            .child(power)
            .child(open)
            .child(more)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.open_server(card_dir.clone());
                cx.notify();
            }))
    }
}
