//! Real CLI contenders for launcher data and profile lifecycle admission.
use std::{fs, process::Command};
use sunrise_launcher::data_location::{self, Roots};

#[test]
fn move_excludes_cli_writers_between_settlement_and_copying() {
    let root = tempfile::tempdir().unwrap();
    let base = root.path().canonicalize().unwrap();
    let game_parent = base.join("game-parent");
    let source = Roots {
        launcher: base.join("launcher"),
        game: game_parent.join("game"),
        build: base.join("build"),
    };
    let target = base.join("NewHome");
    for dir in [&source.launcher, &source.game, &source.build] {
        fs::create_dir_all(dir).unwrap();
    }
    let bootstrap = base.join("location.json");
    let point = |path: &std::path::Path, roots: &Roots| {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            serde_json::json!({"version":1,"roots":roots}).to_string(),
        )
        .unwrap();
    };
    point(&bootstrap, &source);
    let alternate = base.join("alternate/location.json");
    point(&alternate, &source);
    // Sharing any one of the physical roots is enough to make this a contender.
    let spare = Roots {
        launcher: base.join("spare/launcher"),
        game: base.join("spare/game"),
        build: base.join("spare/build"),
    };
    for path in [&spare.launcher, &spare.game, &spare.build] {
        fs::create_dir_all(path).unwrap();
    }
    let single_root = ["launcher", "game", "build"].map(|shared| {
        let path = base.join("alternate").join(format!("{shared}.json"));
        let mut roots = spare.clone();
        match shared {
            "launcher" => roots.launcher = source.launcher.clone(),
            "game" => roots.game = source.game.clone(),
            _ => roots.build = source.build.clone(),
        }
        point(&path, &roots);
        path
    });
    let nested_game = source.game.join("nested-game");
    let sibling_game = game_parent.join("game-sibling");
    fs::create_dir(&nested_game).unwrap();
    fs::create_dir(&sibling_game).unwrap();
    let location_for_game = |name: &str, game| {
        let pointer = base.join("alternate").join(name);
        point(
            &pointer,
            &Roots {
                game,
                ..spare.clone()
            },
        );
        pointer
    };
    let nested = location_for_game("child.json", nested_game.clone());
    let ancestor = location_for_game("parent.json", game_parent.clone());
    let sibling = location_for_game("sibling.json", sibling_game.clone());
    let case_game = game_parent.join("GAME");
    let distinct_case = match fs::create_dir(&case_game) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(error) => panic!("could not probe case-distinct roots: {error}"),
    };
    let in_other_case = location_for_game("case.json", case_game.clone());
    let alternative_case = base.join("newhome/GAME");
    let nonportable = base.join("NewHöme");
    #[cfg(unix)]
    let aliased_nested = {
        let alias = base.join("alias-of-game-parent");
        std::os::unix::fs::symlink(&game_parent, &alias).unwrap();
        location_for_game("aliased-child.json", alias.join("game/nested-game"))
    };
    let create = |location: &std::path::Path, game_override: Option<&std::path::Path>| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_sunrise-launcher"));
        cmd.args(["instance", "create", "Late Writer"])
            .env("SUNRISE_LAUNCHER_LOCATION_FILE", location)
            .env("XDG_DATA_HOME", base.join("fallback-data"))
            .env("XDG_CONFIG_HOME", base.join("fallback-config"))
            .env_remove("SUNRISE_HOME")
            .env_remove("SUNRISE_LAUNCHER_HOME");
        if let Some(game) = game_override {
            cmd.env("SUNRISE_HOME", game)
                .env("SUNRISE_LAUNCHER_HOME", &spare.launcher);
        }
        cmd.output().unwrap()
    };
    // A persisted Move location must already exist. Portable environment overrides
    // can designate a missing root, so use that real startup route for both cases.
    let refused = create(&alternate, Some(&nonportable));
    assert_eq!(refused.status.code(), Some(1), "{refused:?}");
    assert!(String::from_utf8_lossy(&refused.stderr).contains("Choose a portable folder name"));
    assert!(!nonportable.exists());
    let root_claim = "another launcher is using or moving this data";
    let assert_blocked = |path: &std::path::Path, game_override, lock: &str| {
        let blocked = create(path, game_override);
        let error = String::from_utf8_lossy(&blocked.stderr);
        assert_eq!(blocked.status.code(), Some(1), "{blocked:?}");
        assert!(
            error.contains("Cannot lock") && error.contains(lock),
            "{error}"
        );
        assert!(!source.game.join("instances/late-writer").exists());
        assert!(!spare.game.join("instances/late-writer").exists());
    };
    let mut reached_copy = false;
    let mut reached_commit = false;
    let moved = data_location::migrate(&source, &target, &bootstrap, |step| {
        if step == "Committing data location" {
            reached_commit = true;
            assert!(target.join("game").is_dir());
            // On a sensitive volume, both spellings now physically exist. The
            // Move still holds the portable spelling it reserved before mkdir.
            let alternate_parent = alternative_case.parent().unwrap();
            let made_parent = !alternate_parent.exists();
            let made_game = !alternative_case.exists();
            if made_game {
                fs::create_dir_all(&alternative_case).unwrap();
            }
            assert_blocked(&alternate, Some(&alternative_case), root_claim);
            assert!(!alternative_case.join("instances/late-writer").exists());
            if made_game {
                fs::remove_dir(&alternative_case).unwrap();
            }
            if made_parent {
                fs::remove_dir(alternate_parent).unwrap();
            }
            return;
        }
        if step != "Copying and verifying launcher data" {
            return;
        }
        reached_copy = true;
        assert!(
            !target.exists(),
            "prove the contention before the destination mkdir"
        );
        assert_blocked(&bootstrap, None, "migration.lock");
        assert_blocked(&alternate, None, root_claim);
        for game in [&source.game, &target, &alternative_case] {
            assert_blocked(&alternate, Some(game), root_claim);
        }
        assert!(!alternative_case.parent().unwrap().exists());
        for path in single_root.iter().chain([&nested, &ancestor]) {
            assert_blocked(path, None, root_claim);
        }
        #[cfg(unix)]
        assert_blocked(&aliased_nested, None, root_claim);
        for game in [&nested_game, &game_parent, &target] {
            assert!(!game.join("instances/late-writer").exists());
        }
        let disjoint = create(&sibling, None);
        assert!(disjoint.status.success(), "{disjoint:?}");
        assert!(sibling_game.join("instances/late-writer").is_dir());
        if distinct_case {
            let case_disjoint = create(&in_other_case, None);
            assert!(case_disjoint.status.success(), "{case_disjoint:?}");
            assert!(case_game.join("instances/late-writer").is_dir());
        } else {
            assert_blocked(&in_other_case, None, root_claim);
        }
    })
    .unwrap();
    assert!(reached_copy && reached_commit);
    assert!(!moved.game.join("instances/late-writer").exists());
    assert!(!moved
        .game
        .join("nested-game/instances/late-writer")
        .exists());
    let accepted = create(&bootstrap, None);
    assert!(accepted.status.success(), "{accepted:?}");
    assert!(moved.game.join("instances/late-writer").is_dir());
    assert!(!source.game.join("instances/late-writer").exists());
}

