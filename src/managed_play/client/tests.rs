use super::*;
use serde_json::json;
use std::{fs, sync::Arc};
use sunrise_launcher_smod::{index, index_v2, manifest, mod_list};

/// A signed v2 catalogue that lists nothing.
fn empty_index() -> index_v2::Index {
    serde_json::from_value(json!({
        "schema": 2, "serial": 1, "generated": "2026-11-24T00:00:00Z", "builds": [],
        "categories": [], "projects": [], "revoked": []
    }))
    .unwrap()
}

fn recovery_mod(home: &Path, id: &str, name: &str, native: bool, dependencies: &str) {
    let body = format!(
        "id='{id}'\nname='{name}'\nversion='1.0.1'\napi='{}'\n{dependencies}",
        manifest::API
    );
    let body = if native {
        format!(
            "{body}\n[native]\ngame='26.3'\nsunrise='{}'\n",
            "a".repeat(40)
        )
    } else {
        body
    };
    let cargo = format!("[package]\nname='{id}'\nversion='1.0.1'\n");
    let files = [
        ("Cargo.toml", cargo.as_bytes()),
        ("src/lib.rs", &b"fn fixture() {}"[..]),
    ];
    let bytes = manifest::Package::write_files(&body, if native { &files } else { &[] }).unwrap();
    let folder = home.join(if native {
        mod_list::NATIVE_DIR
    } else {
        mod_list::DIR
    });
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join(format!("{id}.smod")), bytes).unwrap();
}

fn off() -> Vec<(String, String)> {
    vec![("blur".into(), "1.0.1".into())]
}

#[test]
fn recovery_refuses_a_required_dependent_but_keeps_an_optional_one() {
    let scratch = tempfile::tempdir().unwrap();
    let home = scratch.path();
    recovery_mod(home, "blur", "Motion Blur", true, "");
    recovery_mod(home, "map", "Map", false, "[dependencies]\nblur='^1'\n");
    let list = home.join(mod_list::FILE);
    let original = b"mod.blur:true\r\nmod.map:true\r\nkey.blur.toggle:key.keyboard.x";
    fs::write(&list, original).unwrap();
    let error = turn_off_mods(
        home,
        &off(),
        &capture_native_digests(home),
        |_, _| unreachable!(),
    )
    .unwrap_err();
    assert_eq!(error, "Map needs Motion Blur. No mods were turned off.");
    assert_eq!(fs::read(&list).unwrap(), original);

    for declaration in [
        "{version='^1', optional=true}",
        "{version='^1', side='server'}",
    ] {
        recovery_mod(
            home,
            "map",
            "Map",
            false,
            &format!("[dependencies]\nblur={declaration}\n"),
        );
        turn_off_mods(
            home,
            &off(),
            &capture_native_digests(home),
            |mut file, bytes| file.write_all(bytes),
        )
        .unwrap();
        assert_eq!(
            fs::read(&list).unwrap(),
            b"mod.map:true\r\nkey.blur.toggle:key.keyboard.x"
        );
        fs::write(&list, original).unwrap();
    }
    assert!(turn_off_mods(
        home,
        &[("blur".into(), "1.0.0".into())],
        &capture_native_digests(home),
        |_, _| unreachable!()
    )
    .unwrap_err()
    .contains("No mods were turned off"));
    assert_eq!(fs::read(&list).unwrap(), original);

    // Replacing a local archive with different bytes at the same version must be refused.
    let digests_before = capture_native_digests(home);
    recovery_mod(home, "blur", "Motion Blur (replaced)", true, "");
    let replaced_err =
        turn_off_mods(home, &off(), &digests_before, |_, _| unreachable!()).unwrap_err();
    assert_eq!(
        replaced_err,
        "The enabled mods changed. No mods were turned off."
    );
    assert_eq!(fs::read(&list).unwrap(), original);
}

