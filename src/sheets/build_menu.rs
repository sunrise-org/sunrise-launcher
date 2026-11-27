//! The game-build chooser under Home's build button, and the profile menu beside it.

use super::*;
use crate::widgets::button::secondary;

impl Launcher {
    /// The latest published build, or the latest installed one when the channel is offline.
    fn channel_switch_target(&self, channel: instances::Channel) -> Option<&String> {
        channel
            .builds(self.game_releases())
            .next()
            .map(|release| &release.tag)
            .or_else(|| match channel {
                instances::Channel::Nightly => self.known_nightly.as_ref(),
                _ => instances::newest_installed(channel, &self.installed),
            })
    }

    pub(super) fn switch_channel(&mut self, channel: instances::Channel) {
        use sunrise_launcher::browser::sunrise::BuildChoice;
        let Some(target) = self.channel_switch_target(channel).cloned() else {
            return;
        };
        self.begin_switch(target, Some(BuildChoice::Follow(channel)));
    }

    pub(super) fn build_sheet(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let Some(instance) = self.selected_instance() else {
            return div().id("alt-build");
        };
        let pinned = instance.version.clone();
        let plays = self.version_of(instance).cloned();
        let status = if pinned.is_some() {
            Some(tr("Stays on this build"))
        } else if plays.is_none() {
            let signed_in = self.accounts.selected().is_some()
                || sunrise_launcher::developer_offline::selected();
            Some(if signed_in {
                tr("Not installed yet")
            } else {
                tr("Installs after you sign in")
            })
        } else {
            None
        };
        let playing_target = self.profile_target(instance);
        let active_channel = playing_target
            .as_deref()
            .and_then(instances::Channel::of_build);
        let waits_for_stable =
            instance.channel == instances::Channel::Stable && !self.stable_available();
        let follows = pinned
            .is_none()
            .then_some(following_now(instance.channel, waits_for_stable));
        let is_pack = instance.kind == instances::Kind::Pack;
        let option = |id: &'static str, on: bool, head: String, sub: String| {
            div()
                .id(id)
                .reach()
                .flex()
                .gap(px(12.))
                .p(px(10.))
                .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                .cursor_pointer()
                .when(on, |d| d.bg(rgb(palette().selected)))
                .child(
                    div()
                        .mt(px(3.))
                        .size(px(14.))
                        .flex_none()
                        .rounded_full()
                        .border_1()
                        .border_color(rgb(if on {
                            palette().accent
                        } else {
                            palette().muted
                        }))
                        .when(on, |d| d.bg(rgb(palette().accent))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().text_size(px(tokens::BODY)).child(head))
                        // Plain text: the whole option is the button, nothing in it selects.
                        .child(
                            div()
                                .text_size(px(tokens::SMALL))
                                .text_color(rgb(palette().muted))
                                .child(sub),
                        ),
                )
        };
        use instances::Channel;
        let mut list = div().flex().flex_col().gap(px(2.));
        for (id, channel, unavailable) in [
            (
                "alt-build-stable",
                Channel::Stable,
                tr("No Stable build yet."),
            ),
            (
                "alt-build-nightly",
                Channel::Nightly,
                tr("No Nightly build yet."),
            ),
            ("alt-build-dev", Channel::Dev, tr("No Dev build yet.")),
        ] {
            let selected = follows == Some(channel);
            // Updating a channel already in use is the job of Update game, not this row.
            let current = selected && active_channel == Some(channel) && plays.is_some();
            let tag = if current {
                playing_target.as_ref()
            } else {
                self.channel_switch_target(channel)
            };
            // Stable says its version; every row says when its build came out.
            let version = tag.and_then(|tag| {
                let (_, version) = channel_build(tag)?;
                Some(match (channel, self.build_when(tag)) {
                    (Channel::Stable, Some(when)) => trf(
                        "{version} · {when}",
                        &[("version", &version), ("when", &when)],
                    ),
                    (Channel::Stable, None) => version,
                    (_, when) => when.unwrap_or(version),
                })
            });
            let unavailable_row = is_pack || version.is_none();
            // A Stable profile plays Nightly until Stable has a build; say so on the row it
            // will move to, rather than marking a row that can't play.
            let unavailable = if channel == Channel::Stable && waits_for_stable && pinned.is_none()
            {
                tr("No Stable build yet. Plays Nightly until there is one.")
            } else {
                unavailable
            };
            list = list.child(
                option(
                    id,
                    selected,
                    latest_name(channel).into(),
                    version.unwrap_or_else(|| unavailable.into()),
                )
                .when(unavailable_row, |d| d.opacity(0.5))
                .when(unavailable_row || current, |d| {
                    d.cursor_default().tab_stop(false)
                })
                .when(!unavailable_row && !current, |d| {
                    d.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.switch_channel(channel);
                        cx.notify();
                    }))
                }),
            );
        }
        list = list.child(
            option(
                "alt-build-keep",
                pinned.is_some(),
                tr("A specific version…").into(),
                match &pinned {
                    Some(tag) => build_name(tag),
                    None => tr("Pick one build and keep it until you change this.").into(),
                },
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.open_versions();
                cx.notify();
            })),
        );
        // Only the choices scroll when the window is short: the heading and an error stay in
        // view (#184).
        let choices = div()
            .id("alt-build-choices")
            .flex()
            .flex_col()
            .gap(px(10.))
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&crate::frame::scrolls("build-list"))
            .children(status.map(muted))
            .child(list);
        let body = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex_none()
                    .text_size(px(tokens::CAPTION))
                    .text_color(rgb(palette().muted))
                    .child(tr("GAME BUILD")),
            )
            .child(choices)
            .children(self.ui.sheets.error.clone().map(|error| {
                build_menu_error(error, |whole| {
                    secondary("alt-build-error-details", tr("Details")).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.open_notice(whole.clone(), None);
                            cx.notify();
                        },
                    ))
                })
            }));
        // A popover under Home's build picker: the scrim is clear.
        let at = self.ui.home.chooser_at();
        self.popover(("alt-build", "alt-build-body"), body, at, cx)
    }

    /// The selected profile's options, under Home's menu button: Sunrise Extras (here only, no
    /// longer in the build chooser) and the profile's folder.
    pub(super) fn profile_menu(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let Some(instance) = self.selected_instance() else {
            return div().id("alt-profile-menu");
        };
        let home = instance.home.clone();
        let folder = div()
            .id("alt-profile-folder")
            .reach()
            .flex()
            .items_center()
            .gap(px(12.))
            .p(px(10.))
            .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
            .cursor_pointer()
            .hover(|d| d.bg(rgb(palette().hover)))
            .child(crate::glass_ui::icon(
                "folder-01",
                16.,
                rgb(palette().text).into(),
            ))
            .child(
                div()
                    .text_size(px(tokens::BODY))
                    .child(tr("Open profile folder")),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.close_sheet();
                crate::toast::open(cx, &home);
                cx.notify();
            }));
        let body = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(extras().px(px(10.)))
            .child(div().h(px(1.)).bg(rgb(palette().line)))
            .child(folder);
        let at = self.ui.home.menu_at();
        self.popover(("alt-profile-menu", "alt-profile-menu-body"), body, at, cx)
    }

    /// A popover under one of Home's controls, its top left at `at`: a clear layer that a
    /// press outside closes, and `body` on a raised plate inside the window. `ids`: the
    /// layer's and the plate's.
    fn popover(
        &mut self,
        (layer, plate): (&'static str, &'static str),
        body: Div,
        (left, top): (f32, f32),
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let width = 372_f32.min(self.window_width - 24.);
        div()
            .id(layer)
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_down(MouseButton::Right, |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation()
            })
            .on_mouse_down(MouseButton::Middle, |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation()
            })
            .on_scroll(|_, _, cx| cx.stop_propagation())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.close_sheet();
                    cx.notify();
                }),
            )
            .child(
                body.id(plate)
                    .absolute()
                    // Never taller than the room under the picker: the choices scroll.
                    .max_h(px((self.window_height - top - 12.).max(120.)))
                    .overflow_hidden()
                    // Inside the window, however narrow it is.
                    .left(px(left.min(self.window_width - width - 12.).max(12.)))
                    .top(px(top))
                    .w(px(width))
                    .p(px(14.))
                    .rounded(crate::tokens::r(crate::tokens::R_CARD))
                    .border_1()
                    .border_color(palette().input_edge())
                    .bg(rgb(palette().raised))
                    .text_color(rgb(palette().text))
                    .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                        cx.stop_propagation()
                    }),
            )
    }
}

