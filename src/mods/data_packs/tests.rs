use super::*;
use std::io::Write;
use std::time::SystemTime;

/// A setup's folder with one world per name.
fn home(names: &[&str]) -> (tempfile::TempDir, Vec<World>) {
    let dir = tempfile::tempdir().unwrap();
    let worlds = names
        .iter()
        .map(|n| {
            let path = dir.path().join("saves").join(n);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("level.dat"), b"").unwrap();
            World {
                folder: n.to_string(),
                path,
                name: n.to_string(),
                mode: "Survival",
                last_played: SystemTime::UNIX_EPOCH,
                version: None,
                size: 0,
                icon: None,
                mods: Vec::new(),
            }
        })
        .collect();
    (dir, worlds)
}

/// A data pack zip the game reads.
fn pack_zip(dir: &Path) -> PathBuf {
    let path = dir.join("lanterns.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("pack.mcmeta", options).unwrap();
    zip.write_all(br#"{"pack":{"pack_format":48,"description":"Lanterns"}}"#)
        .unwrap();
    zip.start_file("data/lanterns/function/tick.mcfunction", options)
        .unwrap();
    zip.write_all(b"say hi").unwrap();
    zip.finish().unwrap();
    path
}

fn listed(home: &Path, world: &World) -> Vec<Listed> {
    let notes = origins(home);
    datapacks::list(&world.path)
        .into_iter()
        .map(|pack| Listed {
            origin: notes
                .iter()
                .find(|o| o.world == world.folder && o.file == pack.file)
                .cloned(),
            pack,
        })
        .collect()
}

#[test]
fn a_copy_lands_in_each_chosen_world_and_is_known_there_afterwards() {
    let (dir, worlds) = home(&["One", "Two", "Three"]);
    let outside = tempfile::tempdir().unwrap();
    let first = datapacks::add(&worlds[0].path, &pack_zip(outside.path())).unwrap();
    let origin = Origin {
        world: "One".into(),
        file: first.file.clone(),
        source: SourceId::Modrinth,
        project: "lanterns".into(),
        title: "Lanterns".into(),
        number: "1.2".into(),
    };
    note_origins(dir.path(), vec![origin.clone()]).unwrap();
    let what = What::Copy {
        from: worlds[0].path.clone(),
        title: "Lanterns 1.2".into(),
        pack: first,
        origin: Some(origin),
    };
    assert!(what.in_world(&worlds[0], &[]));
    assert!(!what.in_world(&worlds[1], &listed(dir.path(), &worlds[1])));

    let said = add_to_worlds(&what, dir.path(), &worlds[1..], "26.3", &|_| {}).unwrap();
    assert_eq!(said.lines().next(), Some("Added to 2 worlds"));
    assert!(said.contains("Two\nThree"), "{said}");
    for world in &worlds[1..] {
        assert!(world.path.join("datapacks/lanterns.zip").is_file());
        // The project it came from travels with the file.
        assert!(what.in_world(world, &listed(dir.path(), world)));
    }
    // A removed file's note is no longer read.
    std::fs::remove_file(worlds[2].path.join("datapacks/lanterns.zip")).unwrap();
    assert_eq!(origins(dir.path()).len(), 2);
}

#[test]
fn a_world_that_left_the_setup_is_named_and_the_others_still_get_the_pack() {
    let (dir, worlds) = home(&["One", "Two", "Gone"]);
    let outside = tempfile::tempdir().unwrap();
    let pack = datapacks::add(&worlds[0].path, &pack_zip(outside.path())).unwrap();
    std::fs::remove_dir_all(&worlds[2].path).unwrap();
    let what = What::Copy {
        from: worlds[0].path.clone(),
        title: "lanterns".into(),
        pack,
        origin: None,
    };
    let said = add_to_worlds(&what, dir.path(), &worlds[1..], "26.3", &|_| {}).unwrap_err();
    assert!(worlds[1].path.join("datapacks/lanterns.zip").is_file());
    assert!(
        said.starts_with("Added to 1 world · 1 failed\n")
            && said.contains("Added to:\nTwo\n")
            && said.contains("Failed:\nGone:"),
        "{said}"
    );
}
