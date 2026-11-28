//! The Mods, Packs and Shaders tabs' rows, drawn as the Worlds tab draws its own: a bordered card
//! of one height with a picture, a name over a muted line, and on the right a switch and a "…" menu.
//! Pressing the card opens the item; the switch and the menu take their own presses. The tick
//! box of a multi-selection has a column left of the picture, always drawn faintly and at full
//! strength under the pointer.
use super::*;
use crate::widgets::button::{icon_button_opening, Style};
use crate::widgets::check::switch;
use crate::widgets::focus::c;
use std::path::PathBuf;
use std::{cell::Cell, rc::Rc};

/// The picture in a card: a world's size, so the three tabs line up.
const ICON: f32 = WORLD_ICON;
/// The room between two cards.
pub(crate) const GAP: f32 = WORLD_ROW_GAP;
/// The menu's width, and how close to the window's edge it may come.
const MENU_WIDTH: f32 = 210.;
const MENU_EDGE: f32 = 8.;
/// The column of the tick box: as wide as the box (18 px); the card's 12 px gap after it is the
/// breathing room before the picture.
const CHECK_SLOT: f32 = 18.;
/// How strongly an unticked box shows when the pointer is not on its card, and while something
/// in the list is ticked (the boxes are then what the player is working with).
const CHECK_REST: f32 = 0.45;
const CHECK_WHILE_TICKING: f32 = 0.7;

/// One thing a card's "…" menu can do.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ItemAction {
    Details,
    Update,
    ShowFile(PathBuf),
    CopyName(String),
    OpenPage(String),
    Remove,
}

/// A row of the menu. `off` leaves it dimmed and dead, with the reason as its tooltip ("" for
/// none).
pub(crate) struct MenuEntry {
    pub(crate) id: &'static str,
    pub(crate) icon: &'static str,
    pub(crate) label: SharedString,
    pub(crate) danger: bool,
    pub(crate) off: Option<SharedString>,
    pub(crate) action: ItemAction,
}

/// The card whose "…" menu is open, and where its button was drawn (window coordinates).
#[derive(Clone)]
pub(crate) struct ItemMenu {
    pub(crate) collection: Collection,
    pub(crate) key: String,
    button: gpui::Bounds<gpui::Pixels>,
}

/// What a card shows.
pub(crate) struct Card {
    pub(crate) id: SharedString,
    pub(crate) picture: AnyElement,
    pub(crate) title: String,
    pub(crate) detail: String,
    pub(crate) on: bool,
    /// False for a file the launcher can't switch: its switch is dimmed and does nothing.
    pub(crate) switchable: bool,
    pub(crate) pick: Pick,
}

/// A 32 px menu row, as the sidebar profile menu and the Worlds menu draw theirs.
fn menu_row(
    entry: MenuEntry,
    go: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let p = palette();
    let ink = if entry.danger { p.danger } else { p.text };
    let row = div()
        .id(entry.id)
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(tokens::CONTROL))
        .px(px(8.))
        .rounded(tokens::r(tokens::R_SMALL))
        .text_size(px(tokens::LABEL))
        .text_color(rgb(ink))
        .child(crate::glass_ui::icon(entry.icon, 16., c(ink)))
        .child(entry.label);
    match entry.off {
        Some(why) => row
            .opacity(0.45)
            .cursor_default()
            .when(!why.is_empty(), |d| d.tooltip(crate::glass_ui::tip(why)))
            .into_any_element(),
        None => row
            .reach()
            .cursor_pointer()
            .hover(|s| s.bg(rgb(p.hover)))
            .on_click(go)
            .into_any_element(),
    }
}

impl Launcher {
    /// Opens the "…" menu of the card `key`, hung from `button`.
    pub(crate) fn open_item_menu(
        &mut self,
        collection: Collection,
        key: String,
        button: gpui::Bounds<gpui::Pixels>,
    ) {
        self.close_menus();
        self.ui.home.tabs.menu = Some(ItemMenu {
            collection,
            key,
            button,
        });
    }

    /// Shuts the card menu if it is up. True when it was.
    pub(crate) fn close_item_menu(&mut self) -> bool {
        let was = self.ui.home.tabs.menu.take().is_some();
        if was {
            crate::glass_ui::menus_changed();
        }
        was
    }

