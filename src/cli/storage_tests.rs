//! The `storage` command and settings compatibility.
use super::*;

#[test]
fn storage_command_and_legacy_cache_limit_setting() {
    assert!(matches!(
        Cli::try_parse_from(["sunrise-launcher", "storage"])
            .unwrap()
            .command,
        Some(Cmd::Storage { clear: None })
    ));
    assert!(matches!(
        Cli::try_parse_from(["sunrise-launcher", "storage", "--clear", "all"])
            .unwrap()
            .command,
        Some(Cmd::Storage {
            clear: Some(ClearStorage::All)
        })
    ));
    assert!(matches!(
        Cli::try_parse_from(["sunrise-launcher", "storage", "--clear", "mod-build-cache"])
            .unwrap()
            .command,
        Some(Cmd::Storage {
            clear: Some(ClearStorage::Row(Category::ModBuildCache))
        })
    ));
    assert!(Cli::try_parse_from(["sunrise-launcher", "storage", "--clear", "worlds"]).is_err());
    assert!(Cli::try_parse_from(["sunrise-launcher", "native-cache"]).is_err());
    assert!(
        Cli::try_parse_from(["sunrise-launcher", "storage", "--clear", "other-caches"]).is_ok()
    );
    assert!(
        Cli::try_parse_from(["sunrise-launcher", "settings", "cache-limit-gib", "25"]).is_err()
    );
    // Older settings.json files that still carry cache_limit_gib load cleanly and drop it on save.
    let old: backend::Settings =
        serde_json::from_str(r#"{"cache_limit_gib":25,"native_cache_gib":100,"cat":false}"#)
            .unwrap();
    assert!(!old.show_cat());
    let saved = serde_json::to_string(&old).unwrap();
    assert!(!saved.contains("cache_limit_gib"), "{saved}");
    assert!(!saved.contains("native_cache_gib"), "{saved}");
    let parsed = backend::Settings::parse(r#"{"cache_limit_gib":50,"cat":false}"#);
    assert!(!parsed.show_cat());
    assert!(!serde_json::to_string(&parsed)
        .unwrap()
        .contains("cache_limit_gib"));
}

#[test]
fn the_cat_is_on_until_the_player_switches_it_off() {
    let mut settings = backend::Settings::default();
    assert!(settings.show_cat());
    assert!(serde_json::from_str::<backend::Settings>("{}")
        .unwrap()
        .show_cat());
    let old: backend::Settings = serde_json::from_str(r#"{"native_cache_gib":100}"#).unwrap();
    assert!(old.show_cat());
    assert_eq!(settings_json(&settings)["cat"], true);
    set_setting(&mut settings, "cat", "off").unwrap();
    assert!(!settings.show_cat());
    assert_eq!(settings_json(&settings)["cat"], false);
    let loaded: backend::Settings =
        serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
    assert!(!loaded.show_cat());
    set_setting(&mut settings, "cat", "on").unwrap();
    assert!(settings.show_cat());
    assert!(set_setting(&mut settings, "cat", "maybe").is_err());
    assert!(Cli::try_parse_from(["sunrise-launcher", "settings", "cat", "off"]).is_ok());
}

#[test]
fn the_worlds_setting_picks_rows_or_tiles() {
    use backend::WorldLayout;
    let mut settings = backend::Settings::default();
    assert_eq!(settings_json(&settings)["world_layout"], "rows");
    set_setting(&mut settings, "worlds", "tiles").unwrap();
    assert_eq!(settings.world_layout, WorldLayout::Tiles);
    assert_eq!(settings_json(&settings)["world_layout"], "tiles");
    let loaded = backend::Settings::parse(&serde_json::to_string(&settings).unwrap());
    assert_eq!(loaded.world_layout, WorldLayout::Tiles);
    assert!(set_setting(&mut settings, "worlds", "grid").is_err());
    assert_eq!(settings.world_layout, WorldLayout::Tiles);
    set_setting(&mut settings, "worlds", "rows").unwrap();
    assert_eq!(settings.world_layout, WorldLayout::Rows);
    assert!(Cli::try_parse_from(["sunrise-launcher", "settings", "worlds", "tiles"]).is_ok());
}

#[cfg(unix)]
#[test]
fn storage_scopes_clear_and_redacts_cli_errors() {
    const ACTOR: &str = "SUNRISE_STORAGE_CLI_TEST";
    if let Ok(mode) = std::env::var(ACTOR) {
        let (clear, as_json) = match mode.as_str() {
            "scoped" => (Some(ClearStorage::Row(Category::RustCompiler)), true),
            "fatal" => (Some(ClearStorage::Row(Category::ModBuildCache)), true),
            "kept" => (Some(ClearStorage::Row(Category::BuildFiles)), true),
            _ => (None, false),
        };
        std::process::exit(execute(Cmd::Storage { clear }, as_json).unwrap());
    }
    let scratch = tempfile::tempdir().unwrap();
    let private = scratch.path().join("private-home-canary");
    let launcher = private.join("launcher");
    let child = |mode: &str| {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "cli::storage_tests::storage_scopes_clear_and_redacts_cli_errors",
                // The child exits with the CLI status before libtest can emit captured stdout.
                "--nocapture",
                "--quiet",
            ])
            .env(ACTOR, mode)
            .env("SUNRISE_LAUNCHER_HOME", &launcher)
            .env("SUNRISE_HOME", private.join("game"))
            .env("SUNRISE_LAUNCHER_LOCATION_FILE", private.join("location"))
            .env("SUNRISE_NO_KEYCHAIN", "1");
        command
    };
    let run = |mode| child(mode).output().unwrap();
    let rust = launcher.join("rust/old/output");
    let native = launcher.join("native");
    let outside = private.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::create_dir_all(rust.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&native).unwrap();
    std::fs::write(&rust, b"output").unwrap();
    std::os::unix::fs::symlink(&outside, native.join("w")).unwrap();
    let output = run("scoped");
    assert!(output.status.success(), "{output:?}");
    assert!(!rust.exists());
    let output = run("fatal");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("unexpected file or link"), "{stderr}");
    assert!(!stderr.contains("private-home-canary"), "{stderr}");
    std::fs::remove_file(native.join("w")).unwrap();
    let attempt = native.join("source/revision");
    std::fs::create_dir_all(&attempt).unwrap();
    std::os::unix::fs::symlink(&outside, attempt.join("unexpected")).unwrap();
    let output = run("kept");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let body: serde_json::Value =
        serde_json::from_str(&stdout[stdout.find('{').unwrap()..]).unwrap();
    assert_eq!(body["rows"][0]["category"], "build-files");
    assert_eq!(body["rows"][0]["kept"].as_array().unwrap().len(), 1);
    assert!(!stdout.contains("private-home-canary"), "{stdout}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Build files"), "{stderr}");
    assert!(!stderr.contains("private-home-canary"), "{stderr}");
    let output = run("count");
    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Counted Rust compiler:"), "{stderr}");

    use std::os::{fd::OwnedFd, unix::net::UnixStream};
    let (reader, writer) = UnixStream::pair().unwrap();
    drop(reader);
    let output = child("count")
        .stderr(std::process::Stdio::from(OwnedFd::from(writer)))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let body = String::from_utf8(output.stdout).unwrap();
    let report: serde_json::Value = serde_json::from_str(&body[body.find('{').unwrap()..]).unwrap();
    assert_eq!(
        report["rows"].as_array().unwrap().len(),
        Category::ALL.len()
    );
}

