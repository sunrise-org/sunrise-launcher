//! What a world on Home can do beyond Play: its "…" menu (data packs, open the folder, copy,
//! rename, delete) and the Add world button. Copying, deleting, renaming and adding go to the
//! shared backend (`sunrise_launcher::worlds`, which the CLI uses too) in the background, and
//! say how it went in a toast.
use super::*;
use crate::frame::Reach as _;
use crate::paint::palette;
use crate::sheets::{modal_frame, title};
use crate::widgets::button::{button, danger, primary, secondary, Look, Size, Style};
use gpui::{
    rgb, Bounds, ClickEvent, KeyDownEvent, MouseButton, MouseDownEvent, PathPromptOptions, Pixels,
};
use std::path::PathBuf;
use sunrise_launcher::worlds::{self, World};

/// What is open over the Worlds tab.
pub(crate) enum WorldAction {
    /// The "…" menu of a world, hung from its button (window coordinates).
    Menu {
        world: World,
        button: Bounds<Pixels>,
    },
    /// Delete's question.
    Confirm(World),
    /// The new name's field (its text is `Worlds::rename`).
    Rename { world: World, focus: FocusHandle },
}

impl Worlds {
    /// Shuts the "…" menu if it is up; a question or the name field stays.
    pub(crate) fn close_menu(&mut self) {
        if matches!(self.action, Some(WorldAction::Menu { .. })) {
            self.action = None;
        }
    }
}

/// A 32 px menu row, as the sidebar profile menu draws its own (`profile_delete/menu.rs`): a
/// 16 px icon and its label, filled on hover. `off` (its reason, "" for none) leaves it dimmed
/// and dead.
fn menu_row(
    id: &'static str,
    icon: &'static str,
    label: SharedString,
    ink: u32,
    off: Option<SharedString>,
    go: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let row = div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(tokens::CONTROL))
        .px(px(8.))
        .rounded(tokens::r(tokens::R_SMALL))
        .text_size(px(tokens::LABEL))
        .text_color(rgb(ink))
        .child(crate::glass_ui::icon(
            icon,
            16.,
            crate::widgets::focus::c(ink),
        ))
        .child(label);
    match off {
        Some(why) => row
            .opacity(0.45)
            .cursor_default()
            .when(!why.is_empty(), |d| d.tooltip(crate::glass_ui::tip(why)))
            .into_any_element(),
        None => row
            .reach()
            .cursor_pointer()
            .hover(|s| s.bg(rgb(palette().hover)))
            .on_click(go)
            .into_any_element(),
    }
}

impl Launcher {
    /// Why a world can't be copied, renamed or deleted now: the game holds its profile's worlds.
    fn world_lock(&self) -> Option<SharedString> {
        (self.client.is_some()
            && self.worlds.home.is_some()
            && self.client_home.as_ref() == self.worlds.home.as_ref())
        .then(|| tr("Stop the game before changing the worlds of the profile it is using.").into())
    }

    /// The profile whose worlds the tab shows.
    fn world_profile(&self) -> Option<instances::Instance> {
        self.selected_instance()
            .filter(|i| Some(&i.home) == self.worlds.home.as_ref())
            .cloned()
    }

    fn close_world_action(&mut self) {
        self.worlds.action = None;
    }

    /// Opens the "…" menu of `world`, hung from `button`.
    pub(crate) fn open_world_menu(&mut self, world: &World, button: Bounds<Pixels>) {
        self.close_menus();
        self.worlds.action = Some(WorldAction::Menu {
            world: world.clone(),
            button,
        });
    }

    /// Whether the "…" menu of `folder` is up.
    pub(crate) fn world_menu_open(&self, folder: &str) -> bool {
        matches!(&self.worlds.action, Some(WorldAction::Menu { world, .. }) if world.folder == folder)
    }

