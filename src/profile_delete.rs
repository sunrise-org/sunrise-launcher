//! One confirmation for profile removal, whether opened in Settings or in the sidebar.
use super::*;
use crate::widgets::button::{primary, secondary};
use crate::{
    frame::Reach as _,
    mods::refused::{details_box, details_label},
    paint::palette,
    sheets::{modal_frame, title},
};
use gpui::{rgb, Global, MouseButton, MouseDownEvent};
use instances::{DeleteError, Instance};

/// A native close or Quit must not abandon a folder in the middle of the move.
#[derive(Default)]
struct MoveInFlight {
    quit_requested: bool,
}

impl Global for MoveInFlight {}

pub(super) fn defer_shutdown(cx: &mut App) -> bool {
    if !cx.has_global::<MoveInFlight>() {
        return false;
    }
    cx.global_mut::<MoveInFlight>().quit_requested = true;
    true
}

pub(super) enum Action {
    Menu {
        target: Instance,
        top: f32,
    },
    Confirm(Instance),
    Deleting(String),
    Interrupted(Vec<instances::InterruptedDeletion>),
    Problem {
        heading: String,
        message: Option<String>,
        error: String,
        details: bool,
        recovery: Option<instances::InterruptedDeletion>,
    },
}

impl Launcher {
    pub(super) fn open_profile_menu(&mut self, target: &Instance, top: f32) {
        self.ui.sheets.profile_action = Some(Action::Menu {
            target: target.clone(),
            top,
        });
    }

    pub(super) fn confirm_profile_delete(&mut self, target: &Instance) {
        self.ui.sheets.profile_action = Some(Action::Confirm(target.clone()));
    }

    pub(super) fn close_profile_action(&mut self) {
        match self.ui.sheets.profile_action.take() {
            Some(action @ Action::Deleting(_)) => self.ui.sheets.profile_action = Some(action),
            Some(Action::Interrupted(notices)) => {
                for notice in notices {
                    notice.acknowledge();
                }
            }
            Some(Action::Problem {
                recovery: Some(notice),
                ..
            }) => notice.acknowledge(),
            _ => {}
        }
    }

