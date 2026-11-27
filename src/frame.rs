//! The launcher frame owns navigation and the single launch transaction. Backends still
//! own the on-disk formats, Microsoft device flow, preparation and process lifecycle.
use super::*;
use crate::paint::palette;
use crate::sheets::Sheet;
use gpui::{rgb, AnimationExt};

mod page;
mod play;
mod title_bar;
pub(crate) use crate::widgets::focus::{c, focus_ring, Reach};
use crate::widgets::scrolls::scrolls_begin;
pub(crate) use crate::widgets::scrolls::{drawn_scroll, focused_bounds, scrolls, scrolls_with};
pub(crate) use page::{Page, State};

impl Launcher {
    /// Shows `page`, remembering the one left so Back returns to it.
    pub(super) fn navigate(&mut self, page: Page) {
        if page != self.ui.page {
            let from = self.ui.page;
            self.ui.remember(from);
        }
        self.open_page(page);
    }

    /// Back: shows the page navigated away from last. Returns whether there was one.
    pub(super) fn go_back(&mut self) -> bool {
        let Some(page) = self.ui.back.pop() else {
            return false;
        };
        self.open_page(page);
        true
    }

    /// Shows `page` without remembering the one left (`navigate` and Back do that).
    fn open_page(&mut self, page: Page) {
        let before = self.ui.page;
        self.multiplayer.target = None;
        match page {
            Page::Discover => self.open_mods(),
            Page::Packs => self.open_packs(),
            Page::Join | Page::Hosting => {
                self.ui.last_server = Some(page);
                self.ui.page = page;
            }
            Page::Skins => {
                self.skins_open();
                self.ui.page = page;
            }
            _ => self.ui.page = page,
        }
        if self.ui.page != before {
            if before == Page::Hosting {
                self.servers.reset_public_lookup();
            }
            self.storage_builds.confirm = None;
            self.refresh_open_storage_builds();
        }
    }

