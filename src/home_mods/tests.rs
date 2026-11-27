//! Tests for the profile inventory: the read, the watched snapshot and the tab lists.
use super::*;

fn item(name: &str, kind: Kind, on: bool) -> Item {
    Item {
        id: name.to_lowercase(),
        name: name.into(),
        kind,
        picture: Picture::Local {
            package: None,
            source: None,
        },
        open: Open::Installed(name.into()),
        on,
        manual_badge: None,
        mod_version: (kind == Kind::Mod).then(|| "1.0.0".into()),
    }
}

fn names(list: &[Item]) -> Vec<&str> {
    list.iter().map(|i| i.name.as_str()).collect()
}

#[test]
fn only_a_paint_sized_thumbnail_reaches_the_renderer() {
    let png = |wide: u32, tall: u32| {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(wide, tall, image::Rgba([220, 80, 40, 255]))
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        assert!(
            bytes.len() < 4 << 20,
            "under the pack entry's compressed read cap"
        );
        bytes
    };
    let art = pack_thumbnail(png(1024, 64)).expect("an ordinary large PNG is resized");
    assert_eq!((art.size(0).width.0, art.size(0).height.0), (60, 60));
    assert!(
        pack_thumbnail(png(4097, 1)).is_none(),
        "reject over-limit dimensions"
    );
}

#[test]
fn the_mod_tab_lists_enabled_mods_and_keeps_off_packs_in_the_pack_tab() {
    let tabs: Collections = [
        item("Lanternlight", Kind::Mod, true),
        item("Trailmarks", Kind::Mod, false),
        item("Quiet Rain", Kind::Mod, true),
        item("Soft Edges", Kind::Resource, true),
        item("Old Paper", Kind::Resource, false),
    ]
    .into_iter()
    .collect();
    assert_eq!(names(&tabs.mods), ["Lanternlight", "Quiet Rain"]);
    assert_eq!(tabs.mods_off, 1);
    assert_eq!(names(&tabs.resources), ["Soft Edges", "Old Paper"]);
    assert!(!tabs.resources[1].on);
}

fn query(path: impl Into<PathBuf>) -> Query {
    Query {
        home: path.into(),
        version: None,
    }
}

#[test]
fn the_profiles_list_is_read_once_and_again_only_when_asked() {
    let a = query("/profiles/a");
    let start = Instant::now();
    let mut kept = Inventory::default();
    let (generation, stamp) = kept.draw(&a, start).unwrap();
    assert!(stamp.is_empty() && kept.snapshot.is_none());
    assert!(kept.draw(&a, start).is_none(), "one read at a time");
    kept.took(
        generation,
        Some((
            "one".into(),
            Snapshot {
                tabs: [item("Lanternlight", Kind::Mod, true)]
                    .into_iter()
                    .collect(),
                launch: LaunchSummary::default(),
            },
        )),
    );
    let soon = start + Duration::from_millis(100);
    let shown = kept.snapshot.clone().unwrap();
    assert!(kept.draw(&a, soon).is_none(), "a redraw reads nothing");
    kept.stale = true;
    let (generation, known) = kept.draw(&a, soon).unwrap();
    assert_eq!(known, "one");
    kept.took(generation, None);
    assert!(
        Arc::ptr_eq(kept.snapshot.as_ref().unwrap(), &shown),
        "unchanged: kept"
    );
    let later = soon + AWAY + Duration::from_millis(1);
    assert_eq!(kept.draw(&a, later).unwrap().1, "one");
}

#[test]
fn going_back_to_a_profile_does_not_accept_its_old_in_flight_answer() {
    let (a, b) = (query("/profiles/a"), query("/profiles/b"));
    let now = Instant::now();
    let mut kept = Inventory::default();
    let first_a = kept.draw(&a, now).unwrap().0;
    let for_b = kept.draw(&b, now).unwrap().0;
    let second_a = kept.draw(&a, now).unwrap().0;
    kept.took(for_b, None);
    kept.took(first_a, Some(("old-a".into(), Snapshot::default())));
    assert!(kept.snapshot.is_none());
    assert!(
        kept.reading,
        "older answers can't clear the new read's flag"
    );
    kept.took(second_a, Some(("new-a".into(), Snapshot::default())));
    kept.took(first_a, Some(("old-a".into(), Snapshot::default())));
    assert_eq!(kept.stamp, "new-a");
    assert!(kept.snapshot.is_some() && !kept.reading);
}