#[test]
fn storage_json_count_prints_valid_json_with_empty_stderr() {
    const ACTOR: &str = "SUNRISE_STORAGE_JSON_CLI_TEST";
    if std::env::var_os(ACTOR).is_some() {
        std::process::exit(execute(Cmd::Storage { clear: None }, true).unwrap());
    }
    let scratch = tempfile::tempdir().unwrap();
    let launcher = scratch.path().join("launcher");
    let rust = launcher.join("rust/old/output");
    std::fs::create_dir_all(rust.parent().unwrap()).unwrap();
    std::fs::write(&rust, b"output").unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "cli::storage_tests::storage_json_count_prints_valid_json_with_empty_stderr",
            // The child exits with the CLI status before libtest can emit captured stdout.
            "--nocapture",
            "--quiet",
        ])
        .env(ACTOR, "1")
        .env("SUNRISE_LAUNCHER_HOME", &launcher)
        .env("SUNRISE_HOME", scratch.path().join("game"))
        .env(
            "SUNRISE_LAUNCHER_LOCATION_FILE",
            scratch.path().join("location"),
        )
        .env("SUNRISE_NO_KEYCHAIN", "1")
        .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
        .env("XDG_DATA_HOME", scratch.path().join("xdg-data"))
        .env("XDG_CONFIG_HOME", scratch.path().join("xdg-config"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.is_empty(), "expected empty stderr, got: {stderr}");
    assert!(!stderr.contains("Counted"), "{stderr}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json = stdout
        .find('{')
        .map(|start| &stdout[start..])
        .unwrap_or(&stdout);
    let report: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(
        report["rows"].as_array().unwrap().len(),
        Category::ALL.len()
    );
}
