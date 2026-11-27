use super::super::create::{create_with_publish, CopyFrom};
use super::super::edit::edit_in;
use super::*;
use crate::instances::*;
use std::fs;
use std::path::{Path, PathBuf};

fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("a parent")).expect("a folder");
    fs::write(path, text).expect("a file");
}

/// A game home with something of each kind, and a played default profile.
fn played_home(root: &Path) -> (PathBuf, PathBuf) {
    let (home, data) = (root.join("home"), root.join("data"));
    put(&home.join("saves/World/level.dat"), "world");
    put(&home.join("options.txt"), "fov:90\n");
    put(&home.join("resourcepacks/faithful.zip"), "pack");
    put(&home.join("screenshots/a.png"), "shot");
    put(&home.join("shaderpacks/glow.zip"), "shader");
    put(&home.join("servers.dat"), "servers");
    put(&home.join("sunrise-mods/atlas/sunrise-mod.toml"), "mod");
    put(&home.join("sunrise-mods.txt"), "atlas");
    edit_in(&home, &data, DEFAULT, |m| {
        m.played_secs = 7200;
        m.last_played = Some(1_700_000_000);
    })
    .expect("play time saved");
    (home, data)
}

fn copy_of(home: &Path, data: &Path, options: CopyOptions) -> Instance {
    let from = list_in(home, data).remove(0);
    create_with_publish(
        home,
        data,
        "Copy",
        Some(CopyFrom::Selected(&from, options)),
        None,
        |_| {},
        |_| {},
    )
    .expect("a copy")
}

#[test]
fn a_copy_with_the_default_options_is_what_a_copy_always_was() {
    let root = tempfile::tempdir().expect("a folder");
    let (home, data) = played_home(root.path());
    let copy = copy_of(&home, &data, CopyOptions::default());
    for kept in [
        "saves/World/level.dat",
        "options.txt",
        "resourcepacks/faithful.zip",
        "screenshots/a.png",
        "shaderpacks/glow.zip",
        "servers.dat",
        "sunrise-mods/atlas/sunrise-mod.toml",
        "sunrise-mods.txt",
    ] {
        assert!(copy.home.join(kept).is_file(), "{kept} is copied");
    }
    assert_eq!(copy.played, std::time::Duration::ZERO);
    assert_eq!(copy.last_played, None);
}

#[test]
fn each_option_off_leaves_out_only_its_own_files() {
    type Off = fn(&mut CopyOptions);
    let cases: [(Off, &[&str]); 7] = [
        (|o| o.worlds = false, &["saves/World/level.dat"]),
        (|o| o.options = false, &["options.txt"]),
        (
            |o| o.resource_packs = false,
            &["resourcepacks/faithful.zip"],
        ),
        (|o| o.screenshots = false, &["screenshots/a.png"]),
        (|o| o.shader_packs = false, &["shaderpacks/glow.zip"]),
        (|o| o.servers = false, &["servers.dat"]),
        (
            |o| o.mods = false,
            &["sunrise-mods/atlas/sunrise-mod.toml", "sunrise-mods.txt"],
        ),
    ];
    let everything = [
        "saves/World/level.dat",
        "options.txt",
        "resourcepacks/faithful.zip",
        "screenshots/a.png",
        "shaderpacks/glow.zip",
        "servers.dat",
        "sunrise-mods/atlas/sunrise-mod.toml",
        "sunrise-mods.txt",
    ];
    for (n, (turn_off, gone)) in cases.into_iter().enumerate() {
        let root = tempfile::tempdir().expect("a folder");
        let (home, data) = played_home(root.path());
        let mut options = CopyOptions::everything();
        turn_off(&mut options);
        let from = list_in(&home, &data).remove(0);
        let copy = create_with_publish(
            &home,
            &data,
            &format!("Copy {n}"),
            Some(CopyFrom::Selected(&from, options)),
            None,
            |_| {},
            |_| {},
        )
        .expect("a copy");
        for file in everything {
            assert_eq!(
                copy.home.join(file).is_file(),
                !gone.contains(&file),
                "{file} with case {n}"
            );
        }
    }
}

#[test]
fn play_time_is_kept_only_when_asked() {
    let root = tempfile::tempdir().expect("a folder");
    let (home, data) = played_home(root.path());
    let kept = copy_of(&home, &data, CopyOptions::everything());
    assert_eq!(kept.played, std::time::Duration::from_secs(7200));
    assert!(kept.last_played.is_some());
}
