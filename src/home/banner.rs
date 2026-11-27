//! The profile page's banner: the profile's mark, name and build line, Play, Profile settings and
//! the cat's lane beside them. Under it: a build under way, and what the profile's state asks of
//! the player (a problem, a held update, an update).
use super::*;
use crate::launch_state::{Act, Main, View};
use crate::sheets::Sheet;
use crate::widgets::button::{button, icon_button, Look, Size, Style};
use crate::widgets::feedback::{banner, progress, Tone};
use gpui::rgb;

/// The banner's height when nothing under the buttons adds to it (Figma: the art's 208).
const BANNER_HEIGHT: f32 = 208.;
/// Play's width (Figma).
const PLAY_WIDTH: f32 = 196.;
/// How far above the banner's bottom the cat's feet stand: on the dark ground strip along
/// the art's bottom edge.
const GROUND: f32 = 10.;

impl Launcher {
    /// The selected profile's top: the banner, then the state the profile is in.
    pub(super) fn profile_header(
        &mut self,
        instance: &instances::Instance,
        compact: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let view = self.launch_facts().view();
        let banner = self.profile_banner(instance, compact, &view, window, cx);
        let state = profile_state(&view, cx);
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(banner)
            .children(state)
    }

    /// The banner (Figma "Profile · …"): the scenery on its right, fading into the panel; the
    /// mark, name and build line with Change build; under them Play and Profile settings, and
    /// the cat's lane beside them. Under the banner: a build under way and what the profile says.
    fn profile_banner(
        &mut self,
        instance: &instances::Instance,
        compact: bool,
        view: &View,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let (build, tag) = self.build_name(instance);
        let identity = self.identity(instance);
        let controls = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(self.play_button(view, compact, cx))
            .child(self.profile_options_button(cx));
        // Play and the gear, and under them the build it plays, which opens the chooser.
        let actions = div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(8.))
            .child(controls)
            .child(self.change_build(build, tag, cx));
        // The cat walks the ground along the art's bottom edge, right of the buttons.
        let (lane, _) = self.cat(true, false, window, cx);
        let pad = if compact { 16. } else { 24. };
        let ground = div()
            .absolute()
            .left(px(pad + PLAY_WIDTH + 8. + tokens::PLAY + 16.))
            .right(px(pad))
            .bottom(px(GROUND - crate::cat::BELOW_FEET))
            .flex()
            .child(lane);
        let art = self.ui.home.art.take();
        let panel = palette().background;
        div()
            .relative()
            .w_full()
            .overflow_hidden()
            .rounded_t(crate::tokens::r(crate::tokens::R_PANEL))
            .when_some(art, |d, art| match art {
                crate::home::Art::Fading(art) => d.child(art),
                // A scene stands on the right; the panel's colour washes over its left part so
                // the name and the buttons read on the panel, not on the picture.
                crate::home::Art::Scene(art) => {
                    d.child(art)
                        .child(div().absolute().inset_0().bg(gpui::linear_gradient(
                            90.,
                            gpui::linear_color_stop(rgb(panel), 0.3),
                            gpui::linear_color_stop(gpui::Hsla::from(rgb(panel)).opacity(0.), 0.7),
                        )))
                }
            })
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .gap(px(20.))
                    .p(px(if compact { 16. } else { 24. }))
                    .pt(px(if compact { 16. } else { 32. }))
                    // As tall as the design's art (208), however little the banner holds.
                    .min_h(px(BANNER_HEIGHT))
                    .child(identity)
                    .child(actions)
                    .children(self.build_progress(view, cx))
                    .children(self.says(instance, view, false, cx)),
            )
            .child(ground)
    }

    /// The profile's mark, its name and, under the name, when it was last played and for how
    /// long.
    fn identity(&mut self, instance: &instances::Instance) -> Div {
        let p = palette();
        div()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(16.))
            .child(self.profile_mark(instance, 60.))
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(tokens::HERO))
                            .line_height(px(tokens::HERO * 1.2))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(p.text))
                            .child(instance.name.clone()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(tokens::LABEL))
                            .text_color(rgb(p.muted))
                            .child(identity_line(instance)),
                    ),
            )
    }

    /// The build's name and the tag it was made from (for the tooltip). A profile on Dev with no
    /// build says why it has none.
    fn build_name(&self, instance: &instances::Instance) -> (String, Option<String>) {
        let version = self.profile_target(instance);
        // A built mod image reads as its base; keep the actual tag in the tooltip.
        let base = version
            .as_deref()
            .filter(|t| native_mods::is_native(t))
            .and_then(crate::profile_builds::native_base);
        let build: String = version.as_ref().map_or_else(
            // Dev never falls back to another channel, so say why it has nothing to play: the
            // build list is still on its way or didn't load, else the channel has no build yet
            // (sunrise-org/sunrise-launcher#417).
            || match (instance.channel, &self.releases) {
                (instances::Channel::Dev, Load::Loading) => tr("Dev · finding builds…").into(),
                (instances::Channel::Dev, Load::Failed(_)) => tr("Dev · builds didn't load").into(),
                (instances::Channel::Dev, Load::Ready(_)) => tr("Dev · no build yet").into(),
                _ => tr("Sunrise").into(),
            },
            |tag| {
                let shown = base.as_deref().unwrap_or(tag);
                // A Nightly or Dev build says how long ago it came out, as the menu does.
                // Figma: "Nightly build · 6 days old".
                match (crate::sheets::dated_channel(shown), self.build_age(shown)) {
                    (Some(channel), Some(age)) => trf(
                        "{channel} build · {age}",
                        &[("channel", channel), ("age", &age)],
                    ),
                    (Some(channel), None) => trf("{channel} build", &[("channel", channel)]),
                    _ => self.version_label(shown),
                }
            },
        );
        (build, version)
    }

    /// "Change build": the build chooser opens under it.
    fn change_build(
        &mut self,
        build: String,
        tag: Option<String>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let at = self.ui.home.picker.clone();
        let p = palette();
        // A quiet picker under Play: the build's glyph, its name and age, a chevron. No fill
        // until the pointer is on it.
        let change = div()
            .id("alt-build")
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(28.))
            .px(px(8.))
            .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
            .cursor_pointer()
            .text_size(px(tokens::LABEL))
            .text_color(rgb(p.text))
            .hover(move |d| d.bg(rgb(p.hover)))
            .reach()
            .child(crate::glass_ui::icon(
                "package-01",
                16.,
                rgb(p.muted).into(),
            ))
            .child(build)
            .child(crate::glass_ui::icon(
                "arrow-down-01",
                14.,
                rgb(p.muted).into(),
            ))
            .when_some(tag, |d, tag| d.tooltip(crate::glass_ui::tip(tag)))
            .child(
                gpui::canvas(
                    move |bounds, window, cx| {
                        if at.replace(Some(bounds)) != Some(bounds) {
                            // An open chooser follows it, in the frame after this one.
                            let window = window.window_handle();
                            cx.defer(move |cx| {
                                window.update(cx, |_, window, _| window.refresh()).ok();
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            );
        change.on_click(cx.listener(|this, _, _, cx| {
            if !this.launch_busy() && this.client.is_none() {
                this.open_sheet(Sheet::Build);
            }
            cx.notify();
        }))
    }

    /// Play: a Primary button at Play's height. It is still the one entry point for Play's
    /// state, label, tooltip and enabled flag; the cat sleeps on it when it is idle.
    fn play_button(&mut self, view: &View, compact: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        // The play glyph stands before what starts the game, signing in first included.
        let idle = matches!(
            view.main,
            Main::Play | Main::BuildAndPlay | Main::SignIn | Main::Install
        );
        let look = Look::new(Style::Primary, Size::Medium).disabled(!view.enabled);
        // Figma: the play glyph before the label, while Play (or Sign in & play) is what it does.
        let look = if idle { look.icon("play") } else { look };
        let play = button("alt-play", view.main.label(), look)
            .h(px(tokens::PLAY))
            .min_w(px(if compact { 132. } else { PLAY_WIDTH }))
            .px(px(if compact { 12. } else { 20. }))
            .text_size(px(tokens::LEAD))
            .font_weight(FontWeight::SEMIBOLD)
            // Nothing is added to the page: signed out, the tooltip says Play signs in first.
            .when_some(view.main.tooltip(), |d, text| {
                d.tooltip(crate::glass_ui::tip(text))
            });
        if view.enabled {
            play.on_click(cx.listener(|this, _, _, cx| {
                this.cat_off_play();
                this.press_play();
                cx.notify();
            }))
        } else {
            play
        }
    }

    /// The profile's own options (Sunrise Extras, its folder), in a menu like the build chooser's.
    fn profile_options_button(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let at = self.ui.home.menu.clone();
        icon_button(
            "alt-quick-menu",
            "settings-01",
            tr("Profile options"),
            Style::Secondary,
        )
        .size(px(tokens::PLAY))
        .child(
            gpui::canvas(
                move |bounds, window, cx| {
                    if at.replace(Some(bounds)) != Some(bounds) {
                        // An open menu follows it, in the frame after this one.
                        let window = window.window_handle();
                        cx.defer(move |cx| {
                            window.update(cx, |_, window, _| window.refresh()).ok();
                        });
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        )
        .on_click(cx.listener(|this, _, _, cx| {
            if this.ui.sheet == Some(Sheet::ProfileMenu) {
                this.close_sheet();
            } else {
                this.open_sheet(Sheet::ProfileMenu);
            }
            cx.notify();
        }))
    }

    /// While native mods build: the build's line and bar (a press opens the build window) and
    /// its Stop.
    fn build_progress(&mut self, view: &View, cx: &mut Context<Self>) -> Option<Div> {
        if view.act != Some(Act::StopBuild) {
            return None;
        }
        let run = self.build_here().filter(|run| run.running())?;
        let fraction = run.fraction();
        let value = format!("{:.0}%", 100. * fraction);
        let bar = div()
            .id("alt-build-open")
            .flex_1()
            .min_w_0()
            .cursor_pointer()
            .tooltip(crate::glass_ui::tip(tr("Show the build")))
            .child(progress(run.line(), Some(value.into()), Some(fraction)))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.build_all();
                cx.notify();
            }));
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(bar)
                .children(view.act.clone().map(|act| act_button(act, cx))),
        )
    }

    /// What the profile says under the banner when no state banner says it: a build's outcome,
    /// the other profile that is playing, a newer build, mods off, and the build receipt.
    fn says(
        &mut self,
        instance: &instances::Instance,
        view: &View,
        scenery: bool,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let p = palette();
        let (text, link_ink) = if scenery {
            (p.hero.text, p.hero.link)
        } else {
            (p.text, p.accent_ink)
        };
        let other_running = self
            .client_run
            .as_ref()
            .filter(|(running, _)| {
                running.id != instance.id || running.incarnation != instance.incarnation
            })
            .map(|(running, _)| trf("Playing: {profile}", &[("profile", &running.name)]));
        let tone = state_tone(view);
        let said = view.said.clone().filter(|_| tone.is_none());
        let build_said = self.build_said();
        let receipt =
            instance.kind != instances::Kind::Pack && self.check_build_changes(&instance.home, cx);
        if said.is_none()
            && build_said.is_none()
            && other_running.is_none()
            && view.mods_off.is_none()
            && !receipt
        {
            return None;
        }
        let lines = div()
            .min_w_0()
            .text_size(px(tokens::LABEL))
            .text_color(rgb(text))
            .line_clamp(2)
            .text_ellipsis()
            .children(build_said)
            .children(other_running)
            .children(said)
            // Nothing to act on here: the row under Mods says why and what to do.
            .children(view.mods_off.map(|count| {
                div()
                    .id("alt-not-on-build")
                    .cursor_pointer()
                    .hover(move |s| s.text_color(rgb(link_ink)))
                    .child(match count {
                        1 => tr("1 mod not on this build").to_string(),
                        n => trf(
                            "{count} mods not on this build",
                            &[("count", &n.to_string())],
                        ),
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_installed_mods();
                        cx.notify();
                    }))
            }));
        let lines = self.ui.home.shade_behind(lines, scenery);
        Some(
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap(px(8.))
                .child(lines)
                .children(receipt.then(|| {
                    button(
                        "home-build-changes",
                        tr("Build changes"),
                        Look::new(Style::Secondary, Size::Small),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_build_changes(cx);
                        cx.notify();
                    }))
                })),
        )
    }
}

