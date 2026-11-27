use super::*;
use crate::worlds::tests::{name, scratch, world};

/// A zip of `files` (path, bytes), written to `path`.
fn zip_of(path: &Path, files: &[(&str, &[u8])]) {
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    for (entry, bytes) in files {
        zip.start_file(*entry, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

/// A world's files, read back as a zip would hold them: its level.dat and a region file.
fn world_files(scratch_home: &Path) -> (Vec<u8>, Vec<u8>) {
    let dir = world(scratch_home, "Source", vec![("LevelName", name("Zipped"))]);
    (
        fs::read(dir.join("level.dat")).unwrap(),
        fs::read(dir.join("region/r.0.0.mca")).unwrap(),
    )
}

#[test]
fn a_world_folder_is_copied_in() {
    let from = scratch("import-folder-from");
    let home = scratch("import-folder-home");
    let dir = world(&from, "Friend's world", vec![("LevelName", name("Friend"))]);
    let imported = import_into(&home, &dir).unwrap();
    assert_eq!(imported.folder, "Friend's world");
    assert_eq!(imported.name, "Friend");
    assert!(home.join("saves/Friend's world/region/r.0.0.mca").is_file());
    // The game's lock is never carried over, and the source is left alone.
    assert!(!home.join("saves/Friend's world").join(LOCK).exists());
    assert!(dir.join("level.dat").is_file());
    fs::remove_dir_all(from).unwrap();
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn a_zip_with_the_world_in_a_folder_is_extracted() {
    let from = scratch("import-nested-from");
    let home = scratch("import-nested-home");
    let (dat, region) = world_files(&from);
    let zip = from.join("backup.zip");
    zip_of(
        &zip,
        &[
            ("Nested/level.dat", &dat),
            ("Nested/region/r.0.0.mca", &region),
            ("Nested/session.lock", b"x"),
            ("__MACOSX/Nested/._level.dat", b"junk"),
        ],
    );
    let imported = import_into(&home, &zip).unwrap();
    assert_eq!(imported.folder, "Nested");
    assert_eq!(imported.name, "Zipped");
    assert_eq!(
        fs::read(home.join("saves/Nested/region/r.0.0.mca")).unwrap(),
        region
    );
    assert!(!home.join("saves/Nested").join(LOCK).exists());
    assert!(!home.join("saves/__MACOSX").exists());
    fs::remove_dir_all(from).unwrap();
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn a_zip_with_the_world_at_its_root_is_named_after_the_zip() {
    let from = scratch("import-root-from");
    let home = scratch("import-root-home");
    let (dat, region) = world_files(&from);
    let zip = from.join("My Survival.zip");
    zip_of(&zip, &[("level.dat", &dat), ("region/r.0.0.mca", &region)]);
    let imported = import_into(&home, &zip).unwrap();
    assert_eq!(imported.folder, "My Survival");
    assert!(home.join("saves/My Survival/region/r.0.0.mca").is_file());
    fs::remove_dir_all(from).unwrap();
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn input_that_is_not_a_world_is_refused_and_leaves_nothing() {
    let from = scratch("import-invalid-from");
    let home = scratch("import-invalid-home");
    let empty = from.join("Empty");
    fs::create_dir_all(&empty).unwrap();
    let text = from.join("notes.txt");
    fs::write(&text, "hello").unwrap();
    let no_level = from.join("no-level.zip");
    zip_of(&no_level, &[("Stuff/readme.txt", b"hi")]);
    let bad_level = from.join("bad-level.zip");
    zip_of(&bad_level, &[("level.dat", b"not nbt")]);
    for source in [&empty, &text, &no_level, &bad_level, &from.join("missing")] {
        assert!(import_into(&home, source).is_err(), "{}", source.display());
    }
    assert!(list(&home).is_empty());
    // No staging folder is left in saves/.
    assert_eq!(fs::read_dir(home.join("saves")).unwrap().count(), 0);
    fs::remove_dir_all(from).unwrap();
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn a_name_that_is_taken_gets_a_number_in_its_folder_and_level_name() {
    let from = scratch("import-clash-from");
    let home = scratch("import-clash-home");
    let dir = world(&from, "Same", vec![("LevelName", name("Same"))]);
    world(&home, "Same", vec![("LevelName", name("Same"))]);
    let first = import_into(&home, &dir).unwrap();
    assert_eq!(
        (first.folder.as_str(), first.name.as_str()),
        ("Same (2)", "Same (2)")
    );
    let second = import_into(&home, &dir).unwrap();
    assert_eq!(second.folder, "Same (3)");
    // The world that was there is untouched.
    assert_eq!(find(&home, "Same").unwrap().name, "Same");
    fs::remove_dir_all(from).unwrap();
    fs::remove_dir_all(home).unwrap();
}
