use super::*;
use crate::{account, mojang, sources};
use std::cmp::Ordering::*;
use std::ffi::OsStr;
use std::io::Read;
use std::path::PathBuf;
use std::process::Stdio;

fn settings_file_reads(data: &Path) -> u64 {
    SETTINGS_FILE_READS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(data)
        .copied()
        .unwrap_or(0)
}

#[test]
fn settings_load_reads_the_file_once_until_it_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(crate::paths::SETTINGS);
    fs::write(&path, r#"{"selected":"first"}"#).unwrap();
    let before = settings_file_reads(dir.path());
    for _ in 0..10_000 {
        assert_eq!(
            Settings::load_cached_from(dir.path()).selected.as_deref(),
            Some("first")
        );
    }
    assert_eq!(settings_file_reads(dir.path()) - before, 1);

    let mut saved = Settings::load_cached_from(dir.path());
    saved.selected = Some("saved".into());
    saved.save_to(dir.path()).unwrap();
    assert_eq!(
        Settings::load_cached_from(dir.path()).selected.as_deref(),
        Some("saved")
    );

    fs::write(&path, r#"{"selected":"external edit"}"#).unwrap();
    assert_eq!(
        Settings::load_cached_from(dir.path()).selected.as_deref(),
        Some("external edit")
    );
    assert_eq!(settings_file_reads(dir.path()) - before, 3);
}

/// The parsed cache is keyed by data folder, so two homes (or a test's scratch home and the
/// player's) never see each other's settings, and the first answer stays stable.
#[test]
fn settings_cache_is_per_data_folder() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    fs::write(
        first.path().join(crate::paths::SETTINGS),
        r#"{"selected":"one"}"#,
    )
    .unwrap();
    fs::write(
        second.path().join(crate::paths::SETTINGS),
        r#"{"selected":"two"}"#,
    )
    .unwrap();
    let selected = |data: &Path| Settings::load_cached_from(data).selected;
    assert_eq!(selected(first.path()).as_deref(), Some("one"));
    assert_eq!(selected(second.path()).as_deref(), Some("two"));
    assert_eq!(selected(first.path()).as_deref(), Some("one"));
}

#[cfg(unix)]
#[test]
fn a_cancellable_zip_unpack_keeps_every_executable_runnable_and_stops_when_asked() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let zip_path = dir.path().join("game.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
    let exec = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
    for name in [
        "Sunrise.app/Contents/MacOS/sunrise-client",
        "Sunrise.app/Contents/MacOS/sunrise-prepare",
    ] {
        zip.start_file(name, exec).unwrap();
        zip.write_all(b"#!/bin/sh\n").unwrap();
    }
    zip.start_file(
        "Sunrise.app/Contents/Info.plist",
        zip::write::SimpleFileOptions::default().unix_permissions(0o644),
    )
    .unwrap();
    zip.write_all(b"<plist/>").unwrap();
    zip.finish().unwrap();

    let to = dir.path().join("unpacked");
    let stop = crate::native_mods::Cancellation::default();
    unpack(&zip_path, "game.zip", &to, Some(&stop)).unwrap();
    for name in ["sunrise-client", "sunrise-prepare"] {
        let mode = fs::metadata(to.join("Sunrise.app/Contents/MacOS").join(name))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "{name} stays executable");
    }

    stop.stop();
    assert!(unpack(
        &zip_path,
        "game.zip",
        &dir.path().join("stopped"),
        Some(&stop)
    )
    .is_err());
}

#[cfg(unix)]
#[test]
fn private_files_are_readable_by_their_owner_alone() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.tmp");
    write_private(&file, b"{\"proxy\":\"http://u:p@h:1\"}").unwrap();
    let mode = fs::metadata(&file).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    assert_eq!(fs::read(&file).unwrap(), b"{\"proxy\":\"http://u:p@h:1\"}");
}

#[cfg(unix)]
#[test]
fn a_leftover_looser_file_is_made_private_too() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.1.tmp");
    fs::write(&file, b"old old old").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    write_private(&file, b"new").unwrap();
    assert_eq!(
        fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(fs::read(&file).unwrap(), b"new");
}

