use super::*;
use std::collections::BTreeMap;

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "sunrise-launcher-instances-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// An instance with nothing chosen but its name and what it copies.
fn create_in(
    game_home: &Path,
    data: &Path,
    name: &str,
    from: Option<&Instance>,
) -> Result<Instance, String> {
    create_whole_in(game_home, data, name, from, None, |_| {})
}

fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// Every file under `dir`, relative, with its bytes.
fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for e in fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                out.insert(p.strip_prefix(root).unwrap().into(), fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// A game home as a player has it before instances: worlds, options, packs, a session, an
/// imported modpack and a registry modpack with their own worlds.
fn player_home(home: &Path) {
    put(&home.join("saves/World/level.dat"), "world");
    put(&home.join("saves/World/region/r.0.0.mca"), "region");
    put(&home.join("saves/Other/level.dat"), "other");
    put(&home.join("options.txt"), "fov:90\n");
    put(&home.join("sunrise-player.txt"), "name:Player123\n");
    put(&home.join("resourcepacks/faithful.zip"), "pack");
    put(&home.join("logs/latest.log"), "log");
    put(&home.join("sessions/a.json"), "token");
    put(&home.join("telemetry/consent.json"), "{}");
    put(
        &home.join("ports/small-pack/launcher-status.json"),
        r#"{"format": 1, "pack": {"name": "Small Pack"}}"#,
    );
    put(
        &home.join("ports/small-pack/home/saves/Modded/level.dat"),
        "modded",
    );
    put(
        &home.join("sunrise-packs/cozy/pack.json"),
        r#"{"id":"cozy","name":"Cozy","version":"1.0.0","sha256":"00","mods":[]}"#,
    );
    put(
        &home.join("sunrise-packs/cozy/home/saves/Cozy/level.dat"),
        "cozy",
    );
}

#[test]
fn build_choices_start_balanced_and_are_remembered_per_instance() {
    use crate::native_mods::Profile;
    let root = scratch("native-build-choices");
    let game = root.join("game");
    let data = root.join("launcher");
    for id in ["first", "second"] {
        fs::create_dir_all(game.join("instances").join(id)).unwrap();
    }
    for instance in list_in(&game, &data) {
        assert_eq!(instance.native_profile, Profile::Optimized);
    }
    let choose = |id, profile| {
        edit_in(&game, &data, id, |meta| meta.native_profile = Some(profile)).unwrap()
    };
    choose("first", Profile::Optimized);
    choose("second", Profile::Optimized);
    let before = fs::read(data.join("instances.json")).unwrap();
    for unknown in [serde_json::json!("future"), serde_json::json!(42)] {
        let mut stored: serde_json::Value = serde_json::from_slice(&before).unwrap();
        stored["instances"]["first"]["native_profile"] = unknown;
        stored["instances"]["first"]["version"] = serde_json::json!("kept-version");
        fs::write(
            data.join("instances.json"),
            serde_json::to_vec(&stored).unwrap(),
        )
        .unwrap();
        edit_in(&game, &data, "second", |meta| {
            meta.name = Some("Still here".into())
        })
        .unwrap();
        let loaded = list_in(&game, &data);
        assert_eq!(
            find_in(&loaded, "first").unwrap().native_profile,
            Profile::Optimized
        );
        assert_eq!(
            find_in(&loaded, "first").unwrap().version.as_deref(),
            Some("kept-version")
        );
        let neighbour = find_in(&loaded, "second").unwrap();
        assert_eq!(neighbour.native_profile, Profile::Optimized);
        assert_eq!(neighbour.name, "Still here");
    }
    fs::write(data.join("instances.json"), before).unwrap();
    choose("second", Profile::Optimized);
    let again = list_in(&game, &data);
    assert_eq!(
        find_in(&again, "first").unwrap().native_profile,
        Profile::Optimized
    );
    assert_eq!(
        find_in(&again, "second").unwrap().native_profile,
        Profile::Optimized
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn existing_worlds_stay_where_they_are_and_every_folder_becomes_an_instance() {
    let root = scratch("migrate");
    let (home, data) = (root.join("sunrise"), root.join("launcher"));
    player_home(&home);
    let before = files(&home);

    // A launcher with instances, on a player's existing folders: no instances.json yet.
    let all = list_in(&home, &data);
    let ids: Vec<&str> = all.iter().map(|i| i.id.as_str()).collect();
    // A pack an earlier launcher ported (`ports/`) is not listed: its folder is left as it is.
    assert_eq!(ids, [DEFAULT, "pack:cozy"]);
    assert_eq!(all[0].name, "Sunrise");
    assert_eq!(all[0].home, home);
    assert_eq!(all[1].home, home.join("sunrise-packs/cozy/home"));

    // Everything the player can do with instances, then not one byte of theirs has moved.
    let empty = create_in(&home, &data, "Survival", None).unwrap();
    let copy = create_in(&home, &data, "My copy", Some(&all[0])).unwrap();
    edit_in(&home, &data, DEFAULT, |m| m.name = Some("Main".into())).unwrap();
    edit_in(&home, &data, "Cozy", |m| {
        m.icon = Some(Icon::Texture("diamond".into()));
        m.group = Some("Modded".into());
    })
    .unwrap();
    let after = files(&home);
    for (path, bytes) in &before {
        assert_eq!(after.get(path), Some(bytes), "{} changed", path.display());
    }
    assert!(!data.join("settings.json").exists());

    // The empty one has only the player's options and name; the copy has the worlds and
    // packs, never the sessions, logs, data collection or other instances.
    assert_eq!(empty.home, home.join("instances/survival"));
    let empty_files: Vec<PathBuf> = files(&empty.home).into_keys().collect();
    assert_eq!(
        empty_files,
        [PathBuf::from("options.txt"), "sunrise-player.txt".into()]
    );
    let copied = files(&copy.home);
    for kept in [
        "saves/World/region/r.0.0.mca",
        "saves/Other/level.dat",
        "resourcepacks/faithful.zip",
    ] {
        assert_eq!(copied[Path::new(kept)], before[Path::new(kept)]);
    }
    assert!(copied.keys().all(|p| {
        let top = p.components().next().unwrap().as_os_str().to_str().unwrap();
        !NOT_COPIED.contains(&top)
    }));

    let all = list_in(&home, &data);
    assert_eq!(all[0].name, "Main");
    let cozy = find_in(&all, "pack:cozy").unwrap();
    assert_eq!(cozy.group.as_deref(), Some("Modded"));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn names_stay_unique_and_packs_are_not_copied() {
    let root = scratch("names");
    let (home, data) = (root.join("sunrise"), root.join("launcher"));
    player_home(&home);
    create_in(&home, &data, "Survival", None).unwrap();
    assert_eq!(
        create_in(&home, &data, "survival", None).unwrap_err(),
        "There is already a profile called survival. Choose another name."
    );
    assert!(create_in(&home, &data, "  ", None).is_err());
    // Another name with the same folder name gets the next free folder.
    let second = create_in(&home, &data, "Survival!", None).unwrap();
    assert_eq!(second.id, "survival-2");
    assert_eq!(second.name, "Survival!");
    assert!(edit_in(&home, &data, "survival-2", |m| m.name =
        Some("SURVIVAL".into()))
    .is_err());
    let port = find_in(&list_in(&home, &data), "pack:cozy").unwrap();
    assert!(create_in(&home, &data, "Pack copy", Some(&port)).is_err());
    assert!(!home.join("instances/pack-copy").exists());
    // An empty group is no group.
    let grouped = edit_in(&home, &data, "Survival", |m| m.group = Some(" ".into())).unwrap();
    assert_eq!(grouped.group, None);
    // An import named like an instance: both can still change everything but their names.
    let mut store = Store::load(&data);
    store.instances.entry(port.id.clone()).or_default().name = Some("Survival".into());
    store.save(&data).unwrap();
    let launched = edit_in(&home, &data, "survival", |m| {
        m.launch.wrapper = "nice".into()
    })
    .unwrap();
    assert_eq!(launched.launch.wrapper, "nice");
    assert!(edit_in(&home, &data, "survival-2", |m| m.name =
        Some("survival".into()))
    .is_err());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn an_instance_follows_the_players_answers() {
    let root = scratch("follow");
    let (main, own) = (root.join("main"), root.join("own"));
    put(&main.join("options.txt"), "fov:90\n");
    put(&main.join("sunrise-player.txt"), "name:Player123\n");
    put(&main.join("telemetry/consent.json"), r#"{"version":1}"#);
    crate::no_ai::set(&main, true).unwrap();
    put(&own.join("options.txt"), "fov:70\n");
    follow_player(&main, &own);
    assert_eq!(
        fs::read_to_string(own.join("options.txt")).unwrap(),
        "fov:70\n"
    );
    assert_eq!(
        fs::read_to_string(own.join("sunrise-player.txt")).unwrap(),
        "name:Player123\n"
    );
    assert_eq!(
        fs::read_to_string(own.join("telemetry/consent.json")).unwrap(),
        r#"{"version":1}"#
    );
    assert!(crate::no_ai::is_on(&own));
    crate::no_ai::set(&main, false).unwrap();
    follow_player(&main, &own);
    assert!(!crate::no_ai::is_on(&own));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn icons_are_a_texture_name_or_a_colour() {
    assert_eq!(Icon::parse("#3FA7D6"), Ok(Icon::Colour("#3fa7d6".into())));
    assert_eq!(
        Icon::parse("grass_block"),
        Ok(Icon::Texture("grass_block".into()))
    );
    for bad in ["#12345", "../x", "Grass", "", "#zzzzzz"] {
        assert!(Icon::parse(bad).is_err(), "{bad}");
    }
    let root = scratch("icon");
    put(
        &root.join("assets/minecraft/textures/item/diamond.png"),
        "png",
    );
    let diamond = Icon::Texture("diamond".into()).texture(&root).unwrap();
    assert!(diamond.ends_with("item/diamond.png"));
    assert_eq!(Icon::Texture("stone".into()).texture(&root), None);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn an_instance_keeps_its_own_version_until_it_follows_updates_again() {
    let root = scratch("version");
    let (home, data) = (root.join("sunrise"), root.join("launcher"));
    player_home(&home);
    create_in(&home, &data, "Old world", None).unwrap();
    let pinned = edit_in(&home, &data, "Old world", |m| {
        m.version = Some("v0.1.0".into())
    })
    .unwrap();
    assert_eq!(pinned.version.as_deref(), Some("v0.1.0"));
    // Renaming keeps it; the other instances follow the launcher's version.
    let renamed = edit_in(&home, &data, "old-world", |m| m.name = Some("Older".into())).unwrap();
    assert_eq!(renamed.version.as_deref(), Some("v0.1.0"));
    assert!(list_in(&home, &data)
        .iter()
        .filter(|i| i.id != "old-world")
        .all(|i| i.version.is_none()));
    let follows = edit_in(&home, &data, "Older", |m| m.version = None).unwrap();
    assert_eq!(follows.version, None);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn play_time_adds_up_per_instance_and_survives_a_rename() {
    let root = scratch("play-time");
    let (home, data) = (root.join("home"), root.join("data"));
    fs::create_dir_all(home.join(OWN).join("skyblock")).unwrap();
    let ended = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    record_play_in(&data, "skyblock", Duration::from_secs(25 * 60 + 10), ended).unwrap();
    edit_in(&home, &data, "skyblock", |m| m.name = Some("Sky".into())).unwrap();
    let later = ended + Duration::from_secs(3600);
    record_play_in(&data, "skyblock", Duration::from_secs(40 * 60), later).unwrap();
    let all = list_in(&home, &data);
    let sky = find_in(&all, "Sky").unwrap();
    assert_eq!(sky.played, Duration::from_secs(65 * 60 + 10));
    assert_eq!(sky.last_played, Some(later));
    let default = find_in(&all, DEFAULT).unwrap();
    assert_eq!(
        (default.played, default.last_played),
        (Duration::ZERO, None)
    );
    // Never played: nothing written for it.
    let text = fs::read_to_string(data.join(FILE)).unwrap();
    assert!(!text.contains("sunrise"), "{text}");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn play_time_reads_as_minutes_then_hours() {
    let t = |s| play_time(Duration::from_secs(s));
    assert_eq!(t(59), "under a minute");
    assert_eq!(t(12 * 60 + 59), "12 min");
    assert_eq!(t(3 * 3600 + 5 * 60), "3 h 05 min");
    assert_eq!(t(140 * 3600 + 7 * 60), "140 h");
}

/// An instance an earlier launcher made for Minecraft: Java Edition is not listed, and it is not
/// touched: its folder and its whole record in instances.json come through listing, creating and
/// editing other instances, and through a save, exactly as they were.
#[test]
fn a_java_instance_survives_listing_and_saving_unchanged() {
    let root = scratch("java");
    let (home, data) = (root.join("sunrise"), root.join("launcher"));
    put(
        &home.join("instances/vanilla/saves/World/level.dat"),
        "world",
    );
    put(&home.join("instances/vanilla/mods/a.jar"), "jar");
    put(&home.join("instances/vanilla/options.txt"), "fov:90\n");
    put(&home.join("instances/back/saves/Other/level.dat"), "other");
    let record = serde_json::json!({
        "instances": {
            "vanilla": {
                "name": "Vanilla 1.21.4", "icon": {"colour": "#3fa7d6"}, "group": "Old",
                "played_secs": 7200, "last_played": 1_700_000_000,
                "launch": {"wrapper": "nice"},
                "java": {
                    "minecraft": "1.21.4", "home": "/opt/java", "memory_mb": 6144,
                    "jvm_args": "-XX:+UseZGC", "window": [1280, 720],
                    "loader": {"kind": "Fabric", "version": "0.16.9", "future": true}
                }
            },
            "back": {
                "name": "Back to Sunrise",
                "java_kept": {"minecraft": "1.20.1", "memory_mb": 2048}
            }
        }
    });
    fs::create_dir_all(&data).unwrap();
    let file = data.join("instances.json");
    fs::write(&file, record.to_string()).unwrap();
    let folders = files(&home.join("instances"));

    // Listing skips the Java instance; one that only keeps Java settings aside is a Sunrise one.
    let all = list_in(&home, &data);
    let ids: Vec<&str> = all.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, [DEFAULT, "back"]);

    // Saving other instances rewrites the file: the Java record is the same value after it.
    create_in(&home, &data, "Survival", None).unwrap();
    edit_in(&home, &data, "back", |m| m.group = Some("Mine".into())).unwrap();
    let after: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(
        after["instances"]["vanilla"],
        record["instances"]["vanilla"]
    );
    assert_eq!(
        after["instances"]["back"]["java_kept"],
        record["instances"]["back"]["java_kept"]
    );
    assert_eq!(after["instances"]["back"]["group"], "Mine");
    // The Java instance's folder, with its worlds and mods, is byte for byte as it was, and a new
    // instance never takes its name or folder.
    let now = files(&home.join("instances"));
    for (path, bytes) in &folders {
        assert_eq!(now.get(path), Some(bytes), "{} changed", path.display());
    }
    assert_eq!(
        create_in(&home, &data, "Vanilla", None).unwrap().home,
        home.join("instances/vanilla-2")
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn deleted_instances_go_to_the_trash_and_are_forgotten() {
    let root = scratch("delete");
    let (home, data, trash) = (root.join("home"), root.join("data"), root.join("trash"));
    player_home(&home);
    let to_trash = |p: &Path| crate::desktop_seams::to_folder(p, &trash);
    let survival = create_in(&home, &data, "Survival", None).unwrap();
    put(&survival.home.join("saves/Base/level.dat"), "base");
    edit_in(&home, &data, "Survival", |m| m.group = Some("Mine".into())).unwrap();
    let gone = delete_in(&home, &data, "Survival", &to_trash).unwrap();
    assert_eq!(gone.id, "survival");
    assert_eq!(
        fs::read(trash.join("survival/saves/Base/level.dat")).unwrap(),
        b"base"
    );
    assert!(find_in(&list_in(&home, &data), "Survival").is_err());
    assert!(!Store::load(&data).instances.contains_key("survival"));
    // The game's home stays.
    assert!(delete_in(&home, &data, DEFAULT, &to_trash)
        .unwrap_err()
        .detail
        .contains("can't be deleted"));
    assert!(home.join("saves/World/level.dat").is_file());
    // A folder removed by hand: its choices are dropped, a new one of that name starts clean.
    create_in(&home, &data, "Skyblock", None).unwrap();
    edit_in(&home, &data, "Skyblock", |m| m.group = Some("Old".into())).unwrap();
    fs::remove_dir_all(home.join(OWN).join("skyblock")).unwrap();
    assert!(find_in(&list_in(&home, &data), "Skyblock").is_err());
    assert!(!Store::load(&data).instances.contains_key("skyblock"));
    let again = create_in(&home, &data, "Skyblock", None).unwrap();
    assert_eq!(again.group, None);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_failed_selection_save_does_not_undo_the_confirmed_delete() {
    let root = tempfile::tempdir().unwrap();
    let (game, data, trash) = (
        root.path().join("game"),
        root.path().join("data"),
        root.path().join("trash"),
    );
    let profile = create_in(&game, &data, "Survival", None).unwrap();
    backend::Settings {
        instance: Some(profile.id.clone()),
        ..Default::default()
    }
    .save_to(&data)
    .unwrap();
    // Leave the old selection readable but make the fenced write fail.
    let gate = data.join(".settings.json.lock");
    fs::remove_file(&gate).unwrap();
    fs::create_dir(&gate).unwrap();
    let gone = delete_selected(&game, &data, profile, &|path| {
        crate::desktop_seams::to_folder(path, &trash)
    })
    .unwrap();
    let mut live = backend::Settings::load_from(&data);
    assert!(clear_deleted_selection(&mut live, &gone.id));
    assert!(live.save_to(&data).is_err());
    assert!(trash.join("survival").is_dir());
    assert!(!gone.home.exists());
}

#[test]
fn a_profile_lookup_pairs_its_old_row_with_its_old_directory() {
    let root = scratch("lookup-snapshot");
    let (game, cache, trash) = (root.join("game"), root.join("cache"), root.join("trash"));
    let old = create_in(&game, &cache, "Castle!", None).unwrap();
    let snapshot = list_with_snapshot(&game, &cache, true, || {
        // Pause after reading the old row, before opening any profile directory. This is
        // the exact lock held across the competing Delete and Create directory renames.
        let competitor = fs::File::options()
            .read(true)
            .write(true)
            .open(cache.join(".instances.lock"))
            .unwrap();
        assert!(matches!(
            competitor.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
    });
    let selected = find_in(&snapshot, "Castle!").unwrap();
    assert_eq!(selected.incarnation, old.incarnation);
    delete_in(&game, &cache, "Castle!", &|path| {
        fs::rename(path, &trash).map_err(|e| e.to_string())
    })
    .unwrap();
    let replacement = create_in(&game, &cache, "Castle?", None).unwrap();
    assert_eq!(selected.id, replacement.id);
    assert_ne!(selected.incarnation, replacement.incarnation);
    put(
        &replacement.home.join("saves/keep/level.dat"),
        "replacement",
    );
    assert_eq!(
        delete_selected(&game, &cache, selected, &|_| panic!(
            "replacement reached Trash"
        ))
        .unwrap_err()
        .detail,
        lifecycle::changed()
    );
    assert_eq!(
        fs::read_to_string(replacement.home.join("saves/keep/level.dat")).unwrap(),
        "replacement"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn play_completion_waits_for_the_departing_games_profile_gate() {
    let root = scratch("exit-gate");
    let (game, cache) = (root.join("game"), root.join("cache"));
    let accepted = create_in(&game, &cache, "Survival", None).unwrap();
    let owner = ManagedProfile::open_instance(&game, &cache, &accepted).unwrap();
    let elapsed = Duration::from_secs(73);
    std::thread::scope(|scope| {
        scope.spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            drop(owner);
        });
        finish_play_in(&game, &cache, &accepted, elapsed, || {
            assert!(ManagedProfile::open_instance(&game, &cache, &accepted).is_err());
            put(&accepted.home.join("owned-hook"), "ran after release");
        })
        .unwrap();
    });
    assert_eq!(
        find_in(&list_in(&game, &cache), &accepted.id)
            .unwrap()
            .played,
        elapsed
    );
    assert_eq!(
        fs::read_to_string(accepted.home.join("owned-hook")).unwrap(),
        "ran after release"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_delayed_play_completion_never_records_time_or_runs_a_hook_for_a_replacement() {
    let root = scratch("delayed-exit");
    let (game, cache, trash) = (root.join("game"), root.join("cache"), root.join("trash"));
    let accepted = create_in(&game, &cache, "Rocket!", None).unwrap();
    let elapsed = Duration::from_secs(73);
    // Model a delivered exit using the same completion routine as the GUI and CLI. The hook
    // also asserts it runs while the accepted profile is still admitted.
    finish_play_in(&game, &cache, &accepted, elapsed, || {
        assert!(ManagedProfile::open_instance(&game, &cache, &accepted).is_err());
        put(&accepted.home.join("owned-hook"), "old profile");
    })
    .unwrap();
    assert_eq!(
        find_in(&list_in(&game, &cache), &accepted.id)
            .unwrap()
            .played,
        elapsed
    );
    delete_in(&game, &cache, &accepted.id, &|dir| {
        fs::rename(dir, &trash).map_err(|e| e.to_string())
    })
    .unwrap();
    let replacement = create_in(&game, &cache, "Rocket?", None).unwrap();
    assert_eq!(accepted.id, replacement.id);
    assert_ne!(accepted.incarnation, replacement.incarnation);
    let before = fs::read(cache.join(FILE)).unwrap();
    let result = finish_play_in(&game, &cache, &accepted, elapsed, || {
        panic!("the detached run's hook reached a replacement profile");
    });
    assert_eq!(result.unwrap_err(), lifecycle::changed());
    assert_eq!(fs::read(cache.join(FILE)).unwrap(), before);
    assert_eq!(
        find_in(&list_in(&game, &cache), &replacement.id)
            .unwrap()
            .played,
        Duration::ZERO
    );
    assert!(!replacement.home.join("owned-hook").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_delayed_delete_does_not_admit_a_replacement_at_the_same_slug() {
    let root = scratch("delete-old-incarnation");
    let (game, cache, trash) = (root.join("game"), root.join("cache"), root.join("trash"));
    let selected = create_in(&game, &cache, "Survival", None).unwrap();
    let owner = ManagedProfile::open_instance(&game, &cache, &selected).unwrap();
    assert!(delete_in(&game, &cache, "survival", &|_| panic!(
        "live profile reached Trash"
    ))
    .unwrap_err()
    .detail
    .contains("in use"));
    drop(owner);
    delete_in(&game, &cache, "survival", &|p| {
        crate::desktop_seams::to_folder(p, &trash)
    })
    .unwrap();
    let replacement = create_in(&game, &cache, "Survival", None).unwrap();
    put(
        &replacement.home.join("saves/New/level.dat"),
        "a different world",
    );
    assert_ne!(selected.incarnation, replacement.incarnation);
    let error = delete_selected(&game, &cache, selected, &|_| {
        panic!("stale Delete reached Trash")
    })
    .unwrap_err();
    assert!(
        error.detail.contains("changed since you reviewed"),
        "{error}"
    );
    assert_eq!(
        fs::read(replacement.home.join("saves/New/level.dat")).unwrap(),
        b"a different world"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_replacement_detached_after_admission_is_restored_without_overwriting() {
    for name_reused in [false, true] {
        let root = scratch("replacement-at-detach");
        let (game, cache) = (root.join("game"), root.join("cache"));
        let selected = create_in(&game, &cache, "Survival!", None).unwrap();
        let visible = selected.home.clone();
        let displaced = root.join("displaced");
        put(&visible.join("original"), "first profile");
        let before = fs::read(cache.join(FILE)).unwrap();
        let mut retained = None;
        let error = delete_selected_at(
            &game,
            &cache,
            selected,
            |from, to| {
                fs::rename(from, &displaced)?;
                put(&from.join("saves/Mine/level.dat"), "late replacement");
                durable::move_new_directory(from, to)?;
                if name_reused {
                    put(&from.join("new-occupant"), "third profile");
                }
                retained = Some(to.to_owned());
                Ok(())
            },
            &|_| panic!("late replacement reached Trash"),
        )
        .unwrap_err();
        let retained = retained.unwrap();
        let replacement = if name_reused {
            assert_eq!(
                error.retained.as_ref().map(InterruptedDeletion::folder),
                Some(retained.as_path())
            );
            assert_eq!(
                fs::read(visible.join("new-occupant")).unwrap(),
                b"third profile"
            );
            retained
        } else {
            assert_eq!(error.detail, lifecycle::changed());
            assert!(error.retained.is_none());
            assert!(!retained.parent().unwrap().exists());
            visible
        };
        assert_eq!(
            fs::read(replacement.join("saves/Mine/level.dat")).unwrap(),
            b"late replacement"
        );
        assert_eq!(
            fs::read(displaced.join("original")).unwrap(),
            b"first profile"
        );
        assert_eq!(fs::read(cache.join(FILE)).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn profile_snapshot_refuses_a_busy_world_created_after_enumeration() {
    for existing_saves in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("game");
        let saves = home.join("saves");
        fs::create_dir(&home).unwrap();
        if existing_saves {
            put(&saves.join("Old/level.dat"), "already there");
        }
        let claims = crate::worlds::SavedWorlds::claim(&saves).unwrap();
        let late = saves.join("Late");
        put(&late.join("level.dat"), "game is still writing this world");
        let game = fs::File::create(late.join(crate::worlds::LOCK)).unwrap();
        game.try_lock().unwrap();
        let stage = root.path().join("private");
        let error = copy_home(&home, &stage, &claims).unwrap_err();
        assert!(error.to_string().contains("World Late changed"), "{error}");
        assert!(!stage.join("saves/Late/level.dat").exists());
        assert_eq!(
            fs::read(late.join("level.dat")).unwrap(),
            b"game is still writing this world"
        );
    }
}

#[test]
fn a_metadata_write_failure_after_the_rename_reports_the_retained_folder() {
    let root = tempfile::tempdir().unwrap();
    let (game, data) = (root.path().join("game"), root.path().join("data"));
    let profile = create_in(&game, &data, "Survival", None).unwrap();
    put(&profile.home.join("saves/Base/level.dat"), "base");
    let home = profile.home.clone();
    let failure = delete_selected_at(
        &game,
        &data,
        profile,
        |from, to| {
            durable::move_new_directory(from, to)?;
            // The metadata path was readable before detach. Make its atomic write fail now.
            fs::remove_file(data.join(FILE))?;
            fs::create_dir(data.join(FILE))
        },
        &|_| panic!("the move to Trash must not run after a metadata error"),
    )
    .unwrap_err();
    let text = failure.to_string();
    let retained = failure
        .retained
        .expect("the location of the detached profile");
    assert!(text.contains(&*retained.folder().to_string_lossy()));
    assert!(!home.exists());
    assert_eq!(
        fs::read(retained.folder().join("saves/Base/level.dat")).unwrap(),
        b"base"
    );
    assert!(failure.detail.contains(FILE));
}

#[test]
fn a_trash_failure_preserves_the_whole_detached_profile() {
    let root = scratch("failed-trash");
    let (game, cache) = (root.join("game"), root.join("cache"));
    let selected = create_in(&game, &cache, "Survival", None).unwrap();
    put(
        &selected.home.join("saves/Mine/level.dat"),
        "keep this world",
    );
    let kept = std::cell::OnceCell::new();
    let error = delete_in(&game, &cache, "survival", &|path| {
        kept.set(path.to_owned()).unwrap();
        Err("trash unavailable".into())
    })
    .unwrap_err();
    assert!(error.detail.contains("trash unavailable"));
    let kept = kept.into_inner().unwrap();
    assert_eq!(
        error.retained.as_ref().map(InterruptedDeletion::folder),
        Some(kept.as_path())
    );
    assert!(!selected.home.exists());
    assert!(deletion_recovery::interrupted_in(&game).is_empty());
    drop(error); // Quit before the result is delivered or while it remains open.
    let mut notices = deletion_recovery::settled_scan(&game, |found| found.len() == 1);
    assert_eq!(notices.len(), 1);
    let notice = notices.pop().unwrap();
    // The worker canonicalizes its root; startup can see a Windows short-name alias.
    assert_eq!(
        notice.folder().canonicalize().unwrap(),
        kept.canonicalize().unwrap()
    );
    assert_eq!(notice.id(), selected.id);
    notice.acknowledge();
    assert!(deletion_recovery::interrupted_in(&game).is_empty());
    assert_eq!(
        fs::read(kept.join("saves/Mine/level.dat")).unwrap(),
        b"keep this world"
    );
    assert!(create_in(&game, &cache, "Survival", None).is_ok());
    fs::remove_dir_all(root).unwrap();
}

/// A chosen picture is kept scaled to 128 pixels in the instance's folder; removing it moves it
/// to the trash; a file that isn't an image is refused.
#[test]
fn a_picture_is_kept_small_and_goes_to_the_trash() {
    let root = scratch("picture");
    let (home, trash) = (root.join("home"), root.join("trash"));
    let mut photo = Vec::new();
    image::RgbaImage::from_pixel(600, 400, image::Rgba([200, 90, 40, 255]))
        .write_to(
            &mut std::io::Cursor::new(&mut photo),
            image::ImageFormat::Png,
        )
        .unwrap();
    let png = picture(photo).unwrap();
    let to_trash = |p: &Path| crate::desktop_seams::to_folder(p, &trash);
    set_picture(&home, Some(&png), &to_trash).unwrap();
    let kept = image::open(home.join(PICTURE)).unwrap();
    assert_eq!((kept.width(), kept.height()), (128, 85));
    set_picture(&home, None, &to_trash).unwrap();
    assert!(!home.join(PICTURE).exists());
    assert!(trash.join(PICTURE).is_file());
    // Nothing to remove is fine.
    set_picture(&home, None, &to_trash).unwrap();
    assert!(picture(b"not an image".to_vec()).is_err());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_profile_channel_schema_accepts_only_its_current_channel_names() {
    for (name, channel) in [
        ("stable", Channel::Stable),
        ("nightly", Channel::Nightly),
        ("dev", Channel::Dev),
    ] {
        let saved = serde_json::json!(name);
        assert_eq!(
            serde_json::from_value::<Channel>(saved.clone()).unwrap(),
            channel
        );
        assert_eq!(serde_json::to_value(channel).unwrap(), saved);
    }
    let error = serde_json::from_value::<Channel>(serde_json::json!("preview"))
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        "unknown variant `preview`, expected one of `stable`, `nightly`, `dev`"
    );
}

#[test]
fn an_instance_without_a_channel_is_nightly_and_an_unknown_one_too() {
    let meta: Meta = serde_json::from_str("{}").unwrap();
    assert_eq!(meta.channel.unwrap_or_default(), Channel::Nightly);
    let meta: Meta = serde_json::from_str(r#"{"channel":"stable","group":"g"}"#).unwrap();
    assert_eq!(meta.channel, Some(Channel::Stable));
    let dev: Meta = serde_json::from_str(r#"{"channel":"dev"}"#).unwrap();
    assert_eq!(dev.channel, Some(Channel::Dev));
    assert_eq!(serde_json::to_value(dev).unwrap()["channel"], "dev");
    let meta: Meta = serde_json::from_str(r#"{"channel":"beta","group":"g"}"#).unwrap();
    assert_eq!((meta.channel, meta.group.as_deref()), (None, Some("g")));
}

/// A saved selection that names a ported pack (listed by an earlier launcher, its folder still in
/// `ports/`) is retired: a command with no `--instance` refuses and names it instead of acting
/// on Sunrise's setup, and the window clears it. Listed and unset selections are untouched.
#[test]
fn a_retired_selection_is_refused_not_replaced_by_sunrise() {
    let root = scratch("retired");
    let (home, data) = (root.join("sunrise"), root.join("launcher"));
    put(&home.join("ports/old-pack/saves/World/level.dat"), "world");
    let ported = files(&home.join("ports"));
    let all = list_in(&home, &data);
    assert!(all.iter().all(|i| !i.id.contains("old-pack")));

    let refused = selection_in(&all, Some("port:old-pack")).unwrap_err();
    assert!(refused.contains("old-pack"), "{refused}");
    assert!(refused.contains("instance select"), "{refused}");
    assert_eq!(
        retired_selection(&all, Some("port:old-pack")).as_deref(),
        Some("port:old-pack")
    );

    let made = create_in(&home, &data, "Survival", None).unwrap();
    let all = list_in(&home, &data);
    assert_eq!(selection_in(&all, None).unwrap().id, DEFAULT);
    assert_eq!(selection_in(&all, Some(DEFAULT)).unwrap().id, DEFAULT);
    assert_eq!(selection_in(&all, Some(&made.id)).unwrap().id, made.id);
    assert_eq!(retired_selection(&all, Some(&made.id)), None);
    assert_eq!(retired_selection(&all, None), None);
    assert_eq!(
        files(&home.join("ports")),
        ported,
        "its files are untouched"
    );
    fs::remove_dir_all(&root).unwrap();
}

/// The same for a Java instance an earlier launcher made: it isn't listed, so a saved selection of
/// it is retired and refused by name, not read as Sunrise.
#[test]
fn a_retired_java_selection_is_refused_by_name() {
    let root = scratch("retired-java");
    let (home, data) = (root.join("sunrise"), root.join("launcher"));
    put(
        &home.join("instances/vanilla/saves/World/level.dat"),
        "world",
    );
    fs::create_dir_all(&data).unwrap();
    fs::write(
        data.join("instances.json"),
        r#"{"instances":{"vanilla":{"name":"Vanilla 1.21.4","java":{"minecraft":"1.21.4"}}}}"#,
    )
    .unwrap();
    let all = list_in(&home, &data);
    assert!(all.iter().all(|i| i.id != "vanilla"));
    let refused = selection_in(&all, Some("vanilla")).unwrap_err();
    assert!(refused.contains("vanilla"), "{refused}");
    assert_eq!(
        retired_selection(&all, Some("vanilla")).as_deref(),
        Some("vanilla")
    );
    assert_eq!(selection_in(&all, None).unwrap().id, DEFAULT);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_last_played_no_calendar_holds_reads_as_never_played() {
    let root = scratch("far-future");
    let (home, data) = (root.join("home"), root.join("data"));
    fs::create_dir_all(home.join(OWN).join("far")).unwrap();
    fs::create_dir_all(home.join(OWN).join("near")).unwrap();
    // Both fit a u64 of seconds; only one is a date.
    put(
        &data.join(FILE),
        &format!(
            r#"{{"instances": {{
                "sunrise": {{"last_played": {}, "played_secs": 60}},
                "far": {{"last_played": 9223372036854775807, "name": "Far"}},
                "near": {{"last_played": 1763424000, "name": "Near"}}}}}}"#,
            u64::MAX
        ),
    );
    let all = list_in(&home, &data);
    let played = |id: &str| all.iter().find(|i| i.id == id).unwrap().last_played;
    assert_eq!(played("sunrise"), None);
    assert_eq!(played("far"), None);
    let near = played("near").unwrap();
    assert_eq!(near, UNIX_EPOCH + Duration::from_secs(1_763_424_000));
    // What Home does with one: every time kept converts to a local date without a panic.
    for kept in [near, calendar_time(LATEST).unwrap()] {
        let _: chrono::DateTime<chrono::Local> = kept.into();
    }
    assert_eq!(calendar_time(LATEST + Duration::from_secs(1)), None);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_creation_that_fails_leaves_no_instance_and_the_name_free() {
    let root = scratch("create-fails");
    let (home, data) = (root.join("home"), root.join("data"));
    player_home(&home);
    let all = list_in(&home, &data);
    let before = files(&home);
    // The choices can't be saved: instances.json is a folder.
    fs::create_dir_all(data.join(FILE)).unwrap();
    for from in [None, Some(&all[0])] {
        let failed = create_whole_in(&home, &data, "Skyblock", from, Some(Some(b"png")), |m| {
            m.group = Some("Mine".into())
        });
        assert!(failed.is_err());
        assert!(!home.join(OWN).join("skyblock").exists());
        assert!(fs::read_dir(home.join(STAGING)).unwrap().next().is_none());
        assert_eq!(files(&home), before, "nothing of the attempt stays");
    }
    fs::remove_dir(data.join(FILE)).unwrap();
    assert_eq!(list_in(&home, &data).len(), all.len());

    // The same name again, now whole: its folder, picture and every choice at once.
    let made = create_whole_in(
        &home,
        &data,
        "Skyblock",
        Some(&all[0]),
        Some(Some(b"png")),
        |m| {
            m.group = Some("Mine".into());
            m.version = Some("nightly-2026-11-13-aaaaaaa".into());
        },
    )
    .unwrap();
    assert_eq!(made.id, "skyblock");
    assert_eq!(made.group.as_deref(), Some("Mine"));
    assert_eq!(made.version.as_deref(), Some("nightly-2026-11-13-aaaaaaa"));
    assert_eq!(fs::read(made.home.join(PICTURE)).unwrap(), b"png");
    assert!(made.home.join("saves/World/level.dat").is_file());
    assert!(fs::read_dir(home.join(STAGING)).unwrap().next().is_none());
    // A copy asked to have no picture doesn't keep the one it copied.
    let plain = create_whole_in(&home, &data, "Plain", Some(&made), Some(None), |_| {}).unwrap();
    assert!(!plain.home.join(PICTURE).exists());
    assert!(made.home.join(PICTURE).is_file());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_kill_after_copy_visibility_keeps_the_exact_copy_and_recovers_its_choices() {
    use crate::managed_play::{Active, FirstBuild, Held, HeldReason, HeldRoot, HeldSource};
    use sha2::{Digest as _, Sha256};
    use std::{process::Command, time::Instant};
    use sunrise_launcher_smod::{manifest, mod_list};
    const TEST: &str = "instances::tests::a_kill_after_copy_visibility_keeps_the_exact_copy_and_recovers_its_choices";
    const ROLE: &str = "SUNRISE_COPY_TEST_ROLE";
    const BUILD: &str = "v0.1.7";
    const POINTER: &str = "mod-browser/active-build.json";
    const ENABLED: &str = sunrise_launcher_smod::mod_list::FILE;
    let Ok(role) = std::env::var(ROLE) else {
        let root = tempfile::tempdir().unwrap();
        let game = root.path().join("game");
        let cache = root.path().join("launcher");
        let actor = |role| {
            let mut cmd = Command::new(std::env::current_exe().unwrap());
            cmd.args(["--exact", TEST, "--nocapture"])
                .env(ROLE, role)
                .env("SUNRISE_HOME", &game)
                .env("SUNRISE_LAUNCHER_HOME", &cache)
                .env("XDG_CONFIG_HOME", root.path().join("config"));
            cmd
        };
        assert!(actor("input").status().unwrap().success());
        let mut copy = actor("copy").spawn().unwrap();
        let ready = root.path().join("visible");
        let until = Instant::now() + Duration::from_secs(10);
        while !ready.is_file() {
            if copy.try_wait().unwrap().is_some() || Instant::now() > until {
                let _ = copy.kill();
                let _ = copy.wait();
                panic!("copy did not reach visibility");
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        copy.kill().unwrap();
        assert!(!copy.wait().unwrap().success());
        let source = game.join("instances/source");
        let visible = game.join("instances/new-copy");
        assert!(!Store::read(&cache)
            .unwrap()
            .instances
            .contains_key("new-copy"));
        assert_eq!(
            fs::read(visible.join(ENABLED)).unwrap(),
            fs::read(source.join(ENABLED)).unwrap()
        );
        assert_eq!(
            fs::read(visible.join("saves/Mine/level.dat")).unwrap(),
            b"copied world"
        );
        let pointer = |home: &Path| {
            serde_json::from_slice::<serde_json::Value>(&fs::read(home.join(POINTER)).unwrap())
                .unwrap()
        };
        let old = pointer(&source);
        let new = pointer(&visible);
        for field in ["base", "executable", "inputs", "resources"] {
            assert_eq!(new[field], old[field], "{field}");
        }
        assert_eq!(old["receipts"].as_array().unwrap().len(), 1);
        assert_eq!(new["receipts"], serde_json::json!([]));
        assert_eq!(new["generation"], 1);
        assert_ne!(new["profile"], old["profile"]);
        let recovered = find_in(&list_in(&game, &cache), "new-copy").unwrap();
        assert_eq!(recovered.group.as_deref(), Some("Kept after exit"));
        assert_eq!(recovered.version.as_deref(), Some(BUILD));
        assert!(Store::read(&cache)
            .unwrap()
            .instances
            .contains_key("new-copy"));

        let source_roots = crate::data_location::Roots {
            launcher: cache.clone(),
            game: game.clone(),
            build: cache.join("native-build"),
        };
        let relocated = crate::data_location::migrate(
            &source_roots,
            &root.path().join("new-location"),
            &root.path().join("location.json"),
            |_| {},
        )
        .unwrap();
        assert_eq!(pointer(&relocated.game.join("instances/source")), old);
        assert_eq!(pointer(&relocated.game.join("instances/new-copy")), new);
        assert!(
            source.join(POINTER).is_file(),
            "original data remains the backup"
        );
        let roster: serde_json::Value = serde_json::from_slice(
            &fs::read(relocated.launcher.join("profile-artifact-roots.json")).unwrap(),
        )
        .unwrap();
        let entries = roster["roots"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0][0],
            serde_json::to_value(relocated.game.canonicalize().unwrap()).unwrap()
        );
        drop(
            ReferencePublication::begin_at(&relocated.launcher, &relocated.game, &relocated.game)
                .unwrap(),
        );
        return;
    };
    let game = backend::game_home();
    let cache = backend::data_dir();
    match role.as_str() {
        "input" => {
            let source = create_in(&game, &cache, "Source", None).unwrap();
            let preview = CopyPreview::read(&source).unwrap();
            assert_eq!(preview.base(), None);
            let folder = backend::version_dir(BUILD);
            let resources = backend::resources(BUILD);
            fs::create_dir_all(&resources).unwrap();
            fs::write(resources.join("sunrise-resources.json"), b"{}").unwrap();
            fs::write(folder.join(".installed"), []).unwrap();
            fs::write(folder.join(backend::CLIENT), "fixture client").unwrap();
            put(&source.home.join(ENABLED), "\n");
            put(&source.home.join("saves/Mine/level.dat"), "copied world");
            let kept = format!("{}/example.smod", mod_list::NATIVE_DIR);
            let package = manifest::Package::write_files(
                &format!("id='example'\nname='Example'\nversion='1.0.0'\napi='{}'\ncapabilities=['registries']\n[native]\nsunrise='{}'\ngame='26.3'\n", manifest::API, "a".repeat(40)),
                &[("Cargo.toml", b"[package]\nname='example'\nversion='1.0.0'"), ("src/lib.rs", b"")],
            ).unwrap();
            fs::create_dir_all(source.home.join(mod_list::NATIVE_DIR)).unwrap();
            fs::write(source.home.join(&kept), &package).unwrap();
            let held = Held::new(
                "example".into(),
                "Example".into(),
                Sha256::digest(&package).into(),
                (HeldRoot::Native, "example.smod".into()),
                HeldSource::Local {
                    version: "1.0.0".into(),
                },
                true,
                HeldReason::NoFit {
                    detail: None,
                    build: BUILD.into(),
                },
            )
            .unwrap();
            let mut managed = ManagedProfile::open_instance(&game, &cache, &source).unwrap();
            let first = managed
                .begin_first(|| FirstBuild::installed(BUILD), |_| {})
                .unwrap();
            first.prepare(&|_| {}).unwrap();
            let cancellation = crate::native_mods::Cancellation::default();
            let image = first
                .exact_image(
                    crate::native_mods::Profile::Optimized,
                    &cancellation,
                    |_| {},
                    None,
                )
                .unwrap();
            let active = first.commit(&image, &cancellation).unwrap();
            active.verify_artifacts().unwrap();
            let mut exact = serde_json::to_value(active).unwrap();
            let receipt = [7u8; 16];
            exact["receipts"] = serde_json::json!([{
                "id": receipt, "waiting_for_update": [held], "removed": [], "acknowledged": false,
            }]);
            let bytes = serde_json::to_vec(&exact).unwrap();
            Active::parse(&bytes).unwrap();
            durable::replace(&source.home, &source.home.join(POINTER), &bytes).unwrap();
            drop(managed);
            let stale = create_with_publish(
                &game,
                &cache,
                "Old Preview",
                Some(CopyFrom::Preview(&preview)),
                None,
                |_| {},
                |_| panic!("a changed binding was published"),
            );
            assert_eq!(stale.unwrap_err(), lifecycle::changed());
            assert!(!game.join("instances/old-preview").exists());
        }
        "copy" => {
            let source = find_in(&list_in(&game, &cache), "source").unwrap();
            let preview = CopyPreview::read(&source).unwrap();
            assert_eq!(preview.base(), Some(BUILD));
            create_with_publish(
                &game,
                &cache,
                "New Copy",
                Some(CopyFrom::Preview(&preview)),
                None,
                |meta| {
                    meta.group = Some("Kept after exit".into());
                    meta.version = Some("not-the-copied-build".into());
                },
                |visible| {
                    assert!(visible.join(POINTER).is_file());
                    fs::write(game.with_file_name("visible"), []).unwrap();
                    loop {
                        std::thread::park();
                    }
                },
            )
            .unwrap();
        }
        _ => panic!("unknown copy fixture role"),
    }
}

#[test]
fn creation_does_not_reuse_or_remove_another_attempts_temporary_paths() {
    let root = scratch("creation-owns-only-its-stage");
    let (home, data) = (root.join("home"), root.join("data"));
    let older_stage = home
        .join(STAGING)
        .join(format!("skyblock.{}", std::process::id()));
    let older_choices = data.join(format!("{FILE}.{}.part", std::process::id()));
    put(&older_stage.join("options.txt"), "somebody else's stage");
    put(&older_choices, "somebody else's choices");

    fs::create_dir(data.join(FILE)).unwrap();
    assert!(create_whole_in(&home, &data, "Skyblock", None, None, |_| {}).is_err());
    assert!(!home.join(OWN).join("skyblock").exists());
    fs::remove_dir(data.join(FILE)).unwrap();
    assert!(create_whole_in(&home, &data, "Skyblock", None, None, |_| {}).is_ok());
    assert_eq!(
        fs::read_to_string(older_stage.join("options.txt")).unwrap(),
        "somebody else's stage"
    );
    assert_eq!(
        fs::read_to_string(older_choices).unwrap(),
        "somebody else's choices"
    );
    assert_eq!(fs::read_dir(home.join(STAGING)).unwrap().count(), 1);
    assert!(data.join(FILE).is_file());
    assert!(data.join(".instances.lock").is_file());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn creation_refuses_linked_staging_and_destination_parents() {
    let root = scratch("creation-parent-links");
    let (home, data, outside) = (root.join("home"), root.join("data"), root.join("outside"));
    fs::create_dir(&home).unwrap();
    put(
        &outside.join(format!("skyblock.{}/keep.txt", std::process::id())),
        "keep me",
    );
    for parent in [STAGING, OWN] {
        let linked = home.join(parent);
        let _ = fs::remove_dir(&linked);
        std::os::unix::fs::symlink(&outside, &linked).unwrap();
        assert!(create_whole_in(&home, &data, "Skyblock", None, None, |_| {}).is_err());
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
        assert!(outside
            .join(format!("skyblock.{}/keep.txt", std::process::id()))
            .is_file());
        fs::remove_file(linked).unwrap();
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn same_name_profiles_cannot_erase_the_winning_copy() {
    let root = scratch("competing-profile-names");
    let (game, cache) = (root.join("game"), root.join("cache"));
    fs::create_dir(&game).unwrap();
    let cutoff = std::sync::Barrier::new(2);
    let attempt = |icon| {
        create_whole_in(&game, &cache, "One name", None, Some(Some(icon)), |_| {
            cutoff.wait();
        })
    };
    let (left, right) = std::thread::scope(|scope| {
        let left = scope.spawn(|| attempt(b"first candidate"));
        let right = attempt(b"second candidate");
        (left.join().unwrap(), right)
    });
    let (winner, picture): (_, &[u8]) = match (left, right) {
        (Ok(profile), Err(_)) => (profile, b"first candidate"),
        (Err(_), Ok(profile)) => (profile, b"second candidate"),
        outcomes => panic!("expected one winning publication: {outcomes:?}"),
    };
    assert_eq!(winner.id, "one-name");
    assert_eq!(fs::read(winner.home.join(PICTURE)).unwrap(), picture);
    assert_eq!(fs::read_dir(game.join(OWN)).unwrap().count(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn profile_publication_does_not_replace_an_empty_destination() {
    let root = scratch("creation-no-clobber");
    let (source, dest) = (root.join("source"), root.join("dest"));
    put(&source.join("options.txt"), "new");
    fs::create_dir(&dest).unwrap();
    assert!(durable::move_new_directory(&source, &dest).is_err());
    assert!(source.join("options.txt").is_file());
    assert!(fs::read_dir(&dest).unwrap().next().is_none());
    fs::remove_dir(dest).unwrap();
    assert!(durable::move_new_directory(&source, &root.join("dest")).is_ok());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_saved_texture_name_is_held_to_the_typed_ones_and_stays_inside_the_resources() {
    let root = scratch("saved-icon");
    let (home, data) = (root.join("home"), root.join("data"));
    for slug in ["escape", "odd", "fine", "tinted"] {
        fs::create_dir_all(home.join(OWN).join(slug)).unwrap();
    }
    put(
        &data.join(FILE),
        r##"{"instances": {
            "escape": {"icon": {"texture": "../../../../secret"}, "group": "Kept"},
            "odd": {"icon": {"colour": "red; x"}, "name": "Odd"},
            "fine": {"icon": {"texture": "diamond"}},
            "tinted": {"icon": {"colour": "#3fa7d6"}}}}"##,
    );
    let all = list_in(&home, &data);
    let of = |id: &str| all.iter().find(|i| i.id == id).unwrap();
    // The bad icon alone is dropped: the instance and its other choices stay.
    assert_eq!(of("escape").icon, None);
    assert_eq!(of("escape").group.as_deref(), Some("Kept"));
    assert_eq!(
        (of("odd").icon.clone(), of("odd").name.as_str()),
        (None, "Odd")
    );
    assert_eq!(of("fine").icon, Some(Icon::Texture("diamond".into())));
    assert_eq!(of("tinted").icon, Some(Icon::Colour("#3fa7d6".into())));

    // Even a name made in code reads nothing outside the textures.
    let resources = root.join("resources");
    put(
        &resources.join("assets/minecraft/textures/item/diamond.png"),
        "png",
    );
    put(&root.join("secret.png"), "secret");
    for name in ["../../../../secret", "item/diamond", "/etc/passwd", ""] {
        assert_eq!(
            Icon::Texture(name.into()).texture(&resources),
            None,
            "{name}"
        );
    }
    #[cfg(unix)]
    {
        fs::create_dir_all(resources.join("assets/minecraft/textures/block")).unwrap();
        std::os::unix::fs::symlink(
            root.join("secret.png"),
            resources.join("assets/minecraft/textures/block/linked.png"),
        )
        .unwrap();
        assert_eq!(Icon::Texture("linked".into()).texture(&resources), None);
    }
    assert!(Icon::Texture("diamond".into())
        .texture(&resources)
        .unwrap()
        .ends_with("item/diamond.png"));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn artwork_is_read_only_up_to_its_limit() {
    let root = scratch("artwork");
    let small = root.join("icon.png");
    fs::write(&small, b"png").unwrap();
    assert_eq!(read_artwork(&small).as_deref(), Some(&b"png"[..]));
    // One byte over: refused before anything of it is read (a sparse file, so no disk either).
    let big = root.join("big.png");
    fs::File::create(&big)
        .unwrap()
        .set_len(MAX_ARTWORK + 1)
        .unwrap();
    assert_eq!(read_artwork(&big), None);
    let most = root.join("most.png");
    fs::File::create(&most)
        .unwrap()
        .set_len(MAX_ARTWORK)
        .unwrap();
    assert_eq!(
        read_artwork(&most).map(|b| b.len() as u64),
        Some(MAX_ARTWORK)
    );
    assert_eq!(read_artwork(&root), None, "a folder is no picture");
    assert_eq!(read_artwork(&root.join("missing.png")), None);
    fs::remove_dir_all(&root).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn artwork_reader_does_not_wait_for_a_pipe_writer() {
    use std::os::unix::ffi::OsStrExt as _;

    let root = scratch("artwork-pipe");
    let path = root.join("icon.png");
    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: `name` is a live, NUL-terminated path into our scratch directory.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert_eq!(read_artwork(&path), None);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn first_profile_is_stable_without_migrating_old_players() {
    let tmp = tempfile::tempdir().unwrap();
    let fresh = tmp.path().join("fresh");
    let home = tmp.path().join("game");
    initialize_default_in(&home, &fresh).unwrap();
    assert_eq!(list_in(&home, &fresh)[0].channel, Channel::Stable);
    fs::write(fresh.join("settings.json"), "{}").unwrap();
    initialize_default_in(&home, &fresh).unwrap();
    assert_eq!(list_in(&home, &fresh)[0].channel, Channel::Stable);
    assert_eq!(
        create_whole_in(&home, &fresh, "Another profile", None, None, |_| {})
            .unwrap()
            .channel,
        Channel::Stable
    );

    let existing = tmp.path().join("existing");
    fs::create_dir_all(&existing).unwrap();
    let old = br#"{"instances":{"sunrise":{"version":"v2026.10.13"}}}"#;
    fs::write(existing.join(FILE), old).unwrap();
    initialize_default_in(&home, &existing).unwrap();
    assert_eq!(fs::read(existing.join(FILE)).unwrap(), old);
    let profile = &list_in(&home, &existing)[0];
    assert_eq!(profile.channel, Channel::Nightly);
    assert_eq!(profile.version.as_deref(), Some("v2026.10.13"));

    fs::remove_file(existing.join(FILE)).unwrap();
    fs::write(existing.join("settings.json"), "{}").unwrap();
    initialize_default_in(&home, &existing).unwrap();
    assert_eq!(list_in(&home, &existing)[0].channel, Channel::Nightly);
    assert!(!existing.join(FILE).exists());

    // Playing on the implicit profile could leave only the game files.
    fs::remove_file(existing.join("settings.json")).unwrap();
    fs::create_dir_all(home.join("saves")).unwrap();
    initialize_default_in(&home, &existing).unwrap();
    assert_eq!(list_in(&home, &existing)[0].channel, Channel::Nightly);
    assert!(!existing.join(FILE).exists());
}

#[cfg(unix)]
#[test]
fn an_installed_legacy_pin_still_starts_its_client() {
    use std::os::unix::fs::PermissionsExt;
    const ACTOR: &str = "SUNRISE_STABLE_LEGACY_PLAY_TEST";
    const OLD: &str = "v2026.10.13";
    const UUID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    if std::env::var_os(ACTOR).is_none() {
        let root = tempfile::tempdir().unwrap();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "instances::tests::an_installed_legacy_pin_still_starts_its_client"
            ])
            .env(ACTOR, "1")
            .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
            .env("SUNRISE_HOME", root.path().join("game"))
            .env("SUNRISE_TELEMETRY_URL", "off")
            .env("SUNRISE_NO_KEYCHAIN", "1")
            // No Microsoft app: the stored token stands in for a sign-in and no service is
            // contacted, also in a build that has the app ID built in (the release build).
            .env("SUNRISE_MSA_CLIENT_ID", "")
            .status()
            .unwrap()
            .success());
        return;
    }
    let data = backend::data_dir();
    fs::create_dir_all(backend::version_dir("26.3-0.1.0")).unwrap();
    fs::write(backend::version_dir("26.3-0.1.0").join(".installed"), "").unwrap();
    fs::create_dir_all(backend::resources(OLD)).unwrap();
    fs::write(backend::version_dir(OLD).join(".installed"), "").unwrap();
    fs::write(backend::resources(OLD).join("sunrise-resources.json"), "{}").unwrap();
    fs::write(
        data.join(FILE),
        format!(r#"{{"instances":{{"sunrise":{{"version":"{OLD}"}}}}}}"#),
    )
    .unwrap();
    let client = backend::client_program(OLD);
    fs::write(&client, "#!/bin/sh\nprintf 'legacy client reached\\n'\n").unwrap();
    fs::set_permissions(&client, fs::Permissions::from_mode(0o700)).unwrap();

    // Local stand-in for an already authenticated session; no service is contacted.
    let accounts =
        serde_json::json!({"selected": UUID, "accounts": [{"uuid": UUID, "name": "Player"}]});
    fs::write(data.join("accounts.json"), accounts.to_string()).unwrap();
    let secrets = serde_json::json!({"refresh_token":"fixture", "minecraft_token":"fixture",
        "minecraft_expires_at":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 3600});
    crate::account::store()
        .set(UUID, &secrets.to_string())
        .unwrap();

    assert!(!backend::is_game_tag(OLD));
    let instance = selection().unwrap();
    let tag = instance.resolve_version(None).unwrap();
    assert_eq!(tag, OLD);
    let mut cmd = backend::client_as(&tag, None).unwrap();
    apply(&mut cmd, &instance).unwrap();
    let result = cmd.output().unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"legacy client reached\n");
    crate::account::discard_unread_sessions();
}

#[cfg(unix)]
#[test]
fn same_day_rolling_nightly_remains_the_play_target_offline() {
    use std::os::unix::fs::PermissionsExt;
    const ACTOR: &str = "SUNRISE_TEST_OFFLINE_NIGHTLY";
    const CURRENT: &str = "nightly-2026-11-20-aaaaaaa";
    const OLD: &str = "nightly-2026-11-20-bbbbbbb";
    if std::env::var_os(ACTOR).is_none() {
        let root = tempfile::tempdir().unwrap();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "instances::tests::same_day_rolling_nightly_remains_the_play_target_offline"
            ])
            .env(ACTOR, "1")
            .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
            .env("SUNRISE_HOME", root.path().join("game"))
            .env("SUNRISE_TELEMETRY_URL", "off")
            .env("SUNRISE_NO_KEYCHAIN", "1")
            // No Microsoft app: the stored token stands in for a sign-in and no service is
            // contacted, also in a build that has the app ID built in (the release build).
            .env("SUNRISE_MSA_CLIENT_ID", "")
            .status()
            .unwrap()
            .success());
        return;
    }
    let data = backend::data_dir();
    let stable = "26.3-0.1.0";
    for tag in [OLD, CURRENT, stable] {
        let version = backend::version_dir(tag);
        fs::create_dir_all(&version).unwrap();
        fs::write(version.join(".installed"), "").unwrap();
    }
    fs::write(
        data.join("settings.json"),
        format!(r#"{{"selected":"{stable}"}}"#),
    )
    .unwrap();
    // Game installation records the verified rolling build before placing its completion marker.
    // Both builds were published in the same minute, and the rolling hash sorts BEFORE the old one.
    for tag in [OLD, CURRENT] {
        fs::write(
            backend::version_dir(tag).join(crate::nightly::INSTALLED_TIME),
            "11:05",
        )
        .unwrap();
    }
    let (installed, unknown) = backend::play_installations(None);
    assert!(unknown.is_none(), "must not guess from {installed:?}");
    crate::nightly::record_current(&data.join("versions"), CURRENT).unwrap();
    let (installed, known) = backend::play_installations(Some(stable));
    let profile = selection().unwrap();
    let would_play = profile
        .channel
        .play_target(&[], &installed, known.as_ref(), None)
        .unwrap();
    assert_eq!(would_play, CURRENT, "offline GUI target");
    let tag = profile.resolve_version(None).unwrap();
    assert_eq!(tag, CURRENT, "offline CLI Play target");
    for name in [OLD, CURRENT] {
        fs::create_dir_all(backend::resources(name)).unwrap();
        fs::write(
            backend::resources(name).join("sunrise-resources.json"),
            "{}",
        )
        .unwrap();
        let client = backend::client_program(name);
        fs::write(&client, format!("#!/bin/sh\nprintf '%s' '{name}'\n")).unwrap();
        fs::set_permissions(client, fs::Permissions::from_mode(0o700)).unwrap();
    }
    const UUID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let accounts =
        serde_json::json!({"selected": UUID, "accounts": [{"uuid": UUID, "name": "Player"}]});
    fs::write(data.join("accounts.json"), accounts.to_string()).unwrap();
    let secrets = serde_json::json!({"refresh_token":"fixture", "minecraft_token":"fixture",
        "minecraft_expires_at":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 3600});
    crate::account::store()
        .set(UUID, &secrets.to_string())
        .unwrap();
    let mut cmd = backend::client_as(&tag, None).unwrap();
    apply(&mut cmd, &profile).unwrap();
    let result = cmd.output().unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, CURRENT.as_bytes());
    crate::account::discard_unread_sessions();
}

#[test]
fn channel_less_cli_play_refuses_to_fall_through_to_installed_stable() {
    const ACTOR: &str = "SUNRISE_TEST_NIGHTLY_WITH_ONLY_STABLE";
    const STABLE: &str = "26.3-0.1.0";
    if std::env::var_os(ACTOR).is_none() {
        let root = tempfile::tempdir().unwrap();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "instances::tests::channel_less_cli_play_refuses_to_fall_through_to_installed_stable"])
            .env(ACTOR, "1")
            .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
            .env("SUNRISE_HOME", root.path().join("game"))
            .status().unwrap().success());
        return;
    }
    put(&backend::version_dir(STABLE).join(".installed"), "");
    put(
        &backend::data_dir().join("settings.json"),
        &format!(r#"{{"selected":"{STABLE}"}}"#),
    );
    put(
        &backend::data_dir().join(FILE),
        r#"{"instances":{"sunrise":{}}}"#,
    );
    let profile = selection().unwrap();
    assert_eq!(profile.channel, Channel::Nightly);
    assert!(profile.version.is_none());
    assert!(profile.resolve_version(None).is_err());
    assert_eq!(profile.resolve_version(Some(STABLE)).unwrap(), STABLE);
    assert_eq!(
        edit(DEFAULT, |meta| meta.version = Some(STABLE.into()))
            .unwrap()
            .resolve_version(None)
            .unwrap(),
        STABLE
    );
}

#[test]
fn copy_home_preserves_chosen_versions_pins_and_skips_caches() {
    let from = tempfile::tempdir().unwrap();
    let to = tempfile::tempdir().unwrap();
    let target = to.path().join("copy");
    put(
        &from.path().join("mod-browser/chosen-versions.json"),
        r#"{"Ab3dE9xQ":"1.0.0"}"#,
    );
    put(&from.path().join("mod-browser/sunrise-index.json"), "{}");
    let worlds = crate::worlds::SavedWorlds::claim(&from.path().join("saves")).unwrap();
    copy_home(from.path(), &target, &worlds).unwrap();
    assert_eq!(
        fs::read_to_string(target.join("mod-browser/chosen-versions.json")).unwrap(),
        r#"{"Ab3dE9xQ":"1.0.0"}"#
    );
    assert!(!target.join("mod-browser/sunrise-index.json").exists());
}

#[test]
fn storage_profile_snapshot_stops_when_cancelled_mid_discovery() {
    let root = scratch("cancel-snapshot");
    let (home, data) = (root.join("home"), root.join("data"));
    let own = home.join(OWN);
    fs::create_dir_all(&own).unwrap();
    // Use regular files so `own` stays empty and downstream loops run only 4 checks in total
    // (entry, `registry_pack::list_in_until`, and the two `found` loops over `DEFAULT`).
    // Cancelling on the 5th call (`count >= 4`) can only be reached inside the `read_dir` loop.
    for index in 0..8 {
        fs::write(own.join(format!("not-a-profile-{index}")), b"").unwrap();
    }
    let checks = std::cell::Cell::new(0usize);
    let result = list_snapshot_until(&home, &data, Store::default(), true, false, &|| {
        let count = checks.get();
        checks.set(count + 1);
        count >= 4
    });
    assert!(result.is_none());
    assert_eq!(
        checks.get(),
        5,
        "must stop inside the profile discovery loop"
    );
    fs::remove_dir_all(root).unwrap();
}
