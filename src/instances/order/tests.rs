use super::*;

/// A game home and data folder with the profiles `names` made in that order, in no group.
fn with_profiles(test: &str, names: &[&str]) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::Builder::new()
        .prefix(&format!("sunrise-order-{test}-"))
        .tempdir()
        .expect("a scratch folder");
    let (home, data) = (root.path().join("game"), root.path().join("data"));
    fs::create_dir_all(&home).expect("the game folder");
    for name in names {
        create_whole_in(&home, &data, name, None, None, |_| {}).expect("a profile");
    }
    (root, home, data)
}

fn names(home: &Path, data: &Path) -> Vec<String> {
    list_in(home, data).into_iter().map(|i| i.name).collect()
}

#[test]
fn old_files_without_an_order_list_by_name() {
    let (_root, home, data) = with_profiles("old", &["Cabin", "Attic", "Barn"]);
    assert_eq!(names(&home, &data), ["Sunrise", "Attic", "Barn", "Cabin"]);
    let saved = fs::read_to_string(data.join(FILE)).expect("the saved choices");
    assert!(!saved.contains("order"), "nothing is written until a move");
}

#[test]
fn moving_swaps_a_profile_with_its_neighbour() {
    let (_root, home, data) = with_profiles("swap", &["Attic", "Barn", "Cabin"]);
    move_profile_in(&home, &data, "cabin", true).expect("moves up");
    assert_eq!(names(&home, &data), ["Sunrise", "Attic", "Cabin", "Barn"]);
    move_profile_in(&home, &data, "attic", false).expect("moves down");
    assert_eq!(names(&home, &data), ["Sunrise", "Cabin", "Attic", "Barn"]);
    // At an end it stays.
    move_profile_in(&home, &data, "cabin", true).expect("already first");
    move_profile_in(&home, &data, "barn", false).expect("already last");
    assert_eq!(names(&home, &data), ["Sunrise", "Cabin", "Attic", "Barn"]);
}

#[test]
fn the_default_profile_stays_first_and_does_not_move() {
    let (_root, home, data) = with_profiles("default", &["Attic", "Barn"]);
    assert!(move_profile_in(&home, &data, DEFAULT, false).is_err());
    move_profile_in(&home, &data, "attic", true).expect("already first of its group");
    move_profile_in(&home, &data, "barn", true).expect("moves up");
    assert_eq!(names(&home, &data), ["Sunrise", "Barn", "Attic"]);
}

#[test]
fn moving_only_reorders_within_the_group_and_the_order_is_kept() {
    let (_root, home, data) = with_profiles("group", &["Attic", "Barn", "Cabin"]);
    edit_in(&home, &data, "attic", |m| m.group = Some("Farm".into())).expect("grouped");
    edit_in(&home, &data, "barn", |m| m.group = Some("Farm".into())).expect("grouped");
    // Cabin has no group and is alone in it; Farm sorts after the ungrouped.
    move_profile_in(&home, &data, "barn", true).expect("moves up");
    assert_eq!(names(&home, &data), ["Sunrise", "Cabin", "Barn", "Attic"]);
    move_profile_in(&home, &data, "cabin", false).expect("alone: nothing to swap");
    assert_eq!(names(&home, &data), ["Sunrise", "Cabin", "Barn", "Attic"]);
    // A rename or a new profile does not undo it: unordered ones come after.
    edit_in(&home, &data, "attic", |m| m.name = Some("Zed".into())).expect("renamed");
    create_whole_in(&home, &data, "Aaa", None, None, |_| {}).expect("a new one");
    let saved = Store::load(&data);
    assert_eq!(saved.instances["barn"].order, Some(0));
    assert_eq!(saved.instances["attic"].order, Some(1));
    assert_eq!(
        names(&home, &data),
        ["Sunrise", "Aaa", "Cabin", "Barn", "Zed"]
    );
}

#[test]
fn an_unreadable_order_is_read_as_none() {
    let meta: Meta = serde_json::from_str(r#"{"order": "first"}"#).expect("tolerant");
    assert_eq!(meta.order, None);
    let meta: Meta = serde_json::from_str(r#"{"order": 3}"#).expect("a number");
    assert_eq!(meta.order, Some(3));
}