/// Sunrise Extras' switch, shown off and dim: it arrives with the build that supports it.
fn extras() -> Div {
    div()
        .flex()
        .items_start()
        .justify_between()
        .gap(px(12.))
        .opacity(0.5)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().text_size(px(tokens::BODY)).child(tr("Sunrise Extras")))
                .child(
                    div()
                        .text_size(px(tokens::SMALL))
                        .text_color(rgb(palette().muted))
                        .child(tr(
                            "Sunrise’s additions on top of vanilla. This switch arrives with the build that supports it.",
                        )),
                ),
        )
        .child(
            div()
                .mt(px(2.))
                .flex_none()
                .w(px(34.))
                .h(px(20.))
                .rounded_full()
                .border_1()
                .border_color(rgb(palette().line)),
        )
}

/// The pinned preview has room for two lines. Every error can open in the scrollable notice.
pub(super) fn build_menu_error(
    error: String,
    details: impl FnOnce(String) -> Stateful<Div>,
) -> Div {
    div()
        .flex_none()
        .flex()
        .items_start()
        .gap(px(8.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .line_clamp(2)
                .text_ellipsis()
                .text_color(rgb(palette().danger))
                .text_size(px(tokens::SMALL))
                .child(error.clone()),
        )
        .child(details(error))
}
