//! The profile page's banner: the profile's mark, name and build line, Play, Profile settings and
//! the cat's lane beside them. Under it: a build under way, and what the profile's state asks of
//! the player (a problem, a held update, an update).
use super::*;
use crate::launch_state::{Act, Main, View};
use crate::sheets::Sheet;
use crate::widgets::button::{button, icon_button_opening, Look, Size, Style};
use crate::widgets::feedback::{banner_button, progress, Banner, Tone};
use gpui::rgb;

/// The banner's height when nothing under the buttons adds to it (Figma: the art's 208).
const BANNER_HEIGHT: f32 = 208.;
/// Play's width (Figma).
const PLAY_WIDTH: f32 = 196.;
/// The art's size, and the dark strip along its bottom (`assets/media/banner.svg`, the path
/// `M0 193.818L1036 187.2V208H0Z`): the darkest band, under the trees, which the page's colour
/// fades into on the left. Its top edge runs from `GROUND_TOP_LEFT` at the left to
/// `GROUND_TOP_RIGHT` at the right; the trees stand above it, their feet at 176 to 189.
const ART_WIDTH: f32 = 1036.;
const GROUND_TOP_LEFT: f32 = 193.818;
const GROUND_TOP_RIGHT: f32 = 187.2;

/// How far above the banner's bottom the cat's feet stand, with `x` px from the banner's left:
/// the vertical middle of the dark strip, which is where the feet are, and the shadow
/// (`BELOW_FEET` under them) stays inside it while the strip is that deep.
///
/// The art is drawn to cover the banner, centred, so it is scaled by `max(width / 1036, 1)` and
/// the overflow is cropped evenly on every side. The strip is only about 14 to 21 px deep in the
/// art, and a wide banner crops its bottom part away (at 1190 px wide, 12 px of the art's 208
/// are gone from the bottom, leaving a strip of 5 to 11 px), so its depth is worked out for the
/// banner's width rather than fixed. It follows the strip's slope at `x`. A banner taller than
/// 208 (a build under way, a notice) scales the art further and is not accounted for.
fn ground_height(banner_width: f32, x: f32) -> f32 {
    let scale = (banner_width / ART_WIDTH).max(1.);
    let art_x = ((x + (ART_WIDTH * scale - banner_width) / 2.) / scale).clamp(0., ART_WIDTH);
    let along = art_x / ART_WIDTH;
    let top_in_art = GROUND_TOP_LEFT + (GROUND_TOP_RIGHT - GROUND_TOP_LEFT) * along;
    let crop = (BANNER_HEIGHT * scale - BANNER_HEIGHT) / 2.;
    let strip = BANNER_HEIGHT - (top_in_art * scale - crop);
    // The feet in the strip's middle. The lane drops below the banner's edge when the strip is
    // shallower than the shadow (the banner clips it); the paws stay on the banner.
    (strip / 2.).max(1.)
}

impl Launcher {
    /// The selected profile's top: the banner, then the state the profile is in.
    pub(super) fn profile_header(
        &mut self,
        instance: &instances::Instance,
        compact: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let facts = self.launch_facts();
        let view = facts.view();
        let banner = self.profile_banner(instance, compact, &view, window, cx);
        let state = profile_state(&view, crate::home::tabs::error_log(&facts), cx);
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(banner)
            .children(state)
    }

