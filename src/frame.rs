//! The launcher frame owns navigation and the single launch transaction. Backends still
//! own the on-disk formats, Microsoft device flow, preparation and process lifecycle.
use super::*;
use crate::paint::palette;
use crate::sheets::Sheet;
use gpui::{rgb, AnimationExt};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum Page {
    #[default]
    Home,
    Discover,
    Installed,
    Packs,
    /// Packs › Discover (`Packs` is its Installed).
    PackFind,
    /// A pack's page, opened from Packs › Discover.
    PackProject,
    Project,
    Join,
    Hosting,
    /// The Skins tab (`skins_page`): the look of the account that plays.
    Skins,
    Settings,
}

impl Page {
    /// The pages `sunrise-launcher benchmark` switches through, as a player clicks them: the
    /// header's tabs, then Settings. Not Skins: opening it reads the account's profile (the
    /// keychain, Microsoft, Mojang) and keeps the skin worn, and a benchmark touches no account.
    pub(super) const BENCH: [Page; 5] = [
        Page::Home,
        Page::Discover,
        Page::Packs,
        Page::Join,
        Page::Settings,
    ];

    /// The tab that stays selected on a subpage.
    fn active_tab(self) -> Self {
        match self {
            Self::Installed | Self::Project => Self::Discover,
            Self::PackFind | Self::PackProject => Self::Packs,
            Self::Hosting => Self::Join,
            other => other,
        }
    }
}

#[derive(Default)]
pub(super) struct State {
    pub page: Page,
    pub home: crate::home::HomeState,
    pub backdrop: crate::backdrop::Backdrop,
    pub mods: crate::mods::ModsState,
    pub sheet: Option<Sheet>,
    pub sheets: crate::sheets::SheetsState,
    pub settings: crate::settings::SettingsView,
    pub cat: crate::cat::CatState,
    /// The Logs panel: open or not, its tab and the lines it has read.
    pub logs: crate::logs_panel::LogsPanel,
    pub last_server: Option<Page>,
    pub after_install: Option<(crate::launcher_play::PlayIntent, String)>,
    /// A Play (including a destination) that explicitly asked for the sign-in now under way.
    pub after_sign_in: Option<crate::launcher_play::PlayIntent>,
    /// The sidebar's last open or close, while it may still be easing there.
    pub rail: Option<crate::motion::Fade>,
    /// How open the sidebar is in the frame being drawn, or was in the last one: sampled once
    /// as each frame begins, so the rail, the top bar and the page lay out for the same width.
    pub rail_drawn: Option<f32>,
    /// Tests: draw the sidebar this far open, as if part way through an ease.
    #[cfg(test)]
    pub rail_pin: Option<f32>,
    /// Where the last frame drew each profile that has a context menu: the open menu covers
    /// them, so a right-click on one reaches it through here.
    pub profile_rows: crate::rail::ProfileRows,
    /// When the status strip appeared, so each appearance eases in once without jumping the page.
    pub status_strip_since: Option<Instant>,
    /// When the status strip went away, so its space closes as smoothly as it opened.
    pub status_strip_gone: Option<Instant>,
    /// The last drawn page and when we switched to it (None on first draw or once settled).
    pub page_switched: Option<(Page, Option<Instant>)>,
}

impl State {
    /// Returns the instant the current page entrance started while its transition is still active,
    /// or `None` on initial render, under reduced motion, or after [`crate::motion::PAGE`] ends.
    pub(super) fn page_transition(
        &mut self,
        page: Page,
        now: Instant,
        still: bool,
    ) -> Option<Instant> {
        match self.page_switched {
            Some((last_page, switched_at)) => {
                if last_page.active_tab() != page.active_tab() {
                    let started = (!still).then_some(now);
                    self.page_switched = Some((page, started));
                    started
                } else {
                    if last_page != page {
                        self.page_switched = Some((page, switched_at));
                    }
                    if still {
                        None
                    } else {
                        switched_at.filter(|&at| {
                            now.saturating_duration_since(at) < crate::motion::PAGE.duration()
                        })
                    }
                }
            }
            None => {
                self.page_switched = Some((page, None));
                None
            }
        }
    }
}

