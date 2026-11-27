//! A profile's menu in the sidebar: opened by its dots or a right-click, under the button or
//! the pointer. Settings, duplicate, open the folder, move up and down, which group it is in, and
//! Delete. The frame and rows are the group menu's too (`group_menu.rs`).
use super::*;
use gpui::ClickEvent;

/// How wide the menu is, and how far it keeps from the window's edge.
pub(super) const WIDTH: f32 = 236.;
const EDGE: f32 = 8.;
/// How wide the list of groups beside it is.
const SIDE_WIDTH: f32 = 208.;

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
            rows.push(Row::new(id, icon, label, p.text).draw(cx.listener(
                move |this, _, _, cx| {
                    this.close_profile_action();
                    this.move_profile(&moved, way);
                    cx.notify();
                },
            )));
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
            .draw(cx.listener(|this, _, _, cx| {
                if let Some(Action::Menu { groups, .. }) = &mut this.ui.sheets.profile_action {
                    *groups = !*groups;
                }
                cx.notify();
            })),
        );
        rows
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
                .draw(cx.listener(move |this, _, _, cx| {
                    this.confirm_profile_delete(&doomed);
                    cx.notify();
                })),
            );
            count += 1;
        }
        let side = groups.then(|| self.group_choices(target, cx));
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
    /// list beside it, with its rows and their count. Pressing the layer under it closes it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn menu_frame(
        &self,
        id: &'static str,
        name: String,
        rows: Vec<AnyElement>,
        (count, divided): (usize, bool),
        side: Option<(Vec<AnyElement>, usize)>,
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
        let side = side.map(|(rows, count)| {
            let height =
                (count as f32 * tokens::CONTROL + 26. + 18.).min(self.window_height - 2. * EDGE);
            // Beside the menu, on its right, or on its left when the window ends first.
            let left = if at + WIDTH + 4. + SIDE_WIDTH + EDGE <= self.window_width {
                at + WIDTH + 4.
            } else {
                (at - SIDE_WIDTH - 4.).max(EDGE)
            };
            div()
                .id("profile-menu-side")
                .absolute()
                .left(px(left))
                .top(px(top.min(self.window_height - height - EDGE).max(EDGE)))
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
                .child(
                    div()
                        .h(px(26.))
                        .px(px(8.))
                        .truncate()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(tokens::SMALL))
                        .text_color(rgb(p.muted))
                        .child(tr("Move to group")),
                )
                .children(rows)
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
