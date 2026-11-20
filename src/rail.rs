//! The full-height profiles sidebar. Its middle scrolls independently of its controls.
use super::*;
use crate::{
    frame::{Page, Reach},
    paint::palette,
};
use gpui::rgb;
use instances::{Icon, Instance};

/// How wide the sidebar is, open or closed.
pub(crate) fn rail_width(open: bool) -> f32 {
    if open {
        204.
    } else {
        54.
    }
}

/// The column a sidebar icon is centred in, open or closed: a closed row's inside (the closed
/// rail less its 1 px edges, its 5 px padding and the row's own 1 px edges). Labels follow it,
/// so no icon moves when the sidebar opens or closes.
fn icon_slot(icon: impl IntoElement) -> Div {
    div()
        .flex_none()
        .w(px(rail_width(false) - 2. * (1. + 5. + 1.)))
        .flex()
        .items_center()
        .justify_center()
        .child(icon)
}

fn profile_lock_reason(
    work: &crate::launch_state::Facts,
    located: bool,
    pending: bool,
) -> Option<&'static str> {
    if !located {
        Some(tr(
            "Fix the data folder in Settings before changing profiles.",
        ))
    } else if work.busy() {
        Some(tr(
            "Wait for the current operation to finish before changing profiles.",
        ))
    } else if pending {
        Some(tr(
            "Finish or cancel the pending launch before changing profiles.",
        ))
    } else {
        None
    }
}

impl Launcher {
    pub(super) fn toggle_sidebar(&mut self) {
        self.settings.sidebar_expanded = !self.settings.sidebar_expanded;
        self.save_settings();
    }

