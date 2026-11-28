//! The full-height profiles sidebar. Its middle scrolls independently of its controls.
use super::*;
use crate::{
    frame::{Page, Reach},
    paint::palette,
};
use gpui::{rgb, MouseButton, MouseDownEvent};
use instances::{Icon, Instance, Kind};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

/// Each drawn profile row that opens a context menu, with its bounds in the window.
pub(crate) type ProfileRows = Rc<RefCell<Vec<(gpui::Bounds<gpui::Pixels>, Instance)>>>;

/// How wide the sidebar is closed, and open.
const CLOSED: f32 = 54.;
const OPEN: f32 = 204.;
/// The room a row has inside the closed sidebar: its padding and both borders taken off.
const CLOSED_ROW: f32 = CLOSED - 2. * 5. - 2. - 2.;

/// How wide the sidebar is, `open` of the way from closed (0) to open (1).
pub(crate) fn rail_width(open: f32) -> f32 {
    CLOSED + (OPEN - CLOSED) * open
}

/// A label's opacity as the sidebar opens: it waits for room before showing.
fn label_opacity(open: f32) -> f32 {
    ((open - 0.3) / 0.7).clamp(0., 1.)
}

/// The column a sidebar icon is centred in, open or closed: a closed row's inside (the closed
/// rail less its 1 px edges, its 5 px padding and the row's own 1 px edges). Labels follow it,
/// so no icon moves when the sidebar opens or closes.
fn icon_slot(icon: impl IntoElement) -> Div {
    div()
        .flex_none()
        .w(px(CLOSED_ROW))
        .flex()
        .items_center()
        .justify_center()
        .child(icon)
}

/// Where the selected profile's actions button is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ActionsAt {
    /// In the row under the mark, the sidebar closed.
    Under,
    /// Beside the name, fading in with it.
    Beside,
}

/// Where the actions button is built `open` of the way, if at all. Closed, it is under the mark.
/// Opening, it leaves the row for its place beside the name, which it takes once it fades in,
/// when it is already clear of the mark: until then it would sit, unseen, over the mark, where a
/// press on the mark could open the profile's menu, so it isn't built.
fn actions_at(open: f32) -> Option<ActionsAt> {
    if open <= 0. {
        Some(ActionsAt::Under)
    } else if label_opacity(open) > 0. {
        Some(ActionsAt::Beside)
    } else {
        None
    }
}

/// The room under the mark the closed actions button took (28 px), `open` of the way.
fn actions_room(open: f32) -> f32 {
    28. * (1. - open).clamp(0., 1.)
}

fn profile_lock_reason(
    work: &crate::launch_state::Facts,
    located: bool,
    pending: bool,
) -> Option<&'static str> {
    if !located {
        Some(tr(
            "Fix the data folder in Settings before changing profiles.",
        ))
    } else if work.holds_profile() {
        Some(tr(
            "Wait for the current operation to finish before changing profiles.",
        ))
    } else if pending {
        Some(tr(
            "Finish or cancel the pending launch before changing profiles.",
        ))
    } else {
        None
    }
}

/// The heading above each profile in the sidebar: a group's name over its first profile.
/// `instances::list` keeps the default profile first and sorts the rest by group, so only the
/// rest are headed: a group the default profile has would otherwise seem to hold the ungrouped
/// profiles under it. A blank group, as an earlier launcher saved an empty field, is no group.
fn group_headings<'a>(groups: impl Iterator<Item = Option<&'a str>>) -> Vec<Option<String>> {
    let mut before: Option<&str> = None;
    groups
        .enumerate()
        .map(|(n, group)| {
            let group = group.filter(|g| !g.trim().is_empty());
            let heads = n > 0 && group.is_some() && group != before;
            if n > 0 {
                before = group;
            }
            heads.then(|| group.unwrap_or_default().to_owned())
        })
        .collect()
}

impl Launcher {
    pub(super) fn toggle_sidebar(&mut self) {
        let open = !self.settings.sidebar_expanded;
        self.settings.sidebar_expanded = open;
        // From wherever the last toggle had got to: a second press turns it around there.
        let mut fade = crate::motion::Fade::toward(self.ui.rail, open, std::time::Instant::now());
        fade.motion = crate::motion::RAIL;
        self.ui.rail = Some(fade);
        self.save_settings();
    }

