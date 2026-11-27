//! A profile's menu in the sidebar: opened by its dots or a right-click, under the button or
//! the pointer. Settings, duplicate, open the folder, move up and down, which group it is in, and
//! Delete. The frame and rows are the group menu's too (`group_menu.rs`).
use super::*;
use gpui::ClickEvent;
use std::{cell::Cell, rc::Rc, time::Duration};

/// How wide the menu is, and how far it keeps from the window's edge.
pub(super) const WIDTH: f32 = 236.;
const EDGE: f32 = 8.;
/// How wide the list of groups beside it is, how far it sits from the menu, and how far it
/// slides while it fades in.
const SIDE_WIDTH: f32 = 208.;
const SIDE_GAP: f32 = 2.;
const SIDE_SLIDE: f32 = 4.;
/// How long the pointer may be away from the "Move to group" row and the list before it closes.
const LEAVE_DELAY: Duration = Duration::from_millis(200);

/// Where the pointer is, for the list of groups beside the menu: on its row, on the list, and
/// where the row was last painted (its top is where the list's goes).
#[derive(Default)]
pub(crate) struct GroupSub {
    row: bool,
    over: bool,
    bounds: Rc<Cell<gpui::Bounds<gpui::Pixels>>>,
}

type HoverFn = Box<dyn Fn(&bool, &mut Window, &mut App)>;

/// One row of the menu.
pub(super) struct Row {
    id: ElementId,
    icon: &'static str,
    label: SharedString,
    ink: u32,
    /// Why it can't be used now, if so ("" for no reason to give): it stays, dimmed.
    off: Option<SharedString>,
    /// It opens a list beside the menu: an arrow ends the row.
    opens: bool,
    /// What the pointer entering or leaving the row does.
    hover: Option<HoverFn>,
    /// Where the row is painted, kept here for whoever places something beside it.
    seen: Option<Rc<Cell<gpui::Bounds<gpui::Pixels>>>>,
}

impl Row {
    pub(super) fn new(
        id: impl Into<ElementId>,
        icon: &'static str,
        label: impl Into<SharedString>,
        ink: u32,
    ) -> Self {
        Self {
            id: id.into(),
            icon,
            label: label.into(),
            ink,
            off: None,
            opens: false,
            hover: None,
            seen: None,
        }
    }

    pub(super) fn unless(mut self, off: bool, why: impl Into<SharedString>) -> Self {
        if off {
            self.off = Some(why.into());
        }
        self
    }

    fn opening(mut self) -> Self {
        self.opens = true;
        self
    }

    fn on_hover(mut self, hover: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.hover = Some(Box::new(hover));
        self
    }

    fn painted_at(mut self, seen: Rc<Cell<gpui::Bounds<gpui::Pixels>>>) -> Self {
        self.seen = Some(seen);
        self
    }

    /// A 32 px row: a 16 px icon and its label, filled on hover, a Tab stop while it works.
    /// `go` runs on a press, unless the row is off.
    pub(super) fn draw(
        self,
        go: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        let p = palette();
        let row = div()
            .id(self.id)
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(tokens::CONTROL))
            .px(px(8.))
            .rounded(tokens::r(tokens::R_SMALL))
            .text_size(px(tokens::LABEL))
            .text_color(rgb(self.ink))
            .child(crate::glass_ui::icon(
                self.icon,
                16.,
                crate::widgets::focus::c(self.ink),
            ))
            .child(self.label)
            .when_some(self.seen, |d, seen| {
                d.child(
                    gpui::canvas(move |bounds, _, _| seen.set(bounds), |_, _, _, _| {})
                        .absolute()
                        .inset_0(),
                )
            })
            .when_some(self.hover, |d, hover| d.on_hover(hover))
            .when(self.opens, |d| {
                d.child(div().flex_1()).child(crate::glass_ui::icon(
                    "arrow-right-01",
                    14.,
                    crate::widgets::focus::c(self.ink),
                ))
            });
        match self.off {
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
}

/// Another row under the pointer closes the list of groups at once.
fn away(cx: &mut Context<Launcher>) -> impl Fn(&bool, &mut Window, &mut App) + 'static {
    let view = cx.weak_entity();
    move |hovered: &bool, _: &mut Window, cx: &mut App| {
        if *hovered {
            let _ = view.update(cx, |this, cx| this.group_sub_close_now(cx));
        }
    }
}

