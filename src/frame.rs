//! The launcher frame owns navigation and the single launch transaction. Backends still
//! own the on-disk formats, Microsoft device flow, preparation and process lifecycle.
use super::*;
use crate::sheets::Sheet;
use gpui::rgb;

pub(super) use crate::tokens::{BAD, BG, CARD, CHROME, GOLD, LINE, MUTED, SOFT, TEXT};

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
    Hosting,
    Settings,
}

#[derive(Default)]
pub(super) struct State {
    pub page: Page,
    pub home: crate::home::HomeState,
    pub mods: crate::mods::ModsState,
    pub sheet: Option<Sheet>,
    pub sheets: crate::sheets::SheetsState,
    pub settings: crate::settings::SettingsView,
    pub cat: crate::cat::CatState,
    pub picker: bool,
    pub after_install: Option<(String, String)>,
    /// A Play (including a destination) that explicitly asked for the sign-in now under way.
    pub after_sign_in: Option<String>,
}

/// The first sentence of `text`, or its first line when that is shorter: what the play bar
/// has room to say.
fn first_sentence(text: &str) -> &str {
    let line = text.trim().lines().next().unwrap_or_default();
    match line.find(". ") {
        Some(end) => &line[..=end],
        None => line,
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
    button_base(id, label).hover(|s| s.bg(rgb(0x393233)))
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
            .rounded(px(tokens::R_SMALL))
            .focus_visible(|s| {
                s.shadow(vec![gpui::BoxShadow {
                    inset: true,
                    spread_radius: px(1.),
                    ..focus_ring(GOLD).remove(0)
                }])
            })
            .seen()
    }

    fn reach_edge(self) -> Self {
        self.tab_index(0)
            .focus_visible(|s| s.border_color(rgb(GOLD)))
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
        .bg(rgb(TEXT))
        .border_color(rgb(TEXT))
        .text_color(rgb(BG))
        .hover(|s| s.bg(gpui::white()))
        .focus_visible(|s| s.shadow(focus_ring(GOLD)))
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
        .rounded(px(tokens::R_CONTROL))
        .border_1()
        .border_color(rgb(LINE))
        .px(px(12.))
        .h(px(tokens::CONTROL))
        .text_color(rgb(TEXT))
        .text_size(px(tokens::LABEL))
        .font_weight(FontWeight::SEMIBOLD)
        .cursor_pointer()
        .focus_visible(|s| s.border_color(rgb(GOLD)))
        .seen()
        .child(label.into())
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
        self.crash = None;
        self.install_error = None;
        let Some(instance) = self.selected_instance().cloned() else {
            return;
        };
        if instance.java.is_some() {
            self.client_error =
                Some(tr("This setup does not run Sunrise. Choose a Sunrise setup.").into());
            return;
        }
        if self.accounts.selected().is_none() && !sunrise_launcher::developer_offline::selected() {
            let destination = self.quick_play.take();
            let invite = self.joining.take();
            self.sign_in_to_play();
            self.quick_play = destination;
            self.joining = invite;
            self.ui.after_sign_in = Some(instance.id);
            return;
        }
        let expected = instance.version.as_ref();
        let tag = self.version_of(&instance).cloned();
        let Some(tag) = tag else {
            let release = match &self.releases {
                Load::Ready(rows) => rows
                    .iter()
                    .find(|r| {
                        r.asset.is_some()
                            && expected.map_or_else(
                                || match instance.channel {
                                    instances::Channel::Stable => !r.prerelease,
                                    instances::Channel::Nightly => {
                                        sunrise_launcher::nightly::date_of(&r.tag).is_some()
                                    }
                                },
                                |tag| tag == &r.tag,
                            )
                    })
                    .cloned(),
                _ => None,
            };
            if let Some(release) = release {
                self.ui.after_install = Some((instance.id, release.tag.clone()));
                self.install(&release);
            } else {
                self.client_error = Some(match &self.releases {
                    Load::Loading => {
                        tr("Checking available builds. Press Play when the list is ready.").into()
                    }
                    Load::Failed(e) => e.clone(),
                    Load::Ready(_) => {
                        tr("No downloadable build matches this setup. Choose a game build.").into()
                    }
                });
                if matches!(self.releases, Load::Failed(_)) {
                    self.fetch_startup_releases();
                }
            }
            return;
        };
        if !backend::check_prepared(&tag) {
            self.play_when_prepared = Some((tag, instance.id));
            self.prepare(backend::MinecraftFrom::Install);
            return;
        }
        self.native_error = None;
        if matches!(
            self.native_state_of(&instance),
            native_status::State::NotBuilt | native_status::State::Outdated
        ) {
            self.play_after_build = Some(instance.id);
            self.build_native();
        } else {
            self.play();
        }
    }

    /// The selected setup's channel, when its last build failed because the build it stays on
    /// has no mod tools any more and another build is the way on (`sdk_cache::way_on`).
    fn build_gone(&self) -> Option<instances::Channel> {
        let error = self.native_error.as_ref()?;
        let instance = self.selected_instance()?;
        let mine = self.native_for.as_deref() == Some(instance.id.as_str());
        (mine
            && instance.version.is_some()
            && native_mods::sdk_cache::way_on(error)
                == Some(native_mods::sdk_cache::WayOn::OtherBuild))
        .then_some(instance.channel)
    }

    pub(super) fn launch_busy(&self) -> bool {
        self.mutating()
            || self.installing.is_some()
            || self.preparing
            || self.native_step.is_some()
            || self.starting_client.is_some()
    }

    fn dock(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let active = self.selected_instance().cloned();
        let name = self.accounts.selected().map(|a| a.name.clone());
        let signed_name = name.is_some();
        // The player's own face, hat layer and all, once it is read; their letter until then.
        let head = self
            .accounts
            .selected()
            .filter(|a| self.profile_heads.has(&a.uuid))
            .map(|a| {
                self.profile_heads
                    .draw(Some(&a.uuid), 32., self.scale, &self.theme)
            });
        let signed_in = signed_name || sunrise_launcher::developer_offline::selected();
        let version = active.as_ref().and_then(|i| self.version_of(i));
        // A game built with mods reads as the build it was made from, like any other.
        let made_from = version
            .filter(|t| native_mods::is_native(t))
            .and_then(|t| crate::glass_instance::native_base(t));
        let nightly = |t: &str| sunrise_launcher::nightly::date_of(t).is_some();
        let build = match version {
            Some(t) if nightly(made_from.as_deref().unwrap_or(t)) => trf(
                "Sunrise · {build}",
                &[("build", &crate::sheets::build_name(t))],
            ),
            Some(t) => self.version_label(t),
            None => tr("Sunrise").into(),
        };
        let follows = match &active {
            _ if version.is_none() && !signed_in => tr("Installs after you sign in"),
            _ if version.is_none() => tr("Not installed yet"),
            Some(i) if i.version.is_some() => tr("Stays on this build"),
            Some(i) if i.channel == instances::Channel::Stable => tr("Stable"),
            _ => tr("Nightly"),
        };
        let multiple = self
            .instances
            .list
            .iter()
            .filter(|i| i.java.is_none())
            .count()
            > 1
            || active.as_ref().is_some_and(|i| i.java.is_some());
        // A held update says so itself and its sheet carries the build's words: only then is
        // the build's error left to it.
        let held = self.held_update().is_some();
        let status = self
            .client_error
            .clone()
            .or_else(|| self.prepare_error.clone())
            .or_else(|| self.native_error.clone().filter(|_| !held))
            .or_else(|| self.install_error.as_ref().map(|(_, e)| e.clone()))
            .or_else(|| self.crash.as_ref().map(|c| c.summary.clone()));
        // The bar says a problem in one sentence; the rest opens as a sheet.
        let brief = status.as_deref().map(first_sentence);
        let more = status
            .clone()
            .filter(|all| brief.is_some_and(|b| b.len() < all.trim().len()));
        let busy = self.launch_busy();
        let label = if let Some((_, progress)) = &self.installing {
            trf(
                "Installing · {percent}%",
                &[("percent", &format!("{:.0}", 100. * progress))],
            )
        } else if self.preparing {
            tr("Preparing…").into()
        } else if let Some(step) = &self.native_step {
            let _ = step;
            tr("Building mods…").into()
        } else if self.starting_client.is_some() {
            tr("Starting…").into()
        } else if self.mutating() {
            tr("Changing mods…").into()
        } else if self.client.is_some() {
            tr("Stop game").into()
        } else if !signed_in {
            tr("Sign in with Microsoft").into()
        } else if version.is_none() {
            tr("Install and play").into()
        } else {
            tr("Play Sunrise").into()
        };
        let hold = self
            .held_update()
            .map(|h| trf("The update to {build} is on hold", &[("build", &h.to)]));
        let update = self.updates.sunrise.clone().filter(|_| {
            active.as_ref().is_some_and(|i| {
                i.java.is_none() && i.version.is_none() && i.channel == instances::Channel::Nightly
            }) && version.is_some()
                && !busy
                && self.client.is_none()
        });
        let said = brief
            .map(str::to_owned)
            .filter(|_| self.native_step.is_none())
            .or(hold.clone())
            .or_else(|| {
                update.as_ref().map(|r| {
                    trf(
                        "{build} is available",
                        &[("build", &crate::sheets::build_name(&r.tag))],
                    )
                })
            });
        // Mods the game was made without are no problem to raise: one quiet line, at most.
        self.mods_read_once();
        let waiting = (said.is_none() && self.native_step.is_none())
            .then(|| self.mods_not_on_build())
            .filter(|count| *count > 0);
        let quiet = said.is_none() && waiting.is_none() && self.build_line().is_none();
        let bar = div()
            .flex()
            .items_center()
            .gap(px(14.))
            .h(px(tokens::BAR))
            .flex_none()
            .px(px(16.))
            .bg(rgb(CHROME))
            // Signed out, the Play slot is the sign-in: no empty account beside it.
            .children(name.map(|name| {
                div()
                    .id("alt-who")
                    .reach()
                    .flex()
                    .items_center()
                    .gap(px(9.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .cursor_pointer()
                    .child(
                        div()
                            .flex_none()
                            .size(px(32.))
                            .rounded(px(tokens::R_CONTROL))
                            .overflow_hidden()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(rgb(0x45503c))
                            .children(head.is_none().then(|| {
                                name.chars()
                                    .next()
                                    .map(|c| c.to_uppercase().to_string())
                                    .unwrap_or_default()
                            }))
                            .children(head),
                    )
                    .child(name)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ui.page = Page::Settings;
                        cx.notify();
                    }))
            }))
            .when(multiple, |d| {
                d.child(
                    button(
                        "alt-setup",
                        active
                            .as_ref()
                            .map_or(tr("Setup").to_string(), |i| i.name.clone()),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ui.picker = !this.ui.picker;
                        cx.notify();
                    })),
                )
            })
            .when(signed_name || multiple, |d| {
                d.child(div().h(px(30.)).w(px(1.)).bg(rgb(LINE)))
            })
            .child(
                div()
                    .id("alt-build")
                    .tab_index(0)
                    .rounded(px(tokens::R_CONTROL))
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .focus_visible(|s| s.border_color(rgb(GOLD)))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap(px(9.))
                    .px(px(8.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(build))
                            .child(
                                div()
                                    .text_size(px(tokens::CAPTION))
                                    .text_color(rgb(MUTED))
                                    .child(follows),
                            ),
                    )
                    .child(blocks::icon("chevron-down", 13., rgb(MUTED).into()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if !this.launch_busy() && this.client.is_none() {
                            this.open_sheet(Sheet::Build);
                        }
                        cx.notify();
                    })),
            )
            .child(
                div()
                    // As wide as what it says: the rest of the bar is the cat's.
                    .min_w_0()
                    .text_size(px(tokens::LABEL))
                    .text_color(rgb(MUTED))
                    // Two lines fit the bar; a longer message is cut, never drawn outside it.
                    .line_clamp(2)
                    .text_ellipsis()
                    .children(self.build_line())
                    .children(said)
                    // Nothing to act on here: the row under Mods says why and what to do.
                    .children(waiting.map(|count| {
                        div()
                            .id("alt-not-on-build")
                            .reach()
                            .cursor_pointer()
                            .hover(|s| s.text_color(rgb(TEXT)))
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
                    })),
            );
        // One action at most, and one name for it: "Fix…" opens the place where the problem
        // can be dealt with, "Details…" a problem with nothing to do but read it.
        let way_out = if status.is_none() {
            None
        } else if self.prepare_error.is_some() {
            Some(crate::sheets::WayOut::MinecraftFolder)
        } else {
            self.build_gone()
                .filter(|_| !busy)
                .map(crate::sheets::WayOut::Newest)
        };
        let action = if self.native_step.is_some() {
            Some(
                button("alt-build-stop", tr("Stop")).on_click(cx.listener(|_, _, _, cx| {
                    native_mods::cancel();
                    cx.notify();
                })),
            )
        } else if let Some(all) = status
            .clone()
            .filter(|_| way_out.is_some() || more.is_some())
        {
            let label = if way_out.is_some() {
                tr("Fix…")
            } else {
                tr("Details…")
            };
            Some(
                button("alt-fix", label).on_click(cx.listener(move |this, _, _, cx| {
                    this.open_notice(all.clone(), way_out);
                    cx.notify();
                })),
            )
        } else if hold.is_some() && !busy {
            Some(
                button("alt-fix", tr("Fix…")).on_click(cx.listener(|this, _, _, cx| {
                    this.open_sheet(Sheet::Resolve);
                    cx.notify();
                })),
            )
        } else {
            update.map(|release| {
                button("alt-update", tr("Update game")).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.install_update_if_safe(&release);
                        cx.notify();
                    },
                ))
            })
        };
        let enabled = !busy && self.location_error.is_none();
        let ink = rgb(0x221203);
        // The cat may sleep on Play only while Play is idle and the bar has nothing to say.
        let free = quiet && !busy && self.client.is_none() && self.location_error.is_none();
        let (lane, on_play) = self.cat(action.is_none(), free, window, cx);
        let play = div()
            .id("alt-play")
            .tab_index(0)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .rounded(px(tokens::R_CARD))
            .h(px(tokens::PLAY))
            .min_w(px(160.))
            .px(px(20.))
            .font_weight(FontWeight::BOLD)
            .text_size(px(tokens::LEAD))
            .text_color(ink)
            .bg(if signed_in { rgb(GOLD) } else { rgb(TEXT) })
            .when(signed_in && !busy && self.client.is_none(), |d| {
                d.child(blocks::icon("play", 17., ink.into()))
            })
            .child(label)
            .focus_visible(move |s| s.shadow(focus_ring(if signed_in { TEXT } else { GOLD })))
            .when(!enabled, |d| d.opacity(0.6))
            .when(enabled, |d| {
                d.cursor_pointer().on_click(cx.listener(|this, _, _, cx| {
                    if let Some(client) = &this.client {
                        client.kill();
                    } else {
                        this.quick_play = None;
                        this.joining = None;
                        this.play_game();
                    }
                    cx.notify();
                }))
            });
        bar.child(lane).children(action).child(
            div()
                .relative()
                .flex_none()
                // A press on Play sends a cat asleep on it off; the press itself is Play's.
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.cat_off_play();
                        cx.notify();
                    }),
                )
                .child(play)
                .children(on_play),
        )
    }

    fn install_update_if_safe(&mut self, release: &Release) {
        let Some(instance) = self.selected_instance() else {
            return;
        };
        if self.native_state_of(instance) != native_status::State::NoMods {
            // Never overwrite a working native build while staging its candidate.
            self.instance_install = Some(release.tag.clone());
            self.install(release);
        } else {
            self.install_update();
        }
    }

    pub(super) fn frame(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let size = window.viewport_size();
        self.window_width = f32::from(size.width);
        self.window_height = f32::from(size.height);
        self.scale = window.scale_factor();
        self.scale_factor = self.scale;
        self.window_long_side = self.window_width.max(self.window_height);
        scrolls_begin();
        self.modal_before(window, cx);
        self.servers.note_focus(window, cx);
        self.friend_focused = self.friend_focus.is_focused(window);
        self.mods.note_focus(window);
        if window.focused(cx).is_none() {
            window.focus(&self.root_focus, cx);
        }
        let page = self.ui.page;
        let rail_link = |p: Page, id: &'static str, label: &str, icon: &'static str| {
            let on = page == p
                || (p == Page::Discover && matches!(page, Page::Installed | Page::Project))
                || (p == Page::Packs && matches!(page, Page::PackFind | Page::PackProject));
            div()
                .id(id)
                .tab_index(0)
                .w(px(56.))
                .h(px(56.))
                .rounded(px(tokens::R_CARD))
                .border_1()
                .border_color(gpui::transparent_black())
                .focus_visible(|s| s.border_color(rgb(GOLD)))
                .relative()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(3.))
                .cursor_pointer()
                .text_size(px(tokens::CAPTION))
                .text_color(rgb(if on { TEXT } else { MUTED }))
                .font_weight(FontWeight::SEMIBOLD)
                .when(on, |d| {
                    d.bg(rgb(CARD)).child(
                        div()
                            .absolute()
                            .left(px(-8.))
                            .h(px(26.))
                            .w(px(3.))
                            .bg(rgb(GOLD)),
                    )
                })
                .child(blocks::icon(
                    icon,
                    20.,
                    rgb(if on { TEXT } else { MUTED }).into(),
                ))
                .child(label.to_string())
                .on_click(cx.listener(move |this, _, _, cx| {
                    match p {
                        Page::Discover => this.open_mods(),
                        Page::Packs => this.open_packs(),
                        _ => this.ui.page = p,
                    }
                    this.ui.picker = false;
                    cx.notify();
                }))
        };
        let rail = div()
            .flex()
            .flex_col()
            .items_center()
            .flex_none()
            .w(px(tokens::RAIL))
            .py(px(14.))
            .gap(px(4.))
            .child(div().mb(px(12.)).child(crate::mark(36.)))
            .child(rail_link(Page::Home, "alt-nav-home", tr("Home"), "home"))
            .child(rail_link(
                Page::Discover,
                "alt-nav-mods",
                tr("Mods"),
                "package",
            ))
            .child(rail_link(
                Page::Packs,
                "alt-nav-packs",
                tr("Packs"),
                "archive",
            ))
            .child(rail_link(
                Page::Hosting,
                "alt-nav-hosting",
                tr("Host"),
                "server",
            ))
            .child(div().flex_1())
            .child(rail_link(
                Page::Settings,
                "alt-nav-settings",
                tr("Settings"),
                "sliders",
            ));
        let content = match page {
            Page::Settings => self.settings(window, cx),
            Page::Hosting => self.hosting(cx),
            _ if self.selected_instance().is_some_and(|i| i.java.is_some()) => div()
                .p(px(24.))
                .child(tr(
                    "Choose a Sunrise setup in the play bar. Your other setups haven't changed.",
                ))
                .into_any_element(),
            Page::Home => self.home(window, cx),
            _ => self.mods(window, cx),
        };
        let page_body = div()
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .rounded_bl(px(tokens::R_PANEL))
            .bg(rgb(BG))
            .border_1()
            .border_color(rgb(SOFT))
            .overflow_hidden()
            .child(content);
        let mut frame = div()
            .id("alt-root")
            .track_focus(&self.root_focus)
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(CHROME))
            .text_color(rgb(TEXT))
            .font_family("Figtree")
            .text_size(px(tokens::BODY))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match ev.keystroke.key.as_str() {
                    "tab" => {
                        if ev.keystroke.modifiers.shift {
                            window.focus_prev(cx)
                        } else {
                            window.focus_next(cx)
                        }
                    }
                    "escape" if this.owner_screen_open => {
                        this.cancel_sign_in();
                        this.owner_screen_open = false;
                    }
                    "escape" if this.sheet_escape() => {}
                    "escape" if this.ui.page == Page::Hosting && this.hosting_escape() => {}
                    "escape" => {
                        this.ui.sheet = None;
                        this.ui.picker = false;
                    }
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .children(titlebar::render(&self.theme, window))
            .children(self.settings_save_notice(cx))
            .child(div().flex().flex_1().min_h_0().child(rail).child(page_body))
            .child(self.dock(window, cx));
        if self.ui.picker {
            let mut choices = div()
                .absolute()
                .bottom(px(68.))
                .left(px(12.))
                .rounded(px(tokens::R_CARD))
                .p(px(8.))
                .flex()
                .flex_col()
                .gap(px(4.))
                .bg(rgb(CARD))
                .border_1()
                .border_color(rgb(LINE));
            for (n, item) in self
                .instances
                .list
                .iter()
                .filter(|i| i.java.is_none())
                .cloned()
                .enumerate()
            {
                choices = choices.child(
                    button(ElementId::named_usize("alt-pick", n), item.name.clone()).on_click(
                        cx.listener(move |this, _, _, cx| {
                            if !this.launch_busy() && this.client.is_none() {
                                this.select_instance(&item);
                                this.ui.picker = false;
                                this.client_error = None;
                            }
                            cx.notify();
                        }),
                    ),
                );
            }
            frame = frame.child(choices);
        }
        // What lies over the page, bottom to top. The top one has the keyboard (`modal`).
        self.versions_focus(window);
        let create = (page == Page::Hosting)
            .then(|| self.hosting_create(cx))
            .flatten();
        let sheet = self.live_sheet(cx);
        let sign_in = self.owner_screen_open.then(|| self.owner_screen(cx));
        let mut layers: Vec<_> = [create, sheet, sign_in].into_iter().flatten().collect();
        let top = self.modal(layers.pop(), window, cx);
        frame = frame.children(layers).children(top);
        self.pictures.free_retired(window);
        div()
            .relative()
            .size_full()
            .child(frame)
            .children(titlebar::resize_handles(window))
    }
}
