//! Home: the selected profile, Play and its game build up front, and the profile's recent
//! worlds to continue.
//!
//! Everything shown is read from the player's real launcher data, and nothing is written until
//! the player acts. Play sets `quick_play` and calls `play_game`, which owns the sign-in,
//! install, prepare, build and trust gates:
//! - the header: the profile's mark and name, how long it was played and when last
//!   (`instances.json`), Play in every state `launch_state` knows with its one action beside
//!   it, and the build the profile plays, which opens the build chooser;
//! - worlds: `worlds::list` of the selected profile's `saves/` (`level.dat` and `icon.png`, read
//!   only), newest first; Play starts the game straight into one (`quick_play`), and the box
//!   opens its data packs;
//! - the shared artwork worker reads and decodes world pictures in the background, for rows
//!   and tile bands near the view; a world without `icon.png` uses `dirt.png` of the player's own
//!   prepared game files (`backend::resources`, nothing is shipped), else a plain tile;
//! - signed out with no worlds, the welcome stands where the worlds will be, beside the mods.
use super::*;
use crate::frame::button;
use crate::frame::Reach as _;
use crate::launch_state::Main;
use crate::paint::palette;
use crate::sheets::Sheet;
use crate::worlds_page;
use gpui::rgb;
use std::cell::Cell;
use std::rc::Rc;
use sunrise_launcher::worlds::World;
use sunrise_launcher_theme::glass::radius;

/// A world's picture in a row.
const WORLD_ICON: f32 = 40.;
/// And as a tile: its 64 pixels, doubled.
const WORLD_TILE: f32 = 128.;
/// How much of a tile's data packs button shows while nothing points at the tile.
const PACKS_AT_REST: f32 = 0.6;
/// The room between the header and the panel under it.
const GAP: f32 = 20.;
/// How far the header stands in from the panel's edge.
const INSET: f32 = 32.;

#[derive(Default)]
pub(crate) struct HomeState {
    /// Where the build picker was last drawn: the build chooser opens under it.
    picker: Rc<Cell<Option<Bounds<gpui::Pixels>>>>,
    /// The current frame's unboxed profile, measured before the backdrop is painted.
    hero: Rc<Cell<Option<Bounds<gpui::Pixels>>>>,
    /// What the selected profile has installed, for the Mods column.
    pub(crate) mods: crate::home_mods::Inventory,
    /// Where the worlds and the Mods rows were last drawn: only those near the window's view
    /// are built.
    worlds_at: Near,
    pub(crate) mods_at: Near,
}

/// A row of Home's lists, and the room to the next.
pub(crate) const ROW: f32 = 52.;
pub(crate) const ROW_GAP: f32 = 4.;
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
pub(crate) struct Near(Rc<Cell<Option<Bounds<gpui::Pixels>>>>);

impl Near {
    /// How wide the list was last drawn.
    fn width(&self) -> Option<f32> {
        self.0.get().map(|at| f32::from(at.size.width))
    }