    /// The window's width less what the sidebar takes beyond its closed width, as it is drawn
    /// now: what the top bar's narrow layouts go by, so they change when the room does, part
    /// way through an ease, not on the press that starts it.
    pub(crate) fn bar_width(&self) -> f32 {
        self.window_width - (rail_width(self.rail_open()) - CLOSED)
    }

    /// How open the sidebar is drawn: 0 closed, 1 open, between while it eases. The frame's
    /// one sample (`frame::State::rail_drawn`): a part built later in the same frame never sees
    /// a width the rail wasn't drawn at.
    pub(crate) fn rail_open(&self) -> f32 {
        self.ui
            .rail_drawn
            .unwrap_or_else(|| self.rail_open_at(std::time::Instant::now()))
    }

    /// How open the sidebar is at `now`.
    pub(crate) fn rail_open_at(&self, now: std::time::Instant) -> f32 {
        let at_rest = if self.settings.sidebar_expanded {
            1.
        } else {
            0.
        };
        self.ui
            .rail
            .map_or(at_rest, |fade| fade.value(now).clamp(0., 1.))
    }

    pub(super) fn profile_lock(&self) -> Option<&'static str> {
        profile_lock_reason(
            &self.launch_activity(),
            !self.location_unavailable,
            self.ui.after_sign_in.is_some() || self.ui.after_install.is_some(),
        )
    }

    pub(super) fn switch_profile(&mut self, instance: &Instance, cx: &mut Context<Self>) {
        if self.profile_lock().is_some() {
            return;
        }
        if !self
            .selected_instance()
            .is_some_and(|i| i.id == instance.id)
        {
            self.select_instance(instance);
            self.clear_profile_notices();
        }
        self.navigate(Page::Home);
        cx.notify();
    }

    pub(super) fn clear_profile_notices(&mut self) {
        self.client_error = None;
        self.prepare_error = None;
        self.install_error = None;
        self.crash = None;
    }

    pub(super) fn profile_mark(&mut self, i: &Instance, size: f32) -> Div {
        let p = palette();
        let picture = self.icon_image(format!("picture:{}", i.id), size, || {
            Some(i.home.join(instances::PICTURE))
        });
        let texture = match &i.icon {
            Some(icon @ Icon::Texture(name)) if picture.is_none() => {
                self.version_of(i).cloned().and_then(|tag| {
                    self.icon_image(format!("texture:{tag}:{name}"), size, || {
                        instances::icon_texture(icon, &backend::resources(&tag))
                    })
                })
            }
            _ => None,
        };
        let tile = div()
            .size(px(size))
            .flex_none()
            .rounded(px(size * 0.22))
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center();
        if let Some(image) = picture.or(texture) {
            return tile.child(
                gpui::img(image)
                    .size_full()
                    .object_fit(gpui::ObjectFit::Contain),
            );
        }
        match &i.icon {
            Some(Icon::Colour(hex)) => {
                let colour =
                    u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(p.selected);
                let ink = if sunrise_launcher_theme::contrast(colour, p.text)
                    > sunrise_launcher_theme::contrast(colour, p.background)
                {
                    p.text
                } else {
                    p.background
                };
                tile.bg(rgb(colour))
                    .text_color(rgb(ink))
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(size * 0.54))
                    .child(
                        i.name
                            .chars()
                            .next()
                            .unwrap_or('S')
                            .to_uppercase()
                            .to_string(),
                    )
            }
            _ => crate::mark(size),
        }
    }

    pub(super) fn sidebar(&mut self, cx: &mut Context<Self>) -> Div {
        let wide = self.settings.sidebar_expanded;
        let open = self.rail_open();
        // Labels are drawn once there is any room for them; they fade in as it opens.
        let labelled = open > 0.;
        let p = palette();
        let lock = self.profile_lock();
        let row = |id: ElementId, label: String, on: bool| {
            div()
                .id(id)
                .reach_edge()
                .h(px(40.))
                .flex_none()
                .w_full()
                .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                .border_1()
                .border_color(if on {
                    rgb(p.accent_ink).into()
                } else {
                    gpui::transparent_black()
                })
                // The chrome's own lift, in the theme's hue, under the accent's edge: the
                // accent washed into the fill stood out in themes whose hue differs from it.
                .when(on, |d| d.bg(rgb(p.chrome_raised)))
                .flex()
                .items_center()
                // Icons stay in their column (`icon_slot`); the room after the label eases in.
                .gap(px(4.))
                .pr(px(8. * open))
                .when(lock.is_none(), |d| {
                    d.cursor_pointer().hover(|d| d.bg(rgb(p.chrome_raised)))
                })
                .when(lock.is_some(), |d| d.opacity(0.45).tab_stop(false))
                .when(!labelled || lock.is_some(), |d| {
                    d.tooltip(crate::glass_ui::tip(lock.map_or(label, str::to_owned)))
                })
        };
        let txt = |s: String| {
            div()
                .min_w_0()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                // An ellipsis would flicker along a label still being uncovered.
                .when(open >= 1., |d| d.text_ellipsis())
                .opacity(label_opacity(open))
                .text_size(px(tokens::LABEL))
                .child(s)
        };
        // A small toolbar button, not a row: it names itself in its tooltip only. Ctrl+B does
        // the same, and the search lists that key.
        let me = cx.weak_entity();
        let toggle = div()
            .flex()
            .flex_none()
            .h(px(32.))
            .items_center()
            // In line with the rows' icons, whose rows have a 1 px edge.
            .px(px(1.))
            .child(icon_slot(
                div()
                    .id("sidebar-toggle")
                    .reach()
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                    .cursor_pointer()
                    .hover(|d| d.bg(rgb(p.chrome_raised)))
                    // Read when the tooltip shows: one that was on its way when the toggle was
                    // pressed would otherwise offer what it just did.
                    .tooltip(crate::glass_ui::tip_with(move |cx| {
                        let wide = me
                            .upgrade()
                            .map_or(wide, |me| me.read(cx).settings.sidebar_expanded);
                        if wide {
                            tr("Hide sidebar")
                        } else {
                            tr("Show sidebar")
                        }
                        .into()
                    }))
                    .child(
                        div()
                            .w(px(16.))
                            .h(px(13.))
                            .flex()
                            .rounded(px(3.))
                            .border_1()
                            .border_color(rgb(p.chrome_muted))
                            .child(div().ml(px(4.)).w(px(1.)).h_full().bg(rgb(p.chrome_muted))),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_sidebar();
                        cx.notify();
                    })),
            ));
        let add = row("sidebar-create".into(), tr("New profile").into(), false)
            .child(icon_slot(blocks::icon(
                "plus",
                20.,
                rgb(p.chrome_text).into(),
            )))
            .when(labelled, |d| d.child(txt(tr("New profile").into())))
            .when(lock.is_none(), |d| {
                d.on_click(cx.listener(|this, _, _, cx| {
                    if this.profile_lock().is_none() {
                        this.new_profile(cx);
                    }
                }))
            });
        let selected = self.selected_instance().map(|i| i.id.clone());
        let rows = self.ui.profile_rows.clone();
        rows.borrow_mut().clear();
        let mut list = div()
            .id("sidebar-profiles")
            .flex()
            .flex_col()
            .min_h_0()
            .flex_1()
            .gap(px(4.))
            .overflow_y_scroll()
            .track_scroll(&crate::frame::scrolls("sidebar"));
        let headings = group_headings(self.instances.list.iter().map(|i| i.group.as_deref()));
        for (n, i) in self.instances.list.clone().into_iter().enumerate() {
            if let Some(group) = headings[n].clone() {
                list = list.child(
                    div()
                        .flex_none()
                        .h(px(22.))
                        .mt(px(6.))
                        .px(px(9.))
                        .flex()
                        .items_end()
                        .truncate()
                        .text_size(px(crate::tokens::SMALL))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(p.chrome_muted))
                        .opacity(label_opacity(open))
                        // With the names hidden a short rule still marks where a group begins.
                        .when(!labelled, |d| {
                            d.h(px(1.))
                                .mt(px(4.))
                                .mx(px(8.))
                                .bg(rgb(p.chrome_muted))
                                .opacity(0.5)
                        })
                        .when(labelled, |d| d.child(group)),
                );
            }
            let on = selected.as_ref() == Some(&i.id);
            let deletable = i.kind == Kind::Own && !self.location_unavailable;
            let more = on && deletable;
            let profile = row(
                ElementId::named_usize("sidebar-profile", n),
                i.name.clone(),
                on,
            )
            .child(
                icon_slot(self.profile_mark(&i, 28.))
                    .when(on, |d| d.debug_selector(|| "selected-profile-mark".into())),
            )
            .when(labelled, |d| d.child(txt(i.name.clone())))
            .when(labelled && more, |d| d.pr(px(36.)))
            .when(deletable, |d| {
                let (drawn, at) = (i.clone(), rows.clone());
                let i = i.clone();
                d.child(
                    gpui::canvas(
                        // Only the part of the row the list shows counts: a row scrolled past
                        // either end lies under the rail's controls.
                        move |bounds, window, _| {
                            let visible = bounds.intersect(&window.content_mask().bounds);
                            if !visible.is_empty() {
                                at.borrow_mut().push((visible, drawn.clone()));
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .inset_0(),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        this.open_profile_menu(&i, f32::from(ev.position.y));
                        cx.notify();
                    }),
                )
            });
            let actions = more.then(|| actions_at(open)).flatten().map(|at| {
                let top = Rc::new(Cell::new(0.));
                let read_top = top.clone();
                let i = i.clone();
                div()
                    .id(SharedString::from(format!("profile-actions-{}", i.id)))
                    .debug_selector(|| "selected-profile-actions".into())
                    .reach()
                    .when(at == ActionsAt::Beside, |d| {
                        d.absolute()
                            .top(px(6.))
                            .right(px(5.))
                            .w(px(28.))
                            .opacity(label_opacity(open))
                    })
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|d| d.bg(rgb(p.chrome_raised)))
                    .tooltip(crate::glass_ui::tip(tr("Profile actions")))
                    .child(div().flex().gap(px(2.)).children(
                        (0..3).map(|_| div().size(px(3.)).rounded_full().bg(rgb(p.chrome_text))),
                    ))
                    .child(
                        gpui::canvas(
                            move |bounds, _, _| top.set(f32::from(bounds.top())),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_profile_menu(&i, read_top.get());
                        cx.notify();
                    }))
            });
            list = list.child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .child(profile.when(lock.is_none(), |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| this.switch_profile(&i, cx)))
                    }))
                    .children(actions)
                    // The room the closed button took under the mark eases away with the
                    // width, so the profiles below don't jump as it leaves the row.
                    .when(more && labelled, |d| {
                        d.child(div().flex_none().h(px(actions_room(open))))
                    }),
            );
        }
        let mut rail = div()
            .flex()
            .flex_col()
            .w(px(rail_width(open)))
            .min_h_0()
            .flex_none()
            .gap(px(4.))
            .p(px(5.))
            .rounded(crate::tokens::r(crate::tokens::R_PANEL))
            .bg(p.chrome_plate())
            .border_1()
            .border_color(p.rule())
            .text_color(rgb(p.chrome_text))
            .child(toggle)
            .child(add)
            .child(list);
        for (id, label, icon, url) in [
            (
                "sidebar-discord",
                tr("Discord"),
                "discord",
                self.links.discord.clone(),
            ),
            (
                "sidebar-website",
                tr("Website"),
                "earth",
                self.links.website.clone(),
            ),
            (
                "sidebar-logs",
                tr("Logs"),
                "computer-terminal-01",
                String::new(),
            ),
            (
                "sidebar-settings",
                tr("Settings"),
                "sliders-horizontal",
                String::new(),
            ),
        ] {
            let settings = id == "sidebar-settings";
            let logs = id == "sidebar-logs";
            rail = rail.child(
                row(
                    id.into(),
                    label.into(),
                    (settings && self.ui.page == Page::Settings) || (logs && self.logs_open()),
                )
                .child(icon_slot(crate::glass_ui::icon(
                    icon,
                    18.,
                    rgb(p.chrome_muted).into(),
                )))
                .when(labelled, |d| d.child(txt(label.into())))
                .when(settings || logs || !url.is_empty(), |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| {
                        if logs {
                            this.toggle_logs();
                        } else if settings {
                            this.navigate(Page::Settings);
                        } else {
                            Self::open_url(&url, cx);
                        }
                        cx.notify();
                    }))
                }),
            );
        }
        rail
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launch_state::Facts;

    #[test]
    fn the_actions_button_is_under_the_mark_closed_beside_the_name_open_and_absent_between() {
        // Both settled layouts have it.
        assert_eq!(actions_at(0.), Some(ActionsAt::Under));
        assert_eq!(actions_at(1.), Some(ActionsAt::Beside));
        let (mut hidden, mut beside) = (0, false);
        for step in 1..=1000 {
            let open = step as f32 / 1000.;
            // Inside the rail's padding (5) and border (1): the button sits 5 px from the
            // right and is 28 wide. The mark is centred in its column after the row's border,
            // and the column doesn't move as the sidebar opens.
            let button_left = rail_width(open) - 2. * 5. - 2. - 5. - 28.;
            let mark_right = 1. + (CLOSED_ROW - 28.) / 2. + 28.;
            match actions_at(open) {
                Some(ActionsAt::Under) => panic!("at {open} the button is back under the mark"),
                Some(ActionsAt::Beside) => {
                    beside = true;
                    assert!(
                        button_left >= mark_right,
                        "at {open} the button ({button_left}) is over the mark ({mark_right})"
                    );
                    assert!(label_opacity(open) > 0., "at {open} it is built unseen");
                }
                None => {
                    assert!(!beside, "at {open} it went away again after showing");
                    hidden += 1;
                }
            }
            if button_left < mark_right {
                assert_eq!(
                    actions_at(open),
                    None,
                    "at {open} it would be over the mark"
                );
            }
        }
        assert!(hidden > 0, "the start of opening leaves it out");
        // Closed, the button is in the row under the mark; the room it took eases away.
        assert_eq!(actions_room(0.), 28.);
        assert_eq!(actions_room(1.), 0.);
        assert!(
            (actions_room(0.001) - 28.).abs() < 0.1,
            "no jump as opening begins"
        );
    }

    #[cfg(debug_assertions)]
    /// Runs `test` again in a child process with scratch homes, as the other window tests do:
    /// `None` here, the scratch root in the child.
    fn scratch_child(test: &str) -> Option<std::path::PathBuf> {
        const CHILD: &str = "SUNRISE_RAIL_TEST";
        if let Some(root) = std::env::var_os(CHILD) {
            return Some(root.into());
        }
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        let nowhere = "http://127.0.0.1:9";
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &format!("rail::tests::{test}")])
            .env(CHILD, root)
            .env("SUNRISE_HOME", root.join("game"))
            .env("SUNRISE_LAUNCHER_HOME", root.join("launcher"))
            .env("SUNRISE_LAUNCHER_LOCATION_FILE", root.join("location.json"))
            .env("SUNRISE_NO_KEYCHAIN", "1")
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("SUNRISE_TELEMETRY_URL", "off")
            .env("XDG_DATA_HOME", root.join("data"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("HTTP_PROXY", nowhere)
            .env("HTTPS_PROXY", nowhere)
            .env("ALL_PROXY", nowhere)
            .env_remove("NO_PROXY")
            .env_remove("SUNRISE_GITHUB_MIRROR")
            .env_remove("SUNRISE_LAUNCHER_BENCH")
            .status()
            .unwrap();
        assert!(status.success(), "{test} failed in its child process");
        None
    }

    // GPUI Fast records `debug_selector` bounds only with debug assertions: the release
    // workflow's `cargo test --release` has none to find, so it leaves this test out. Every
    // debug test run (ci.yml, on each platform) runs it.
    #[cfg(debug_assertions)]
    #[test]
    fn the_actions_button_takes_presses_settled_and_leaves_the_mark_alone_while_hidden() {
        if scratch_child(
            "the_actions_button_takes_presses_settled_and_leaves_the_mark_alone_while_hidden",
        )
        .is_none()
        {
            return;
        }
        let profile = instances::create("Cabin", None).unwrap();
        let mut settings = Settings::load();
        settings.instance = Some(profile.id.clone());
        settings.sidebar_expanded = false;
        settings.save().unwrap();
        let mut app = gpui::TestAppContext::single();
        let (shell, cx) = app.add_window_view(crate::window_root::Shell::new_for_test);
        let launcher = shell.read_with(cx, |shell, _| shell.launcher());
        // A frame with the sidebar `open` of the way, and no menu up.
        let draw = |cx: &mut gpui::VisualTestContext, open: f32| {
            launcher.update(cx, |l, cx| {
                l.ui.rail_pin = Some(open);
                l.ui.sheets.profile_action = None;
                cx.notify();
            });
            cx.run_until_parked();
        };
        let menu = |cx: &mut gpui::VisualTestContext| {
            launcher.read_with(cx, |l, _| {
                matches!(
                    l.ui.sheets.profile_action,
                    Some(crate::profile_delete::Action::Menu { .. })
                )
            })
        };
        for open in [0., 1.] {
            draw(cx, open);
            let button = cx
                .debug_bounds("selected-profile-actions")
                .unwrap_or_else(|| panic!("at {open} the button is drawn"));
            cx.simulate_click(button.center(), gpui::Modifiers::none());
            assert!(menu(cx), "at {open} a press on the button opens the menu");
        }
        for open in [0.05, 0.15, 0.25] {
            draw(cx, open);
            // Not drawn, so nothing to hit and no Tab stop.
            assert_eq!(
                cx.debug_bounds("selected-profile-actions"),
                None,
                "at {open}"
            );
            let mark = cx
                .debug_bounds("selected-profile-mark")
                .unwrap_or_else(|| panic!("at {open} the mark is drawn"));
            cx.simulate_click(mark.center(), gpui::Modifiers::none());
            assert!(!menu(cx), "at {open} a press on the mark opened the menu");
        }
    }

    #[test]
    fn icons_keep_the_closed_rows_column_and_labels_wait_for_room() {
        // The icon column is a closed row's inside: the rail less its edges, padding and the
        // row's own edges, as `icon_slot` was sized from before the sidebar eased.
        assert_eq!(CLOSED_ROW, rail_width(0.) - 2. * (1. + 5. + 1.));
        assert_eq!(rail_width(0.), 54.);
        assert_eq!(rail_width(1.), 204.);
        assert_eq!(label_opacity(0.2), 0.);
        assert_eq!(label_opacity(1.), 1.);
    }

    #[test]
    fn groups_head_their_first_profile_and_the_pinned_default_heads_none() {
        let heads = |groups: &[Option<&str>]| group_headings(groups.iter().copied());
        let a = || Some("A".to_owned());
        assert_eq!(
            heads(&[None, None, Some("A"), Some("A"), Some("B")]),
            [None, None, a(), None, Some("B".to_owned())]
        );
        // The default profile in a group, then ungrouped ones: no heading would cover them.
        assert_eq!(heads(&[Some("A"), None, Some("A")]), [None, None, a()]);
        // A blank group is no group.
        assert_eq!(heads(&[None, Some(" "), Some("A")]), [None, None, a()]);
    }

    #[test]
    fn profiles_unlock_when_starting_finishes_even_while_the_game_runs() {
        let mut work = Facts {
            starting: true,
            ..Facts::default()
        };
        assert!(profile_lock_reason(&work, true, false).is_some());
        work.running = true;
        work.starting = false;
        assert_eq!(profile_lock_reason(&work, true, false), None);
        assert!(profile_lock_reason(&work, false, false).is_some());
        assert!(profile_lock_reason(&work, true, true).is_some());
    }

    #[test]
    fn a_build_downloading_leaves_profiles_free_but_mod_changes_do_not() {
        let mut work = Facts {
            installing: Some(sunrise_launcher::backend::InstallPhase::Downloading(0.4)),
            ..Facts::default()
        };
        assert_eq!(profile_lock_reason(&work, true, false), None);
        work.mutating = true;
        assert!(profile_lock_reason(&work, true, false).is_some());
        // Play waiting on the download still holds them.
        work.mutating = false;
        assert!(profile_lock_reason(&work, true, true).is_some());
    }
}