    fn delete_profile(&mut self, cx: &mut Context<Self>) {
        let Some(Action::Confirm(target)) = &self.ui.sheets.profile_action else {
            return;
        };
        let target = target.clone();
        let (id, name) = (target.id.clone(), target.name.clone());
        self.ui.sheets.profile_action = Some(Action::Deleting(name.clone()));
        cx.set_global(MoveInFlight::default());
        let task = cx
            .background_executor()
            .spawn(async move { instances::trash_confirmed(target) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.finish_profile_delete(&id, &name, result);
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

    fn finish_profile_delete(
        &mut self,
        id: &str,
        name: &str,
        result: Result<Instance, DeleteError>,
    ) {
        self.refresh_instances_list();
        let removed = result.is_ok() || !self.instances.list.iter().any(|i| i.id == id);
        // Downloads also save these settings. Clear the profile in the window's current
        // settings and commit them here, not from a second copy in the Trash worker.
        let failed_save = if removed && instances::clear_deleted_selection(&mut self.settings, id) {
            self.clear_profile_notices();
            self.save_settings();
            self.settings_save_error.clone()
        } else {
            None
        };
        self.ui.sheets.profile_action = match result {
            Ok(gone) => {
                let heading = trf("{name} is in the Trash.", &[("name", &gone.name)]);
                if let Some(error) = failed_save {
                    Some(Action::Problem {
                        heading,
                        message: Some(
                            tr("The new profile selection could not be saved.").to_owned(),
                        ),
                        error,
                        details: false,
                        recovery: None,
                    })
                } else {
                    crate::toast::say(heading);
                    None
                }
            }
            Err(error) => Some(Action::Problem {
                heading: trf("Couldn't move {name} to the Trash", &[("name", name)]),
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

    pub(super) fn profile_action_sheet(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let action = self.ui.sheets.profile_action.as_ref()?;
        let body = div().flex().flex_col();
        let footer = || div().flex().justify_end().gap(px(8.));
        let close = |id, label, cx: &mut Context<Self>| {
            secondary(id, label).on_click(cx.listener(|this, _, _, cx| {
                this.close_profile_action();
                cx.notify();
            }))
        };
        let body = match action {
            Action::Menu { target, top } => return Some(self.profile_actions_menu(target, *top, cx)),
            Action::Confirm(target) => body
                .child(title(trf("Delete {name}?", &[("name", &target.name)])))
                .child(
                    div()
                        .text_color(rgb(palette().muted))
                        .child(tr("This profile and its worlds and mods will move to the Trash. You can restore it from there.")),
                )
                .child(div().text_size(px(tokens::SMALL)).text_color(rgb(palette().muted)).child(crate::selectable::selectable(trf(
                    "If you restore it from the Trash, move the restored folder back to {folder} in your game folder.",
                    &[("folder", &target.work.strip_prefix(backend::game_home()).unwrap_or(&target.work).to_string_lossy())],
                ))))
                .child(
                    footer()
                        .child(close("profile-delete-cancel", tr("Cancel"), cx))
                        .child(
                            primary("profile-delete-confirm", tr("Delete profile"))
                                .on_click(cx.listener(|this, _, _, cx| this.delete_profile(cx))),
                        ),
                ),
            Action::Deleting(name) => {
                body.child(title(trf("Moving {name} to the Trash…", &[("name", name)])))
            }
            Action::Interrupted(notices) => body
                .child(title(tr("Profile deletion was interrupted").into()))
                .children(notices.iter().map(|notice| {
                    crate::selectable::selectable(trf(
                        "The profile was kept in {folder}",
                        &[("folder", &notice.folder().to_string_lossy())],
                    ))
                }))
                .child(footer().child(close("profile-recovery-close", tr("Close"), cx))),
            Action::Problem {
                heading,
                message,
                error,
                details,
                ..
            } => body
                .child(title(heading.clone()))
                .children(message.clone().map(crate::selectable::selectable))
                .children(details.then(|| details_box("profile-delete-error", error.clone())))
                .child(
                    footer()
                        .child(
                            secondary("profile-delete-details", details_label(*details)).on_click(
                                cx.listener(|this, _, _, cx| {
                                    if let Some(Action::Problem { details, .. }) =
                                        &mut this.ui.sheets.profile_action
                                    {
                                        *details = !*details;
                                    }
                                    cx.notify();
                                }),
                            ),
                        )
                        .child(close("profile-delete-close", tr("Close"), cx)),
                ),
        };
        Some(modal_frame(
            "profile-delete-sheet",
            450.,
            body,
            cx,
            Self::close_profile_action,
        ))
    }

    fn profile_actions_menu(
        &self,
        target: &Instance,
        top: f32,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = palette();
        let at = tokens::FRAME_INSET + crate::rail::rail_width(self.rail_open()) + 8.;
        let name = target.name.clone();
        let item = |id: &'static str, icon: &'static str, label: &'static str, ink: u32| {
            div()
                .id(id)
                .reach()
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(tokens::CONTROL))
                .px(px(8.))
                .rounded(tokens::r(tokens::R_SMALL))
                .cursor_pointer()
                .text_size(px(tokens::LABEL))
                .text_color(rgb(ink))
                .hover(|s| s.bg(rgb(p.hover)))
                .child(crate::glass_ui::icon(
                    icon,
                    16.,
                    crate::widgets::focus::c(ink),
                ))
                .child(label)
        };
        let edited = target.clone();
        let edit = item(
            "profile-menu-edit",
            "pencil-edit-02",
            tr("Edit profile…"),
            p.text,
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            this.close_profile_action();
            this.edit_profile(&edited, cx);
        }));
        let target = target.clone();
        let delete = item(
            "profile-menu-delete",
            "delete-02",
            tr("Delete profile…"),
            p.danger,
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            this.confirm_profile_delete(&target);
            cx.notify();
        }));
        // Two rows, the title (26), a divider (9), 8 padding each side and the 1 px border.
        let action_height = 2. * tokens::CONTROL + 26. + 9. + 18.;
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
                    .top(px(top.min(self.window_height - action_height - 8.)))
                    .w(px(236.))
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
                    .child(edit)
                    .child(div().h(px(1.)).my(px(4.)).bg(rgb(p.divider)))
                    .child(delete),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch_child(test: &str) -> Option<PathBuf> {
        const CHILD: &str = "SUNRISE_PROFILE_DELETE_TEST";
        if let Some(root) = std::env::var_os(CHILD) {
            return Some(root.into());
        }
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        let nowhere = "http://127.0.0.1:9";
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &format!("profile_delete::tests::{test}")])
            .env(CHILD, root)
            .env("SUNRISE_HOME", root.join("game"))
            .env("SUNRISE_LAUNCHER_HOME", root.join("launcher"))
            .env("SUNRISE_LAUNCHER_LOCATION_FILE", root.join("location.json"))
            .env("SUNRISE_LAUNCHER_TRASH", root.join("trash"))
            .env("SUNRISE_NO_KEYCHAIN", "1")
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("SUNRISE_TELEMETRY_URL", "off")
            .env("XDG_DATA_HOME", root.join("data"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("HTTP_PROXY", nowhere)
            .env("HTTPS_PROXY", nowhere)
            .env("ALL_PROXY", nowhere)
            .env_remove("NO_PROXY")
            .env_remove("SUNRISE_GITHUB_MIRROR")
            .env_remove("SUNRISE_LAUNCHER_BENCH")
            .status()
            .unwrap();
        assert!(status.success());
        None
    }

    #[test]
    fn a_right_click_on_another_profile_moves_the_open_menu_to_it() {
        if scratch_child("a_right_click_on_another_profile_moves_the_open_menu_to_it").is_none() {
            return;
        }
        let cabin = instances::create("Cabin", None).unwrap();
        let tower = instances::create("Tower", None).unwrap();
        let mut settings = Settings::load();
        settings.instance = Some(cabin.id.clone());
        settings.save().unwrap();
        let mut app = gpui::TestAppContext::single();
        let (shell, cx) = app.add_window_view(crate::window_root::Shell::new);
        let launcher = shell.read_with(cx, |shell, _| shell.launcher());
        cx.run_until_parked();
        let row = |cx: &mut gpui::VisualTestContext, id: &str| {
            launcher.read_with(cx, |l, _| {
                l.ui.profile_rows
                    .borrow()
                    .iter()
                    .find(|(_, profile)| profile.id == id)
                    .map(|(bounds, _)| bounds.center())
                    .unwrap_or_else(|| panic!("{id}'s row is drawn"))
            })
        };
        let menu = |cx: &mut gpui::VisualTestContext| {
            launcher.read_with(cx, |l, _| match &l.ui.sheets.profile_action {
                Some(Action::Menu { target, .. }) => Some(target.id.clone()),
                _ => None,
            })
        };
        let right = |cx: &mut gpui::VisualTestContext, at| {
            cx.simulate_mouse_down(at, MouseButton::Right, gpui::Modifiers::none());
            cx.simulate_mouse_up(at, MouseButton::Right, gpui::Modifiers::none());
            cx.run_until_parked();
        };
        let (cabin_row, tower_row) = (row(cx, &cabin.id), row(cx, &tower.id));
        right(cx, cabin_row);
        assert_eq!(menu(cx), Some(cabin.id.clone()));
        // With Cabin's menu open over the profiles, one right-click moves it to Tower.
        right(cx, tower_row);
        assert_eq!(menu(cx), Some(tower.id.clone()));
        // A right-click away from any profile only closes it.
        right(cx, gpui::point(gpui::px(600.), gpui::px(400.)));
        assert_eq!(menu(cx), None);
    }

    #[test]
    fn a_right_click_outside_the_scrolled_profiles_or_on_the_menu_moves_nothing() {
        if scratch_child("a_right_click_outside_the_scrolled_profiles_or_on_the_menu_moves_nothing")
            .is_none()
        {
            return;
        }
        let first = instances::create("Profile 0", None).unwrap();
        let count = 40;
        for n in 1..count {
            instances::create(&format!("Profile {n}"), None).unwrap();
        }
        let mut settings = Settings::load();
        settings.instance = Some(first.id.clone());
        settings.save().unwrap();
        let mut app = gpui::TestAppContext::single();
        let (shell, cx) = app.add_window_view(crate::window_root::Shell::new);
        let launcher = shell.read_with(cx, |shell, _| shell.launcher());
        cx.run_until_parked();
        let rows = |cx: &mut gpui::VisualTestContext| {
            launcher.read_with(cx, |l, _| {
                l.ui.profile_rows
                    .borrow()
                    .iter()
                    .map(|(bounds, profile)| (*bounds, profile.id.clone()))
                    .collect::<Vec<_>>()
            })
        };
        let menu = |cx: &mut gpui::VisualTestContext| {
            launcher.read_with(cx, |l, _| match &l.ui.sheets.profile_action {
                Some(Action::Menu { target, top }) => Some((target.id.clone(), *top)),
                _ => None,
            })
        };
        let right = |cx: &mut gpui::VisualTestContext, at| {
            cx.simulate_mouse_down(at, MouseButton::Right, gpui::Modifiers::none());
            cx.simulate_mouse_up(at, MouseButton::Right, gpui::Modifiers::none());
            cx.run_until_parked();
        };
        let drawn = rows(cx);
        assert!(
            drawn.len() < count,
            "the list scrolls: {} of {count} rows show",
            drawn.len()
        );
        let (first_row, _) = drawn.iter().find(|(_, id)| *id == first.id).unwrap();
        let (last_row, _) = drawn.last().unwrap();
        right(cx, first_row.center());
        let (_, top) = menu(cx).expect("the first profile's menu opens");
        // Just below the list's end, where the next profile would be if it were not scrolled
        // out of view, a right-click lands on the rail's controls and only closes the menu.
        right(
            cx,
            gpui::point(last_row.center().x, last_row.bottom() + gpui::px(10.)),
        );
        assert_eq!(menu(cx), None);
        // A right-click on the open menu itself leaves it open.
        right(cx, first_row.center());
        let at = launcher.read_with(cx, |l, _| {
            tokens::FRAME_INSET + crate::rail::rail_width(l.rail_open()) + 8.
        });
        right(cx, gpui::point(gpui::px(at + 40.), gpui::px(top + 12.)));
        assert_eq!(menu(cx).map(|(id, _)| id), Some(first.id.clone()));
    }

    #[test]
    fn a_download_save_survives_a_profile_delete() {
        let Some(root) = scratch_child("a_download_save_survives_a_profile_delete") else {
            return;
        };
        let profile = instances::create("To Delete", None).unwrap();
        let mut settings = Settings::load();
        settings.instance = Some(profile.id.clone());
        settings.selected = Some("26.3-0.1.0".into());
        settings.prune_old_versions = Some(false);
        settings.save().unwrap();
        let mut app = gpui::TestAppContext::single();
        let (shell, cx) = app.add_window_view(crate::window_root::Shell::new);
        let launcher = shell.read_with(cx, |shell, _| shell.launcher());
        let window = cx.update(|window, _| window.window_handle());
        launcher.update(cx, |launcher, cx| {
            // Feed the real message handler synchronously rather than race the wake relay.
            (launcher.tx, launcher.rx) = std::sync::mpsc::channel();
            // Freeze the worker's completion until the download handler has saved.
            launcher.ui.sheets.profile_action = Some(Action::Deleting(profile.name.clone()));
            assert!(matches!(
                launcher.ui.sheets.profile_action,
                Some(Action::Deleting(_))
            ));

            launcher
                .tx
                .send(Msg::Installed(
                    "26.3-0.1.1".into(),
                    Ok(backend::InstallOutcome::Installed(None)),
                ))
                .unwrap();
            launcher.drain(window, cx);
            let live = Settings::load();
            assert_eq!(live.selected.as_deref(), Some("26.3-0.1.1"));
            assert_eq!(live.instance.as_ref(), Some(&profile.id));

            let trashed = instances::trash_confirmed(profile.clone());
            assert!(trashed.is_ok(), "{trashed:?}");
            launcher.finish_profile_delete(&profile.id, &profile.name, trashed);
            assert!(launcher.ui.sheets.profile_action.is_none());
        });
        let saved = Settings::load();
        assert!(saved.instance.is_none());
        assert_eq!(saved.selected.as_deref(), Some("26.3-0.1.1"));
        assert!(root.join("trash").join(&profile.id).is_dir());
    }

    #[test]
    fn a_queued_update_waits_for_the_recovery_error_and_settings_retry() {
        let Some(root) =
            scratch_child("a_queued_update_waits_for_the_recovery_error_and_settings_retry")
        else {
            return;
        };
        let profile = instances::create("Cabin", None).unwrap();
        let mut settings = Settings::load();
        settings.instance = Some(profile.id.clone());
        settings.save().unwrap();
        std::fs::write(root.join("trash"), b"not a folder").unwrap();
        let settings_path = root.join("launcher/settings.json");

        let mut app = gpui::TestAppContext::single();
        let (shell, cx) = app.add_window_view(crate::window_root::Shell::new);
        let launcher = shell.read_with(cx, |shell, _| shell.launcher());
        launcher.update(cx, |launcher, cx| {
            launcher.self_update = SelfUpdate::Ready(Release {
                tag: "v1.3.0".into(),
                name: String::new(),
                body: String::new(),
                date: String::new(),
                time: String::new(),
                published_at: String::new(),
                asset: None,
                prerelease: false,
                nightly: None,
            });
            launcher.restart_for_launcher_update = true;
            launcher.ui.sheets.profile_action = Some(Action::Deleting(profile.name.clone()));
            launcher.try_restart_launcher(true, cx, |_| panic!("move is in progress"));

            let trashed = instances::trash_confirmed(profile.clone());
            assert!(trashed
                .as_ref()
                .unwrap_err()
                .retained
                .as_ref()
                .unwrap()
                .folder()
                .is_dir());
            launcher.finish_profile_delete(&profile.id, &profile.name, trashed);
            assert!(matches!(
                launcher.ui.sheets.profile_action,
                Some(Action::Problem { .. })
            ));
            launcher.try_restart_launcher(true, cx, |_| panic!("recovery error is open"));
            assert!(launcher.restart_for_launcher_update);
            assert!(launcher.sheet_escape());
            let mut handed_over = false;
            launcher.try_restart_launcher(true, cx, |_| {
                handed_over = true;
                Ok(())
            });
            assert!(handed_over);
            assert!(!launcher.restart_for_launcher_update);

            // A different move succeeds but the cleared selection cannot be saved yet.
            let profile = instances::create("Summer Cabin", None).unwrap();
            launcher.settings.instance = Some(profile.id.clone());
            std::fs::remove_file(root.join("trash")).unwrap();
            std::fs::remove_file(&settings_path).unwrap();
            std::fs::create_dir(&settings_path).unwrap();
            let trashed = instances::trash_confirmed(profile.clone());
            assert!(trashed.is_ok(), "{trashed:?}");
            launcher.finish_profile_delete(&profile.id, &profile.name, trashed);
            assert!(launcher.settings.instance.is_none());
            launcher.restart_for_launcher_update = true;
            assert!(launcher.sheet_escape());
            assert!(launcher.settings_save_notice(cx).is_some());
            launcher.try_restart_launcher(true, cx, |_| panic!("settings still need Retry"));
            std::fs::remove_dir(&settings_path).unwrap();
            assert!(launcher.save_settings());
            assert!(launcher.settings_save_notice(cx).is_none());
            let mut handed_over = false;
            launcher.try_restart_launcher(true, cx, |_| {
                handed_over = true;
                Ok(())
            });
            assert!(handed_over);
            assert!(Settings::load().instance.is_none());
        });
    }
}
