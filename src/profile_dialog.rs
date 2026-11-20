//! Xmilon's preview and creation form, scoped to Sunrise profiles. Saving uses the same
//! instance creation/copy transaction as the launcher; the dialog doesn't write game files.
use super::*;
use crate::{
    frame::{button, button_base, primary},
    instances_page::InstanceDialog,
    paint::palette,
    servers_page::Field,
};
use gpui::rgb;
use instances::{Channel, Icon};

impl Launcher {
    pub(super) fn new_profile(&mut self, cx: &mut Context<Self>) {
        self.servers.text(Field::InstanceName).set(String::new());
        self.servers.text(Field::InstanceGroup).set(String::new());
        self.focus_handle(Field::InstanceName, cx);
        self.servers.focus_soon = Some(Field::InstanceName);
        let channel = if self.stable_available() {
            Channel::Stable
        } else {
            Channel::Nightly
        };
        self.instances.dialog = Some(InstanceDialog {
            editing: None,
            copy: false,
            icon: None,
            picture: None,
            picture_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            error: None,
            busy: false,
            pin: None,
            channel,
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
        let (busy, copy, icon, channel, pin, error) = (
            d.busy,
            d.copy,
            d.icon.clone(),
            d.channel,
            d.pin.clone(),
            d.error.clone(),
        );
        let picture_focus = d.picture_focus.clone();
        let picture_focused = picture_focus.is_focused(window);
        let picture = d
            .picture
            .as_ref()
            .and_then(Option::as_ref)
            .map(|c| c.preview.clone());
        let p = palette();
        let name = self.servers.value(Field::InstanceName).trim().to_owned();
        let stable_available = self.stable_available();
        let tags = match channel {
            Channel::Stable => self.stable_choices(),
            Channel::Nightly => self.sunrise_choices(),
        };
        let channel_available =
            channel != Channel::Stable || stable_available || d.editing.is_some();
        let ready = !name.is_empty() && !busy && channel_available;
        let preview_build = self.draft_sunrise();
        let current = preview_build
            .as_deref()
            .map(|tag| self.version_label(tag))
            .unwrap_or_else(|| {
                match channel {
                    Channel::Stable => tr("Latest stable"),
                    Channel::Nightly => tr("Latest nightly"),
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
                    Some(Icon::Colour(c)) => div()
                        .size(px(64.))
                        .rounded(crate::tokens::r(14.))
                        .bg(rgb(
                            u32::from_str_radix(c.trim_start_matches('#'), 16).unwrap_or(p.accent)
                        ))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(blocks::icon("package", 34., rgb(p.accent_on).into())),
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
        let mut channels = row();
        for (id, c, label) in [
            ("new-stable", Channel::Stable, tr("Stable")),
            ("new-nightly", Channel::Nightly, tr("Nightly")),
        ] {
            let available = c != Channel::Stable || stable_available;
            let choice = if available {
                button(id, label)
            } else {
                button_base(id, label)
            };
            channels = channels.child(
                choice
                    .when(available && c == channel, |d| {
                        d.bg(rgb(p.selected)).border_color(rgb(p.accent_ink))
                    })
                    .when(!available, |d| {
                        d.opacity(0.5).cursor_default().tab_stop(false)
                    })
                    .when(available, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(d) = this.instances.dialog.as_mut().filter(|d| !d.busy) {
                                d.channel = c;
                                d.pin = None;
                            }
                            cx.notify();
                        }))
                    }),
            );
        }
        if !stable_available {
            channels = channels.child(
                div()
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(p.muted))
                    .child(tr("No stable release yet")),
            );
        }
        let mut versions = div()
            .id("new-versions")
            .max_h(px(94.))
            .flex()
            .flex_col()
            .gap(px(3.))
            .overflow_y_scroll()
            .track_scroll(&crate::frame::scrolls("new-versions"));
        for (idx, tag) in channel_available
            .then_some(None)
            .into_iter()
            .chain(tags.into_iter().map(Some))
            .enumerate()
        {
            let label = tag
                .as_deref()
                .map(|t| self.version_label(t))
                .unwrap_or_else(|| tr("Follow latest").into());
            versions = versions.child(
                button(ElementId::named_usize("new-version", idx), label)
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
        form = form.child(channels).child(versions);
        let mut start = row();
        let copy_lock = self.profile_copy_lock();
        for (id, duplicate, label) in [
            ("new-empty", false, tr("Start empty")),
            ("new-copy", true, tr("Copy selected profile")),
        ] {
            let option = button(id, label).when(copy == duplicate, |d| {
                d.bg(rgb(p.selected)).border_color(rgb(p.accent_ink))
            });
            start = start.child(match copy_lock.filter(|_| duplicate) {
                Some(why) => option
                    .opacity(0.45)
                    .tab_stop(false)
                    .cursor_default()
                    .tooltip(crate::glass_ui::tip(why)),
                None => option.on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(d) = this.instances.dialog.as_mut().filter(|d| !d.busy) {
                        d.copy = duplicate;
                    }
                    cx.notify();
                })),
            });
        }
        let mut icons = row().flex_wrap().child(
            button("new-mark", tr("Sunrise"))
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
            icons = icons.child(
                button(ElementId::named_usize("new-colour", idx), "")
                    // Round, and as tall as the Sunrise and Picture buttons on either side: one row, one
                    // height.
                    .px_0()
                    .w(px(tokens::CONTROL))
                    .h(px(tokens::CONTROL))
                    .rounded_full()
                    .bg(rgb(colour))
                    .when(icon.as_ref() == Some(&choice), |d| {
                        d.border_2().border_color(rgb(p.text))
                    })
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
            .child(crate::sheets::title(tr("New profile").into()))
            .child(
                div()
                    .text_color(rgb(p.muted))
                    .child(tr("Name it, pick what it plays, then an icon.")),
            )
            .child(
                row()
                    .items_stretch()
                    .child(preview)
                    .child(form.child(start).child(icons)),
            )
            .children(error.map(|e| div().text_color(rgb(p.danger)).child(e)))
            .child(
                row()
                    .justify_end()
                    .child(button("new-cancel", tr("Cancel")).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.close_instance_dialog();
                            cx.notify();
                        },
                    )))
                    .child(
                        primary(
                            "new-save",
                            if busy {
                                tr("Creating…")
                            } else {
                                tr("Create")
                            },
                        )
                        .when(!ready, |d| d.opacity(0.45).tab_stop(false))
                        .when(ready, |d| {
                            d.on_click(cx.listener(|this, _, _, cx| {
                                this.save_instance(cx);
                                cx.notify();
                            }))
                        }),
                    ),
            );
        Some(crate::sheets::modal_frame(
            "new-profile",
            680.,
            body,
            cx,
            Launcher::close_instance_dialog,
        ))
    }
}
