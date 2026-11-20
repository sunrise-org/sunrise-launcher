//! The launcher frame owns navigation and the single launch transaction. Backends still
//! own the on-disk formats, Microsoft device flow, preparation and process lifecycle.
use super::*;
use crate::paint::palette;
use crate::sheets::Sheet;
use gpui::rgb;

#[derive(Clone, Copy, Default, Eq, PartialEq)]
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
    Versions,
    Settings,
}

impl Page {
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
    pub last_server: Option<Page>,
    pub after_install: Option<(crate::launcher_play::PlayIntent, String)>,
    /// A Play (including a destination) that explicitly asked for the sign-in now under way.
    pub after_sign_in: Option<String>,
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
}

thread_local! {
    static SCROLLS: std::cell::RefCell<Scrolls> = std::cell::RefCell::default();
}

/// Starts a frame: the pages that scroll name themselves again as they are built.
fn scrolls_begin() {
    SCROLLS.with_borrow_mut(|s| {
        let live = std::mem::take(&mut s.live);
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

/// The same for a page that keeps its own handle.
pub(crate) fn scrolls_with(handle: &gpui::ScrollHandle) {
    SCROLLS.with_borrow_mut(|s| s.live.push(("", handle.clone())));
}

/// Scrolls the page a control is on so the control shows, with a little room around it, when
/// the keyboard has just landed on it. `bounds` is where the control is.
fn scroll_to_show(bounds: Bounds<gpui::Pixels>, window: &mut Window, cx: &mut App) {
    const ROOM: f32 = 24.;
    if window.text_style().background_color != Some(focus_mark())
        || !window.last_input_was_keyboard()
    {
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

pub(super) fn button_base(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
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
        .child(label.into())
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
    pub(super) fn play_game(&mut self) {
        if self.client.is_some() || self.launch_busy() {
            return;
        }
        self.prepare_error = None;
        self.client_error = None;
        self.play_registry_notices.clear();
        self.crash = None;
        self.install_error = None;
        let Some(instance) = self.selected_instance().map(|i| i.id.clone()) else {
            return;
        };
        let intent = launcher_play::PlayIntent::new(instance, self.accounts.selected());
        if intent.account.is_none() && !sunrise_launcher::developer_offline::selected() {
            let destination = self.quick_play.take();
            let invite = self.joining.take();
            self.sign_in_to_play();
            self.quick_play = destination;
            self.joining = invite;
            self.ui.after_sign_in = Some(intent.instance);
            return;
        }
        self.resume_play(intent);
    }

    /// Continue the same accepted Play after a launch step; never recapture the account.
    pub(super) fn resume_play(&mut self, mut intent: launcher_play::PlayIntent) {
        if self.client.is_some() || self.launch_busy() {
            return;
        }
        let Some(instance) = self
            .selected_instance()
            .filter(|i| i.id == intent.instance)
            .cloned()
        else {
            return;
        };
        use sunrise_launcher::browser::sunrise::txn::{Lease, Target};
        let owner = match intent
            .owner
            .take()
            .map(Ok)
            .unwrap_or_else(|| Lease::admit(&instance.home).map(std::sync::Arc::new))
        {
            Ok(owner) if owner.home() == instance.home => owner,
            Ok(_) => return,
            Err(error) => {
                self.client_error = Some(error);
                return;
            }
        };
        let target = match owner.play_target(|| {
            let tag = self
                .target_version(instance.version.as_ref(), instance.channel)
                .ok_or_else(|| match &self.releases {
                    Load::Loading => {
                        tr("Checking available builds. Press Play when the list is ready.").into()
                    }
                    Load::Failed(error) => error.clone(),
                    Load::Ready(_) => {
                        tr("No downloadable build matches this profile. Choose a game build.")
                            .into()
                    }
                })?;
            Target::first_play(tag, |base| Ok(self.release_to_install(base)))
        }) {
            Ok(target) => target,
            Err(error) => {
                self.client_error = Some(error);
                return;
            }
        };
        intent.owner = Some(owner.clone());
        let base = target.base().to_owned();
        match owner.needs_bootstrap_download(&target) {
            Ok(true) => {
                self.ui.after_install = Some((intent, base));
                self.install_bootstrap(owner, target);
                return;
            }
            Err(error) => {
                self.client_error = Some(error);
                return;
            }
            Ok(false) => {}
        }
        let fresh = match owner.active_build() {
            Ok(active) => active.is_none(),
            Err(error) => {
                self.client_error = Some(error);
                return;
            }
        };
        if fresh && !backend::check_prepared(&base) {
            self.play_when_prepared = Some((base.clone(), intent));
            self.prepare_tag(base, backend::MinecraftFrom::Install, Some(owner));
            return;
        }
        let cancellation = native_mods::Cancellation::for_profile(owner.clone());
        self.native_cancellation = Some(cancellation.clone());
        self.native_for = Some(instance.id);
        self.native_error = None;
        self.native_step = Some(tr("Starting the build").into());
        self.play_after_build = Some(intent);
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result =
                owner.play_image(&target, instance.native_profile, &cancellation, |step| {
                    let _ = tx.send(Msg::NativeProgress(step));
                });
            let _ = tx.send(Msg::PlayImage(result));
        });
    }

    pub(super) fn launch_busy(&self) -> bool {
        self.launch_activity().busy()
    }

    pub(super) fn navigate(&mut self, page: Page) {
        match page {
            Page::Discover => self.open_mods(),
            Page::Packs => self.open_packs(),
            Page::Versions => {
                self.reset_versions();
                self.ui.page = page;
            }
            Page::Join | Page::Hosting => {
                self.ui.last_server = Some(page);
                self.ui.page = page;
            }
            _ => self.ui.page = page,
        }
    }

    fn header(&mut self, cx: &mut Context<Self>) -> Div {
        let p = palette();
        let active = self.ui.page.active_tab();
        let compact = self.window_width < 900. && self.settings.sidebar_expanded;
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
            (Page::Versions, "nav-versions", "archive", tr("Versions")),
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
                    .tooltip(crate::glass_ui::tip(label))
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
                    .child(self.account_chip(cx)),
            )
    }

    pub(super) fn frame(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        crate::clock::set_preference(
            self.settings.reduce_motion,
            self.settings.reduced_graphics(),
        );
        cx.set_reduce_motion(crate::motion::still());
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
        crate::selectable::begin();
        crate::tokens::set_roundness(self.corner_roundness());
        self.modal_before(window, cx);
        self.servers.note_focus(window, cx);
        self.friend_focused = self.friend_focus.is_focused(window);
        self.mods.note_focus(window);
        if window.focused(cx).is_none() {
            window.focus(&self.root_focus, cx);
        }
        let rail = self.sidebar(cx);
        let header = self.header(cx);
        let content = match page {
            Page::Settings => self.settings(window, cx),
            Page::Versions => self.versions_page(cx).into_any_element(),
            Page::Home => self.home(window, cx),
            Page::Join | Page::Hosting => {
                let body = if page == Page::Hosting {
                    self.hosting(cx)
                } else {
                    self.join(cx)
                };
                // The page's header, as Mods has it: its name, then Join and Host as one
                // control, the chosen one filled.
                let tabs = div()
                    .flex()
                    .items_center()
                    .gap(px(20.))
                    .px(px(24.))
                    .pt(px(24.))
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(tokens::TITLE))
                            .font_weight(FontWeight::BOLD)
                            .child(tr("Servers")),
                    )
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
                    ));
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
        let status = self.status_strip(cx);
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
                    "left" if this.viewer_step(-1, cx) => {}
                    "right" if this.viewer_step(1, cx) => {}
                    "escape" if this.owner_screen_open => {
                        this.cancel_sign_in();
                        this.owner_screen_open = false;
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
            .children(titlebar::render(&self.theme, window))
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
        let sign_in = self.owner_screen_open.then(|| self.owner_screen(cx));
        let profile = self.profile_dialog(window, cx);
        let mut layers: Vec<_> = [create, profile, sheet, sign_in]
            .into_iter()
            .flatten()
            .collect();
        let top = self.modal(layers.pop(), window, cx);
        frame = frame.children(layers).children(top);
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
    fn pack_views_keep_the_packs_tab_selected() {
        for page in [Page::Packs, Page::PackFind, Page::PackProject] {
            assert!(page.active_tab() == Page::Packs);
        }
        for page in [Page::Discover, Page::Installed, Page::Project] {
            assert!(page.active_tab() == Page::Discover);
        }
    }
}
