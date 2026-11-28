use super::*;
use std::fs;

fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("a parent")).expect("a folder");
    fs::write(path, text).expect("a file");
}

/// A game folder with one of each thing an import brings, and a mod it must not.
fn game(dir: &Path) {
    put(&dir.join("saves/World/level.dat"), "world");
    put(&dir.join("saves/World/session.lock"), "lock");
    put(&dir.join("saves/World/region/r.0.0.mca"), "region");
    put(&dir.join("saves/Notes.txt"), "not a world");
    put(
        &dir.join("options.txt"),
        "fov:100\nkey_key.jump:key.keyboard.space\n",
    );
    put(&dir.join("optionsof.txt"), "ofFastRender:true\n");
    put(&dir.join("resourcepacks/faithful.zip"), "pack");
    put(&dir.join("shaderpacks/glow.zip"), "shader");
    put(&dir.join("screenshots/a.png"), "shot");
    put(&dir.join("screenshots/b.txt"), "not a shot");
    put(&dir.join("servers.dat"), "servers");
    put(&dir.join("mods/sodium.jar"), "mod");
    put(&dir.join("config/sodium.json"), "mod config");
}

fn folders(root: &Path) -> Folders {
    Folders {
        home: root.join("home"),
        data: root.join("data"),
        config: root.join("config"),
    }
}

fn found(root: &Path) -> Vec<Foreign> {
    import_sources_in(Os::Linux, &folders(root), &[])
}

#[test]
fn prism_profiles_are_found_with_their_play_time_and_version() {
    let root = tempfile::tempdir().expect("a folder");
    let prism = root.path().join("data/PrismLauncher/instances/Fabric 1.21");
    game(&prism.join(".minecraft"));
    put(
        &prism.join("instance.cfg"),
        "[General]\nname=My Fabric\ntotalTimePlayed=7200\nlastLaunchTime=1700000000000\n",
    );
    put(
        &prism.join("mmc-pack.json"),
        r#"{"components":[{"uid":"net.minecraft","version":"1.21"}]}"#,
    );
    // A profile that was never launched has no game folder, and a broken one no settings.
    put(
        &root
            .path()
            .join("data/PrismLauncher/instances/Never/instance.cfg"),
        "name=Never\n",
    );
    fs::create_dir_all(
        root.path()
            .join("data/PrismLauncher/instances/Broken/.minecraft"),
    )
    .expect("a folder");
    let all = found(root.path());
    assert_eq!(all.len(), 1, "{all:#?}");
    let one = &all[0];
    assert_eq!(one.key(), "prism-launcher/My Fabric");
    assert_eq!(one.version.as_deref(), Some("1.21"));
    assert_eq!(one.played_secs, Some(7200));
    assert_eq!(one.last_played, Some(1_700_000_000));
    assert_eq!(
        one.contents,
        Contents {
            worlds: 1,
            options: true,
            resource_packs: 1,
            shader_packs: 1,
            screenshots: 1,
            servers: true,
            mods: 1,
        }
    );
}

#[test]
fn each_launchers_layout_is_read() {
    let root = tempfile::tempdir().expect("a folder");
    let data = root.path().join("data");
    let modrinth = data.join("ModrinthApp/profiles/Cave");
    game(&modrinth);
    put(
        &modrinth.join("profile.json"),
        r#"{"metadata":{"name":"Cave Pack","game_version":"1.20.1","recent_time_played":60,
            "submitted_time_played":40,"last_played":"2024-01-02T03:04:05Z"}}"#,
    );
    let atl = data.join("atlauncher/instances/Vanilla");
    game(&atl);
    put(
        &atl.join("instance.json"),
        r#"{"id":"1.19.2","launcher":{"name":"Vanilla Plus","totalPlayTime":90}}"#,
    );
    let gd = data.join("gdlauncher_carbon/data/instances/Old");
    game(&gd.join("instance"));
    let gd_next = root.path().join("config/gdlauncher_next/instances/Legacy");
    game(&gd_next);
    put(
        &gd_next.join("config.json"),
        r#"{"loader":{"mcVersion":"1.16.5"},"lastPlayed":1700000000000}"#,
    );
    let official = root.path().join("home/.minecraft");
    game(&official);
    let all = found(root.path());
    let by = |key: &str| {
        all.iter()
            .find(|f| f.key() == key)
            .unwrap_or_else(|| panic!("{key} in {all:#?}"))
    };
    let cave = by("modrinth-app/Cave Pack");
    assert_eq!(cave.version.as_deref(), Some("1.20.1"));
    assert_eq!(cave.played_secs, Some(100));
    assert_eq!(cave.last_played, Some(1_704_164_645));
    assert_eq!(by("atlauncher/Vanilla Plus").played_secs, Some(90));
    assert_eq!(
        by("atlauncher/Vanilla Plus").version.as_deref(),
        Some("1.19.2")
    );
    assert_eq!(by("gdlauncher/Old").folder, gd.join("instance"));
    assert_eq!(by("gdlauncher/Legacy").version.as_deref(), Some("1.16.5"));
    assert_eq!(by("minecraft-launcher/Minecraft Launcher").folder, official);
}