/// The keyboard's ring, in `color`, around a filled control that has no edge of its own to turn
/// gold. Behind a see-through control it would show as a fill: give that one an edge instead.
pub(super) fn focus_ring(color: u32) -> Vec<gpui::BoxShadow> {
    vec![gpui::BoxShadow {
        color: rgb(color).into(),
        offset: gpui::point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(2.),
        inset: false,
    }]
}

pub(super) fn button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    button_base(id, label).hover(|s| s.bg(rgb(palette().hover)))
}

/// A [`button`] that draws its own label (the launcher update's, with its icon): no text child,
/// which would take a share of the gap and push what it draws off centre.
pub(super) fn button_drawn(id: impl Into<ElementId>) -> Stateful<Div> {
    button_frame(id).hover(|s| s.bg(rgb(palette().hover)))
}

/// What makes a clickable thing that is not a button reachable by keyboard: a Tab stop that shows
/// the ring. Enter and Space press it, as a click does.
pub(crate) trait Reach {
    /// For a thing with no edge of its own: a gold line just inside it, which takes no room and
    /// is never cut off by what it sits in.
    fn reach(self) -> Self;
    /// For a thing that has an edge: that edge turns gold.
    fn reach_edge(self) -> Self;
    /// For every Tab stop: the page it is on scrolls to show it when the keyboard lands on it.
    fn seen(self) -> Self;
}

impl<T: InteractiveElement + ParentElement + Styled> Reach for T {
    fn reach(self) -> Self {
        self.tab_index(0)
            .rounded(crate::tokens::r(crate::tokens::R_SMALL))
            .focus_visible(|s| {
                s.shadow(vec![gpui::BoxShadow {
                    inset: true,
                    spread_radius: px(1.),
                    ..focus_ring(palette().focus).remove(0)
                }])
            })
            .seen()
    }

    fn reach_edge(self) -> Self {
        self.tab_index(0)
            .focus_visible(|s| s.border_color(rgb(palette().focus)))
            .seen()
    }

    fn seen(self) -> Self {
        // The toolkit tells nobody where the focused control is. So a focused control marks the
        // text style it hands its children (a background nothing can see), and a child that
        // covers it reads the mark while the frame is laid out, when it knows its place.
        self.focus(|s| s.text_bg(focus_mark())).child(
            gpui::canvas(scroll_to_show, |_, _, _, _| {})
                .absolute()
                .inset_0(),
        )
    }
}

fn focus_mark() -> gpui::Hsla {
    gpui::hsla(0.125, 0.5, 0.25, 0.)
}

