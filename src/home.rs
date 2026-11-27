//! The selected profile, its launch controls, and four browsable collections.
use super::*;
use crate::frame::Reach as _;
use crate::launch_state::Main;
use crate::paint::palette;
use crate::sheets::Sheet;
use crate::widgets::button::secondary;
use crate::worlds_page;
use gpui::rgb;
use std::cell::Cell;
use std::rc::Rc;
use sunrise_launcher::worlds::World;
use sunrise_launcher_theme::glass::radius;

pub(crate) mod tabs;

/// A world's picture in a row.
const WORLD_ICON: f32 = 40.;
/// A world's card row, and the room between two: taller than the other tabs' rows, for the
/// picture and the two lines beside it.
const WORLD_ROW: f32 = 52.;
const WORLD_ROW_GAP: f32 = 4.;
/// And as a tile: its 64 pixels, doubled.
const WORLD_TILE: f32 = 128.;
/// How much of a tile's data packs button shows while nothing points at the tile.
const PACKS_AT_REST: f32 = 0.6;
/// The room between the header and the panel under it.
const GAP: f32 = 20.;
/// How far the header stands in from the panel's edge: as far as the panel's own text (its
/// 1 px edge and 22 px padding; the tabs panel pads `INSET - 1`), at every width and with the
/// sidebar open or closed.
const INSET: f32 = 1. + 22.;

#[derive(Default)]
pub(crate) struct HomeState {
    /// Where the build picker was last drawn: the build chooser opens under it.
    picker: Rc<Cell<Option<Bounds<gpui::Pixels>>>>,
    /// Where the profile menu's button was last drawn: the menu opens under it.
    menu: Rc<Cell<Option<Bounds<gpui::Pixels>>>>,
    /// The current frame's unboxed profile, measured before the backdrop is painted.
    hero: Rc<Cell<Option<Bounds<gpui::Pixels>>>>,
    /// What the selected profile has installed.
    pub(crate) mods: crate::home_mods::Inventory,
    tabs: tabs::Tabs,
}

/// A row in any profile tab.
pub(crate) const ROW: f32 = 40.;
/// A world's tile with its two lines, and the room between tiles.
const TILE_TALL: f32 = 180.;
const TILE_GAP: f32 = 16.;

/// Which of `count` items to build, `across` to a band and the bands `pitch` apart, when the
/// page shows the list from `from` to `to` (measured down from the list's top): the bands in
/// view and a few on either side, so the next Tab stop and the next turn of the wheel are there.
pub(crate) fn near(
    count: usize,
    across: usize,
    pitch: f32,
    (from, to): (f32, f32),
) -> std::ops::Range<usize> {
    const SPARE: f32 = 3.;
    let band = |y: f32| (y / pitch).max(0.) as usize;
    let first = band(from - SPARE * pitch) * across;
    let last = band(to + SPARE * pitch + pitch) * across;
    first.min(count)..last.min(count)
}

/// Where a long list of Home was last drawn. All of its items stay reachable (the list is as
/// tall as all of them) while only those near the view are built.
#[derive(Clone, Default)]
struct Near(Rc<Cell<Option<Bounds<gpui::Pixels>>>>);

impl Near {
    /// How wide the list was last drawn.
    fn width(&self) -> Option<f32> {
        self.0.get().map(|at| f32::from(at.size.width))
    }