#[test]
fn curseforge_profiles_are_found_on_windows_and_mac_only() {
    let root = tempfile::tempdir().expect("a folder");
    let dir = root
        .path()
        .join("home/curseforge/minecraft/Instances/All the Mods");
    game(&dir);
    put(
        &dir.join("minecraftinstance.json"),
        r#"{"name":"All the Mods","gameVersion":"1.20.1","lastPlayed":"2024-01-02T03:04:05.000Z"}"#,
    );
    let on = |os| import_sources_in(os, &folders(root.path()), &[]);
    let windows = on(Os::Windows);
    assert_eq!(windows.len(), 1, "{windows:#?}");
    assert_eq!(windows[0].key(), "curseforge/All the Mods");
    assert_eq!(windows[0].version.as_deref(), Some("1.20.1"));
    assert_eq!(windows[0].last_played, Some(1_704_164_645));
    assert!(on(Os::Linux).is_empty());
}

#[test]
fn a_folder_that_is_sunrises_own_is_left_out() {
    let root = tempfile::tempdir().expect("a folder");
    let official = root.path().join("home/.minecraft");
    game(&official);
    assert_eq!(found(root.path()).len(), 1);
    let own = import_sources_in(Os::Linux, &folders(root.path()), &[official]);
    assert!(own.is_empty());
}

#[test]
fn a_folder_picked_by_hand_is_read_like_a_launchers() {
    let root = tempfile::tempdir().expect("a folder");
    let plain = root.path().join("plain");
    game(&plain);
    let got = read_import_folder(&plain).expect("a game folder");
    assert_eq!(got.name, "plain");
    assert_eq!(got.folder, plain);
    // The root of a launcher's profile, with the game one folder down.
    let prism = root.path().join("prism");
    game(&prism.join("minecraft"));
    put(&prism.join("instance.cfg"), "name=Picked\n");
    assert_eq!(
        read_import_folder(&prism).expect("a profile").name,
        "Picked"
    );
    let empty = root.path().join("empty");
    fs::create_dir_all(&empty).expect("a folder");
    assert!(read_import_folder(&empty).is_err());
    assert!(read_import_folder(&root.path().join("missing")).is_err());
}

#[test]
fn a_profile_is_found_by_launcher_and_name() {
    let root = tempfile::tempdir().expect("a folder");
    game(
        &root
            .path()
            .join("data/PrismLauncher/instances/A/.minecraft"),
    );
    put(
        &root
            .path()
            .join("data/PrismLauncher/instances/A/instance.cfg"),
        "name=Alpha\n",
    );
    let all = found(root.path());
    for spec in [
        "prism-launcher/Alpha",
        "Prism Launcher/alpha",
        "alpha",
        "prism-launcher",
    ] {
        assert_eq!(find_import(&all, spec).expect(spec).name, "Alpha");
    }
    assert!(find_import(&all, "prism-launcher/Beta").is_err());
}