#[cfg(unix)]
#[test]
fn saving_settings_leaves_settings_json_private() {
    use std::os::unix::fs::PermissionsExt;
    Settings::default().save().unwrap();
    let mode = fs::metadata(data_dir().join("settings.json"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn a_cached_server_jar_that_fails_its_sha1_is_fetched_again() {
    let dir = std::env::temp_dir().join(format!("sunrise-jar-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let jar = dir.join("server-jars/26.3.jar");
    let good = b"the real server jar".to_vec();
    // The sha1 of `good`, from `printf 'the real server jar' | sha1sum`.
    let manifest = serde_json::json!({"downloads": {"server": {
        "url": "https://piston-data.mojang.com/server.jar",
        "sha1": "0527c3af27483cb5b7a62b046066baf42588a449"}}});
    let served = |body: Vec<u8>| move |_: &str| Ok(body.clone());
    let nothing = |_: String| {};
    // A captive portal's page was cached: it is replaced.
    fs::create_dir_all(jar.parent().unwrap()).unwrap();
    fs::write(&jar, b"<html>sign in</html>").unwrap();
    ensure_server_jar(&jar, &manifest, &served(good.clone()), &nothing).unwrap();
    assert_eq!(fs::read(&jar).unwrap(), good);
    // A whole cached jar is not fetched again.
    let refuse = |_: &str| -> Result<Vec<u8>, String> { Err("fetched".into()) };
    ensure_server_jar(&jar, &manifest, &refuse, &nothing).unwrap();
    // A bad download is refused and never cached.
    fs::remove_file(&jar).unwrap();
    let error = ensure_server_jar(&jar, &manifest, &served(b"cut".to_vec()), &nothing);
    assert!(error.is_err());
    assert!(!jar.exists());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_server_jar_whose_version_file_has_no_sha1_is_kept_and_not_checked() {
    let dir = std::env::temp_dir().join(format!("sunrise-jar-nosha-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let jar = dir.join("server-jars/26.3.jar");
    let manifest = serde_json::json!({"downloads": {"server": {
        "url": "https://piston-data.mojang.com/server.jar"}}});
    let nothing = |_: String| {};
    let served = |_: &str| Ok(b"whatever came down".to_vec());
    // With nothing to check against, a download is kept as it came.
    ensure_server_jar(&jar, &manifest, &served, &nothing).unwrap();
    assert_eq!(fs::read(&jar).unwrap(), b"whatever came down");
    // And a cached jar is trusted, not fetched again.
    let refuse = |_: &str| -> Result<Vec<u8>, String> { Err("fetched".into()) };
    ensure_server_jar(&jar, &manifest, &refuse, &nothing).unwrap();
    assert_eq!(fs::read(&jar).unwrap(), b"whatever came down");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_launchers_build_is_its_selected_one_when_installed_else_the_newest() {
    let installed = ["v0.4.0".to_owned(), "v0.3.0".to_owned()];
    assert_eq!(launcher_build(Some("v0.3.0"), &installed), Some("v0.3.0"));
    assert_eq!(launcher_build(Some("v0.2.0"), &installed), Some("v0.4.0"));
    assert_eq!(launcher_build(None, &installed), Some("v0.4.0"));
    assert_eq!(launcher_build(Some("v0.3.0"), &[]), None);
}

#[test]
fn an_installed_build_keeps_its_release_file_only_when_it_is_the_signed_one() {
    let dir = tempfile::tempdir().unwrap();
    let file = br#"{"version_id":"26.4-pre1","vanilla":"26.4 Pre-Release 1"}"#;
    let signed = sunrise_launcher_release_verify::sha256_hex(file);
    assert!(keep_release_file(dir.path(), b"{}", &signed).is_err());
    assert!(!dir.path().join(RELEASE_FILE).exists());
    keep_release_file(dir.path(), file, &signed).unwrap();
    let kept = read_json(&dir.path().join(RELEASE_FILE)).unwrap();
    assert_eq!(kept["version_id"], "26.4-pre1");
}

#[test]
fn the_played_game_comes_from_the_release_file_else_the_lock_else_the_pin() {
    let versions = tempfile::tempdir().unwrap();
    let v = versions.path();
    let played = |selected, field| played_in(v, selected, field);
    assert_eq!(played(None, "version_id"), PINNED_GAME);
    for (tag, files) in [
        ("v0.3.1", vec![]),
        (
            "v0.3.2",
            vec![("research-lock.json", r#"{"minecraft":{"version":"26.2"}}"#)],
        ),
        (
            "v0.3.3",
            vec![
                ("research-lock.json", r#"{"minecraft":{"version":"26.2"}}"#),
                (
                    "sunrise-release.json",
                    r#"{"version_id":"26.4-pre1","vanilla":"26.4 Pre-Release 1"}"#,
                ),
            ],
        ),
        (
            "v0.3.4",
            vec![("research-lock.json", r#"{"minecraft":{"version":"26.9"}}"#)],
        ),
    ] {
        fs::create_dir_all(v.join(tag)).unwrap();
        for (name, text) in files {
            fs::write(v.join(tag).join(name), text).unwrap();
        }
    }
    assert_eq!(
        played(None, "version_id"),
        PINNED_GAME,
        "only installed builds count"
    );
    for tag in ["v0.3.1", "v0.3.2", "v0.3.3"] {
        fs::write(v.join(tag).join(COMPLETE), "").unwrap();
    }
    assert_eq!(played(None, "version_id"), "26.4-pre1");
    assert_eq!(played(None, "vanilla"), "26.4 Pre-Release 1");
    assert_eq!(played(Some("v0.3.2"), "vanilla"), "26.2");
    assert_eq!(played(Some("v0.3.1"), "version_id"), PINNED_GAME);
    assert_eq!(played(Some("v9.9.9"), "version_id"), "26.4-pre1");
}

fn pe_architecture(bytes: &[u8]) -> Option<GameArchitecture> {
    if bytes.get(..2)? != b"MZ" {
        return None;
    }
    let pe = u32::from_le_bytes(bytes.get(0x3c..0x40)?.try_into().ok()?) as usize;
    if bytes.get(pe..pe.checked_add(4)?)? != b"PE\x00\x00" {
        return None;
    }
    architecture_for_pe_machine(u16::from_le_bytes(
        bytes
            .get(pe.checked_add(4)?..pe.checked_add(6)?)?
            .try_into()
            .ok()?,
    ))
}

#[test]
#[cfg(windows)]
fn failed_removal_is_incomplete_and_can_be_retried() {
    use std::os::windows::fs::OpenOptionsExt;
    let Some(root) = std::env::var_os("REMOVE_TEST_ROOT") else {
        let root = tempfile::tempdir().unwrap();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "backend::tests::failed_removal_is_incomplete_and_can_be_retried"
            ])
            .env("REMOVE_TEST_ROOT", root.path())
            .env("SUNRISE_LAUNCHER_HOME", root.path())
            .env("SUNRISE_HOME", root.path().join("home"))
            .env("XDG_DATA_HOME", root.path().join("xdg-data"))
            .env("XDG_CONFIG_HOME", root.path().join("xdg-config"))
            .env("SUNRISE_LAUNCHER_TRASH", root.path().join("trash"))
            .env("SUNRISE_LAUNCHER_CLIPBOARD", root.path().join("clipboard"))
            .env("SUNRISE_LAUNCHER_DESKTOP", root.path().join("desktop"))
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("SUNRISE_NO_AUDIO", "1")
            .status()
            .unwrap()
            .success());
        return;
    };
    let versions = PathBuf::from(root).join("versions");
    let dir = versions.join("v0.2.0");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(COMPLETE), b"").unwrap();
    fs::write(dir.join(CLIENT), sunrise_launcher_smod::holder::CAPABILITY).unwrap();
    crate::run_leases::publish(&dir).unwrap();
    let running = fs::File::options()
        .read(true)
        .share_mode(1)
        .open(dir.join(CLIENT))
        .unwrap();
    assert!(crate::cache_cleanup::delete_game_unit(&dir).is_err());
    assert!(installed_in(&versions).is_empty());
    assert!(!dir.join(COMPLETE).exists());
    drop(running);
    crate::cache_cleanup::delete_game_unit_observed(&dir, || {
        assert!(matches!(
            crate::run_leases::installation(),
            Err(crate::native_cache::Error::Busy)
        ));
    })
    .unwrap();
    assert!(!dir.exists());
}

/// A build from before runtime leases has no lease marker in its client; with nothing
/// running from it, removing it must still work.
#[test]
fn a_version_without_the_lease_marker_can_be_removed_when_idle() {
    let dir = version_dir("v0.0.1-preleases");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(COMPLETE), b"").unwrap();
    fs::write(dir.join(CLIENT), b"a client from before leases").unwrap();
    assert!(installed().contains(&"v0.0.1-preleases".to_string()));
    remove("v0.0.1-preleases").unwrap();
    assert!(!dir.exists());
    assert!(!installed().contains(&"v0.0.1-preleases".to_string()));
}

/// The launcher downloads Mojang's game files (client jar, libraries, assets) only for an
/// account the services said owns the game: never without one, and never past the gate by
/// the developer option (`Settings::dev_offline`), whose pass has no owner.
#[test]
fn mojang_downloads_only_for_an_owner() {
    let dir = std::env::temp_dir().join(format!("sunrise-owner-only-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let pins = sources::Pins::read(&serde_json::json!({ "minecraft": {
        "version": "26.3",
        "client_sha1": "00",
        "manifest_sha1": "00",
    }}))
    .unwrap();
    let asked = std::sync::Mutex::new(Vec::<String>::new());
    let fetch = |url: &str| -> Result<Vec<u8>, String> {
        asked.lock().unwrap().push(url.to_string());
        Err("no network in tests".into())
    };
    let log = |_: String| {};
    let no_owner = || Ok(account::Launch::default());
    let store = sources::Store {
        dir: &dir,
        sources: &[],
    };
    for (pass, signed_in) in [
        (Err(account::NoOwner::default()), false),
        (Err(account::NoOwner::default()), true),
        (no_owner(), false),
        (no_owner(), true),
    ] {
        assert!(download_for(
            &pins,
            &store,
            pass,
            signed_in,
            &fetch,
            &mojang::CANCEL,
            &log
        )
        .is_err());
    }
    assert!(asked.lock().unwrap().is_empty());
    assert!(!dir.exists());
    // With no account, the message stands alone once its bullet is taken off.
    let said = download_for(
        &pins,
        &store,
        Err(account::NoOwner::default()),
        false,
        &fetch,
        &mojang::CANCEL,
        &log,
    )
    .unwrap_err();
    assert_eq!(
        said.message.trim_start_matches("\n  • "),
        "Sign in with the Microsoft account that owns Minecraft: Sunrise then downloads 26.3 from Mojang itself."
    );
    // An owner: Mojang is asked (and the download fails here, without network).
    let owner = account::Account {
        uuid: "069a79f444e94726a5befca90e38aaf5".into(),
        name: "Notch".into(),
        skin: None,
        cape: None,
    };
    let secrets = serde_json::from_str(
        r#"{"refresh_token":"r","minecraft_token":"t","minecraft_expires_at":1,"owns":true}"#,
    )
    .unwrap();
    let pass = Ok(account::Launch {
        owner: Some((owner, secrets)),
        ..Default::default()
    });
    assert!(download_for(&pins, &store, pass, true, &fetch, &mojang::CANCEL, &log).is_err());
    assert!(!asked.lock().unwrap().is_empty());
    let _ = fs::remove_dir_all(&dir);

    // A store that lacks assets is not filled for a non-owner either: nothing is asked.
    fs::create_dir_all(dir.join("assets/indexes")).unwrap();
    fs::write(
        dir.join("assets/indexes/34.json"),
        r#"{"objects":{"a":{"hash":"00","size":1}}}"#,
    )
    .unwrap();
    asked.lock().unwrap().clear();
    for (pass, signed_in) in [(Err(account::NoOwner::default()), true), (no_owner(), true)] {
        assert!(download_for(
            &pins,
            &store,
            pass,
            signed_in,
            &fetch,
            &mojang::CANCEL,
            &log
        )
        .is_err());
    }
    assert!(asked.lock().unwrap().is_empty());
    let _ = fs::remove_dir_all(&dir);

    // In the sources: one call downloads the game, in `download_for` (backend/prepare/assets.rs),
    // and the only caller of that asks the strict gate. Game-file hosts appear nowhere else.
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    const MOVED_FROM_BACKEND: &[&str] = &[
        "backend/architecture.rs",
        "backend/http.rs",
        "backend/releases.rs",
        "backend/install.rs",
        "backend/prepare.rs",
        "backend/prepare/state.rs",
        "backend/prepare/reports.rs",
        "backend/prepare/reuse.rs",
        "backend/prepare/assets.rs",
        "backend/prepare/java.rs",
        "backend/self_update.rs",
        "backend/runtime.rs",
        "backend/player_settings.rs",
        "backend/game_dirs.rs",
        "backend/play_commands.rs",
    ];
    let mut files = vec![src.clone()];
    let mut sources = Vec::new();
    while let Some(path) = files.pop() {
        if path.is_dir() {
            files.extend(fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
        } else if path.extension().is_some_and(|e| e == "rs") {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            // Code split out of backend.rs is still code, though it now sits in a folder.
            let moved = path
                .strip_prefix(&src)
                .is_ok_and(|p| MOVED_FROM_BACKEND.iter().any(|m| p == Path::new(m)));
            let in_tests = !moved && (name.contains("tests") || path.parent().unwrap() != src);
            // The code alone: test modules (this one included) come after it.
            // A Windows checkout has CRLF line ends: the searches below look for "\n".
            let mut text = fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
            text.truncate(text.find("#[cfg(test)]").unwrap_or(text.len()));
            // Looked up by path below: two files can share a name (`reuse.rs`).
            let path = path
                .strip_prefix(&src)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            sources.push((name, in_tests, text, path));
        }
    }
    let calls: Vec<_> = sources
        .iter()
        .filter(|(_, tests, ..)| !tests)
        .flat_map(|(name, _, text, _)| {
            text.matches("mojang::download(")
                .map(move |_| name.as_str())
        })
        .collect();
    assert_eq!(calls, ["assets.rs"]);
    let source = |file: &str| &sources.iter().find(|(.., p)| p == file).unwrap().2;
    let assets = source("backend/prepare/assets.rs");
    let body = |f: &str| {
        let start = assets.find(&format!(" fn {f}(")).unwrap();
        &assets[start..start + assets[start + 1..].find("\n}\n").unwrap()]
    };
    assert!(body("download_for").contains("mojang::download("));
    assert!(body("download_for_owner").contains("account::launch("));
    assert!(!body("download_for_owner").contains("launch_for_game("));
    // `download_for` is defined once and called once, by `download_for_owner`.
    assert_eq!(assets.matches("download_for(").count(), 2);
    assert!(body("download_for_owner").contains("download_for("));
    // Refilling an incomplete store asks the same gate and names no host or download itself.
    assert!(body("refill_own_store").contains("download_for_owner("));
    assert!(!body("refill_own_store").contains("mojang::"));
    // It runs at start-up, never on Play, which also runs `complete_assets`.
    let public = |file: &str, f: &str| {
        let text = source(file);
        let start = text.find(&format!("\npub fn {f}(")).unwrap();
        &text[start..start + text[start + 1..].find("\n}\n").unwrap()]
    };
    assert!(
        public("backend/prepare/reuse.rs", "reuse_prepared_everywhere")
            .contains("refill_own_store(")
    );
    assert!(!public("backend/prepare/assets.rs", "complete_assets").contains("refill_own_store("));
    for host in [
        "piston-data",
        "libraries.minecraft.net",
        "resources.download.minecraft.net",
    ] {
        for (name, tests, text, _) in &sources {
            assert!(
                *tests || name == "mojang.rs" || !text.contains(host),
                "{name} names {host}"
            );
        }
    }
}

/// A signed-in owner prepares from Sunrise's own pinned store even when another launcher
/// (e.g. Modrinth App) has the matching client jar alongside a re-issued/patched library:
/// matching files are copied locally and differing ones are downloaded from Mojang.
#[test]
fn signed_in_owner_prepares_from_pinned_store_copying_matching_and_downloading_differing() {
    let root = tempfile::tempdir().unwrap();
    let own = root.path().join("own");
    let modrinth = root.path().join("ModrinthApp/meta");
    let client = b"fake 26.3 client jar";
    let common = b"unchanged common library";
    let pinned_freetype = b"pinned lwjgl-freetype 3.4.3";
    let sound = b"fake sound ogg";
    let index = serde_json::json!({"objects": {
        "minecraft/sounds/a.ogg": {"hash": sources::sha1_hex(sound), "size": sound.len()},
    }})
    .to_string();
    let manifest = serde_json::json!({
        "id": "26.3",
        "assetIndex": {
            "id": "34",
            "sha1": sources::sha1_hex(index.as_bytes()),
            "url": "https://piston-meta.mojang.com/v1/packages/idx/34.json",
        },
        "downloads": {"client": {
            "sha1": sources::sha1_hex(client),
            "size": client.len(),
            "url": "https://piston-data.mojang.com/client.jar",
        }},
        "libraries": [
            {"name": "org.example:common:1.0", "downloads": {"artifact": {
                "path": "org/example/common/1.0/common-1.0.jar",
                "sha1": sources::sha1_hex(common),
                "url": "https://libraries.minecraft.net/org/example/common/1.0/common-1.0.jar",
            }}},
            {"name": "org.lwjgl:lwjgl-freetype:3.4.3", "downloads": {"artifact": {
                "path": "org/lwjgl/lwjgl-freetype/3.4.3/lwjgl-freetype-3.4.3.jar",
                "sha1": sources::sha1_hex(pinned_freetype),
                "url": "https://libraries.minecraft.net/org/lwjgl/lwjgl-freetype/3.4.3/lwjgl-freetype-3.4.3.jar",
            }}},
        ],
    })
    .to_string();
    let manifest_sha1 = sources::sha1_hex(manifest.as_bytes());
    let pins = sources::Pins::read(&serde_json::json!({"minecraft": {
        "version": "26.3",
        "client_sha1": sources::sha1_hex(client),
        "manifest_sha1": manifest_sha1,
    }}))
    .unwrap();

    // Modrinth has matching client jar + common library, plus a patched freetype instead of 3.4.3.
    let write = |base: &Path, rel: &str, data: &[u8]| {
        let p = base.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, data).unwrap();
    };
    write(&modrinth, "versions/26.3/26.3.jar", client);
    write(
        &modrinth,
        "libraries/org/example/common/1.0/common-1.0.jar",
        common,
    );
    write(
        &modrinth,
        "libraries/org/lwjgl/lwjgl-freetype/3.4.3-mojang-2.15.0/lwjgl-freetype-3.4.3-mojang-2.15.0.jar",
        b"re-issued patched freetype",
    );

    let sound_hash = sources::sha1_hex(sound);
    let fixture = std::collections::HashMap::from([
        (
            format!("https://piston-meta.mojang.com/v1/packages/{manifest_sha1}/26.3.json"),
            manifest.into_bytes(),
        ),
        (
            "https://piston-data.mojang.com/client.jar".to_string(),
            client.to_vec(),
        ),
        (
            "https://libraries.minecraft.net/org/example/common/1.0/common-1.0.jar".to_string(),
            common.to_vec(),
        ),
        (
            "https://libraries.minecraft.net/org/lwjgl/lwjgl-freetype/3.4.3/lwjgl-freetype-3.4.3.jar".to_string(),
            pinned_freetype.to_vec(),
        ),
        (
            "https://piston-meta.mojang.com/v1/packages/idx/34.json".to_string(),
            index.into_bytes(),
        ),
        (
            format!(
                "{}/{}/{sound_hash}",
                sunrise_launcher_addresses::MOJANG_RESOURCES,
                &sound_hash[..2]
            ),
            sound.to_vec(),
        ),
    ]);
    let asked = std::sync::Mutex::new(Vec::<String>::new());
    let fetch = |url: &str| -> Result<Vec<u8>, String> {
        asked.lock().unwrap().push(url.to_string());
        fixture
            .get(url)
            .cloned()
            .ok_or_else(|| format!("unexpected url {url}"))
    };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let candidate_list = [modrinth];
    let store = sources::Store {
        dir: &own,
        sources: &candidate_list,
    };

    let options = PrepareStoreOptions {
        os: "linux",
        reused: false,
        allow_local_without_owner: false,
        local_sources: &[],
    };
    let folder = prepare_store_with(
        &pins,
        &store,
        &options,
        &cancel,
        &|| {
            mojang::download(&pins, &store, "linux", 2, &fetch, &cancel, &|_| {}).map_err(|e| {
                OwnerDownloadError {
                    verified_owner: true,
                    message: e,
                }
            })
        },
        &|_| {},
    )
    .unwrap();
    assert_eq!(folder, own);
    assert_eq!(
        fs::read(own.join("libraries/org/lwjgl/lwjgl-freetype/3.4.3/lwjgl-freetype-3.4.3.jar"))
            .unwrap(),
        pinned_freetype
    );
    let asked = asked.lock().unwrap();
    assert!(
        !asked.iter().any(|u| u.ends_with("client.jar")),
        "matching client jar should be copied from Modrinth: {asked:#?}"
    );
    assert!(
        !asked.iter().any(|u| u.contains("common-1.0.jar")),
        "matching common library should be copied from Modrinth: {asked:#?}"
    );
    assert!(
        asked.iter().any(|u| u.contains("lwjgl-freetype-3.4.3.jar")),
        "missing pinned freetype should be downloaded from Mojang: {asked:#?}"
    );

    // An unsigned-in ordinary player never borrows another launcher's install: they are told to
    // sign in, even when another launcher has a complete pinned copy.
    let fresh_own = root.path().join("fresh-own");
    let complete_other = root.path().join("CompleteOther");
    let manifest_url =
        format!("https://piston-meta.mojang.com/v1/packages/{manifest_sha1}/26.3.json");
    write(
        &complete_other,
        "versions/26.3/26.3.json",
        &fixture[&manifest_url],
    );
    write(&complete_other, "versions/26.3/26.3.jar", client);
    write(
        &complete_other,
        "libraries/org/example/common/1.0/common-1.0.jar",
        common,
    );
    write(
        &complete_other,
        "libraries/org/lwjgl/lwjgl-freetype/3.4.3/lwjgl-freetype-3.4.3.jar",
        pinned_freetype,
    );
    let other_sources = [complete_other];
    let fresh_store = sources::Store {
        dir: &fresh_own,
        sources: &other_sources,
    };
    let sign_in_err = prepare_store_with(
        &pins,
        &fresh_store,
        &PrepareStoreOptions {
            os: "linux",
            reused: false,
            allow_local_without_owner: false,
            local_sources: &other_sources,
        },
        &cancel,
        &|| {
            Err(OwnerDownloadError {
                verified_owner: false,
                message: "\n  • Sign in with the Microsoft account that owns Minecraft: Sunrise then downloads 26.3 from Mojang itself.".into(),
            })
        },
        &|_| {},
    )
    .unwrap_err();
    assert!(
        sign_in_err.contains("Sign in with the Microsoft account that owns Minecraft"),
        "{sign_in_err}"
    );
    assert!(
        !fresh_own.exists(),
        "unsigned-in ordinary player must not borrow from another launcher"
    );

    // Behind the developer route (`allow_local_without_owner = true`), every file is checked by
    // hash and copied into Sunrise's own store (never linked).
    let dev_folder = prepare_store_with(
        &pins,
        &fresh_store,
        &PrepareStoreOptions {
            os: "linux",
            reused: false,
            allow_local_without_owner: true,
            local_sources: &other_sources,
        },
        &cancel,
        &|| {
            Err(OwnerDownloadError {
                verified_owner: false,
                message: "\n  • Sign in with the Microsoft account that owns Minecraft: Sunrise then downloads 26.3 from Mojang itself.".into(),
            })
        },
        &|_| {},
    )
    .unwrap();
    assert_eq!(dev_folder, fresh_own);
    assert_eq!(
        fs::read(fresh_own.join("versions/26.3/26.3.jar")).unwrap(),
        client
    );
    assert!(
        !fs::symlink_metadata(fresh_own.join("libraries"))
            .unwrap()
            .file_type()
            .is_symlink(),
        "developer route must copy, never symlink"
    );
}

/// A reused-import preparation (`reused = true`) with a partial `data_dir()/minecraft` store
/// (e.g. an earlier cancelled download that created `assets/indexes/` without the index or sound
/// objects) refills the missing files for a signed-in owner instead of short-circuiting.
#[test]
fn reused_import_refills_partial_store_for_owner() {
    let root = tempfile::tempdir().unwrap();
    let own = root.path().join("own");
    let client = b"fake 26.3 client jar";
    let sound = b"fake sound ogg";
    let index = serde_json::json!({"objects": {
        "minecraft/sounds/a.ogg": {"hash": sources::sha1_hex(sound), "size": sound.len()},
    }})
    .to_string();
    let manifest = serde_json::json!({
        "id": "26.3",
        "assetIndex": {
            "id": "34",
            "sha1": sources::sha1_hex(index.as_bytes()),
            "url": "https://piston-meta.mojang.com/v1/packages/idx/34.json",
        },
        "downloads": {"client": {
            "sha1": sources::sha1_hex(client),
            "size": client.len(),
            "url": "https://piston-data.mojang.com/client.jar",
        }},
        "libraries": [],
    })
    .to_string();
    let manifest_sha1 = sources::sha1_hex(manifest.as_bytes());
    let pins = sources::Pins::read(&serde_json::json!({"minecraft": {
        "version": "26.3",
        "client_sha1": sources::sha1_hex(client),
        "manifest_sha1": manifest_sha1,
    }}))
    .unwrap();

    // Simulate a partial store left by an interrupted download: manifest + empty assets/indexes dir.
    fs::create_dir_all(own.join("versions/26.3")).unwrap();
    fs::write(own.join("versions/26.3/26.3.json"), &manifest).unwrap();
    fs::create_dir_all(own.join("assets/indexes")).unwrap();

    let sound_hash = sources::sha1_hex(sound);
    let fixture = std::collections::HashMap::from([
        (
            "https://piston-data.mojang.com/client.jar".to_string(),
            client.to_vec(),
        ),
        (
            "https://piston-meta.mojang.com/v1/packages/idx/34.json".to_string(),
            index.into_bytes(),
        ),
        (
            format!(
                "{}/{}/{sound_hash}",
                sunrise_launcher_addresses::MOJANG_RESOURCES,
                &sound_hash[..2]
            ),
            sound.to_vec(),
        ),
    ]);
    let fetch = |url: &str| -> Result<Vec<u8>, String> {
        fixture
            .get(url)
            .cloned()
            .ok_or_else(|| format!("unexpected url {url}"))
    };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let store = sources::Store {
        dir: &own,
        sources: &[],
    };

    let folder = prepare_store_with(
        &pins,
        &store,
        &PrepareStoreOptions {
            os: "linux",
            reused: true,
            allow_local_without_owner: false,
            local_sources: &[],
        },
        &cancel,
        &|| {
            mojang::download(&pins, &store, "linux", 2, &fetch, &cancel, &|_| {}).map_err(|e| {
                OwnerDownloadError {
                    verified_owner: true,
                    message: e,
                }
            })
        },
        &|_| {},
    )
    .unwrap();
    assert_eq!(folder, own);
    assert!(own.join("assets/indexes/34.json").is_file());
    assert_eq!(
        fs::read(
            own.join("assets/objects")
                .join(&sound_hash[..2])
                .join(&sound_hash)
        )
        .unwrap(),
        sound
    );
}

#[test]
fn native_resources_resolve_to_the_base_and_prevent_its_removal() {
    const ACTOR: &str = "SUNRISE_RESOURCE_TEST_ACTOR";
    if std::env::var_os(ACTOR).is_none() {
        let home = tempfile::tempdir().unwrap();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "backend::tests::native_resources_resolve_to_the_base_and_prevent_its_removal",
                "--nocapture"
            ])
            .env(ACTOR, "1")
            .env("SUNRISE_LAUNCHER_HOME", home.path())
            .status()
            .unwrap()
            .success());
        return;
    }
    let base = format!("resource-test-{}", std::process::id());
    let native = format!("native-resource-test-{}", std::process::id());
    let base_dir = version_dir(&base);
    let native_dir = version_dir(&native);
    fs::create_dir_all(&base_dir).unwrap();
    fs::create_dir_all(&native_dir).unwrap();
    fs::write(base_dir.join(COMPLETE), "").unwrap();
    fs::write(native_dir.join(COMPLETE), "").unwrap();
    fs::write(
        native_dir.join("native-mods.json"),
        serde_json::json!({
            "base": base, "sunrise": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "target": "client"
        })
        .to_string(),
    )
    .unwrap();
    assert_eq!(resources(&native), resources(&base));
    // Preparing a native build prepares the base whose resources it reads.
    assert_eq!(resources_owner(&native), base);
    assert_eq!(resources_owner(&base), base);
    // Two native builds on one base, and the base itself, share one preparation lock.
    let sibling = format!("{native}-sibling");
    fs::create_dir_all(version_dir(&sibling)).unwrap();
    fs::copy(
        native_dir.join("native-mods.json"),
        version_dir(&sibling).join("native-mods.json"),
    )
    .unwrap();
    {
        let _held = prepare_lock(&native).unwrap();
        assert!(prepare_lock(&base).is_err());
        assert!(prepare_lock(&sibling).is_err());
    }
    assert!(prepare_lock(&base).is_ok());
    // Preparing a native build runs the base's preparer, and says so when it can't.
    let try_prepare = || {
        let preparation = prepare_lock(&native).unwrap();
        preparer(&native, None, &|_| {}, &preparation)
            .map(|_| ())
            .unwrap_err()
    };
    assert_eq!(try_prepare(), crate::preparer::too_old(&base));
    fs::remove_file(base_dir.join(COMPLETE)).unwrap();
    let error = try_prepare();
    assert!(
        error.contains(&base) && !error.contains("too old"),
        "{error}"
    );
    fs::write(base_dir.join(COMPLETE), "").unwrap();
    fs::remove_dir_all(version_dir(&sibling)).unwrap();
    assert!(crate::native_mods::resource_dependent(&base).is_some());
    assert!(remove_version(&base).is_err());
    assert!(remove(&base).is_err());
    assert!(base_dir.join(COMPLETE).exists());
    fs::remove_dir_all(&base_dir).unwrap();
    let error = need_prepared(&native).unwrap_err();
    assert!(error.contains(&base), "{error}");
    fs::remove_dir_all(&native_dir).unwrap();
    assert!(crate::native_mods::resource_dependent(&base).is_none());
}

#[test]
fn resources_without_sounds_are_reported() {
    let dir = std::env::temp_dir().join(format!("sunrise-no-sounds-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("assets/minecraft/textures")).unwrap();
    fs::write(dir.join(RESOURCES_MANIFEST), "{}").unwrap();
    assert!(!sounds_in(&dir));
    fs::write(dir.join("assets/minecraft/sounds.json"), "{}").unwrap();
    assert!(sounds_in(&dir));
    fs::remove_dir_all(&dir).unwrap();

    assert_eq!(
        resources_summary(true, true, "26.3"),
        "Prepared for Minecraft 26.3"
    );
    let silent = resources_summary(true, false, "26.3");
    assert!(silent.contains("without its sounds") && silent.contains("Prepare again"));
    // Not prepared says only that, sounds or not.
    assert_eq!(
        resources_summary(false, false, "26.3"),
        resources_summary(false, true, "26.3")
    );
}

#[test]
fn each_mac_picks_the_build_for_its_processor() {
    assert_eq!(
        platform_assets("macos", "aarch64"),
        Some((
            "sunrise-macos-arm64.zip",
            "sunrise-launcher-macos-arm64.zip"
        ))
    );
    assert_eq!(
        platform_assets("macos", "x86_64"),
        Some(("sunrise-macos-x64.zip", "sunrise-launcher-macos-x64.zip"))
    );
    assert_eq!(
        platform_assets("windows", "x86_64"),
        Some((
            "sunrise-windows-x64.zip",
            "sunrise-launcher-windows-x64.zip"
        ))
    );
    assert_eq!(
        platform_assets("windows", "aarch64"),
        Some((
            "sunrise-windows-arm64.zip",
            "sunrise-launcher-windows-arm64.zip"
        ))
    );
    assert_eq!(platform_assets("linux", "aarch64"), None);
    assert_eq!(platform_assets("macos", "x86"), None);
    assert_eq!(
        ASSET,
        platform_assets(std::env::consts::OS, std::env::consts::ARCH).map(|(game, _)| game)
    );
    assert_eq!(
        game_fallback_for("windows", "aarch64"),
        Some("sunrise-windows-x64.zip")
    );
    assert_eq!(game_fallback_for("windows", "x86_64"), None);
    assert_eq!(game_fallback_for("macos", "aarch64"), None);
}

#[test]
fn dev_catalogue_updates_only_dev_and_orders_same_day_builds_by_publication() {
    use crate::instances::Channel::{Dev, Nightly, Stable};
    const OLDER: &str = "dev-2026-11-13-bbbbbbb";
    const NEWEST: &str = "dev-2026-11-13-aaaaaaa";
    const NIGHTLY: &str = "nightly-2026-11-12-aaaaaaa";
    const STABLE: &str = "26.3-0.1.81";
    let zip = "sunrise-windows-x64.zip";
    let files = serde_json::json!([{"name": zip, "size": 1}]);
    let list = serde_json::json!([
        {"tag_name": OLDER, "name": "Dev 2026-11-13 · bbbbbbb", "prerelease": true,
         "published_at": "2026-11-13T10:00:01Z", "assets": files},
        {"tag_name": NEWEST, "name": "Dev 2026-11-13 · aaaaaaa", "prerelease": true,
         "published_at": "2026-11-13T10:00:02Z", "assets": files},
        {"tag_name": "dev-2026-11-18-ccccccc", "name": "Dev 2026-11-18 · ccccccc", "assets": files},
        {"tag_name": "dev-2026-11-19", "name": "Dev 2026-11-19", "prerelease": true, "assets": files},
        {"tag_name": "dev-2026-11-20-ddddddd", "name": "Dev 2026-11-20 · eeeeeee", "prerelease": true, "assets": files},
        {"tag_name": "dev-2026-11-21-eeeeeee", "name": "Dev 2026-11-21 · eeeeeee", "prerelease": true, "draft": true, "assets": files},
        {"tag_name": "dev-2026-11-22-fffffff", "name": "Dev 2026-11-22 · fffffff", "prerelease": true},
        {"tag_name": "nightly", "name": "Nightly 2026-11-12 · aaaaaaa", "prerelease": true, "assets": files},
        {"tag_name": STABLE, "assets": files}
    ]);
    let list = list.as_array().unwrap();
    let releases = game_releases_with(list, &[Some(zip)]);
    assert_eq!(
        releases.iter().map(|r| r.tag.as_str()).collect::<Vec<_>>(),
        [NIGHTLY, "dev-2026-11-22-fffffff", NEWEST, OLDER, STABLE]
    );
    let dev = Dev.builds(&releases).next().unwrap();
    assert_eq!(dev.nightly.as_deref(), Some(NEWEST));
    assert_eq!(
        dev.asset.as_ref().unwrap().0,
        format!("https://github.com/sunrise-org/sunrise/releases/download/{NEWEST}/{zip}")
    );
    let installed = [OLDER.to_owned()];
    assert_eq!(
        sunrise_update(&releases, &installed, Dev).unwrap().tag,
        NEWEST
    );
    assert_eq!(
        sunrise_update(&releases, &installed, Nightly).unwrap().tag,
        NIGHTLY
    );
    assert_eq!(
        sunrise_update(&releases, &installed, Stable).unwrap().tag,
        STABLE
    );
    assert!(sunrise_update(&releases, &[NEWEST.into()], Dev).is_none());
    assert!(select_after_update(Some(OLDER), &installed, NEWEST));
    assert!(!select_after_update(Some(OLDER), &installed, NIGHTLY));
    assert!(!select_after_update(Some(STABLE), &[STABLE.into()], NEWEST));
}

#[test]
fn dev_same_day_sort_handles_mixed_offsets_and_invalid_dates_without_panicking() {
    let zip = "sunrise-windows-x64.zip";
    let files = serde_json::json!([{"name": zip, "size": 1}]);
    // Lexicographic text order ("12:00+02:00" > "11:00 bad" > "10:15Z" > "09:30-01:00")
    // disagrees with UTC instant order (10:30Z > 10:15Z > 10:00Z). Parsing each timestamp once
    // keeps valid instants newest first and invalid or missing timestamps consistently last
    // within the build day regardless of input order.
    let mut entries = vec![
        serde_json::json!({
            "tag_name": "dev-2026-11-13-1111111",
            "name": "Dev 2026-11-13 · 1111111",
            "prerelease": true,
            "published_at": "2026-11-13T12:00:00+02:00",
            "assets": files,
        }),
        serde_json::json!({
            "tag_name": "dev-2026-11-13-bad0000",
            "name": "Dev 2026-11-13 · bad0000",
            "prerelease": true,
            "published_at": "2026-11-13T11:00:00 not-rfc3339",
            "assets": files,
        }),
        serde_json::json!({
            "tag_name": "dev-2026-11-13-3333333",
            "name": "Dev 2026-11-13 · 3333333",
            "prerelease": true,
            "published_at": "2026-11-13T09:30:00-01:00",
            "assets": files,
        }),
        serde_json::json!({
            "tag_name": "dev-2026-11-13-2222222",
            "name": "Dev 2026-11-13 · 2222222",
            "prerelease": true,
            "published_at": "2026-11-13T10:15:00Z",
            "assets": files,
        }),
        serde_json::json!({
            "tag_name": "dev-2026-11-12-0000000",
            "name": "Dev 2026-11-12 · 0000000",
            "prerelease": true,
            "published_at": "2026-11-12T23:59:59Z",
            "assets": files,
        }),
    ];
    for reversed in [false, true] {
        if reversed {
            entries.reverse();
        }
        let releases = game_releases_with(&entries, &[Some(zip)]);
        assert_eq!(
            releases.iter().map(|r| r.tag.as_str()).collect::<Vec<_>>(),
            [
                "dev-2026-11-13-3333333",
                "dev-2026-11-13-2222222",
                "dev-2026-11-13-1111111",
                "dev-2026-11-13-bad0000",
                "dev-2026-11-12-0000000",
            ]
        );
    }
}

/// The ARM64 Windows launcher installs the ARM64 build where a release has one, and the x64
/// build (under emulation) from the releases that have only that, checked against that file's
/// signed checksum.
#[test]
fn an_arm64_windows_launcher_falls_back_to_the_x64_game() {
    let (arm, x64) = ("sunrise-windows-arm64.zip", "sunrise-windows-x64.zip");
    let asset = |name: &str, url: &str| serde_json::json!({"name": name, "browser_download_url": url, "size": 1});
    let list = serde_json::json!([
        {"tag_name": "26.3-0.1.5", "assets": [
            asset(x64, "https://example.test/v2026.11.05/sunrise-windows-x64.zip"),
            asset(arm, "https://example.test/v2026.11.05/sunrise-windows-arm64.zip")]},
        {"tag_name": "26.3-0.1.4", "assets": [
            asset(x64, "https://example.test/v2026.11.04/sunrise-windows-x64.zip")]},
        {"tag_name": "26.3-0.1.3", "assets": []},
    ]);
    let list = list.as_array().unwrap();
    let releases = game_releases_with(list, &[Some(arm), Some(x64)]);
    let urls: Vec<_> = releases
        .iter()
        .map(|r| {
            r.asset
                .as_ref()
                .map(|(url, _)| url.rsplit('/').next().unwrap())
        })
        .collect();
    assert_eq!(urls, [Some(arm), Some(x64), None]);
    let installs: Vec<_> = releases
        .iter()
        .map(|r| asset_installed(r, Some(arm), Some(x64)))
        .collect();
    assert_eq!(installs, [Some(arm), Some(x64), None]);

    // An x64 launcher, on any Windows, never looks at the ARM64 build.
    let x64_only = game_releases_with(list, &[Some(x64)]);
    assert!(x64_only[..2]
        .iter()
        .all(|r| r.asset.as_ref().unwrap().0.ends_with(x64)));
}

#[test]
fn a_game_architecture_choice_is_independent_of_the_launcher_process() {
    let (arm, x64) = ("sunrise-windows-arm64.zip", "sunrise-windows-x64.zip");
    // GetNativeSystemInfo reports ARM64 even when std::env::consts::ARCH is x86_64.
    assert!(supports_game_architecture_choice(
        "windows",
        Some("aarch64")
    ));
    assert!(!supports_game_architecture_choice(
        "windows",
        Some("x86_64")
    ));
    assert!(!supports_game_architecture_choice("linux", Some("aarch64")));

    // An x64 launcher keeps the legacy x64 default, but can opt into ARM64.
    assert_eq!(
        game_assets_for("windows", Some("aarch64"), "x86_64", GameArchitecture::Auto),
        Some((x64, None))
    );
    assert_eq!(
        game_assets_for(
            "windows",
            Some("aarch64"),
            "x86_64",
            GameArchitecture::Arm64
        ),
        Some((arm, Some(x64)))
    );

    // The ARM64 launcher defaults to ARM64 and can choose x64 for compatibility.
    assert_eq!(
        game_assets_for(
            "windows",
            Some("aarch64"),
            "aarch64",
            GameArchitecture::Auto
        ),
        Some((arm, Some(x64)))
    );
    assert_eq!(
        game_assets_for("windows", Some("aarch64"), "aarch64", GameArchitecture::X64),
        Some((x64, None))
    );

    // Native x64 Windows and the other platforms keep their old process-based selection.
    assert_eq!(
        game_assets_for("windows", Some("x86_64"), "x86_64", GameArchitecture::Arm64),
        Some((x64, None))
    );
    assert_eq!(
        game_assets_for("linux", None, "x86_64", GameArchitecture::Arm64),
        Some(("sunrise-linux-x86_64.tar.gz", None))
    );
}

#[cfg(windows)]
#[test]
fn windows_native_architecture_detection_sees_the_host_under_emulation() {
    let native = native_windows_architecture().expect("supported Windows machine");
    assert!(matches!(native, "x86_64" | "aarch64"));
    if std::env::var_os("SUNRISE_TEST_EXPECT_NATIVE_ARM64").is_some() {
        assert_eq!(std::env::consts::ARCH, "x86_64");
        assert_eq!(native, "aarch64");
    }
    if native == "aarch64" {
        assert!(supports_game_architecture_choice("windows", Some(native)));
        let (asset, fallback) = game_assets_for(
            "windows",
            Some(native),
            std::env::consts::ARCH,
            GameArchitecture::Auto,
        )
        .unwrap();
        if std::env::consts::ARCH == "x86_64" {
            assert_eq!((asset, fallback), ("sunrise-windows-x64.zip", None));
        } else {
            assert_eq!(
                (asset, fallback),
                ("sunrise-windows-arm64.zip", Some("sunrise-windows-x64.zip"))
            );
        }
    }
}

#[test]
fn selected_architecture_uses_available_assets_and_only_the_safe_fallback() {
    let arm = "sunrise-windows-arm64.zip";
    let x64 = "sunrise-windows-x64.zip";
    let asset = |name: &str| serde_json::json!({"name": name, "browser_download_url": format!("https://example.test/{name}"), "size": 1});
    let list = serde_json::json!([
        {"tag_name": "26.3-0.1.5", "assets": [asset(x64), asset(arm)]},
        {"tag_name": "26.3-0.1.4", "assets": [asset(x64)]},
        {"tag_name": "26.3-0.1.3", "assets": []}
    ]);
    let list = list.as_array().unwrap();

    let arm_choice = game_releases_with(list, &[Some(arm), Some(x64)]);
    assert_eq!(
        arm_choice
            .iter()
            .map(|release| release
                .asset
                .as_ref()
                .and_then(|(url, _)| url.rsplit('/').next()))
            .collect::<Vec<_>>(),
        [Some(arm), Some(x64), None]
    );
    assert_eq!(
        asset_installed(&arm_choice[0], Some(arm), Some(x64)),
        Some(arm)
    );
    assert_eq!(
        asset_installed(&arm_choice[1], Some(arm), Some(x64)),
        Some(x64)
    );
    assert_eq!(asset_installed(&arm_choice[2], Some(arm), Some(x64)), None);

    let x64_choice = game_releases_with(list, &[Some(x64)]);
    assert!(x64_choice.iter().take(2).all(|release| release
        .asset
        .as_ref()
        .unwrap()
        .0
        .ends_with(x64)));
    assert_eq!(
        asset_installed(&arm_choice[0], Some(x64), None),
        None,
        "a stale ARM64 release selection must not install under an x64 choice"
    );
}

#[test]
fn sidebar_starts_collapsed_and_remembers_both_positions() {
    let mut settings = Settings::parse("{}");
    assert!(!settings.sidebar_expanded);
    for expanded in [true, false] {
        settings.sidebar_expanded = expanded;
        let saved = serde_json::to_string(&settings).unwrap();
        assert_eq!(Settings::parse(&saved).sidebar_expanded, expanded);
    }
}

#[test]
fn selected_game_build_and_safe_fallback_satisfy_the_saved_choice() {
    let fallback = InstalledGameArchitecture {
        requested: GameArchitecture::Arm64,
        binary: GameArchitecture::X64,
    };
    assert!(installed_architecture_matches(
        GameArchitecture::Arm64,
        fallback
    ));
    assert!(installed_architecture_matches(
        GameArchitecture::X64,
        fallback
    ));
    assert!(!installed_architecture_matches(
        GameArchitecture::Arm64,
        InstalledGameArchitecture {
            requested: GameArchitecture::X64,
            binary: GameArchitecture::X64,
        }
    ));
}

#[test]
fn native_mod_builds_report_when_the_selected_game_has_no_matching_compiler() {
    assert_eq!(
        native_mod_architecture_message(GameArchitecture::X64, GameArchitecture::Arm64),
        Some(
            "Native mods cannot build for the selected ARM64 game yet. The available compiler builds x64 only."
        )
    );
    assert_eq!(
        native_mod_architecture_message(GameArchitecture::Arm64, GameArchitecture::X64),
        Some(
            "Native mods cannot build for the selected x64 game yet. This ARM64 launcher has no matching compiler."
        )
    );
    assert_eq!(
        native_mod_architecture_message(GameArchitecture::Arm64, GameArchitecture::Arm64),
        None
    );
}

#[test]
fn architecture_reinstall_repairs_only_a_still_offered_exact_game() {
    use GameArchitecture::{Arm64, X64};
    let arm = "sunrise-windows-arm64.zip";
    let x64 = "sunrise-windows-x64.zip";
    let current = "26.3-0.1.0";
    let next = "26.3-0.1.1";
    let release = |tag: &str, file: &str| Release {
        tag: tag.into(),
        name: String::new(),
        body: String::new(),
        date: String::new(),
        time: String::new(),
        published_at: String::new(),
        asset: Some((format!("https://example.invalid/{file}"), 1)),
        prerelease: false,
        nightly: None,
    };
    let have = |requested, binary| Some(InstalledGameArchitecture { requested, binary });
    let exact = [
        release(next, arm),
        release(current, arm),
        release("v2026.10.13", arm),
    ];
    let repair =
        |tag, installed| architecture_reinstall_for(&exact, tag, Arm64, "x86_64", installed);
    assert_eq!(
        repair(current, have(X64, X64)).map(|r| r.tag.as_str()),
        Some(current)
    );
    assert!(repair(current, have(Arm64, Arm64)).is_none());
    assert!(repair("26.3-0.0.9", have(X64, X64)).is_none());
    assert!(repair("v2026.10.13", have(X64, X64)).is_none());
    assert!(
        repair(current, have(Arm64, X64)).is_some(),
        "promote when the real ARM64 build exists"
    );
    let fallback = [release(current, x64)];
    let repair_fallback =
        |installed| architecture_reinstall_for(&fallback, current, Arm64, "x86_64", installed);
    assert!(
        repair_fallback(have(X64, X64)).is_some(),
        "record the requested fallback"
    );
    assert!(
        repair_fallback(have(Arm64, X64)).is_none(),
        "do not re-offer an accepted fallback"
    );
    assert!(
        architecture_reinstall_for(&exact, current, X64, "x86_64", have(Arm64, Arm64)).is_none(),
        "a stale ARM64 catalogue cannot repair to x64"
    );
}

#[test]
fn architecture_launch_guidance_never_offers_an_unavailable_repair() {
    use GameArchitecture::{Arm64, X64};
    let stable = "26.3-0.1.0";
    let nightly = "nightly-2026-11-20-aaaaaaa";
    let release = |tag: &str, nightly: Option<String>| Release {
        tag: tag.into(),
        name: String::new(),
        body: String::new(),
        date: String::new(),
        time: String::new(),
        published_at: String::new(),
        asset: Some((
            "https://example.invalid/sunrise-windows-arm64.zip".into(),
            1,
        )),
        prerelease: false,
        nightly,
    };
    let catalogue = [
        release(stable, None),
        release(nightly, Some("nightly".into())),
    ];
    let installed = Some(InstalledGameArchitecture {
        requested: X64,
        binary: X64,
    });
    for tag in ["v2026.10.13", "nightly-2026-11-19-bbbbbbb"] {
        let help = architecture_mismatch_for(tag, &catalogue, Arm64, "x86_64", installed).unwrap();
        assert!(!help.contains("version picker"), "{help}");
        assert!(help.contains("Switch back to the installed architecture or Automatic, or choose another available build."), "{help}");
    }
    for tag in [stable, nightly] {
        assert!(
            architecture_mismatch_for(tag, &catalogue, Arm64, "x86_64", installed)
                .unwrap()
                .contains("Reinstall it from the version picker beside Play")
        );
        assert!(
            !architecture_mismatch_for(tag, &[], Arm64, "x86_64", installed)
                .unwrap()
                .contains("version picker")
        );
    }
    assert!(architecture_mismatch_for(stable, &catalogue, X64, "x86_64", installed).is_none());
}

#[test]
fn windows_pe_machine_types_identify_the_installed_game_architecture() {
    let pe = |machine: u16| {
        let mut bytes = vec![0; 0x86];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\x00\x00");
        bytes[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        bytes
    };
    assert_eq!(pe_architecture(&pe(0x8664)), Some(GameArchitecture::X64));
    assert_eq!(pe_architecture(&pe(0xaa64)), Some(GameArchitecture::Arm64));
    assert_eq!(pe_architecture(b"not a PE executable"), None);
}

/// launcher.yml publishes a release even when its ARM64 build failed. An ARM64 launcher then
/// stays on its version until a release has its own zip: it never takes the x64 one.
#[test]
fn an_arm64_launcher_skips_a_release_without_its_zip() {
    let (arm, x64) = (
        "sunrise-launcher-windows-arm64.zip",
        "sunrise-launcher-windows-x64.zip",
    );
    let asset = |name: &str| serde_json::json!({"name": name, "browser_download_url": format!("https://example.test/{name}"), "size": 1});
    let list = serde_json::json!([
        {"tag_name": "v1.3.0", "assets": [asset(x64)]},
        {"tag_name": "v1.2.0", "assets": [asset(x64), asset(arm)]},
    ]);
    let list = list.as_array().unwrap();
    let releases = parse_launcher_releases(list, Some(arm));
    assert!(launcher_update_from(&releases, "v1.2.0").is_none());
    let update = launcher_update_from(&releases, "v1.1.0").unwrap();
    assert_eq!(update.tag, "v1.2.0");
    assert!(update.asset.unwrap().0.ends_with(arm));
}

#[test]
fn the_game_plays_under_its_own_audio_stream_name() {
    if std::env::var_os("PIPEWIRE_PROPS").is_some() {
        return;
    }
    let cmd = client_command("v2026.10.13");
    let props = cmd
        .get_envs()
        .find(|(k, _)| *k == "PIPEWIRE_PROPS")
        .and_then(|(_, v)| v);
    assert!(props.is_some_and(|v| {
        v.to_string_lossy()
            .contains("application.name = \"Sunrise\"")
    }));
}

#[test]
fn a_crash_after_the_vulkan_adapter_line_and_before_the_first_frame_is_a_start_crash() {
    let start = [
        "Sunrise: resources loaded in 11.3s; 35723 block states, 1658 items, 67 biomes",
        "Sunrise renderer: Intel(R) HD Graphics 530 (Vulkan, IntegratedGpu)",
    ];
    assert!(vulkan_start_crashed(start));
    // A step line after the adapter keeps it a start crash.
    let step = [start[1], "Sunrise renderer: creating the pipelines"];
    assert!(vulkan_start_crashed(step));
    // A crash after the first frame is the game's, not the renderer start's.
    let later = [start[1], "Sunrise: first frame after 812 ms"];
    assert!(!vulkan_start_crashed(later));
    // Already on DirectX 12, or no renderer yet: nothing to switch.
    let dx12 = ["Sunrise renderer: Intel(R) HD Graphics 530 (Dx12, IntegratedGpu)"];
    assert!(!vulkan_start_crashed(dx12));
    assert!(!vulkan_start_crashed(&start[..1]));
    // Vulkan failed to open and DirectX 12 took over: its line is the last adapter.
    assert!(!vulkan_start_crashed([start[1], dx12[0]]));
}

#[test]
fn a_renderer_start_crash_switches_auto_to_dx12_once() {
    let home = std::env::temp_dir().join(format!("sunrise-dx12-switch-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    // No settings file yet: one with the API alone.
    assert_eq!(switch_from_auto(&home, "dx12"), Ok(true));
    assert_eq!(
        fs::read_to_string(home.join(GRAPHICS_FILE)).unwrap(),
        "graphicsApi:dx12\n"
    );
    // Already DirectX 12, or the player's own Vulkan: left alone.
    assert_eq!(switch_from_auto(&home, "dx12"), Ok(false));
    fs::write(home.join(GRAPHICS_FILE), "sky:false\ngraphicsApi:vulkan\n").unwrap();
    assert_eq!(switch_from_auto(&home, "dx12"), Ok(false));
    // Auto becomes DirectX 12 and the other settings stay.
    fs::write(
        home.join(GRAPHICS_FILE),
        "sky:false\ngraphicsApi:auto\nfog:true\n",
    )
    .unwrap();
    assert_eq!(switch_from_auto(&home, "dx12"), Ok(true));
    assert_eq!(
        fs::read_to_string(home.join(GRAPHICS_FILE)).unwrap(),
        "sky:false\ngraphicsApi:dx12\nfog:true\n"
    );
    let _ = fs::remove_dir_all(&home);
}

/// #4293: an Ivy Bridge on Linux whose Vulkan driver (Mesa's hasvk) crashes at the renderer's
/// start plays on OpenGL next time; Windows keeps DirectX 12 (#746), macOS has nothing else.
#[test]
fn a_vulkan_start_crash_on_linux_switches_auto_to_opengl() {
    assert_eq!(vulkan_crash_fallback("windows").unwrap().0, "dx12");
    assert_eq!(vulkan_crash_fallback("macos"), None);
    let (api, _) = vulkan_crash_fallback("linux").unwrap();
    let home = std::env::temp_dir().join(format!("sunrise-gl-switch-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join(GRAPHICS_FILE), "graphicsApi:auto\nvsync:true\n").unwrap();
    assert_eq!(switch_from_auto(&home, api), Ok(true));
    assert_eq!(
        fs::read_to_string(home.join(GRAPHICS_FILE)).unwrap(),
        "graphicsApi:opengl\nvsync:true\n"
    );
    // The next crash (OpenGL had no GPU either and Vulkan ran again) leaves it: no loop.
    assert_eq!(switch_from_auto(&home, api), Ok(false));
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn extra_game_arguments_split_like_a_shell() {
    assert!(split_args("").is_empty());
    assert!(split_args("   ").is_empty());
    assert_eq!(split_args("--server-thread"), ["--server-thread"]);
    assert_eq!(
        split_args("--render-distance 32 --server-thread"),
        ["--render-distance", "32", "--server-thread"]
    );
    // Quotes keep a word with spaces together; either kind works.
    assert_eq!(
        split_args("--name 'Two Words' --world \"My World\""),
        ["--name", "Two Words", "--world", "My World"]
    );
    // An empty quoted word is still a word.
    assert_eq!(split_args("--name \"\""), ["--name", ""]);
    // A backslash escapes a space outside quotes, and " and \ inside double quotes.
    assert_eq!(split_args("--name My\\ World"), ["--name", "My World"]);
    assert_eq!(split_args("a\\tb c"), ["atb", "c"]);
    assert_eq!(split_args("\"a\\\"b\" 'c\\d'"), ["a\"b", "c\\d"]);
}

#[test]
fn the_game_arguments_setting_round_trips_and_reaches_the_command() {
    let settings = Settings::parse(r#"{"game_args":"--server-thread"}"#);
    assert_eq!(settings.game_args, "--server-thread");
    // Unset in an older settings.json: no arguments, and nothing is written for them.
    let old = Settings::parse(r#"{"installed":true}"#);
    assert_eq!(old.game_args, "");
    assert!(!serde_json::to_string(&old).unwrap().contains("game_args"));

    let mut cmd = Command::new("true");
    cmd.args(split_args("--server-thread --render-distance 40"));
    let args: Vec<_> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(args, ["--server-thread", "--render-distance", "40"]);
}

#[test]
fn launcher_tags_keep_their_original_grammar() {
    assert_eq!(launcher_version("v1.2.0"), Some("1.2.0"));
    assert_eq!(launcher_version("vnext"), None);
    assert_eq!(launcher_version("launcher@1.0.0"), None);
    let file = |name: &str| serde_json::json!([{"name": name, "size": 1}]);
    let launcher_list = serde_json::json!([
        {"tag_name": "v1.1.0", "assets": file("sunrise-launcher-linux-x86_64.tar.gz")},
        {"tag_name": "v1.0.9", "draft": true, "assets": []},
    ]);
    let launcher_list = launcher_list.as_array().unwrap();
    let launcher: Vec<_> = parse_launcher_releases(launcher_list, LAUNCHER_ASSET)
        .into_iter()
        .map(|r| r.tag)
        .collect();
    assert_eq!(launcher, ["v1.1.0"]);
}

/// Rolling a release back (`gh release edit <tag> --draft`, docs/release-flow.md) makes the
/// previous one the newest again: it is what `install latest` and the update check offer.
#[test]
fn withdrawn_release_is_never_offered() {
    let asset = ASSET.unwrap_or("sunrise-linux-x86_64.tar.gz");
    let list = serde_json::json!([
        {"tag_name": "26.3-0.1.2", "draft": true, "assets": [{"name": asset, "browser_download_url": "bad", "size": 1}]},
        {"tag_name": "26.3-0.1.1", "assets": [{"name": asset, "browser_download_url": "good", "size": 1}]},
        {"tag_name": "26.3-0.1.0", "assets": [{"name": asset, "browser_download_url": "old", "size": 1}]},
    ]);
    let list = list.as_array().unwrap();
    let releases = parse_releases(list, &[Some(asset)], is_game_tag);
    assert!(releases.iter().all(|r| r.tag != "26.3-0.1.2"));
    assert_eq!(
        latest(&releases).map(|r| r.tag.as_str()),
        Some("26.3-0.1.1")
    );
    let offered = |installed: &[&str]| {
        let installed: Vec<String> = installed.iter().map(|t| t.to_string()).collect();
        sunrise_update(&releases, &installed, crate::instances::Channel::Stable)
            .map(|r| r.tag.clone())
    };
    assert_eq!(offered(&["26.3-0.1.0"]), Some("26.3-0.1.1".into()));
    assert_eq!(offered(&[]), Some("26.3-0.1.1".into()));
    // Who installed the withdrawn one is not told to "update" to an older version.
    assert_eq!(offered(&["26.3-0.1.2", "26.3-0.1.0"]), None);
}

#[test]
fn worlds_are_rows_unless_tiles_are_picked() {
    assert_eq!(Settings::default().world_layout, WorldLayout::Rows);
    assert_eq!(Settings::parse("{}").world_layout, WorldLayout::Rows);
    // A file from before the choice existed keeps everything else it says.
    let old = Settings::parse(r#"{"while_playing": "hide", "game_args": "--x"}"#);
    assert_eq!(
        (old.world_layout, old.while_playing, old.game_args.as_str()),
        (WorldLayout::Rows, WhilePlaying::Hide, "--x")
    );
    let tiles = Settings::parse(r#"{"world_layout": "tiles", "game_args": "--x"}"#);
    assert_eq!(tiles.world_layout, WorldLayout::Tiles);
    let saved = serde_json::to_string(&tiles).unwrap();
    assert!(saved.contains(r#""world_layout":"tiles""#), "{saved}");
    assert_eq!(Settings::parse(&saved).world_layout, WorldLayout::Tiles);
    // A layout this launcher doesn't know, or one that isn't a name, loses only itself.
    for text in [
        r#"{"world_layout": "carousel", "game_args": "--x"}"#,
        r#"{"world_layout": 3, "game_args": "--x"}"#,
    ] {
        let future = Settings::parse(text);
        assert_eq!(
            (future.world_layout, future.game_args.as_str()),
            (WorldLayout::Rows, "--x"),
            "{text}"
        );
    }
    assert_eq!(WorldLayout::parse(" Tiles "), Some(WorldLayout::Tiles));
    assert_eq!(WorldLayout::parse("grid"), None);
}

#[test]
fn the_window_stays_while_playing_unless_picked() {
    assert_eq!(Settings::parse("{}").while_playing, WhilePlaying::Stay);
    let hide = Settings::parse(r#"{"while_playing": "hide", "game_args": "--x"}"#);
    assert_eq!(hide.while_playing, WhilePlaying::Hide);
    let future = Settings::parse(r#"{"while_playing": "fade", "game_args": "--x"}"#);
    assert_eq!(
        (future.while_playing, future.game_args.as_str()),
        (WhilePlaying::Stay, "--x")
    );
    assert_eq!(
        WhilePlaying::parse("Minimise"),
        Some(WhilePlaying::Minimize)
    );
    assert_eq!(WhilePlaying::parse("off"), Some(WhilePlaying::Stay));
    assert_eq!(WhilePlaying::parse("close"), None);
    for w in WhilePlaying::ALL {
        let text = serde_json::to_string(&w).unwrap();
        assert_eq!(text, format!("\"{}\"", w.name()));
    }
    let here = WhilePlaying::Hide.here();
    assert_eq!(here == WhilePlaying::Hide, cfg!(target_os = "macos"));
}

#[test]
fn the_retired_auto_install_setting_is_ignored_on_a_normal_settings_round_trip() {
    let settings = Settings::parse(
        r#"{"install_new_versions":true,"selected":"26.3-0.1.0","skip_opening":true}"#,
    );
    let saved = serde_json::to_value(settings).unwrap();
    assert_eq!(saved["selected"], "26.3-0.1.0");
    assert_eq!(saved["skip_opening"], true);
    assert!(saved.get("install_new_versions").is_none());
}

#[test]
fn the_game_architecture_defaults_to_auto_and_persists_manual_choices() {
    assert_eq!(
        Settings::parse("{}").game_architecture,
        GameArchitecture::Auto
    );
    let arm = Settings::parse(r#"{"game_architecture":"arm64","game_args":"--x"}"#);
    assert_eq!(
        (arm.game_architecture, arm.game_args.as_str()),
        (GameArchitecture::Arm64, "--x")
    );
    let saved = serde_json::to_value(&arm).unwrap();
    assert_eq!(saved["game_architecture"], "arm64");

    let unknown = Settings::parse(r#"{"game_architecture":"future","game_args":"--x"}"#);
    assert_eq!(
        (unknown.game_architecture, unknown.game_args.as_str()),
        (GameArchitecture::Auto, "--x")
    );
    assert_eq!(GameArchitecture::parse("X64"), Some(GameArchitecture::X64));
    assert_eq!(GameArchitecture::parse("other"), None);
}

#[test]
fn the_language_follows_the_system_unless_picked() {
    use crate::i18n::Language;
    assert_eq!(Settings::parse("{}").language, None);
    let fr = Settings::parse(r#"{"language": "fr"}"#);
    assert_eq!(fr.language(), Language::French);
    let future = Settings::parse(r#"{"language": "tlh", "game_args": "--x"}"#);
    assert_eq!((future.language, future.game_args.as_str()), (None, "--x"));
    let saved = serde_json::to_value(Settings::default()).unwrap();
    assert!(saved.get("language").is_none(), "unset is left out");
}

#[test]
fn glass_and_blur_default_to_the_mockups_and_stay_in_range() {
    let unset = Settings::parse("{}");
    assert_eq!((unset.glass_tint(), unset.glass_blur()), (0.06, 24.));
    let set = Settings::parse(r#"{"glass": 12, "blur": 8.5}"#);
    assert_eq!((set.glass_tint(), set.glass_blur()), (0.12, 8.5));
    let wild = Settings::parse(r#"{"glass": 90, "blur": -3}"#);
    assert_eq!((wild.glass_tint(), wild.glass_blur()), (0.16, 0.));
    // Not a number: the default, and the other settings survive.
    let wrong = Settings::parse(r#"{"glass": "frosty", "blur": null, "game_args": "--x"}"#);
    assert_eq!(
        (
            wrong.glass_tint(),
            wrong.glass_blur(),
            wrong.game_args.as_str()
        ),
        (0.06, 24., "--x")
    );
}

#[test]
fn reduced_graphics_turns_off_blur_without_forgetting_its_value() {
    let mut s = Settings::parse(r#"{"blur":8.5,"performance":true}"#);
    assert_eq!(s.glass_blur(), 0.);
    let saved = serde_json::to_string(&s).unwrap();
    s = Settings::parse(&saved);
    assert_eq!(s.glass_blur(), 0.);
    s.reduced_graphics = Some(false);
    assert_eq!(s.glass_blur(), 8.5);
}

#[test]
fn a_malformed_sidebar_choice_keeps_every_other_setting_through_a_save() {
    for bad in ["null", "\"yes\"", "1", "[true]"] {
        let saved = format!(
            r#"{{"sidebar_expanded": {bad}, "instance": "skyblock", "ping_off": true,
                    "selected": "nightly-2026-11-13-aaaaaaa", "game_args": "--x"}}"#
        );
        // What the sidebar's toggle does: load, flip, save, and what the next start reads.
        let mut loaded = Settings::parse(&saved);
        assert!(!loaded.sidebar_expanded, "{bad} starts collapsed");
        loaded.sidebar_expanded = !loaded.sidebar_expanded;
        let again = Settings::parse(&serde_json::to_string(&loaded).unwrap());
        assert!(again.sidebar_expanded);
        assert_eq!(again.instance.as_deref(), Some("skyblock"), "{bad}");
        assert!(again.ping_off, "{bad} must not turn the ping back on");
        assert_eq!(
            again.selected.as_deref(),
            Some("nightly-2026-11-13-aaaaaaa")
        );
        assert_eq!(again.game_args, "--x");
    }
    assert!(Settings::parse(r#"{"sidebar_expanded": true}"#).sidebar_expanded);
}

#[test]
fn a_new_install_is_dawn_and_an_unknown_accent_keeps_the_rest() {
    use crate::themes::Accent;
    let fresh = Settings::parse("{}");
    assert_eq!(crate::themes::accent_key(fresh.theme.as_deref()), "dawn");
    assert_eq!(
        fresh.accent,
        crate::themes::starting_accent(Some("dawn"), &Default::default())
    );
    assert_ne!(fresh.accent, Accent::VIOLET);
    assert_eq!(
        Settings::parse(r#"{"theme": "sunrise"}"#).accent,
        Accent::VIOLET
    );
    let teal =
        Settings::parse(r#"{"accent": "teal", "background": "background-0123456789ab.png"}"#);
    assert_eq!(
        (teal.accent, teal.background.as_deref()),
        (Accent::TEAL, Some("background-0123456789ab.png"))
    );
    let future = Settings::parse(r#"{"accent": "plaid", "game_args": "--x"}"#);
    assert_eq!(
        (future.accent, future.game_args.as_str()),
        (Accent::VIOLET, "--x")
    );
}

#[test]
fn an_old_appearance_moves_to_a_theme_and_an_unknown_one_keeps_the_rest() {
    assert_eq!(Settings::parse("{}").appearance, Appearance::Dark);
    assert_eq!(Settings::parse("{}").theme, None);
    // Light, alone or under a theme, opens on Taiga and keeps the rest.
    for text in [
        r#"{"appearance": "light", "game_args": "--x"}"#,
        r#"{"appearance": "light", "theme": "deep_ocean", "game_args": "--x"}"#,
    ] {
        let light = Settings::parse(text);
        assert_eq!(
            (
                light.appearance,
                light.theme.as_deref(),
                light.game_args.as_str()
            ),
            (Appearance::Dark, Some("taiga"), "--x"),
            "{text}"
        );
    }
    // Dark and the system's keep the saved theme's look.
    for appearance in ["dark", "system"] {
        let text = format!(r#"{{"appearance": "{appearance}", "theme": "nord"}}"#);
        let kept = Settings::parse(&text);
        assert_eq!(
            (kept.appearance, kept.theme.as_deref()),
            (Appearance::Dark, Some("nord"))
        );
    }
    let future = Settings::parse(r#"{"appearance": "sepia", "theme": "nord", "game_args": "--x"}"#);
    assert_eq!(
        (
            future.appearance,
            future.theme.as_deref(),
            future.game_args.as_str()
        ),
        (Appearance::Dark, Some("nord"), "--x")
    );
    // Saved back as dark, so a migrated light stays on Taiga only until the player picks.
    let text = serde_json::to_string(&Settings::parse(r#"{"appearance": "light"}"#)).unwrap();
    assert!(text.contains(r#""appearance":"dark""#), "{text}");
}

#[test]
fn the_old_update_channel_settings_are_dropped_and_the_rest_kept() {
    for old in [
        r#"{"update_channel": "alpha", "game_args": "--x", "selected": "v0.2.0"}"#,
        r#"{"update_channel": "stable", "game_args": "--x", "selected": "v0.2.0"}"#,
        r#"{"beta_builds": true, "game_args": "--x", "selected": "v0.2.0"}"#,
        r#"{"beta_builds": false, "update_channel": "nightly", "game_args": "--x", "selected": "v0.2.0"}"#,
    ] {
        let settings = Settings::parse(old);
        assert_eq!(settings.game_args, "--x", "{old}");
        assert_eq!(settings.selected.as_deref(), Some("v0.2.0"), "{old}");
        let saved = serde_json::to_value(&settings).unwrap();
        assert!(saved.get("update_channel").is_none(), "{old}");
        assert!(saved.get("beta_builds").is_none(), "{old}");
    }
}

#[test]
fn the_version_is_the_one_the_release_workflow_injects() {
    assert_eq!(version_or(Some("0.4.2")), "0.4.2");
    assert_eq!(version_or(Some("")), env!("CARGO_PKG_VERSION"));
    assert_eq!(version_or(None), env!("CARGO_PKG_VERSION"));
    assert_eq!(
        VERSION,
        option_env!("SUNRISE_LAUNCHER_VERSION")
            .filter(|v| !v.is_empty())
            .unwrap_or(env!("CARGO_PKG_VERSION"))
    );
    // Self-update compares releases with that version: this launcher's own release is no update.
    let r = |tag: String| Release {
        tag,
        name: String::new(),
        body: String::new(),
        date: String::new(),
        time: String::new(),
        published_at: String::new(),
        asset: Some(("u".into(), 1)),
        prerelease: false,
        nightly: None,
    };
    assert!(launcher_update(&[r(own_tag())]).is_none());
    assert_eq!(launcher_version(&own_tag()), Some(VERSION));
    assert_eq!(
        parse_version_output(&format!("{}{VERSION}\n", crate::identity::VERSION_PREFIX)),
        Some(VERSION)
    );
}

#[test]
fn a_launcher_knows_which_line_it_was_released_on() {
    // release.yml gives the tag; a build without one is its version as a tag of this repository.
    assert_eq!(own_tag_of(Some("v1.1.0"), Some("1.1.0")), "v1.1.0");
    assert_eq!(own_tag_of(None, Some("1.1.0")), "v1.1.0");
    assert_eq!(own_tag_of(Some(""), Some("1.1.0")), "v1.1.0");
    assert_eq!(
        own_tag_of(None, None),
        format!("v{}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn a_launcher_from_the_game_repository_updates_to_this_repositorys_releases() {
    let r = |tag: &str| Release {
        tag: tag.into(),
        name: String::new(),
        body: String::new(),
        date: String::new(),
        time: String::new(),
        published_at: String::new(),
        asset: Some(("u".into(), 1)),
        prerelease: false,
        nightly: None,
    };
    let mut list = vec![r("v1.1.0"), r("v1.0.1"), r("v1.2.0")];
    list.sort_by(|a, b| compare_versions(&b.tag, &a.tag));
    let tags: Vec<_> = list.iter().map(|r| r.tag.as_str()).collect();
    assert_eq!(tags, ["v1.2.0", "v1.1.0", "v1.0.1"]);
    let offered = |own: &str| launcher_update_from(&list, own).map(|r| r.tag);
    // 1.0.0 was released from the game repository as launcher@1.0.0.
    assert_eq!(offered("launcher@1.0.0"), Some("v1.2.0".into()));
    assert_eq!(offered("v1.1.0"), Some("v1.2.0".into()));
    assert_eq!(offered("v1.2.0"), None);
    assert_eq!(offered("v1.3.0-rc.1"), None);
    // Only this repository's own tags are launcher releases here.
    assert_eq!(launcher_version("v1.2.0"), Some("1.2.0"));
    assert_eq!(launcher_version("launcher@1.0.0"), None);
    assert_eq!(launcher_version("nightly"), None);
}

/// A launcher in an AppImage updates with the AppImage's own file: the `.tar.gz` binary would
/// be written inside the AppImage's read-only mount.
#[test]
fn an_appimage_updates_with_the_appimage() {
    assert_eq!(launcher_asset_running_from(false), LAUNCHER_ASSET);
    assert_eq!(launcher_asset_running_from(true), LAUNCHER_APPIMAGE_ASSET);
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    assert_eq!(
        launcher_asset_running_from(true),
        Some("sunrise-launcher-linux-x86_64.AppImage")
    );
}

/// A program opened inside a `.zip` runs from the temporary folder, under a name that depends on
/// the tool, and is deleted later: an update written there is lost, so such a copy is told to
/// download the launcher again (#9287). A folder of the same look elsewhere is a real install.
#[test]
fn a_launcher_run_from_the_temp_folder_is_a_temporary_copy() {
    let temp = Path::new("/tmp/t");
    let inside = |dir: &str| in_temp_folder(Path::new(dir), temp);
    assert!(inside("/tmp/t"));
    assert!(inside(
        "/tmp/t/Temp12_sunrise-launcher-windows-x64.zip/inner"
    ));
    assert!(inside("/tmp/t/7zO8A1B2C3D"));
    assert!(!inside("/tmp/tools"));
    assert!(!inside("/home/me/Temp1_sunrise.zip"));
}

/// The same folder spelled two ways, as %TEMP% and the launcher's own path can be on an account
/// with a short name: `temp_dir()` as the system gives it against its resolved form, and a
/// spelling through `..`, which a literal comparison misses on every system.
#[test]
fn the_temp_folder_is_recognized_however_it_is_spelled() {
    let root = scratch("temp-spelling");
    let temp = root.join("temp");
    let app = temp.join("Temp1_sunrise.zip");
    let outside = root.join("installed");
    fs::create_dir_all(&app).unwrap();
    fs::create_dir_all(&outside).unwrap();

    assert!(in_temp_folder(
        &app,
        &root.join("installed").join("..").join("temp")
    ));
    assert!(!in_temp_folder(&outside, &temp));

    let system = std::env::temp_dir();
    let long = system.canonicalize().unwrap();
    let copy = long.join(format!("sunrise-temp-copy-{}", std::process::id()));
    fs::create_dir_all(&copy).unwrap();
    let found = in_temp_folder(&copy, &system);
    let _ = fs::remove_dir_all(&copy);
    assert!(found);
    let _ = fs::remove_dir_all(root);
}

/// A fresh folder for one test, under the temporary folder.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sunrise-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn stage_named(work: &Path, pid: u32) -> PathBuf {
    let stage = work.join(format!("{pid}-0123456789abcdef0123456789abcdef"));
    fs::create_dir_all(&stage).unwrap();
    fs::write(stage.join("download.zip"), b"archive").unwrap();
    stage
}

/// A crashed launcher's stage goes, and nothing another process may still use does: a live
/// process's however old, a recent one whose process has gone, and one handed to an installer
/// after sitting in a long-lived window, though its folder is old.
#[test]
fn only_abandoned_stages_are_cleaned_up() {
    let work = scratch("stage-cleanup");
    let live = stage_named(&work, 100);
    let crashed = stage_named(&work, 200);
    let handed_over = stage_named(&work, 300);
    let old_launcher_file = work.join("download");
    fs::write(&old_launcher_file, b"partial").unwrap();
    let is_alive = |pid: u32| pid == 100;
    let later = std::time::SystemTime::now() + 2 * STAGE_GRACE;
    let marker = fs::File::create(handed_over.join(HANDED_OVER)).unwrap();
    marker.set_modified(later).unwrap();
    drop(marker);

    remove_abandoned_stages(&work, std::time::SystemTime::now(), is_alive);
    assert!(
        crashed.exists(),
        "gone only just now: its installer may be working"
    );

    remove_abandoned_stages(&work, later, is_alive);
    assert_eq!(fs::read(live.join("download.zip")).unwrap(), b"archive");
    assert!(handed_over.join("download.zip").exists());
    assert!(!crashed.exists());
    assert!(!old_launcher_file.exists());
    let _ = fs::remove_dir_all(work);
}

/// Each stage is its own folder, and removing one leaves another process's as it was. Only a
/// path inside a stage folder removes anything.
#[test]
fn removing_a_stage_leaves_the_others_alone() {
    let work = scratch("stage-remove");
    let mine = new_stage_folder(&work).unwrap();
    let theirs = new_stage_folder(&work).unwrap();
    assert_ne!(mine, theirs);
    fs::write(theirs.join("download.zip"), b"their download").unwrap();
    fs::write(mine.join("download.zip"), b"mine").unwrap();

    remove_stage(&work, &work.join("unpacked").join("sunrise-launcher"));
    remove_stage(&work, &mine.join("..").join("sunrise-launcher"));
    remove_stage(&work, &work);
    assert!(mine.exists() && theirs.exists());

    remove_stage(&work, &mine.join("download.zip"));
    assert!(!mine.exists());
    assert_eq!(
        fs::read(theirs.join("download.zip")).unwrap(),
        b"their download"
    );
    let _ = fs::remove_dir_all(work);
}

/// A download that doesn't match keeps nothing, not even a temporary file, and doesn't touch a
/// finished file already there. One that matches appears under its name only.
#[test]
fn a_download_takes_its_name_only_after_the_hash_check() {
    let dir = scratch("save-checked");
    let to = dir.join("download.zip");
    let bytes: &[u8] = b"the launcher";
    let right = sunrise_launcher_release_verify::sha256_hex(bytes);
    let wrong = sunrise_launcher_release_verify::sha256_hex(b"something else");
    fs::write(&to, b"finished earlier").unwrap();

    assert!(save_checked(bytes, 0, "asset", &wrong, &to, &|_: f32| {}).is_err());
    assert_eq!(fs::read(&to).unwrap(), b"finished earlier");
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);

    save_checked(bytes, 0, "asset", &right, &to, &|_: f32| {}).unwrap();
    assert_eq!(fs::read(&to).unwrap(), bytes);
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_process_that_has_exited_is_not_alive() {
    assert!(process_alive(std::process::id()));
    // The test binary itself, which exits at once on an option it doesn't know.
    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    child.wait().unwrap();
    drop(child);
    assert!(!process_alive(pid));
}

/// A launcher whose folder takes no new file (a disk image, macOS's translocated copy of an
/// app it was not moved from) can't put the new one in place.
#[cfg(unix)]
#[test]
fn a_read_only_folder_takes_no_update() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("sunrise-read-only-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    assert!(writable(&dir));
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
    // root writes anywhere.
    if unsafe { libc::geteuid() } != 0 {
        assert!(!writable(&dir));
    }
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        fs::read_dir(&dir).unwrap().count(),
        0,
        "the probe is removed"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// `$APPIMAGE` names the file the runtime mounted; a path that is not there (moved or deleted
/// since the launcher started) is no AppImage, and the launcher updates its own binary again.
/// Nor is one inherited from another AppImage's program: the binary has to run from `$APPDIR`.
// AppImages are Linux's, and the paths below are absolute only on Unix.
#[cfg(unix)]
#[test]
fn only_an_appimage_file_counts() {
    use std::ffi::OsStr;
    let mount = OsStr::new("/tmp/.mount_Sunrise123");
    let exe = Path::new("/tmp/.mount_Sunrise123/usr/bin/sunrise-launcher");
    assert_eq!(appimage_path_in(None, Some(mount), exe), None);
    assert_eq!(
        appimage_path_in(Some(OsStr::new("Sunrise.AppImage")), Some(mount), exe),
        None,
        "a relative path is no $APPIMAGE"
    );
    assert_eq!(
        appimage_path_in(
            Some(OsStr::new("/nonexistent/Sunrise.AppImage")),
            Some(mount),
            exe
        ),
        None
    );
    let file = std::env::temp_dir().join(format!("sunrise-appimage-{}", std::process::id()));
    std::fs::write(&file, b"#!AppImage").unwrap();
    assert_eq!(
        appimage_path_in(Some(file.as_os_str()), Some(mount), exe),
        Some(file.clone())
    );
    // Inherited from another AppImage: this launcher is installed elsewhere.
    let elsewhere = Path::new("/home/a/.local/bin/sunrise-launcher");
    assert_eq!(
        appimage_path_in(Some(file.as_os_str()), Some(mount), elsewhere),
        None
    );
    assert_eq!(appimage_path_in(Some(file.as_os_str()), None, exe), None);
    let _ = std::fs::remove_file(&file);
}

#[test]
fn launcher_semver_and_historical_order_stay_the_same() {
    assert_eq!(compare_versions("v0.2.0", "v2026.10.19.15"), Greater);
    assert_eq!(compare_versions("v0.2.0", "v2026.10.21"), Greater);
    assert_eq!(compare_versions("v2099.12.31.9", "v0.2.0"), Less);
    assert_eq!(compare_versions("v0.2.1", "v0.2.0"), Greater);
    assert_eq!(compare_versions("v0.2.10", "v0.2.9"), Greater);
    assert_eq!(compare_versions("v0.3.0", "v0.2.57"), Greater);
    assert_eq!(compare_versions("v0.2.0", "v0.2.0"), Equal);
    // Pre-releases come before their release and after the one before.
    assert_eq!(compare_versions("v0.3.0-rc.1", "v0.3.0"), Less);
    assert_eq!(compare_versions("v0.3.0-rc.1", "v0.2.9"), Greater);
    assert_eq!(compare_versions("v0.3.0-beta.11", "v0.3.0-beta.2"), Greater);
    assert_eq!(compare_versions("v0.3.0-rc.1", "v0.3.0-beta.9"), Greater);
    assert_eq!(compare_versions("v0.3.0-beta.1", "v0.3.0-beta"), Greater);
    assert_eq!(compare_versions("v0.3.0-alpha", "v0.3.0-1"), Greater);
    assert_eq!(compare_versions("v0.2.1-beta.1", "v2026.10.21"), Greater);
    // The two launcher tag eras still compare in the same order.
    assert_eq!(
        compare_versions("launcher@0.2.0", "launcher-v0.16.0"),
        Greater
    );
    assert_eq!(compare_versions("launcher@0.2.0", "v0.2.0"), Equal);
}

#[test]
fn before_the_first_launcher_release_there_is_no_update() {
    assert!(parse_launcher_releases(&[], launcher_asset()).is_empty());
    assert!(launcher_update_from(&[], "v1.0.0").is_none());
}

#[test]
fn launcher_discovery_keeps_only_exact_vxyz_tags() {
    let asset = launcher_asset().unwrap_or("unsupported");
    let release = |tag: &str| {
        serde_json::json!({"tag_name": tag, "assets": [
            {"name": asset, "browser_download_url": "u", "size": 1}
        ]})
    };
    let list = [
        "v1.1.0",
        "v1.2.0-rc1",
        "v1.2",
        "v1.3.0/../x",
        "launcher@1.4.0",
    ]
    .map(release);
    let tags: Vec<_> = parse_launcher_releases(&list, launcher_asset())
        .into_iter()
        .map(|r| r.tag)
        .collect();
    assert_eq!(tags, ["v1.1.0"]);
}

#[test]
fn versions_compare_by_number() {
    assert_eq!(compare_versions("v2026.10.13", "v2026.10.12"), Greater);
    assert_eq!(compare_versions("v2026.9.30", "v2026.10.1"), Less);
    assert_eq!(compare_versions("v2026.10.13.1", "v2026.10.13"), Greater);
    assert_eq!(compare_versions("0.10.0", "0.9.9"), Greater);
    assert_eq!(compare_versions("launcher-v1.2", "1.2.0"), Equal);
    assert_eq!(compare_versions("1.0.0-rc1", "1.0.0"), Equal);
}

#[test]
fn launcher_updates_from_earlier_launcher_tags() {
    let r = |tag: &str| Release {
        tag: tag.into(),
        name: String::new(),
        body: String::new(),
        date: String::new(),
        time: String::new(),
        published_at: String::new(),
        asset: Some(("u".into(), 1)),
        prerelease: false,
        nightly: None,
    };
    assert!(launcher_update(&[r("launcher-v0.0.1")]).is_none());
    assert!(launcher_update(&[r("launcher@99.0.0")]).is_some());
    assert!(launcher_update_from(&[r("launcher-v99.0.0")], "launcher-v0.16.0").is_some());
}

#[test]
fn published_builds_and_updates_are_channel_scoped() {
    use crate::instances::Channel::{Nightly, Stable};
    let asset = "sunrise-linux-x86_64.tar.gz";
    let files = serde_json::json!([{"name": asset, "size": 1}]);
    let list = serde_json::json!([
        {"tag_name": "nightly", "name": "Nightly 2026-11-20 · aaaaaaa", "prerelease": true, "assets": files},
        {"tag_name": "nightly-2026-11-19", "assets": files},
        {"tag_name": "v2026.10.13", "assets": files},
        {"tag_name": "v1.0.0", "assets": files},
        {"tag_name": "launcher@0.2.21", "assets": files},
        {"tag_name": "26.3-0.1.0", "assets": files},
        {"tag_name": "26.3-0.1.2", "prerelease": true, "assets": files},
        {"tag_name": "26.3-0.1.3-rc.1", "assets": files},
        {"tag_name": "26.3-0.1.4", "draft": true, "assets": files},
        {"tag_name": "26.3-0.1.1", "assets": files},
    ]);
    let releases = game_releases_with(list.as_array().unwrap(), &[Some(asset)]);
    let nightly = "nightly-2026-11-20-aaaaaaa";
    assert_eq!(
        releases.iter().map(|r| r.tag.as_str()).collect::<Vec<_>>(),
        [nightly, "26.3-0.1.1", "26.3-0.1.0"]
    );
    assert_eq!(
        releases[1].asset.as_ref().unwrap().0,
        format!("https://github.com/sunrise-org/sunrise/releases/download/26.3-0.1.1/{asset}")
    );
    let offered = |catalogue: &[Release], installed: &[&str], channel| {
        let installed = installed.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        sunrise_update(catalogue, &installed, channel).map(|r| r.tag.clone())
    };
    assert_eq!(
        offered(&releases, &[nightly, "26.3-0.1.0"], Stable),
        Some("26.3-0.1.1".into())
    );
    assert_eq!(
        offered(
            &releases,
            &["26.3-0.1.1", "nightly-2026-11-20-bbbbbbb"],
            Nightly
        ),
        Some(nightly.into())
    );
    assert_eq!(offered(&releases, &[nightly, "26.3-0.1.1"], Stable), None);
    assert_eq!(offered(&releases[..1], &["26.3-0.1.0"], Stable), None);
    assert_eq!(
        offered(&releases[1..], &["nightly-2026-11-19-aaaaaaa"], Nightly),
        None
    );
    assert_eq!(
        offered(&releases, &["nightly-2026-11-21-aaaaaaa"], Nightly),
        None
    );
    assert_eq!(offered(&releases, &["26.4-0.1.0"], Stable), None);
}

#[test]
fn links_fall_back_to_the_built_in_ones() {
    let defaults = Links::default();
    assert_eq!(defaults.discord, "https://discord.gg/eswQcHfRKC");
    assert_eq!(defaults.website, "https://playsunrise.net");
    assert_eq!(defaults.roadmap, "");
    assert_eq!(defaults.docs, "");
    assert_eq!(
        Links::parse(r#"{"roadmap":"https://example.org/tasks","docs":" "}"#),
        Links {
            roadmap: "https://example.org/tasks".into(),
            docs: String::new(),
            ..defaults.clone()
        }
    );
    // The file on main says the same thing as the defaults.
    assert_eq!(Links::parse(include_str!("../../links.json")), defaults);
    // Unreadable or missing, offline included: the defaults.
    assert_eq!(Links::parse(""), defaults);
    assert_eq!(Links::parse("<html>404</html>"), defaults);
    assert_eq!(Links::parse("[1]"), defaults);
    assert_eq!(Links::parse(r#"{"discord": 5}"#), defaults);
    // A key that is missing keeps its default; a new address replaces it; empty hides it.
    assert_eq!(
        Links::parse(r#"{"discord": "https://x.y/z"}"#),
        Links {
            discord: "https://x.y/z".into(),
            ..defaults.clone()
        }
    );
    assert_eq!(
        Links::parse(r#"{"discord": "https://x.y/z", "website": " "}"#),
        Links {
            discord: "https://x.y/z".into(),
            website: String::new(),
            ..defaults.clone()
        }
    );
    // A key this launcher does not know is ignored.
    assert_eq!(Links::parse(r#"{"mastodon": "https://x.y/z"}"#), defaults);
}

#[test]
fn a_drawn_page_asks_the_disk_once_and_the_launch_check_asks_again() {
    use std::time::{Duration, Instant};
    let temp = tempfile::tempdir().unwrap();
    let version = temp.path();
    let dir = resources_at(version);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(RESOURCES_MANIFEST), "{}").unwrap();
    fs::write(dir.join("items.json"), "{}").unwrap();
    let cache = Mutex::new(PreparedCache::default());
    let t0 = Instant::now();
    // The page's question: without the cache each of these would run the check.
    let disk_reads = std::cell::Cell::new(0);
    let draw = |now| {
        cached_prepared(&cache, version, now, || {
            disk_reads.set(disk_reads.get() + 1);
            check_play_resources(version, &dir).is_ok()
        })
    };
    assert!(draw(t0), "a complete folder is prepared");

    // A table is emptied behind the cache (a killed export): a frame still answers from
    // memory, without touching the disk, for hundreds of frames.
    fs::write(dir.join("items.json"), "").unwrap();
    for frame in 1..=500 {
        assert!(draw(t0 + Duration::from_millis(frame)), "frame {frame}");
    }
    assert_eq!(disk_reads.get(), 1, "500 frames read the disk once");

    // Before a launch the folder is checked on the disk, and finds the empty table.
    assert!(
        checked_prepared(&cache, version, version, t0).is_err(),
        "an empty table is caught"
    );
    // ... and the pages then show it without checking again.
    assert!(!draw(t0 + Duration::from_secs(1)));
    assert_eq!(disk_reads.get(), 1);

    // Repaired, the folder is believed prepared again once the answer is dropped or old.
    fs::write(dir.join("items.json"), "{}").unwrap();
    assert!(!draw(t0 + Duration::from_secs(2)), "still the old answer");
    cache.lock().unwrap().0.clear();
    assert!(draw(t0 + Duration::from_secs(2)));
    assert_eq!(disk_reads.get(), 2);
    fs::write(dir.join("items.json"), "").unwrap();
    assert!(draw(t0 + Duration::from_secs(3)), "answer is a second old");
    assert!(
        !draw(t0 + Duration::from_secs(2) + PREPARED_TTL),
        "past the time limit a change made by another process shows"
    );
    assert_eq!(disk_reads.get(), 3);
}

#[test]
fn each_version_keeps_its_own_prepared_answer() {
    let now = std::time::Instant::now();
    let cache = Mutex::new(PreparedCache::default());
    let (a, b) = (Path::new("versions/a"), Path::new("versions/b"));
    assert!(cached_prepared(&cache, a, now, || true));
    assert!(!cached_prepared(&cache, b, now, || false));
    assert!(cached_prepared(&cache, a, now, || unreachable!(
        "a is known"
    )));
    assert!(!cached_prepared(&cache, b, now, || unreachable!(
        "b is known"
    )));
}

#[test]
fn a_prepared_folder_with_an_empty_table_is_not_prepared() {
    let dir = std::env::temp_dir().join(format!("sunrise-launcher-empty-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    assert!(!prepared_at(&dir), "no manifest is not prepared");

    fs::write(dir.join(RESOURCES_MANIFEST), "{}").unwrap();
    assert!(prepared_at(&dir));

    // A killed export left the biggest table empty: Play must prepare
    // again instead of starting a game that crashes on it.
    fs::write(dir.join("gameplay-facts.json"), "").unwrap();
    assert!(!prepared_at(&dir));

    fs::write(dir.join("gameplay-facts.json"), "{}").unwrap();
    assert!(prepared_at(&dir));

    fs::write(dir.join(RESOURCES_MANIFEST), "").unwrap();
    assert!(!prepared_at(&dir), "an empty manifest is not prepared");
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn a_version_is_prepared_once_at_a_time() {
    let dir = std::env::temp_dir().join(format!(
        "sunrise-launcher-prepare-lock-{}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    let first = prepare_lock_at(&dir, "v1").unwrap();
    let second = prepare_lock_at(&dir, "v1").err().unwrap();
    assert!(second.contains("already being prepared"), "{second}");
    drop(first);
    assert!(
        prepare_lock_at(&dir, "v1").is_ok(),
        "free once the first ends"
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn a_prepared_version_asks_its_preparer_for_a_data_file_once() {
    use crate::preparer::DATA_FILE;
    use std::os::unix::fs::PermissionsExt;
    let root =
        std::env::temp_dir().join(format!("sunrise-launcher-data-file-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let resources = root.join("resources");
    fs::create_dir_all(&resources).unwrap();
    let binary = root.join("sunrise-prepare");
    fs::write(
        &binary,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$(dirname \"$0\")/asked\"\necho '{\"format\": 1, \"written\": true}'\n",
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let asked = || fs::read_to_string(root.join("asked")).unwrap_or_default();
    // Not prepared yet: nothing to hold, nothing asked.
    write_data_file(&root, &resources).unwrap();
    assert_eq!(asked(), "");
    fs::write(resources.join(RESOURCES_MANIFEST), "{}").unwrap();
    write_data_file(&root, &resources).unwrap();
    let args: Vec<String> = asked().lines().map(str::to_owned).collect();
    assert_eq!(args[0], "data-file");
    assert!(
        args.windows(2)
            .any(|pair| pair[0] == "--output" && pair[1] == resources.to_string_lossy()),
        "{args:?}"
    );
    // One already there stays: the game judges whether it can use it.
    fs::write(resources.join(DATA_FILE), "kept").unwrap();
    fs::remove_file(root.join("asked")).unwrap();
    write_data_file(&root, &resources).unwrap();
    assert_eq!(asked(), "");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_new_version_reuses_resources_prepared_by_a_compatible_one() {
    let versions =
        std::env::temp_dir().join(format!("sunrise-launcher-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&versions);
    let oracle = "mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaHeightmapOracle.java";
    let camera = "mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCameraOracle.java";
    let collision = "mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCollisionOracle.java";
    let write = |tag: &str, path: &str, text: &str| {
        let path = versions.join(tag).join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    let release = |tag: &str, minecraft: &str| {
        write(tag, COMPLETE, "");
        let lock = serde_json::json!({"minecraft": {"version": minecraft, "client_sha1": "c", "server_sha1": "s"}});
        write(tag, "research-lock.json", &lock.to_string());
        write(tag, "tools/prepare_native_client.py", "schema 7");
        write(tag, oracle, "heightmap");
        write(tag, collision, "collision");
        write(tag, camera, tag); // Not an exporter: it may change freely.
    };
    let prepared = |tag: &str, minecraft: &str| {
        let manifest =
            serde_json::json!({"schema": 7, "collision_exporters_sha256": {collision: "x"}});
        write(
            tag,
            &format!(".local/sunrise-client/{minecraft}/{RESOURCES_MANIFEST}"),
            &manifest.to_string(),
        );
        write(
            tag,
            &format!(".local/sunrise-client/{minecraft}/worldgen/biomes.json"),
            "{}",
        );
    };
    let is_prepared = |tag: &str| prepared_at(&resources_at(&versions.join(tag)));

    release("v2026.10.13", "26.3");
    prepared("v2026.10.13", "26.3");
    release("v2026.10.12", "26.2");
    prepared("v2026.10.12", "26.2");
    // A data file names its own root: the new version writes its own.
    let data_file = crate::preparer::DATA_FILE;
    write(
        "v2026.10.13",
        &format!(".local/sunrise-client/26.3/{data_file}"),
        "written for v2026.10.13",
    );
    release("v2026.10.13.1", "26.3");
    assert_eq!(
        reuse_prepared(&versions, "v2026.10.13.1"),
        Some("v2026.10.13".into())
    );
    assert!(is_prepared("v2026.10.13.1"));
    assert!(resources_at(&versions.join("v2026.10.13.1"))
        .join("worldgen/biomes.json")
        .exists());
    assert!(!resources_at(&versions.join("v2026.10.13.1"))
        .join(data_file)
        .exists());
    assert!(resources_at(&versions.join("v2026.10.13"))
        .join(data_file)
        .exists());
    let left: Vec<_> = fs::read_dir(versions.join("v2026.10.13.1/.local/sunrise-client"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(left, ["26.3"]);
    // Already prepared: nothing to do.
    assert_eq!(reuse_prepared(&versions, "v2026.10.13.1"), None);

    // Another Minecraft version, a changed exporter or a changed prepare script: prepare again.
    release("v2026.10.14", "26.4");
    assert_eq!(reuse_prepared(&versions, "v2026.10.14"), None);
    release("v2026.10.15", "26.3");
    write("v2026.10.15", collision, "collision, fixed");
    assert_eq!(reuse_prepared(&versions, "v2026.10.15"), None);
    release("v2026.10.16", "26.3");
    write("v2026.10.16", "tools/prepare_native_client.py", "schema 8");
    assert_eq!(reuse_prepared(&versions, "v2026.10.16"), None);
    assert!(!is_prepared("v2026.10.15") && !is_prepared("v2026.10.16"));

    // A version an older launcher installed without preparing it is caught up by the sweep
    // that starting the window, `versions` and `update` run; the incompatible ones stay as
    // they are, and a second sweep has nothing to do.
    release("v2026.10.17", "26.3");
    assert_eq!(
        reuse_prepared_in(&versions),
        [("v2026.10.17".to_string(), "v2026.10.13.1".to_string())]
    );
    assert!(is_prepared("v2026.10.17"));
    assert!(!is_prepared("v2026.10.15") && !is_prepared("v2026.10.16"));
    assert!(reuse_prepared_in(&versions).is_empty());

    // Versions with sunrise-prepare: its sources are the preparer, so the same sources
    // reuse each other's resources, and a change or a version without them prepares again.
    let crate_file = "crates/sunrise-prepare/src/lib.rs";
    release("v2026.10.18", "26.3");
    write("v2026.10.18", crate_file, "rust preparer");
    assert_eq!(reuse_prepared(&versions, "v2026.10.18"), None);
    prepared("v2026.10.18", "26.3");
    release("v2026.10.19", "26.3");
    write("v2026.10.19", crate_file, "rust preparer");
    assert_eq!(
        reuse_prepared(&versions, "v2026.10.19"),
        Some("v2026.10.18".into())
    );
    release("v2026.10.20", "26.3");
    write("v2026.10.20", crate_file, "rust preparer, changed");
    assert_eq!(reuse_prepared(&versions, "v2026.10.20"), None);
    let _ = fs::remove_dir_all(&versions);
}

#[test]
fn prepared_resources_follow_the_preparers_output_version() {
    let versions = std::env::temp_dir().join(format!(
        "sunrise-launcher-output-test-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&versions);
    let write = |tag: &str, path: &str, text: &str| {
        let path = versions.join(tag).join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    let lib = "crates/sunrise-prepare/src/lib.rs";
    // A preparer with its sources (as releases ship them) and the exporters it runs.
    let release = |tag: &str, code: &str| {
        write(tag, COMPLETE, "");
        let lock = serde_json::json!({"minecraft": {"version": "26.3", "client_sha1": "c", "server_sha1": "s"}});
        write(tag, "research-lock.json", &lock.to_string());
        write(tag, "crates/sunrise-prepare/Cargo.toml", "[package]");
        write(tag, "crates/sunrise-prepare/build.rs", "fn main() {}");
        write(tag, lib, code);
        for name in LEGACY_EXPORTERS {
            write(
                tag,
                &format!("mods/sunrise/src/test/java/dev/sunriselab/parity/{name}"),
                name,
            );
        }
    };
    let prepared = |tag: &str| {
        write(
            tag,
            &format!(".local/sunrise-client/26.3/{RESOURCES_MANIFEST}"),
            r#"{"schema": 7}"#,
        );
        write(tag, ".local/sunrise-client/26.3/facts.json", tag);
    };
    let facts =
        |tag: &str| fs::read_to_string(resources_at(&versions.join(tag)).join("facts.json")).ok();

    // The preparer's code changed, not what it makes: the resources are kept.
    release(
        "v0.3.0",
        "pub const OUTPUT_VERSION: u32 = 1;\nfn prepare() {}",
    );
    prepared("v0.3.0");
    release(
        "v0.3.1",
        "pub const OUTPUT_VERSION: u32 = 1;\nfn prepare() { faster() }",
    );
    write(
        "v0.3.0",
        &format!(".local/sunrise-client/26.3/{PREPARATION_STAMP}"),
        r#"{"output_version":1}"#,
    );
    assert_eq!(reuse_prepared(&versions, "v0.3.1"), Some("v0.3.0".into()));
    assert_eq!(facts("v0.3.1").as_deref(), Some("v0.3.0"));
    // Its output changed, so the number did: prepare again.
    release(
        "v0.3.2",
        "pub const OUTPUT_VERSION: u32 = 2;\nfn prepare() {}",
    );
    assert_eq!(reuse_prepared(&versions, "v0.3.2"), None);
    assert_eq!(facts("v0.3.2"), None);
    // Another Minecraft jar, same number: prepare again.
    release(
        "v0.3.3",
        "pub const OUTPUT_VERSION: u32 = 1;\nfn prepare() {}",
    );
    let lock = serde_json::json!({"minecraft": {"version": "26.3", "client_sha1": "other", "server_sha1": "s"}});
    write("v0.3.3", "research-lock.json", &lock.to_string());
    assert_eq!(reuse_prepared(&versions, "v0.3.3"), None);

    // A preparer from before the number is known by its fingerprint.
    release("v0.2.64", "fn prepare() {}");
    let fingerprint = legacy_fingerprint(&versions.join("v0.2.64")).unwrap();
    let legacy = [(fingerprint.as_str(), 1)];
    let at = |tag: &str| {
        let root = versions.join(tag);
        output_version_at(&root)
            .unwrap()
            .or_else(|| legacy_output_version(&root, &legacy))
    };
    assert_eq!(at("v0.2.64"), Some(1));
    assert_eq!(at("v0.3.2"), Some(2));
    // The destination's historical fingerprint is not a reason to trust an unstamped
    // folder that came from a source that declared a version.
    let (versioned, unstamped) = (versions.join("v0.3.0"), versions.join("v0.2.64"));
    let source_resources = resources_at(&versioned);
    assert!(resources_compatible(
        &versioned,
        &unstamped,
        &source_resources,
        &legacy
    ));
    fs::remove_file(source_resources.join(PREPARATION_STAMP)).unwrap();
    assert!(!resources_compatible(
        &versioned,
        &unstamped,
        &source_resources,
        &legacy
    ));
    // Line endings as checked out don't count; an exporter's or the sources' change does.
    write(
        "v0.2.64",
        "mods/sunrise/src/test/java/dev/sunriselab/parity/ParityData.java",
        "ParityData.java\r",
    );
    assert_eq!(at("v0.2.64"), Some(1));
    write(
        "v0.2.64",
        "mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCollisionOracle.java",
        "changed",
    );
    assert_eq!(at("v0.2.64"), None);
    release("v0.2.65", "fn prepare() { changed() }");
    assert_eq!(at("v0.2.65"), None);
    // A folder without the preparer's sources has no number.
    assert_eq!(at("missing"), None);
    let _ = fs::remove_dir_all(&versions);
}

#[test]
fn reuse_checks_the_source_stamp_even_when_both_preparers_declare_the_same_version() {
    let temp = tempfile::tempdir().unwrap();
    let versions = temp.path();
    let (source, target) = (versions.join("v0.1.0"), versions.join("v0.1.1"));
    for root in [&source, &target] {
        let lib = root.join(PREPARER_CRATE).join("src/lib.rs");
        fs::create_dir_all(lib.parent().unwrap()).unwrap();
        fs::write(lib, "pub const OUTPUT_VERSION: u32 = 8;\n").unwrap();
        fs::write(root.join(COMPLETE), "").unwrap();
        fs::write(
            root.join("research-lock.json"),
            r#"{"minecraft":{"version":"26.3","client_sha1":"c","server_sha1":"s"}}"#,
        )
        .unwrap();
    }
    let from = resources_at(&source);
    let to = resources_at(&target);
    fs::create_dir_all(&from).unwrap();
    fs::write(from.join(RESOURCES_MANIFEST), "{}").unwrap();

    let stamp = from.join(PREPARATION_STAMP);
    assert_eq!(reuse_prepared(versions, "v0.1.1"), None);
    for invalid in [
        "{",
        "{}",
        r#"{"output_version":"8"}"#,
        r#"{"output_version":7}"#,
        r#"{"output_version":9}"#,
    ] {
        fs::write(&stamp, invalid).unwrap();
        assert_eq!(reuse_prepared(versions, "v0.1.1"), None, "{invalid}");
        assert!(!to.exists());
    }

    let current = r#"{"output_version":8}"#;
    fs::write(&stamp, current).unwrap();
    fs::write(from.join(RESOURCES_MANIFEST), r#"{"profile":"server"}"#).unwrap();
    assert_eq!(reuse_prepared(versions, "v0.1.1"), None);
    fs::write(from.join(RESOURCES_MANIFEST), "{}").unwrap();
    assert_eq!(
        reuse_prepared(versions, "v0.1.1").as_deref(),
        Some("v0.1.0")
    );
    assert_eq!(
        fs::read_to_string(to.join(PREPARATION_STAMP)).unwrap(),
        current
    );
}

#[test]
fn any_missing_extra_sends_the_store_refill_once_per_run() {
    use std::sync::atomic::AtomicBool;
    let lacks = |kinds: &[&str]| -> crate::preparer::Extras {
        kinds.iter().map(|k| (k.to_string(), "x".into())).collect()
    };
    for kinds in [
        &["fonts"][..],
        &["packs"],
        &["panorama"],
        &["sounds", "fonts"],
        &["languages"],
    ] {
        let asked = AtomicBool::new(false);
        assert!(
            refill_wanted(true, Some(&lacks(kinds)), &asked),
            "{kinds:?}"
        );
        // Asked once: the second call in the run says no.
        assert!(
            !refill_wanted(true, Some(&lacks(kinds)), &asked),
            "{kinds:?}"
        );
    }
    // Nothing missing, no answer from the preparer, or no asset index: not asked, and a
    // refusal does not use up the run's one ask.
    let asked = AtomicBool::new(false);
    assert!(!refill_wanted(true, Some(&lacks(&[])), &asked));
    assert!(!refill_wanted(true, None, &asked));
    assert!(!refill_wanted(false, Some(&lacks(&["fonts"])), &asked));
    assert!(refill_wanted(true, Some(&lacks(&["fonts"])), &asked));
}

#[test]
fn prepared_versions_get_the_run_time_reports_without_preparing_again() {
    let versions = std::env::temp_dir().join(format!(
        "sunrise-launcher-reports-test-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&versions);
    let write = |tag: &str, path: &str, text: &str| {
        let path = versions.join(tag).join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    let release = |tag: &str, server: &str| {
        write(tag, COMPLETE, "");
        let lock = serde_json::json!({"minecraft": {"version": "26.3", "client_sha1": "c", "server_sha1": server}});
        write(tag, "research-lock.json", &lock.to_string());
        write(
            tag,
            &format!(".local/sunrise-client/26.3/{RESOURCES_MANIFEST}"),
            "{}",
        );
    };
    let resources = |tag: &str| resources_at(&versions.join(tag));
    let datagen = |tag: &str, name: &str| {
        write(tag, &format!(".local/datagen/out/reports/{name}"), tag);
    };

    // Prepared before the reports shipped with the resources: its own scratch output.
    release("v2026.10.13", "s");
    datagen("v2026.10.13", "registries.json");
    datagen("v2026.10.13", "commands.json");
    // Resources copied from it: the reports come from the version that ran the generator.
    release("v2026.10.14", "s");
    // Another server jar: the reports of the others would be wrong, and it has none.
    release("v2026.10.15", "other");
    assert!(reuse_prepared_in(&versions).is_empty());
    for tag in ["v2026.10.13", "v2026.10.14"] {
        for name in RUNTIME_REPORTS {
            assert_eq!(
                fs::read_to_string(resources(tag).join(name)).unwrap(),
                "v2026.10.13"
            );
        }
    }
    assert!(!resources("v2026.10.15").join("registries.json").exists());
    assert!(!complete_reports(&versions, "v2026.10.15"));
    // A copy that already holds the old shared temp name (another call mid-copy) does not
    // stop this call: each call copies to a name of its own.
    let squatted =
        resources("v2026.10.14").join(format!(".commands.json.partial-{}", std::process::id()));
    fs::remove_file(resources("v2026.10.14").join("commands.json")).unwrap();
    fs::create_dir(&squatted).unwrap();
    assert!(complete_reports(&versions, "v2026.10.14"));
    assert_eq!(
        fs::read_to_string(resources("v2026.10.14").join("commands.json")).unwrap(),
        "v2026.10.13"
    );
    fs::remove_dir(&squatted).unwrap();
    // Once there they stay as they are, even when the scratch output is gone.
    fs::remove_dir_all(versions.join("v2026.10.13/.local/datagen")).unwrap();
    assert!(complete_reports(&versions, "v2026.10.13"));
    let left: Vec<_> = fs::read_dir(resources("v2026.10.14"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().into_string().unwrap())
        .filter(|n| n.contains("partial"))
        .collect();
    assert!(left.is_empty());
    let _ = fs::remove_dir_all(&versions);
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn prepared_versions_get_the_extras_their_own_preparer_adds_once() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("sunrise-launcher-extras-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let versions = dir.join("versions");
    // Each version's own preparer: what its resources lack, and what `extras` did with the folders it got.
    let version = |tag: &str, prepared: bool, preparer: &str| {
        let root = versions.join(tag);
        fs::create_dir_all(&root).unwrap();
        if prepared {
            let resources = resources_at(&root);
            fs::create_dir_all(&resources).unwrap();
            fs::write(resources.join(RESOURCES_MANIFEST), b"{}").unwrap();
        }
        let binary = root.join("sunrise-prepare");
        fs::write(&binary, format!("#!/bin/sh\n{preparer}\n")).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        root
    };
    let lacking = r#"{"format": 1, "prepared": true, "missing": [{"kind": "packs", "what": "The built-in packs"}, {"kind": "sounds", "what": "The sounds"}]}"#;
    let added = r#"{"format": 1, "added": ["packs"], "failed": [{"kind": "sounds", "error": "no sound index"}], "missing": [{"kind": "sounds", "what": "The sounds"}]}"#;
    let preparer = format!(
        r#"case "$1" in
missing-extras) printf '%s\n' '{lacking}' ;;
extras) printf '%s\n' "$@" >> "$(dirname "$0")/extras-calls"; printf '%s\n' '{{"message": "Built-in packs ready"}}' '{added}'; exit 1 ;;
*) exit 2 ;;
esac"#
    );
    let lines = std::cell::RefCell::new(Vec::new());
    let log = |line: String| lines.borrow_mut().push(line);
    let asked = std::cell::Cell::new(0);
    let folders = || {
        asked.set(asked.get() + 1);
        vec![dir.join("official"), dir.join("prism")]
    };

    let root = version("v1", true, &preparer);
    complete_assets_in(&versions, "v1", folders, &log, None);
    complete_assets_in(&versions, "v1", folders, &log, None);
    assert_eq!(
        *lines.borrow(),
        [
            "Built-in packs ready",
            "v1: The sounds not added: no sound index"
        ]
    );
    assert_eq!(asked.get(), 1, "one try per run");
    let calls = fs::read_to_string(root.join("extras-calls")).unwrap();
    let args: Vec<&str> = calls.lines().collect();
    for wanted in [
        &["--only", "packs,sounds"][..],
        &["--minecraft-dir", dir.join("official").to_str().unwrap()],
        &["--minecraft-dir", dir.join("prism").to_str().unwrap()],
    ] {
        assert!(
            args.windows(2).any(|pair| pair == wanted),
            "{wanted:?} in {args:?}"
        );
    }

    // A preparer from before `missing-extras`, a version not prepared, and one that lacks nothing: no folder is asked for.
    lines.borrow_mut().clear();
    version("v0", true, "exit 2");
    version("v2", false, &preparer);
    version(
        "v3",
        true,
        r#"printf '%s\n' '{"format": 1, "prepared": true, "missing": []}'"#,
    );
    for tag in ["v0", "v2", "v3"] {
        complete_assets_in(
            &versions,
            tag,
            || unreachable!("no folder is needed"),
            &log,
            None,
        );
    }
    assert!(lines.borrow().is_empty(), "{:?}", lines.borrow());

    // An interrupted writer can change the preparation before it reports what it added.
    let interrupted = version(
        "interrupted",
        true,
        &format!(
            r#"case "$1" in
missing-extras) printf '%s\n' '{lacking}' ;;
extras) test ! -e .local/launcher-prepared/current.json || exit 2
        printf changed > "$6/writer-proof"; exit 1 ;;
esac"#
        ),
    );
    let store = interrupted.join(".local/launcher-prepared");
    let snapshot = store.join("0".repeat(64)).join("resources");
    fs::create_dir_all(&snapshot).unwrap();
    fs::write(snapshot.join("writer-proof"), b"original").unwrap();
    let receipt = store.join("current.json");
    let before = serde_json::json!({"primary": vec![0; 32], "accelerator": null});
    fs::write(&receipt, before.to_string()).unwrap();
    let preparation = prepare_lock_at(&interrupted, "interrupted").unwrap();
    complete_assets_in(
        &versions,
        "interrupted",
        folders,
        &log,
        Some(&preparation.handoff(None)),
    );
    assert!(!receipt.exists());
    assert_eq!(
        fs::read(resources_at(&interrupted).join("writer-proof")).unwrap(),
        b"changed"
    );
    assert_eq!(
        fs::read(snapshot.join("writer-proof")).unwrap(),
        b"original"
    );
    drop(preparation);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_research_pin_can_only_name_one_prepared_version_component() {
    let dir = tempfile::tempdir().unwrap();
    let pin = dir.path().join("research-lock.json");
    for version in [
        "..",
        "../prepared",
        "/prepared",
        "a/b",
        "a\\b",
        "C:prepared",
    ] {
        fs::write(
            &pin,
            serde_json::json!({"minecraft": {"version": version}}).to_string(),
        )
        .unwrap();
        assert_eq!(locked_game(dir.path()), None);
        assert_eq!(
            resources_at(dir.path()),
            dir.path().join(".local/sunrise-client").join(PINNED_GAME)
        );
    }
}

#[test]
fn finds_the_app_bundle_around_the_binary() {
    let exe = Path::new("/Applications/Sunrise.app/Contents/MacOS/sunrise-launcher");
    assert_eq!(
        app_bundle(exe),
        Some(Path::new("/Applications/Sunrise.app"))
    );
    assert_eq!(app_bundle(Path::new("/Users/me/sunrise-launcher")), None);
    assert_eq!(
        app_bundle(Path::new("/x/Contents/MacOS/sunrise-launcher")),
        None
    );
    let work = std::env::temp_dir().join(format!("sunrise-find-app-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(work.join("dist/Sunrise.app/Contents/MacOS")).unwrap();
    assert_eq!(find_app(&work), Some(work.join("dist/Sunrise.app")));
    let _ = fs::remove_dir_all(&work);
}

/// The whole update of an app bundle, as the release ships it: signed, zipped with `ditto`,
/// unpacked by the launcher, swapped in place of the installed one, still valid.
#[cfg(target_os = "macos")]
#[test]
fn an_app_bundle_updates_in_place() {
    use std::os::unix::fs::PermissionsExt;
    let work = std::env::temp_dir().join(format!("sunrise-bundle-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).unwrap();
    let source = work.join("true.c");
    fs::write(&source, "int main(void) { return 0; }\n").unwrap();
    let make = |dir: &Path, version: &str| {
        let app = dir.join("Sunrise.app");
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        fs::create_dir_all(app.join("Contents/Resources")).unwrap();
        let bin = app.join("Contents/MacOS").join(crate::identity::EXECUTABLE);
        // Built here, not a copy of /usr/bin/true: that is an arm64e system program, which
        // macOS stops running once re-signed ad hoc below unless System Integrity Protection is off.
        let built = Command::new("cc")
            .arg(&source)
            .arg("-o")
            .arg(&bin)
            .status()
            .unwrap();
        assert!(built.success());
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(app.join("Contents/Resources/version"), version).unwrap();
        fs::write(
            app.join("Contents/Info.plist"),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
                 <key>CFBundleExecutable</key><string>sunrise-launcher</string>\
                 <key>CFBundleIdentifier</key><string>dev.sunrise.test</string>\
                 <key>CFBundlePackageType</key><string>APPL</string></dict></plist>\n"
                .replace("sunrise-launcher", crate::identity::EXECUTABLE),
        )
        .unwrap();
        let signed = Command::new("codesign")
            .args(["--force", "--sign", "-"])
            .arg(&app)
            .status()
            .unwrap();
        assert!(signed.success());
        app
    };
    let installed = make(&work.join("Applications"), "1.0.0");
    let release = make(&work.join("release"), "2.0.0");
    let zip = work.join("release.zip");
    let zipped = Command::new("ditto")
        .args(["-c", "-k", "--norsrc", "--noextattr", "--keepParent"])
        .arg(&release)
        .arg(&zip)
        .status()
        .unwrap();
    assert!(zipped.success());
    unpack(&zip, "release.zip", &work.join("unpacked"), None).unwrap();
    let new = find_app(&work.join("unpacked")).unwrap();
    replace_bundle(&new, &installed).unwrap();
    let version = fs::read_to_string(installed.join("Contents/Resources/version")).unwrap();
    assert_eq!(version, "2.0.0");
    let bin = installed
        .join("Contents/MacOS")
        .join(crate::identity::EXECUTABLE);
    assert!(
        Command::new(&bin).status().unwrap().success(),
        "still executable"
    );
    let valid = Command::new("codesign")
        .args(["--verify", "--strict"])
        .arg(&installed)
        .status()
        .unwrap();
    assert!(valid.success(), "the updated bundle's signature holds");
    let left: Vec<_> = fs::read_dir(work.join("Applications"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(
        left,
        ["Sunrise.app"],
        "nothing staged or moved aside is left"
    );

    // A bundle changed after it was signed is refused, and the installed one stays.
    let tampered = make(&work.join("tampered"), "3.0.0");
    fs::write(tampered.join("Contents/Resources/version"), "3.0.1").unwrap();
    assert!(replace_bundle(&tampered, &installed).is_err());
    let version = fs::read_to_string(installed.join("Contents/Resources/version")).unwrap();
    assert_eq!(version, "2.0.0");
    let left = fs::read_dir(work.join("Applications")).unwrap().count();
    assert_eq!(left, 1, "nothing staged is left");
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn launcher_versions_read_from_the_binary() {
    assert_eq!(
        parse_version_output("sunrise-launcher 0.1.5\n"),
        Some("0.1.5")
    );
    assert_eq!(parse_version_output("something else"), None);
    assert_eq!(parse_version_output(""), None);
    // This test binary is not a launcher: no version, so an update goes ahead.
    assert_eq!(
        installed_launcher_version(&std::env::current_exe().unwrap()),
        None
    );
    assert_eq!(
        installed_launcher_version(Path::new("/nonexistent/sunrise-launcher")),
        None
    );
}

/// The launcher's file was replaced while it ran (an update from the command line, or an
/// earlier one): the next replacement still lands at the path it started from.
#[cfg(unix)]
#[test]
fn a_replaced_binary_is_replaced_again_at_its_path() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("sunrise-launcher-replace-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("sunrise-launcher");
    fs::write(&exe, "old").unwrap();
    // Another update swaps the file under the running launcher.
    fs::write(dir.join("other"), "other update").unwrap();
    fs::rename(dir.join("other"), &exe).unwrap();
    fs::write(dir.join("new"), "new").unwrap();
    replace_binary(&dir.join("new"), &exe).unwrap();
    assert_eq!(fs::read_to_string(&exe).unwrap(), "new");
    // The file there keeps its mode, made executable (the swapped-in one was 0644).
    assert_eq!(
        fs::metadata(&exe).unwrap().permissions().mode() & 0o777,
        0o755
    );
    // Gone altogether: it is put back, executable.
    fs::remove_file(&exe).unwrap();
    replace_binary(&dir.join("new"), &exe).unwrap();
    assert_eq!(fs::read_to_string(&exe).unwrap(), "new");
    assert_eq!(
        fs::metadata(&exe).unwrap().permissions().mode() & 0o777,
        0o755
    );
    // No temporary file is left beside it.
    let mut names: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["new", "sunrise-launcher"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn crash_reports_are_found_and_summarised() {
    let dir = std::env::temp_dir().join(format!("sunrise-launcher-crash-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let start = std::time::SystemTime::now() - std::time::Duration::from_secs(2);
    assert!(crash_report_in_since(&dir, start).is_none());
    fs::create_dir_all(&dir).unwrap();
    let report = "---- Sunrise Crash Report ----\n// Details below.\n\nTime: 2026-10-14 09:05:07\nDescription: Unexpected error\n\npanicked at src/game.rs:10:5:\nboom\n\t0: sunrise_client::game::frame\n";
    fs::write(dir.join("crash-2026-10-14_09.05.07-client.txt"), report).unwrap();
    fs::write(dir.join("notes.txt"), "x").unwrap();
    assert_eq!(
        crash_report_in_since(&dir, start).unwrap(),
        dir.join("crash-2026-10-14_09.05.07-client.txt")
    );
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
    assert!(crash_report_in_since(&dir, later).is_none());
    assert_eq!(
        crash_summary(report).as_deref(),
        Some("panicked at src/game.rs:10:5: boom")
    );
    assert_eq!(crash_summary("no report"), None);
    let log = dir.join("latest.log");
    fs::write(&log, "a\nb\nc\n").unwrap();
    assert_eq!(tail_of(&log, 2).unwrap(), "b\nc\n");
    assert_eq!(tail_of(&log, 100).unwrap(), "a\nb\nc\n");
    assert!(tail_of(&dir.join("missing.log"), 5).is_err());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn plain_notes() {
    let text =
        markdown_to_text("# Title\n\nSome *words* and [a link](https://x.y).\n\n- one\n- two\n");
    assert_eq!(
        text,
        "Title\n\nSome words and a link (https://x.y).\n\n- one\n- two\n"
    );
}

#[test]
fn the_game_does_not_inherit_the_launchers_renderer_workarounds() {
    let mut cmd = Command::new("true");
    cmd.env("GALLIUM_DRIVER", "llvmpipe")
        .env("LIBGL_ALWAYS_SOFTWARE", "1")
        .env("WEBKIT_DISABLE_DMABUF_RENDERER", "1")
        .env("WEBKIT_DISABLE_COMPOSITING_MODE", "1")
        .env("PIPEWIRE_PROPS", "kept");
    strip_launcher_render_env(&mut cmd);
    let envs = cmd.get_envs().collect::<std::collections::HashMap<_, _>>();
    for key in LAUNCHER_RENDER_ENV {
        assert_eq!(envs.get(OsStr::new(key)), Some(&None), "{key} is removed");
    }
    // Only the launcher's own workarounds go: the rest of the environment still reaches the game.
    assert_eq!(
        envs.get(OsStr::new("PIPEWIRE_PROPS")),
        Some(&Some(OsStr::new("kept")))
    );
}

#[test]
fn cancelling_a_download_stops_mid_transfer() {
    use std::io::{Read, Write};
    use std::time::{Duration, Instant};

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("sunrise.tar.gz");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/sunrise.tar.gz", listener.local_addr().unwrap());
    let (release_server, server_gate) = std::sync::mpsc::channel::<()>();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = stream.read(&mut [0; 4096]);
        let total = 256 * 1024;
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n"
        );
        let chunk = vec![b'a'; 64 * 1024];
        let _ = stream.write_all(&chunk);
        let _ = stream.flush();
        // Hold the second chunk until the client cancels or the test ends.
        let _ = server_gate.recv_timeout(Duration::from_secs(5));
        let _ = stream.write_all(&chunk);
    });

    let stop = crate::native_mods::Cancellation::default();
    let stop_on_progress = stop.clone();
    let release_on_progress = release_server.clone();
    let started = Instant::now();
    let result = download_cancellable(
        &url,
        256 * 1024,
        &"0".repeat(64),
        &dest,
        Some(&stop),
        &|fraction| {
            if fraction > 0.0 {
                stop_on_progress.stop();
                let _ = release_on_progress.send(());
            }
        },
    );
    let _ = release_server.send(());
    server.join().unwrap();
    assert_eq!(result.unwrap_err(), crate::native_mods::CANCELLED);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(!dest.exists());
}

#[cfg(unix)]
#[test]
fn cancelling_finish_preparation_stops_running_data_file_helper() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    let tag = format!("cancel-finish-{}", std::process::id());
    let root = version_dir(&tag);
    let _cleanup = Cleanup(root.clone());
    let resources = resources_at(&root);
    fs::create_dir_all(&resources).unwrap();
    fs::write(resources.join(RESOURCES_MANIFEST), b"{}").unwrap();
    let started_marker = root.join("data-file-started");
    let binary = root.join("sunrise-prepare");
    fs::write(
        &binary,
        format!(
            "#!/bin/sh\ncase \"$1\" in\n\
                 missing-extras) printf '%s\\n' '{{\"format\": 1, \"prepared\": true, \"missing\": []}}' ;;\n\
                 data-file) printf started > \"{}\"; exec sleep 30 ;;\n\
                 *) exit 2 ;;\n\
                 esac\n",
            started_marker.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();

    let stop = crate::native_mods::Cancellation::default();
    let cancel_on_start = stop.clone();
    let marker = started_marker.clone();
    let watcher = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !marker.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        cancel_on_start.stop();
    });

    let preparation = prepare_lock(&tag).unwrap();
    let started = Instant::now();
    let result = finish_preparation_cancellable(&tag, &preparation, Some(&stop));
    watcher.join().unwrap();
    assert!(
        started_marker.exists(),
        "data-file helper should have started"
    );
    assert_eq!(result.unwrap_err(), crate::native_mods::CANCELLED);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn cancelling_download_interrupts_stalled_read() {
    use std::io::Write as _;
    use std::time::{Duration, Instant};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (sent_tx, sent_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut req = [0u8; 512];
        let _ = stream.read(&mut req);
        let _ = stream.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\nConnection: close\r\n\r\npartial",
        );
        let _ = stream.flush();
        let _ = sent_tx.send(());
        // Keep the socket open without sending any more bytes until the test finishes.
        let _ = done_rx.recv();
    });

    let stop = crate::native_mods::Cancellation::default();
    let cancel = stop.clone();
    let stopper = std::thread::spawn(move || {
        sent_rx.recv().unwrap();
        std::thread::sleep(Duration::from_millis(50));
        cancel.stop();
    });

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("archive.tar.gz");
    let started = Instant::now();
    let result = download_cancellable(
        &format!("http://{addr}/archive.tar.gz"),
        100_000,
        &"0".repeat(64),
        &dest,
        Some(&stop),
        &|_| {},
    );
    let elapsed = started.elapsed();
    stopper.join().unwrap();
    let _ = done_tx.send(());
    let _ = server.join();
    assert_eq!(result.unwrap_err(), crate::native_mods::CANCELLED);
    assert!(
        elapsed < Duration::from_secs(2),
        "stalled read should cancel promptly, took {elapsed:?}"
    );
    assert!(!dest.exists());
}

#[test]
fn reuse_prepared_skips_native_builds_without_creating_local_or_resealing_base() {
    const CHILD: &str = "SUNRISE_REUSE_NATIVE_SKIP_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let home = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "backend::tests::reuse_prepared_skips_native_builds_without_creating_local_or_resealing_base",
            ])
            .env(CHILD, "1")
            .env("SUNRISE_LAUNCHER_HOME", home.path().join("launcher"))
            .env("SUNRISE_HOME", home.path().join("game"))
            .env("XDG_DATA_HOME", home.path().join("xdg-data"))
            .env("XDG_CONFIG_HOME", home.path().join("xdg-config"))
            .env("SUNRISE_LAUNCHER_LOCATION_FILE", home.path().join("location"))
            .env("SUNRISE_LAUNCHER_TRASH", home.path().join("trash"))
            .env("SUNRISE_LAUNCHER_CLIPBOARD", home.path().join("clipboard"))
            .env("SUNRISE_LAUNCHER_DESKTOP", home.path().join("desktop"))
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("SUNRISE_NO_AUDIO", "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }

    let versions = versions_dir();
    let base = "dev-2026-11-26-ba1f24e";
    let native = "native-aaaaaaaaaaaa-fixture";
    let write = |tag: &str, path: &str, text: &str| {
        let path = versions.join(tag).join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    let lock = r#"{"minecraft":{"version":"26.3","client_sha1":"c","server_sha1":"s"}}"#;
    let seed_common = |tag: &str| {
        write(tag, COMPLETE, "");
        write(tag, "research-lock.json", lock);
        write(tag, "crates/sunrise-prepare/Cargo.toml", "[package]");
        write(tag, "crates/sunrise-prepare/build.rs", "fn main() {}");
        write(
            tag,
            "crates/sunrise-prepare/src/lib.rs",
            "pub const OUTPUT_VERSION: u32 = 1;\n",
        );
    };

    seed_common(base);
    write(
        base,
        &format!(".local/sunrise-client/26.3/{RESOURCES_MANIFEST}"),
        "{}",
    );
    write(
        base,
        &format!(".local/sunrise-client/26.3/{PREPARATION_STAMP}"),
        r#"{"output_version":1}"#,
    );
    for report in RUNTIME_REPORTS {
        write(base, &format!(".local/sunrise-client/26.3/{report}"), "{}");
    }
    {
        let preparation = prepare_lock(base).unwrap();
        crate::managed_play::Resources::prepare(base, &preparation, None).unwrap();
    }
    let receipt = version_dir(base).join(".local/launcher-prepared/current.json");
    let receipt_bytes = fs::read(&receipt).unwrap();
    let old_mtime = std::time::SystemTime::now() - std::time::Duration::from_secs(120);
    fs::OpenOptions::new()
        .write(true)
        .open(&receipt)
        .unwrap()
        .set_modified(old_mtime)
        .unwrap();
    let stamped_mtime = fs::metadata(&receipt).unwrap().modified().unwrap();

    // Native client build shares the base's resources (`copy_base` copies research-lock.json and
    // sunrise-prepare sources but omits `.local`).
    seed_common(native);
    write(
        native,
        crate::native_mods::RECORD,
        &serde_json::json!({
            "base": base,
            "sunrise": "a".repeat(40),
            "target": "client",
            "mods": [],
        })
        .to_string(),
    );

    let before = (crate::storage::walks(), crate::storage::entries_calls());
    let reused = reuse_prepared_everywhere();
    assert!(reused.is_empty(), "{reused:?}");
    assert!(
        !version_dir(native).join(".local").exists(),
        "reuse_prepared_everywhere must not copy .local into a native build"
    );
    assert_eq!(fs::read(&receipt).unwrap(), receipt_bytes);
    assert_eq!(
        fs::metadata(&receipt).unwrap().modified().unwrap(),
        stamped_mtime,
        "sealed base must not be re-sealed"
    );
    assert_eq!(
        (crate::storage::walks(), crate::storage::entries_calls()),
        before,
        "desktop startup must perform zero storage tree walks"
    );

    // A native build that still carries a leftover `.local` from an older launcher is never used
    // as a donor for an unprepared base either.
    let donor_root = tempfile::tempdir().unwrap();
    let donor_versions = donor_root.path();
    let write_at = |tag: &str, path: &str, text: &str| {
        let path = donor_versions.join(tag).join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    for tag in [native, "dev-2026-11-27-target"] {
        write_at(tag, COMPLETE, "");
        write_at(tag, "research-lock.json", lock);
        write_at(tag, "crates/sunrise-prepare/Cargo.toml", "[package]");
        write_at(tag, "crates/sunrise-prepare/build.rs", "fn main() {}");
        write_at(
            tag,
            "crates/sunrise-prepare/src/lib.rs",
            "pub const OUTPUT_VERSION: u32 = 1;\n",
        );
    }
    write_at(
        native,
        crate::native_mods::RECORD,
        &serde_json::json!({
            "base": base,
            "sunrise": "a".repeat(40),
            "target": "client",
            "mods": [],
        })
        .to_string(),
    );
    write_at(
        native,
        &format!(".local/sunrise-client/26.3/{RESOURCES_MANIFEST}"),
        "{}",
    );
    write_at(
        native,
        &format!(".local/sunrise-client/26.3/{PREPARATION_STAMP}"),
        r#"{"output_version":1}"#,
    );
    assert_eq!(
        reuse_prepared(donor_versions, "dev-2026-11-27-target"),
        None
    );
    assert!(reuse_prepared_in(donor_versions).is_empty());

    // Runtime-report repair also skips resource-borrowing native builds even if they carry
    // leftover `.local` files or datagen reports.
    let report_target = "dev-2026-11-27-report-target";
    write_at(report_target, COMPLETE, "");
    write_at(report_target, "research-lock.json", lock);
    write_at(
        report_target,
        &format!(".local/sunrise-client/26.3/{RESOURCES_MANIFEST}"),
        "{}",
    );
    write_at(
        report_target,
        ".local/sunrise-client/26.3/registries.json",
        "{}",
    );
    write_at(
        native,
        ".local/sunrise-client/26.3/commands.json",
        r#"{"from":"native"}"#,
    );
    write_at(
        native,
        ".local/datagen/out/reports/commands.json",
        r#"{"from":"native-datagen"}"#,
    );
    assert!(!complete_reports(donor_versions, report_target));
    assert!(
        !donor_versions
            .join(report_target)
            .join(".local/sunrise-client/26.3/commands.json")
            .exists(),
        "complete_reports must not read reports from a resource-borrowing native build"
    );
}

#[test]
fn startup_does_not_lock_or_write_a_complete_base_even_while_preparing_lock_is_held() {
    const CHILD: &str = "SUNRISE_STARTUP_LOCKED_BASE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let home = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "backend::tests::startup_does_not_lock_or_write_a_complete_base_even_while_preparing_lock_is_held",
            ])
            .env(CHILD, "1")
            .env("SUNRISE_LAUNCHER_HOME", home.path().join("launcher"))
            .env("SUNRISE_HOME", home.path().join("game"))
            .env("XDG_DATA_HOME", home.path().join("xdg-data"))
            .env("XDG_CONFIG_HOME", home.path().join("xdg-config"))
            .env("SUNRISE_LAUNCHER_LOCATION_FILE", home.path().join("location"))
            .env("SUNRISE_LAUNCHER_TRASH", home.path().join("trash"))
            .env("SUNRISE_LAUNCHER_CLIPBOARD", home.path().join("clipboard"))
            .env("SUNRISE_LAUNCHER_DESKTOP", home.path().join("desktop"))
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("SUNRISE_NO_AUDIO", "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }

    let base = "dev-2026-11-26-ba1f24e";
    let dir = version_dir(base);
    let write = |path: &str, text: &str| {
        let path = dir.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write(COMPLETE, "");
    write(
        "research-lock.json",
        r#"{"minecraft":{"version":"26.3","client_sha1":"c","server_sha1":"s"}}"#,
    );
    write("crates/sunrise-prepare/Cargo.toml", "[package]");
    write("crates/sunrise-prepare/build.rs", "fn main() {}");
    write(
        "crates/sunrise-prepare/src/lib.rs",
        "pub const OUTPUT_VERSION: u32 = 1;\n",
    );
    write(
        &format!(".local/sunrise-client/26.3/{RESOURCES_MANIFEST}"),
        "{}",
    );
    write(
        &format!(".local/sunrise-client/26.3/{PREPARATION_STAMP}"),
        r#"{"output_version":1}"#,
    );
    for report in RUNTIME_REPORTS {
        write(&format!(".local/sunrise-client/26.3/{report}"), "{}");
    }
    {
        let preparation = prepare_lock(base).unwrap();
        crate::managed_play::Resources::prepare(base, &preparation, None).unwrap();
    }
    let receipt = dir.join(".local/launcher-prepared/current.json");
    let receipt_bytes = fs::read(&receipt).unwrap();
    let old_mtime = std::time::SystemTime::now() - std::time::Duration::from_secs(120);
    fs::OpenOptions::new()
        .write(true)
        .open(&receipt)
        .unwrap()
        .set_modified(old_mtime)
        .unwrap();
    let stamped_mtime = fs::metadata(&receipt).unwrap().modified().unwrap();

    #[cfg(unix)]
    let (probe_log, mutated_marker) = {
        use std::os::unix::fs::PermissionsExt;
        let probe_log = data_dir().join("probe.log");
        let mutated = data_dir().join("mutated");
        let binary = dir.join(crate::preparer::BINARY);
        fs::write(
            &binary,
            format!(
                "#!/bin/sh\nset -eu\ncmd=$1\nprintf '%s\\n' \"$cmd\" >> \"{}\"\nif [ \"$cmd\" = missing-extras ]; then\n  printf '%s\\n' '{{\"format\":1,\"missing\":[]}}'\nelse\n  : > \"{}\"\n  exit 2\nfi\n",
                probe_log.display(),
                mutated.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
        (probe_log, mutated)
    };

    let held = prepare_lock(base).unwrap();
    let before = (crate::storage::walks(), crate::storage::entries_calls());
    let reused = reuse_prepared_everywhere();
    assert!(reused.is_empty(), "{reused:?}");
    assert_eq!(fs::read(&receipt).unwrap(), receipt_bytes);
    assert_eq!(
        fs::metadata(&receipt).unwrap().modified().unwrap(),
        stamped_mtime
    );
    assert_eq!(
        (crate::storage::walks(), crate::storage::entries_calls()),
        before
    );
    #[cfg(unix)]
    {
        assert!(
            !mutated_marker.exists(),
            "startup ran a mutating preparer step"
        );
        assert_eq!(
            fs::read_to_string(&probe_log)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            ["missing-extras"],
            "Pass 2 should probe missing-extras once, unlocked"
        );
    }
    // need_prepared keeps its non-blocking probe while a real writer holds .preparing.lock.
    let error = need_prepared(base).unwrap_err();
    assert!(error.contains("already being prepared"), "{error}");
    drop(held);
    need_prepared(base).unwrap();
}

#[cfg(unix)]
#[test]
fn startup_probes_missing_extras_once_even_when_repair_runs() {
    use std::os::unix::fs::PermissionsExt;
    const CHILD: &str = "SUNRISE_STARTUP_REPAIR_PROBE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let home = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "backend::tests::startup_probes_missing_extras_once_even_when_repair_runs",
            ])
            .env(CHILD, "1")
            .env("SUNRISE_LAUNCHER_HOME", home.path().join("launcher"))
            .env("SUNRISE_HOME", home.path().join("game"))
            .env("XDG_DATA_HOME", home.path().join("xdg-data"))
            .env("XDG_CONFIG_HOME", home.path().join("xdg-config"))
            .env(
                "SUNRISE_LAUNCHER_LOCATION_FILE",
                home.path().join("location"),
            )
            .env("SUNRISE_LAUNCHER_TRASH", home.path().join("trash"))
            .env("SUNRISE_LAUNCHER_CLIPBOARD", home.path().join("clipboard"))
            .env("SUNRISE_LAUNCHER_DESKTOP", home.path().join("desktop"))
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("SUNRISE_NO_AUDIO", "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }

    let direct = "dev-2026-11-26-direct";
    let reused_target = "dev-2026-11-27-reused";
    let lock = r#"{"minecraft":{"version":"26.3","client_sha1":"c","server_sha1":"s"}}"#;
    let write = |tag: &str, path: &str, text: &str| {
        let path = version_dir(tag).join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    let install_preparer = |tag: &str, log_path: &Path| {
        let binary = version_dir(tag).join(crate::preparer::BINARY);
        fs::write(
            &binary,
            format!(
                "#!/bin/sh\nset -eu\ncmd=$1\nprintf '%s\\n' \"$cmd\" >> \"{}\"\ncase \"$cmd\" in\n  missing-extras) printf '%s\\n' '{{\"format\":1,\"missing\":[{{\"kind\":\"sounds\",\"what\":\"sounds\"}}]}}';;\n  extras) printf '%s\\n' '{{\"format\":1,\"added\":[\"sounds\"]}}';;\n  data-file) printf '%s\\n' '{{\"format\":1,\"written\":false}}';;\n  *) exit 2;;\nesac\n",
                log_path.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    };
    for tag in [direct, reused_target] {
        write(tag, COMPLETE, "");
        write(tag, "research-lock.json", lock);
        write(tag, "crates/sunrise-prepare/Cargo.toml", "[package]");
        write(tag, "crates/sunrise-prepare/build.rs", "fn main() {}");
        write(
            tag,
            "crates/sunrise-prepare/src/lib.rs",
            "pub const OUTPUT_VERSION: u32 = 1;\n",
        );
    }
    write(
        direct,
        &format!(".local/sunrise-client/26.3/{RESOURCES_MANIFEST}"),
        "{}",
    );
    write(
        direct,
        &format!(".local/sunrise-client/26.3/{PREPARATION_STAMP}"),
        r#"{"output_version":1}"#,
    );
    for report in RUNTIME_REPORTS {
        write(
            direct,
            &format!(".local/sunrise-client/26.3/{report}"),
            "{}",
        );
    }
    let direct_log = data_dir().join("direct.log");
    let reused_log = data_dir().join("reused.log");
    install_preparer(direct, &direct_log);
    install_preparer(reused_target, &reused_log);

    let reused = reuse_prepared_everywhere();
    assert_eq!(reused, [(reused_target.to_string(), direct.to_string())]);
    assert_eq!(
        fs::read_to_string(&direct_log)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["missing-extras", "extras"],
        "direct repair must feed the unlocked missing-extras probe without probing again under lock"
    );
    assert_eq!(
        fs::read_to_string(&reused_log)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["missing-extras", "extras", "data-file"],
        "finish_preparation must feed the unlocked missing-extras probe without probing again under lock"
    );
}
