//! The sidebar's profiles: the heading with its + button, the list (each profile's mark and
//! name, its actions button, its context menu), and the switch between them.
use super::*;
use crate::widgets::button::{icon_button, Style};
use crate::widgets::focus::Reach;
use gpui::{rgb, MouseButton, MouseDownEvent};
use instances::{Icon, Instance, Kind};

/// What the profiles are drawn with: how open the sidebar is, why profiles are locked (if they
/// are), and whether the page is Home, where the selected profile is shown as selected.
#[derive(Clone, Copy)]
pub(super) struct Look {
    pub open: f32,
    pub lock: Option<&'static str>,
    pub on_home: bool,
}

/// A profile group's heading over its first profile: the group's name, or with the names hidden a
/// short rule that still marks where a group begins.
fn profile_heading(open: f32, group: String) -> Div {
    let p = palette();
    let labelled = open > 0.;
    div()
        .flex_none()
        .h(px(22.))
        .mt(px(6.))
        .px(px(9.))
        .flex()
        .items_end()
        .truncate()
        .text_size(px(crate::tokens::SMALL))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(p.chrome_muted))
        .opacity(label_opacity(open))
        .when(!labelled, |d| {
            d.h(px(1.))
                .mt(px(4.))
                .mx(px(8.))
                .bg(rgb(p.chrome_muted))
                .opacity(0.5)
        })
        .when(labelled, |d| d.child(group))
}

/// A profile's name, fading in as the sidebar opens.
/// A profile row's height: its name over its channel.
const PROFILE_ROW: f32 = 48.;

fn label_text(open: f32, name: String, under: Option<&'static str>) -> Div {
    let line = |text: String| {
        div()
            .overflow_hidden()
            .whitespace_nowrap()
            // An ellipsis would flicker along a label still being uncovered.
            .when(open >= 1., |d| d.text_ellipsis())
            .child(text)
    };
    div()
        .min_w_0()
        .flex_1()
        .flex()
        .flex_col()
        .opacity(label_opacity(open))
        .child(line(name).text_size(px(tokens::LABEL)))
        // The profile's channel under its name (Figma: "Nightly · 3 mods"; the count is known
        // only for the open profile, so the line names the channel).
        .children(under.map(|under| {
            line(under.to_owned())
                .text_size(px(tokens::CAPTION))
                .text_color(rgb(palette().muted))
        }))
}

/// A profile's row: its selected fill (no accent edge), its hover while it can be picked, and
/// its tooltip when its name is hidden or it is locked (then with the reason).
fn profile_button(id: ElementId, name: String, on: bool, look: Look) -> Stateful<Div> {
    let p = palette();
    let labelled = look.open > 0.;
    div()
        .id(id)
        .reach_edge()
        .h(px(PROFILE_ROW))
        .flex_none()
        .w_full()
        // Open, the selected fill reaches a little past the mark on its left.
        .pl(px(6. * look.open))
        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
        .flex()
        .items_center()
        .gap(px(4.))
        .pr(px(8. * look.open))
        .when(on, |d| d.bg(rgb(p.selected)))
        .when(look.lock.is_none(), |d| {
            d.cursor_pointer()
                .when(!on, |d| d.hover(|d| d.bg(rgb(p.chrome_raised))))
        })
        .when(look.lock.is_some(), |d| d.opacity(0.45).tab_stop(false))
        .when(!labelled || look.lock.is_some(), |d| {
            d.tooltip(crate::glass_ui::tip(
                look.lock.map_or(name.clone(), str::to_owned),
            ))
        })
}

