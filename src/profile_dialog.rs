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
            naming_group: false,
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
            error: None,
            busy: false,
            pin: target.version.clone(),
            channel: target.channel,
            discarding: false,
            menu: None,
            pinning: false,
            naming_group: false,
        });
        cx.notify();
    }

    pub(super) fn profile_dialog(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
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
        let side = self.profile_side(&name, cx);
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
        let naming = self
            .instances
            .dialog
            .as_ref()
            .is_some_and(|d| d.naming_group);
        let popup = naming.then(|| self.new_group_popup(cx));
        let plate = div()
            .relative()
            .flex()
            .child(side)
            .child(right)
            .children(popup);
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
        let mut group = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .min_w_0()
            .flex_1()
            .child(menus::small_label(tr("Group").into()))
            .child(self.profile_group_picker(cx));
        group.style().flex_grow = Some(2.);
        div()
            .flex()
            .gap(px(12.))
            .child(part(tr("Name").into(), name, 3.))
            .child(group)
    }

    /// Group: a dropdown of the groups there are, and No group. Typing a name that is no group
    /// yet offers to make it, so a new group needs no other place.
    fn profile_group_picker(&mut self, cx: &mut Context<Self>) -> Div {
        use crate::widgets::dropdown::{Dropdown, Item};
        let now = self.servers.value(Field::InstanceGroup).trim().to_string();
        let open = self
            .instances
            .dialog
            .as_ref()
            .is_some_and(|d| d.menu == Some(crate::instances_page::DialogMenu::Group));
        let mut groups: Vec<String> = self
            .instances
            .list
            .iter()
            .filter_map(|i| i.group.clone())
            .chain((!now.is_empty()).then(|| now.clone()))
            .collect();
        groups.sort_by_key(|g| g.to_lowercase());
        groups.dedup();
        let mut items = vec![Item::new(tr("No group"))
            .current(now.is_empty())
            .rule_after()];
        items.extend(
            groups
                .iter()
                .map(|g| Item::new(g.clone()).current(*g == now)),
        );
        let set = |this: &mut Launcher, value: String| {
            this.servers.text(Field::InstanceGroup).set(value);
            if let Some(d) = this.instances.dialog.as_mut() {
                d.menu = None;
            }
        };
        Dropdown::new("new-group", items, open)
            .fill()
            .on_toggle({
                let this = cx.entity();
                move |_, cx| {
                    this.update(cx, |this, cx| {
                        let was = open;
                        this.close_menus();
                        if let Some(d) = this.instances.dialog.as_mut().filter(|d| !d.busy) {
                            d.menu = (!was).then_some(crate::instances_page::DialogMenu::Group);
                        }
                        cx.notify();
                    })
                }
            })
            .on_close({
                let this = cx.entity();
                move |_, cx| {
                    this.update(cx, |this, cx| {
                        if let Some(d) = this.instances.dialog.as_mut() {
                            d.menu = None;
                        }
                        cx.notify();
                    })
                }
            })
            .on_pick({
                let this = cx.entity();
                move |n, _, cx| {
                    let value = if n == 0 {
                        String::new()
                    } else {
                        groups.get(n - 1).cloned().unwrap_or_default()
                    };
                    this.update(cx, |this, cx| {
                        set(this, value.clone());
                        cx.notify();
                    })
                }
            })
            .on_create(
                |name| trf("New group “{name}”", &[("name", name)]),
                crate::i18n::tr_key("New group…"),
                crate::i18n::tr_key("Type the new group's name"),
                {
                    let this = cx.entity();
                    move |name, _, cx| {
                        this.update(cx, |this, cx| {
                            set(this, name.clone());
                            cx.notify();
                        })
                    }
                },
            )
            .on_offer({
                let this = cx.entity();
                move |_, cx| {
                    this.update(cx, |this, cx| {
                        this.open_new_group(cx);
                        cx.notify();
                    })
                }
            })
            .build()
    }

    /// Opens the "New group" popup over the dialog, its name field empty and focused.
    fn open_new_group(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.instances.dialog.as_mut() else {
            return;
        };
        d.menu = None;
        d.naming_group = true;
        self.servers.text(Field::NewGroup).set(String::new());
        self.focus_handle(Field::NewGroup, cx);
        self.servers.focus_soon = Some(Field::NewGroup);
    }

    pub(super) fn close_new_group(&mut self) {
        if let Some(d) = self.instances.dialog.as_mut() {
            d.naming_group = false;
        }
        self.servers.focus_soon = Some(Field::InstanceName);
    }

    /// Create in the popup: the dialog's group becomes the name typed.
    pub(super) fn commit_new_group(&mut self) {
        let name = self.servers.value(Field::NewGroup).trim().to_string();
        if name.is_empty() {
            return;
        }
        self.servers.text(Field::InstanceGroup).set(name);
        self.close_new_group();
    }

    /// The popup laid over the dialog while a new group is named: a dim layer, and a small card
    /// with the name field and Cancel / Create.
    fn new_group_popup(&mut self, cx: &mut Context<Self>) -> Div {
        let p = palette();
        let field = self.field(Field::NewGroup, tr("Group name"), 260., cx);
        let ready = !self.servers.value(Field::NewGroup).trim().is_empty();
        let card = div()
            .id("new-group-popup")
            .w(px(320.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(20.))
            .rounded(tokens::r(tokens::R_PANEL))
            .border_1()
            .border_color(rgb(p.line))
            .bg(rgb(p.raised))
            .shadow_lg()
            .occlude()
            .child(
                div()
                    .text_size(px(tokens::HEADING))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(tr("New group")),
            )
            .child(field.w_full())
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        secondary("new-group-cancel", tr("Cancel")).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.close_new_group();
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        primary("new-group-create", tr("Create group"))
                            .when(!ready, |d| d.opacity(0.5))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.commit_new_group();
                                cx.notify();
                            })),
                    ),
            );
        div()
            .absolute()
            .inset_0()
            .rounded(tokens::r(tokens::R_PANEL))
            .bg(gpui::Hsla::from(rgb(0x000000)).opacity(0.35))
            .flex()
            .items_center()
            .justify_center()
            .child(card)
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