    /// The items to build of `count`, and the list they stand in: `items` is given the built
    /// ones' place in `list`, between two blanks as tall as the ones left out.
    fn list(
        &self,
        scroll: &gpui::ScrollHandle,
        count: usize,
        across: usize,
        tall: f32,
        gap: f32,
    ) -> (std::ops::Range<usize>, impl FnOnce(Div) -> Div + use<>) {
        let pitch = tall + gap;
        let height = f32::from(scroll.bounds().size.height);
        let from = f32::from(-scroll.offset().y);
        let built = if height > 0. {
            near(count, across, pitch, (from, from + height))
        } else {
            // Not drawn yet: a first screenful, measured by the next frame.
            0..count.min(across * 12)
        };
        let bands = |items: usize| items.div_ceil(across) as f32;
        let blank =
            |items: usize| (items > 0).then(|| div().flex_none().h(px(bands(items) * pitch - gap)));
        let (above, below) = (blank(built.start), blank(count - built.end));
        let at = self.0.clone();
        let list = move |items: Div| {
            div()
                .relative()
                .flex_none()
                .flex()
                .flex_col()
                .gap(px(gap))
                .children(above)
                .child(items.flex_none())
                .children(below)
                .child(
                    gpui::canvas(
                        move |bounds, window, cx| {
                            if at.replace(Some(bounds)) != Some(bounds) {
                                // Measure the viewport on first paint and when it resizes.
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
        };
        (built, list)
    }
}

impl HomeState {
    /// Measure the uncovered text, not the full width of the header or the controls.
    fn shade_behind(&self, text: Div, on: bool) -> Div {
        text.when(on, |d| {
            let hero = self.hero.clone();
            d.relative().child(
                gpui::canvas(
                    move |bounds, _, _| {
                        if !bounds.is_empty() {
                            hero.set(Some(hero.get().map_or(bounds, |old| old.union(&bounds))));
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
        })
    }

    /// Feather the scene just behind the text. There is no fill or outline on the header;
    /// the picture stays unshaded away from the text, and the shade is a wide, soft oval with
    /// no edge to read as a box.
    pub(crate) fn profile_scrim(&self) -> impl IntoElement {
        const BLUR: f32 = 40.;
        let hero = self.hero.clone();
        let shade = gpui::BoxShadow {
            color: Hsla::from(rgb(palette().hero.shade))
                .opacity((palette().hero.shade_alpha + 0.05).min(1.)),
            offset: gpui::point(px(0.), px(0.)),
            blur_radius: px(BLUR),
            spread_radius: px(0.),
            inset: false,
        };
        gpui::canvas(
            |_, _, _| {},
            move |_, (), window, _| {
                if let Some(hero) = hero.get() {
                    // At three blur radii in from each edge, the text is in the shadow's
                    // full-strength centre, above the opacity Paint uses to fit the ink.
                    // Rounded to an oval: the text's corners still sit well inside it.
                    let area = hero.dilate(px(3. * BLUR));
                    let round = area.size.width.min(area.size.height) / 2.;
                    window.paint_drop_shadows(area, gpui::Corners::all(round), &[shade]);
                }
            },
        )
        .absolute()
        .inset_0()
    }

    /// Where the build chooser opens: the left edge and the top, in the window.
    pub(crate) fn chooser_at(&self) -> (f32, f32) {
        match self.picker.get() {
            Some(picker) => (f32::from(picker.left()), f32::from(picker.bottom()) + 6.),
            // Home was not drawn yet; it is by the next frame.
            None => (INSET, 96.),
        }
    }

    /// Where the profile menu opens: the left edge and the top, in the window.
    pub(crate) fn menu_at(&self) -> (f32, f32) {
        match self.menu.get() {
            Some(button) => (f32::from(button.left()), f32::from(button.bottom()) + 6.),
            None => (INSET, 96.),
        }
    }
}

pub(crate) fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// How long a profile was played and when last, as the line under its name. None until it
/// was played.
pub(crate) fn played_line(
    played: Duration,
    last: Option<std::time::SystemTime>,
) -> Option<(String, String)> {
    let last = last?;
    Some((
        trf("{time} played", &[("time", &instances::play_time(played))]),
        trf("Last played {when}", &[("when", &worlds_page::ago(last))]),
    ))
}

pub(crate) fn card() -> Div {
    div()
        .bg(palette().card())
        .border_1()
        .border_color(palette().rule())
        .rounded(crate::tokens::r(crate::tokens::R_CARD))
        .overflow_hidden()
}

pub(crate) fn heading(label: &str) -> Div {
    div()
        .text_size(px(tokens::HEADING))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(palette().text))
        .child(crate::selectable::selectable(label.to_string()))
}

pub(crate) fn muted(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(tokens::SMALL))
        .text_color(rgb(palette().muted))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(text.into())
}

impl Launcher {
    pub(crate) fn home(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.ui.home.hero.set(None);
        self.refresh_home_inventory(cx);
        let Some(instance) = self.selected_instance().cloned() else {
            return div()
                .p(px(32.))
                .text_color(rgb(palette().muted))
                .child(tr("Getting ready…"))
                .into_any_element();
        };
        self.ui.home.tabs.select_profile(&instance.id);
        let compact = self.window_width < 960. || self.window_height < 680.;
        let inset = if compact { 16. } else { INSET };
        let header = self.profile_header(&instance, compact, window, cx);
        let body = self.profile_tabs(&instance, inset, compact, window, cx);
        div()
            .id("alt-home")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(GAP))
            .pt(px(if compact { 12. } else { 24. }))
            // The profile sits on the scene, inset like the text of the panel under it.
            .child(header.flex_none().px(px(inset)))
            .child(body)
            .into_any_element()
    }

    /// The selected profile: its mark and name, its play time, then Play, the build it plays
    /// and what Play has to say.
    fn profile_header(
        &mut self,
        instance: &instances::Instance,
        compact: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let played = match played_line(instance.played, instance.last_played) {
            Some((time, last)) => format!("{time}  ·  {last}"),
            None => tr("Not played yet").to_string(),
        };
        let p = palette();
        let scenery = self.settings.scenery();
        let text = rgb(if scenery { p.hero.text } else { p.text });
        // Centred over the page, so the profile fills the scene rather than its left half.
        let labels = div()
            .flex()
            .flex_col()
            .items_center()
            .self_center()
            .max_w_full()
            .child(
                div()
                    .text_size(px(tokens::SMALL))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(text)
                    .child(tr("Selected profile")),
            )
            .child(
                div()
                    .mt(px(if compact { 6. } else { 12. }))
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .child(self.profile_mark(instance, 56.))
                    .child(
                        div()
                            .min_w_0()
                            .text_size(px(tokens::HERO))
                            .line_height(px(tokens::HERO * 1.15))
                            .font_weight(FontWeight::BOLD)
                            .text_color(text)
                            .truncate()
                            .child(instance.name.clone()),
                    ),
            )
            .child(
                div()
                    .mt(px(if compact { 5. } else { 10. }))
                    .max_w_full()
                    .text_size(px(tokens::BODY))
                    .text_color(text)
                    .truncate()
                    .child(played),
            );
        let receipt =
            instance.kind != instances::Kind::Pack && self.check_build_changes(&instance.home, cx);
        div()
            .flex()
            .flex_col()
            .child(self.ui.home.shade_behind(labels, scenery))
            .child(
                self.play_row(instance, compact, window, cx)
                    .mt(px(if compact { 12. } else { 18. })),
            )
            .children(receipt.then(|| {
                secondary("home-build-changes", tr("Build changes"))
                    .self_center()
                    .mt(px(10.))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_build_changes(cx);
                        cx.notify();
                    }))
            }))
    }

    /// Play, the build picker and profile shortcuts, then Play's status and the cat's lane.
    fn play_row(
        &mut self,
        instance: &instances::Instance,
        compact: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let view = self.launch_facts().view();
        let idle = matches!(view.main, Main::Play | Main::BuildAndPlay);
        let p = palette();
        let on = rgb(p.accent_on);
        // Xmilon's accent halo sits outside the button. Hover strengthens it without putting
        // a veil over the gradient or its text. Inside, the glass controls' top highlight: a
        // 1 px line of light along the upper edge, clear of the label.
        let glow = move |hover| {
            vec![
                gpui::BoxShadow {
                    color: Hsla::from(rgb(p.play_glow)).opacity(if hover { 0.78 } else { 0.58 }),
                    offset: gpui::point(px(0.), px(if hover { 9. } else { 6. })),
                    blur_radius: px(if hover { 32. } else { 22. }),
                    spread_radius: px(-8.),
                    inset: false,
                },
                gpui::BoxShadow {
                    color: gpui::white().opacity(0.28),
                    offset: gpui::point(px(0.), px(1.)),
                    blur_radius: px(0.),
                    spread_radius: px(0.),
                    inset: true,
                },
            ]
        };
        let play = crate::widgets::field::field_box(false, false)
            .id("alt-play")
            .flex_none()
            .justify_center()
            .gap(px(10.))
            .rounded(px(crate::glass_ui::rad(radius::LARGE)))
            .h(px(tokens::PLAY))
            .min_w(px(if compact { 132. } else { 172. }))
            .px(px(if compact { 12. } else { 22. }))
            .font_weight(FontWeight::BOLD)
            .text_size(px(tokens::LEAD))
            .text_color(on)
            // The glass edge every control has, lighter on the accent so it shows.
            .bg(p.play_gradient())
            .border_color(gpui::white().opacity(0.18))
            .shadow(glow(false))
            // Nothing is added to the page: signed out, the tooltip says Play signs in first.
            .when_some(view.main.tooltip(), |d, text| {
                d.tooltip(crate::glass_ui::tip(text))
            })
            .when(idle, |d| d.child(blocks::icon("play", 17., on.into())))
            .child(view.main.label())
            .when(view.enabled, |d| {
                d.cursor_pointer()
                    .hover(move |s| {
                        s.border_color(Hsla::from(on).opacity(0.25))
                            .shadow(glow(true))
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cat_off_play();
                        this.press_play();
                        cx.notify();
                    }))
            })
            .reach_edge();
        let icon_button = |id, icon, label| {
            crate::widgets::field::field_box(false, false)
                .id(id)
                .flex_none()
                .justify_center()
                .rounded(px(crate::glass_ui::rad(radius::LARGE)))
                .size(px(tokens::PLAY))
                .cursor_pointer()
                .hover(move |s| s.bg(rgb(p.hover)))
                .reach_edge()
                .tooltip(crate::glass_ui::tip(label))
                .child(crate::glass_ui::icon(icon, 21., rgb(p.text).into()))
        };
        // One button for the profile's own options (Sunrise Extras, its folder), in a menu
        // like the build chooser's. Mods has its tab in the top bar.
        let at = self.ui.home.menu.clone();
        let menu = icon_button("alt-quick-menu", "settings-01", tr("Profile options"))
            .when(self.ui.sheet == Some(Sheet::ProfileMenu), |d| {
                d.bg(rgb(p.hover))
            })
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
            }));
        let act = view.act.clone().map(|act| {
            let id = match act {
                crate::launch_state::Act::StopBuild => "alt-build-stop",
                crate::launch_state::Act::Update => "alt-update",
                _ => "alt-fix",
            };
            secondary(id, act.label()).on_click(cx.listener(move |this, _, _, cx| {
                this.launch_act(&act);
                cx.notify();
            }))
        });
        let p = palette();
        let (text, warning, link_ink) = if self.settings.scenery() {
            (p.hero.text, p.hero.warning, p.hero.link)
        } else {
            (p.text, p.warning, p.accent_ink)
        };
        let status_ink = if view.failed { warning } else { text };
        // Recovery needs the whole row, not the space left over after the profile shortcuts.
        let recovering = matches!(
            view.act.as_ref(),
            Some(crate::launch_state::Act::Notice {
                way_out: Some(crate::sheets::WayOut::ProfileFolder(_)),
                ..
            })
        );
        let recovery = recovering.then(|| {
            let line = div()
                .min_w_0()
                .text_size(px(tokens::LABEL))
                .text_color(rgb(status_ink))
                .children(view.said.clone());
            self.ui.home.shade_behind(line, self.settings.scenery())
        });
        let other_running = self
            .client_run
            .as_ref()
            .filter(|(running, _)| {
                running.id != instance.id || running.incarnation != instance.incarnation
            })
            .map(|(running, _)| trf("Playing: {profile}", &[("profile", &running.name)]));
        let other_elsewhere = other_running.is_some();
        let says = div()
            // As wide as what it says: the rest of the row is the cat's.
            .min_w_0()
            .text_size(px(tokens::LABEL))
            .text_color(rgb(status_ink))
            // Two lines fit beside Play; a longer message is cut, never drawn outside the row.
            .line_clamp(2)
            .text_ellipsis()
            .children(self.build_line(cx))
            .children(self.build_said())
            .children(other_running)
            .children(view.said.clone().filter(|_| !recovering))
            // Nothing to act on here: the row under Mods says why and what to do.
            .children(view.mods_off.map(|count| {
                div()
                    .id("alt-not-on-build")
                    .reach()
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
        let says = self.ui.home.shade_behind(says, self.settings.scenery());
        // The cat walks the empty stretch and stands on the panel's edge. It stays in while a
        // button stands in its way.
        let cat_beside = act.is_none();
        let (lane, _) = self.cat(cat_beside, false, window, cx);
        let has_status = view.said.is_some()
            || view.mods_off.is_some()
            || self.build_said().is_some()
            || self.native_step.is_some()
            || other_elsewhere;
        // The update target is part of the offer. On a narrow Home, put the whole offer
        // below Play instead of squeezing the target between the button and the edge.
        let narrow_update = matches!(&view.act, Some(crate::launch_state::Act::Update))
            && f32::from(window.viewport_size().width) < 920.;
        let (act, says, update_below) = if narrow_update {
            let line = div()
                .flex()
                .items_center()
                .gap(px(12.))
                .mt(px(8.))
                .children(act)
                .child(says);
            (None, None, Some(line))
        } else {
            (act, Some(says), None)
        };
        // A compact Home says it under Play, where the row has room for it.
        let (beside, under) = match says {
            Some(says) if compact && has_status => (None, Some(says.mt(px(6.)))),
            says => (says, None),
        };
        div()
            .flex()
            .flex_col()
            .items_center()
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_end()
                    // Matches the cat's lane and its margins, so the buttons sit in the middle.
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .mr(px(12.))
                            .when(cat_beside, |d| d.ml(px(-(14. + crate::cat::UNDER)))),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .justify_center()
                            .items_center()
                            .gap(px(if compact { 8. } else { 12. }))
                            .min_w_0()
                            .min_h(px(tokens::PLAY))
                            .child(play)
                            .child(self.build_picker(instance, cx))
                            .child(menu)
                            .children(act)
                            .children(beside),
                    )
                    .child(lane.ml(px(12.)).mb(px(-GAP - crate::cat::BELOW_FEET))),
            )
            .children(under)
            .children(update_below)
            .children(recovery.map(|line| line.mt(px(8.))))
    }

    /// The build the profile plays. Press to see how it follows new ones.
    fn build_picker(
        &mut self,
        instance: &instances::Instance,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let version = self.profile_target(instance);
        // A built mod image reads as its base; keep the actual tag in the tooltip.
        let base = version
            .as_deref()
            .filter(|t| native_mods::is_native(t))
            .and_then(crate::profile_builds::native_base);
        let build = version.as_ref().map_or_else(
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
        let at = self.ui.home.picker.clone();
        div()
            .id("alt-build")
            .when_some(version, |d, tag| d.tooltip(crate::glass_ui::tip(tag)))
            .min_w(px(120.))
            .max_w(px(340.))
            .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
            .border_1()
            .border_color(palette().plate_edge())
            .bg(palette().card())
            .reach_edge()
            .cursor_pointer()
            .hover(|s| s.bg(rgb(palette().hover)))
            .flex()
            .items_center()
            .gap(px(14.))
            .h(px(tokens::PLAY))
            .pl(px(16.))
            .pr(px(14.))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(tokens::BODY))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette().text))
                    .child(build),
            )
            .child(
                div()
                    .flex_none()
                    .child(crate::widgets::motion_bits::chevron(
                        "home-build",
                        self.ui.sheet == Some(Sheet::Build),
                        13.,
                        rgb(palette().muted).into(),
                    )),
            )
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
            )
            .on_click(cx.listener(|this, _, _, cx| {
                if !this.launch_busy() && this.client.is_none() {
                    this.open_sheet(Sheet::Build);
                }
                cx.notify();
            }))
    }

    /// A world's picture, `side` points square: its own `icon.png` (64 pixels square, as the
    /// game takes it) at a whole multiple of its pixels, so it stays sharp; else `dirt.png` of
    /// the player's own game files; else a plain tile with the map glyph.
    fn world_icon(&mut self, world: &World, side: f32) -> Div {
        let tile = div()
            .flex_none()
            .size(px(side))
            .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(palette().selected));
        let stamp = world
            .last_played
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let own = world.icon.clone().and_then(|icon| {
            let key = format!("alt-world:{}:{stamp}", icon.display());
            self.icon_image(key, side, move || Some(icon))
        });
        // Always dirt, from the player's own prepared files; none shipped.
        let picture = own.or_else(|| {
            let file = self
                .game_resources()
                .join("assets/minecraft/textures/block/dirt.png");
            let key = format!("alt-dirt:{}", file.display());
            self.icon_image(key, side, move || Some(file))
        });
        match picture {
            Some(image) => tile.child(gpui::img(image).size(px(side))),
            None => tile.child(blocks::icon("map", side * 0.4, rgb(palette().muted).into())),
        }
    }

    /// "Survival · Today".
    fn world_facts(world: &World) -> String {
        format!(
            "{} · {}",
            tr(world.mode),
            sentence(&worlds_page::ago(world.last_played))
        )
    }

    /// One world as a card row: its picture, what it is, and its two small buttons, each
    /// named by its tooltip. A name too long for the row is cut: it shows whole in a tooltip,
    /// and in the row itself while the keyboard is on Play.
    fn world_row(
        &mut self,
        world: &World,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let (folder, path) = (world.folder.clone(), world.path.clone());
        let focus = self
            .worlds
            .tile_focus
            .entry(world.folder.clone())
            .or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone();
        let on_play = focus.is_focused(window);
        // The row's own id scopes these two: no world's button is another's.
        let small = |id: &'static str, glyph: &'static str, says: &'static str| {
            secondary(id, "")
                .size(px(30.))
                .px_0()
                .child(blocks::icon(glyph, 14., rgb(palette().text).into()))
                .tooltip(crate::glass_ui::tip(tr(says)))
        };
        let packs = small("alt-world-packs", "archive", "Data packs").on_click(cx.listener(
            move |this, _: &ClickEvent, window, cx| {
                this.ui.home.tabs.remember(window.focused(cx));
                this.show_world_packs(path.clone());
                cx.notify();
            },
        ));
        let play = small("alt-world-play", "play", "Play this world")
            .track_focus(&focus)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.play_to("--world", folder.clone());
                cx.notify();
            }));
        card()
            .id(SharedString::from(format!("alt-world:{}", world.folder)))
            .flex_none()
            .h(px(WORLD_ROW))
            .flex()
            .items_center()
            .gap(px(10.))
            .p(px(5.))
            .pr(px(8.))
            .child(self.world_icon(world, WORLD_ICON))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("alt-world-name")
                            .text_size(px(tokens::LABEL))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette().text))
                            .when(!on_play, |d| d.truncate())
                            .child(world.name.clone())
                            .tooltip(crate::glass_ui::tip(world.name.clone())),
                    )
                    .child(muted(Self::world_facts(world)).truncate()),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .child(packs)
                    .child(play),
            )
    }

    /// One world as a tile: its picture, which plays it, with what it is under it. Its data
    /// packs button sits on the picture, faint until the pointer is over the tile or the
    /// keyboard is on the picture or on the button.
    fn world_tile(
        &mut self,
        world: &World,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let (folder, path) = (world.folder.clone(), world.path.clone());
        let focus = self
            .worlds
            .tile_focus
            .entry(world.folder.clone())
            .or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone();
        let group = SharedString::from(format!("alt-world-tile-{}", world.folder));
        let picture = div()
            .id("alt-world-play")
            .track_focus(&focus)
            .relative()
            .cursor_pointer()
            .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
            .focus_visible(|s| s.shadow(crate::frame::focus_ring(palette().focus)))
            .seen()
            .child(self.world_icon(world, WORLD_TILE))
            // What a press does, said on the picture itself.
            .child(
                div()
                    .absolute()
                    .right(px(8.))
                    .bottom(px(8.))
                    .size(px(32.))
                    .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgb(palette().text))
                    .child(blocks::icon("play", 15., rgb(palette().background).into())),
            )
            .tooltip(crate::glass_ui::tip(trf(
                "Play {world}",
                &[("world", &world.name)],
            )))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.play_to("--world", folder.clone());
                cx.notify();
            }));
        let packs = secondary("alt-world-packs", "")
            .absolute()
            .left(px(8.))
            .bottom(px(8.))
            .size(px(32.))
            .px_0()
            .bg(rgb(palette().raised))
            .occlude()
            .when(!focus.is_focused(window), |d| d.opacity(PACKS_AT_REST))
            .group_hover(group.clone(), |s| s.opacity(1.))
            .focus_visible(|s| s.opacity(1.).border_color(rgb(palette().focus)))
            .child(blocks::icon("archive", 17., rgb(palette().text).into()))
            .tooltip(crate::glass_ui::tip(tr("Data packs")))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.ui.home.tabs.remember(window.focused(cx));
                this.show_world_packs(path.clone());
                cx.notify();
            }));
        div()
            // The tile's own id scopes what is in it: no world's control is another's.
            .id(SharedString::from(format!("alt-world:{}", world.folder)))
            .flex_none()
            .w(px(WORLD_TILE))
            .h(px(TILE_TALL))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().group(group).relative().child(picture).child(packs))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        // A long name is cut at the picture's width: the tooltip has all of it.
                        div()
                            .id("alt-world-name")
                            .text_size(px(tokens::BODY))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette().text))
                            .truncate()
                            .child(world.name.clone())
                            .tooltip(crate::glass_ui::tip(world.name.clone())),
                    )
                    .child(muted(Self::world_facts(world))),
            )
    }

    /// Every destination goes through `play_game`, which runs the sign-in, install, prepare,
    /// build and trust gates before the game starts.
    pub(crate) fn play_to(&mut self, flag: &'static str, value: String) {
        self.play_game(launcher_play::PlayDestination::Quick(flag, value));
    }
}

#[cfg(test)]
mod tests;