fn write_recovery_world(home: &Path, saved_mod: &str) {
    use sunrise_launcher_nbt::{self as nbt, Limits, Tag};
    let dir = home.join("saves/world");
    fs::create_dir_all(&dir).unwrap();
    let root = Tag::compound([
        (
            "Data",
            Tag::compound([("LevelName", Tag::String("Lantern Lake".into()))]),
        ),
        (
            "sunrise",
            Tag::compound([(
                "ContentModList",
                Tag::List(vec![Tag::compound([
                    ("ModId", Tag::String(saved_mod.into())),
                    ("ModVersion", Tag::String("1.0.1".into())),
                ])]),
            )]),
        ),
    ]);
    fs::write(
        dir.join("level.dat"),
        nbt::write_named(
            &nbt::NamedTag {
                name: "".into(),
                tag: root,
            },
            Limits::default(),
        )
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn recovery_refuses_a_saved_world_and_never_begins_play() {
    const TEST: &str =
        "managed_play::client::tests::recovery_refuses_a_saved_world_and_never_begins_play";
    const FIXTURE: &str = "PLAY_SAVED_WORLD_RECOVERY_FIXTURE";
    const BUILD: &str = "v0.1.2";
    if std::env::var_os(FIXTURE).is_none() {
        let root = tempfile::tempdir().unwrap();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(FIXTURE, "1")
            .env("SUNRISE_HOME", root.path().join("game"))
            .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
            .env(
                "SUNRISE_LAUNCHER_LOCATION_FILE",
                root.path().join("location")
            )
            .env("SUNRISE_NO_KEYCHAIN", "1")
            .status()
            .unwrap()
            .success());
        return;
    }
    let home = backend::game_home();
    let list = home.join(mod_list::FILE);
    let original = b"mod.blur:true\nkey.blur.toggle:key.keyboard.x\n";
    recovery_mod(&home, "blur", "Motion Blur", true, "");
    fs::write(&list, original).unwrap();
    write_recovery_world(&home, "blur");
    let install = backend::version_dir(BUILD);
    fs::create_dir_all(&install).unwrap();
    fs::write(install.join(".installed"), []).unwrap();
    fs::write(install.join(backend::CLIENT), b"not a game").unwrap();
    let instance = instances::selection().unwrap();
    let recover = || {
        Client {
            instance: &instance,
            accepted_account: Some("fixture"),
            explicit_account: false,
            piped: false,
            registry: || None,
        }
        .start_with_recovery(
            Some(ModRecovery {
                build: Arc::new(FailedBuild {
                    build: Build::First(FirstBuild::installed(BUILD).unwrap()),
                    digests: capture_native_digests(&home),
                }),
                mods: off(),
            }),
            || FirstBuild::installed(BUILD),
            |_| unreachable!("recovery must not start a game"),
            |_| unreachable!(),
            &native_mods::Cancellation::default(),
            |_| unreachable!("recovery must stop before preparing or starting the game"),
        )
    };
    assert!(
        matches!(recover(), Err(StartError::Other(detail)) if detail ==
        "Turning off these mods could remove content from: Lantern Lake. No mods were turned off.")
    );
    assert_eq!(fs::read(&list).unwrap(), original);
    // A broken roster is also a refusal, not proof that no world is at risk.
    fs::remove_dir_all(home.join("saves")).unwrap();
    fs::write(home.join("saves"), b"not a directory").unwrap();
    assert!(
        matches!(recover(), Err(StartError::Other(detail)) if detail ==
        "Sunrise couldn't check the saved worlds. No mods were turned off.")
    );
    assert_eq!(fs::read(&list).unwrap(), original);
}

#[test]
fn recovery_proceeds_in_a_profile_with_no_saved_worlds() {
    let scratch = tempfile::tempdir().unwrap();
    let home = scratch.path();
    let list = home.join(mod_list::FILE);
    recovery_mod(home, "blur", "Motion Blur", true, "");
    fs::write(&list, b"mod.blur:true\nmod.map:true\n").unwrap();
    turn_off_mods(
        home,
        &off(),
        &capture_native_digests(home),
        |mut file, bytes| file.write_all(bytes),
    )
    .unwrap();
    assert_eq!(fs::read(&list).unwrap(), b"mod.map:true\n");
}

#[test]
fn recovery_refuses_a_save_whose_last_load_did_not_list_the_mod() {
    // level.dat names only another mod, but level.dat_old and older chunks can still hold this
    // one: a last-loaded list never rules a mod out.
    let scratch = tempfile::tempdir().unwrap();
    let home = scratch.path();
    let list = home.join(mod_list::FILE);
    write_recovery_world(home, "blur");
    let world = home.join("saves/world");
    fs::rename(world.join("level.dat"), world.join("level.dat_old")).unwrap();
    write_recovery_world(home, "map");
    recovery_mod(home, "blur", "Motion Blur", true, "");
    let original = b"mod.blur:true\nmod.map:true\n";
    fs::write(&list, original).unwrap();
    let error = turn_off_mods(home, &off(), &capture_native_digests(home), |_, _| {
        unreachable!("nothing is written")
    })
    .unwrap_err();
    assert_eq!(
        error,
        "Turning off these mods could remove content from: Lantern Lake. No mods were turned off."
    );
    assert_eq!(fs::read(&list).unwrap(), original);
}

#[test]
fn a_failed_enable_list_write_does_not_replace_the_original() {
    let scratch = tempfile::tempdir().unwrap();
    let home = scratch.path();
    let list = home.join(mod_list::FILE);
    let original = b"mod.blur:true\r\nkey.blur.toggle:key.keyboard.x\r\n";
    recovery_mod(home, "blur", "Motion Blur", true, "");
    fs::write(&list, original).unwrap();
    let error = turn_off_mods(
        home,
        &off(),
        &capture_native_digests(home),
        |mut file, bytes| {
            file.write_all(&bytes[..10])?;
            Err(io::Error::new(
                io::ErrorKind::StorageFull,
                "fixture disk full",
            ))
        },
    )
    .unwrap_err();
    assert!(error.contains("fixture disk full"));
    assert_eq!(fs::read(&list).unwrap(), original);
    assert_eq!(
        fs::read_dir(home).unwrap().count(),
        2,
        "temporary file removed"
    );
}

#[cfg(unix)]
#[test]
fn recovery_does_not_follow_or_replace_a_linked_enable_list() {
    let scratch = tempfile::tempdir().unwrap();
    let home = scratch.path();
    let list = home.join(mod_list::FILE);
    let target = home.join("target-list");
    let original = b"mod.blur:true\n";
    recovery_mod(home, "blur", "Motion Blur", true, "");
    fs::write(&target, original).unwrap();
    std::os::unix::fs::symlink(&target, &list).unwrap();
    assert!(turn_off_mods(
        home,
        &off(),
        &capture_native_digests(home),
        |_, _| unreachable!()
    )
    .is_err());
    assert_eq!(fs::read_link(&list).unwrap(), target);
    assert_eq!(fs::read(&target).unwrap(), original);
}

#[test]
fn recovery_binds_first_play_and_refuses_a_result_from_before_switch() {
    const TEST: &str = "managed_play::client::tests::recovery_binds_first_play_and_refuses_a_result_from_before_switch";
    const FIXTURE: &str = "PLAY_TURN_OFF_FIXTURE";
    const BEFORE: &str = "v0.1.1";
    const AFTER: &str = "v0.1.2";
    if std::env::var_os(FIXTURE).is_none() {
        let root = tempfile::tempdir().unwrap();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(FIXTURE, "1")
            .env("SUNRISE_HOME", root.path().join("game"))
            .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
            .env(
                "SUNRISE_LAUNCHER_LOCATION_FILE",
                root.path().join("location")
            )
            .env("SUNRISE_NO_KEYCHAIN", "1")
            .status()
            .unwrap()
            .success());
        return;
    }
    for tag in [BEFORE, AFTER] {
        fs::create_dir_all(backend::resources(tag)).unwrap();
        fs::write(
            backend::resources(tag).join("sunrise-resources.json"),
            b"{}",
        )
        .unwrap();
        fs::write(backend::version_dir(tag).join(".installed"), []).unwrap();
        fs::write(backend::version_dir(tag).join(backend::CLIENT), tag).unwrap();
    }
    let home = backend::game_home();
    let list = home.join(mod_list::FILE);
    let original = b"mod.blur:true\n";
    recovery_mod(&home, "blur", "Motion Blur", true, "");
    fs::write(&list, original).unwrap();
    assert!(crate::native_status::build_mods(&home)
        .iter()
        .any(|m| m.id == "blur" && m.title == "Motion Blur"));
    let instance = instances::selection().unwrap();
    let cancelled = native_mods::Cancellation::default();
    let recover = |selection: Build| ModRecovery {
        build: Arc::new(FailedBuild {
            build: selection,
            digests: capture_native_digests(&home),
        }),
        mods: off(),
    };
    let refuses = |recovery, tag| {
        let result = Client {
            instance: &instance,
            accepted_account: Some("fixture"),
            explicit_account: false,
            piped: false,
            registry: || None,
        }
        .start_with_recovery(
            Some(recovery),
            || FirstBuild::installed(tag),
            |_| unreachable!("stale result cannot start a game"),
            |_| unreachable!(),
            &cancelled,
            |_| {},
        );
        assert!(
            matches!(result, Err(StartError::Other(detail)) if detail == "The profile's game build changed. No mods were turned off.")
        );
        assert_eq!(fs::read(&list).unwrap(), original);
    };
    let first = {
        let mut profile = Profile::open(&home, None).unwrap();
        let attempt = profile
            .begin_first(|| FirstBuild::installed(BEFORE), |_| {})
            .unwrap();
        let first = attempt.recovery_selection().unwrap();
        drop(attempt);
        profile.release_failed_first_play().unwrap();
        first
    };
    refuses(recover(Build::First(first)), AFTER);

    let active = {
        let mut profile = Profile::open(&home, None).unwrap();
        let recovery = recover(Build::First(FirstBuild::installed(BEFORE).unwrap()));
        let selected = {
            let _manifest = profile.lock_inputs().unwrap();
            let selected = recovery
                .verify(&profile, &mut Some(|| FirstBuild::installed(BEFORE)))
                .unwrap()
                .unwrap();
            turn_off_mods(
                &home,
                &recovery.mods,
                &recovery.build.digests,
                |mut file, bytes| file.write_all(bytes),
            )
            .unwrap();
            selected
        };
        assert!(fs::read(&list).unwrap().is_empty());
        let active = profile
            .begin_first(|| Ok(selected), |_| {})
            .unwrap()
            .commit(BEFORE, &cancelled)
            .unwrap();
        let switch = profile.begin_switch(AFTER).unwrap().unwrap();
        let candidate = switch.candidate().unwrap();
        let proof = switch
            .prove(
                candidate.path(),
                native_mods::Profile::Optimized,
                &cancelled,
                |_| {},
                None,
            )
            .unwrap();
        let switched = switch
            .publish_switch(
                proof,
                &cancelled,
                crate::managed_play::SwitchPublication::default(),
                || Ok(()),
                || Ok(()),
            )
            .unwrap();
        assert_eq!(switched.base(), AFTER);
        active
    };
    fs::write(&list, original).unwrap();
    refuses(recover(Build::Active(active)), AFTER);
}

