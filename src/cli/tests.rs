use super::*;
use clap::Parser;

#[test]
fn attached_preserves_real_process_codes_and_unix_signals() {
    let mut codes = vec![0, 37];
    #[cfg(unix)]
    codes.push(143);
    #[cfg(windows)]
    codes.push(0xFEDC_BA98u32 as i32);
    for code in codes {
        let root = tempfile::tempdir().unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "cli::tests::attached_status_actor",
                "--nocapture",
            ])
            .env("SUNRISE_CLI_STATUS_ROLE", "wrapper")
            .env("SUNRISE_CLI_STATUS_CODE", code.to_string())
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("SUNRISE_NO_AUDIO", "1")
            .current_dir(root.path())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit());
        for (key, folder) in [
            ("SUNRISE_HOME", "logs"),
            ("SUNRISE_LAUNCHER_HOME", "launcher"),
            ("XDG_DATA_HOME", "xdg-data"),
            ("XDG_CONFIG_HOME", "xdg-config"),
            ("SUNRISE_LAUNCHER_TRASH", "trash"),
            ("SUNRISE_LAUNCHER_CLIPBOARD", "clipboard"),
            ("SUNRISE_LAUNCHER_DESKTOP", "desktop"),
        ] {
            command.env(key, root.path().join(folder));
        }
        assert_eq!(command.status().unwrap().code(), Some(code));
    }
}

#[test]
fn attached_status_actor() {
    let Ok(role) = std::env::var("SUNRISE_CLI_STATUS_ROLE") else {
        return;
    };
    let code: i32 = std::env::var("SUNRISE_CLI_STATUS_CODE")
        .unwrap()
        .parse()
        .unwrap();
    if role == "child" {
        assert!(std::env::var_os(sunrise_launcher_smod::holder::ENV).is_none());
        assert!(std::env::var_os(sunrise_launcher_smod::holder::ENTRY).is_none());
        use std::io::IsTerminal;
        let shape = format!(
            "{}:{}:{}",
            std::io::stdin().is_terminal(),
            std::io::stdout().is_terminal(),
            std::io::stderr().is_terminal()
        );
        assert_eq!(shape, std::env::var("SUNRISE_CLI_STDIO_SHAPE").unwrap());
        #[cfg(unix)]
        if code == 143 {
            unsafe {
                libc::signal(libc::SIGTERM, libc::SIG_DFL);
                libc::raise(libc::SIGTERM);
            }
            panic!("SIGTERM did not terminate the actor");
        }
        std::process::exit(code);
    }
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "cli::tests::attached_status_actor",
            "--nocapture",
        ])
        .env("SUNRISE_CLI_STATUS_ROLE", "child");
    use std::io::IsTerminal;
    command.env(
        "SUNRISE_CLI_STDIO_SHAPE",
        format!(
            "{}:{}:{}",
            std::io::stdin().is_terminal(),
            std::io::stdout().is_terminal(),
            std::io::stderr().is_terminal()
        ),
    );
    assert_eq!(attached(command).unwrap(), code);
    std::process::exit(code);
}

#[test]
fn completions_cover_each_shell_and_nested_commands() {
    for shell in [
        clap_complete::Shell::Bash,
        clap_complete::Shell::Zsh,
        clap_complete::Shell::Fish,
        clap_complete::Shell::Elvish,
        clap_complete::Shell::PowerShell,
    ] {
        let shell_name = shell.to_string();
        let args = ["sunrise-launcher", "completions", shell_name.as_str()].map(str::to_owned);
        let cli = Cli::try_parse_from(args).unwrap();
        assert!(matches!(cli.command, Some(Cmd::Completions { .. })));
        let mut output = Vec::new();
        clap_complete::generate(shell, &mut command(), "sunrise-launcher", &mut output);
        let script = String::from_utf8(output).unwrap();
        assert!(script.contains("sunrise-launcher"));
        assert!(script.contains("versions"));
        assert!(script.contains("multiplayer"));
    }
    assert!(Cli::try_parse_from(["sunrise-launcher", "completions"]).is_err());
    assert!(Cli::try_parse_from(["sunrise-launcher", "completions", "unknown"]).is_err());
}

