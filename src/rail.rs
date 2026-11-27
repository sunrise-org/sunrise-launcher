//! The sidebar: the navigation (Discover, Servers, Skins), the profiles, and Help & community
//! and Settings at the bottom. Its profiles scroll independently of its controls. The parts are
//! in `rail/nav.rs` and `rail/profiles.rs`.
use super::*;
use crate::{frame::Page, paint::palette};
use gpui::rgb;
use instances::Instance;
use std::{cell::RefCell, rc::Rc};

mod nav;
mod profiles;

/// Each drawn profile row that opens a context menu, with its bounds in the window.
pub(crate) type ProfileRows = Rc<RefCell<Vec<(gpui::Bounds<gpui::Pixels>, Instance)>>>;

/// How wide the sidebar is closed, and open.
const CLOSED: f32 = 54.;
const OPEN: f32 = 232.;
/// The sidebar's padding on every side.
const RAIL_PAD: f32 = 12.;
/// The width a closed sidebar's rows and profile columns have inside its padding.
const CLOSED_ROW: f32 = CLOSED - 2. * RAIL_PAD;
/// A nav row's icon, and the padding either side of it that centres it in the closed column.
const ICON: f32 = 20.;
const ICON_PAD: f32 = (CLOSED_ROW - ICON) / 2.;
/// A label's width per character: a 13 px Geist average, for whether the sun fits.
const CHAR_WIDTH: f32 = 7.2;

/// How wide the sidebar is, `open` of the way from closed (0) to open (1).
pub(crate) fn rail_width(open: f32) -> f32 {
    CLOSED + (OPEN - CLOSED) * open
}

/// A label's opacity as the sidebar opens: it waits for room before showing.
fn label_opacity(open: f32) -> f32 {
    ((open - 0.3) / 0.7).clamp(0., 1.)
}

/// The column a sidebar icon is centred in, open or closed: the closed sidebar's inside. Labels
/// follow it, so no icon moves when the sidebar opens or closes.
fn icon_slot(icon: impl IntoElement) -> Div {
    div()
        .flex_none()
        .w(px(CLOSED_ROW))
        .flex()
        .items_center()
        .justify_center()
        .child(icon)
}

/// The hairline between the sidebar's sections: 1 px, with 8 px of space above and below.
fn divider(p: &crate::paint::Palette) -> Div {
    div().flex_none().h(px(1.)).my(px(8.)).bg(rgb(p.divider))
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

    /// The sidebar: the navigation, a divider, the profiles (which take the room that is left),
    /// then Help & community and Settings at the foot.
    pub(super) fn sidebar(&mut self, cx: &mut Context<Self>) -> Div {
        let open = self.rail_open();
        let p = palette();
        let look = profiles::Look {
            open,
            lock: self.profile_lock(),
            on_home: self.ui.page == Page::Home,
        };
        div()
            .flex()
            .flex_col()
            .w(px(rail_width(open)))
            .min_h_0()
            .flex_none()
            .p(px(RAIL_PAD))
            .bg(rgb(p.chrome))
            .text_color(rgb(p.chrome_text))
            .child(self.nav_section(open, cx))
            .child(divider(&p))
            .child(self.profiles_section(look, cx))
            .child(self.help_section(open, cx))
            .child(self.settings_row(open, cx))
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
            // Inside the rail's padding: the button sits 5 px from the row's right edge, which
            // is the rail's inner edge, and is 28 wide. The mark is centred in its column, which
            // doesn't move as the sidebar opens.
            let button_left = rail_width(open) - RAIL_PAD - 5. - 28.;
            let mark_right = RAIL_PAD + (CLOSED_ROW - 28.) / 2. + 28.;
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
        let (shell, cx) = app.add_window_view(crate::window_root::Shell::new);
        let launcher = shell.read_with(cx, |shell, _| shell.launcher());
        // A frame with the sidebar `open` of the way, and no menu up.
        let draw = |cx: &mut gpui::VisualTestContext, open: f32| {
            launcher.update(cx, |l, cx| {
                l.ui.rail_pin = Some(open);
                l.ui.sheets.profile_action = None;
                l.instances.dialog = None;
                cx.notify();
            });
            cx.run_until_parked();
        };
        // The button opens the profile's menu.
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
            assert!(
                menu(cx),
                "at {open} a press on the button opens its menu"
            );
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
        // The icon column is a closed row's inside: the closed rail less its padding on each side.
        assert_eq!(CLOSED_ROW, rail_width(0.) - 2. * RAIL_PAD);
        // A nav row's icon sits centred in that column, and its label starts where a profile's does.
        assert_eq!(ICON_PAD * 2. + ICON, CLOSED_ROW);
        assert_eq!(rail_width(0.), CLOSED);
        assert_eq!(rail_width(1.), 232.);
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
            installing: Some(0.4),
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