#[test]
fn play_checks_hash_only_takedowns_after_proving_the_actual_package() {
    const TEST: &str = "managed_play::client::tests::play_checks_hash_only_takedowns_after_proving_the_actual_package";
    const BUILD: &str = "v0.1.7";
    if std::env::var_os("PLAY_CATALOGUE_FIXTURE").is_none() {
        let root = tempfile::tempdir().unwrap();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env("PLAY_CATALOGUE_FIXTURE", "1")
            .env("SUNRISE_HOME", root.path().join("game"))
            .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .status()
            .unwrap()
            .success());
        return;
    }
    let home = backend::game_home();
    let build = backend::version_dir(BUILD);
    let prepared = backend::resources(BUILD);
    fs::create_dir_all(&prepared).unwrap();
    fs::write(prepared.join("sunrise-resources.json"), b"{}").unwrap();
    fs::write(build.join(".installed"), []).unwrap();
    fs::write(build.join(backend::CLIENT), "unchanged client").unwrap();
    let row_path = home.join(sunrise_launcher_smod::index::browser_state::MANIFEST);
    fs::create_dir_all(row_path.parent().unwrap()).unwrap();
    let relative = format!("{}/lantern.smod", mod_list::DIR);
    let package = home.join(&relative);
    fs::create_dir_all(package.parent().unwrap()).unwrap();
    let write_package = |version| {
        let body = format!("id='lantern'\nname='Lantern'\nversion='{version}'\napi='{}'\ncapabilities=['registries']\n", manifest::API);
        fs::write(
            &package,
            manifest::Package::write_files(&body, &[]).unwrap(),
        )
        .unwrap();
        index::sha256_file(&package).unwrap()
    };
    let old = write_package("1.0.0");
    fs::write(
        &row_path,
        json!({"entries":[{
            "source":"sunrise", "project":"lantern", "slug":"lantern", "title":"Lantern",
            "kind":"mod", "version":"1.0.0", "number":"1.0.0", "file":relative,
            "checksum":format!("SHA-256:{old}"), "page":"", "dependency":false,
        }]})
        .to_string(),
    )
    .unwrap();
    fs::write(home.join(mod_list::FILE), b"mod.lantern:true\n").unwrap();
    let recorded = fs::read_to_string(&row_path).unwrap();
    write_package("2.0.0");
    let instance = instances::selection().unwrap();
    let client = Client {
        instance: &instance,
        accepted_account: None,
        explicit_account: false,
        piped: false,
        registry: || None,
    };
    let mut profile = Profile::open(&home, None).unwrap();
    let cancellation = native_mods::Cancellation::default();
    let revoked = |project: &str, hash| {
        let mut result = empty_index();
        result.revoked.push(index_v2::Revoked {
            project: project.into(),
            sha256: Some(hash),
            reason: "hash-only takedown fixture".into(),
            date: String::new(),
        });
        Some(Arc::new(result))
    };
    let (active, _) = client
        .ready(
            &mut profile,
            || FirstBuild::installed(BUILD),
            |_, _| Ok(()),
            &cancellation,
            |_| {},
            || {
                let binding = crate::profile_txn::references(&home).unwrap();
                assert!(binding.targets.contains(BUILD));
                assert!(binding.pointer.is_none());
                // A hash-only takedown applies whichever project lists it.
                revoked("Ab3dE9xQ", old)
            },
        )
        .unwrap();
    // The old hash-only takedown does not deny the current input. Neither it nor
    // the current archive's ID and path can turn those new bytes into registry origin.
    assert_eq!(fs::read_to_string(&row_path).unwrap(), recorded);

    let replaced = write_package("3.0.0");
    let error = client
        .ready(
            &mut profile,
            || panic!("already bound"),
            |_, _| Ok(()),
            &cancellation,
            |_| {},
            || revoked("Ab3dE9xQ", replaced),
        )
        .expect_err("actual archive was taken down");
    assert!(String::from(error).contains("hash-only takedown fixture"));
    assert_eq!(profile.active().unwrap(), Some(active.clone()));
    assert_eq!(fs::read_to_string(&row_path).unwrap(), recorded);

    // A package with no browser row can still be a taken-down compiler input.
    let native = home.join(mod_list::NATIVE_DIR).join("native_lantern.smod");
    fs::create_dir_all(native.parent().unwrap()).unwrap();
    let body = format!("id='native_lantern'\nname='Native lantern'\nversion='1.0.0'\napi='{}'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{}'\n", manifest::API, "a".repeat(40));
    fs::write(
        &native,
        manifest::Package::write_files(
            &body,
            &[
                (
                    "Cargo.toml",
                    b"[package]\nname='native_lantern'\nversion='1.0.0'\n",
                ),
                ("src/lib.rs", b"fn fixture() {}"),
            ],
        )
        .unwrap(),
    )
    .unwrap();
    assert!(crate::native_status::enabled(&home).is_empty());
    fs::write(
        home.join(mod_list::FILE),
        b"mod.lantern:true\nmod.native_lantern:true\n",
    )
    .unwrap();
    assert_eq!(crate::native_status::enabled(&home).len(), 1);
    let mut listed = empty_index();
    listed.revoked.push(index_v2::Revoked {
        project: "native_lantern".into(),
        sha256: None,
        reason: "only the listed project was taken down".into(),
        date: String::new(),
    });
    assert!(
        catalogue::inspect(&home, || Some(Arc::new(listed.clone())))
            .unwrap()
            .is_empty(),
        "an ID-wide takedown is not evidence against a local native archive"
    );
    let error = client
        .ready(
            &mut profile,
            || panic!("already bound"),
            |_, _| Ok(()),
            &cancellation,
            |step| assert!(!matches!(step, Progress::Native(_)), "started native work"),
            || {
                let hash = index::sha256_file(&native).unwrap().to_ascii_uppercase();
                listed.revoked.push(index_v2::Revoked {
                    project: "Nl4nTern".into(),
                    sha256: Some(hash),
                    reason: "exact native digest was taken down".into(),
                    date: String::new(),
                });
                Some(Arc::new(listed))
            },
        )
        .expect_err("native input was taken down");
    let error = String::from(error);
    assert!(error.contains("Native lantern was taken down"), "{error}");
    assert!(
        error.contains("exact native digest was taken down"),
        "{error}"
    );
    assert_eq!(profile.active().unwrap(), Some(active));
}

