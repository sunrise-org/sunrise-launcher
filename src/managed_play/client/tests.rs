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
    let row_path = home.join(crate::browser::installed::MANIFEST);
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
    let (active, _, _) = client
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
    let (active, _, _) = retry(&mut profile).unwrap();
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

/// One Play hashes the prepared files once, at the handoff right before the child. Damage still
/// refuses every Play, from the saved generation and from the one a refresh advanced to, and
/// re-preparing restores Play without removing the damaged snapshot by hand.
#[test]
fn play_hashes_the_prepared_files_once_and_still_refuses_damage() {
    use super::super::FULL_VERIFICATIONS;
    use std::cell::Cell;
    const TEST: &str =
        "managed_play::client::tests::play_hashes_the_prepared_files_once_and_still_refuses_damage";
    #[cfg(target_os = "linux")]
    const BUILD: &str = "developer-snapshot-recovery";
    #[cfg(not(target_os = "linux"))]
    const BUILD: &str = "v0.1.8";
    const BLOB: &[u8] = b"original bytes";
    const FIXTURE: &str = "PLAY_ONE_HASH_FIXTURE";
    let Some(fixture) = std::env::var_os(FIXTURE) else {
        let root = tempfile::Builder::new()
            .prefix("play-one-hash-")
            .tempdir()
            .unwrap();
        fs::write(root.path().join("fixture"), TEST).unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(FIXTURE, root.path())
            .env("SUNRISE_HOME", root.path().join("game"))
            .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
            .env(
                "SUNRISE_LAUNCHER_LOCATION_FILE",
                root.path().join("location"),
            )
            .env("SUNRISE_NO_KEYCHAIN", "1")
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("SUNRISE_LAUNCHER_DEVELOPER_OFFLINE", "1")
            .env("SUNRISE_MOD_INDEX_V2", root.path().join("no-index.signed"))
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success()
                && stdout
                    .lines()
                    .any(|line| line.starts_with("test result: ok. 1 passed;")),
            "{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!String::from_utf8_lossy(&output.stderr).contains(&*root.path().to_string_lossy()));
        return;
    };
    let fixture = std::path::PathBuf::from(fixture);
    let canonical = fixture.canonicalize().unwrap();
    assert_eq!(
        canonical.parent().unwrap(),
        std::env::temp_dir().canonicalize().unwrap()
    );
    assert!(canonical
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("play-one-hash-"));
    assert_eq!(fs::read_to_string(fixture.join("fixture")).unwrap(), TEST);
    for (var, name) in [
        ("SUNRISE_HOME", "game"),
        ("SUNRISE_LAUNCHER_HOME", "launcher"),
        ("SUNRISE_LAUNCHER_LOCATION_FILE", "location"),
        ("XDG_CONFIG_HOME", "config"),
        ("SUNRISE_MOD_INDEX_V2", "no-index.signed"),
    ] {
        let path = fixture.join(name);
        assert_eq!(std::env::var_os(var).as_deref(), Some(path.as_os_str()));
        assert_eq!(
            fs::symlink_metadata(path).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }
    let log_dir = fixture.join("logs");
    crate::launcher_log::init(&log_dir);
    let home = backend::game_home();
    let build = backend::version_dir(BUILD);
    let prepared = backend::resources(BUILD);
    fs::create_dir_all(&prepared).unwrap();
    fs::write(prepared.join("sunrise-resources.json"), b"{}").unwrap();
    fs::write(prepared.join("fixture-blob.bin"), BLOB).unwrap();
    fs::write(build.join(".installed"), []).unwrap();
    #[cfg(not(target_os = "linux"))]
    let client_bytes = b"unchanged client";
    #[cfg(target_os = "linux")]
    let client_bytes = br#"#!/bin/sh
if [ "$1" = --launch-api ]; then
  echo '{"options":[{"name":"--developer-single-player"}]}'
else
  touch "$SUNRISE_HOME/snapshot-recovery-started"
fi
"#;
    fs::write(build.join(backend::CLIENT), client_bytes).unwrap();
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            build.join(backend::CLIENT),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        fs::write(
            build.join("developer.json"),
            json!({"revision": "b".repeat(40)}).to_string(),
        )
        .unwrap();
    }
    super::super::Artifact::record_archive(&build, BUILD, &"a".repeat(64)).unwrap();
    fs::create_dir_all(home.join(mod_list::DIR)).unwrap();
    let write_package = |version: &str| {
        let body = format!("id='lantern'\nname='Lantern'\nversion='{version}'\napi='{}'\ncapabilities=['registries']\n", manifest::API);
        fs::write(
            home.join(mod_list::DIR).join("lantern.smod"),
            manifest::Package::write_files(&body, &[]).unwrap(),
        )
        .unwrap();
    };
    write_package("1.0.0");
    fs::write(home.join(mod_list::FILE), b"mod.lantern:true\n").unwrap();

    let instance = instances::selection().unwrap();
    let client = Client {
        instance: &instance,
        accepted_account: None,
        explicit_account: false,
        piped: false,
    };
    let cancellation = native_mods::Cancellation::default();
    let mut profile = Profile::open(&home, None).unwrap();
    // The game's own two steps of a Play, each with the full verifications it asked for.
    let ready = |profile: &mut Profile| {
        FULL_VERIFICATIONS.take();
        let (active, _, first) = client.ready(
            profile,
            || FirstBuild::installed(BUILD),
            |_, _| Ok(()),
            &cancellation,
            |_| {},
            || None,
        )?;
        Ok::<_, StartError>((active, FULL_VERIFICATIONS.take(), first))
    };
    let spawned = Cell::new(false);
    let handoff = |profile: &Profile, active: &Active, proof: Option<VerifiedSnapshot>| {
        spawned.set(false);
        FULL_VERIFICATIONS.take();
        let result = profile.verified_handoff_for(active, proof, &cancellation, |_| {
            spawned.set(true);
            Ok::<(), String>(())
        });
        (result, FULL_VERIFICATIONS.take())
    };
    let intact = |profile: &mut Profile| {
        let (active, before_handoff, proof) = ready(profile).unwrap();
        let initial = u32::from(proof.is_some());
        assert_eq!(before_handoff, initial);
        let (result, at_handoff) = handoff(profile, &active, proof);
        result.unwrap();
        assert!(spawned.get());
        assert_eq!(at_handoff, 1 - initial);
        active
    };
    let snapshot_error = |active: &Active, error: String| {
        let prefix =
            "Sunrise couldn't verify the profile's saved game build (managed_play/resources.rs:";
        let line = error
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_suffix(")."));
        assert!(
            line.is_some_and(|line| line.parse::<u32>().is_ok()),
            "{error}"
        );
        assert!(!error.contains(&active.resource_path().to_string_lossy().into_owned()));
    };

    // First Play without a receipt seals inside prepare (no separate verify pass after the
    // seal), and a repeat Play still verifies once at handoff.
    let fresh_home = home.join("instances/fresh");
    fs::create_dir_all(&fresh_home).unwrap();
    let fresh_active = {
        let mut fresh = Profile::open(&fresh_home, None).unwrap();
        let (active, before_handoff, proof) = ready(&mut fresh).unwrap();
        assert_eq!(before_handoff, 0);
        assert!(proof.is_some());
        let (result, at_handoff) = handoff(&fresh, &active, proof);
        result.unwrap();
        assert!(spawned.get());
        assert_eq!(at_handoff, 0);
        assert_eq!(intact(&mut fresh), active);
        active
    };

    // Without a receipt, Play must not uncounted-hash or repair an already-existing damaged
    // snapshot; it refuses once at commit, and accepts an intact one with one verify total.
    let receipt_file = build.join(".local/launcher-prepared/current.json");
    fs::remove_file(&receipt_file).unwrap();
    let existing_blob = fresh_active.resource_path().join("fixture-blob.bin");
    fs::write(&existing_blob, b"damaged without receipt").unwrap();
    let no_receipt_home = home.join("instances/no-receipt");
    fs::create_dir_all(&no_receipt_home).unwrap();
    {
        let mut no_receipt = Profile::open(&no_receipt_home, None).unwrap();
        let error = ready(&mut no_receipt).unwrap_err();
        assert!(
            matches!(error, StartError::Snapshot(detail) if detail.contains("managed_play/resources.rs:"))
        );
        assert_eq!(FULL_VERIFICATIONS.take(), 1);
        assert!(no_receipt.active().unwrap().is_none());
        assert_eq!(
            fs::read(&existing_blob).unwrap(),
            b"damaged without receipt"
        );
        assert!(!receipt_file.exists());
    }
    fs::write(&existing_blob, BLOB).unwrap();
    {
        let mut no_receipt = Profile::open(&no_receipt_home, None).unwrap();
        let (active, before_handoff, proof) = ready(&mut no_receipt).unwrap();
        assert_eq!(before_handoff, 1);
        assert!(proof.is_some());
        let (result, at_handoff) = handoff(&no_receipt, &active, proof);
        result.unwrap();
        assert!(spawned.get());
        assert_eq!(at_handoff, 0);
    }
    fs::remove_dir_all(build.join(".local/launcher-prepared")).unwrap();

    // A cached snapshot with an intact manifest but damaged payload must not bind First Play.
    let prepared_snapshot = {
        let preparation = backend::prepare_lock(BUILD).unwrap();
        super::super::Resources::prepare(BUILD, &preparation, None).unwrap()
    };
    let snapshot_blob = prepared_snapshot.path(BUILD).join("fixture-blob.bin");
    fs::write(&snapshot_blob, b"already damaged").unwrap();
    let saw_verify = Cell::new(false);
    FULL_VERIFICATIONS.take();
    let error = client
        .ready(
            &mut profile,
            || FirstBuild::installed(BUILD),
            |_, _| Ok(()),
            &cancellation,
            |step| {
                if matches!(step, Progress::Verify) {
                    saw_verify.set(true);
                }
            },
            || None,
        )
        .unwrap_err();
    assert!(saw_verify.get());
    assert!(
        matches!(error, StartError::Snapshot(detail) if detail.contains("managed_play/resources.rs:"))
    );
    assert_eq!(FULL_VERIFICATIONS.take(), 1);
    assert!(profile.active().unwrap().is_none());
    let before_reopen = crate::profile_txn::references(&home).unwrap();
    assert!(before_reopen.pointer.is_none());
    assert!(before_reopen.targets.contains(BUILD));
    // A failed unsealed attempt keeps its safety fence until the next Play takes admission.
    drop(profile);
    let mut profile = Profile::open(&home, None).unwrap();
    let recovered = crate::profile_txn::references(&home).unwrap();
    assert!(recovered.pointer.is_none());
    assert!(recovered.targets.is_empty());
    fs::write(snapshot_blob, BLOB).unwrap();

    // First Play, an unchanged Play, and a Play that refreshes for a changed mod.
    let first = intact(&mut profile);
    assert_eq!(first.generation(), 1);
    assert_eq!(intact(&mut profile), first);
    write_package("2.0.0");
    let refreshed = intact(&mut profile);
    assert_eq!(refreshed.generation(), 2);

    // Tamper after publication: the resources must still be checked at the handoff.
    let (active, before_handoff, _) = ready(&mut profile).unwrap();
    assert_eq!(before_handoff, 0);
    let blob = refreshed.resource_path().join("fixture-blob.bin");
    fs::write(&blob, b"tampered bytes").unwrap();
    assert_eq!(active, refreshed);
    let (result, at_handoff) = handoff(&profile, &active, None);
    snapshot_error(&active, result.unwrap_err());
    assert_eq!((spawned.get(), at_handoff), (false, 1));

    let failing_hook = crate::launch::Launch {
        pre_launch: "exit 23".into(),
        ..Default::default()
    };
    let succeeding_hook = crate::launch::Launch {
        pre_launch: if cfg!(windows) {
            "exit /b 0".into()
        } else {
            "true".into()
        },
        ..Default::default()
    };
    let try_hook = |launch: &crate::launch::Launch| {
        let mut command = Command::new(build.join(backend::CLIENT));
        command
            .current_dir(&home)
            .env("SUNRISE_HOME", &home)
            .arg("--game-dir")
            .arg(&home)
            .arg("--resources")
            .arg(active.resource_path());
        profile.spawn_configured(&active, &cancellation, command, launch, true)
    };
    // With a damaged snapshot, the pre-launch hook runs first: a failing hook stops before
    // verification, while a succeeding hook falls through to the single handoff verify.
    assert!(matches!(try_hook(&failing_hook), Err(StartError::Hook(_))));
    assert_eq!(FULL_VERIFICATIONS.take(), 0);
    let error = try_hook(&succeeding_hook)
        .err()
        .expect("damaged snapshot launched");
    assert!(matches!(error, StartError::Snapshot(_)), "{error:?}");
    snapshot_error(&active, error.into());
    assert_eq!(FULL_VERIFICATIONS.take(), 1);

    // Mod recovery still hashes the files itself, and refuses in its own words.
    let recovery = ModRecovery {
        build: Arc::new(FailedBuild {
            build: Build::Active(refreshed.clone()),
            digests: Default::default(),
        }),
        mods: off(),
    };
    let verify = |profile: &Profile| {
        FULL_VERIFICATIONS.take();
        let result = recovery.verify(profile, &mut None::<fn() -> Result<FirstBuild, String>>);
        (result, FULL_VERIFICATIONS.take())
    };
    let (result, count) = verify(&profile);
    assert_eq!(
        result.unwrap_err(),
        "The profile's game build changed. No mods were turned off."
    );
    assert_eq!(count, 1);

    // A refresh over the changed file advances without sealing it: the next generation keeps
    // the original digest, so the next Play refuses the same way.
    write_package("3.0.0");
    let (advanced, before_handoff, _) = ready(&mut profile).unwrap();
    assert_eq!((advanced.generation(), before_handoff), (3, 0));
    assert_eq!(advanced.resources, refreshed.resources);
    assert_eq!(profile.active().unwrap(), Some(advanced.clone()));
    let (result, at_handoff) = handoff(&profile, &advanced, None);
    snapshot_error(&advanced, result.unwrap_err());
    assert_eq!((spawned.get(), at_handoff), (false, 1));
    let (again, _, _) = ready(&mut profile).unwrap();
    let (result, _) = handoff(&profile, &again, None);
    snapshot_error(&again, result.unwrap_err());
    assert!(!spawned.get());

    // When a refresh would run native compilation, existing resource damage is reported
    // before the native build starts and without advancing the generation.
    recovery_mod(&home, "blur", "Motion Blur", true, "");
    fs::write(
        home.join(mod_list::FILE),
        b"mod.lantern:true\nmod.blur:true\n",
    )
    .unwrap();
    let native_started = Cell::new(false);
    FULL_VERIFICATIONS.take();
    let error = client
        .ready(
            &mut profile,
            || FirstBuild::installed(BUILD),
            |_, _| Ok(()),
            &cancellation,
            |step| {
                if matches!(step, Progress::NativeBuild { .. } | Progress::Native(_)) {
                    native_started.set(true);
                }
            },
            || None,
        )
        .unwrap_err();
    assert!(!native_started.get());
    assert!(matches!(error, StartError::Snapshot(_)), "{error:?}");
    snapshot_error(&advanced, error.into());
    assert_eq!(FULL_VERIFICATIONS.take(), 1);
    assert_eq!(profile.active().unwrap(), Some(advanced.clone()));
    let direct_error = profile
        .refresh_for_play(
            instance.native_profile,
            &cancellation,
            |_| panic!("native build started over damaged cached resources"),
            None,
            None,
        )
        .unwrap_err();
    assert!(
        matches!(&direct_error, super::super::first::RefreshError::Snapshot(detail)
            if detail.contains("managed_play/resources.rs:")),
        "{direct_error:?}"
    );
    assert_eq!(FULL_VERIFICATIONS.take(), 1);
    assert_eq!(profile.active().unwrap(), Some(advanced.clone()));
    fs::remove_dir_all(home.join(mod_list::NATIVE_DIR)).unwrap();
    fs::write(home.join(mod_list::FILE), b"mod.lantern:true\n").unwrap();

    // Prepare the same build without deleting its damaged snapshot. The committed digest
    // and generation need not change to recover.
    {
        let preparation = backend::prepare_lock(BUILD).unwrap();
        backend::finish_preparation(BUILD, &preparation).unwrap();
    }
    assert_eq!(intact(&mut profile), advanced);

    #[cfg(target_os = "linux")]
    {
        drop(profile);
        let marker = home.join("snapshot-recovery-started");
        assert!(!marker.exists());
        FULL_VERIFICATIONS.take();
        let mut game = Client {
            instance: &instance,
            accepted_account: None,
            explicit_account: false,
            piped: false,
        }
        .start_with_recovery(
            None,
            || panic!("the repaired profile is already bound"),
            |_| Ok(()),
            |_| Ok(()),
            &cancellation,
            |_| {},
        )
        .unwrap();
        assert!(game.child.wait().unwrap().success());
        assert!(marker.is_file());
        assert_eq!(FULL_VERIFICATIONS.take(), 1);
        drop(game);
        profile = Profile::open(&home, None).unwrap();
        assert_eq!(profile.active().unwrap(), Some(advanced.clone()));
    }

    // A client snapshot cannot authorize its own replacement by claiming to be server-only.
    let manifest_file = advanced.resource_path().join("sunrise-resources.json");
    fs::write(&manifest_file, br#"{"profile":"server"}"#).unwrap();
    let error = ready(&mut profile).unwrap_err();
    snapshot_error(&advanced, String::from(error));
    assert_eq!(profile.active().unwrap(), Some(advanced.clone()));
    assert_eq!(FULL_VERIFICATIONS.take(), 1);
    fs::write(&manifest_file, b"{}").unwrap();

    // A deleted manifest leaves the output incomplete: that branch hashes the files before it
    // would prepare anything, and refuses.
    let manifest_file = advanced.resource_path().join("sunrise-resources.json");
    fs::remove_file(&manifest_file).unwrap();
    let error = ready(&mut profile).unwrap_err();
    snapshot_error(&advanced, String::from(error));
    assert_eq!(FULL_VERIFICATIONS.take(), 1);
    fs::write(&manifest_file, b"{}").unwrap();
    assert_eq!(intact(&mut profile), advanced);

    // A changed binary is refused before refreshing changed inputs, without hashing
    // the prepared files, and again at the handoff when it changes after Play was ready.
    let program = build.join(backend::CLIENT);
    write_package("4.0.0");
    fs::write(&program, b"changed client").unwrap();
    let error = ready(&mut profile).unwrap_err();
    assert!(String::from(error).contains("managed_play/artifact.rs:"));
    assert_eq!(FULL_VERIFICATIONS.take(), 0);
    assert_eq!(profile.active().unwrap(), Some(advanced.clone()));
    write_package("3.0.0");
    fs::write(&program, client_bytes).unwrap();
    let (active, _, _) = ready(&mut profile).unwrap();
    fs::write(&program, b"changed client").unwrap();
    let (result, _) = handoff(&profile, &active, None);
    assert!(result.unwrap_err().contains("managed_play/artifact.rs:"));
    assert!(!spawned.get());
    fs::write(&program, client_bytes).unwrap();
    assert_eq!(intact(&mut profile), advanced);

    // Saved image evidence covers both the preparer and the archive receipt.
    let archive = build.join(super::super::Artifact::ARCHIVE_RECEIPT);
    let receipt = fs::read(&archive).unwrap();
    let mut wrong_receipt: serde_json::Value = serde_json::from_slice(&receipt).unwrap();
    wrong_receipt["archive"] = json!("b".repeat(64));
    fs::write(&archive, serde_json::to_vec(&wrong_receipt).unwrap()).unwrap();
    let error = ready(&mut profile).unwrap_err();
    assert!(
        matches!(&error, StartError::Native(native_mods::Failed::Other(detail), None)
            if detail.contains("managed_play/artifact.rs:")),
        "{error:?}"
    );
    assert_eq!(FULL_VERIFICATIONS.take(), 0);
    fs::write(&archive, receipt).unwrap();
    let preparer = build.join(crate::preparer::BINARY);
    fs::write(&preparer, b"not the saved preparer").unwrap();
    let error = ready(&mut profile).unwrap_err();
    assert!(
        matches!(&error, StartError::Native(native_mods::Failed::Other(detail), None)
            if detail.contains("managed_play/artifact.rs:")),
        "{error:?}"
    );
    assert_eq!(FULL_VERIFICATIONS.take(), 0);
    fs::remove_file(&preparer).unwrap();
    assert_eq!(profile.active().unwrap(), Some(advanced.clone()));

    // Seed the legacy pointer to a real server-only tree while leaving a playable client
    // preparation available. Neither changing a server payload nor claiming the client is a
    // server should authorize a replacement; the intact legacy tree should upgrade.
    let legacy = tempfile::tempdir_in(build.join(".local")).unwrap();
    let legacy_tree = legacy.path().join("resources");
    fs::create_dir(&legacy_tree).unwrap();
    fs::write(
        legacy_tree.join("sunrise-resources.json"),
        br#"{"profile":"server"}"#,
    )
    .unwrap();
    fs::write(legacy_tree.join("fixture-blob.bin"), BLOB).unwrap();
    let primary = super::super::artifact::tree(&legacy_tree, true)
        .unwrap()
        .unwrap();
    let resources: super::super::Resources = serde_json::from_value(json!({
        "primary": primary, "accelerator": null,
    }))
    .unwrap();
    let path = resources.path(BUILD);
    fs::create_dir(path.parent().unwrap()).unwrap();
    fs::rename(legacy_tree, &path).unwrap();
    let (server, _) = profile
        .refresh_for_play(
            instance.native_profile,
            &cancellation,
            |_| {},
            None,
            Some(resources.clone()),
        )
        .unwrap()
        .unwrap();
    let blob = path.join("fixture-blob.bin");
    fs::write(&blob, b"damaged server payload").unwrap();
    let error = ready(&mut profile).unwrap_err();
    snapshot_error(&server, error.into());
    assert_eq!(FULL_VERIFICATIONS.take(), 1);
    assert_eq!(profile.active().unwrap(), Some(server.clone()));
    fs::write(&blob, BLOB).unwrap();
    let (upgraded, old_verifications, proof) = ready(&mut profile).unwrap();
    assert_eq!(old_verifications, 1);
    assert!(proof.is_none());
    assert_eq!(upgraded.generation(), server.generation() + 1);
    assert_eq!(upgraded.resources, advanced.resources);
    let (result, replacement_verifications) = handoff(&profile, &upgraded, proof);
    result.unwrap();
    assert!(spawned.get());
    assert_eq!(replacement_verifications, 1);

    // An intact ServerOnly upgrade that freshly seals the new client snapshot inside prepare
    // verifies the old snapshot once and does not run a second verify pass after the seal.
    let (server_again, _) = profile
        .refresh_for_play(
            instance.native_profile,
            &cancellation,
            |_| {},
            None,
            Some(resources),
        )
        .unwrap()
        .unwrap();
    fs::remove_file(&receipt_file).unwrap();
    fs::remove_dir_all(upgraded.resource_path().parent().unwrap()).unwrap();
    let (sealed_upgrade, old_verifications, proof) = ready(&mut profile).unwrap();
    assert_eq!(old_verifications, 1);
    assert!(proof.is_some());
    assert_eq!(sealed_upgrade.generation(), server_again.generation() + 1);
    assert_eq!(sealed_upgrade.resources, advanced.resources);
    let (result, replacement_verifications) = handoff(&profile, &sealed_upgrade, proof);
    result.unwrap();
    assert!(spawned.get());
    assert_eq!(replacement_verifications, 0);
    assert_eq!(intact(&mut profile), sealed_upgrade);
    let log = fs::read_to_string(log_dir.join("launcher.log")).unwrap();
    assert!(log.contains("Verifying game files: done"));
    assert!(!log.contains(&*fixture.to_string_lossy()));
}