fn scratch(root: &Path) -> (PathBuf, PathBuf) {
    let (home, data) = (root.join("sunrise-home"), root.join("sunrise-data"));
    fs::create_dir_all(&home).expect("a home");
    fs::create_dir_all(&data).expect("a data folder");
    (home, data)
}

fn foreign(root: &Path) -> Foreign {
    let dir = root.join("data/PrismLauncher/instances/X");
    game(&dir.join(".minecraft"));
    put(
        &dir.join("instance.cfg"),
        "name=Imported\ntotalTimePlayed=300\n",
    );
    found(root).remove(0)
}

#[test]
fn an_import_brings_what_was_ticked_and_never_mods() {
    let root = tempfile::tempdir().expect("a folder");
    let from = foreign(root.path());
    let (home, data) = scratch(root.path());
    // The player's own options start the profile; the imported ones replace them.
    put(&home.join("options.txt"), "fov:70\n");
    let made = create_importing_in(
        &home,
        &data,
        "Imported",
        &from,
        CopyOptions {
            screenshots: false,
            ..CopyOptions::everything()
        },
        &Cancellation::default(),
        &Progress::default(),
        None,
        |_| {},
    )
    .expect("an import");
    let got = |rel: &str| fs::read_to_string(made.home.join(rel)).ok();
    assert_eq!(
        got("options.txt").as_deref(),
        Some("fov:100\nkey_key.jump:key.keyboard.space\n")
    );
    assert_eq!(got("optionsof.txt").as_deref(), Some("ofFastRender:true\n"));
    assert_eq!(
        got("saves/World/region/r.0.0.mca").as_deref(),
        Some("region")
    );
    assert!(!made.home.join("saves/World/session.lock").exists());
    assert!(made.home.join("resourcepacks/faithful.zip").is_file());
    assert!(made.home.join("shaderpacks/glow.zip").is_file());
    assert!(made.home.join("servers.dat").is_file());
    assert!(!made.home.join("screenshots").exists());
    assert!(!made.home.join("mods").exists());
    assert!(!made.home.join("config").exists());
    assert_eq!(made.played.as_secs(), 300);
    // What was read is as it was.
    assert!(from.folder.join("mods/sodium.jar").is_file());
}

#[test]
fn play_time_comes_only_when_it_is_ticked() {
    let root = tempfile::tempdir().expect("a folder");
    let from = foreign(root.path());
    let (home, data) = scratch(root.path());
    let made = create_importing_in(
        &home,
        &data,
        "Fresh",
        &from,
        CopyOptions::default(),
        &Cancellation::default(),
        &Progress::default(),
        None,
        |_| {},
    )
    .expect("an import");
    assert_eq!(made.played.as_secs(), 0);
}

#[test]
fn a_stopped_import_leaves_no_profile_and_no_folder() {
    let root = tempfile::tempdir().expect("a folder");
    let from = foreign(root.path());
    let (home, data) = scratch(root.path());
    let cancel = Cancellation::default();
    cancel.stop();
    let error = create_importing_in(
        &home,
        &data,
        "Stopped",
        &from,
        CopyOptions::everything(),
        &cancel,
        &Progress::default(),
        None,
        |_| {},
    );
    assert!(error.is_err());
    assert!(list_in(&home, &data).iter().all(|i| i.name != "Stopped"));
    assert!(!home.join("instances/stopped").exists());
}

#[cfg(unix)]
#[test]
fn a_linked_folder_is_not_followed_out_of_the_source() {
    let root = tempfile::tempdir().expect("a folder");
    let from = foreign(root.path());
    let outside = root.path().join("outside");
    put(&outside.join("secret.zip"), "secret");
    fs::remove_dir_all(from.folder.join("resourcepacks")).expect("removed");
    std::os::unix::fs::symlink(&outside, from.folder.join("resourcepacks")).expect("a link");
    let (home, data) = scratch(root.path());
    let made = create_importing_in(
        &home,
        &data,
        "Linked",
        &from,
        CopyOptions::everything(),
        &Cancellation::default(),
        &Progress::default(),
        None,
        |_| {},
    )
    .expect("an import");
    assert!(!made.home.join("resourcepacks").exists());
    assert!(made.home.join("saves/World/level.dat").is_file());
}