/// The pages that scroll, for [`Reach::seen`].
#[derive(Default)]
struct Scrolls {
    /// A page's handle lives while the page is drawn, so coming back to a page starts at its top.
    kept: HashMap<&'static str, gpui::ScrollHandle>,
    /// The handles of the frame being drawn, and their names.
    live: Vec<(&'static str, gpui::ScrollHandle)>,
    /// The focused control that was last scrolled to. It is shown once, so the wheel can then
    /// move the page away from it.
    shown: Option<FocusHandle>,
    /// Where the frame being drawn put the focused control, once it has been laid out.
    focused: Option<Bounds<gpui::Pixels>>,
}

thread_local! {
    static SCROLLS: std::cell::RefCell<Scrolls> = std::cell::RefCell::default();
}

/// Starts a frame: the pages that scroll name themselves again as they are built.
fn scrolls_begin() {
    SCROLLS.with_borrow_mut(|s| {
        let live = std::mem::take(&mut s.live);
        s.focused = None;
        s.kept = live
            .into_iter()
            .filter(|(key, _)| !key.is_empty())
            .collect();
    });
}

/// The scroll handle of the page called `key`, for `track_scroll` beside its `overflow_y_scroll`:
/// what lets the keyboard's focus scroll the page.
pub(crate) fn scrolls(key: &'static str) -> gpui::ScrollHandle {
    SCROLLS.with_borrow_mut(|s| {
        let handle = match s.live.iter().find(|(live, _)| *live == key) {
            Some((_, handle)) => return handle.clone(),
            None => s.kept.get(key).cloned().unwrap_or_default(),
        };
        s.live.push((key, handle.clone()));
        handle
    })
}

/// The handle the last frame drew the page called `key` with, if it drew that page. Read
/// between frames: unlike [`scrolls`], it doesn't keep the page's handle for the next one.
pub(crate) fn drawn_scroll(key: &'static str) -> Option<gpui::ScrollHandle> {
    SCROLLS.with_borrow(|s| {
        s.live
            .iter()
            .find(|(live, _)| *live == key)
            .map(|(_, handle)| handle.clone())
    })
}

/// Where the frame being drawn has so far put the focused control, for a list that keeps the
/// row it is in built (`cull`).
pub(crate) fn focused_bounds() -> Option<Bounds<gpui::Pixels>> {
    SCROLLS.with_borrow(|s| s.focused)
}

/// The same for a page that keeps its own handle.
pub(crate) fn scrolls_with(handle: &gpui::ScrollHandle) {
    SCROLLS.with_borrow_mut(|s| s.live.push(("", handle.clone())));
}

/// Scrolls the page a control is on so the control shows, with a little room around it, when
/// the keyboard has just landed on it. `bounds` is where the control is.
fn scroll_to_show(bounds: Bounds<gpui::Pixels>, window: &mut Window, cx: &mut App) {
    const ROOM: f32 = 24.;
    if window.text_style().background_color != Some(focus_mark()) {
        return;
    }
    SCROLLS.with_borrow_mut(|s| s.focused = Some(bounds));
    if !window.last_input_was_keyboard() {
        return;
    }
    let focused = window.focused(cx);
    let mask = window.content_mask().bounds;
    let Some(page) = SCROLLS.with_borrow_mut(|s| {
        if s.shown == focused {
            return None;
        }
        s.shown = focused;
        // The page is the smallest one the control lies in. A control beside the pages (the
        // rail, the play bar, a sheet) is not cut off by any of them.
        s.live
            .iter()
            .map(|(_, page)| page)
            .filter(|page| {
                let view = page.bounds();
                let all = Bounds::new(
                    view.origin + page.offset(),
                    view.size + gpui::Size::from(page.max_offset()),
                );
                all.contains(&bounds.center()) && (mask.is_empty() || view.intersect(&mask) == mask)
            })
            .min_by(|a, b| {
                let area = |page: &gpui::ScrollHandle| {
                    let size = page.bounds().size;
                    f32::from(size.width) * f32::from(size.height)
                };
                area(a).total_cmp(&area(b))
            })
            .cloned()
    }) else {
        return;
    };
    let view = page.bounds();
    let by = if bounds.top() < view.top() + px(ROOM) {
        view.top() + px(ROOM) - bounds.top()
    } else if bounds.bottom() > view.bottom() - px(ROOM) {
        (view.bottom() - px(ROOM) - bounds.bottom()).max(view.top() + px(ROOM) - bounds.top())
    } else {
        return;
    };
    let mut to = page.offset();
    to.y = (to.y + by).clamp(-page.max_offset().y, px(0.));
    if to == page.offset() {
        return;
    }
    // A frame that is being drawn can't ask for another: do it right after.
    let window = window.window_handle();
    cx.defer(move |cx| {
        page.set_offset(to);
        window.update(cx, |_, window, _| window.refresh()).ok();
    });
}

/// A sheet's or a page's main action: cream, like Play signed out.
pub(super) fn primary(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    button_base(id, label)
        .bg(rgb(palette().text))
        .border_color(rgb(palette().text))
        .text_color(rgb(palette().background))
        .hover(|s| s.bg(rgb(palette().accent_on)))
        .focus_visible(|s| s.shadow(focus_ring(palette().focus)))
}

/// A copy button's face: `label`, or "Copied" with a tick on an accent tint while copy `shown`
/// (its count) shows. The focusable element stays in place while the label and decorative
/// children change, so keyboard focus stays on it.
pub(super) fn copy_face(id: &'static str, label: &str, shown: Option<usize>) -> Stateful<Div> {
    let accent: gpui::Hsla = rgb(palette().accent).into();
    let base = button_base(
        id,
        shown.map_or_else(|| label.to_string(), |_| tr("Copied").into()),
    )
    .relative()
    .gap(px(6.));
    match shown {
        Some(n) => base
            .border_color(accent.opacity(0.5))
            .hover(move |s| s.border_color(accent.opacity(0.75)))
            // Give keyboard focus its own ring so the Copied hover border cannot hide it.
            .focus_visible(|s| s.shadow(focus_ring(palette().focus)))
            .child(crate::code_box::tick(
                &SharedString::from(id),
                n,
                16.,
                rgb(palette().text).into(),
            ))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                    .with_animation(
                        ElementId::named_usize(SharedString::from(format!("{id}-tint")), n),
                        crate::code_box::played(Duration::from_millis(200)),
                        move |d, t| d.bg(accent.opacity(0.16 * t)),
                    ),
            ),
        None => base.hover(|s| s.bg(rgb(palette().hover))),
    }
}

