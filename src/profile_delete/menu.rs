//! A profile's menu in the sidebar: opened by its dots or a right-click, under the button or
//! the pointer. Settings, duplicate, open the folder, move up and down, and Delete.
use super::*;
use gpui::ClickEvent;

/// One row of the menu.
struct Row {
    id: &'static str,
    icon: &'static str,
    label: SharedString,
    ink: u32,
    /// Why it can't be used now, if so ("" for no reason to give): it stays, dimmed.
    off: Option<SharedString>,
}

impl Row {
    fn new(id: &'static str, icon: &'static str, label: impl Into<SharedString>, ink: u32) -> Self {
        Self {
            id,
            icon,
            label: label.into(),
            ink,
            off: None,
        }
    }

    fn unless(mut self, off: bool, why: impl Into<SharedString>) -> Self {
        if off {
            self.off = Some(why.into());
        }
        self
    }

    /// A 32 px row: a 16 px icon and its label, filled on hover, a Tab stop while it works.
    /// `go` runs on a press, unless the row is off.
    fn draw(self, go: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> AnyElement {
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
            .child(self.label);
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
        if target.kind != instances::Kind::Default {
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
        }
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
        top: f32,
        button: Option<(f32, f32)>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = palette();
        const WIDTH: f32 = 236.;
        const EDGE: f32 = 8.;
        let name = target.name.clone();
        let mut rows = self.profile_menu_rows(target, cx);
        // Delete is for the player's own profiles, after a divider.
        let own = target.kind == instances::Kind::Own;
        let mut count = rows.len();
        if own {
            let doomed = target.clone();
            rows.push(
                div()
                    .h(px(1.))
                    .my(px(4.))
                    .bg(rgb(p.divider))
                    .into_any_element(),
            );
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
        // The rows, the title (26), the divider (9), 8 padding each side and the 1 px border.
        let action_height = count as f32 * tokens::CONTROL + 26. + if own { 9. } else { 0. } + 18.;
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
        div()
            .id("profile-menu")
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
    }
}
