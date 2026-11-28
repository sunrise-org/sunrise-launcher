//! The cards of Home's Servers tab: a saved server's (tick box, picture, name over how many
//! are playing, its answer time, Join and a "…" menu) and a featured server's (the same, but for
//! the tick box), and the bar under the list while servers are ticked.
use super::*;
use crate::mods::selection::{bar, Gesture};
use crate::multiplayer_page::{Reach, ServerMenu};
use crate::paint::palette;
use crate::widgets::button::{icon_button, icon_button_opening, icon_only, Look, Size, Style};
use std::{cell::Cell, rc::Rc, sync::Arc};
use sunrise_launcher::server_list::Server;

/// A card's height and the picture in it: a world's, so the tabs line up.
pub(crate) const ROW: f32 = 52.;
const ICON: f32 = 40.;
/// As wide as the tick box, so a card without one lines up with the rest.
const CHECK_SLOT: f32 = 18.;

/// What a server's line under its name says, and in which ink: the players while it answers, that
/// it doesn't, and `asking` (its address, or a note) until it has.
fn reach_line(reach: Option<Reach>, asking: String) -> (SharedString, u32) {
    match reach {
        Some(Reach::Online { players, max }) => (
            trf(
                "{online} of {max} playing",
                &[("online", &players.to_string()), ("max", &max.to_string())],
            )
            .into(),
            palette().success,
        ),
        Some(Reach::Offline) => (tr("No answer").to_string().into(), palette().muted),
        Some(Reach::Asking) | None => (asking.into(), palette().muted),
    }
}

/// Join, the Play button of a world's row: off with the reason as its tooltip while the server
/// doesn't answer.
fn join_button(
    id: String,
    address: String,
    offline: bool,
    cx: &mut Context<Launcher>,
) -> Stateful<Div> {
    if offline {
        return icon_only(id, Look::new(Style::Secondary, Size::Medium).icon("play"))
            .opacity(0.4)
            .cursor_default()
            .tab_stop(false)
            .tooltip(crate::glass_ui::tip(tr(
                "This server doesn't answer right now",
            )));
    }
    icon_button(id, "play", tr("Join"), Style::Secondary).on_click(cx.listener(
        move |this, _: &ClickEvent, _, cx| {
            cx.stop_propagation();
            this.play_to("--server", address.clone());
            cx.notify();
        },
    ))
}

/// The "…" button of a card, the one every list card has: ghost, a step brighter under the
/// pointer than the card's own light. A press closes the open menus and calls `opened` with where
/// the button was drawn (window coordinates), for the menu that hangs from it. `open`: that menu
/// is up, so the tooltip stays away.
pub(crate) fn more_button(
    id: impl Into<SharedString>,
    open: bool,
    cx: &mut Context<Launcher>,
    opened: impl Fn(&mut Launcher, gpui::Bounds<gpui::Pixels>) + 'static,
) -> Stateful<Div> {
    let id: SharedString = id.into();
    let drawn = Rc::new(Cell::new(gpui::Bounds::<gpui::Pixels>::default()));
    let (write, read) = (drawn.clone(), drawn);
    icon_button_opening(
        id,
        "more-horizontal",
        tr("More actions"),
        Style::Ghost,
        open,
    )
    .hover(|s| s.bg(rgb(palette().selected)))
    .child(
        gpui::canvas(move |bounds, _, _| write.set(bounds), |_, _, _, _| {})
            .absolute()
            .inset_0(),
    )
    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
        cx.stop_propagation();
        this.close_menus();
        opened(this, read.get());
        cx.notify();
    }))
}

/// What a server card is made of, left to right.
pub(crate) struct RowParts {
    /// The tick box's column, or [`no_tick`].
    pub(crate) check: AnyElement,
    pub(crate) picture: AnyElement,
    /// [`crate::widgets::label::name_over`]: the name over its line.
    pub(crate) name: Div,
    /// A muted note after the name, such as how long the server took to answer.
    pub(crate) note: Option<Div>,
    /// Join or Start and Stop, then the "…" button, in a world row's spacing.
    pub(crate) buttons: Vec<AnyElement>,
}

/// An empty tick column, for a card that can't be ticked, so it lines up with those that can.
pub(crate) fn no_tick() -> AnyElement {
    div().flex_none().w(px(CHECK_SLOT)).into_any_element()
}

/// A server card, built as a world's row is: its height, padding, gaps and hover light.
pub(crate) fn server_row(id: SharedString, parts: RowParts) -> Stateful<Div> {
    crate::home::card()
        .id(id)
        .flex_none()
        .h(px(ROW))
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(12.))
        .hover(|s| s.bg(rgb(palette().hover)))
        .child(parts.check)
        .child(div().flex_none().size(px(ICON)).child(parts.picture))
        .child(parts.name)
        .children(parts.note)
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .children(parts.buttons),
        )
}

/// A group of cards under a small muted heading, as the sidebar heads its groups.
pub(crate) fn section(heading: impl Into<SharedString>, body: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .px(px(2.))
                .text_size(px(tokens::SMALL))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(palette().muted))
                .child(heading.into()),
        )
        .child(body)
}

impl Launcher {
    /// The "…" button of a card: opens the menu of the server at `index` (a featured one, with
    /// `featured`), hung from where it was drawn.
    fn card_menu_button(
        &self,
        id: &SharedString,
        (index, featured): (usize, bool),
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let open = self
            .multiplayer
            .menu
            .as_ref()
            .is_some_and(|menu| menu.index == index && menu.featured == featured);
        more_button(format!("{id}-more"), open, cx, move |this, button| {
            this.multiplayer.menu = Some(ServerMenu {
                index,
                featured,
                button,
            });
        })
    }