/// A button that copies `text`, then shows the copy on itself for a moment, as [`copy_face`].
/// The copy is remembered with `text`, so a new text (another log, a new report) starts at
/// `label` again.
pub(super) fn copy_button(id: &'static str, label: &str, text: String) -> Stateful<Div> {
    let key = copy_key(id, &text);
    copy_face(id, label, crate::code_box::copied(&key)).on_click(move |_, window, cx| {
        crate::widgets::copy_quietly(cx, gpui::ClipboardItem::new_string(text.clone()));
        crate::code_box::note_copy(key.clone(), window, cx);
    })
}

fn copy_key(id: &str, text: &str) -> SharedString {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{id}:{:016x}", hasher.finish()).into()
}

pub(super) fn button_base(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
    button_frame(id).child(label.into())
}

/// Every button's box, without what it says.
fn button_frame(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .tab_index(0)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
        .border_1()
        .border_color(rgb(palette().line))
        .px(px(12.))
        .h(px(tokens::CONTROL))
        .text_color(rgb(palette().text))
        .text_size(px(tokens::LABEL))
        .font_weight(FontWeight::SEMIBOLD)
        .cursor_pointer()
        .focus_visible(|s| s.border_color(rgb(palette().focus)))
        .seen()
}

/// Every text input's box: one fill, one faint outline, the focus colour while typed in, its
/// text centred. The caller gives it its id, height and padding.
pub(crate) fn input(focused: bool) -> Div {
    div()
        .flex()
        .items_center()
        .overflow_hidden()
        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
        .bg(palette().input_fill())
        .border_1()
        .border_color(if focused {
            rgb(palette().focus).into()
        } else {
            palette().input_edge()
        })
}

impl Launcher {
    /// Also called for world and server destinations. Never send a destination through the
    /// legacy shortcuts, which skip some of the checks on the main Play action.
    pub(super) fn play_game(&mut self, destination: launcher_play::PlayDestination) {
        if self.client.is_some() || (self.launch_busy() && !self.registry_accepts_play()) {
            return;
        }
        self.prepare_error = None;
        self.client_error = None;
        self.play_registry_notices.clear();
        self.crash = None;
        self.install_error = None;
        let Some(instance) = self.selected_instance() else {
            return;
        };
        let kind = instance.kind;
        let intent =
            launcher_play::PlayIntent::new(instance, self.accounts.selected(), destination);
        if self.refuse_pack_play(kind) {
            return;
        }
        if intent.account.is_none() && !sunrise_launcher::developer_offline::selected() {
            self.sign_in_to_play();
            self.ui.after_sign_in = Some(intent);
            return;
        }
        self.resume_play(intent);
    }

