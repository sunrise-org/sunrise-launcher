//! Xmilon's preview and creation form, scoped to Sunrise profiles. Saving uses the same
//! instance creation/copy transaction as the launcher; the dialog doesn't write game files.
use super::*;
use crate::widgets::button::{primary, secondary};
use crate::{frame::Reach, instances_page::InstanceDialog, paint::palette, servers_page::Field};
use gpui::rgb;
use instances::{Channel, Icon, Instance};

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
        });
        cx.notify();
    }

    pub(super) fn profile_dialog(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        let d = self.instances.dialog.as_ref()?;
        let bound = d
            .copy
            .as_ref()
            .and_then(instances::CopyPreview::base)
            .map(|tag| self.version_label(tag));
        let (busy, copy, icon, channel, pin, error) = (
            d.busy,
            d.copy.is_some(),
            d.icon.clone(),
            d.channel,
            d.pin.clone(),
            d.error.clone(),
        );
        let editing = d.editing.is_some();
        let discarding = d.discarding;
        let picture_focus = d.picture_focus.clone();
        let picture_focused = picture_focus.is_focused(window);
        let picture = d
            .picture
            .as_ref()
            .and_then(Option::as_ref)
            .map(|c| c.preview.clone());
        let p = palette();
        let name = self.servers.value(Field::InstanceName).trim().to_owned();
        let tags = self.profile_build_choices(channel);
        // An edit saves only once something in it changed, as its Save says.
        let ready = !name.is_empty() && !busy && (!editing || self.sheet_changed());
        let current = bound
            .clone()
            .or_else(|| {
                self.target_version(pin.as_ref(), channel)
                    .map(|tag| self.version_label(tag))
            })
            .unwrap_or_else(|| {
                match channel {
                    Channel::Stable => tr("Latest stable"),
                    Channel::Nightly => tr("Latest nightly"),
                    Channel::Dev => tr("Latest Dev"),
                }
                .into()
            });
        let preview = div()
            .id("new-picture-preview")
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .w(px(156.))
            .flex_none()
            .p(px(14.))
            .gap(px(12.))
            .rounded(crate::tokens::r(crate::tokens::R_CARD))
            .border_1()
            .border_color(p.input_edge())
            .bg(p.card())
            .when(!busy, |d| {
                d.group("new-picture-preview")
                    .track_focus(&picture_focus)
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(p.hover)).border_color(rgb(p.line)))
                    .focus_visible(|s| s.border_color(rgb(p.focus)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.choose_profile_picture(cx);
                        cx.notify();
                    }))
            })
            .child(if let Some(image) = picture {
                div()
                    .size(px(64.))
                    .rounded(crate::tokens::r(14.))
                    .overflow_hidden()
                    .child(
                        gpui::img(image)
                            .size_full()
                            .object_fit(gpui::ObjectFit::Contain),
                    )
            } else {
                match icon.as_ref() {
                    Some(Icon::Colour(c)) => {
                        let colour =
                            u32::from_str_radix(c.trim_start_matches('#'), 16).unwrap_or(p.accent);
                        let ink = if sunrise_launcher_theme::contrast(colour, p.text)
                            > sunrise_launcher_theme::contrast(colour, p.background)
                        {
                            p.text
                        } else {
                            p.background
                        };
                        div()
                            .size(px(64.))
                            .rounded(crate::tokens::r(14.))
                            .bg(rgb(colour))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(rgb(ink))
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(64. * 0.54))
                            .child(
                                name.chars()
                                    .next()
                                    .unwrap_or('S')
                                    .to_uppercase()
                                    .to_string(),
                            )
                    }
                    _ => crate::mark(64.),
                }
            })
            .child(
                div()
                    .max_w_full()
                    .truncate()
                    .text_size(px(tokens::HEADING))
                    .font_weight(FontWeight::BOLD)
                    .child(if name.is_empty() {
                        tr("New profile").into()
                    } else {
                        name
                    }),
            )
            .child(
                div()
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(p.muted))
                    .child(current),
            )
            .child(
                div()
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(p.accent_ink))
                    .opacity(if picture_focused && !busy { 1. } else { 0. })
                    .group_hover("new-picture-preview", |s| s.opacity(1.))
                    .child(tr("Change picture")),
            );
        let row = || div().flex().items_center().gap(px(8.));
        let mut form = div()
            .flex()
            .flex_col()
            .min_w_0()
            .flex_1()
            .gap(px(10.))
            .child(tr("Name and group"))
            .child(
                row()
                    .child(
                        self.field(Field::InstanceName, tr("Profile name"), 190., cx)
                            .flex_1()
                            .min_w_0(),
                    )
                    .child(
                        self.field(Field::InstanceGroup, tr("No group"), 148., cx)
                            .flex_1()
                            .min_w_0(),
                    ),
            );
        if editing {
            // The build is Home's picker's to change; this dialog keeps the saved one.
        } else if let Some(build) = bound.as_deref() {
            form = form
                .child(tr("Game build"))
                .child(div().text_color(rgb(p.muted)).child(trf(
                    "This copy keeps {build} from the selected profile.",
                    &[("build", build)],
                )));
        } else {
            let mut channels = row().flex_wrap();
            for (id, c, label) in [
                ("new-stable", Channel::Stable, tr("Stable")),
                ("new-nightly", Channel::Nightly, tr("Nightly")),
                ("new-dev", Channel::Dev, tr("Dev")),
            ] {
                // A channel with no build to follow can't be chosen.
                let none = match c {
                    Channel::Stable => !self.stable_available(),
                    Channel::Dev => self.profile_build_choices(Channel::Dev).is_empty(),
                    Channel::Nightly => false,
                };
                let b = secondary(id, label).when(c == channel, |d| {
                    d.bg(rgb(p.selected)).border_color(rgb(p.accent_ink))
                });
                channels = channels.child(if none {
                    b.opacity(0.5).cursor_default().tab_stop(false)
                } else {
                    b.on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(d) = this.instances.dialog.as_mut().filter(|d| !d.busy) {
                            d.channel = c;
                            d.pin = None;
                        }
                        cx.notify();
                    }))
                });
            }
            let hint = match channel {
                Channel::Stable if !self.stable_available() => Some(tr("No Stable build yet.")),
                Channel::Dev if tags.is_empty() => Some(tr("No Dev build yet.")),
                _ => None,
            };
            let mut versions = div()
                .id("new-versions")
                .max_h(px(94.))
                .flex()
                .flex_col()
                .gap(px(3.))
                .overflow_y_scroll()
                .track_scroll(&crate::frame::scrolls("new-versions"));
            for (idx, tag) in std::iter::once(None)
                .chain(tags.into_iter().map(Some))
                .enumerate()
            {
                let label = tag
                    .as_deref()
                    .map(|t| {
                        let name = self.version_label(t);
                        if channel == Channel::Dev {
                            trf(
                                "{build} · {commit}",
                                &[
                                    ("build", &name),
                                    (
                                        "commit",
                                        &sunrise_launcher::nightly::short_commit(
                                            t.rsplit_once('-').map_or(t, |(_, sha)| sha),
                                        ),
                                    ),
                                ],
                            )
                        } else {
                            name
                        }
                    })
                    .unwrap_or_else(|| tr("Follow latest").into());
                versions = versions.child(
                    secondary(format!("new-version-{}", idx), label)
                        .justify_start()
                        .when(pin == tag, |d| {
                            d.bg(rgb(p.selected)).border_color(rgb(p.accent_ink))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(d) = this.instances.dialog.as_mut().filter(|d| !d.busy) {
                                d.pin = tag.clone();
                            }
                            cx.notify();
                        })),
                );
            }
            form = form
                .child(channels)
                .children(hint.map(|hint| {
                    div()
                        .text_size(px(tokens::SMALL))
                        .text_color(rgb(p.muted))
                        .child(hint)
                }))
                .child(versions);
        }
        let mut start = row().when(editing, |d| d.hidden());
        let copy_lock = self.profile_copy_lock();
        for (id, duplicate, label) in [
            ("new-empty", false, tr("Start empty")),
            ("new-copy", true, tr("Copy selected profile")),
        ] {
            let option = secondary(id, label).when(copy == duplicate, |d| {
                d.bg(rgb(p.selected)).border_color(rgb(p.accent_ink))
            });
            start = start.child(match copy_lock.filter(|_| duplicate) {
                Some(why) => option
                    .opacity(0.45)
                    .tab_stop(false)
                    .cursor_default()
                    .tooltip(crate::glass_ui::tip(why)),
                None => option.on_click(cx.listener(move |this, _, _, cx| {
                    let preview = if duplicate {
                        this.selected_instance()
                            .ok_or_else(|| tr("Select a Sunrise profile first.").to_string())
                            .and_then(instances::CopyPreview::read)
                            .map(Some)
                    } else {
                        Ok(None)
                    };
                    if let Some(d) = this.instances.dialog.as_mut().filter(|d| !d.busy) {
                        match preview {
                            Ok(copy) => {
                                d.copy = copy;
                                d.error = None;
                            }
                            Err(error) => d.error = Some(error),
                        }
                    }
                    cx.notify();
                })),
            });
        }
        let mut icons = row().flex_wrap().child(
            secondary("new-mark", tr("Sunrise"))
                .when(icon.is_none(), |d| d.border_color(rgb(p.accent_ink)))
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(d) = this.instances.dialog.as_mut().filter(|d| !d.busy) {
                        d.icon = None;
                        // An explicit Sunrise mark displaces even a picture inherited by a copy.
                        d.picture = Some(None);
                    }
                    cx.notify();
                })),
        );
        for (idx, a) in sunrise_launcher::themes::Accent::ALL.iter().enumerate() {
            let colour = a.rgb();
            let choice = Icon::Colour(format!("#{colour:06x}"));
            let selected = icon.as_ref() == Some(&choice);
            icons = icons.child(
                div()
                    .id(ElementId::named_usize("new-colour", idx))
                    .reach_edge()
                    // Round, and as tall as the Sunrise and Picture buttons on either side: one row, one
                    // height.
                    .size(px(tokens::CONTROL))
                    .rounded_full()
                    .cursor_pointer()
                    .bg(rgb(colour))
                    .border_1()
                    .border_color(p.input_edge())
                    .hover(|s| s.border_color(rgb(p.text)))
                    .when(selected, |d| d.border_2().border_color(rgb(p.text)))
                    .tooltip(crate::glass_ui::tip(a.name()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(d) = this.instances.dialog.as_mut().filter(|d| !d.busy) {
                            d.icon = Some(choice.clone());
                            d.picture = Some(None);
                        }
                        cx.notify();
                    })),
            );
        }
        let body = div()
            .flex()
            .flex_col()
            .child(crate::sheets::title(
                if editing {
                    tr("Edit profile")
                } else {
                    tr("New profile")
                }
                .into(),
            ))
            .child(div().text_color(rgb(p.muted)).child(if editing {
                tr("Rename it, move it to a group or pick another icon.")
            } else if bound.is_some() {
                tr("Name the copy and pick an icon.")
            } else {
                tr("Name it, pick what it plays, then an icon.")
            }))
            .child(
                row()
                    .items_stretch()
                    .child(preview)
                    .child(form.child(start).child(icons)),
            )
            .children(error.map(|e| div().text_color(rgb(p.danger)).child(e)))
            .child(self.profile_dialog_foot(editing, busy, ready, discarding, cx));
        Some(crate::sheets::modal_frame(
            "new-profile",
            680.,
            body,
            cx,
            Launcher::close_instance_dialog,
        ))
    }

    /// Cancel and Create (or Save), or, once a close found edits not saved, the question
    /// whether to drop them: Keep editing goes back to the form, Discard closes it.
    fn profile_dialog_foot(
        &self,
        editing: bool,
        busy: bool,
        ready: bool,
        discarding: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let row = div().flex().items_center().justify_end().gap(px(8.));
        if discarding {
            return row
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(tr("Discard your changes?")),
                )
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
                    crate::settings::danger(secondary("new-discard", tr("Discard"))).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.instances.dialog = None;
                            cx.notify();
                        }),
                    ),
                );
        }
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
                    (false, false) => tr("Create"),
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