    /// The saved server at `place` as a card. Ctrl or Shift on it ticks it (a plain press does
    /// once any is ticked); Join joins it; the "…" menu does the rest.
    pub(crate) fn saved_server_card(
        &mut self,
        place: usize,
        (server, icon): (Server, Option<Arc<gpui::Image>>),
        order: &Rc<Vec<String>>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id: SharedString = format!("join-server:{place}").into();
        let key = place.to_string();
        let ticked = self.multiplayer.picks.contains(&key);
        let any = !self.multiplayer.picks.is_empty();
        let check = self.tick_column(&id, ticked, any, {
            let (key, order) = (key.clone(), order.clone());
            cx.listener(move |this, ev: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.multiplayer
                    .picks
                    .click(&order, &key, ev.modifiers().shift);
                cx.notify();
            })
        });
        let reach = self.multiplayer.reach.get(&server.ip).copied();
        let offline = reach == Some(Reach::Offline);
        let (words, ink) = reach_line(reach, server.ip.clone());
        let answer = self
            .multiplayer
            .latency
            .get(&server.ip)
            .filter(|_| matches!(reach, Some(Reach::Online { .. })))
            .map(|ms| {
                div()
                    .flex_none()
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(palette().muted))
                    .child(trf("{ms} ms", &[("ms", &ms.to_string())]))
            });
        // The picture the game saved, else the one the server just sent.
        let picture = server_picture(
            icon.or_else(|| self.multiplayer.icons.get(&server.ip).cloned()),
            featured_at(&server.ip).map(|f| f.badge),
            (&server.ip, &server.name),
            ICON,
        );
        let join = join_button(
            format!("join-server-{place}-join"),
            server.ip.clone(),
            offline,
            cx,
        );
        let more = self.card_menu_button(&id, (place, false), cx);
        let (pick_key, pick_order) = (key, order.clone());
        let card = server_row(
            id.clone(),
            RowParts {
                check,
                picture,
                name: crate::widgets::label::name_over(
                    server.name.clone(),
                    tokens::LABEL,
                    Some((words, ink)),
                ),
                note: answer,
                buttons: vec![join.into_any_element(), more.into_any_element()],
            },
        )
        .when(ticked, |d| d.bg(rgb(palette().selected)))
        .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
            let gesture = Gesture::of(&ev.modifiers());
            if gesture.selects() || any {
                this.multiplayer
                    .picks
                    .click(&pick_order, &pick_key, gesture.range);
                cx.notify();
            }
        }));
        Self::watch_hover(card, &id, cx)
    }

    /// The featured server `n` (of [`featured`]) as a card: no tick box, and no Join (and no
    /// menu) for one that has no address yet.
    pub(crate) fn featured_server_card(
        &mut self,
        n: usize,
        server: Featured,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id: SharedString = format!("join-featured:{n}").into();
        let reach = server
            .address
            .and_then(|address| self.multiplayer.reach.get(address).copied());
        let asking = if server.address.is_some() {
            tr(server.note).to_string()
        } else {
            tr(server.inert).to_string()
        };
        let (words, ink) = reach_line(reach, asking);
        let (join, more) = match server.address {
            Some(address) => (
                Some(join_button(
                    format!("join-featured-{n}-join"),
                    address.to_string(),
                    reach == Some(Reach::Offline),
                    cx,
                )),
                Some(self.card_menu_button(&id, (n, true), cx)),
            ),
            None => (None, None),
        };
        // The picture the server sends wins; until it does, the bundled one.
        let picture = server_picture(
            server
                .address
                .and_then(|address| self.multiplayer.icons.get(address).cloned()),
            Some(server.badge),
            (server.address.unwrap_or(server.name), server.name),
            ICON,
        );
        let buttons = match (join, more) {
            (Some(join), Some(more)) => vec![join.into_any_element(), more.into_any_element()],
            _ => Vec::new(),
        };
        server_row(
            id,
            RowParts {
                check: no_tick(),
                picture,
                name: crate::widgets::label::name_over(
                    tr(server.name).to_string(),
                    tokens::LABEL,
                    Some((words, ink)),
                ),
                note: None,
                buttons,
            },
        )
    }

    /// The bar under the list while servers are ticked: how many, Select all / Clear, and Remove.
    pub(crate) fn server_pick_bar(
        &mut self,
        order: &Rc<Vec<String>>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let ticks = &self.multiplayer.picks;
        if ticks.is_empty() {
            return None;
        }
        let toggle = bar::link(
            "join-pick-all",
            if ticks.all_of(order) {
                tr("Clear")
            } else {
                tr("Select all")
            },
        )
        .on_click({
            let order = order.clone();
            cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.multiplayer.picks.toggle_all(&order);
                cx.notify();
            })
        });
        let remove = bar::action("join-pick-remove", tr("Remove…"), true, true).on_click(
            cx.listener(|this, _: &ClickEvent, _, cx| {
                this.ask_remove_picked();
                cx.notify();
            }),
        );
        Some(bar::bar(
            ticks.len(),
            toggle,
            vec![remove.into_any_element()],
        ))
    }

    /// Asks once before every ticked server leaves the list.
    fn ask_remove_picked(&mut self) {
        let picked: Vec<(usize, String)> = self
            .multiplayer
            .list
            .iter()
            .enumerate()
            .filter(|(place, _)| self.multiplayer.picks.contains(&place.to_string()))
            .map(|(place, (server, _))| (place, server.name.clone()))
            .collect();
        if picked.is_empty() {
            return;
        }
        self.close_menus();
        self.multiplayer.dialog = Some(crate::multiplayer_page::ServerDialog::Remove(picked));
    }
}