    /// Continue the same accepted Play after a launch step; never recapture the account.
    pub(super) fn resume_play(&mut self, intent: launcher_play::PlayIntent) {
        let Some(instance) = self
            .selected_instance()
            .filter(|i| intent.matches_profile(i))
            .cloned()
        else {
            return;
        };
        if self.refuse_pack_play(instance.kind) {
            return;
        }
        if self.registry_accepts_play() {
            self.defer_registry_play(&instance, intent);
            return;
        }
        if self.client.is_some() || self.launch_busy() {
            return;
        }
        self.start_managed_client(instance, intent, None);
    }

    fn refuse_pack_play(&mut self, kind: instances::Kind) -> bool {
        if kind != instances::Kind::Pack {
            return false;
        }
        self.client_error = Some(sunrise_launcher::browser::sunrise::pack::play_unavailable());
        true
    }

    pub(super) fn play_after_journal_check(
        &mut self,
        intent: launcher_play::PlayIntent,
        home: &std::path::Path,
    ) {
        if self.client.is_some() || self.launch_busy() {
            return;
        }
        let Some(instance) = self
            .selected_instance()
            .filter(|i| intent.matches_profile(i) && i.home == home)
            .cloned()
        else {
            return;
        };
        let target = self.profile_target(&instance);
        let Some(tag) = target
            .as_ref()
            .filter(|tag| self.installed.contains(tag))
            .cloned()
        else {
            let release = target
                .as_deref()
                .and_then(|tag| self.release_to_install(tag));
            if let Some(release) = release {
                self.ui.after_install = Some((intent, release.tag.clone()));
                self.install(&release);
            } else {
                self.client_error = Some(match &self.releases {
                    Load::Loading => {
                        tr("Checking available builds. Press Play when the list is ready.").into()
                    }
                    Load::Failed(e) => e.clone(),
                    Load::Ready(_) => {
                        tr("No downloadable build matches this profile. Choose a game build.")
                            .into()
                    }
                });
                if matches!(self.releases, Load::Failed(_)) {
                    self.check_updates(UpdateCheck::Startup);
                }
            }
            return;
        };
        if !backend::check_prepared(&tag) {
            self.play_when_prepared = Some((tag, intent));
            self.prepare(backend::MinecraftFrom::Install);
            return;
        }
        self.native_error = None;
        if matches!(
            self.native_state_of(&instance),
            native_status::State::NotBuilt | native_status::State::Outdated
        ) {
            self.play_after_build = Some(intent);
            self.build_native();
        } else {
            self.play(intent);
        }
    }

    pub(super) fn launch_busy(&self) -> bool {
        self.launch_activity().busy()
    }

    pub(super) fn navigate(&mut self, page: Page) {
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
        self.finish_page_change(before);
    }

    pub(crate) fn finish_page_change(&mut self, before: Page) {
        if self.ui.page != before {
            if before == Page::Hosting {
                self.servers.reset_public_lookup();
            }
            self.storage_builds.confirm = None;
            self.storage_confirm = None;
            if self.ui.page == Page::Settings
                && self.ui.settings.section == settings::Section::Storage
            {
                self.refresh_storage(None, false);
            }
            self.refresh_open_storage_builds();
        }
    }

