//! The game-build chooser under Home's build button, and the profile menu beside it.

use super::*;
use crate::widgets::button::secondary;
use gpui::AnimationExt as _;
use std::time::Duration;

/// The menu's width, and a row's height.
const MENU_WIDTH: f32 = 232.;
const ROW_HEIGHT: f32 = 32.;
/// The submenu's width, the most it grows before it scrolls, and how far it sits from the menu.
const SUB_WIDTH: f32 = 224.;
const SUB_MAX: f32 = 300.;
const SUB_GAP: f32 = 2.;
/// How long the pointer may be away from every row and the submenu before the submenu closes.
const LEAVE_DELAY: Duration = Duration::from_millis(200);
/// How far the submenu slides while it fades in.
const SLIDE: f32 = 4.;

/// Which channel's submenu is open, and where the pointer is.
#[derive(Default)]
pub(super) struct Sub {
    open: Option<instances::Channel>,
    /// The row the pointer is on.
    row: Option<instances::Channel>,
    /// The pointer is on the submenu.
    over: bool,
}

/// The shared menu card: raised fill, line border, `R_CONTROL` corners, 4 px padding.
fn menu_card() -> Div {
    div()
        .flex()
        .flex_col()
        .p(px(4.))
        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
        .border_1()
        .border_color(palette().input_edge())
        .bg(rgb(palette().raised))
        .text_color(rgb(palette().text))
        .shadow_lg()
        .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation()
        })
}

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
        use instances::Channel;
        let Some(instance) = self.selected_instance() else {
            return div().id("alt-build");
        };
        let p = palette();
        let pinned = instance.version.clone();
        let plays = self.version_of(instance).cloned();
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
        let (left, top) = self.ui.home.chooser_at();
        let width = MENU_WIDTH.min(self.window_width - 24.);
        let left = left.min(self.window_width - width - 12.).max(12.);
        let open = self.ui.sheets.build_sub.open;
        let channels = [
            ("alt-build-stable", Channel::Stable),
            ("alt-build-nightly", Channel::Nightly),
            ("alt-build-dev", Channel::Dev),
        ];
        let mut rows = div().flex().flex_col().gap(px(1.));
        for (id, channel) in channels {
            // The row of the build in use is the one marked: the channel it follows, or the
            // channel of the build it is pinned to.
            let selected = if pinned.is_some() {
                active_channel == Some(channel)
            } else {
                follows == Some(channel)
            };
            let pinned_here = selected && pinned.is_some();
            // Updating a channel already in use is the job of Update game, not this row.
            let current =
                selected && !pinned_here && active_channel == Some(channel) && plays.is_some();
            let target = self.channel_switch_target(channel);
            let tag = if current {
                playing_target.as_ref()
            } else {
                target
            };
            let age: String = match (&pinned, pinned_here) {
                (Some(pin), true) => self.version_label(pin),
                _ => match tag {
                    Some(tag) => self
                        .build_age(tag)
                        .or_else(|| channel_build(tag).map(|(_, version)| version))
                        .unwrap_or_default(),
                    None => tr("none yet").into(),
                },
            };
            let unavailable = is_pack || target.is_none();
            let has_choices = !is_pack && !self.profile_build_choices(channel).is_empty();
            rows = rows.child(
                div()
                    .id(id)
                    .reach()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(ROW_HEIGHT))
                    .px(px(8.))
                    .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                    .cursor_pointer()
                    .text_size(px(tokens::LABEL))
                    .text_color(rgb(p.text))
                    .hover(move |d| d.bg(rgb(p.hover)))
                    .when(open == Some(channel), |d| d.bg(rgb(p.hover)))
                    .child(
                        div()
                            .flex_none()
                            .size(px(14.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .border_1()
                            .border_color(rgb(if selected { p.accent } else { p.muted }))
                            .when(selected, |d| {
                                d.bg(rgb(p.accent))
                                    .child(div().size(px(6.)).rounded_full().bg(rgb(p.accent_ink)))
                            }),
                    )
                    .child(div().flex_none().child(channel_name(channel)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(tokens::SMALL))
                            .text_color(rgb(p.muted))
                            .child(age),
                    )
                    .children(pinned_here.then(|| {
                        div()
                            .flex_none()
                            .text_size(px(tokens::SMALL))
                            .text_color(rgb(p.muted))
                            .child(tr("Pinned"))
                    }))
                    .children(has_choices.then(|| {
                        crate::glass_ui::icon(
                            "arrow-right-01",
                            14.,
                            rgb(if open == Some(channel) {
                                p.text
                            } else {
                                p.muted
                            })
                            .into(),
                        )
                    }))
                    .when(unavailable, |d| d.opacity(0.5))
                    .when(unavailable || current, |d| {
                        d.cursor_default().tab_stop(false)
                    })
                    .when(!unavailable && !current, |d| {
                        d.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.switch_channel(channel);
                            cx.notify();
                        }))
                    })
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if !*hovered {
                            this.build_sub_row_left(channel, cx);
                        } else if has_choices {
                            this.build_sub_enter(channel, cx);
                        } else {
                            this.build_sub_close_now(channel, cx);
                        }
                    })),
            );
        }
        let submenu = open.and_then(|channel| {
            let index = channels.iter().position(|(_, c)| *c == channel)?;
            self.build_submenu(channel, index, (left, top), width, cx)
        });
        let body = menu_card()
            .id("alt-build-body")
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(width))
            .max_h(px((self.window_height - top - 12.).max(120.)))
            .child(rows)
            .children(self.ui.sheets.error.clone().map(|error| {
                div().p(px(6.)).child(build_menu_error(error, |whole| {
                    secondary("alt-build-error-details", tr("Details")).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.open_notice(whole.clone(), None);
                            cx.notify();
                        },
                    ))
                }))
            }));
        // It fades in from 4 px higher up, as the shared dropdown's menu does, except while a
        // benchmark runs (its frames measure the list, not the fade) and under reduced motion.
        let body = if crate::motion::arrivals() {
            body.with_animation(
                "alt-build-in",
                crate::motion::eased(sunrise_launcher_theme::QUICK),
                move |d, t| d.top(px(top - 4. * (1. - t))).opacity(t),
            )
            .into_any_element()
        } else {
            body.into_any_element()
        };
        // A popover under Home's build picker: the layer is clear, and a press on it closes.
        div()
            .id("alt-build")
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
            .child(body)
            .children(submenu)
    }

    /// The pointer is on `channel`'s row (which has builds to pick) or on its submenu: that
    /// submenu is open.
    fn build_sub_enter(&mut self, channel: instances::Channel, cx: &mut Context<Self>) {
        let sub = &mut self.ui.sheets.build_sub;
        sub.row = Some(channel);
        if sub.open != Some(channel) {
            sub.open = Some(channel);
            cx.notify();
        }
    }

    /// The pointer is on a row with nothing to pick from: any open submenu goes at once.
    fn build_sub_close_now(&mut self, channel: instances::Channel, cx: &mut Context<Self>) {
        let sub = &mut self.ui.sheets.build_sub;
        sub.row = Some(channel);
        if sub.open.take().is_some() {
            cx.notify();
        }
    }

    /// The pointer left `channel`'s row.
    fn build_sub_row_left(&mut self, channel: instances::Channel, cx: &mut Context<Self>) {
        let sub = &mut self.ui.sheets.build_sub;
        // Another row may already have been entered.
        if sub.row == Some(channel) {
            sub.row = None;
        }
        self.build_sub_wait_to_close(cx);
    }

    /// Closes the submenu after a short while, unless the pointer is on a row or on the submenu
    /// by then, so a path across the gap between them doesn't lose it.
    fn build_sub_wait_to_close(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LEAVE_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                let sub = &mut this.ui.sheets.build_sub;
                if sub.row.is_none() && !sub.over && sub.open.take().is_some() {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// `channel`'s builds to pin, newest first, in a card beside the menu: opposite the row it
    /// opens from, to its right, or to its left when there is no room.
    fn build_submenu(
        &self,
        channel: instances::Channel,
        index: usize,
        (left, top): (f32, f32),
        width: f32,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        use sunrise_launcher::browser::sunrise::BuildChoice;
        let p = palette();
        let pinned = self.selected_instance()?.version.clone();
        let tags = self.profile_build_choices(channel);
        if tags.is_empty() {
            return None;
        }
        let high = (tags.len() as f32 * (ROW_HEIGHT + 1.) + 8.).min(SUB_MAX);
        // The card's border and padding sit above its first row.
        let row_top = top + 5. + index as f32 * (ROW_HEIGHT + 1.);
        let at = (row_top - 5.).min(self.window_height - high - 12.).max(12.);
        let beside = left + width + SUB_GAP;
        let (at_left, slide) = if beside + SUB_WIDTH <= self.window_width - 8. {
            (beside, -SLIDE)
        } else {
            ((left - SUB_GAP - SUB_WIDTH).max(8.), SLIDE)
        };
        let mut list = div()
            .id("alt-build-sub-list")
            .flex()
            .flex_col()
            .gap(px(1.))
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&crate::frame::scrolls("build-sub"));
        for (n, tag) in tags.iter().enumerate() {
            let on = pinned.as_deref() == Some(tag.as_str());
            let pick = tag.clone();
            list = list.child(
                div()
                    .id(gpui::ElementId::named_usize("alt-build-pin", n))
                    .reach()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(ROW_HEIGHT))
                    .px(px(8.))
                    .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                    .cursor_pointer()
                    .text_size(px(tokens::LABEL))
                    .text_color(rgb(p.text))
                    .when(on, |d| d.bg(rgb(p.selected)))
                    .hover(move |d| d.bg(rgb(p.hover)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(self.version_label(tag)),
                    )
                    .children(self.build_age(tag).map(|age| {
                        div()
                            .flex_none()
                            .text_size(px(tokens::SMALL))
                            .text_color(rgb(p.muted))
                            .child(age)
                    }))
                    .children(
                        on.then(|| crate::glass_ui::icon("tick-02", 16., rgb(p.accent).into())),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.begin_switch(pick.clone(), Some(BuildChoice::Keep));
                        cx.notify();
                    })),
            );
        }
        Some(
            menu_card()
                .id("alt-build-sub")
                .absolute()
                .w(px(SUB_WIDTH))
                .max_h(px(SUB_MAX))
                .child(list)
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    this.ui.sheets.build_sub.over = *hovered;
                    if !*hovered {
                        this.build_sub_wait_to_close(cx);
                    }
                }))
                .with_animation(
                    SharedString::from(format!("alt-build-sub-in-{}", channel_name(channel))),
                    crate::motion::eased(sunrise_launcher_theme::QUICK),
                    move |d, t| {
                        d.opacity(t)
                            .top(px(at))
                            .left(px(at_left + slide * (1. - t)))
                    },
                )
                .into_any_element(),
        )
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
            .child({
                let card = body
                    .id(plate)
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
                    .border_color(rgb(palette().line))
                    .bg(rgb(palette().raised))
                    .shadow(super::chrome::plate_shadow())
                    .text_color(rgb(palette().text))
                    .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                        cx.stop_propagation()
                    });
                // It fades in from 4 px higher up, as the shared dropdown's menu does, except
                // while a benchmark runs (its frames measure the list, not the fade) and under
                // reduced motion.
                if crate::motion::arrivals() {
                    card.with_animation(
                        SharedString::from(format!("{plate}-in")),
                        crate::motion::eased(sunrise_launcher_theme::QUICK),
                        move |d, t| d.top(px(top - 4. * (1. - t))).opacity(t),
                    )
                    .into_any_element()
                } else {
                    card.into_any_element()
                }
            })
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
