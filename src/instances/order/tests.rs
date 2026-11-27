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
    move_profile_in(&home, &data, "sunrise", true).expect("already first");
    move_profile_in(&home, &data, "barn", false).expect("already last");
    assert_eq!(names(&home, &data), ["Sunrise", "Cabin", "Attic", "Barn"]);
    // The default profile is no longer held first.
    move_profile_in(&home, &data, "cabin", true).expect("moves past the default");
    assert_eq!(names(&home, &data), ["Cabin", "Sunrise", "Attic", "Barn"]);
}

#[test]
fn the_default_profile_moves_like_any_other_and_starts_first() {
    let (_root, home, data) = with_profiles("default", &["Attic", "Barn"]);
    // Never moved, it keeps the place it always had.
    assert_eq!(names(&home, &data), ["Sunrise", "Attic", "Barn"]);
    move_profile_in(&home, &data, DEFAULT, true).expect("already first: nothing happens");
    assert_eq!(names(&home, &data), ["Sunrise", "Attic", "Barn"]);
    let saved = fs::read_to_string(data.join(FILE)).expect("the saved choices");
    assert!(!saved.contains("order"), "an edge move writes nothing");
    move_profile_in(&home, &data, DEFAULT, false).expect("moves down");
    assert_eq!(names(&home, &data), ["Attic", "Sunrise", "Barn"]);
    move_profile_in(&home, &data, "barn", true).expect("moves up past it");
    assert_eq!(names(&home, &data), ["Attic", "Barn", "Sunrise"]);
    move_profile_in(&home, &data, DEFAULT, false).expect("already last");
    assert_eq!(names(&home, &data), ["Attic", "Barn", "Sunrise"]);
    // The order is on disk, the default's place included.
    let saved = Store::load(&data);
    assert_eq!(saved.instances[DEFAULT].order, Some(2));
    assert_eq!(saved.instances["attic"].order, Some(0));
    assert_eq!(saved.instances["barn"].order, Some(1));
}

#[test]
fn two_profiles_swap_either_way() {
    let (_root, home, data) = with_profiles("two", &["Attic"]);
    move_profile_in(&home, &data, DEFAULT, false).expect("down");
    assert_eq!(names(&home, &data), ["Attic", "Sunrise"]);
    move_profile_in(&home, &data, DEFAULT, true).expect("up");
    assert_eq!(names(&home, &data), ["Sunrise", "Attic"]);
    move_profile_in(&home, &data, "attic", true).expect("the other one up");
    assert_eq!(names(&home, &data), ["Attic", "Sunrise"]);
    move_profile_in(&home, &data, "attic", false).expect("and down");
    assert_eq!(names(&home, &data), ["Sunrise", "Attic"]);
}

#[test]
fn a_profile_made_after_the_default_moved_comes_last() {
    let (_root, home, data) = with_profiles("made", &["Attic"]);
    move_profile_in(&home, &data, DEFAULT, false).expect("down");
    create_whole_in(&home, &data, "Aaa", None, None, |_| {}).expect("a new one");
    assert_eq!(names(&home, &data), ["Attic", "Sunrise", "Aaa"]);
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

#[test]
fn only_the_moves_that_are_possible_are_offered() {
    let (_root, home, data) = with_profiles("options", &["Attic", "Barn", "Cabin"]);
    let list = list_in(&home, &data);
    let options = |name: &str| {
        let target = list.iter().find(|i| i.name == name).expect("listed");
        move_options(&list, target)
    };
    assert_eq!(options("Sunrise"), (false, true));
    assert_eq!(options("Attic"), (true, true));
    assert_eq!(options("Barn"), (true, true));
    assert_eq!(options("Cabin"), (true, false));
    // A profile alone in its group has nowhere to go.
    edit_in(&home, &data, "attic", |m| m.group = Some("Farm".into())).expect("grouped");
    let list = list_in(&home, &data);
    let attic = list.iter().find(|i| i.name == "Attic").expect("listed");
    assert_eq!(move_options(&list, attic), (false, false));
    // With one other profile, each can go one way.
    let (_root, home, data) = with_profiles("two", &["s"]);
    let list = list_in(&home, &data);
    assert_eq!(move_options(&list, &list[0]), (false, true));
    assert_eq!(move_options(&list, &list[1]), (true, false));
}

#[test]
fn a_move_at_an_edge_changes_nothing_on_disk() {
    let (_root, home, data) = with_profiles("edge", &["s"]);
    move_profile_in(&home, &data, DEFAULT, true).expect("nothing above the default");
    move_profile_in(&home, &data, "s", false).expect("nothing below");
    assert_eq!(names(&home, &data), ["Sunrise", "s"]);
    let saved = fs::read_to_string(data.join(FILE)).expect("the saved choices");
    assert!(!saved.contains("order"));
}