    fn header(&mut self, cx: &mut Context<Self>) -> Div {
        let p = palette();
        let active = self.ui.page.active_tab();
        // Below 900 px with the sidebar open; never with it closed (the window is 800 px at least).
        let compact = self.bar_width() < 750.;
        // Keep every destination readable; the account chip can shorten its label.
        let mut tabs = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(if self.window_width < 1000. { 2. } else { 10. }));
        for (dest, id, icon, label) in [
            (Page::Home, "nav-home", "home", tr("Home")),
            (Page::Discover, "nav-mods", "puzzle", tr("Mods")),
            (Page::Packs, "nav-packs", "package", tr("Packs")),
            (Page::Join, "nav-servers", "server", tr("Servers")),
            (Page::Skins, "nav-skins", "shirt", tr("Skins")),
        ] {
            let on = active == dest;
            let colour = if on { p.chrome_text } else { p.chrome_muted };
            tabs = tabs.child(
                div()
                    .id(id)
                    .reach()
                    .relative()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .gap(px(7.))
                    .px(px(if compact {
                        7.
                    } else if self.window_width < 1000. {
                        8.
                    } else {
                        14.
                    }))
                    .h(px(42.))
                    .whitespace_nowrap()
                    .text_color(rgb(colour))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(p.chrome_raised)))
                    .when(!compact, |d| {
                        d.child(blocks::icon(icon, 16., rgb(colour).into()))
                    })
                    .child(label)
                    .when(on, |d| {
                        d.child(
                            div()
                                .absolute()
                                .bottom_0()
                                .left(px(9.))
                                .right(px(9.))
                                .h(px(2.))
                                .bg(rgb(p.accent)),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let to = if dest == Page::Join {
                            this.ui.last_server.unwrap_or(dest)
                        } else {
                            dest
                        };
                        this.navigate(to);
                        cx.notify();
                    })),
            );
        }
        let update = self.launcher_update_control(cx);
        let compact_account = self.window_width < 970. && update.is_some();
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .px(px(10.))
            .h(px(tokens::HEADER))
            .rounded(crate::tokens::r(crate::tokens::R_PANEL))
            .border_1()
            .border_color(rgb(p.divider))
            .bg(p.chrome_plate())
            .child(tabs)
            .child(
                div()
                    .flex()
                    .min_w_0()
                    .items_center()
                    .gap(px(10.))
                    .child(self.search_button(cx))
                    .children(update)
                    .child(self.account_chip(compact_account, cx)),
            )
    }

    pub(super) fn frame(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        crate::clock::set_preference(
            self.settings.reduce_motion,
            self.settings.reduced_graphics(),
        );
        cx.set_reduce_motion(crate::motion::still());
        crate::motion::note_power(cx);
        let size = window.viewport_size();
        self.window_width = f32::from(size.width);
        self.window_height = f32::from(size.height);
        self.scale = window.scale_factor();
        self.scale_factor = self.scale;
        self.window_long_side = self.window_width.max(self.window_height);
        let page = self.ui.page;
        // Tint the frame from the very same decoded picture that will be drawn behind Home.
        let photo = (page == Page::Home)
            .then(|| self.home_photo(window, cx))
            .flatten();
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
        let rail = self.sidebar(cx);
        if self.ui.rail.is_some_and(|fade| fade.moving(now)) {
            window.request_animation_frame();
        }
        let header = self.header(cx);
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
                let tabs = div().px(px(24.)).pt(px(20.)).child(
                    div()
                        .flex()
                        .items_center()
                        .border_b_1()
                        .border_color(rgb(palette().divider))
                        .child(crate::mods::segmented(
                            "servers-tabs",
                            [
                                (
                                    "servers-join",
                                    tr("Join").into(),
                                    page == Page::Join,
                                    Page::Join,
                                ),
                                (
                                    "servers-host",
                                    tr("Host").into(),
                                    page == Page::Hosting,
                                    Page::Hosting,
                                ),
                            ],
                            Launcher::navigate,
                            cx,
                        )),
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
            .when(page != Page::Home, |d| {
                d.bg(palette().chrome_plate())
                    .border_1()
                    .border_color(rgb(palette().divider))
            })
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
                    // The Logs panel lies over every dialog while it is open.
                    "escape" if this.close_logs() => {}
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
            .children(titlebar::render(&self.theme, window, cx))
            .children(self.settings_save_notice(cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .p(px(tokens::FRAME_INSET))
                    .gap(px(12.))
                    .child(rail)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .gap(px(10.))
                            .child(header)
                            .child(pane),
                    ),
            );
        // What lies over the page, bottom to top. The top one has the keyboard (`modal`).
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
        // Over the dialogs too, so a sign-in that hangs shows its step while its dialog is up.
        frame = frame.children(self.logs_layer(window, cx));
        frame = frame.children(self.viewer(window, cx));
        frame = frame.children(self.toast_float());
        self.pictures.free_retired(window);
        let scenery = self
            .ui
            .backdrop
            .render(&self.settings, &self.theme, photo, window);
        div()
            .relative()
            .size_full()
            .child(scenery)
            .children(
                (page == Page::Home && self.settings.scenery())
                    .then(|| self.ui.home.profile_scrim()),
            )
            .child(frame)
            .children(titlebar::resize_handles(window))
    }
}

