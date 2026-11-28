use super::*;
use chrono::TimeZone;

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "sunrise-launcher-worlds-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// A world as the game saves it: a gzip level.dat with `data`'s fields under `Data`.
fn world(home: &Path, folder: &str, data: Vec<(&str, Tag)>) -> PathBuf {
    let dir = home.join("saves").join(folder);
    fs::create_dir_all(dir.join("region")).unwrap();
    let root = Tag::compound([("Data", Tag::compound(data))]);
    let bytes = nbt::write_named(
        &nbt::NamedTag {
            name: "".into(),
            tag: root,
        },
        Limits::default(),
    )
    .unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&bytes).unwrap();
    fs::write(dir.join("level.dat"), gz.finish().unwrap()).unwrap();
    fs::write(dir.join("region/r.0.0.mca"), [7u8; 1000]).unwrap();
    fs::write(dir.join(LOCK), "lock").unwrap();
    dir
}

fn name(n: &str) -> Tag {
    Tag::String(n.into())
}

fn admitted(home: &Path) -> Result<crate::managed_play::Profile, String> {
    crate::managed_play::Profile::open_at(home, &home.join("test-cache"), home)
}

fn copy_in(home: &Path, world: &str) -> Result<World, String> {
    copy_admitted(admitted(home)?, world, |_| {})
}