#[cfg(unix)]
#[test]
fn a_writable_custom_root_does_not_require_a_writable_parent() {
    use std::{os::unix::fs::PermissionsExt as _, path::PathBuf};
    let root = tempfile::tempdir().unwrap();
    let base = root.path().canonicalize().unwrap();
    let parent = base.join("protected");
    let source = Roots {
        launcher: parent.join("launcher"),
        game: parent.join("game"),
        build: parent.join("build"),
    };
    for dir in [&source.launcher, &source.game, &source.build] {
        fs::create_dir_all(dir).unwrap();
    }
    let location = base.join("location.json");
    fs::write(
        &location,
        serde_json::json!({"version":1,"roots":source}).to_string(),
    )
    .unwrap();
    struct RestoreMode(PathBuf);
    impl Drop for RestoreMode {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
        }
    }
    let _restore = RestoreMode(parent.clone());
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o555)).unwrap();
    let probe = parent.join("parent-write-probe");
    match fs::write(&probe, b"must be denied") {
        Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied),
        Ok(()) => {
            fs::remove_file(probe).unwrap();
            eprintln!("protected-parent fixture needs an unprivileged account");
            return;
        }
    }
    let made = Command::new(env!("CARGO_BIN_EXE_sunrise-launcher"))
        .args(["instance", "create", "Writable Home"])
        .env("SUNRISE_LAUNCHER_LOCATION_FILE", &location)
        .env_remove("SUNRISE_HOME")
        .env_remove("SUNRISE_LAUNCHER_HOME")
        .output()
        .unwrap();
    assert!(made.status.success(), "{made:?}");
    assert!(source.game.join("instances/writable-home").is_dir());
    let moved =
        data_location::migrate(&source, &base.join("destination"), &location, |_| {}).unwrap();
    assert!(moved.game.join("instances/writable-home").is_dir());
    assert_eq!(fs::read_dir(parent).unwrap().count(), 3);
}