/// The line under the profile's name: "Last played today · Time played 3 h", or "Not played
/// yet".
fn identity_line(instance: &instances::Instance) -> String {
    match instance.last_played {
        Some(last) => format!(
            "{} · {}",
            trf(
                "Last played {when}",
                &[("when", &crate::worlds_page::ago(last))]
            ),
            trf(
                "Time played {time}",
                &[("time", &instances::play_time(instance.played))]
            ),
        ),
        None => tr("Not played yet").to_string(),
    }
}

/// The tone of the banner a profile state shows, if it shows one: a problem, a held update, an
/// update.
fn state_tone(view: &View) -> Option<Tone> {
    if view.failed || recovering(view) {
        return Some(Tone::Danger);
    }
    match view.act {
        Some(Act::Update) => Some(Tone::Info),
        Some(Act::Resolve) => Some(Tone::Warning),
        _ => None,
    }
}

/// A problem whose way out is the profile's folder: the banner carries its whole text.
fn recovering(view: &View) -> bool {
    matches!(
        view.act.as_ref(),
        Some(Act::Notice {
            way_out: Some(crate::sheets::WayOut::ProfileFolder(_)),
            ..
        })
    )
}

/// The banner under the profile banner for a problem or an update (with its one action), or
/// the action alone. None when the profile has neither.
fn profile_state(view: &View, cx: &mut Context<Launcher>) -> Option<Div> {
    let tone = state_tone(view);
    let notice = tone
        .zip(view.said.clone())
        .map(|(tone, said)| banner(tone, said, None));
    let act = view
        .act
        .clone()
        .filter(|act| *act != Act::StopBuild)
        .map(|act| act_button(act, cx));
    if notice.is_none() && act.is_none() {
        return None;
    }
    Some(
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .children(notice)
            .children(act),
    )
}

/// The one action a state offers, under its own name. A held update's and an update's are the
/// accent; the rest are secondary.
fn act_button(act: Act, cx: &mut Context<Launcher>) -> Stateful<Div> {
    let (id, style) = match act {
        Act::StopBuild => ("alt-build-stop", Style::Secondary),
        Act::Update => ("alt-update", Style::Primary),
        _ => ("alt-fix", Style::Secondary),
    };
    let label = act.label();
    button(id, label, Look::new(style, Size::Medium)).on_click(cx.listener(
        move |this, _, _, cx| {
            this.launch_act(&act);
            cx.notify();
        },
    ))
}