#[test]
fn active_worlds_cannot_be_backed_up_copied_or_deleted() {
    let home = scratch("active-lock");
    let dir = world(&home, "Active", vec![("LevelName", name("Active"))]);
    let w = find_raw(&home, "Active").unwrap();
    let game = fs::File::options()
        .write(true)
        .open(dir.join(LOCK))
        .unwrap();
    game.try_lock().unwrap();

    assert!(backup(&home, &w).is_err());
    assert!(copy_in(&home, "Active").is_err());
    assert!(delete_world(&w).is_err());
    assert!(!home.join("backups").exists());
    assert!(!home.join("saves/Active (1)").exists());
    assert_eq!(fs::read(dir.join("region/r.0.0.mca")).unwrap(), [7u8; 1000]);

    // As the game does when it closes; dropping alone could leave it held by a process another
    // test is starting.
    game.unlock().unwrap();
    drop(game);
    assert!(backup(&home, &w).unwrap().is_file());
    assert!(copy_in(&home, "Active").unwrap().path.is_dir());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn lists_what_each_world_is() {
    let home = scratch("list");
    let survival = world(
        &home,
        "Survival",
        vec![
            ("LevelName", name("My survival")),
            ("GameType", Tag::Int(0)),
            ("LastPlayed", Tag::Long(1_700_000_000_000)),
            ("Version", Tag::compound([("Name", name("26.3"))])),
        ],
    );
    fs::write(survival.join("icon.png"), "png").unwrap();
    world(
        &home,
        "Hard",
        vec![
            ("LevelName", name("Hard")),
            ("GameType", Tag::Int(0)),
            ("LastPlayed", Tag::Long(1_800_000_000_000)),
            (
                "difficulty_settings",
                Tag::compound([("hardcore", Tag::Byte(1))]),
            ),
        ],
    );
    world(
        &home,
        "Old",
        vec![
            ("GameType", Tag::Int(1)),
            ("hardcore", Tag::Byte(0)),
            ("LastPlayed", Tag::Long(1_600_000_000_000)),
        ],
    );
    fs::create_dir_all(home.join("saves/not a world")).unwrap();

    let all = list_raw(&home);
    let names: Vec<&str> = all.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, ["Hard", "My survival", "Old"]);
    assert_eq!(all[0].mode, "Hardcore");
    assert_eq!(all[1].mode, "Survival");
    assert_eq!(all[1].folder, "Survival");
    assert_eq!(all[1].version.as_deref(), Some("26.3"));
    assert_eq!(all[1].icon, Some(survival.join("icon.png")));
    assert_eq!(
        all[1].last_played,
        UNIX_EPOCH + Duration::from_millis(1_700_000_000_000)
    );
    assert!(all[1].size >= 1000);
    // No LevelName: the folder's name.
    assert_eq!(all[2].mode, "Creative");
    assert_eq!(find_raw(&home, "my SURVIVAL").unwrap().folder, "Survival");
    assert!(find_raw(&home, "nope").is_err());
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn a_backup_is_named_and_laid_out_as_vanillas() {
    let home = scratch("backup");
    world(&home, "World", vec![("LevelName", name("World"))]);
    let w = find_raw(&home, "World").unwrap();
    let at = chrono::Local
        .with_ymd_and_hms(2026, 10, 27, 1, 2, 3)
        .unwrap();
    let first = backup_at(&home, &w, at).unwrap();
    assert_eq!(first, home.join("backups/2026-10-27_01-02-03_World.zip"));
    // The same second again: the next free name.
    let second = backup_at(&home, &w, at).unwrap();
    assert_eq!(
        second,
        home.join("backups/2026-10-27_01-02-03_World (1).zip")
    );
    let mut zip = zip::ZipArchive::new(fs::File::open(&first).unwrap()).unwrap();
    let mut entries: Vec<String> = zip
        .file_names()
        .map(|name| name.unwrap().into_owned())
        .collect();
    entries.sort();
    assert_eq!(entries, ["World/level.dat", "World/region/r.0.0.mca"]);
    let mut region = Vec::new();
    zip.by_name("World/region/r.0.0.mca")
        .unwrap()
        .read_to_end(&mut region)
        .unwrap();
    assert_eq!(region, [7u8; 1000]);
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn a_copy_gets_the_next_free_name_in_its_folder_and_level_dat() {
    let home = scratch("copy");
    world(
        &home,
        "Base",
        vec![("LevelName", name("Base")), ("GameType", Tag::Int(1))],
    );
    let base = find_raw(&home, "Base").unwrap();
    let copy1 = copy_in(&home, "Base").unwrap();
    assert_eq!(copy1.folder, "Base (1)");
    assert_eq!(copy1.name, "Base (1)");
    assert_eq!(copy1.mode, "Creative");
    assert!(!copy1.path.join(LOCK).exists());
    assert_eq!(
        fs::read(copy1.path.join("region/r.0.0.mca")).unwrap(),
        [7u8; 1000]
    );
    assert_eq!(copy_in(&home, "Base").unwrap().folder, "Base (2)");
    // The original is untouched.
    assert_eq!(find_raw(&home, "Base").unwrap(), base);
    // A name that can't be a folder name is made safe.
    assert_eq!(free_name(&home, "a/b: c?", "", 0), "a_b_ c_");
    // A world whose folder isn't its name: the copy still gets a new name.
    world(&home, "f", vec![("LevelName", name("Named"))]);
    let named = copy_in(&home, "f").unwrap();
    assert_eq!(
        (named.folder.as_str(), named.name.as_str()),
        ("Named (1)", "Named (1)")
    );
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn same_name_world_copies_cannot_reuse_or_erase_a_destination() {
    let home = scratch("world-copy-contention");
    world(&home, "one", vec![("LevelName", name("Shared"))]);
    world(&home, "two", vec![("LevelName", name("Shared"))]);
    let loss = copy_admitted(admitted(&home).unwrap(), "one", |destination| {
        assert!(copy_in(&home, "two").is_err());
        fs::create_dir(destination).unwrap();
        fs::write(destination.join("winner"), b"do not delete").unwrap();
    });
    assert!(loss.is_err());
    assert_eq!(
        fs::read(home.join("saves/Shared (1)/winner")).unwrap(),
        b"do not delete"
    );
    let second = copy_in(&home, "two").unwrap();
    assert_eq!(second.folder, "Shared (2)");
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn the_mods_a_world_was_saved_with_are_read_and_matched() {
    let home = scratch("saved-mods");
    let record = |id: &str| Tag::compound([("ModId", name(id)), ("ModVersion", name("1.0.0"))]);
    let dir = home.join("saves").join("Home");
    fs::create_dir_all(&dir).unwrap();
    let root = Tag::compound([
        ("Data", Tag::compound([("LevelName", name("Home"))])),
        (
            "fml",
            Tag::compound([("LoadingModList", Tag::List(vec![record("greeter")]))]),
        ),
        (
            "sunrise",
            Tag::compound([("ContentModList", Tag::List(vec![record("bricks")]))]),
        ),
    ]);
    let bytes = nbt::write_named(
        &nbt::NamedTag {
            name: "".into(),
            tag: root,
        },
        Limits::default(),
    )
    .unwrap();
    fs::write(dir.join("level.dat"), bytes).unwrap();
    world(&home, "Plain", vec![("LevelName", name("Plain"))]);
    let all = list_raw(&home);
    let home_world = all.iter().find(|w| w.folder == "Home").unwrap();
    assert_eq!(home_world.mods, ["greeter", "bricks"]);
    let plain = all.iter().find(|w| w.folder == "Plain").unwrap();
    assert!(plain.mods.is_empty());
    let at_stake = saved_with(&all, &["greeter".into(), "other".into()]);
    assert_eq!(at_stake.len(), 1);
    assert_eq!(at_stake[0].0.name, "Home");
    assert_eq!(at_stake[0].1, ["greeter"]);
    assert!(saved_with(&all, &["other".into()]).is_empty());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn a_level_dat_the_game_loads_is_read_past_the_default_tag_cap() {
    let home = scratch("saved-limits");
    // More tags than `Limits::default` allows: the game reads a world's `.dat` files without
    // that cap, so the scan must too, or the world reads as unknown and holding every mod.
    let many = Tag::List(vec![Tag::Int(0); Limits::default().max_nodes + 1]);
    let dir = home.join("saves/Big");
    fs::create_dir_all(&dir).unwrap();
    let root = Tag::compound([(
        "Data",
        Tag::compound([("LevelName", name("Named")), ("Extra", many)]),
    )]);
    let bytes = nbt::write_named(
        &nbt::NamedTag {
            name: "".into(),
            tag: root,
        },
        Limits::saved(),
    )
    .unwrap();
    assert!(nbt::read_named(&bytes, Limits::default()).is_err());
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&bytes).unwrap();
    let packed = gz.finish().unwrap();
    // level.dat, and level.dat_old when level.dat is damaged (the game's recovery order).
    fs::write(dir.join("level.dat"), &packed).unwrap();
    let world = read(&dir).unwrap();
    assert_eq!(world.name, "Named");
    fs::write(dir.join("level.dat_old"), &packed).unwrap();
    fs::write(dir.join("level.dat"), b"damaged").unwrap();
    assert_eq!(read(&dir).unwrap().name, "Named");
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn a_last_played_too_far_ahead_for_a_system_time_is_clamped_not_a_panic() {
    let home = scratch("last-played-far");
    world(
        &home,
        "Far",
        vec![
            ("LevelName", name("Far")),
            ("LastPlayed", Tag::Long(i64::MAX)),
        ],
    );
    let worlds = list_raw(&home);
    assert_eq!(worlds.len(), 1);
    assert_eq!(
        worlds[0].last_played,
        UNIX_EPOCH + Duration::from_millis(253_402_300_799_000)
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn worlds_played_past_the_clamp_keep_the_newest_first() {
    let home = scratch("last-played-order");
    // Both later than the clamp, the older one first in the folder.
    for (folder, millis) in [("A", i64::MAX - 1), ("B", i64::MAX)] {
        world(
            &home,
            folder,
            vec![
                ("LevelName", name("Twin")),
                ("LastPlayed", Tag::Long(millis)),
            ],
        );
    }
    let folders: Vec<String> = list_raw(&home).into_iter().map(|w| w.folder).collect();
    assert_eq!(folders, ["B", "A"]);
    assert_eq!(find_raw(&home, "twin").unwrap().folder, "B");
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn last_played_hardcore_and_game_type_read_any_number_tag() {
    let home = scratch("number-tags");
    // 1_700_000_000_000.9 ms, as a Double and as a Float (which holds it only roughly).
    world(
        &home,
        "double",
        vec![
            ("LastPlayed", Tag::Double(1_700_000_000_000.9f64.to_bits())),
            ("GameType", Tag::Double(2.0f64.to_bits())),
        ],
    );
    world(
        &home,
        "short",
        vec![
            ("LastPlayed", Tag::Short(30_000)),
            ("GameType", Tag::Short(3)),
        ],
    );
    world(
        &home,
        "float",
        vec![
            ("LastPlayed", Tag::Float(2_000_000.0f32.to_bits())),
            ("hardcore", Tag::Float(1.0f32.to_bits())),
        ],
    );
    // The game's float `longValue` truncates toward zero (-0.5 is 0); a double's floors (-0.5 is -1).
    world(
        &home,
        "float-negative",
        vec![("hardcore", Tag::Float((-0.5f32).to_bits()))],
    );
    world(
        &home,
        "double-negative",
        vec![("hardcore", Tag::Double((-0.5f64).to_bits()))],
    );
    let at = |millis: u64| UNIX_EPOCH + Duration::from_millis(millis);
    let by = |folder: &str| find_raw(&home, folder).unwrap();
    assert_eq!(by("float-negative").mode, "Survival");
    assert_eq!(by("double-negative").mode, "Hardcore");
    assert_eq!(by("double").last_played, at(1_700_000_000_000));
    assert_eq!(by("double").mode, "Adventure");
    assert_eq!(by("short").last_played, at(30_000));
    assert_eq!(by("short").mode, "Spectator");
    assert_eq!(by("float").last_played, at(2_000_000));
    assert_eq!(by("float").mode, "Hardcore");
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn worlds_played_at_the_same_moment_are_ordered_by_folder_name() {
    let home = scratch("same-moment");
    for folder in ["delta", "alpha", "charlie", "bravo"] {
        world(
            &home,
            folder,
            vec![("LastPlayed", Tag::Long(1_700_000_000_000))],
        );
    }
    world(
        &home,
        "newest",
        vec![("LastPlayed", Tag::Long(1_800_000_000_000))],
    );
    let folders: Vec<String> = list_raw(&home).into_iter().map(|w| w.folder).collect();
    assert_eq!(folders, ["newest", "alpha", "bravo", "charlie", "delta"]);
    fs::remove_dir_all(home).unwrap();
}

fn write_evidence(path: &Path, evidence: Option<(&str, &str, Tag)>) {
    let extra = evidence.map(|(container, key, value)| (container, Tag::compound([(key, value)])));
    let tag = Tag::compound(
        [(
            "Data",
            Tag::compound([("LevelName", name("Basin")), ("DataVersion", Tag::Int(9))]),
        )]
        .into_iter()
        .chain(extra),
    );
    let bytes = nbt::write_named(
        &nbt::NamedTag {
            name: "".into(),
            tag,
        },
        Limits::saved(),
    )
    .unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

#[test]
fn review_finds_namespace_only_use_and_does_not_clear_historic_content() {
    let home = tempfile::tempdir().unwrap();
    let dat = home.path().join("saves/basin/level.dat");
    write_evidence(
        &dat,
        Some((
            "sunrise",
            "BlockNamespaces",
            Tag::List(vec![name("construction.v2")]),
        )),
    );
    let sources = BTreeMap::from([
        ("bricks".into(), BTreeSet::from(["construction.v2".into()])),
        ("windows".into(), BTreeSet::new()),
    ]);
    let risk = review_risk(home.path(), &sources).unwrap();
    assert_eq!(risk.len(), 1);
    assert_eq!(risk[0].name, "Basin");
    assert_eq!(risk[0].mods, ["bricks", "windows"]);
    assert_eq!(risk[0].confirmed_mods, ["bricks"]);
    assert!(!risk[0].known);
    assert!(!risk[0].primary_complete);
    assert!(!risk[0].malformed);
    let sources = BTreeMap::from([("bricks".into(), sources["bricks"].clone())]);
    assert!(review_risk(home.path(), &sources).unwrap()[0].known);
    assert!(!dat.with_file_name(LOCK).exists());
}

#[test]
fn review_unknown_evidence_cannot_clear_a_suspect() {
    let home = tempfile::tempdir().unwrap();
    let dat = home.path().join("saves/basin/level.dat");
    let sources = BTreeMap::from([("bricks".into(), BTreeSet::from(["construction".into()]))]);
    let content = |tag| Some(("sunrise", "ContentModList", tag));
    let blocks = |entries| Some(("sunrise", "BlockNamespaces", Tag::List(entries)));
    let cases = [
        None,
        content(Tag::List(vec![])),
        content(name("bricks")),
        content(Tag::List(vec![Tag::compound([
            ("ModId", name("bricks")),
            ("ModVersion", Tag::Int(7)),
        ])])),
        Some((
            "fml",
            "LoadingModList",
            Tag::List(vec![Tag::compound([("ModId", name("bricks"))])]),
        )),
        blocks(vec![name("construction"), name("bad:namespace")]),
        blocks(vec![name("construction"), Tag::Int(7)]),
    ];
    for (index, records) in cases.into_iter().enumerate() {
        write_evidence(&dat, records);
        let risk = review_risk(home.path(), &sources).unwrap();
        assert_eq!(risk.len(), 1);
        assert_eq!(risk[0].mods, ["bricks"]);
        assert!(!risk[0].known);
        assert!(!risk[0].primary_complete);
        // Omitted and explicitly empty lists are unknown, not malformed.
        assert_eq!(risk[0].malformed, index > 1);
        assert!(risk[0].confirmed_mods.is_empty());
    }
}

#[test]
fn complete_last_loaded_lists_cannot_clear_historical_risk() {
    let home = tempfile::tempdir().unwrap();
    let dat = home.path().join("saves/basin/level.dat");
    let old = dat.with_file_name("level.dat_old");
    let unrelated = Tag::compound([
        ("ModId", name("other")),
        ("ModVersion", name("custom-version")),
    ]);
    for path in [&dat, &old] {
        write_evidence(
            path,
            Some((
                "sunrise",
                "ContentModList",
                Tag::List(vec![unrelated.clone()]),
            )),
        );
    }
    let fallback_only = home.path().join("saves/creek/level.dat_old");
    write_evidence(&fallback_only, None);
    let sources = BTreeMap::from([("bricks".into(), BTreeSet::from(["construction".into()]))]);
    let risk = review_risk(home.path(), &sources).unwrap();
    assert_eq!(risk.len(), 2);
    assert!(risk[0].primary_complete);
    assert!(!risk[1].primary_complete);
    for world in &risk {
        assert_eq!(world.mods, ["bricks"]);
        assert!(world.confirmed_mods.is_empty());
        assert!(!world.known);
        assert!(!world.malformed);
    }
    write_evidence(
        &old,
        Some((
            "sunrise",
            "BlockNamespaces",
            Tag::List(vec![name("construction")]),
        )),
    );
    let risk = review_risk(home.path(), &sources).unwrap();
    assert!(risk[0].primary_complete);
    assert!(risk[0].known);
    assert_eq!(risk[0].mods, ["bricks"]);
    assert_eq!(risk[0].confirmed_mods, ["bricks"]);
    fs::write(&old, b"bad").unwrap();
    let risk = review_risk(home.path(), &sources).unwrap();
    assert!(risk[0].primary_complete);
    assert!(risk[0].malformed);
    assert!(!risk[0].known);
    assert_eq!(risk[0].mods, ["bricks"]);
    assert!(risk[0].confirmed_mods.is_empty());
}

#[test]
fn review_keeps_positive_fallback_and_does_not_take_its_game_age() {
    let home = tempfile::tempdir().unwrap();
    let dat = home.path().join("saves/basin/level.dat");
    let old = dat.with_file_name("level.dat_old");
    // The game's code-mod writer uses an empty version when it doesn't know the version.
    let record = Tag::compound([("ModId", name("bricks")), ("ModVersion", name(""))]);
    write_evidence(
        &old,
        Some(("fml", "LoadingModList", Tag::List(vec![record]))),
    );
    write_evidence(&dat, None);
    let sources = BTreeMap::from([("bricks".into(), BTreeSet::new())]);
    let snapshot = review_risk(home.path(), &sources).unwrap();
    assert_eq!(snapshot[0].confirmed_mods, ["bricks"]);
    assert!(snapshot[0].known);
    assert_eq!(snapshot[0].data_version, Some(9));
    fs::write(&dat, b"corrupt").unwrap();
    let recovered = review_risk(home.path(), &sources).unwrap();
    assert_ne!(recovered, snapshot);
    assert_eq!(recovered[0].confirmed_mods, ["bricks"]);
    assert!(recovered[0].malformed);
    assert_eq!(recovered[0].data_version, None);
    fs::remove_file(&dat).unwrap();
    let fallback_only = review_risk(home.path(), &sources).unwrap();
    assert!(!fallback_only[0].malformed);
    assert_eq!(fallback_only[0].confirmed_mods, ["bricks"]);
    fs::write(&dat, b"corrupt").unwrap();
    fs::remove_file(&old).unwrap();
    let unknown = review_risk(home.path(), &sources).unwrap();
    assert_eq!(unknown[0].mods, ["bricks"]);
    assert!(!unknown[0].known);
}

#[test]
fn review_binds_exact_fallback_bytes_and_same_path_directory_identity() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("saves/basin");
    let dat = dir.join("level.dat");
    let old = dir.join("level.dat_old");
    let sources = BTreeMap::from([("bricks".into(), BTreeSet::new())]);
    write_evidence(&dat, None);
    fs::write(&old, b"bad1").unwrap();
    let before = review_risk(home.path(), &sources).unwrap();
    fs::write(&old, b"bad2").unwrap();
    let reviewed = review_risk(home.path(), &sources).unwrap();
    assert_ne!(before, reviewed);
    let held = home.path().join("original");
    fs::rename(&dir, &held).unwrap();
    fs::create_dir(&dir).unwrap();
    fs::copy(held.join("level.dat"), &dat).unwrap();
    fs::copy(held.join("level.dat_old"), &old).unwrap();
    let replacement = review_risk(home.path(), &sources).unwrap();
    assert_eq!(reviewed[0].name, replacement[0].name);
    assert_eq!(reviewed[0].mods, replacement[0].mods);
    assert_ne!(reviewed, replacement);
    let added = home.path().join("saves/new/level.dat");
    fs::create_dir(added.parent().unwrap()).unwrap();
    fs::write(added, b"bad").unwrap();
    assert_eq!(review_risk(home.path(), &sources).unwrap().len(), 2);
}

#[test]
fn review_refuses_aliased_level_evidence_and_a_non_directory_roster() {
    let home = tempfile::tempdir().unwrap();
    let sources = BTreeMap::new();
    assert!(review_risk(home.path(), &sources).unwrap().is_empty());
    let saves = home.path().join("saves");
    fs::write(&saves, b"not a directory").unwrap();
    assert!(review_risk(home.path(), &sources).is_err());
    fs::remove_file(&saves).unwrap();
    let dat = saves.join("basin/level.dat");
    write_evidence(&dat, None);
    fs::hard_link(&dat, dat.with_file_name("level.dat_old")).unwrap();
    assert!(review_risk(home.path(), &sources).is_err());
}

#[cfg(unix)]
#[test]
fn review_refuses_symlinked_worlds_and_evidence() {
    let home = tempfile::tempdir().unwrap();
    let sources = BTreeMap::new();
    let dir = home.path().join("elsewhere");
    let dat = dir.join("level.dat");
    write_evidence(&dat, None);
    let saves = home.path().join("saves");
    std::os::unix::fs::symlink(&dir, &saves).unwrap();
    assert!(review_risk(home.path(), &sources).is_err());
    fs::remove_file(&saves).unwrap();
    fs::create_dir(&saves).unwrap();
    let alias = saves.join("alias");
    std::os::unix::fs::symlink(&dir, &alias).unwrap();
    assert!(review_risk(home.path(), &sources).is_err());
    fs::remove_file(&alias).unwrap();
    fs::create_dir(&alias).unwrap();
    std::os::unix::fs::symlink(&dat, alias.join("level.dat")).unwrap();
    assert!(review_risk(home.path(), &sources).is_err());
}

#[test]
fn reviewed_world_locks_exclude_game_and_release_without_changing_evidence() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("saves/basin");
    write_evidence(&dir.join("level.dat"), None);
    let sources = BTreeMap::new();
    let reviewed = review_risk(home.path(), &sources).unwrap();
    let guard = lock_reviewed(&reviewed).unwrap();
    let game = fs::OpenOptions::new()
        .write(true)
        .open(dir.join(LOCK))
        .unwrap();
    assert!(game.try_lock().is_err());
    assert_eq!(review_risk(home.path(), &sources).unwrap(), reviewed);
    drop(guard);
    game.try_lock().unwrap();
    assert!(lock_reviewed(&reviewed).is_err());
    game.unlock().unwrap();
}

#[test]
fn reviewed_world_locks_never_create_a_lock_in_a_same_path_replacement() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("saves/basin");
    write_evidence(&dir.join("level.dat"), None);
    let reviewed = review_risk(home.path(), &BTreeMap::new()).unwrap();
    fs::rename(&dir, home.path().join("original")).unwrap();
    write_evidence(&dir.join("level.dat"), None);
    assert!(lock_reviewed(&reviewed).is_err());
    assert!(!dir.join(LOCK).exists());
}

#[test]
fn reviewed_world_locks_refuse_an_aliased_session_file() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("saves/basin");
    write_evidence(&dir.join("level.dat"), None);
    let reviewed = review_risk(home.path(), &BTreeMap::new()).unwrap();
    let foreign = home.path().join("foreign");
    fs::write(&foreign, "foreign").unwrap();
    fs::hard_link(&foreign, dir.join(LOCK)).unwrap();
    assert!(lock_reviewed(&reviewed).is_err());
    assert_eq!(fs::read(&foreign).unwrap(), b"foreign");
    #[cfg(unix)]
    {
        fs::remove_file(dir.join(LOCK)).unwrap();
        std::os::unix::fs::symlink(&foreign, dir.join(LOCK)).unwrap();
        assert!(lock_reviewed(&reviewed).is_err());
        assert_eq!(fs::read(foreign).unwrap(), b"foreign");
    }
}

#[test]
fn at_risk_catches_mods_recorded_only_in_level_dat_old() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("saves/basin");
    write_evidence(&dir.join("level.dat"), None);
    write_evidence(
        &dir.join("level.dat_old"),
        Some((
            "fml",
            "LoadingModList",
            Tag::List(vec![Tag::compound([("ModId", name("greeter"))])]),
        )),
    );

    let flagged = at_risk(home.path(), &["greeter".into()]).unwrap();
    assert_eq!(flagged.len(), 1);
    assert_eq!(flagged[0].0.name, "Basin");
    assert_eq!(flagged[0].1, ["greeter"]);
    assert!(flagged[0].2);
}
