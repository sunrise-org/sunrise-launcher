//! What a group's menu and the profile menu's "Move to group" do: ask for a name, rename, move,
//! fold, ungroup and delete a group, and put a profile in one.
use super::*;
use crate::servers_page::Field;
use crate::widgets::button::{danger, primary, secondary};
use crate::{selectable::selectable, sheets::title};

/// What deleting a group did: the profiles that went to the Trash, and the first one that
/// didn't (its name and why).
type Outcome = (Vec<Instance>, Option<(String, DeleteError)>);

/// Moves each of `doomed` to the Trash, stopping at the first that fails, then lets what is
/// still in the group `group` (a profile that can't be deleted) out of it.
fn trash_all(doomed: Vec<Instance>, group: String) -> Outcome {
    let mut gone = Vec::new();
    for target in doomed {
        let name = target.name.clone();
        match instances::trash_confirmed(target) {
            Ok(profile) => gone.push(profile),
            Err(error) => return (gone, Some((name, error))),
        }
    }
    let left = instances::ungroup_group(&group)
        .err()
        .map(|detail| (group, DeleteError::from(detail)));
    (gone, left)
}

impl Launcher {
    /// Opens the box that asks for a group's name: renaming `rename`, or (without) a new group for
    /// the profile `profile`. Enter in it is [`Self::commit_group_name`].
    pub(crate) fn ask_group_name(
        &mut self,
        rename: Option<String>,
        profile: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.servers
            .text(Field::InstanceGroup)
            .set(rename.clone().unwrap_or_default());
        self.focus_handle(Field::InstanceGroup, cx);
        self.servers.focus_soon = Some(Field::InstanceGroup);
        self.close_menus();
        self.ui.sheets.profile_action = Some(Action::GroupName { rename, profile });
        cx.notify();
    }