    fn item_menu_open(&self, collection: Collection, key: &str) -> bool {
        self.ui
            .home
            .tabs
            .menu
            .as_ref()
            .is_some_and(|menu| menu.collection == collection && menu.key == key)
    }

    /// The layer over the window for the open card menu, if one is up on this tab.
    pub(super) fn home_item_menu(
        &mut self,
        active: Tab,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let menu = self.ui.home.tabs.menu.clone()?;
        let here = active.collection() == Some(menu.collection);
        // A dialog over the page, or another tab, takes the menu with it.
        if !here || self.layer_open() {
            self.close_item_menu();
            return None;
        }
        let entries = self.home_item_entries(menu.collection, &menu.key);
        if entries.is_empty() {
            self.close_item_menu();
            return None;
        }
        // The rows, the divider before a danger row (9) and 8 padding each side with the border.
        let dividers = entries.iter().filter(|entry| entry.danger).count() as f32;
        let height = entries.len() as f32 * tokens::CONTROL + dividers * 9. + 18.;
        let mut rows: Vec<AnyElement> = Vec::new();
        for entry in entries {
            if entry.danger {
                rows.push(
                    div()
                        .h(px(1.))
                        .my(px(4.))
                        .bg(rgb(palette().divider))
                        .into_any_element(),
                );
            }
            let (action, key, collection) =
                (entry.action.clone(), menu.key.clone(), menu.collection);
            rows.push(menu_row(
                entry,
                cx.listener(move |this, _, _, cx| {
                    this.close_item_menu();
                    this.home_item_run(collection, &key, action.clone(), cx);
                    cx.notify();
                }),
            ));
        }
        Some(self.menu_layer(rows, height, menu.button, cx))
    }

