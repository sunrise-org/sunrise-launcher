//! A group's menu in the sidebar, opened by the button that slides in on its heading: rename,
//! move it up or down, fold or unfold it, take its profiles out of it, and
//! delete it with them. It is drawn in the profile menu's frame (`menu.rs`).
use super::menu::{divider_row, Row};
use super::*;

impl Launcher {
    /// The menu of the group `group`, under its button at `button` (window coordinates).
    pub(crate) fn open_group_menu_under(
        &mut self,
        group: &str,
        button: gpui::Bounds<gpui::Pixels>,
    ) {
        let Some(first) = self
            .instances
            .list
            .iter()
            .find(|i| i.group.as_deref() == Some(group))
            .cloned()
        else {
            return;
        };
        self.close_menus();
        self.ui.sheets.profile_action = Some(Action::Menu {
            target: first,
            top: f32::from(button.bottom()) + 4.,
            button: Some((f32::from(button.left()), f32::from(button.top()))),
            groups: false,
            sub: super::menu::GroupSub::default(),
            group: Some(group.to_owned()),
        });
    }

    /// Whether the group `group` has its menu open, or a dialog opened from it (or from its plus
    /// button) is up: its buttons stay out and their tooltips stay away.
    pub(crate) fn group_busy_with(&self, group: &str) -> bool {
        let new_here = self.instances.dialog.as_ref().is_some_and(|d| {
            d.editing.is_none()
                && d.copy.is_none()
                && self
                    .servers
                    .value(crate::servers_page::Field::InstanceGroup)
                    .trim()
                    == group
        });
        new_here
            || match &self.ui.sheets.profile_action {
                Some(Action::Menu {
                    group: Some(open), ..
                }) => open == group,
                Some(Action::GroupName {
                    rename: Some(open), ..
                }) => open == group,
                Some(Action::ConfirmGroup { group: open, .. }) => open == group,
                _ => false,
            }
    }

    pub(super) fn group_actions_menu(
        &self,
        group: &str,
        top: f32,
        button: Option<(f32, f32)>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = palette();
        let mut rows: Vec<AnyElement> = Vec::new();
        let named = group.to_owned();
        rows.push(
            Row::new(
                "group-menu-rename",
                "pencil-edit-02",
                tr("Rename group…"),
                p.text,
            )
            .draw(cx.listener({
                let group = named.clone();
                move |this, _, _, cx| {
                    this.close_profile_action();
                    this.ask_group_name(Some(group.clone()), None, cx);
                    cx.notify();
                }
            })),
        );
        let (up, down) = instances::move_group_options(&self.instances.list, group);
        for (id, icon, label, way, on) in [
            (
                "group-menu-up",
                "arrow-up-01",
                tr("Move group up"),
                true,
                up,
            ),
            (
                "group-menu-down",
                "arrow-down-01",
                tr("Move group down"),
                false,
                down,
            ),
        ] {
            if !on {
                continue;
            }
            let moved = named.clone();
            rows.push(Row::new(id, icon, label, p.text).draw(cx.listener(
                move |this, _, _, cx| {
                    this.close_profile_action();
                    this.move_group(&moved, way);
                    cx.notify();
                },
            )));
        }
        let folded = crate::rail::group_collapsed(group);
        let (icon, label) = if folded {
            ("arrow-down-01", tr("Expand"))
        } else {
            ("minus-sign", tr("Collapse"))
        };
        rows.push(
            Row::new("group-menu-fold", icon, label, p.text).draw(cx.listener({
                let group = named.clone();
                move |this, _, _, cx| {
                    this.close_profile_action();
                    this.toggle_group(&group, cx);
                }
            })),
        );
        // Delete asks what goes: the group alone (its profiles stay, with no group), or the
        // profiles with it.
        let mut count = rows.len();
        {
            rows.push(divider_row());
            rows.push(
                Row::new(
                    "group-menu-delete",
                    "delete-02",
                    tr("Delete group…"),
                    p.danger,
                )
                .draw(cx.listener({
                    let group = named.clone();
                    move |this, _, _, cx| {
                        this.confirm_group_delete(&group);
                        cx.notify();
                    }
                })),
            );
            count += 1;
        }
        self.menu_frame(
            "group-menu",
            named,
            rows,
            (count, true),
            None,
            (top, button),
            cx,
        )
    }
}
