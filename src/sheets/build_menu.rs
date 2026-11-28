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

/// The channels, top to bottom, with their row ids.
const CHANNELS: [(&str, instances::Channel); 3] = [
    ("alt-build-stable", instances::Channel::Stable),
    ("alt-build-nightly", instances::Channel::Nightly),
    ("alt-build-dev", instances::Channel::Dev),
];

/// What the chooser chooses the build of: Home's selected profile, or the New and Edit profile
/// dialog's pending choice.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Chooses {
    Profile,
    Dialog,
}

/// What a row of the chooser was picked for.
enum Pick {
    /// Follow the channel's newest build.
    Follow(instances::Channel),
    /// Keep one build.
    Keep(String),
}

/// What the chooser shows about the choice it is for.
struct Facts {
    /// The build kept, if any.
    pinned: Option<String>,
    /// The channel followed (its Newest is ticked while nothing is pinned).
    channel: instances::Channel,
    /// The row with the filled radio.
    marked: Option<instances::Channel>,
    /// The channel of the build in use, whose row needs no press.
    current: Option<instances::Channel>,
    /// The build that plays now, shown on the current row.
    playing_target: Option<String>,
    /// A pack plays its own mods: nothing is chosen.
    locked: bool,
}

/// Where the menu is, and how much room the submenu has. In Home it is in window coordinates,
/// with the window's size to keep the submenu inside; in the dialog it is relative to the field's
/// parent, and the submenu always opens to the right.
struct Place {
    left: f32,
    top: f32,
    width: f32,
    window: Option<(f32, f32)>,
}

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
    pub(crate) fn channel_switch_target(&self, channel: instances::Channel) -> Option<&String> {
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

    /// A channel picked in the chooser: the profile follows it from now on. A played profile
    /// only records that; Play then offers to install the build (`choose_build`).
    fn choose_channel(&mut self, channel: instances::Channel) {
        use sunrise_launcher::browser::sunrise::BuildChoice;
        let Some(target) = self.channel_switch_target(channel).cloned() else {
            return;
        };
        self.choose_build(target, BuildChoice::Follow(channel));
    }

    /// The chooser's own state: which profile it chooses for, and what that profile has now.
    fn profile_facts(&self) -> Option<Facts> {
        let instance = self.selected_instance()?;
        let pinned = instance.version.clone();
        let plays = self.version_of(instance).cloned();
        let playing_target = self.profile_target(instance);
        let active_channel = playing_target
            .as_deref()
            .and_then(instances::Channel::of_build);
        // The marked row is the chosen build's, which isn't the played one while a choice waits
        // for Play's install action.
        let pinned_channel = pinned
            .as_deref()
            .and_then(instances::Channel::of_build)
            .or(active_channel);
        let waits_for_stable =
            instance.channel == instances::Channel::Stable && !self.stable_available();
        let follows = pinned
            .is_none()
            .then_some(following_now(instance.channel, waits_for_stable));
        Some(Facts {
            marked: if pinned.is_some() {
                pinned_channel
            } else {
                follows
            },
            current: active_channel.filter(|_| plays.is_some()),
            channel: instance.channel,
            pinned,
            playing_target,
            locked: instance.kind == instances::Kind::Pack,
        })
    }

    /// The same facts for the New and Edit profile dialog's pending choice, which has no build
    /// in use yet.
    fn dialog_facts(&self) -> Option<Facts> {
        let d = self.instances.dialog.as_ref()?;
        let waits_for_stable = d.channel == instances::Channel::Stable && !self.stable_available();
        let pinned_channel = d.pin.as_deref().and_then(instances::Channel::of_build);
        Some(Facts {
            marked: if d.pin.is_some() {
                pinned_channel
            } else {
                Some(following_now(d.channel, waits_for_stable))
            },
            current: None,
            channel: d.channel,
            pinned: d.pin.clone(),
            playing_target: None,
            locked: false,
        })
    }

    /// A pick in the chooser. Home's profile records it (and Play offers to install the build);
    /// the dialog only sets its own pending choice: nothing switches and nothing downloads.
    fn build_picked(&mut self, chooses: Chooses, pick: Pick) {
        match (chooses, pick) {
            (Chooses::Profile, Pick::Follow(channel)) => self.choose_channel(channel),
            (Chooses::Profile, Pick::Keep(tag)) => {
                self.choose_build(tag, sunrise_launcher::browser::sunrise::BuildChoice::Keep)
            }
            (Chooses::Dialog, pick) => {
                if let Some(d) = self.instances.dialog.as_mut().filter(|d| !d.busy) {
                    match pick {
                        Pick::Follow(channel) => {
                            d.channel = channel;
                            d.pin = None;
                        }
                        Pick::Keep(tag) => d.pin = Some(tag),
                    }
                    d.menu = None;
                }
                self.close_menus();
            }
        }
    }

    /// The dialog's open build menu: the channels, each with its builds in a submenu, hung
    /// under the field `new-build` (whose parent is `relative`) with the shared dropdown's
    /// backdrop, keys and fade. `close` is the dialog's way of closing its menu.
    pub(crate) fn dialog_build_menu(
        &mut self,
        close: crate::widgets::dropdown::Close,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(facts) = self.dialog_facts() else {
            return Vec::new();
        };
        let open = self.ui.sheets.build_sub.open;
        let rows: Vec<AnyElement> = CHANNELS
            .iter()
            .map(|&(id, channel)| {
                self.build_row(Chooses::Dialog, &facts, id, channel, open, cx)
                    .into_any_element()
            })
            .collect();
        let count = rows.len();
        let mut layers: Vec<AnyElement> = crate::widgets::dropdown::layer(
            &SharedString::from("new-build"),
            rows,
            count,
            None,
            Some(MENU_WIDTH),
            close,
        )
        .into_iter()
        .map(IntoElement::into_any_element)
        .collect();
        let submenu = open.and_then(|channel| {
            let index = CHANNELS.iter().position(|&(_, c)| c == channel)?;
            let place = Place {
                left: 0.,
                top: crate::tokens::FIELD + crate::widgets::dropdown::GAP,
                width: MENU_WIDTH,
                window: None,
            };
            self.build_submenu(Chooses::Dialog, &facts, channel, index, place, cx)
        });
        // Above the menu's own layer.
        layers.extend(submenu.map(|sub| gpui::deferred(sub).with_priority(11).into_any_element()));
        layers
    }

    /// One channel's row: its radio, name and build, the arrow when it has builds to pick from.
    fn build_row(
        &self,
        chooses: Chooses,
        facts: &Facts,
        id: &'static str,
        channel: instances::Channel,
        open: Option<instances::Channel>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let p = palette();
        // The row of the build in use is the one marked: the channel it follows, or the
        // channel of the build it is pinned to.
        let selected = facts.marked == Some(channel);
        let pinned_here = selected && facts.pinned.is_some();
        // Updating a channel already in use is the job of Update game, not this row.
        let current = selected && !pinned_here && facts.current == Some(channel);
        let target = self.channel_switch_target(channel);
        let tag = if current {
            facts.playing_target.as_ref()
        } else {
            target
        };
        let age: String = match (&facts.pinned, pinned_here) {
            (Some(pin), true) => self.version_label(pin),
            _ => match tag {
                Some(tag) => self
                    .build_age(tag)
                    .or_else(|| channel_build(tag).map(|(_, version)| version))
                    .unwrap_or_default(),
                None => tr("none yet").into(),
            },
        };
        let unavailable = facts.locked || target.is_none();
        let has_choices = !facts.locked && !self.profile_build_choices(channel).is_empty();
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
                    this.build_picked(chooses, Pick::Follow(channel));
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
            }))
    }

    pub(super) fn build_sheet(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let Some(facts) = self.profile_facts() else {
            return div().id("alt-build");
        };
        let (left, top) = self.ui.home.chooser_at();
        let width = MENU_WIDTH.min(self.window_width - 24.);
        let left = left.min(self.window_width - width - 12.).max(12.);
        let open = self.ui.sheets.build_sub.open;
        let mut rows = div().flex().flex_col().gap(px(1.));
        for (id, channel) in CHANNELS {
            rows = rows.child(self.build_row(Chooses::Profile, &facts, id, channel, open, cx));
        }
        let submenu = open.and_then(|channel| {
            let index = CHANNELS.iter().position(|&(_, c)| c == channel)?;
            let place = Place {
                left,
                top,
                width,
                window: Some((self.window_width, self.window_height)),
            };
            self.build_submenu(Chooses::Profile, &facts, channel, index, place, cx)
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
        chooses: Chooses,
        facts: &Facts,
        channel: instances::Channel,
        index: usize,
        place: Place,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let p = palette();
        let pinned = facts.pinned.clone();
        // "Newest" heads the list: it follows the channel, so a long list never has to be read.
        let follows_this = pinned.is_none() && facts.channel == channel;
        let tags = self.profile_build_choices(channel);
        if tags.is_empty() {
            return None;
        }
        let high = ((tags.len() + 1) as f32 * (ROW_HEIGHT + 1.) + 15.).min(SUB_MAX);
        let (at, at_left, slide) = match place.window {
            Some((window_width, window_height)) => {
                // The card's border and padding sit above its first row.
                let row_top = place.top + 5. + index as f32 * (ROW_HEIGHT + 1.);
                let at = (row_top - 5.).min(window_height - high - 12.).max(12.);
                let beside = place.left + place.width + SUB_GAP;
                if beside + SUB_WIDTH <= window_width - 8. {
                    (at, beside, -SLIDE)
                } else {
                    (at, (place.left - SUB_GAP - SUB_WIDTH).max(8.), SLIDE)
                }
            }
            None => (
                place.top + index as f32 * (ROW_HEIGHT + 1.),
                place.left + place.width + SUB_GAP,
                -SLIDE,
            ),
        };
        let mut list = div()
            .id("alt-build-sub-list")
            .flex()
            .flex_col()
            .gap(px(1.))
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&crate::frame::scrolls("build-sub"));
        list = list
            .child(
                div()
                    .id("alt-build-newest")
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
                    .when(follows_this, |d| d.bg(rgb(p.selected)))
                    .hover(move |d| d.bg(rgb(p.hover)))
                    .child(div().flex_1().min_w_0().truncate().child(tr("Newest")))
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(tokens::SMALL))
                            .text_color(rgb(p.muted))
                            .child(tr("always")),
                    )
                    .children(
                        follows_this
                            .then(|| crate::glass_ui::icon("tick-02", 16., rgb(p.accent).into())),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.build_picked(chooses, Pick::Follow(channel));
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .flex_none()
                    .h(px(1.))
                    .mx(px(6.))
                    .my(px(3.))
                    .bg(rgb(p.divider)),
            );
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
                            .child(self.build_name_with_commit(tag)),
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
                        this.build_picked(chooses, Pick::Keep(pick.clone()));
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
        let Some(instance) = self.selected_instance().cloned() else {
            return div().id("alt-profile-menu");
        };
        let p = palette();
        let home = instance.home.clone();
        // One row of the menu: an icon, its words, and what it does.
        let row = |id: &'static str, icon: Div, label: SharedString| {
            div()
                .id(id)
                .reach()
                .h(px(32.))
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(10.))
                .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                .cursor_pointer()
                .hover(move |d| d.bg(rgb(p.hover)))
                .text_size(px(tokens::LABEL))
                .child(icon)
                .child(label)
        };
        let glyph = |name: &'static str| {
            div()
                .flex_none()
                .child(crate::glass_ui::icon(name, 15., rgb(p.muted).into()))
        };
        let folder = row(
            "alt-profile-folder",
            glyph("folder-01"),
            tr("Open profile folder").into(),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.close_sheet();
            crate::toast::open(cx, &home);
            cx.notify();
        }));
        let settings = row(
            "alt-profile-settings",
            glyph("settings-01"),
            tr("Profile settings…").into(),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.close_sheet();
            this.edit_profile(&instance, cx);
            cx.notify();
        }));
        let body = div()
            .flex()
            .flex_col()
            .gap(px(1.))
            .child(extras())
            .child(div().h(px(1.)).mx(px(6.)).my(px(3.)).bg(rgb(p.divider)))
            .child(folder)
            .child(settings);
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
        let width = 232_f32.min(self.window_width - 24.);
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
                    .p(px(4.))
                    .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
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
    let p = palette();
    div()
        .h(px(32.))
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(10.))
        .opacity(0.55)
        .text_size(px(tokens::LABEL))
        .child(div().flex_none().child(crate::mark(15.)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(tr("Sunrise Extras")),
        )
        .child(
            div()
                .flex_none()
                .text_size(px(tokens::SMALL))
                .text_color(rgb(p.muted))
                .child(tr("soon")),
        )
        // The switch arrives with the build that supports it: drawn off, it takes no press.
        .child(
            div()
                .flex_none()
                .w(px(28.))
                .h(px(16.))
                .p(px(2.))
                .rounded_full()
                .bg(rgb(p.line))
                .child(div().size(px(12.)).rounded_full().bg(rgb(p.muted))),
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