/// The actions button of the selected profile: under its mark while the sidebar is closed, and
/// beside its name once open. It opens the profile's menu where it is.
fn profile_actions(i: &Instance, open: f32, cx: &mut Context<Launcher>) -> Option<Stateful<Div>> {
    let at = actions_at(open)?;
    let p = palette();
    let i = i.clone();
    Some(
        div()
            .id(SharedString::from(format!("profile-actions-{}", i.id)))
            .debug_selector(|| "selected-profile-actions".into())
            .reach()
            .when(at == ActionsAt::Beside, |d| {
                d.absolute()
                    .top(px((PROFILE_ROW - 28.) / 2.))
                    .right(px(6.))
                    .w(px(28.))
                    .opacity(label_opacity(open))
            })
            .h(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
            .hover(|d| d.bg(rgb(p.hover)))
            .tooltip(crate::glass_ui::tip(tr("Profile settings")))
            .child(
                div().flex().gap(px(2.)).children(
                    (0..3).map(|_| div().size(px(3.)).rounded_full().bg(rgb(p.chrome_text))),
                ),
            )
            // The profile's settings (its edit dialog); right-clicking the row still opens its
            // menu, with Delete.
            .on_click(cx.listener(move |this, _, _, cx| {
                this.edit_profile(&i, cx);
                cx.notify();
            })),
    )
}

/// A profile's right-click target: where it was last drawn (for the menu that opens over it),
/// and the menu it opens. Only the part of the row the list shows counts.
fn menu_target(
    button: Stateful<Div>,
    i: &Instance,
    rows: ProfileRows,
    cx: &mut Context<Launcher>,
) -> Stateful<Div> {
    let (drawn, menu) = (i.clone(), i.clone());
    button
        .child(
            gpui::canvas(
                move |bounds, window, _| {
                    let visible = bounds.intersect(&window.content_mask().bounds);
                    if !visible.is_empty() {
                        rows.borrow_mut().push((visible, drawn.clone()));
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                this.open_profile_menu(&menu, f32::from(ev.position.y));
                cx.notify();
            }),
        )
}

impl Launcher {
    pub(crate) fn profile_lock(&self) -> Option<&'static str> {
        profile_lock_reason(
            &self.launch_activity(),
            !self.location_unavailable,
            self.ui.after_sign_in.is_some() || self.ui.after_install.is_some(),
        )
    }

    pub(crate) fn switch_profile(&mut self, instance: &Instance, cx: &mut Context<Self>) {
        if self.profile_lock().is_some() {
            return;
        }
        if !self
            .selected_instance()
            .is_some_and(|i| i.id == instance.id)
        {
            self.select_instance(instance);
            self.clear_profile_notices();
        }
        self.navigate(Page::Home);
        cx.notify();
    }

    pub(crate) fn clear_profile_notices(&mut self) {
        self.client_error = None;
        self.prepare_error = None;
        self.install_error = None;
        self.crash = None;
    }

    pub(crate) fn profile_mark(&mut self, i: &Instance, size: f32) -> Div {
        let p = palette();
        let picture = self.icon_image(format!("picture:{}", i.id), size, || {
            Some(i.home.join(instances::PICTURE))
        });
        let texture = match &i.icon {
            Some(icon @ Icon::Texture(name)) if picture.is_none() => {
                self.version_of(i).cloned().and_then(|tag| {
                    self.icon_image(format!("texture:{tag}:{name}"), size, || {
                        instances::icon_texture(icon, &backend::resources(&tag))
                    })
                })
            }
            _ => None,
        };
        let tile = div()
            .size(px(size))
            .flex_none()
            .rounded(px(size * 0.22))
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center();
        if let Some(image) = picture.or(texture) {
            return tile.child(
                gpui::img(image)
                    .size_full()
                    .object_fit(gpui::ObjectFit::Contain),
            );
        }
        match &i.icon {
            Some(Icon::Colour(hex)) => {
                let colour =
                    u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(p.selected);
                let ink = if sunrise_launcher_theme::contrast(colour, p.text)
                    > sunrise_launcher_theme::contrast(colour, p.background)
                {
                    p.text
                } else {
                    p.background
                };
                tile.bg(rgb(colour))
                    .text_color(rgb(ink))
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(size * 0.54))
                    .child(
                        i.name
                            .chars()
                            .next()
                            .unwrap_or('S')
                            .to_uppercase()
                            .to_string(),
                    )
            }
            _ => crate::mark(size),
        }
    }

    /// The profiles section: the heading, then the list, which takes the room the sidebar has.
    pub(super) fn profiles_section(&mut self, look: Look, cx: &mut Context<Self>) -> Div {
        let heading = self.profiles_heading(look, cx);
        let list = self.profile_list(look, cx);
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(px(4.))
            .child(heading)
            .child(list)
    }

    /// "Profiles", and the + button that makes a new one. Locked, the button says why.
    fn profiles_heading(&mut self, look: Look, cx: &mut Context<Self>) -> Div {
        let p = palette();
        let add = match look.lock {
            None => icon_button("profile-add", "plus-sign", tr("New profile"), Style::Ghost)
                .on_click(cx.listener(|this, _, _, cx| {
                    if this.profile_lock().is_none() {
                        this.new_profile(cx);
                    }
                })),
            Some(reason) => crate::widgets::button::icon_only(
                "profile-add",
                crate::widgets::button::Look::new(
                    Style::Ghost,
                    crate::widgets::button::Size::Medium,
                )
                .icon("plus-sign")
                .disabled(true),
            )
            .tooltip(crate::glass_ui::tip(reason)),
        };
        let title = div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(px(tokens::CAPTION))
            .text_color(rgb(p.chrome_muted))
            .opacity(label_opacity(look.open))
            .child(tr("Profiles"));
        // Closed, the + alone is centred in the column.
        let pad = if look.open > 0. {
            ICON_PAD
        } else {
            (CLOSED_ROW - tokens::CONTROL) / 2.
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .px(px(pad))
            .when(look.open > 0., |d| d.child(title))
            .child(add)
    }

    /// The profiles, each with its group's heading where one begins, in a list that scrolls.
    fn profile_list(&mut self, look: Look, cx: &mut Context<Self>) -> Stateful<Div> {
        let selected = self.selected_instance().map(|i| i.id.clone());
        let rows = self.ui.profile_rows.clone();
        rows.borrow_mut().clear();
        let mut list = div()
            .id("sidebar-profiles")
            .flex()
            .flex_col()
            .min_h_0()
            .flex_1()
            .gap(px(4.))
            .overflow_y_scroll()
            .track_scroll(&crate::frame::scrolls("sidebar"));
        let headings = group_headings(self.instances.list.iter().map(|i| i.group.as_deref()));
        for (n, i) in self.instances.list.clone().into_iter().enumerate() {
            if let Some(group) = headings[n].clone() {
                list = list.child(profile_heading(look.open, group));
            }
            list = list.child(self.profile_entry(n, i, selected.as_deref(), look, cx));
        }
        list
    }

    /// One profile: its face, and its actions button while it is the selected one. The
    /// selected profile looks selected only on the Home page.
    fn profile_entry(
        &mut self,
        n: usize,
        i: Instance,
        selected: Option<&str>,
        look: Look,
        cx: &mut Context<Self>,
    ) -> Div {
        let current = selected == Some(i.id.as_str());
        // Every profile has its settings button (the default one too) while the sidebar is open;
        // closed, only the selected one keeps it, under its mark. Its menu (Delete) stays the
        // player's own profiles' right-click.
        let more = (current || look.open > 0.) && !self.location_unavailable;
        let face = self.profile_face(n, &i, current, more, look, cx);
        let actions = if more {
            profile_actions(&i, look.open, cx)
        } else {
            None
        };
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .child(face)
            .children(actions)
            // The room the closed button took under the mark eases away with the width, so the
            // profiles below don't jump as it leaves the row.
            .when(current && more && look.open > 0., |d| {
                d.child(div().flex_none().h(px(actions_room(look.open))))
            })
    }

    /// A profile's button: its mark and its name (the selected one's more button sits over its
    /// right end). Right-clicking it opens its menu, and a press picks it unless the profiles
    /// are locked.
    fn profile_face(
        &mut self,
        n: usize,
        i: &Instance,
        current: bool,
        more: bool,
        look: Look,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let on = current && look.on_home;
        let labelled = look.open > 0.;
        let mut button = profile_button(
            ElementId::named_usize("sidebar-profile", n),
            i.name.clone(),
            on,
            look,
        )
        .child(icon_slot(self.profile_mark(i, 28.)).when(current, |d| {
            d.debug_selector(|| "selected-profile-mark".into())
        }));
        if labelled {
            // The channel of the build it plays (a Stable profile plays Nightly until Stable has
            // a build), as the banner's build line names it.
            let under = self
                .profile_target(i)
                .as_deref()
                .and_then(crate::sheets::dated_channel)
                .unwrap_or_else(|| crate::sheets::channel_name(i.channel));
            button = button.child(label_text(look.open, i.name.clone(), Some(under)));
        }
        if labelled && more {
            button = button.pr(px(36.));
        }
        if i.kind == Kind::Own && !self.location_unavailable {
            button = menu_target(button, i, self.ui.profile_rows.clone(), cx);
        }
        if look.lock.is_none() {
            let pick = i.clone();
            button = button.on_click(cx.listener(move |this, _, _, cx| {
                this.switch_profile(&pick, cx);
            }));
        }
        button
    }
}
