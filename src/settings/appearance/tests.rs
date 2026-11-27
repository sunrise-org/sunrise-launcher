use super::photo::photo_tone;
use image::{Rgba, RgbaImage};

/// Runs `test` again in a child process with its own scratch homes; Some(root) inside it.
fn scratch_child(test: &str) -> Option<std::path::PathBuf> {
    const CHILD: &str = "SUNRISE_APPEARANCE_TEST";
    if let Some(root) = std::env::var_os(CHILD) {
        return Some(root.into());
    }
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let nowhere = "http://127.0.0.1:9";
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &format!("settings::appearance::tests::{test}")])
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

#[test]
fn a_typed_accent_is_taken_when_the_field_is_left_without_enter() {
    use sunrise_launcher::appearance;
    use sunrise_launcher::backend::Settings;
    use sunrise_launcher::themes::Accent;
    if scratch_child("a_typed_accent_is_taken_when_the_field_is_left_without_enter").is_none() {
        return;
    }
    let mut app = gpui::TestAppContext::single();
    let (shell, cx) = app.add_window_view(crate::window_root::Shell::new);
    let launcher = shell.read_with(cx, |shell, _| shell.launcher());
    launcher.update(cx, |l, cx| {
        l.navigate(crate::frame::Page::Settings);
        l.settings_open(crate::settings::Section::Appearance);
        cx.notify();
    });
    cx.run_until_parked();
    // The field is below the fold of the test window: give it the keyboard directly.
    let focus = launcher.read_with(cx, |l, _| {
        let focus = l.ui.settings.appearance.accent_focus.clone();
        focus.expect("the accent field is drawn")
    });
    cx.update(|window, cx| window.focus(&focus, cx));
    cx.simulate_keystrokes("ctrl-a t e a l");
    cx.run_until_parked();
    let accent = || appearance::accent(&Settings::load());
    assert_ne!(accent(), Accent::TEAL, "not taken before the field is left");
    cx.update(|window, cx| window.blur(cx));
    cx.run_until_parked();
    assert_eq!(accent(), Accent::TEAL);
}

/// The player's own picture is behind every page, not Home's alone: on Settings it is read
/// and decoded for the backdrop just as it is on Home.
#[test]
fn the_players_picture_is_behind_pages_other_than_home() {
    use sunrise_launcher::appearance;
    use sunrise_launcher::backend::{self, Settings};
    let Some(root) = scratch_child("the_players_picture_is_behind_pages_other_than_home") else {
        return;
    };
    let mine = root.join("mine.png");
    RgbaImage::from_pixel(80, 80, Rgba([40, 120, 200, 255]))
        .save(&mine)
        .unwrap();
    let mut settings = Settings::load();
    appearance::set_background(&mut settings, &backend::data_dir(), &mine).unwrap();
    let kept = backend::data_dir().join(settings.background.as_deref().unwrap());
    let mut app = gpui::TestAppContext::single();
    let (shell, cx) = app.add_window_view(crate::window_root::Shell::new);
    let launcher = shell.read_with(cx, |shell, _| shell.launcher());
    launcher.update(cx, |l, cx| {
        l.navigate(crate::frame::Page::Settings);
        cx.notify();
    });
    cx.run_until_parked();
    // Chosen while Settings is open, so Home has never drawn it.
    launcher.update(cx, |l, cx| {
        l.settings.background = settings.background.clone();
        cx.notify();
    });
    cx.run_until_parked();
    launcher.read_with(cx, |l, _| {
        let photo = l.ui.settings.appearance.photo.as_ref();
        assert!(
            photo.is_some_and(|(path, decoded)| *path == kept && decoded.is_some()),
            "the picture is decoded for the backdrop on Settings"
        );
    });
}

#[test]
fn picture_tone_is_weighted_by_visible_coverage() {
    let mut image = RgbaImage::from_pixel(3, 1, Rgba([255, 0, 0, 0]));
    assert_eq!(photo_tone(&image), None);
    image.put_pixel(1, 0, Rgba([90, 150, 210, 60]));
    image.put_pixel(2, 0, Rgba([30, 90, 150, 120]));
    assert_eq!(photo_tone(&image), Some(0x326eaa));
}

#[test]
fn complementary_colours_do_not_cancel_a_pictures_tint() {
    let mut image = RgbaImage::from_pixel(2, 1, Rgba([255, 0, 0, 255]));
    image.put_pixel(1, 0, Rgba([0, 255, 255, 255]));
    assert_eq!(photo_tone(&image), Some(0xff0000));
}

#[test]
fn the_theme_grid_takes_its_columns_from_the_width_it_is_given() {
    use super::rows::theme_columns;
    assert_eq!(theme_columns(0.), 1);
    assert_eq!(theme_columns(300.), 1);
    assert_eq!(theme_columns(760.), 4);
}