    pub(super) fn frame(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        crate::clock::set_preference(
            self.settings.reduce_motion,
            self.settings.reduced_graphics(),
        );
        cx.set_reduce_motion(crate::motion::still());
        crate::motion::note_power(cx);
        crate::motion::note_benchmark(self.bench.is_some());
        let size = window.viewport_size();
        self.window_width = f32::from(size.width);
        self.window_height = f32::from(size.height);
        self.scale = window.scale_factor();
        self.scale_factor = self.scale;
        self.window_long_side = self.window_width.max(self.window_height);
        let page = self.ui.page;
        // Tint the frame from the very same decoded picture that will be drawn behind the page.
        // The picture is behind every page, not Home's alone, so leaving Home keeps it.
        let photo = self.home_photo(window, cx);
        crate::paint::begin(
            &self.settings,
            &self.theme,
            photo.as_ref(),
            self.panel_opacity_preview(),
        );
        scrolls_begin();
        // Records start-up and UI frame times, and lets a benchmark window begin once the
        // window has drawn. Nothing on screen is read here: a benchmark reads where the build
        // list was left, and moves it, between frames (`bench_wheel`).
        let now = std::time::Instant::now();
        self.ui.rail_drawn = Some(self.rail_open_at(now));
        #[cfg(test)]
        if let Some(open) = self.ui.rail_pin {
            self.ui.rail_drawn = Some(open);
        }
        let gap = self.perf.frame(now);
        if let Some(bench) = &mut self.bench {
            bench.frame(now, gap);
        }
        // The scenery (or the player's picture) stands in the profile banner, not behind the
        // window: the window is the flat chrome and the page one solid panel.
        self.ui.home.art = (page == Page::Home && self.settings.scenery()).then(|| {
            // Sunrise draws the design's own banner art (mountains and pines that fade into the
            // panel themselves, the light look's copy into its light panel); a picture or
            // another theme stands its scene.
            let sunrise = photo.is_none()
                && matches!(
                    sunrise_launcher::themes::resolve(self.settings.theme.as_deref()),
                    sunrise_launcher::themes::Resolved::Builtin(t)
                        if t.id == sunrise_launcher::themes::DAWN_THEME.id
                );
            if sunrise {
                crate::home::Art::Fading(
                    div().absolute().inset_0().child(
                        gpui::img(if self.theme.dark {
                            "banner.svg"
                        } else {
                            "banner-light.svg"
                        })
                        .size_full()
                        .object_fit(gpui::ObjectFit::Cover),
                    ),
                )
            } else {
                crate::home::Art::Scene(self.ui.backdrop.render(
                    &self.settings,
                    &self.theme,
                    photo.clone(),
                    window,
                ))
            }
        });
        crate::selectable::begin();
        crate::tokens::set_roundness(self.corner_roundness());
        // The plates drawn with `glass::backdrop` (the title bar, dialogs' backdrops) take the
        // theme's own plate colour, not the Outline look's default.
        crate::glass::set_panel(self.theme.look.plate);
        self.modal_before(window, cx);
        self.servers.note_focus(window, cx);
        self.friend_focused = self.friend_focus.is_focused(window);
        if window.focused(cx).is_none() {
            window.focus(&self.root_focus, cx);
        }
        // Tooltips made from here to the layers belong to the page (`glass_ui::tip`).
        crate::glass_ui::tips_page(self.layer_open());
        let rail = self.sidebar(cx);
        if self.ui.rail.is_some_and(|fade| fade.moving(now)) {
            window.request_animation_frame();
        }
        let title_bar = self.title_bar(window, cx);
        let content = match page {
            Page::Settings => self.settings(window, cx),
            Page::Skins => self.skins_page(window, cx).into_any_element(),
            Page::Home => self.home(window, cx),
            Page::Join | Page::Hosting => {
                let body = if page == Page::Hosting {
                    self.hosting(cx)
                } else {
                    self.join(cx)
                };
                // Join and Host as underline sub-tabs, matching Mods and Packs.
                let this = cx.entity();
                let tabs = div().px(px(24.)).pt(px(20.)).child(
                    div()
                        .flex()
                        .items_center()
                        .border_b_1()
                        .border_color(palette().rule())
                        .child(
                            crate::widgets::nav::tabs(
                                "servers-tabs",
                                &[
                                    (Page::Join, tr("Join").into()),
                                    (Page::Hosting, tr("Host").into()),
                                ],
                                if page == Page::Join {
                                    Page::Join
                                } else {
                                    Page::Hosting
                                },
                                move |page, _, cx| {
                                    this.update(cx, |view, cx| {
                                        view.navigate(page);
                                        cx.notify();
                                    });
                                },
                            )
                            .seen(),
                        ),
                );
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .text_color(rgb(palette().text))
                    .child(tabs)
                    .child(body)
                    .into_any_element()
            }
            _ => self.mods(window, cx),
        };
        let content = if let Some(at) = self.ui.page_transition(page, now, crate::motion::still()) {
            static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
            let start = *START.get_or_init(Instant::now);
            let id = at.saturating_duration_since(start).as_micros() as usize;
            div()
                .relative()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .flex()
                .flex_col()
                .child(content)
                .with_animation(
                    ElementId::named_usize("page-turn", id),
                    crate::motion::eased(crate::motion::PAGE),
                    |d, p| d.opacity(p).top(px(tokens::RISE * (1. - p))),
                )
                .into_any_element()
        } else {
            content
        };
        let status = self.status_strip(cx);
        let status = if let Some(strip) = status {
            let since = *self.ui.status_strip_since.get_or_insert_with(Instant::now);
            static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
            let start = *START.get_or_init(Instant::now);
            let id = since.saturating_duration_since(start).as_micros() as usize;
            Some(
                div()
                    .id("status-strip-wrapper")
                    .flex_none()
                    .overflow_hidden()
                    .with_animation(
                        ElementId::named_usize("status-strip-enter", id),
                        crate::motion::eased(sunrise_launcher_theme::QUICK),
                        |d, p| {
                            if p < 1. {
                                d.max_h(px(64. * p)).opacity(p)
                            } else {
                                d.opacity(1.)
                            }
                        },
                    )
                    .child(strip),
            )
        } else {
            if self.ui.status_strip_since.take().is_some() {
                self.ui.status_strip_gone = Some(Instant::now());
            }
            let quick = sunrise_launcher_theme::QUICK;
            self.ui
                .status_strip_gone
                .filter(|gone| gone.elapsed() < quick.duration())
                .map(|gone| {
                    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
                    let start = *START.get_or_init(Instant::now);
                    let id = gone.saturating_duration_since(start).as_micros() as usize;
                    div().id("status-strip-closing").flex_none().with_animation(
                        ElementId::named_usize("status-strip-exit", id),
                        crate::motion::eased(quick),
                        |d, p| d.h(px((tokens::ACTION + 2.) * (1. - p))),
                    )
                })
        };
        let pane = div()
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .rounded(crate::tokens::r(crate::tokens::R_PANEL))
            .bg(rgb(palette().background))
            .border_1()
            .border_color(rgb(palette().divider))
            .overflow_hidden()
            .children(status)
            .children(self.registry_notice_strip(cx))
            .child(content);
        let mut frame = div()
            .id("launcher-frame")
            .track_focus(&self.root_focus)
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .text_color(rgb(palette().text))
            .font_family(sunrise_launcher_theme::glass::FONT)
            .text_size(px(tokens::BODY))
            .capture_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                let k = &ev.keystroke;
                // Alt+Left goes back, unless a layer (a dialog, a sheet, the search) has the keys.
                if k.modifiers.alt && k.key == "left" && !this.layer_open() && this.go_back() {
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                let command = if cfg!(target_os = "macos") {
                    k.modifiers.platform
                } else {
                    k.modifiers.control
                };
                if !command || k.modifiers.shift || k.modifiers.alt {
                    return;
                }
                match k.key.as_str() {
                    "k" if this.toggle_search() => {}
                    "b" if !this.layer_open() => this.toggle_sidebar(),
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match ev.keystroke.key.as_str() {
                    "tab" => {
                        if ev.keystroke.modifiers.shift {
                            window.focus_prev(cx)
                        } else {
                            window.focus_next(cx)
                        }
                    }
                    "escape" if this.viewer_close(cx) => {}
                    "left" if this.viewer_step(-1, cx) => {}
                    "right" if this.viewer_step(1, cx) => {}
                    "escape" if this.sign_in_screen.open => {
                        this.cancel_sign_in();
                        this.sign_in_screen.open = false;
                    }
                    "escape" if this.sheet_escape() => {}
                    "escape" if this.instances.dialog.is_some() => {
                        this.instance_escape();
                    }
                    "escape" if this.ui.page == Page::Hosting && this.hosting_escape() => {}
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .child(title_bar)
            .children(self.settings_save_notice(cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    // The sidebar runs flush to the window's edge; the page panel keeps a margin
                    // on its other sides (Figma: 12).
                    .pr(px(12.))
                    .pb(px(12.))
                    .child(rail)
                    .child(pane),
            );
        // What lies over the page, bottom to top. The top one has the keyboard (`modal`).
        crate::glass_ui::tips_layers();
        self.versions_focus(window);
        let create = (page == Page::Hosting)
            .then(|| self.hosting_create(cx))
            .flatten();
        let sheet = self.live_sheet(window, cx);
        let sign_in = self.sign_in_screen.open.then(|| self.owner_screen(cx));
        let profile = self.profile_dialog(window, cx);
        let mut layers: Vec<_> = [create, profile, sheet, sign_in]
            .into_iter()
            .flatten()
            .collect();
        let top = self.modal(layers.pop(), window, cx);
        frame = frame.children(layers).children(top);
        frame = frame.children(self.viewer(window, cx));
        frame = frame.children(self.toast_float(titlebar::HEIGHT));
        self.pictures.free_retired(window);
        div()
            .relative()
            .size_full()
            .bg(rgb(palette().chrome))
            .child(frame)
            .children(titlebar::resize_handles(window))
    }
}

#[cfg(test)]
mod tests {
    use super::Page;

    #[test]
    fn pack_views_keep_the_packs_tab_selected() {
        for page in [Page::Packs, Page::PackFind, Page::PackProject] {
            assert!(page.active_tab() == Page::Packs);
        }
        for page in [Page::Discover, Page::Installed, Page::Project] {
            assert!(page.active_tab() == Page::Discover);
        }
    }

    #[test]
    fn page_switch_transitions_only_when_destination_changes_and_rests_after_duration() {
        let mut state = super::State::default();
        let t0 = std::time::Instant::now();
        assert!(state.page_transition(Page::Home, t0, false).is_none());
        assert_eq!(state.page_switched.unwrap().0, Page::Home);
        let t1 = t0 + std::time::Duration::from_millis(50);
        assert_eq!(state.page_transition(Page::Settings, t1, false), Some(t1));
        assert_eq!(
            state.page_transition(
                Page::Settings,
                t1 + std::time::Duration::from_millis(100),
                false
            ),
            Some(t1)
        );
        assert!(state
            .page_transition(
                Page::Settings,
                t1 + std::time::Duration::from_millis(300),
                false
            )
            .is_none());
        // Reduced motion snaps without starting a transition.
        let t2 = t1 + std::time::Duration::from_millis(400);
        assert!(state.page_transition(Page::Home, t2, true).is_none());
        // Sub-tabs within the same top-level section do not replay the whole-page entrance.
        let t3 = t2 + std::time::Duration::from_millis(50);
        assert_eq!(state.page_transition(Page::Installed, t3, false), Some(t3));
        let t4 = t3 + std::time::Duration::from_millis(300);
        assert!(state.page_transition(Page::Discover, t4, false).is_none());
    }
}
