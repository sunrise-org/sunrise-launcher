use super::*;

#[test]
fn a_fact_starts_with_a_capital() {
    assert_eq!(sentence("yesterday"), "Yesterday");
    assert_eq!(sentence(""), "");
}

#[test]
fn a_profile_says_its_play_time_only_once_it_was_played() {
    assert_eq!(played_line(Duration::from_secs(600), None), None);
    let now = std::time::SystemTime::now();
    assert_eq!(
        played_line(Duration::from_secs(3 * 3600 + 300), Some(now)),
        Some(("3 h 05 min played".into(), "Last played today".into()))
    );
}

#[test]
fn a_long_list_builds_the_rows_near_the_view() {
    // 100 rows 56 apart, the page showing the list's first 400 points.
    assert_eq!(near(100, 1, 56., (0., 400.)), 0..11);
    // Scrolled 2000 points down: those in view, three before and a few after.
    assert_eq!(near(100, 1, 56., (2000., 2400.)), 32..46);
    // The list is far under the view, or far over it: nothing of it is built.
    assert_eq!(near(100, 1, 56., (-900., -500.)), 0..0);
    assert_eq!(near(100, 1, 56., (9000., 9400.)), 100..100);
    // Just under the view: its first rows are there for Tab to land on.
    assert_eq!(near(100, 1, 56., (-500., -100.)), 0..2);
    // A short list is whole.
    assert_eq!(near(6, 1, 56., (0., 400.)), 0..6);
    // The last row of a thousand still belongs to the active virtual range.
    assert_eq!(near(1_000, 1, ROW, (39_800., 40_000.)), 992..1_000);
    // Tiles go by whole bands of four.
    assert_eq!(near(30, 4, 196., (400., 800.)), 0..30);
    assert_eq!(near(300, 4, 196., (1960., 2360.)), 28..64);
}

/// Runs `test` again in a child process with scratch homes, as the other window tests do:
/// `None` here, the scratch root in the child.
#[cfg(debug_assertions)]
fn scratch_child(test: &str) -> Option<std::path::PathBuf> {
    const CHILD: &str = "SUNRISE_HOME_WINDOW_TEST";
    if let Some(root) = std::env::var_os(CHILD) {
        return Some(root.into());
    }
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let nowhere = "http://127.0.0.1:9";
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &format!("home::tests::{test}")])
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

/// A Home control's tooltip shows its exact text while the pointer rests on it, and goes away
/// when the control's own menu opens under that same pointer, which doesn't move. GPUI keeps a
/// visible tooltip until the pointer leaves, so only the control dropping its tooltip while the
/// menu is open takes it away. Then, with the menu closed, the tooltip comes back on hover, and a
/// press opens the menu with no tooltip over it. Debug builds only: `debug_selector` bounds are
/// recorded only with debug assertions.
#[cfg(debug_assertions)]
fn a_controls_tooltip_leaves_when_its_menu_opens(
    test: &str,
    control: &'static str,
    tip: &str,
    menu: Sheet,
) {
    if scratch_child(test).is_none() {
        return;
    }
    const TAG: &str = "26.3-0.1.0";
    let build = backend::version_dir(TAG);
    std::fs::create_dir_all(&build).unwrap();
    std::fs::write(build.join(".installed"), []).unwrap();
    let profile = instances::create("Cabin", None).unwrap();
    sunrise_launcher::browser::sunrise::save_build_choice(
        &profile.id,
        TAG,
        sunrise_launcher::browser::sunrise::BuildChoice::Keep,
    )
    .unwrap();
    let mut settings = backend::Settings::load();
    settings.instance = Some(profile.id.clone());
    settings.save().unwrap();
    let mut app = gpui::TestAppContext::single();
    let (shell, cx) = app.add_window_view(crate::window_root::Shell::new_for_test);
    let launcher = shell.read_with(cx, |shell, _| shell.launcher());
    cx.run_until_parked();
    // `debug_bounds` takes a static selector; the test process ends soon after.
    let tip: &'static str = format!("tooltip: {tip}").leak();
    let settle = |cx: &mut gpui::VisualTestContext| {
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(2));
        cx.run_until_parked();
    };
    let hover = |cx: &mut gpui::VisualTestContext| {
        let centre = cx
            .debug_bounds(control)
            .unwrap_or_else(|| panic!("Home draws {control}"))
            .center();
        cx.simulate_mouse_move(
            gpui::point(gpui::px(2.), gpui::px(2.)),
            None,
            gpui::Modifiers::none(),
        );
        cx.simulate_mouse_move(centre, None, gpui::Modifiers::none());
        settle(cx);
        centre
    };
    hover(cx);
    assert!(cx.debug_bounds(tip).is_some(), "{tip:?} shows on hover");
    // The menu opens while the pointer stays on the control and the tooltip is up.
    launcher.update(cx, |l, cx| {
        l.open_sheet(menu);
        cx.notify();
    });
    settle(cx);
    assert!(
        cx.debug_bounds(tip).is_none(),
        "{tip:?} goes away when the menu opens under the pointer"
    );
    launcher.update(cx, |l, cx| {
        l.close_sheet();
        cx.notify();
    });
    settle(cx);
    let centre = hover(cx);
    assert!(
        cx.debug_bounds(tip).is_some(),
        "{tip:?} shows again once the menu is closed"
    );
    cx.simulate_click(centre, gpui::Modifiers::none());
    settle(cx);
    assert_eq!(
        launcher.read_with(cx, |l, _| l.ui.sheet),
        Some(menu),
        "a press opens the menu"
    );
    assert!(
        cx.debug_bounds(tip).is_none(),
        "no {tip:?} over the open menu"
    );
}

/// The build button's exact tag, and its menu (review of #639).
#[cfg(debug_assertions)]
#[test]
fn the_build_tag_leaves_when_the_build_menu_opens() {
    a_controls_tooltip_leaves_when_its_menu_opens(
        "the_build_tag_leaves_when_the_build_menu_opens",
        "home-build",
        "26.3-0.1.0",
        Sheet::Build,
    );
}

/// The gear's "Profile options", and the profile menu (review of #639).
#[cfg(debug_assertions)]
#[test]
fn profile_options_leaves_when_the_profile_menu_opens() {
    a_controls_tooltip_leaves_when_its_menu_opens(
        "profile_options_leaves_when_the_profile_menu_opens",
        "home-profile-menu",
        "Profile options",
        Sheet::ProfileMenu,
    );
}