#[test]
fn profile_copy_and_world_writers_share_admission() {
    use std::io::Write as _;
    let root = tempfile::tempdir().unwrap();
    let game = root.path().join("game");
    let data = root.path().join("launcher");
    let trash = root.path().join("trash");
    fs::create_dir(&trash).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_sunrise-launcher"))
            .args(args)
            .current_dir(root.path())
            .env(
                "SUNRISE_LAUNCHER_LOCATION_FILE",
                root.path().join("location.json"),
            )
            .env("SUNRISE_HOME", &game)
            .env("SUNRISE_LAUNCHER_HOME", &data)
            .env("SUNRISE_LAUNCHER_TRASH", &trash)
            .output()
            .unwrap()
    };
    let source = run(&["instance", "create", "Source"]);
    assert!(source.status.success(), "{source:?}");
    let world = game.join("instances/source/saves/Meadow");
    fs::create_dir_all(&world).unwrap();
    fs::write(world.join("level.dat"), b"old world").unwrap();
    let world_lock = fs::File::create(world.join("session.lock")).unwrap();
    world_lock.try_lock().unwrap();
    let copy = || run(&["instance", "create", "Duplicate", "--copy", "Source"]);
    let refused = copy();
    assert_eq!(refused.status.code(), Some(1), "{refused:?}");
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("Could not lock world Meadow"),
        "{refused:?}"
    );
    assert!(!game.join("instances/duplicate").exists());
    world_lock.unlock().unwrap();
    drop(world_lock);

    let path = root.path().join("lantern.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("pack.mcmeta", options).unwrap();
    zip.write_all(br#"{"pack":{"description":"Lanterns"}}"#)
        .unwrap();
    zip.start_file("data/lantern/function/tick.mcfunction", options)
        .unwrap();
    zip.write_all(b"say lantern").unwrap();
    zip.finish().unwrap();
    let add = || {
        run(&[
            "worlds",
            "datapack-add",
            "Meadow",
            "lantern.zip",
            "--instance",
            "Source",
        ])
    };
    let delete = || run(&["worlds", "delete", "Meadow", "--instance", "Source"]);
    // This is the actual name gate Profile Copy keeps through snapshot and publication.
    let owner = fs::File::options()
        .read(true)
        .write(true)
        .open(game.join(".launcher-instance-gates/instances/source"))
        .unwrap();
    owner.try_lock().unwrap();
    for blocked in [delete(), add()] {
        assert_eq!(blocked.status.code(), Some(1), "{blocked:?}");
        assert!(
            String::from_utf8_lossy(&blocked.stderr).contains("profile is in use"),
            "{blocked:?}"
        );
    }
    assert_eq!(fs::read(world.join("level.dat")).unwrap(), b"old world");
    assert!(!world.join("datapacks").exists());
    owner.unlock().unwrap();
    drop(owner);

    let accepted = add();
    assert!(accepted.status.success(), "{accepted:?}");
    let copied = copy();
    assert!(copied.status.success(), "{copied:?}");
    let snapshot = game.join("instances/duplicate/saves/Meadow");
    assert_eq!(
        fs::read(snapshot.join("datapacks/lantern.zip")).unwrap(),
        fs::read(path).unwrap()
    );
    assert!(!snapshot.join("session.lock").exists());
    let deleted = delete();
    assert!(deleted.status.success(), "{deleted:?}");
    assert!(!world.exists());
    assert!(snapshot.join("level.dat").is_file());
}