/// A profile's game folder with one resource pack and one registry mod, both on.
fn profile(name: &str) -> PathBuf {
    use std::io::Write as _;
    let home = std::env::temp_dir().join(format!(
        "sunrise-launcher-home-mods-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("resourcepacks")).unwrap();
    let file = std::fs::File::create(home.join("resourcepacks/Soft Edges.zip")).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file("pack.mcmeta", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(br#"{"pack":{"pack_format":99,"description":"made for the test"}}"#)
        .unwrap();
    zip.finish().unwrap();
    std::fs::write(
        home.join("options.txt"),
        "resourcePacks:[\"vanilla\",\"file/Soft Edges.zip\"]\n",
    )
    .unwrap();
    std::fs::create_dir_all(home.join("sunrise-native-mods")).unwrap();
    std::fs::write(
        home.join("sunrise-native-mods/lanternlight.smod"),
        b"placeholder",
    )
    .unwrap();
    std::fs::write(home.join("sunrise-mods.txt"), "mod.lanternlight:true\n").unwrap();
    std::fs::write(
        home.join("mod-browser.json"),
        serde_json::json!({"entries": [{
            "source": "sunrise", "project": "lanternlight", "slug": "lanternlight",
            "title": "Lanternlight", "kind": "mod", "version": "1.0.0", "number": "1.0.0",
            "file": "sunrise-native-mods/lanternlight.smod",
            "checksum": format!("SHA-256:{}", "0".repeat(64)), "page": "", "dependency": false,
        }]})
        .to_string(),
    )
    .unwrap();
    home
}

#[test]
fn the_watched_snapshot_updates_pack_mod_and_launch_state_together() {
    let home = profile("cli");
    let mut query = query(&home);
    query.version = Some("test-base".into());
    std::fs::write(
        home.join("native-left-out.json"),
        r#"{
        "base": "test-base",
        "left_out": [
            {"id": "lanternlight", "version": "1.0.0", "why": "did-not-build"},
            {"id": "lanternlight", "version": "0.9.0", "why": "did-not-build"},
            {"id": "absent", "version": "1.0.0", "why": "did-not-build"}
        ]
    }"#,
    )
    .unwrap();
    let mut kept = Inventory::default();
    let now = Instant::now();
    let (generation, known) = kept.draw(&query, now).unwrap();
    kept.took(generation, read_changed(&query, &known));
    let snapshot = kept.snapshot.as_deref().unwrap();
    assert_eq!(names(&snapshot.tabs.mods), ["Lanternlight"]);
    assert_eq!(names(&snapshot.tabs.resources), ["Soft Edges.zip"]);
    assert_eq!(snapshot.tabs.mods_off, 0);
    assert!(snapshot.tabs.resources[0].on);
    assert_eq!(
        snapshot.launch.mods_off, 1,
        "only the enabled recorded version"
    );
    assert!(read_changed(&query, &kept.stamp).is_none());

    // `sunrise-launcher packs off` and `mods disable` use these same setters.
    let pack = packs::find(&home, PackKind::Resource, "Soft Edges.zip").unwrap();
    packs::set_enabled(&home, &pack, false).unwrap();
    mod_list::set_enabled(&home, "lanternlight", false).unwrap();
    assert_eq!(
        kept.launch().unwrap().mods_off,
        1,
        "launch redraw reads only its current snapshot"
    );
    assert_ne!(stamp(&query), kept.stamp);
    kept.stale = true;
    let (generation, known) = kept.draw(&query, now).unwrap();
    kept.took(generation, read_changed(&query, &known));
    let snapshot = kept.snapshot.as_deref().unwrap();
    assert!(snapshot.tabs.mods.is_empty());
    assert_eq!(snapshot.tabs.mods_off, 1);
    assert!(!snapshot.tabs.resources[0].on);
    assert_eq!(snapshot.launch.mods_off, 0);
    std::fs::remove_dir_all(&home).unwrap();
}

#[test]
fn home_read_marks_manual_from_source_and_local_mods_with_badges() {
    use sunrise_launcher::github_mods::{
        install_local_packages, read_local_folder, read_local_smod,
    };
    use sunrise_launcher_smod::Package;

    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("profile");
    let smod = dir.path().join("local-mod.smod");
    let bytes = Package::write_files(
        "id = 'local_mod'\nname = 'Local Mod'\nversion = '0.1.0'\napi = '0.2'\nside = 'both'\n",
        &[],
    )
    .unwrap();
    std::fs::write(&smod, &bytes).unwrap();
    let local_pkg = read_local_smod(&smod).unwrap();

    let repo = dir.path().join("repo-mod");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(
        repo.join("sunrise-mod.toml"),
        "id = 'repo_mod'\nname = 'Repo Mod'\nversion = '0.2.0'\napi = '0.2'\nside = 'both'\n",
    )
    .unwrap();
    let folder_pkgs = read_local_folder(&repo).unwrap();

    install_local_packages(&home, &[local_pkg]).unwrap();
    install_local_packages(&home, &folder_pkgs).unwrap();

    let items = read(&home);
    let local_item = items.iter().find(|i| i.id == "local_mod").unwrap();
    let repo_item = items.iter().find(|i| i.id == "repo_mod").unwrap();
    assert_eq!(
        local_item.manual_badge,
        Some(crate::mods::ManualBadge::Local)
    );
    assert_eq!(
        repo_item.manual_badge,
        Some(crate::mods::ManualBadge::FromSource)
    );
}

#[test]
fn switch_held_mods_count_as_turned_off_on_home() {
    let home = profile("held");
    let extra = sunrise_launcher_smod::Package::write_files(
        "id='creature'\nname='Creature Library'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n",
        &[],
    )
    .unwrap();
    let mods_dir = home.join("sunrise-mods");
    std::fs::create_dir_all(&mods_dir).unwrap();
    std::fs::write(mods_dir.join("creature.smod"), &extra).unwrap();
    std::fs::create_dir_all(home.join("mod-browser")).unwrap();
    std::fs::write(
        home.join("mod-browser/active-build.json"),
        serde_json::json!({
            "version": 1,
            "profile": vec![1u8; 16],
            "generation": 1,
            "base": {"tag": "dev-2026-11-22-bbbbbbb", "executable": vec![2u8; 32], "runtime": vec![2u8; 32]},
            "executable": {"tag": "dev-2026-11-22-bbbbbbb", "executable": vec![2u8; 32], "runtime": vec![2u8; 32]},
            "resources": {"primary": vec![3u8; 32], "accelerator": null},
            "inputs": vec![4u8; 32],
            "receipts": [{
                "id": vec![5u8; 16],
                "waiting_for_update": [{
                    "id": "creature",
                    "title": "Creature Library",
                    "digest": vec![6u8; 32],
                    "root": "content",
                    "leaf": "creature.smod",
                    "source": {"kind": "local", "version": "1.0.0"},
                    "enabled_before": true,
                    "reason": {"kind": "unproved", "build": "dev-2026-11-22-bbbbbbb"}
                }],
                "removed": [],
                "acknowledged": false
            }]
        })
        .to_string(),
    )
    .unwrap();
    assert!(
        sunrise_launcher::managed_play::Active::read_pointer(&home)
            .unwrap()
            .is_some(),
        "fixture pointer must parse"
    );
    let tabs: Collections = read(&home).into_iter().collect();
    assert_eq!(names(&tabs.mods), ["Lanternlight"]);
    assert_eq!(tabs.mods_off, 1);
    std::fs::remove_dir_all(&home).unwrap();
}