#[cfg(test)]
mod tests {
    use super::Page;

    #[test]
    fn a_new_text_starts_its_copy_button_at_its_label() {
        use super::copy_key;
        let log = copy_key("nightly-log-copy", "first log");
        assert_eq!(log, copy_key("nightly-log-copy", "first log"));
        assert_ne!(log, copy_key("nightly-log-copy", "second log"));
        assert_ne!(log, copy_key("receipt-copy", "first log"));
        crate::code_box::remember_copy(&log);
        assert!(crate::code_box::copied(&log).is_some());
        assert!(crate::code_box::copied(&copy_key("nightly-log-copy", "second log")).is_none());
    }

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

    #[test]
    fn leaving_storage_for_installed_mods_or_packs_cancels_counts_and_clears_confirmations() {
        let mut mods = crate::mods::ModsState::default();
        for to in [
            crate::mods::Installed::ModNamed("JEI".into()),
            crate::mods::Installed::PackNamed(
                sunrise_launcher::packs::Kind::Shader,
                "Meadow Shaders".into(),
            ),
        ] {
            let destination = mods.enter(&to);
            assert_ne!(destination, Page::Settings);
            let mut settings = crate::settings::SettingsView::default();
            let mut builds = crate::storage_builds::View::default();
            let count = crate::glass_storage::Work::count_for_test();
            let count_token = count.cancel_token();
            settings.storage_work = Some(count);
            let (generation, builds_cancel) = builds.start_count(false).unwrap();
            builds.confirm = Some(crate::storage_builds::Confirm::One("26.3-0.1.0".into()));
            let mut storage_confirm = Some(Some(sunrise_launcher::storage::Category::OtherCaches));

            crate::glass_storage::stop_storage_counts(
                &mut settings,
                &mut builds,
                &mut storage_confirm,
            );
            assert!(count_token.is_cancelled());
            assert!(!settings.storage_busy());
            assert!(builds_cancel.is_cancelled());
            assert!(builds.confirm.is_none());
            assert!(storage_confirm.is_none());
            builds.accept_rows(generation, Vec::new());
            assert!(!builds.has_rows());
        }
        // A Clear in flight survives leaving Storage while build counting still stops.
        let mut settings = crate::settings::SettingsView::default();
        let mut builds = crate::storage_builds::View::default();
        let clearing = crate::glass_storage::Work::clear_for_test();
        let clear_token = clearing.cancel_token();
        settings.storage_work = Some(clearing);
        let (_, builds_cancel) = builds.start_count(false).unwrap();
        let mut storage_confirm = Some(None);
        crate::glass_storage::stop_storage_counts(&mut settings, &mut builds, &mut storage_confirm);
        assert!(!clear_token.is_cancelled());
        assert!(settings.storage_busy());
        assert!(builds_cancel.is_cancelled());
        assert!(storage_confirm.is_none());
    }
}