#[cfg(target_os = "linux")]
#[test]
fn a_failed_first_download_can_be_followed_by_a_mod_change_and_play() {
    use crate::profile_txn::{Admission, Bootstrap};
    use std::io::{Read, Write};
    use std::os::unix::fs::PermissionsExt;

    const TEST: &str = concat!(
        module_path!(),
        "::a_failed_first_download_can_be_followed_by_a_mod_change_and_play"
    );
    const BUILD: &str = "developer-download-recovery";
    const UNAVAILABLE: &str = "26.3-0.1.7";
    if std::env::var_os("PLAY_DOWNLOAD_FIXTURE").is_none() {
        let root = tempfile::tempdir().unwrap();
        // No registry answers: Play goes on without one when no installed mod needs it.
        let registry = root.path().join("no-index.signed");
        // libtest names omit the crate prefix included by module_path!().
        let test = TEST.split_once("::").unwrap().1;
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--nocapture"])
            .env("PLAY_DOWNLOAD_FIXTURE", "1")
            .env("SUNRISE_HOME", root.path().join("game"))
            .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
            .env("SUNRISE_LAUNCHER_DEVELOPER_OFFLINE", "1")
            .env("SUNRISE_MOD_INDEX_V2", registry)
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let passed = stdout
            .lines()
            .find(|line| line.starts_with("test result: ok. 1 passed;"));
        assert!(
            output.status.success() && passed.is_some(),
            "{}\n{}",
            stdout,
            String::from_utf8_lossy(&output.stderr)
        );
        println!("Isolated download fixture: {}", passed.unwrap());
        return;
    }

    let home = backend::game_home();
    fs::create_dir_all(home.join(mod_list::DIR)).unwrap();
    let manifest = format!(
        "id='lantern'\nname='Lantern'\nversion='1.0.0'\napi='{}'\ncapabilities=['registries']\n",
        manifest::API
    );
    fs::write(
        home.join(mod_list::DIR).join("lantern.smod"),
        manifest::Package::write_files(&manifest, &[]).unwrap(),
    )
    .unwrap();
    fs::write(home.join(mod_list::FILE), b"mod.lantern:true\n").unwrap();
    let asset = backend::ASSET.unwrap();
    let archive = backend::installation::GameDownload::select(&backend::Release {
        tag: UNAVAILABLE.into(),
        name: String::new(),
        body: String::new(),
        date: String::new(),
        time: String::new(),
        published_at: String::new(),
        asset: Some((crate::signed::asset_url(UNAVAILABLE, asset), 42)),
        prerelease: false,
        nightly: None,
    })
    .unwrap();
    // Resume an already-verified choice and fail the actual archive request before staging.
    let mut ticket = Admission::open(&home, None).unwrap();
    ticket
        .bind_bootstrap(
            || {
                Ok(Bootstrap::new(
                    UNAVAILABLE.into(),
                    UNAVAILABLE.into(),
                    [1; 16],
                    super::super::inputs(&home)?,
                    json!({"kind":"download", "archive":archive}),
                ))
            },
            |_| Ok(()),
        )
        .unwrap();
    ticket
        .prove_bootstrap(|_| Ok(json!({"archive":"a".repeat(64),"facts":null})))
        .unwrap();
    drop(ticket);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    crate::signed::TEST_GAME_ASSET_ORIGIN
        .set(Some(format!("http://{}", listener.local_addr().unwrap())));
    let http = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 4096];
        let count = stream.read(&mut request).unwrap();
        assert!(String::from_utf8_lossy(&request[..count])
            .starts_with(&format!("GET /{UNAVAILABLE}/{asset} ")));
        stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
    });
    let instance = instances::selection().unwrap();
    let client = || Client {
        instance: &instance,
        accepted_account: None,
        explicit_account: false,
        piped: false,
        registry: || None,
    };
    let stop = native_mods::Cancellation::default();
    let Err(error) = client().start(
        || panic!("resuming a saved choice"),
        |_| Ok(()),
        |_| Ok(()),
        &stop,
        |_| {},
    ) else {
        panic!("the unavailable download launched");
    };
    crate::signed::TEST_GAME_ASSET_ORIGIN.set(None);
    assert!(String::from(error).contains("404"));
    http.join().unwrap();
    assert!(!home.join(crate::profile_txn::POINTER).exists());
    assert!(crate::profile_txn::references(&home)
        .unwrap()
        .targets
        .is_empty());

    fs::write(home.join(mod_list::FILE), []).unwrap();
    let base = backend::version_dir(BUILD);
    let resources = backend::resources(BUILD);
    fs::create_dir_all(&resources).unwrap();
    fs::write(resources.join("sunrise-resources.json"), b"{}").unwrap();
    fs::write(base.join(".installed"), []).unwrap();
    fs::write(
        base.join("developer.json"),
        json!({"revision": "b".repeat(40)}).to_string(),
    )
    .unwrap();
    let executable = base.join(backend::CLIENT);
    fs::write(&executable, "#!/bin/sh\nif [ \"$1\" = --launch-api ]; then\n  echo '{\"options\":[{\"name\":\"--developer-single-player\"}]}'\nelse\n  touch \"$SUNRISE_HOME/started\"\nfi\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let mut started = client()
        .start(
            || FirstBuild::installed(BUILD),
            |_| Ok(()),
            |_| Ok(()),
            &stop,
            |_| {},
        )
        .unwrap();
    assert!(started.child.wait().unwrap().success());
    assert_eq!(started.executable, BUILD);
    assert!(home.join("started").is_file());
}

