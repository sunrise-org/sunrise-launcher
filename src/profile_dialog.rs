//! Xmilon's preview and creation form, scoped to Sunrise profiles. Saving uses the same
//! instance creation/copy transaction as the launcher; the dialog doesn't write game files.
use super::*;
use crate::widgets::button::{primary, secondary};
use crate::{instances_page::InstanceDialog, paint::palette, servers_page::Field};
use gpui::rgb;
use instances::{Channel, Instance};

mod copy_box;
mod menus;
mod side;

/// The new-profile dialog's height with the copy box open.
const STEADY_HEIGHT: f32 = 516.;

impl Launcher {
    pub(super) fn new_profile(&mut self, cx: &mut Context<Self>) {
        self.servers.text(Field::InstanceName).set(String::new());
        self.servers.text(Field::InstanceGroup).set(String::new());
        self.focus_handle(Field::InstanceName, cx);
        self.servers.focus_soon = Some(Field::InstanceName);
        self.instances.dialog = Some(InstanceDialog {
            editing: None,
            copy: None,
            icon: None,
            picture: None,
            picture_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            error: None,
            busy: false,
            pin: None,
            channel: if self.stable_available() {
                Channel::Stable
            } else {
                Channel::Nightly
            },
            discarding: false,
            menu: None,
            pinning: false,
        });
        cx.notify();
    }

    /// The same dialog for a profile that exists: its name, group and icon. Its build stays
    /// with Home's picker beside Play, so the dialog keeps the saved one untouched.
    pub(super) fn edit_profile(&mut self, target: &Instance, cx: &mut Context<Self>) {
        self.servers
            .text(Field::InstanceName)
            .set(target.name.clone());
        self.servers
            .text(Field::InstanceGroup)
            .set(target.group.clone().unwrap_or_default());
        self.focus_handle(Field::InstanceName, cx);
        self.servers.focus_soon = Some(Field::InstanceName);
        self.instances.dialog = Some(InstanceDialog {
            editing: Some(target.id.clone()),
            copy: None,
            icon: target.icon.clone(),
            picture: None,
            picture_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            error: None,
            busy: false,
            pin: target.version.clone(),
            channel: target.channel,
            discarding: false,
            menu: None,
            pinning: false,
        });
        cx.notify();
    }

    pub(super) fn profile_dialog(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        let d = self.instances.dialog.as_ref()?;
        if d.discarding {
            return Some(self.discard_profile_sheet(cx));
        }
        let (busy, copying, editing, error) = (
            d.busy,
            d.copy.is_some(),
            d.editing.is_some(),
            d.error.clone(),
        );
        let p = palette();
        let name = self.servers.value(Field::InstanceName).trim().to_owned();
        // An edit saves only once something in it changed, as its Save says.
        let ready = !name.is_empty() && !busy && (!editing || self.sheet_changed());
        let title = if editing {
            tr("Edit profile")
        } else if copying {
            tr("Duplicate profile")
        } else {
            tr("New profile")
        };
        let side = self.profile_side(window, &name, cx);
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .px(px(24.))
            .child(self.profile_name_row(cx));
        if !editing {
            // Game build and Start from side by side, each half the form.
            body = body.child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(self.profile_build_row(cx).flex_1().min_w_0())
                    .child(self.profile_start_row(cx).flex_1().min_w_0()),
            );
            body = body.children(self.copy_box(cx));
        }
        // A new profile or a copy keeps one height whatever is chosen: tall enough for the copy
        // box, so the dialog doesn't jump as the choices change. The footer stays at the bottom.
        let right = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .when(!editing, |d| d.min_h(px(STEADY_HEIGHT)))
            .child(
                div()
                    .p(px(24.))
                    .pb(px(16.))
                    .text_size(px(tokens::HEADING))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(body)
            .children(error.map(|e| {
                div()
                    .px(px(24.))
                    .pt(px(12.))
                    .text_color(rgb(p.danger))
                    .child(e)
            }))
            .child(div().flex_1())
            .child(self.profile_dialog_foot(editing, busy, ready, cx));
        let plate = div().flex().child(side).child(right);
        Some(crate::sheets::modal_plate(
            "new-profile",
            720.,
            plate,
            cx,
            Launcher::close_instance_dialog,
            0.,
            p.background,
        ))
    }

    /// Name over Group, side by side: three parts to two.
    fn profile_name_row(&mut self, cx: &mut Context<Self>) -> Div {
        let part = |label: SharedString, field: Stateful<Div>, grow: f32| {
            let mut column = div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .min_w_0()
                .flex_1()
                .child(menus::small_label(label))
                .child(field.w_full().min_w_0());
            column.style().flex_grow = Some(grow);
            column
        };
        let name = self.field(Field::InstanceName, tr("Profile name"), 300., cx);
        let group = self.field(Field::InstanceGroup, tr("No group"), 200., cx);
        div()
            .flex()
            .gap(px(12.))
            .child(part(tr("Name").into(), name, 3.))
            .child(part(tr("Group").into(), group, 2.))
    }

    /// The question a close asks once it found edits not saved: Keep editing goes back to the
    /// form, Discard closes it.
    fn discard_profile_sheet(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let body = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(crate::sheets::title(tr("Discard your changes?").into()))
            .child(
                div()
                    .text_color(rgb(palette().muted))
                    .child(tr("The name and icon you changed won't be saved.")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(8.))
                    .pt(px(12.))
                    .child(
                        secondary("new-keep", tr("Keep editing")).on_click(cx.listener(
                            |this, _, _, cx| {
                                if let Some(d) = &mut this.instances.dialog {
                                    d.discarding = false;
                                }
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        crate::widgets::button::danger("new-discard", tr("Discard")).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.instances.dialog = None;
                                cx.notify();
                            }),
                        ),
                    ),
            );
        crate::sheets::modal_frame(
            "new-profile",
            450.,
            body,
            cx,
            Launcher::close_instance_dialog,
        )
    }

    /// A rule, then Cancel and Create (or Save).
    fn profile_dialog_foot(
        &self,
        editing: bool,
        busy: bool,
        ready: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let row = div()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(8.))
            .mt(px(20.))
            .px(px(24.))
            .py(px(14.))
            .border_t_1()
            .border_color(rgb(palette().divider));
        row.child(
            secondary("new-cancel", tr("Cancel")).on_click(cx.listener(|this, _, _, cx| {
                this.close_instance_dialog();
                cx.notify();
            })),
        )
        .child(
            primary(
                "new-save",
                match (editing, busy) {
                    (true, true) => tr("Saving…"),
                    (true, false) => tr("Save"),
                    (false, true) => tr("Creating…"),
                    (false, false) => tr("Create profile"),
                },
            )
            .when(!ready, |d| d.opacity(0.45).tab_stop(false))
            .when(ready, |d| {
                d.on_click(cx.listener(|this, _, _, cx| {
                    this.save_instance(cx);
                    cx.notify();
                }))
            }),
        )
    }
}