/// The command line makes the same choices of look as Settings › Appearance does.
#[test]
fn settings_choose_the_look_as_appearance_does() {
    use sunrise_launcher::themes::{self, Accent};
    let mut settings = backend::Settings::parse("{}");
    set_setting(&mut settings, "theme", "nord").unwrap();
    assert_eq!(settings.theme.as_deref(), Some("nord"));
    assert_eq!(settings.accent, themes::NORD.accent.accent());
    set_setting(&mut settings, "accent", "#C2366F").unwrap();
    assert_eq!(settings.theme_accents["nord"].rgb(), 0xc2366f);
    set_setting(&mut settings, "theme", "default").unwrap();
    assert_eq!(settings.theme, None);
    assert_eq!(settings.accent.rgb(), 0xc2552d, "no saved theme is Dawn");
    set_setting(&mut settings, "theme", "nord").unwrap();
    assert_eq!(settings.accent.rgb(), 0xc2366f, "its colour is back");
    set_setting(&mut settings, "accent", "theme").unwrap();
    assert_eq!(settings.accent, themes::NORD.accent.accent());
    assert!(settings.theme_accents.is_empty());
    // "default" is still the violet, as it was before themes kept their accents.
    set_setting(&mut settings, "accent", "default").unwrap();
    assert_eq!(settings.accent, Accent::VIOLET);
    set_setting(&mut settings, "scenery", "off").unwrap();
    assert!(!settings.scenery());
    assert!(set_setting(&mut settings, "scenery", "maybe").is_err());
    set_setting(&mut settings, "scenery", "on").unwrap();
    assert!(settings.scenery());
    // What is shown says the theme painted and the one saved.
    let shown = settings_json(&settings);
    assert_eq!(
        (&shown["theme"], &shown["theme_choice"]),
        (&"nord".into(), &"nord".into())
    );
    assert_eq!(shown["scenery"], true);
    assert!(shown["themes_dir"].as_str().unwrap().ends_with("themes"));
}

#[test]
fn appearance_cli_sets_dark_light_or_system_and_the_json_says_so() {
    let mut settings = backend::Settings::parse("{}");
    assert_eq!(settings_json(&settings)["appearance"], "dark");
    for value in ["light", "system", "dark"] {
        set_setting(&mut settings, "appearance", value).unwrap();
        // Saved and read back, the choice is still the one the player made.
        settings = backend::Settings::parse(&serde_json::to_string(&settings).unwrap());
        assert_eq!(settings_json(&settings)["appearance"], value);
    }
    assert!(set_setting(&mut settings, "appearance", "sepia").is_err());
    assert_eq!(settings_json(&settings)["appearance"], "dark");
}

#[test]
fn reduced_graphics_cli_sets_and_reads_the_shared_choice() {
    let mut settings = backend::Settings::parse(
        r#"{"reduce_motion":false,"cat":true,
            "scenery":false,"background":"background-0123456789ab.png"}"#,
    );
    assert_eq!(settings_json(&settings)["reduced_graphics"], false);
    for (value, on) in [("on", true), ("off", false)] {
        let cli = Cli::try_parse_from(["sunrise-launcher", "settings", "reduced-graphics", value])
            .unwrap();
        let Some(Cmd::Settings {
            key: Some(key),
            value: Some(value),
        }) = cli.command
        else {
            panic!("expected settings key and value");
        };
        set_setting(&mut settings, &key, &value).unwrap();
        let before = serde_json::to_value(&settings).unwrap();
        assert!(set_setting(&mut settings, &key, "maybe").is_err());
        assert_eq!(serde_json::to_value(&settings).unwrap(), before);
        settings = backend::Settings::parse(&before.to_string());
        assert_eq!(settings_json(&settings)["reduced_graphics"], on);
        assert_eq!(settings.reduce_motion, Some(false));
        assert!(settings.show_cat());
        assert!(!settings.scenery());
        assert_eq!(
            settings.background.as_deref(),
            Some("background-0123456789ab.png")
        );
    }
}