#[cfg(windows)]
#[test]
fn windows_bound_x64_fallback_survives_pending_retry_and_committed_replay() {
    use crate::{
        backend::GameArchitecture::Arm64,
        profile_txn::{Admission, Bootstrap},
    };
    const TEST: &str = "managed_play::client::tests::windows_bound_x64_fallback_survives_pending_retry_and_committed_replay";
    const BUILD: &str = "26.3-0.1.7";
    if std::env::var_os("PLAY_VARIANT_FIXTURE").is_none() {
        let root = tempfile::tempdir().unwrap();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env("PLAY_VARIANT_FIXTURE", "1")
            .env("SUNRISE_HOME", root.path().join("game"))
            .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
            .status()
            .unwrap()
            .success());
        return;
    }
    let home = backend::game_home();
    fs::create_dir_all(&home).unwrap();
    let directory = backend::version_dir(BUILD);
    let prepared = backend::resources(BUILD);
    fs::create_dir_all(&prepared).unwrap();
    fs::write(prepared.join("sunrise-resources.json"), b"{}").unwrap();
    fs::write(directory.join(".installed"), []).unwrap();
    let receipt = directory.join(super::super::Artifact::ARCHIVE_RECEIPT);
    let archive = "a".repeat(64);
    let install_candidate = |archive: &str, binary| {
        if receipt.is_file() {
            fs::remove_file(&receipt).unwrap();
        }
        fs::write(
            directory.join(backend::CLIENT),
            format!("{binary} fixture image"),
        )
        .unwrap();
        fs::write(
            directory.join(backend::GAME_BUILD_ARCHITECTURE),
            json!({"requested":"x64", "binary":binary}).to_string(),
        )
        .unwrap();
        super::super::Artifact::record_archive(&directory, BUILD, archive).unwrap();
    };
    install_candidate(&archive, "x64");
    assert!(backend::check_game_architecture(BUILD, None, Arm64).is_some());
    let interrupt_after_proof = || {
        let mut admission = Admission::open(&home, None).unwrap();
        admission
            .bind_bootstrap(
                || {
                    Ok(Bootstrap::new(
                        BUILD.into(),
                        BUILD.into(),
                        [1; 16],
                        super::super::inputs(&home).unwrap(),
                        json!({"kind":"download", "archive":{
                            "source":BUILD, "file":"sunrise-windows-x64.zip", "size":0,
                            "requested":"arm64", "time":"",
                        }}),
                    ))
                },
                |_| Ok(()),
            )
            .unwrap();
        admission
            .prove_bootstrap(|_| Ok(json!({"archive":archive, "facts":null})))
            .unwrap();
    };
    interrupt_after_proof();
    let instance = instances::selection().unwrap();
    let client = Client {
        instance: &instance,
        accepted_account: None,
        explicit_account: false,
        piped: false,
        registry: || None,
    };
    let cancellation = native_mods::Cancellation::default();
    let retry = |profile: &mut Profile| {
        client.ready(
            profile,
            || panic!("re-resolved the platform archive"),
            |tag, variant| {
                assert!(
                    variant.is_some(),
                    "fell back to an unbound cache preference"
                );
                backend::check_game_architecture(tag, variant, Arm64)
                    .map_or(Ok(()), |e| Err(e.into()))
            },
            &cancellation,
            |_| {},
            || None,
        )
    };
    let mut profile = Profile::open(&home, None).unwrap();
    install_candidate(&"b".repeat(64), "x64");
    let error = retry(&mut profile).expect_err("accepted an unrelated cached x64 archive");
    assert_eq!(
        String::from(error),
        crate::i18n::trf(
            "A different installation already occupies {build}. It was not replaced.",
            &[("build", BUILD)],
        )
    );
    assert!(profile.active().unwrap().is_none());
    assert!(crate::profile_txn::references(&home)
        .unwrap()
        .targets
        .is_empty());
    drop(profile);

    // A returned refusal releases the choice. Seed a separate interrupted Play to
    // verify that a pending proof (rather than a settled failure) resumes exactly.
    install_candidate(&archive, "x64");
    interrupt_after_proof();
    let mut profile = Profile::open(&home, None).unwrap();
    let (active, _) = retry(&mut profile).unwrap();
    drop(profile);
    let mut reopened = Profile::open(&home, None).unwrap();
    assert_eq!(retry(&mut reopened).unwrap().0, active);
    install_candidate(&archive, "arm64");
    assert!(
        retry(&mut reopened).is_err(),
        "accepted a different runtime binary"
    );
    assert_eq!(reopened.active().unwrap(), Some(active));
}