    /// The items to build of `count`, and the list they stand in: `items` is given the built
    /// ones' place in `list`, between two blanks as tall as the ones left out.
    pub(crate) fn list(
        &self,
        count: usize,
        across: usize,
        tall: f32,
        gap: f32,
    ) -> (std::ops::Range<usize>, impl FnOnce(Div) -> Div) {
        let pitch = tall + gap;
        let view = crate::frame::scrolls("home").bounds();
        let built = match self.0.get() {
            Some(at) => near(
                count,
                across,
                pitch,
                (
                    f32::from(view.top() - at.top()),
                    f32::from(view.bottom() - at.top()),
                ),
            ),
            // Not drawn yet: a first screenful, measured by the next frame.
            None => 0..count.min(across * 12),
        };
        let bands = |items: usize| items.div_ceil(across) as f32;
        let blank =
            |items: usize| (items > 0).then(|| div().flex_none().h(px(bands(items) * pitch - gap)));
        let (above, below) = (blank(built.start), blank(count - built.end));
        let at = self.0.clone();
        let list = move |items: Div| {
            div()
                .relative()
                .flex()
                .flex_col()
                .gap(px(gap))
                .children(above)
                .child(items)
                .children(below)
                .child(
                    gpui::canvas(
                        move |bounds, window, cx| {
                            if at.replace(Some(bounds)) != Some(bounds) {
                                // The page moved: what is near is chosen again next frame.
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
    /// the picture stays unshaded away from the text and fades at all four edges.
    pub(crate) fn profile_scrim(&self) -> impl IntoElement {
        const BLUR: f32 = 24.;
        let hero = self.hero.clone();
        let shade = gpui::BoxShadow {
            color: Hsla::from(rgb(palette().hero.shade)).opacity(0.63),
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
                    window.paint_drop_shadows(
                        hero.dilate(px(3. * BLUR)),
                        Default::default(),
                        &[shade],
                    );
                }
            },
        )
        .absolute()
        .inset_0()
    }

    /// Whether Play and what it says show: false once Home is scrolled so far that the row
    /// they stand in is more than half above the page's top edge. The status strip then says
    /// it instead.
    pub(crate) fn play_in_view(&self) -> bool {
        play_shows(self.picker.get(), crate::frame::scrolls("home").bounds())
    }

    /// Where the build chooser opens: the left edge and the top, in the window.
    pub(crate) fn chooser_at(&self) -> (f32, f32) {
        match self.picker.get() {
            Some(picker) => (f32::from(picker.left()), f32::from(picker.bottom()) + 6.),
            // Home was not drawn yet; it is by the next frame.
            None => (INSET, 96.),
        }
    }
}

/// Whether the play row, drawn at `row` (as tall as the build picker in it), shows in the page
/// drawn at `view`. A Home not drawn yet shows it.
fn play_shows(row: Option<Bounds<gpui::Pixels>>, view: Bounds<gpui::Pixels>) -> bool {
    row.is_none_or(|row| row.center().y >= view.top())
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
        .border_color(rgb(palette().divider))
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

/// Whether Home shows the launcher-update notice, and for which version: one that is downloaded
/// and ready (`None`: Update applies it), or one the last check found that this copy can fetch
/// (`Some`: Update starts the download). Dismissed, mid-download, failed or not allowed: none.
fn launcher_notice(
    hidden: bool,
    state: &SelfUpdate,
    found: Option<&Release>,
    allowed: bool,
) -> Option<(String, Option<Release>)> {
    if hidden {
        return None;
    }
    match (state, found) {
        (SelfUpdate::Ready(version), _) => Some((version.clone(), None)),
        (SelfUpdate::Idle, Some(release)) if allowed => Some((
            backend::launcher_version(&release.tag)
                .unwrap_or_default()
                .to_string(),
            Some(release.clone()),
        )),
        _ => None,
    }
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
        self.load_worlds(instance.home.clone(), cx);
        let header = self.profile_header(&instance, window, cx);
        let body = self.continue_playing(&instance, window, cx);
        let notice = self.launcher_update_notice(cx);
        div()
            .id("alt-home")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&crate::frame::scrolls("home"))
            .flex()
            .flex_col()
            .gap(px(GAP))
            .pt(px(24.))
            // The profile sits on the scene, inset like the text of the panel under it.
            .child(header.flex_none().px(px(INSET)))
            .children(notice)
            // The panel is the sidebar's material and fills Home's box to its left, right and
            // bottom edges, so the frame lines it up with the sidebar. It reaches the foot when
            // the worlds leave room and is never squeezed: a short window scrolls the page.
            .child(
                div()
                    .flex_grow(1.)
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .p(px(22.))
                    .rounded(crate::tokens::r(crate::tokens::R_PANEL))
                    .bg(palette().chrome_plate())
                    .border_1()
                    .border_color(rgb(palette().divider))
                    .child(body),
            )
            .into_any_element()
    }

    /// A quiet line under the profile when a launcher update is ready: its version and Update,
    /// and a close that hides it until the launcher restarts. Settings keeps the same action.
    fn launcher_update_notice(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let (version, release) = launcher_notice(
            self.launcher_notice_hidden,
            &self.self_update,
            self.updates.launcher.as_ref(),
            backend::self_update_allowed(),
        )?;
        let update = button("alt-home-launcher-update", tr("Update")).on_click(cx.listener(
            move |this, _: &ClickEvent, _, cx| {
                match &release {
                    Some(release) => this.update_launcher(release.clone()),
                    None => this.apply_launcher_update(cx),
                }
                cx.notify();
            },
        ));
        let close = div()
            .id("alt-home-launcher-dismiss")
            .reach()
            .cursor_pointer()
            .px(px(8.))
            .text_size(px(tokens::SMALL))
            .text_color(rgb(palette().muted))
            .tooltip(crate::glass_ui::tip(tr("Hide until the launcher restarts")))
            .child("×")
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.launcher_notice_hidden = true;
                cx.notify();
            }));
        Some(
            div().px(px(INSET)).flex_none().child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(16.))
                    .py(px(10.))
                    .rounded(crate::tokens::r(crate::tokens::R_CARD))
                    .bg(palette().chrome_plate())
                    .border_1()
                    .border_color(rgb(palette().divider))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(tokens::LABEL))
                            .text_color(rgb(palette().text))
                            .child(crate::selectable::selectable(trf(
                                "Sunrise Launcher {version} available",
                                &[("version", &version)],
                            ))),
                    )
                    .child(update)
                    .child(close),
            ),
        )
    }

    /// The selected profile: its mark and name, its play time, then Play, the build it plays
    /// and what Play has to say.
    fn profile_header(
        &mut self,
        instance: &instances::Instance,
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
        let labels = div()
            .flex()
            .flex_col()
            .self_start()
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
                    .mt(px(12.))
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
                    .mt(px(10.))
                    .text_size(px(tokens::BODY))
                    .text_color(text)
                    .truncate()
                    .child(played),
            );
        div()
            .flex()
            .flex_col()
            .child(self.ui.home.shade_behind(labels, scenery))
            .child(self.play_row(instance, window, cx).mt(px(18.)))
    }

    /// Play, the build picker and profile shortcuts, then Play's status and the cat's lane.
    fn play_row(
        &mut self,
        instance: &instances::Instance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let view = self.launch_facts().view();
        let idle = view.main == Main::Play;
        let p = palette();
        let on = rgb(p.accent_on);
        // Xmilon's accent halo sits outside the button. Hover strengthens it without putting
        // a veil over the gradient or its text.
        let glow = move |hover| {
            vec![gpui::BoxShadow {
                color: Hsla::from(rgb(p.play_glow)).opacity(if hover { 0.78 } else { 0.58 }),
                offset: gpui::point(px(0.), px(if hover { 9. } else { 6. })),
                blur_radius: px(if hover { 32. } else { 22. }),
                spread_radius: px(-8.),
                inset: false,
            }]
        };
        let signed_out = view.main == Main::SignIn;
        let play = crate::frame::input(false)
            .id("alt-play")
            .flex_none()
            .justify_center()
            .gap(px(10.))
            .rounded(px(crate::glass_ui::rad(radius::LARGE)))
            .h(px(tokens::PLAY))
            .min_w(px(172.))
            .px(px(22.))
            .font_weight(FontWeight::BOLD)
            .text_size(px(tokens::LEAD))
            .text_color(on)
            // Signed out: plainly disabled, a muted fill with no glow.
            .when(!signed_out, |d| d.bg(p.play_gradient()).shadow(glow(false)))
            .when(signed_out, |d| {
                d.bg(rgb(p.hover))
                    .text_color(rgb(p.muted))
                    .opacity(0.8)
                    .cursor_default()
            })
            // Nothing is added to the page: the reason is only a tooltip on the disabled Play.
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
            crate::frame::input(false)
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
        let mods = icon_button("alt-quick-mods", "puzzle", tr("Manage mods")).on_click(
            cx.listener(|this, _, _, cx| {
                this.show_installed_mods();
                cx.notify();
            }),
        );
        let home = instance.home.clone();
        let folder = icon_button("alt-quick-folder", "folder-01", tr("Open folder")).on_click(
            cx.listener(move |_, _, _, cx| {
                crate::glass_toast::open(cx, &home);
                cx.notify();
            }),
        );
        let act = view.act.clone().map(|act| {
            let id = match act {
                crate::launch_state::Act::StopBuild => "alt-build-stop",
                crate::launch_state::Act::Update => "alt-update",
                _ => "alt-fix",
            };
            button(id, act.label()).on_click(cx.listener(move |this, _, _, cx| {
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
            .filter(|(id, _)| id != &instance.id)
            .and_then(|(id, _)| self.instances.list.iter().find(|profile| &profile.id == id))
            .map(|profile| trf("Playing: {profile}", &[("profile", &profile.name)]));
        let says = div()
            // As wide as what it says: the rest of the row is the cat's.
            .min_w_0()
            .text_size(px(tokens::LABEL))
            .text_color(rgb(status_ink))
            // Two lines fit beside Play; a longer message is cut, never drawn outside the row.
            .line_clamp(2)
            .text_ellipsis()
            .children(self.build_line(true, cx))
            .children(self.build_said())
            .children(other_running)
            .children(view.said.filter(|_| !recovering))
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
        let (lane, _) = self.cat(act.is_none(), false, window, cx);
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
        let tall = tokens::PLAY;
        let controls = div()
            .flex()
            .items_end()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .min_w_0()
                    .h(px(tall))
                    .child(play)
                    .child(self.build_picker(instance, cx))
                    .child(mods)
                    .child(folder)
                    .children(act)
                    .children(says),
            )
            .child(lane.ml(px(12.)).mb(px(-GAP - crate::cat::BELOW_FEET)));
        div()
            .flex()
            .flex_col()
            .child(controls)
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
        // A game built with mods reads as the build it was made from; the tooltip names
        // the actual tag, including when a pin isn't installed yet.
        let made_from = version
            .filter(|t| native_mods::is_native(t))
            .and_then(|t| crate::glass_instance::native_base(t));
        let build = version.map_or_else(
            || tr("Sunrise").into(),
            |tag| self.version_label(made_from.as_ref().unwrap_or(tag)),
        );
        let at = self.ui.home.picker.clone();
        div()
            .id("alt-build")
            .when_some(version.cloned(), |d, tag| {
                d.tooltip(crate::glass_ui::tip(tag))
            })
            .flex_none()
            .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
            .border_1()
            .border_color(palette().input_edge())
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
                    .text_size(px(tokens::BODY))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette().text))
                    .child(build),
            )
            .child(blocks::icon(
                "chevron-down",
                13.,
                rgb(palette().muted).into(),
            ))
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

    /// Home's lower panel: the selected profile's worlds (or what to do when it has none)
    /// beside its mods.
    fn continue_playing(
        &mut self,
        instance: &instances::Instance,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let list: Vec<World> = self.worlds.list.clone();
        let tiles = self.settings.world_layout == backend::WorldLayout::Tiles;
        let top = div()
            .h(px(tokens::CONTROL))
            .flex()
            .items_center()
            .child(heading(tr("Continue playing")));
        let body = if !list.is_empty() {
            let near = self.ui.home.worlds_at.clone();
            if tiles {
                // As many tiles to a band as the column is wide; the rest wrap under them.
                let across = near.width().map_or(4, |wide| {
                    ((wide + TILE_GAP) / (WORLD_TILE + TILE_GAP)).max(1.) as usize
                });
                let (built, place) = near.list(list.len(), across, TILE_TALL, TILE_GAP);
                let mut grid = div().flex().flex_wrap().gap(px(TILE_GAP));
                for world in &list[built] {
                    grid = grid.child(self.world_tile(world, window, cx));
                }
                place(grid)
            } else {
                let (built, place) = near.list(list.len(), 1, ROW, ROW_GAP);
                let mut rows = div().flex().flex_col().gap(px(ROW_GAP));
                for world in &list[built] {
                    rows = rows.child(self.world_row(world, window, cx));
                }
                place(rows)
            }
        } else if !self.worlds.fresh {
            // Still being read: no "none yet" until that is known.
            div()
        } else {
            let saves = instance.home.join("saves");
            div()
                .flex_grow(1.)
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .py(px(28.))
                .child(blocks::icon("globe", 28., rgb(palette().muted).into()))
                .child(heading(tr("No worlds here yet")))
                .child(
                    div()
                        .max_w(px(560.))
                        .text_center()
                        .text_size(px(tokens::LABEL))
                        .text_color(rgb(palette().muted))
                        .child(crate::selectable::selectable(tr(
                            "Press Play and make a world in the game. It shows up here, ready to continue.",
                        ))),
                )
                .child(
                    button("alt-worlds-folder", tr("Open folder"))
                        .mt(px(8.))
                        .on_click(move |_, _, cx| {
                            let _ = std::fs::create_dir_all(&saves);
                            crate::glass_toast::open(cx, &saves);
                        }),
                )
        };
        // The first start (signed out, nothing to continue) says welcome where the worlds will
        // be; what the profile has installed stays beside it.
        let first_start = self.accounts.selected().is_none() && self.worlds.list.is_empty();
        let worlds = div().flex_1().min_w_0().flex().flex_col().gap(px(10.));
        let worlds = if first_start {
            worlds.child(self.welcome(cx))
        } else {
            worlds.child(top).child(body)
        };
        let mods = self.home_mods(&instance.home, cx).flex_1().min_w_0();
        // Worlds, then what the profile plays with: side by side, or in that order down a
        // narrow window.
        let both = div()
            .flex_grow(1.)
            .flex()
            .gap(px(22.))
            .child(worlds)
            .child(mods);
        if crate::home_mods::side_by_side(f32::from(window.viewport_size().width)) {
            both
        } else {
            both.flex_col()
        }
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

    /// One world as a compact row: its picture, what it is, and its two small buttons, each
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
            button(id, "")
                .size(px(30.))
                .px_0()
                .child(blocks::icon(glyph, 14., rgb(palette().text).into()))
                .tooltip(crate::glass_ui::tip(tr(says)))
        };
        let packs = small("alt-world-packs", "archive", "Data packs").on_click(cx.listener(
            move |this, _: &ClickEvent, _, cx| {
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
            .min_h(px(ROW))
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
                    .child(muted(Self::world_facts(world))),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
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
        let packs = crate::frame::button_base("alt-world-packs", "")
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
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
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
        self.quick_play = Some((flag, value));
        self.play_game();
    }

    /// The first start: signed out, nothing to play yet. Nothing is read from outside the
    /// launcher's own data.
    fn welcome(&mut self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .pr(px(12.))
            .py(px(8.))
            .child(
                div()
                    .text_size(px(tokens::DISPLAY))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(palette().text))
                    .child(tr("Welcome to Sunrise")),
            )
            .child(
                div()
                    .text_size(px(tokens::BODY))
                    .text_color(rgb(palette().text))
                    .child(crate::selectable::selectable(tr(
                        "Sign in with the Microsoft account that owns Minecraft: Java Edition. Sunrise then installs itself and you can play.",
                    ))),
            )
            .child(
                div()
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(palette().muted))
                    .child(crate::selectable::selectable(tr(
                        "While you play, Sunrise counts you as playing once a minute. Nothing else is sent.",
                    ))),
            )
            .child(
                div().flex().child(
                    div()
                        .id("alt-privacy")
                        .reach()
                        .cursor_pointer()
                        .text_size(px(tokens::LABEL))
                        .text_color(rgb(palette().accent_ink))
                        .child(tr("Privacy choices"))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.settings_open(crate::settings::Section::Privacy);
                            this.navigate(crate::frame::Page::Settings);
                            cx.notify();
                        })),
                ),
            )
    }
}

#[cfg(test)]
#[path = "home/tests.rs"]
mod tests;