#[test]
fn pre_launch_hook_runs_outside_the_manifest_lock_and_reacquires_before_spawn() {
    use super::super::FULL_VERIFICATIONS;
    const TEST: &str = "managed_play::client::tests::pre_launch_hook_runs_outside_the_manifest_lock_and_reacquires_before_spawn";
    const ACTOR: &str = "PLAY_HOOK_LOCK_ACTOR";
    const BUILD: &str = "v0.1.9";
    const SENTINEL: &str = "stop-before-spawn";
    let Ok(role) = std::env::var(ACTOR) else {
        let root = tempfile::Builder::new()
            .prefix("play-hook-lock-")
            .tempdir()
            .unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(ACTOR, "scenario")
            .env("SUNRISE_HOME", root.path().join("game"))
            .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
            .env(
                "SUNRISE_LAUNCHER_LOCATION_FILE",
                root.path().join("location"),
            )
            .env("SUNRISE_NO_KEYCHAIN", "1")
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("SUNRISE_MOD_INDEX_V2", root.path().join("no-index.signed"))
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success()
                && stdout
                    .lines()
                    .any(|line| line.starts_with("test result: ok. 1 passed;")),
            "{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    };
    let home = backend::game_home();
    match role.as_str() {
        "lock-home" => {
            let guard = sunrise_launcher_smod::install_lock::try_lock_home(&home)
                .expect("manifest lock was still held while the pre-launch hook ran");
            fs::write(home.join("hook-acquired-manifest-lock"), b"ok").unwrap();
            drop(guard);
            return;
        }
        "expect-locked" => {
            assert!(
                sunrise_launcher_smod::install_lock::try_lock_home(&home).is_none(),
                "manifest lock was not reacquired before spawn"
            );
            return;
        }
        "scenario" => {}
        other => panic!("unknown hook-lock actor: {other}"),
    }
    let build = backend::version_dir(BUILD);
    let prepared = backend::resources(BUILD);
    fs::create_dir_all(&prepared).unwrap();
    fs::write(prepared.join("sunrise-resources.json"), b"{}").unwrap();
    fs::write(prepared.join("fixture-blob.bin"), b"hook bytes").unwrap();
    fs::write(build.join(".installed"), []).unwrap();
    fs::write(build.join(backend::CLIENT), b"hook client").unwrap();
    fs::create_dir_all(&home).unwrap();
    let instance = instances::selection().unwrap();
    let client = Client {
        instance: &instance,
        accepted_account: None,
        explicit_account: false,
        piped: false,
    };
    let cancellation = native_mods::Cancellation::default();
    let mut profile = Profile::open(&home, None).unwrap();
    let exe = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let mut launch = crate::launch::Launch {
        pre_launch: if cfg!(windows) {
            format!("\"%HOOK_EXE%\" --exact {TEST} --nocapture")
        } else {
            format!("\"$HOOK_EXE\" --exact {TEST} --nocapture")
        },
        ..Default::default()
    };
    launch.env.insert("HOOK_EXE".into(), exe.clone());
    launch.env.insert(ACTOR.into(), "lock-home".into());
    let marker = home.join("hook-acquired-manifest-lock");
    let command = |active: &Active| {
        let mut command = Command::new(build.join(backend::CLIENT));
        command
            .current_dir(&home)
            .env("SUNRISE_HOME", &home)
            .arg("--game-dir")
            .arg(&home)
            .arg("--resources")
            .arg(active.resource_path());
        command
    };
    let verify_locked_at_spawn = || {
        let output = Command::new(&exe)
            .args(["--exact", TEST, "--nocapture"])
            .env(ACTOR, "expect-locked")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success()
                && stdout
                    .lines()
                    .any(|line| line.starts_with("test result: ok. 1 passed;")),
            "{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };

    // Exercise both First Play (which carries an owned `VerifiedSnapshot` holding its own
    // `HomeLockGuard`) and repeat Play (`proof == None`). Because the hook runs unlocked
    // before verification, both paths perform one full verification after the hook returns.
    for first_play in [true, false] {
        if marker.exists() {
            fs::remove_file(&marker).unwrap();
        }
        FULL_VERIFICATIONS.take();
        let (active, _, proof) = client
            .ready(
                &mut profile,
                || FirstBuild::installed(BUILD),
                |_, _| Ok(()),
                &cancellation,
                |_| {},
                || None,
            )
            .unwrap();
        assert_eq!(proof.is_some(), first_play);
        let result = profile.spawn_checked_account(
            &active,
            &cancellation,
            command(&active),
            (&launch, true, &|_| {}, proof),
            |_| Ok(()),
            |_| {
                verify_locked_at_spawn();
                Err(StartError::Other(SENTINEL.into()))
            },
        );
        match result {
            Err(StartError::Other(detail)) => assert_eq!(detail, SENTINEL),
            Err(other) => panic!("unexpected start error: {other:?}"),
            Ok(_) => panic!("expected hook sentinel error"),
        }
        assert!(marker.is_file(), "hook did not acquire the manifest lock");
        assert_eq!(FULL_VERIFICATIONS.take(), 1);
    }
}

#[test]
fn successful_mutating_pre_launch_hook_is_refused_before_spawn_with_one_verify() {
    use super::super::FULL_VERIFICATIONS;
    const TEST: &str = "managed_play::client::tests::successful_mutating_pre_launch_hook_is_refused_before_spawn_with_one_verify";
    const ACTOR: &str = "PLAY_MUTATING_HOOK_ACTOR";
    const BUILD: &str = "v0.1.10";
    let Ok(role) = std::env::var(ACTOR) else {
        let root = tempfile::Builder::new()
            .prefix("play-mutating-hook-")
            .tempdir()
            .unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(ACTOR, "scenario")
            .env("SUNRISE_HOME", root.path().join("game"))
            .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
            .env(
                "SUNRISE_LAUNCHER_LOCATION_FILE",
                root.path().join("location"),
            )
            .env("SUNRISE_NO_KEYCHAIN", "1")
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("SUNRISE_MOD_INDEX_V2", root.path().join("no-index.signed"))
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success()
                && stdout
                    .lines()
                    .any(|line| line.starts_with("test result: ok. 1 passed;")),
            "{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    };
    let home = backend::game_home();
    match role.as_str() {
        "mutate-inputs" => {
            fs::write(home.join(mod_list::FILE), b"mod.lantern:false\n").unwrap();
            fs::write(home.join("hook-mutated-inputs"), b"ok").unwrap();
            return;
        }
        "mutate-binary" => {
            fs::write(
                backend::version_dir(BUILD).join(backend::CLIENT),
                b"tampered client",
            )
            .unwrap();
            fs::write(home.join("hook-mutated-binary"), b"ok").unwrap();
            return;
        }
        "mutate-resource" => {
            let blob = std::path::PathBuf::from(std::env::var_os("HOOK_RESOURCE_BLOB").unwrap());
            fs::write(blob, b"tampered resource").unwrap();
            fs::write(home.join("hook-mutated-resource"), b"ok").unwrap();
            return;
        }
        "scenario" => {}
        other => panic!("unknown mutating-hook actor: {other}"),
    }
    let build = backend::version_dir(BUILD);
    let prepared = backend::resources(BUILD);
    fs::create_dir_all(&prepared).unwrap();
    fs::write(prepared.join("sunrise-resources.json"), b"{}").unwrap();
    fs::write(prepared.join("fixture-blob.bin"), b"hook bytes").unwrap();
    fs::write(build.join(".installed"), []).unwrap();
    #[cfg(not(target_os = "linux"))]
    let client_bytes = b"unchanged client";
    #[cfg(target_os = "linux")]
    let client_bytes = br#"#!/bin/sh
touch "$SUNRISE_HOME/game-spawned"
"#;
    fs::write(build.join(backend::CLIENT), client_bytes).unwrap();
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            build.join(backend::CLIENT),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    fs::create_dir_all(home.join(mod_list::DIR)).unwrap();
    let body = format!(
        "id='lantern'\nname='Lantern'\nversion='1.0.0'\napi='{}'\ncapabilities=['registries']\n",
        manifest::API
    );
    fs::write(
        home.join(mod_list::DIR).join("lantern.smod"),
        manifest::Package::write_files(&body, &[]).unwrap(),
    )
    .unwrap();
    fs::write(home.join(mod_list::FILE), b"mod.lantern:true\n").unwrap();

    let instance = instances::selection().unwrap();
    let client = Client {
        instance: &instance,
        accepted_account: None,
        explicit_account: false,
        piped: false,
    };
    let cancellation = native_mods::Cancellation::default();
    let mut profile = Profile::open(&home, None).unwrap();
    let (active, _, _) = client
        .ready(
            &mut profile,
            || FirstBuild::installed(BUILD),
            |_, _| Ok(()),
            &cancellation,
            |_| {},
            || None,
        )
        .unwrap();
    let exe = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let hook_for = |actor: &str| {
        let mut launch = crate::launch::Launch {
            pre_launch: if cfg!(windows) {
                format!("\"%HOOK_EXE%\" --exact {TEST} --nocapture")
            } else {
                format!("\"$HOOK_EXE\" --exact {TEST} --nocapture")
            },
            ..Default::default()
        };
        launch.env.insert("HOOK_EXE".into(), exe.clone());
        launch.env.insert(ACTOR.into(), actor.into());
        launch.env.insert(
            "HOOK_RESOURCE_BLOB".into(),
            active
                .resource_path()
                .join("fixture-blob.bin")
                .to_string_lossy()
                .into_owned(),
        );
        launch
    };
    let command = || {
        let mut command = Command::new(build.join(backend::CLIENT));
        command
            .current_dir(&home)
            .env("SUNRISE_HOME", &home)
            .arg("--game-dir")
            .arg(&home)
            .arg("--resources")
            .arg(active.resource_path());
        command
    };

    // Case 1: a hook that exits 0 after mutating profile inputs must be refused before spawn;
    // inputs are checked before walking prepared resources, so no resource walk runs.
    FULL_VERIFICATIONS.take();
    let error = profile
        .spawn_checked_account(
            &active,
            &cancellation,
            command(),
            (&hook_for("mutate-inputs"), true, &|_| {}, None),
            |_| Ok(()),
            |_| panic!("mutated profile reached spawn"),
        )
        .err()
        .expect("mutating hook launched the game");
    assert!(
        matches!(&error, StartError::Other(detail) if detail.contains("managed_play/first.rs:")),
        "{error:?}"
    );
    assert!(home.join("hook-mutated-inputs").is_file());
    assert!(!home.join("game-spawned").exists());
    assert_eq!(FULL_VERIFICATIONS.take(), 0);

    // Case 2: a hook that exits 0 after mutating the game binary must also be refused before
    // spawn; binaries are checked before walking prepared resources.
    fs::write(home.join(mod_list::FILE), b"mod.lantern:true\n").unwrap();
    FULL_VERIFICATIONS.take();
    let error = profile
        .spawn_checked_account(
            &active,
            &cancellation,
            command(),
            (&hook_for("mutate-binary"), true, &|_| {}, None),
            |_| Ok(()),
            |_| panic!("mutated binary reached spawn"),
        )
        .err()
        .expect("binary-mutating hook launched the game");
    assert!(
        matches!(&error, StartError::Other(detail) if detail.contains("managed_play.rs:")),
        "{error:?}"
    );
    assert!(home.join("hook-mutated-binary").is_file());
    assert!(!home.join("game-spawned").exists());
    assert_eq!(FULL_VERIFICATIONS.take(), 0);

    // Case 3: a hook that exits 0 after mutating a file inside the active resource snapshot
    // (leaving `sunrise-resources.json` intact) must be caught by the single handoff verify.
    fs::write(build.join(backend::CLIENT), client_bytes).unwrap();
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            build.join(backend::CLIENT),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    FULL_VERIFICATIONS.take();
    let error = profile
        .spawn_checked_account(
            &active,
            &cancellation,
            command(),
            (&hook_for("mutate-resource"), true, &|_| {}, None),
            |_| Ok(()),
            |_| panic!("mutated resource reached spawn"),
        )
        .err()
        .expect("resource-mutating hook launched the game");
    assert!(matches!(error, StartError::Snapshot(_)), "{error:?}");
    assert!(home.join("hook-mutated-resource").is_file());
    assert!(!home.join("game-spawned").exists());
    assert_eq!(FULL_VERIFICATIONS.take(), 1);
}
