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

/// Opens Settings › Appearance in a test window.
fn appearance_window(
    app: &mut gpui::TestAppContext,
) -> (gpui::Entity<crate::Launcher>, &mut gpui::VisualTestContext) {
    let (shell, cx) = app.add_window_view(crate::window_root::Shell::new);
    let launcher = shell.read_with(cx, |shell, _| shell.launcher());
    launcher.update(cx, |l, cx| {
        l.navigate(crate::frame::Page::Settings);
        l.settings_open(crate::settings::Section::Appearance);
        cx.notify();
    });
    cx.run_until_parked();
    (launcher, cx)
}

/// "+ New theme" makes a copy of the theme painted ("Sunrise copy"), chooses it and opens the
/// Edit dialog on it; a colour set there is painted at once, saved in the settings and kept in
/// the copy's own theme file, and Done keeps it.
#[test]
fn a_colour_set_in_the_edit_dialog_is_painted_and_kept_in_the_new_themes_file() {
    use sunrise_launcher::backend::Settings;
    use sunrise_launcher::themes::library;
    use sunrise_launcher::{appearance, theme_files};
    let test = "a_colour_set_in_the_edit_dialog_is_painted_and_kept_in_the_new_themes_file";
    if scratch_child(test).is_none() {
        return;
    }
    let mut app = gpui::TestAppContext::single();
    let (launcher, cx) = appearance_window(&mut app);
    cx.update(|window, cx| {
        launcher.update(cx, |l, cx| {
            l.theme_new(window);
            l.colour_set(library::ACCENT, 0x123456, window);
            cx.notify();
        })
    });
    cx.run_until_parked();
    let slug = launcher.read_with(cx, |l, _| {
        let edit =
            l.ui.settings
                .appearance
                .edit
                .as_ref()
                .expect("the dialog is open");
        // The copy is now the chosen theme, named after the one it was made from.
        assert_eq!(edit.start.name, l.current_name());
        assert!(edit.start.name.ends_with(" copy"), "{}", edit.start.name);
        assert_eq!(l.ui.sheet, Some(crate::sheets::Sheet::ThemeDialog));
        edit.slug.clone()
    });
    let saved = Settings::load();
    assert_eq!(saved.theme, Some(format!("local:{slug}")));
    assert_eq!(appearance::accent(&saved).rgb(), 0x123456);
    let file = theme_files::dir().join(&slug).join(theme_files::FILE);
    let text = std::fs::read_to_string(file).unwrap();
    assert!(text.contains("accent = \"#123456\""), "{text}");
    cx.update(|window, cx| {
        launcher.update(cx, |l, cx| {
            l.edit_done(window);
            cx.notify();
        })
    });
    launcher.read_with(cx, |l, _| {
        assert_eq!(l.editor_roles().accent, 0x123456);
        assert!(l.ui.settings.appearance.edit.is_none());
        assert!(l.ui.settings.appearance.reverting.is_none());
        assert_eq!(l.ui.sheet, None);
    });
    assert_eq!(Settings::load().theme, Some(format!("local:{slug}")));
}

/// Cancel (or Escape) deletes the theme made for the dialog and goes back to the look from
/// before it opened.
#[test]
fn cancelling_a_new_theme_deletes_it_and_restores_the_look() {
    use sunrise_launcher::backend::Settings;
    use sunrise_launcher::theme_files;
    use sunrise_launcher::themes::library;
    let test = "cancelling_a_new_theme_deletes_it_and_restores_the_look";
    if scratch_child(test).is_none() {
        return;
    }
    let mut app = gpui::TestAppContext::single();
    let (launcher, cx) = appearance_window(&mut app);
    let before = Settings::load().theme;
    cx.update(|window, cx| {
        launcher.update(cx, |l, cx| {
            l.theme_new(window);
            l.colour_set(library::ACCENT, 0x123456, window);
            l.close_sheet();
            l.edit_revert(window);
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert_eq!(Settings::load().theme, before);
    assert!(library::saved(&theme_files::dir()).is_empty());
    launcher.read_with(cx, |l, _| {
        assert!(l.ui.settings.appearance.edit.is_none());
        assert_ne!(l.editor_roles().accent, 0x123456);
    });
}

/// Whatever colour the player picks is painted and saved, however it reads on the others.
#[test]
fn a_colour_that_reads_badly_is_painted_and_saved_all_the_same() {
    use sunrise_launcher::backend::Settings;
    use sunrise_launcher::themes::library::SLOTS;
    if scratch_child("a_colour_that_reads_badly_is_painted_and_saved_all_the_same").is_none() {
        return;
    }
    let mut app = gpui::TestAppContext::single();
    let (launcher, cx) = appearance_window(&mut app);
    cx.update(|window, cx| {
        launcher.update(cx, |l, cx| {
            l.theme_new(window);
            // Mid grey page under white text.
            l.colour_set(0, 0x777777, window);
            cx.notify();
        })
    });
    cx.run_until_parked();
    let slug = launcher.read_with(cx, |l, _| {
        l.ui.settings
            .appearance
            .edit
            .as_ref()
            .map(|e| e.slug.clone())
    });
    assert_eq!(
        Settings::load().theme,
        slug.map(|slug| format!("local:{slug}")),
        "painted and saved"
    );
    launcher.read_with(cx, |l, _| {
        assert_eq!((SLOTS[0].get)(&l.editor_roles()), 0x777777);
    });
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