    /// The banner (Figma "Profile · …"): the scenery across it, fading into the panel at its
    /// left edge; on the window's middle the mark and name, the play time, then Play and Profile
    /// settings with the build under them, and the cat's lane beside them. Under the banner: a build under way and what the profile says.
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
            .items_center()
            .gap(px(8.))
            .child(controls)
            // The build picker is centred under Play and the gear together while it fits there.
            // Auto margins rather than centring, so a longer label starts at Play's left edge
            // instead of overhanging it.
            .child(
                div()
                    .flex()
                    .w(px(if compact { 132. } else { PLAY_WIDTH }
                        + 8.
                        + tokens::PLAY))
                    .child(
                        div()
                            .flex_none()
                            .mx_auto()
                            .child(self.change_build(build, tag, cx)),
                    ),
            );
        // The cat walks the ground along the art's bottom edge, right of the buttons.
        let (lane, _) = self.cat(true, false, window, cx);
        let pad = if compact { 16. } else { 24. };
        // Where the banner was last drawn; before the first frame, where the sidebar puts it.
        let measured = self.ui.home.banner_left.clone();
        let banner_left = measured
            .get()
            .unwrap_or_else(|| crate::rail::rail_width(self.rail_open()) + tokens::FRAME_INSET);
        let banner_width = self.window_width - banner_left - tokens::FRAME_INSET;
        // The name, Play and the build stand on the window's middle (the top bar's search is
        // there too), sidebar included; where that leaves them too little room on the left,
        // on the banner's middle.
        let controls_width = if compact { 132. } else { PLAY_WIDTH } + 8. + tokens::PLAY;
        let centre = centre_in_banner(self.window_width / 2. - banner_left, banner_width);
        let lane_left = centre + controls_width / 2. + 16.;
        // The lane's middle: the strip's depth hardly changes along it.
        let middle = (lane_left + banner_width - pad) / 2.;
        let feet = ground_height(banner_width, middle);
        let ground = div()
            .absolute()
            .left(px(lane_left))
            .right(px(pad))
            .bottom(px(feet - crate::cat::BELOW_FEET))
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
                // A scene runs across the banner under an even veil of the panel, so the name
                // and the buttons in its middle read on any picture; the left edge fades into
                // the panel as the design's own art does.
                crate::home::Art::Scene(art) => {
                    let veil = gpui::Hsla::from(rgb(panel));
                    d.child(art)
                        .child(div().absolute().inset_0().bg(veil.opacity(0.4)))
                        .child(div().absolute().inset_0().bg(gpui::linear_gradient(
                            90.,
                            gpui::linear_color_stop(veil.opacity(0.6), 0.),
                            gpui::linear_color_stop(veil.opacity(0.), 0.3),
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
                    .child(centred(identity, centre, pad))
                    .child(centred(actions, centre, pad))
                    .children(self.build_progress(view, cx))
                    .children(self.says(instance, view, false, cx)),
            )
            .child(ground)
            .child(left_edge(measured, cx))
    }

    /// The profile's mark beside its name, centred together, and under them, centred, when it
    /// was last played and for how long.
    fn identity(&mut self, instance: &instances::Instance) -> Div {
        let p = palette();
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .min_w_0()
                    .max_w_full()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .child(self.profile_mark(instance, 60.))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(tokens::HERO))
                            .line_height(px(tokens::HERO * 1.2))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(p.text))
                            .child(instance.name.clone()),
                    ),
            )
            .child(
                div()
                    .min_w_0()
                    .max_w_full()
                    .truncate()
                    .text_size(px(tokens::LABEL))
                    .text_color(rgb(p.muted))
                    .child(identity_line(instance)),
            )
    }

    /// The build's name and the tag it was made from (for the tooltip). A profile on Dev with no
    /// build says why it has none.
    fn build_name(&self, instance: &instances::Instance) -> (String, Option<String>) {
        // The chosen build, while it isn't the one played: Play then says how to get it.
        let version = self
            .pending_build(instance)
            .or_else(|| self.profile_target(instance));
        // A built mod image reads as its base; keep the actual tag in the tooltip.
        let base = version
            .as_deref()
            .filter(|t| native_mods::is_native(t))
            .and_then(crate::profile_builds::native_base);
        // The same test the chooser uses (`build_sheet`): an unpinned Stable profile with no
        // Stable build yet.
        let waits_for_stable = instance.version.is_none()
            && instance.channel == instances::Channel::Stable
            && !self.stable_available();
        let build: String = version.as_ref().map_or_else(
            // Dev never falls back to another channel, so say why it has nothing to play: the
            // build list is still on its way or didn't load, else the channel has no build yet
            // (sunrise-org/sunrise-launcher#417).
            || match (instance.channel, &self.releases) {
                (instances::Channel::Dev, Load::Loading) => tr("Dev · finding builds…").into(),
                (instances::Channel::Dev, Load::Failed(_)) => tr("Dev · builds didn't load").into(),
                (instances::Channel::Dev, Load::Ready(_)) => tr("Dev · no build yet").into(),
                // A Stable profile with no Stable build plays Nightly, as the chooser says.
                (instances::Channel::Stable, _) if waits_for_stable => trf(
                    "{channel} build",
                    &[(
                        "channel",
                        crate::sheets::channel_name(instances::Channel::Nightly),
                    )],
                ),
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
        // Say why it isn't Stable, with the chooser's own words; the build's tag follows.
        let version = if waits_for_stable {
            let hint = tr("No Stable build yet. Plays Nightly until there is one.");
            Some(match version {
                Some(tag) => format!("{hint}\n{tag}"),
                None => hint.into(),
            })
        } else {
            version
        };
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
        let build_open = self.ui.sheet == Some(Sheet::Build);
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
            .child(
                div()
                    .flex_none()
                    .child(crate::widgets::motion_bits::chevron(
                        "alt-build-chevron",
                        build_open,
                        14.,
                        rgb(p.muted).into(),
                    )),
            )
            .when_some(tag, |d, tag| {
                d.tooltip(crate::glass_ui::tip_unless(build_open, tag))
            })
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
        // It sinks a pixel while held, as the shared dropdown's field does.
        crate::widgets::press(change, &SharedString::from("alt-build"), |d| d.top(px(1.))).on_click(
            cx.listener(|this, _, _, cx| {
                if !this.launch_busy() && this.client.is_none() {
                    this.open_sheet(Sheet::Build);
                }
                cx.notify();
            }),
        )
    }

    /// Play: a Primary button at Play's height. It is still the one entry point for Play's
    /// state, label, tooltip and enabled flag; the cat sleeps on it when it is idle.
    fn play_button(&mut self, view: &View, compact: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        // Work Play waits for that can stop (a download, a preparation, the start): the button
        // is its Stop, and the status strip above carries the progress.
        if let Some(halt) = view.halt {
            let stopping = view.stopping;
            let label: SharedString = if stopping {
                tr("Stopping…").into()
            } else {
                tr("Stop").into()
            };
            let look = Look::new(Style::Secondary, Size::Medium).disabled(stopping);
            let stop = button("alt-play", label, look)
                .h(px(tokens::PLAY))
                .min_w(px(if compact { 132. } else { PLAY_WIDTH }))
                .px(px(if compact { 12. } else { 20. }))
                .text_size(px(tokens::LEAD))
                .font_weight(FontWeight::SEMIBOLD);
            return if stopping {
                stop
            } else {
                stop.on_click(cx.listener(move |this, _, _, cx| {
                    this.halt(halt);
                    cx.notify();
                }))
            };
        }
        // The play glyph stands before what starts the game, signing in first included.
        let idle = matches!(
            view.main,
            Main::Play | Main::BuildAndPlay | Main::SignIn | Main::Install
        );
        let look = Look::new(Style::Primary, Size::Medium).disabled(!view.enabled);
        // Figma: the play glyph before the label, while Play (or Sign in & play) is what it does.
        // Signed out, the button is the account chip's "Sign in" and does what that does.
        let signing_in = view.main == Main::SignIn;
        let look = match (idle, signing_in) {
            (_, true) => look.icon("user"),
            (true, false) => look.icon("play"),
            (false, false) => look,
        };
        let label: SharedString = if signing_in {
            tr("Sign in").into()
        } else {
            view.main.label()
        };
        let play = button("alt-play", label, look)
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
            play.on_click(cx.listener(move |this, _, _, cx| {
                this.cat_off_play();
                if signing_in {
                    this.account_action(crate::account_chip::Action::Add);
                } else {
                    this.press_play();
                }
                cx.notify();
            }))
        } else {
            play
        }
    }

    /// The profile's own options (Sunrise Extras, its folder), in a menu like the build chooser's.
    fn profile_options_button(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let at = self.ui.home.menu.clone();
        icon_button_opening(
            "alt-quick-menu",
            "settings-01",
            tr("Profile options"),
            Style::Secondary,
            self.ui.sheet == Some(Sheet::ProfileMenu),
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
/// Records where the banner's left edge is drawn; a move (the sidebar opening, a resize)
/// draws again, so the header follows within a frame.
fn left_edge(
    at: std::rc::Rc<std::cell::Cell<Option<f32>>>,
    cx: &mut Context<Launcher>,
) -> gpui::Canvas<()> {
    let this = cx.weak_entity();
    gpui::canvas(
        move |bounds, _, cx| {
            let left = f32::from(bounds.left());
            if at.replace(Some(left)) != Some(left) {
                let this = this.clone();
                cx.defer(move |cx| {
                    this.update(cx, |_, cx| cx.notify()).ok();
                });
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}

/// Where in a banner `width` wide the header's middle goes: `wanted` (the window's middle, from
/// the banner's left), unless that leaves less than `HALF` on a side; then the banner's middle.
fn centre_in_banner(wanted: f32, width: f32) -> f32 {
    const HALF: f32 = 200.;
    if wanted >= HALF && width - wanted >= HALF {
        wanted
    } else {
        width / 2.
    }
}

/// `content` centred on `centre` (from the banner's left edge), inside the banner's padding
/// `pad`: a box twice as wide as the distance to the nearer side, with the content in its middle.
fn centred(content: Div, centre: f32, pad: f32) -> Div {
    let half = (centre - pad).max(0.);
    div()
        .w(px(2. * half))
        .flex()
        .justify_center()
        .child(content.max_w_full())
}

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

/// The banner under the profile banner for a problem or an update, with its one action and, for
/// a problem that left a log, View logs; or the action alone. None when the profile has
/// neither. `log` is the log the problem left (`tabs::error_log`).
fn profile_state(
    view: &View,
    log: Option<crate::home::tabs::LogSource>,
    cx: &mut Context<Launcher>,
) -> Option<Div> {
    let tone = state_tone(view);
    let act = view.act.clone().filter(|act| *act != Act::StopBuild);
    if let Some((tone, said)) = tone.zip(view.said.clone()) {
        let mut strip = Banner::new(tone, said);
        if let Some(act) = act {
            strip = strip.action(act_button(act, cx));
        }
        if let Some(source) = log.filter(|_| tone == Tone::Danger) {
            strip = strip.action(banner_button("alt-view-logs", tr("View logs")).on_click(
                cx.listener(move |this, _, _, cx| {
                    this.show_logs(source);
                    cx.notify();
                }),
            ));
        }
        return Some(strip.build());
    }
    act.map(|act| div().child(act_button(act, cx)))
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