#[test]
fn settings_accepts_german_and_chinese() {
    use crate::i18n::Language;
    let mut settings = backend::Settings::default();
    for (code, language) in [("de", Language::German), ("zh", Language::Chinese)] {
        set_setting(&mut settings, "language", code).unwrap();
        assert_eq!(settings.language, Some(language));
    }
    // A wrong code lists every language the launcher has, not only the first few.
    let wrong = set_setting(&mut settings, "language", "xx").unwrap_err();
    assert!(
        wrong.starts_with("xx: say system or one of en, "),
        "{wrong}"
    );
    for code in ["ja", "pt-br", "th", "pl"] {
        assert!(
            wrong.contains(&format!(" {code},")) || wrong.ends_with(&format!(" {code}")),
            "{code}: {wrong}"
        );
    }
    set_setting(&mut settings, "language", "system").unwrap();
    assert_eq!(settings.language, None);
}

#[test]
fn play_takes_an_account_for_one_launch() {
    let cli = Cli::try_parse_from(["sunrise-launcher", "play", "--account", "Alex"]).unwrap();
    match cli.command {
        Some(Cmd::Play(play::PlayArgs { account, .. })) => {
            assert_eq!(account.as_deref(), Some("Alex"))
        }
        _ => panic!("not play"),
    }
}

/// Play's own build refuses unreviewed mods and names this flag (#472), so play takes it too.
#[test]
fn play_takes_trust_native_code_for_one_launch() {
    let parse = |args: &[&str]| match Cli::try_parse_from(args).unwrap().command {
        Some(Cmd::Play(play::PlayArgs {
            trust_native_code, ..
        })) => trust_native_code,
        _ => panic!("not play"),
    };
    assert!(parse(&["sunrise-launcher", "play", "--trust-native-code"]));
    assert!(!parse(&["sunrise-launcher", "play"]));
}

#[test]
fn play_takes_extra_arguments_that_start_with_hyphens() {
    let cli = Cli::try_parse_from([
        "sunrise-launcher",
        "play",
        "--arg",
        "--server-thread",
        "--arg",
        "40",
    ])
    .unwrap();
    match cli.command {
        Some(Cmd::Play(play::PlayArgs { arg, .. })) => assert_eq!(arg, ["--server-thread", "40"]),
        _ => panic!("expected play"),
    }
}

#[test]
fn game_args_takes_one_line_that_starts_with_hyphens() {
    let cli = Cli::try_parse_from([
        "sunrise-launcher",
        "game-args",
        "--server-thread --render-distance 40",
    ])
    .unwrap();
    match cli.command {
        Some(Cmd::GameArgs { args }) => {
            assert_eq!(
                args.as_deref(),
                Some("--server-thread --render-distance 40")
            );
        }
        _ => panic!("expected game-args"),
    }
}

#[test]
fn mods_commands_name_their_instance() {
    let parse = |args: &[&str]| {
        let cli = Cli::try_parse_from([&["sunrise-launcher", "mods"], args].concat()).unwrap();
        match cli.command {
            Some(Cmd::Mods { action }) => action,
            _ => panic!("expected mods"),
        }
    };
    match parse(&["install", "fresh-animations", "--instance", "Skyblock"]) {
        Some(ModsCmd::Install {
            project, instance, ..
        }) => assert_eq!(
            (project.as_str(), instance.as_deref()),
            ("fresh-animations", Some("Skyblock"))
        ),
        _ => panic!("expected mods install"),
    }
    assert!(matches!(
        parse(&["list"]),
        Some(ModsCmd::List { instance: None })
    ));
    assert!(parse(&[]).is_none());
    match parse(&[
        "add-github",
        "https://github.com/a/b",
        "--mod",
        "zoom",
        "--all",
    ]) {
        Some(ModsCmd::AddGithub {
            url,
            mod_id,
            all,
            trust_native_code,
            ..
        }) => assert_eq!(
            (url.as_str(), mod_id.as_deref(), all, trust_native_code),
            ("https://github.com/a/b", Some("zoom"), true, false)
        ),
        _ => panic!("expected mods add-github"),
    }
    assert!(matches!(
        parse(&["update-github", "--check"]),
        Some(ModsCmd::UpdateGithub {
            check: true,
            id: None,
            ..
        })
    ));
    assert!(matches!(
        parse(&["search", "x", "--instance", "A"]),
        Some(ModsCmd::Search { instance: Some(i), .. }) if i == "A"
    ));
    // The Java mod check is gone.
    assert!(Cli::try_parse_from(["sunrise-launcher", "mods", "check", "x"]).is_err());
}