    /// The "Add world" button: a compact secondary with a plus.
    pub(crate) fn add_world_button(
        &self,
        id: &'static str,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        button(
            id,
            tr("Add world"),
            Look::new(Style::Secondary, Size::Medium).icon("plus-sign"),
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.add_world(cx);
        }))
    }

    /// The layer over the page for the open menu or dialog, if any.
    pub(crate) fn world_action_sheet(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        match self.worlds.action.as_ref()? {
            WorldAction::Menu { world, button } => Some(self.world_menu(world, *button, cx)),
            WorldAction::Confirm(world) => Some(self.world_delete_sheet(world, cx)),
            WorldAction::Rename { world, focus } => {
                Some(self.world_rename_sheet(world, focus, window, cx))
            }
        }
    }

    /// The menu's rows, the divider and Delete last.
    fn world_menu_rows(&self, world: &World, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = palette();
        let locked = self.world_lock();
        let off = |on: bool| on.then(|| locked.clone().unwrap_or_default());
        let mut rows: Vec<AnyElement> = Vec::new();
        let (packs, folder, copied, renamed, doomed) = (
            world.clone(),
            world.path.clone(),
            world.clone(),
            world.clone(),
            world.clone(),
        );
        rows.push(menu_row(
            "world-menu-packs",
            "package-01",
            tr("Data packs").into(),
            p.text,
            None,
            cx.listener(move |this, _, window, cx| {
                this.worlds.action = None;
                this.ui.home.tabs.remember(window.focused(cx));
                this.show_world_packs(packs.path.clone());
                cx.notify();
            }),
        ));
        rows.push(menu_row(
            "world-menu-folder",
            "folder-01",
            tr("Open folder").into(),
            p.text,
            None,
            cx.listener(move |this, _, _, cx| {
                this.worlds.action = None;
                crate::toast::open(cx, &folder);
                cx.notify();
            }),
        ));
        rows.push(menu_row(
            "world-menu-copy",
            "copy-01",
            tr("Copy").into(),
            p.text,
            off(locked.is_some()),
            cx.listener(move |this, _, _, cx| {
                this.worlds.action = None;
                this.copy_world(copied.clone(), cx);
                cx.notify();
            }),
        ));
        rows.push(menu_row(
            "world-menu-rename",
            "pencil-edit-02",
            tr("Rename…").into(),
            p.text,
            off(locked.is_some()),
            cx.listener(move |this, _, window, cx| {
                this.start_world_rename(renamed.clone(), window, cx);
                cx.notify();
            }),
        ));
        rows.push(
            div()
                .h(px(1.))
                .my(px(4.))
                .bg(rgb(p.divider))
                .into_any_element(),
        );
        rows.push(menu_row(
            "world-menu-delete",
            "delete-02",
            tr("Delete…").into(),
            p.danger,
            off(locked.is_some()),
            cx.listener(move |this, _, _, cx| {
                this.worlds.action = Some(WorldAction::Confirm(doomed.clone()));
                cx.notify();
            }),
        ));
        rows
    }

    fn world_menu(
        &self,
        world: &World,
        button: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        const WIDTH: f32 = 200.;
        const EDGE: f32 = 8.;
        let p = palette();
        let rows = self.world_menu_rows(world, cx);
        // Five rows, the divider (9), 8 padding each side and the 1 px border.
        let height = 5. * tokens::CONTROL + 9. + 18.;
        let left = (f32::from(button.right()) - WIDTH)
            .min(self.window_width - WIDTH - EDGE)
            .max(EDGE);
        let below = f32::from(button.bottom()) + 4.;
        let top = if below + height + EDGE <= self.window_height {
            below
        } else {
            (f32::from(button.top()) - 4. - height).max(EDGE)
        };
        div()
            .id("world-menu")
            .absolute()
            .inset_0()
            .occlude()
            .on_scroll(|_, _, cx| cx.stop_propagation())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.worlds.action = None;
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.worlds.action = None;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .absolute()
                    .left(px(left))
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
                    .children(rows),
            )
    }

    fn world_delete_sheet(&self, world: &World, cx: &mut Context<Self>) -> Stateful<Div> {
        let doomed = world.clone();
        let body =
            div()
                .flex()
                .flex_col()
                .child(title(trf("Delete {name}?", &[("name", &world.name)])))
                .child(div().text_color(rgb(palette().muted)).child(tr(
                    "This world will move to the Trash. You can restore it from there.",
                )))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(secondary("world-delete-cancel", tr("Cancel")).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.close_world_action();
                                cx.notify();
                            }),
                        ))
                        .child(danger("world-delete-confirm", tr("Delete world")).on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.worlds.action = None;
                                this.delete_world(doomed.clone(), cx);
                                cx.notify();
                            }),
                        )),
                );
        modal_frame(
            "world-delete-sheet",
            450.,
            body,
            cx,
            Self::close_world_action,
        )
    }

    fn world_rename_sheet(
        &self,
        world: &World,
        focus: &FocusHandle,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let focused = focus.is_focused(window);
        let field = crate::widgets::field::field_box(focused, false)
            .id("world-rename-field")
            .track_focus(focus)
            .flex_1()
            .min_w(px(0.))
            .h(px(tokens::FIELD))
            .px(px(12.))
            .text_size(px(tokens::BODY))
            .text_color(rgb(palette().text))
            .cursor_text()
            .child(crate::line_edit::content(
                &self.worlds.rename,
                &self.theme,
                focused,
                false,
                false,
                "World name",
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                if let Some(WorldAction::Rename { focus, .. }) = &this.worlds.action {
                    window.focus(focus, cx);
                }
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                match crate::line_edit::handle(&mut this.worlds.rename, ev, cx) {
                    crate::line_edit::Key::Pass => return,
                    crate::line_edit::Key::Enter => this.apply_world_rename(cx),
                    crate::line_edit::Key::Escape => this.worlds.action = None,
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        let field = crate::line_edit::mouse(field, cx, |this| &mut this.worlds.rename);
        let body =
            div()
                .flex()
                .flex_col()
                .child(title(trf("Rename {name}", &[("name", &world.name)])))
                .child(div().flex().child(field))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(secondary("world-rename-cancel", tr("Cancel")).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.close_world_action();
                                cx.notify();
                            }),
                        ))
                        .child(primary("world-rename-confirm", tr("Rename")).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.apply_world_rename(cx);
                                cx.notify();
                            }),
                        )),
                );
        modal_frame(
            "world-rename-sheet",
            450.,
            body,
            cx,
            Self::close_world_action,
        )
    }

    fn start_world_rename(&mut self, world: World, window: &mut Window, cx: &mut Context<Self>) {
        let focus = cx.focus_handle().tab_index(0).tab_stop(true);
        self.worlds.rename.set(world.name.clone());
        self.worlds.rename.select_all();
        window.focus(&focus, cx);
        self.worlds.action = Some(WorldAction::Rename { world, focus });
    }

    /// Gives the world the typed name (its level.dat's `LevelName`; the folder keeps its name).
    fn apply_world_rename(&mut self, cx: &mut Context<Self>) {
        let Some(WorldAction::Rename { world, .. }) = &self.worlds.action else {
            return;
        };
        let name = self.worlds.rename.text().trim().to_owned();
        if name.is_empty() {
            return;
        }
        let folder = world.folder.clone();
        self.worlds.action = None;
        let Some(instance) = self.world_profile() else {
            return;
        };
        self.world_job(
            cx,
            move || worlds::rename(&instance, &folder, &name),
            |renamed| trf("Renamed to {world}", &[("world", &renamed.name)]),
        );
    }

    /// Copies the world beside itself; it can be large, so it runs in the background.
    fn copy_world(&mut self, world: World, cx: &mut Context<Self>) {
        let Some(instance) = self.world_profile() else {
            return;
        };
        crate::toast::say(trf("Copying {world}…", &[("world", &world.name)]));
        let name = world.name.clone();
        self.world_job(
            cx,
            move || worlds::copy(&instance, &world.folder),
            move |copy| {
                trf(
                    "Copied {world} as {copy}",
                    &[("world", &name), ("copy", &copy.name)],
                )
            },
        );
    }

    /// Moves the world to the system's trash.
    fn delete_world(&mut self, world: World, cx: &mut Context<Self>) {
        let Some(instance) = self.world_profile() else {
            return;
        };
        self.world_job(
            cx,
            move || worlds::delete(&instance, &world.folder),
            |gone| trf("{world} is in the Trash", &[("world", &gone.name)]),
        );
    }

    /// Asks for a world folder or a zip of one, and adds it to the profile.
    pub(crate) fn add_world(&mut self, cx: &mut Context<Self>) {
        let Some(instance) = self.world_profile() else {
            return;
        };
        let pick = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: false,
            prompt: Some(tr("Add world").into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = pick.await {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update(cx, |this, cx| this.import_world(instance, path, cx));
                }
            }
        })
        .detach();
    }

    fn import_world(
        &mut self,
        instance: instances::Instance,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        // The window may have moved to another profile while the picker was up.
        if self
            .world_profile()
            .is_none_or(|now| now.home != instance.home)
        {
            return;
        }
        crate::toast::say(tr("Adding the world…"));
        self.world_job(
            cx,
            move || worlds::import(&instance, &path),
            |added| trf("Added {world}", &[("world", &added.name)]),
        );
    }

    /// Runs `job` off the window's thread, then says what `said` makes of its result (or the
    /// error) in a toast and reads the list again.
    fn world_job<T: Send + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        job: impl FnOnce() -> Result<T, String> + Send + 'static,
        said: impl FnOnce(T) -> String + 'static,
    ) {
        let task = cx.background_executor().spawn(async move { job() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                crate::toast::say(match result {
                    Ok(done) => said(done),
                    Err(error) => error,
                });
                this.worlds.stale();
                cx.notify();
            });
        })
        .detach();
    }
}
