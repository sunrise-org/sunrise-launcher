//! A group's menu in the sidebar, opened by the button that slides in on its heading: rename,
//! a new profile in it, move it up or down, fold or unfold it, take its profiles out of it, and
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
            group: Some(group.to_owned()),
        });
    }

    /// Whether the group `group` has its menu open, or a dialog opened from it is up: its
    /// button stays out and its tooltip stays away.
    pub(crate) fn group_busy_with(&self, group: &str) -> bool {
        match &self.ui.sheets.profile_action {
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
        let locked = self.profile_lock();
        rows.push(
            Row::new(
                "group-menu-new-profile",
                "plus-sign",
                tr("New profile in this group"),
                p.text,
            )
            .unless(locked.is_some(), locked.unwrap_or_default())
            .draw(cx.listener({
                let group = named.clone();
                move |this, _, _, cx| {
                    this.close_profile_action();
                    this.new_profile_in_group(&group, cx);
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
        rows.push(
            Row::new("group-menu-ungroup", "cancel-01", tr("Ungroup"), p.text).draw(cx.listener({
                let group = named.clone();
                move |this, _, _, cx| {
                    this.close_profile_action();
                    this.ungroup(&group);
                    cx.notify();
                }
            })),
        );
        // Deleting is for the player's own profiles: a group of only the default profile or
        // packs has nothing to delete.
        let deletable = self
            .instances
            .list
            .iter()
            .any(|i| i.group.as_deref() == Some(group) && i.kind == instances::Kind::Own);
        let mut count = rows.len();
        if deletable {
            rows.push(divider_row());
            rows.push(
                Row::new(
                    "group-menu-delete",
                    "delete-02",
                    tr("Delete group and its profiles…"),
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
            (count, deletable),
            None,
            (top, button),
            cx,
        )
    }
}
