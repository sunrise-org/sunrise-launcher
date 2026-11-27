use super::*;
use crate::worlds::tests::{name, scratch, world};

#[test]
fn renaming_changes_the_level_name_and_keeps_the_folder() {
    let home = scratch("rename");
    world(&home, "Old", vec![("LevelName", name("Old"))]);
    let renamed = rename_in(&home, "Old", "  Fresh start ").unwrap();
    assert_eq!(renamed.name, "Fresh start");
    assert_eq!(renamed.folder, "Old");
    assert_eq!(find(&home, "Old").unwrap().name, "Fresh start");
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn an_empty_name_or_an_unknown_world_is_refused() {
    let home = scratch("rename-refused");
    world(&home, "Old", vec![("LevelName", name("Old"))]);
    assert!(rename_in(&home, "Old", "   ").is_err());
    assert!(rename_in(&home, "Missing", "X").is_err());
    assert_eq!(find(&home, "Old").unwrap().name, "Old");
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn a_world_the_game_has_open_is_not_renamed() {
    let home = scratch("rename-open");
    let dir = world(&home, "Open", vec![("LevelName", name("Open"))]);
    let game = fs::File::options()
        .write(true)
        .open(dir.join(LOCK))
        .unwrap();
    game.try_lock().unwrap();
    assert!(rename_in(&home, "Open", "Shut").is_err());
    assert_eq!(find(&home, "Open").unwrap().name, "Open");
    game.unlock().unwrap();
    fs::remove_dir_all(home).unwrap();
}