    fn profile_lock(&self) -> Option<&'static str> {
        profile_lock_reason(
            &self.launch_activity(),
            !self.location_unavailable,
            self.pending_consent.is_some()
                || self.ui.after_sign_in.is_some()
                || self.ui.after_install.is_some(),
        )
    }

    pub(super) fn switch_profile(&mut self, instance: &Instance, cx: &mut Context<Self>) {
        if self.profile_lock().is_some() {
            return;
        }
        if !self
            .selected_instance()
            .is_some_and(|i| i.id == instance.id)
        {
            self.select_instance(instance);
            self.quick_play = None;
            self.joining = None;
            self.client_error = None;
            self.prepare_error = None;
            self.install_error = None;
            self.crash = None;
        }
        self.navigate(Page::Home);
        cx.notify();
    }

    pub(super) fn profile_mark(&mut self, i: &Instance, size: f32) -> Div {
        let p = palette();
        let picture = self.icon_image(format!("picture:{}", i.id), size, || {
            Some(i.home.join(instances::PICTURE))
        });
        let texture = match &i.icon {
            Some(icon @ Icon::Texture(name)) if picture.is_none() => {
                self.version_of(i).cloned().and_then(|tag| {
                    self.icon_image(format!("texture:{tag}:{name}"), size, || {
                        icon.texture(&backend::resources(&tag))
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

    pub(super) fn sidebar(&mut self, cx: &mut Context<Self>) -> Div {
        let wide = self.settings.sidebar_expanded;
        let p = palette();
        let lock = self.profile_lock();
        let row = |id: ElementId, label: String, on: bool| {
            div()
                .id(id)
                .reach_edge()
                .h(px(40.))
                .flex_none()
                .w_full()
                .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                .border_1()
                .border_color(if on {
                    rgb(p.accent_ink).into()
                } else {
                    gpui::transparent_black()
                })
                // The chrome's own lift, in the theme's hue, under the accent's edge: the
                // accent washed into the fill stood out in themes whose hue differs from it.
                .when(on, |d| d.bg(rgb(p.chrome_raised)))
                .flex()
                .items_center()
                .gap(px(4.))
                .when(wide, |d| d.pr(px(8.)))
                .when(lock.is_none(), |d| {
                    d.cursor_pointer().hover(|d| d.bg(rgb(p.chrome_raised)))
                })
                .when(lock.is_some(), |d| d.opacity(0.45).tab_stop(false))
                .tooltip(crate::glass_ui::tip(lock.map_or(label, str::to_owned)))
        };
        let txt = |s: String| {
            div()
                .min_w_0()
                .flex_1()
                .truncate()
                .text_size(px(tokens::LABEL))
                .child(s)
        };
        // A small toolbar button, not a row: it names itself in its tooltip only. Ctrl+B does
        // the same, and the search lists that key.
        let toggle = div()
            .flex()
            .flex_none()
            .h(px(32.))
            .items_center()
            // In line with the rows' icons, whose rows have a 1 px edge.
            .px(px(1.))
            .child(icon_slot(
                div()
                    .id("sidebar-toggle")
                    .reach()
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                    .cursor_pointer()
                    .hover(|d| d.bg(rgb(p.chrome_raised)))
                    .tooltip(crate::glass_ui::tip(if wide {
                        tr("Hide sidebar")
                    } else {
                        tr("Show sidebar")
                    }))
                    .child(
                        div()
                            .w(px(16.))
                            .h(px(13.))
                            .flex()
                            .rounded(px(3.))
                            .border_1()
                            .border_color(rgb(p.chrome_muted))
                            .child(div().ml(px(4.)).w(px(1.)).h_full().bg(rgb(p.chrome_muted))),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_sidebar();
                        cx.notify();
                    })),
            ));
        let add = row("sidebar-create".into(), tr("New profile").into(), false)
            .child(icon_slot(blocks::icon(
                "plus",
                20.,
                rgb(p.chrome_text).into(),
            )))
            .when(wide, |d| d.child(txt(tr("New profile").into())))
            .when(lock.is_none(), |d| {
                d.on_click(cx.listener(|this, _, _, cx| {
                    if this.profile_lock().is_none() {
                        this.new_profile(cx);
                    }
                }))
            });
        let selected = self.selected_instance().map(|i| i.id.clone());
        let mut list = div()
            .id("sidebar-profiles")
            .flex()
            .flex_col()
            .min_h_0()
            .flex_1()
            .gap(px(4.))
            .overflow_y_scroll()
            .track_scroll(&crate::frame::scrolls("sidebar"));
        for (n, i) in self.instances.list.clone().into_iter().enumerate() {
            let on = selected.as_ref() == Some(&i.id);
            list = list.child(
                row(
                    ElementId::named_usize("sidebar-profile", n),
                    i.name.clone(),
                    on,
                )
                .child(icon_slot(self.profile_mark(&i, 28.)))
                .when(wide, |d| d.child(txt(i.name.clone())))
                .when(lock.is_none(), |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| this.switch_profile(&i, cx)))
                }),
            );
        }
        let mut rail = div()
            .flex()
            .flex_col()
            .w(px(rail_width(wide)))
            .min_h_0()
            .flex_none()
            .gap(px(4.))
            .p(px(5.))
            .rounded(crate::tokens::r(crate::tokens::R_PANEL))
            .bg(p.chrome_plate())
            .border_1()
            .border_color(rgb(p.divider))
            .text_color(rgb(p.chrome_text))
            .child(toggle)
            .child(add)
            .child(list);
        for (id, label, icon, url) in [
            (
                "sidebar-discord",
                tr("Discord"),
                "discord",
                self.links.discord.clone(),
            ),
            (
                "sidebar-website",
                tr("Website"),
                "earth",
                self.links.website.clone(),
            ),
            (
                "sidebar-settings",
                tr("Settings"),
                "sliders-horizontal",
                String::new(),
            ),
        ] {
            let settings = id == "sidebar-settings";
            rail = rail.child(
                row(
                    id.into(),
                    label.into(),
                    settings && self.ui.page == Page::Settings,
                )
                .child(icon_slot(crate::glass_ui::icon(
                    icon,
                    18.,
                    rgb(p.chrome_muted).into(),
                )))
                .when(wide, |d| d.child(txt(label.into())))
                .when(settings || !url.is_empty(), |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        if settings {
                            this.navigate(Page::Settings);
                        } else {
                            Self::open_url(&url, cx);
                        }
                        cx.notify();
                    }))
                }),
            );
        }
        rail
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launch_state::Facts;

    #[test]
    fn profiles_unlock_when_starting_finishes_even_while_the_game_runs() {
        let mut work = Facts {
            starting: true,
            ..Facts::default()
        };
        assert!(profile_lock_reason(&work, true, false).is_some());
        work.running = true;
        work.starting = false;
        assert_eq!(profile_lock_reason(&work, true, false), None);
        assert!(profile_lock_reason(&work, false, false).is_some());
        assert!(profile_lock_reason(&work, true, true).is_some());
    }
}