    /// `rows`, `height` tall, in a menu hung from `button`, over a layer that takes the window's
    /// presses: one outside the menu shuts it.
    fn menu_layer(
        &self,
        rows: Vec<AnyElement>,
        height: f32,
        button: gpui::Bounds<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette();
        let left = (f32::from(button.right()) - MENU_WIDTH)
            .min(self.window_width - MENU_WIDTH - MENU_EDGE)
            .max(MENU_EDGE);
        let below = f32::from(button.bottom()) + 4.;
        let top = if below + height + MENU_EDGE <= self.window_height {
            below
        } else {
            (f32::from(button.top()) - 4. - height).max(MENU_EDGE)
        };
        let layer = div()
            .id("home-item-menu")
            .relative()
            .w(px(self.window_width))
            .h(px(self.window_height))
            .occlude()
            .on_scroll(|_, _, cx| cx.stop_propagation())
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.close_item_menu();
                    cx.notify();
                }),
            )
            .on_mouse_down(
                gpui::MouseButton::Right,
                cx.listener(|this, _: &gpui::MouseDownEvent, _, cx| {
                    this.close_item_menu();
                    cx.notify();
                }),
            )
            .child(
                div()
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(MENU_WIDTH))
                    .p(px(8.))
                    .rounded(tokens::r(tokens::R_CARD))
                    .border_1()
                    .border_color(rgb(p.line))
                    .bg(rgb(p.raised))
                    .text_color(rgb(p.text))
                    .shadow_lg()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(gpui::MouseButton::Right, |_, _, cx| cx.stop_propagation())
                    .children(rows),
            );
        gpui::deferred(
            gpui::anchored()
                .position(gpui::point(px(0.), px(0.)))
                .child(layer),
        )
        .with_priority(20)
        .into_any_element()
    }

    /// The tick box, in a column of its own left of the picture. An unticked box is drawn faintly
    /// so it reads as something to tick, eases up to full strength while the pointer is on the
    /// card `id`, and a ticked one is always full (the accent, with its tick). `any`: something
    /// in the list is ticked, so the faint boxes are a little stronger. `press` takes the box's
    /// own click.
    pub(crate) fn tick_column(
        &mut self,
        id: &SharedString,
        ticked: bool,
        any: bool,
        press: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        let column = div()
            .flex_none()
            .w(px(CHECK_SLOT))
            .flex()
            .items_center()
            .child(
                checkbox(
                    SharedString::from(format!("{id}-pick")),
                    if ticked { Tick::On } else { Tick::Off },
                )
                .on_click(press),
            );
        if ticked {
            return column.into_any_element();
        }
        let rest = if any { CHECK_WHILE_TICKING } else { CHECK_REST };
        match self.ui.home.tabs.lit(id) {
            Some(fade) => fade.play(
                &SharedString::from(format!("{id}-fade")),
                column,
                move |d, lit| d.opacity(rest + (1. - rest) * lit),
            ),
            None => column.opacity(rest).into_any_element(),
        }
    }

    /// `card` (named `id`) telling the tabs when the pointer enters or leaves it, so its tick
    /// box can ease.
    pub(crate) fn watch_hover(
        card: Stateful<Div>,
        id: &SharedString,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = id.clone();
        card.on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            this.ui.home.tabs.warm(&id, *hovered);
            cx.notify();
        }))
    }

    /// A mod or pack card's tick box.
    fn card_check(&mut self, id: &SharedString, pick: &Pick, cx: &mut Context<Self>) -> AnyElement {
        let (collection, key, order) = (pick.collection, pick.key.clone(), pick.order.clone());
        self.tick_column(
            id,
            pick.ticked,
            pick.any,
            cx.listener(move |this, ev: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.home_pick(collection)
                    .click(&order, &key, ev.modifiers().shift);
                cx.notify();
            }),
        )
    }

    /// The card's switch and "…" button.
    fn card_controls(&mut self, id: &SharedString, card: &Card, cx: &mut Context<Self>) -> Div {
        let (collection, key) = (card.pick.collection, card.pick.key.clone());
        let switchable = card.switchable;
        let flip = key.clone();
        let switch = switch(SharedString::from(format!("{id}-switch")), card.on)
            .when(!switchable, |d| d.opacity(0.45))
            .tooltip(crate::glass_ui::tip(if card.on {
                tr("Disable")
            } else {
                tr("Enable")
            }))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                if switchable {
                    this.home_item_toggle(collection, &flip, cx);
                }
                cx.notify();
            }));
        // Where the "…" was drawn, for the menu that hangs from it.
        let drawn = Rc::new(Cell::new(gpui::Bounds::<gpui::Pixels>::default()));
        let (write, read) = (drawn.clone(), drawn);
        let more = icon_button_opening(
            SharedString::from(format!("{id}-more")),
            "more-horizontal",
            tr("More actions"),
            Style::Ghost,
            self.item_menu_open(collection, &key),
        )
        // The card is already lit under the pointer: the button's own light is a step brighter,
        // or it would not show on it.
        .hover(|s| s.bg(rgb(palette().selected)))
        .child(
            gpui::canvas(move |bounds, _, _| write.set(bounds), |_, _, _, _| {})
                .absolute()
                .inset_0(),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            cx.stop_propagation();
            this.open_item_menu(collection, key.clone(), read.get());
            cx.notify();
        }));
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .child(switch)
            .child(more)
    }

    /// One mod or pack as a card. A press opens it as `open` says; a press with Ctrl (Cmd) or
    /// Shift held ticks it instead.
    pub(crate) fn home_card(
        &mut self,
        card: Card,
        cx: &mut Context<Self>,
        open: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        let id = card.id.clone();
        let focus = self.ui.home.tabs.focus_for(id.clone(), cx);
        let check = self.card_check(&id, &card.pick, cx);
        let controls = self.card_controls(&id, &card, cx);
        let Card {
            picture,
            title,
            detail,
            pick,
            ..
        } = card;
        let name = div()
            .id(SharedString::from(format!("{id}-name")))
            .text_size(px(tokens::LABEL))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(c(palette().text))
            .truncate()
            .child(title.clone());
        let ticked = pick.ticked;
        let lit = id.clone();
        let card = crate::home::card()
            .id(id)
            .track_focus(&focus)
            .reach()
            .cursor_pointer()
            .flex_none()
            .h(px(WORLD_ROW))
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(12.))
            .hover(|s| s.bg(rgb(palette().hover)))
            .when(ticked, |d| d.bg(rgb(palette().selected)))
            .child(check)
            .child(div().flex_none().size(px(ICON)).child(picture))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(name)
                    .child(muted(detail).truncate()),
            )
            .child(controls)
            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                let gesture = Gesture::of(&ev.modifiers());
                if gesture.selects() {
                    this.home_pick(pick.collection)
                        .click(&pick.order, &pick.key, gesture.range);
                    cx.notify();
                    return;
                }
                this.ui.home.tabs.remember(Some(focus.clone()));
                open(this, cx);
                cx.notify();
            }));
        Self::watch_hover(card, &lit, cx)
    }
}