impl Launcher {
    /// The menu's rows for `target` down to Open folder and the moves.
    fn profile_menu_rows(&self, target: &Instance, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = palette();
        let mut rows: Vec<AnyElement> = Vec::new();
        let edited = target.clone();
        rows.push(
            Row::new(
                "profile-menu-edit",
                "settings-01",
                tr("Profile settings…"),
                p.text,
            )
            .on_hover(away(cx))
            .draw(cx.listener(move |this, _, _, cx| {
                this.close_profile_action();
                this.edit_profile(&edited, cx);
            })),
        );
        let copied = target.clone();
        let locked = self.copy_lock_for(target);
        rows.push(
            Row::new(
                "profile-menu-duplicate",
                "copy-01",
                tr("Duplicate…"),
                p.text,
            )
            .unless(locked.is_some(), locked.unwrap_or_default())
            .on_hover(away(cx))
            .draw(cx.listener(move |this, _, _, cx| {
                this.close_profile_action();
                this.duplicate_profile(&copied, cx);
            })),
        );
        let folder = target.home.clone();
        rows.push(
            Row::new(
                "profile-menu-folder",
                "folder-01",
                tr("Open folder"),
                p.text,
            )
            .on_hover(away(cx))
            .draw(cx.listener(move |this, _, _, cx| {
                this.close_profile_action();
                crate::toast::open(cx, &folder);
                cx.notify();
            })),
        );
        let (up, down) = instances::move_options(&self.instances.list, target);
        for (id, icon, label, way, on) in [
            ("profile-menu-up", "arrow-up-01", tr("Move up"), true, up),
            (
                "profile-menu-down",
                "arrow-down-01",
                tr("Move down"),
                false,
                down,
            ),
        ] {
            if !on {
                continue;
            }
            let moved = target.id.clone();
            rows.push(
                Row::new(id, icon, label, p.text)
                    .on_hover(away(cx))
                    .draw(cx.listener(move |this, _, _, cx| {
                        this.close_profile_action();
                        this.move_profile(&moved, way);
                        cx.notify();
                    })),
            );
        }
        // The group it is in: a list of the groups opens beside the menu.
        rows.push(
            Row::new(
                "profile-menu-group",
                "folder-01",
                tr("Move to group"),
                p.text,
            )
            .opening()
            .painted_at(self.group_sub_bounds())
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if *hovered {
                    this.group_sub_enter(cx);
                } else {
                    this.group_sub_row_left(cx);
                }
            }))
            .draw(cx.listener(|this, _, _, cx| {
                // A press opens it too, for the keyboard and touch.
                this.group_sub_enter(cx);
            })),
        );
        rows
    }

    /// Where the "Move to group" row was last painted, kept by the open menu.
    fn group_sub_bounds(&self) -> Rc<Cell<gpui::Bounds<gpui::Pixels>>> {
        match &self.ui.sheets.profile_action {
            Some(Action::Menu { sub, .. }) => sub.bounds.clone(),
            _ => Rc::default(),
        }
    }

    /// The pointer is on the "Move to group" row (or it was pressed): the list is open.
    fn group_sub_enter(&mut self, cx: &mut Context<Self>) {
        if let Some(Action::Menu { groups, sub, .. }) = &mut self.ui.sheets.profile_action {
            sub.row = true;
            if !*groups {
                *groups = true;
                cx.notify();
            }
        }
    }

    /// The pointer is on another row: the list goes at once.
    fn group_sub_close_now(&mut self, cx: &mut Context<Self>) {
        if let Some(Action::Menu { groups, sub, .. }) = &mut self.ui.sheets.profile_action {
            sub.row = false;
            if std::mem::take(groups) {
                cx.notify();
            }
        }
    }

    /// The pointer left the "Move to group" row.
    fn group_sub_row_left(&mut self, cx: &mut Context<Self>) {
        if let Some(Action::Menu { sub, .. }) = &mut self.ui.sheets.profile_action {
            sub.row = false;
        }
        self.group_sub_wait_to_close(cx);
    }

    /// The pointer is on the list, or left it.
    fn group_sub_over(&mut self, over: bool, cx: &mut Context<Self>) {
        if let Some(Action::Menu { sub, .. }) = &mut self.ui.sheets.profile_action {
            sub.over = over;
        }
        if !over {
            self.group_sub_wait_to_close(cx);
        }
    }

    /// Closes the list after a short while, unless the pointer is on the row or on it by then, so
    /// a path across the gap between them doesn't lose it.
    fn group_sub_wait_to_close(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LEAVE_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                if let Some(Action::Menu { groups, sub, .. }) = &mut this.ui.sheets.profile_action {
                    if !sub.row && !sub.over && std::mem::take(groups) {
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    /// The New profile dialog, set to copy `profile`.
    fn duplicate_profile(&mut self, profile: &Instance, cx: &mut Context<Self>) {
        self.new_profile(cx);
        let preview = instances::CopyPreview::read(profile);
        if let Some(d) = self.instances.dialog.as_mut() {
            match preview {
                Ok(copy) => d.copy = Some(copy),
                Err(error) => d.error = Some(error),
            }
        }
        cx.notify();
    }

    fn move_profile(&mut self, id: &str, up: bool) {
        if let Err(error) = instances::move_profile(id, up) {
            crate::toast::say(error);
        }
        self.refresh_instances_list();
    }

    pub(super) fn profile_actions_menu(
        &self,
        target: &Instance,
        groups: bool,
        top: f32,
        button: Option<(f32, f32)>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = palette();
        let mut rows = self.profile_menu_rows(target, cx);
        // Delete is for the player's own profiles, after a divider.
        let own = target.kind == instances::Kind::Own;
        let mut count = rows.len();
        if own {
            let doomed = target.clone();
            rows.push(divider_row());
            rows.push(
                Row::new(
                    "profile-menu-delete",
                    "delete-02",
                    tr("Delete profile…"),
                    p.danger,
                )
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    if *hovered {
                        this.group_sub_close_now(cx);
                    }
                }))
                .draw(cx.listener(move |this, _, _, cx| {
                    this.confirm_profile_delete(&doomed);
                    cx.notify();
                })),
            );
            count += 1;
        }
        let row_top = match &self.ui.sheets.profile_action {
            Some(Action::Menu { sub, .. }) => f32::from(sub.bounds.get().top()),
            _ => top,
        };
        let side = groups.then(|| {
            let (rows, count) = self.group_choices(target, cx);
            (rows, count, row_top)
        });
        self.menu_frame(
            "profile-menu",
            target.name.clone(),
            rows,
            (count, own),
            side,
            (top, button),
            cx,
        )
    }

    /// The list beside the profile's menu: the groups to move it to, none, and a new one.
    /// Returns its rows and their count.
    fn group_choices(&self, target: &Instance, cx: &mut Context<Self>) -> (Vec<AnyElement>, usize) {
        let p = palette();
        let mut rows: Vec<AnyElement> = Vec::new();
        let (current, id) = (target.group.clone(), target.id.clone());
        for (n, name) in instances::group_names(&self.instances.list)
            .into_iter()
            .enumerate()
        {
            let here = current.as_deref() == Some(name.as_str());
            let (moved, to) = (id.clone(), name.clone());
            rows.push(
                Row::new(
                    ElementId::named_usize("profile-menu-to-group", n),
                    if here { "tick-02" } else { "folder-01" },
                    name,
                    p.text,
                )
                .unless(here, "")
                .draw(cx.listener(move |this, _, _, cx| {
                    this.close_profile_action();
                    this.put_in_group(&moved, Some(to.clone()));
                    cx.notify();
                })),
            );
        }
        if current.is_some() {
            let moved = id.clone();
            rows.push(
                Row::new("profile-menu-no-group", "cancel-01", tr("No group"), p.text).draw(
                    cx.listener(move |this, _, _, cx| {
                        this.close_profile_action();
                        this.put_in_group(&moved, None);
                        cx.notify();
                    }),
                ),
            );
        }
        rows.push(
            Row::new(
                "profile-menu-new-group",
                "plus-sign",
                tr("New group…"),
                p.text,
            )
            .draw(cx.listener(move |this, _, _, cx| {
                this.close_profile_action();
                this.ask_group_name(None, Some(id.clone()), cx);
                cx.notify();
            })),
        );
        let count = rows.len();
        (rows, count)
    }

    /// The menu's frame: a title, the `rows` (`count` of them, with a divider if `divided`), at
    /// `top` (and hung from the actions button's `(left, top)`, if it has one). `side` is a
    /// list beside it, with its rows, their count and the top of the row it opens from; it fades
    /// in beside the menu without moving it. Pressing the layer under it closes it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn menu_frame(
        &self,
        id: &'static str,
        name: String,
        rows: Vec<AnyElement>,
        (count, divided): (usize, bool),
        side: Option<(Vec<AnyElement>, usize, f32)>,
        (top, button): (f32, Option<(f32, f32)>),
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = palette();
        // The rows, the title (26), the divider (9), 8 padding each side and the 1 px border.
        let action_height =
            count as f32 * tokens::CONTROL + 26. + if divided { 9. } else { 0. } + 18.;
        let (at, top) = match button {
            // Hung from the actions button: its left edge, below it, or above it when the
            // window ends first.
            Some((left, button_top)) => {
                let fits = top + action_height + EDGE <= self.window_height;
                (
                    left.min(self.window_width - WIDTH - EDGE).max(EDGE),
                    if fits {
                        top
                    } else {
                        (button_top - 4. - action_height).max(EDGE)
                    },
                )
            }
            None => (
                tokens::FRAME_INSET + crate::rail::rail_width(self.rail_open()) + 8.,
                top.min(self.window_height - action_height - EDGE),
            ),
        };
        let side = side.map(|(rows, count, row_top)| {
            let height = (count as f32 * tokens::CONTROL + 18.).min(self.window_height - 2. * EDGE);
            // Beside the menu, on its right, or on its left when the window ends first.
            let (left, slide) = if at + WIDTH + SIDE_GAP + SIDE_WIDTH + EDGE <= self.window_width {
                (at + WIDTH + SIDE_GAP, -SIDE_SLIDE)
            } else {
                ((at - SIDE_WIDTH - SIDE_GAP).max(EDGE), SIDE_SLIDE)
            };
            // Its first row is level with the menu's row: the card's border and padding sit above.
            let top = (row_top - 9.)
                .min(self.window_height - height - EDGE)
                .max(EDGE);
            div()
                .id("profile-menu-side")
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(SIDE_WIDTH))
                .max_h(px(self.window_height - 2. * EDGE))
                .overflow_y_scroll()
                .p(px(8.))
                .rounded(tokens::r(tokens::R_CARD))
                .border_1()
                .border_color(rgb(p.line))
                .bg(rgb(p.raised))
                .text_color(rgb(p.text))
                .shadow_lg()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    this.group_sub_over(*hovered, cx);
                }))
                .children(rows)
                .with_animation(
                    "profile-menu-side-in",
                    crate::motion::eased(sunrise_launcher_theme::QUICK),
                    move |d, t| d.opacity(t).left(px(left + slide * (1. - t))),
                )
        });
        div()
            .id(id)
            .absolute()
            .inset_0()
            .occlude()
            .on_scroll(|_, _, cx| cx.stop_propagation())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.close_profile_action();
                    cx.notify();
                }),
            )
            // This layer covers the profiles: a right-click on another one closes this menu
            // and opens that profile's, and anywhere else it only closes this one.
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    let under = this
                        .ui
                        .profile_rows
                        .borrow()
                        .iter()
                        .find(|(bounds, _)| bounds.contains(&ev.position))
                        .map(|(_, profile)| profile.clone());
                    this.close_profile_action();
                    if let Some(profile) = under {
                        this.open_profile_menu(&profile, f32::from(ev.position.y));
                    }
                    cx.notify();
                }),
            )
            .child(
                div()
                    .absolute()
                    .left(px(at))
                    .top(px(top))
                    .w(px(WIDTH))
                    .p(px(8.))
                    .rounded(tokens::r(tokens::R_CARD))
                    .border_1()
                    .border_color(rgb(p.line))
                    .bg(rgb(p.raised))
                    .text_color(rgb(p.text))
                    .shadow_lg()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .h(px(26.))
                            .px(px(8.))
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(tokens::SMALL))
                            .text_color(rgb(p.muted))
                            .child(name),
                    )
                    .children(rows),
            )
            .children(side)
    }
}

/// The hairline between a menu's rows: 1 px with 4 px above and below.
pub(super) fn divider_row() -> AnyElement {
    div()
        .h(px(1.))
        .my(px(4.))
        .bg(rgb(palette().divider))
        .into_any_element()
}
