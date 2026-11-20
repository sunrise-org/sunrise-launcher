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
fn an_instance_without_a_channel_is_nightly_and_an_unknown_one_too() {
    let meta: Meta = serde_json::from_str("{}").unwrap();
    assert_eq!(meta.channel.unwrap_or_default(), Channel::Nightly);
    let meta: Meta = serde_json::from_str(r#"{"channel":"stable","group":"g"}"#).unwrap();
    assert_eq!(meta.channel, Some(Channel::Stable));
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
        retired_in(&all, Some("port:old-pack")).as_deref(),
        Some("port:old-pack")
    );

    let made = create_in(&home, &data, "Survival", None).unwrap();
    let all = list_in(&home, &data);
    assert_eq!(selection_in(&all, None).unwrap().id, DEFAULT);
    assert_eq!(selection_in(&all, Some(DEFAULT)).unwrap().id, DEFAULT);
    assert_eq!(selection_in(&all, Some(&made.id)).unwrap().id, made.id);
    assert_eq!(retired_in(&all, Some(&made.id)), None);
    assert_eq!(retired_in(&all, None), None);
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
        retired_in(&all, Some("vanilla")).as_deref(),
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
    assert_eq!(fs::read_dir(&data).unwrap().count(), 2);
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
fn profile_publication_does_not_replace_an_empty_destination() {
    let root = scratch("creation-no-clobber");
    let (source, dest) = (root.join("source"), root.join("dest"));
    put(&source.join("options.txt"), "new");
    fs::create_dir(&dest).unwrap();
    assert!(publish_new_folder(&source, &dest).is_err());
    assert!(source.join("options.txt").is_file());
    assert!(fs::read_dir(&dest).unwrap().next().is_none());
    fs::remove_dir(dest).unwrap();
    assert!(publish_new_folder(&source, &root.join("dest")).is_ok());
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

    // A historical profile named "nightly" has the same exact-pin contract as any other.
    const OLD: &str = "nightly-2026-11-15-aaaaaaa";
    const NEW: &str = "nightly-2026-11-16-bbbbbbb";
    fs::create_dir_all(backend::game_home().join(OWN).join("nightly")).unwrap();
    put(&backend::version_dir(OLD).join(".installed"), "");
    edit("nightly", |meta| meta.version = Some(OLD.into())).unwrap();
    crate::nightly::after_update(NEW);
    assert_eq!(
        edit("nightly", |_| {})
            .unwrap()
            .resolve_version(None)
            .unwrap(),
        OLD
    );
}