    /// The box that asks for a group's name: its field, and Cancel and the button that applies it.
    pub(super) fn group_name_sheet(
        &mut self,
        renaming: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let field = self.field(Field::InstanceGroup, tr("Group name"), 340., cx);
        let ready = !self.servers.value(Field::InstanceGroup).trim().is_empty();
        let (heading, apply) = if renaming {
            (tr("Rename group"), tr("Rename"))
        } else {
            (tr("New group"), tr("Create group"))
        };
        let body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(title(heading.into()))
            .child(field)
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        secondary("group-name-cancel", tr("Cancel")).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.close_profile_action();
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        primary("group-name-apply", apply)
                            .when(!ready, |d| d.opacity(0.45).tab_stop(false))
                            .when(ready, |d| {
                                d.on_click(cx.listener(|this, _, _, cx| {
                                    this.commit_group_name(cx);
                                }))
                            }),
                    ),
            );
        modal_frame(
            "group-name-sheet",
            400.,
            body,
            cx,
            Self::close_profile_action,
        )
    }

    /// Applies the name typed in the group name box. False when that box isn't open, so Enter in
    /// the field can go on to what it does elsewhere (`save_instance`).
    pub(crate) fn commit_group_name(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(Action::GroupName { rename, profile }) = &self.ui.sheets.profile_action else {
            return false;
        };
        let (rename, profile) = (rename.clone(), profile.clone());
        let name = self.servers.value(Field::InstanceGroup).trim().to_owned();
        if name.is_empty() {
            return true;
        }
        let result = match (&rename, &profile) {
            (Some(old), _) => instances::rename_group(old, &name),
            (None, Some(id)) => instances::edit(id, |m| m.group = Some(name.clone())).map(|_| ()),
            (None, None) => Ok(()),
        };
        match result {
            Ok(()) => self.ui.sheets.profile_action = None,
            Err(error) => crate::toast::say(error),
        }
        self.groups_changed();
        cx.notify();
        true
    }

    /// The list of profiles is read again, and the folds with it, after a change to the groups.
    fn groups_changed(&mut self) {
        crate::rail::reload_group_folds();
        self.refresh_instances_list();
    }

    /// Puts the profile `id` in the group `group`, or (None) takes it out of its group.
    pub(super) fn put_in_group(&mut self, id: &str, group: Option<String>) {
        if let Err(error) = instances::edit(id, |m| m.group = group) {
            crate::toast::say(error);
        }
        self.groups_changed();
    }

    pub(super) fn move_group(&mut self, group: &str, up: bool) {
        if let Err(error) = instances::move_group(group, up) {
            crate::toast::say(error);
        }
        self.groups_changed();
    }

    /// Takes every profile out of the group `group`; they stay, with no group.
    pub(super) fn ungroup(&mut self, group: &str) {
        if let Err(error) = instances::ungroup_group(group) {
            crate::toast::say(error);
        }
        self.groups_changed();
    }

    /// The New profile dialog with Group already set to `group`.
    pub(super) fn new_profile_in_group(&mut self, group: &str, cx: &mut Context<Self>) {
        self.new_profile(cx);
        self.servers
            .text(Field::InstanceGroup)
            .set(group.to_owned());
        cx.notify();
    }

    /// Asks before deleting the group `group` and its profiles.
    pub(super) fn confirm_group_delete(&mut self, group: &str) {
        let inside: Vec<Instance> = self
            .instances
            .list
            .iter()
            .filter(|i| i.group.as_deref() == Some(group))
            .cloned()
            .collect();
        let doomed: Vec<Instance> = inside
            .iter()
            .filter(|i| i.kind == instances::Kind::Own)
            .cloned()
            .collect();
        if doomed.is_empty() {
            return;
        }
        let kept = inside.len() - doomed.len();
        self.ui.sheets.profile_action = Some(Action::ConfirmGroup {
            group: group.to_owned(),
            doomed,
            kept,
        });
    }

    /// The question before a group is deleted: how many profiles go to the Trash.
    pub(super) fn group_delete_body(
        &self,
        body: Div,
        group: &str,
        doomed: &[Instance],
        kept: usize,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = palette();
        let names = doomed
            .iter()
            .map(|i| i.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let single = doomed.len() == 1;
        let count = doomed.len().to_string();
        let what = if single {
            tr("This profile and its worlds and mods will move to the Trash. You can restore it from there.")
                .to_owned()
        } else {
            trf(
                "These {count} profiles and their worlds and mods will move to the Trash. You can restore them from there.",
                &[("count", &count)],
            )
        };
        body.gap(px(8.))
            .child(title(trf(
                "Delete {name} and its profiles?",
                &[("name", group)],
            )))
            .child(div().text_color(rgb(p.muted)).child(what))
            .child(
                div()
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(p.muted))
                    .child(selectable(names)),
            )
            .children((kept > 0).then(|| {
                div()
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(p.muted))
                    .child(tr(
                        "The group's other profiles can't be deleted: they stay, with no group.",
                    ))
            }))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        secondary("group-delete-cancel", tr("Cancel")).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.close_profile_action();
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        danger(
                            "group-delete-confirm",
                            if single {
                                tr("Delete profile")
                            } else {
                                tr("Delete profiles")
                            },
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.delete_group(cx))),
                    ),
            )
    }

    fn delete_group(&mut self, cx: &mut Context<Self>) {
        let Some(Action::ConfirmGroup { group, doomed, .. }) = &self.ui.sheets.profile_action
        else {
            return;
        };
        let (group, doomed) = (group.clone(), doomed.clone());
        self.ui.sheets.profile_action = Some(Action::Deleting(group.clone()));
        cx.set_global(MoveInFlight::default());
        let named = group.clone();
        let task = cx
            .background_executor()
            .spawn(async move { trash_all(doomed, named) });
        cx.spawn(async move |this, cx| {
            let (gone, failed) = task.await;
            let _ = this.update(cx, |this, cx| {
                this.finish_group_delete(&group, &gone, failed);
                let movement = cx.remove_global::<MoveInFlight>();
                // A failed move or save leaves its result on screen, even after a close request.
                if movement.quit_requested && this.ui.sheets.profile_action.is_none() {
                    App::defer(cx, crate::shut_down);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn finish_group_delete(
        &mut self,
        group: &str,
        gone: &[Instance],
        failed: Option<(String, DeleteError)>,
    ) {
        self.groups_changed();
        // Downloads also save these settings: clear the deleted profiles in the window's own
        // copy and commit it here, as deleting one profile does.
        let mut cleared = false;
        for profile in gone {
            cleared |= instances::clear_deleted_selection(&mut self.settings, &profile.id);
        }
        let failed_save = if cleared {
            self.clear_profile_notices();
            self.save_settings();
            self.settings_save_error.clone()
        } else {
            None
        };
        self.ui.sheets.profile_action = match failed {
            None => {
                let heading = trf(
                    "{name} and its profiles are in the Trash.",
                    &[("name", group)],
                );
                match failed_save {
                    Some(error) => Some(Action::Problem {
                        heading,
                        message: Some(
                            tr("The new profile selection could not be saved.").to_owned(),
                        ),
                        error,
                        details: false,
                        recovery: None,
                    }),
                    None => {
                        crate::toast::say(heading);
                        None
                    }
                }
            }
            Some((name, error)) => Some(Action::Problem {
                heading: trf(
                    "Couldn't move {name} to the Trash",
                    &[("name", name.as_str())],
                ),
                message: error.retained.as_ref().map(|notice| {
                    trf(
                        "The profile was kept in {folder}",
                        &[("folder", &notice.folder().to_string_lossy())],
                    )
                }),
                error: error.detail,
                details: false,
                recovery: error.retained,
            }),
        };
    }
}