#[test]
fn native_build_sync_is_explicit_and_never_selects_an_instance() {
    let parse = |args: &[&str]| {
        Cli::try_parse_from(
            [
                &["sunrise-launcher", "native-build", "--sync-internals"],
                args,
            ]
            .concat(),
        )
    };
    assert!(matches!(
        parse(&["mod", "--trust-native-code"]).unwrap().command,
        Some(Cmd::NativeBuild(native::NativeBuildArgs {
            sync_internals: true,
            folder: Some(_),
            ..
        }))
    ));
    assert!(parse(&[]).is_err());
    for args in [
        ["mod", "--instance", "A"],
        ["mod", "--server-output", "out"],
        ["mod", "--profile", "developer"],
    ] {
        assert!(parse(&args).is_err());
    }
}

#[test]
fn native_build_export_mode_needs_a_folder_and_refuses_an_instance() {
    let parse = |args: &[&str]| {
        Cli::try_parse_from(
            [
                &["sunrise-launcher", "native-build", "--trust-native-code"],
                args,
            ]
            .concat(),
        )
    };
    // Export mode: a mod folder and an output folder.
    match parse(&["mods", "--server-output", "engine-out"])
        .unwrap()
        .command
    {
        Some(Cmd::NativeBuild(native::NativeBuildArgs {
            folder,
            server_output,
            instance,
            version,
            profile,
            ..
        })) => {
            assert_eq!(folder.as_deref(), Some(Path::new("mods")));
            assert_eq!(server_output.as_deref(), Some(Path::new("engine-out")));
            assert_eq!(instance, None);
            assert_eq!(version, None);
            assert_eq!(profile, None);
        }
        _ => panic!("expected native-build"),
    }
    assert!(matches!(
        parse(&[
            "mods",
            "--server-output",
            "engine-out",
            "--allow-jar-downloads"
        ])
        .unwrap()
        .command,
        Some(Cmd::NativeBuild(native::NativeBuildArgs {
            allow_jar_downloads: true,
            ..
        }))
    ));
    assert!(parse(&["mods", "--allow-jar-downloads"]).is_err());
    // The mod folder is explicit in export mode: no default from an instance.
    assert!(parse(&["--server-output", "engine-out"]).is_err());
    // Export mode never touches an instance.
    assert!(parse(&["mods", "--server-output", "engine-out", "--instance", "A"]).is_err());
    // Export mode accepts an explicit version and compilation profile.
    match parse(&[
        "mods",
        "--server-output",
        "engine-out",
        "--version",
        "v2026.10.13",
        "--profile",
        "optimized",
    ])
    .unwrap()
    .command
    {
        Some(Cmd::NativeBuild(native::NativeBuildArgs {
            version, profile, ..
        })) => {
            assert_eq!(version.as_deref(), Some("v2026.10.13"));
            assert_eq!(
                profile.map(native_mods::Profile::from),
                Some(native_mods::Profile::Optimized)
            );
        }
        _ => panic!("expected native-build"),
    }
    assert!(parse(&["mods", "--allow-version-mismatch"]).is_err());
    // The game mode is unchanged: an instance build without --server-output.
    assert!(matches!(
        parse(&["--instance", "A"]).unwrap().command,
        Some(Cmd::NativeBuild(native::NativeBuildArgs { instance: Some(i), server_output: None, .. })) if i == "A"
    ));
}

#[test]
fn export_mode_base_is_an_installed_release_not_a_native_build() {
    // The export mode picks the newest installed release or Nightly, never a native build
    // (`native-*`), and `--update` changes nothing when it already is the newest.
    let installed = vec![
        "26.3-0.1.1".to_string(),
        "26.3-0.1.0".to_string(),
        "native-2b03c1".to_string(),
    ];
    for update in [false, true] {
        assert_eq!(
            native_mods::base_for(None, &installed, update).as_deref(),
            Some("26.3-0.1.1")
        );
    }
    assert_eq!(native_mods::base_for(None, &[], false), None);
}
