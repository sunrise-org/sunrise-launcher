//! The profile page's banner: the profile's mark, name and build line, Play, Profile settings and
//! the cat's lane beside them. Under it: a build under way, and what the profile's state asks of
//! the player (a problem, a held update, an update).
use super::*;
use crate::launch_state::{Act, Main, View};
use crate::sheets::Sheet;
use crate::widgets::button::{button, icon_button, Look, Size, Style};
use crate::widgets::feedback::{banner, progress, Tone};
use gpui::rgb;

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

    /// The banner: the identity on the left, then Play, Profile settings and the cat's lane.
    /// Under them, the build under way and what the profile says (or the receipt of a build).
    fn profile_banner(
        &mut self,
        instance: &instances::Instance,
        compact: bool,
        view: &View,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let scenery = self.settings.scenery();
        let (build, tag) = self.build_name(instance);
        let identity = self.identity(instance, &build, tag, scenery, cx);
        let controls = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(if compact { 8. } else { 12. }))
            .child(self.play_button(view, compact, cx))
            .child(profile_settings_button(instance, cx))
            .child(self.profile_options_button(cx));
        // The cat walks the room beside the controls and sleeps on Play.
        let (lane, _) = self.cat(true, false, window, cx);
        let row = div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(16.))
            .child(identity)
            .child(controls)
            // Its feet on the row's bottom line, as they stood on the old play bar's edge.
            .child(lane.ml(px(12.)).self_end().mb(px(-crate::cat::BELOW_FEET)));
        let frame = if scenery { div() } else { card() };
        frame
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(if compact { 14. } else { 20. }))
            .child(row)
            .children(self.build_progress(view, cx))
            .children(self.says(instance, view, scenery, cx))
    }

    /// The profile's mark, name and build line, with Change build under the line.
    fn identity(
        &mut self,
        instance: &instances::Instance,
        build: &str,
        tag: Option<String>,
        scenery: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = palette();
        let text = rgb(if scenery { p.hero.text } else { p.text });
        let ink = rgb(if scenery { p.hero.text } else { p.muted });
        let line = self.ui.home.shade_behind(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(tokens::SMALL))
                .text_color(ink)
                .child(identity_line(build.to_string(), instance)),
            scenery,
        );
        div()
            .max_w(px(460.))
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(16.))
            .child(self.profile_mark(instance, 56.))
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(tokens::TITLE))
                            .font_weight(FontWeight::BOLD)
                            .text_color(text)
                            .child(instance.name.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .min_w_0()
                            .child(line)
                            .child(self.change_build(tag, cx)),
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
                match (crate::sheets::dated_channel(shown), self.build_when(shown)) {
                    (Some(channel), Some(when)) => trf(
                        "{channel} · {version}",
                        &[("channel", channel), ("version", &when)],
                    ),
                    _ => self.version_label(shown),
                }
            },
        );
        (build, version)
    }

    /// "Change build": the build chooser opens under it.
    fn change_build(&mut self, tag: Option<String>, cx: &mut Context<Self>) -> Stateful<Div> {
        let at = self.ui.home.picker.clone();
        let change = button(
            "alt-build",
            tr("Change build"),
            Look::new(Style::Secondary, Size::Small),
        )
        .flex_none()
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
        let idle = matches!(view.main, Main::Play | Main::BuildAndPlay);
        let look = Look::new(Style::Primary, Size::Medium).disabled(!view.enabled);
        let look = if idle { look.icon("play") } else { look };
        let play = button("alt-play", view.main.label(), look)
            .h(px(tokens::PLAY))
            .min_w(px(if compact { 132. } else { 172. }))
            .px(px(if compact { 12. } else { 22. }))
            .text_size(px(tokens::LEAD))
            .font_weight(FontWeight::BOLD)
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

/// The build line: the build's name, then how long the profile was played, when it was.
fn identity_line(build: String, instance: &instances::Instance) -> String {
    match played_line(instance.played, instance.last_played) {
        Some((time, _)) => format!("{build}  ·  {time}"),
        None => build,
    }
}

/// Profile settings: the profile's edit dialog, as its menu's "Edit profile…".
fn profile_settings_button(
    instance: &instances::Instance,
    cx: &mut Context<Launcher>,
) -> Stateful<Div> {
    let target = instance.clone();
    button(
        "alt-profile-settings",
        tr("Profile settings"),
        Look::new(Style::Secondary, Size::Medium),
    )
    .flex_none()
    .on_click(cx.listener(move |this, _, _, cx| {
        this.edit_profile(&target, cx);
        cx.notify();
    }))
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
