use super::*;

/// A game home and data folder with the profiles of `placed` (name, group) made in that order.
fn with_groups(
    test: &str,
    placed: &[(&str, Option<&str>)],
) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::Builder::new()
        .prefix(&format!("sunrise-groups-{test}-"))
        .tempdir()
        .expect("a scratch folder");
    let (home, data) = (root.path().join("game"), root.path().join("data"));
    fs::create_dir_all(&home).expect("the game folder");
    for (name, group) in placed {
        create_whole_in(&home, &data, name, None, None, |m| {
            m.group = group.map(str::to_owned);
        })
        .expect("a profile");
    }
    (root, home, data)
}

fn names(home: &Path, data: &Path) -> Vec<String> {
    list_in(home, data).into_iter().map(|i| i.name).collect()
}

fn groups_of(home: &Path, data: &Path) -> Vec<String> {
    group_names(&list_in(home, data))
}

const THREE: [(&str, Option<&str>); 4] = [
    ("Attic", Some("Play")),
    ("Barn", Some("Play")),
    ("Cabin", Some("Beta")),
    ("Dock", Some("Zoo")),
];

#[test]
fn groups_never_moved_list_by_name_and_write_nothing() {
    let (_root, home, data) = with_groups("by-name", &THREE);
    assert_eq!(groups_of(&home, &data), ["Beta", "Play", "Zoo"]);
    assert_eq!(
        names(&home, &data),
        ["Sunrise", "Cabin", "Attic", "Barn", "Dock"]
    );
    let saved = fs::read_to_string(data.join(FILE)).expect("the saved choices");
    assert!(!saved.contains("group_order") && !saved.contains("collapsed_groups"));
}

#[test]
fn moving_a_group_swaps_it_with_its_neighbour_and_sticks() {
    let (_root, home, data) = with_groups("move", &THREE);
    move_group_in(&home, &data, "Zoo", true).expect("moves up");
    assert_eq!(groups_of(&home, &data), ["Beta", "Zoo", "Play"]);
    move_group_in(&home, &data, "Beta", false).expect("moves down");
    assert_eq!(groups_of(&home, &data), ["Zoo", "Beta", "Play"]);
    assert_eq!(
        names(&home, &data),
        ["Sunrise", "Dock", "Cabin", "Attic", "Barn"]
    );
    // At an end it stays, and nothing is written for it.
    move_group_in(&home, &data, "Zoo", true).expect("already first");
    move_group_in(&home, &data, "Play", false).expect("already last");
    assert_eq!(groups_of(&home, &data), ["Zoo", "Beta", "Play"]);
    assert!(move_group_in(&home, &data, "Nowhere", true).is_err());
}

#[test]
fn a_group_made_after_a_move_comes_after_the_placed_ones() {
    let (_root, home, data) = with_groups("later", &THREE);
    move_group_in(&home, &data, "Zoo", true).expect("moves up");
    create_whole_in(&home, &data, "Echo", None, None, |m| {
        m.group = Some("Alpha".into())
    })
    .expect("a profile");
    assert_eq!(groups_of(&home, &data), ["Beta", "Zoo", "Play", "Alpha"]);
}

#[test]
fn ungrouped_profiles_stay_above_every_group() {
    let (_root, home, data) = with_groups("loose", &[("Attic", Some("Play")), ("Barn", None)]);
    move_group_in(&home, &data, "Play", true).expect("alone: nothing to swap");
    assert_eq!(names(&home, &data), ["Sunrise", "Barn", "Attic"]);
}

#[test]
fn renaming_moves_every_profile_and_keeps_the_place_and_fold() {
    let (_root, home, data) = with_groups("rename", &THREE);
    move_group_in(&home, &data, "Zoo", true).expect("moves up");
    collapse_group_in(&data, "Zoo", true).expect("folds");
    rename_group_in(&home, &data, "Play", "  Main ").expect("renames");
    assert_eq!(groups_of(&home, &data), ["Beta", "Zoo", "Main"]);
    let group = |name: &str| {
        list_in(&home, &data)
            .into_iter()
            .find(|i| i.name == name)
            .and_then(|i| i.group)
    };
    assert_eq!(group("Attic").as_deref(), Some("Main"));
    assert_eq!(group("Barn").as_deref(), Some("Main"));
    rename_group_in(&home, &data, "Zoo", "Park").expect("renames a folded one");
    let choices = group_choices_in(&data);
    assert_eq!(choices.order, ["Beta", "Park", "Main"]);
    assert!(choices.collapsed.contains("Park") && !choices.collapsed.contains("Zoo"));
    assert!(rename_group_in(&home, &data, "Park", " ").is_err());
    assert!(rename_group_in(&home, &data, "Nowhere", "Anywhere").is_err());
}

#[test]
fn renaming_to_an_existing_group_joins_the_two() {
    let (_root, home, data) = with_groups("join", &THREE);
    move_group_in(&home, &data, "Zoo", true).expect("moves up");
    rename_group_in(&home, &data, "Beta", "Play").expect("joins");
    assert_eq!(groups_of(&home, &data), ["Zoo", "Play"]);
    assert_eq!(group_choices_in(&data).order, ["Zoo", "Play"]);
}

#[test]
fn ungrouping_keeps_the_profiles_and_forgets_the_group() {
    let (_root, home, data) = with_groups("ungroup", &THREE);
    move_group_in(&home, &data, "Zoo", true).expect("moves up");
    collapse_group_in(&data, "Play", true).expect("folds");
    ungroup_group_in(&home, &data, "Play").expect("ungroups");
    assert_eq!(groups_of(&home, &data), ["Beta", "Zoo"]);
    let all = list_in(&home, &data);
    for name in ["Attic", "Barn"] {
        let found = all.iter().find(|i| i.name == name).expect("still listed");
        assert_eq!(found.group, None);
    }
    let choices = group_choices_in(&data);
    assert_eq!(choices.order, ["Beta", "Zoo"]);
    assert!(choices.collapsed.is_empty());
}

#[test]
fn folding_is_saved_and_undone() {
    let (_root, home, data) = with_groups("fold", &THREE);
    collapse_group_in(&data, "Play", true).expect("folds");
    assert!(group_choices_in(&data).collapsed.contains("Play"));
    collapse_group_in(&data, "Play", false).expect("unfolds");
    assert!(group_choices_in(&data).collapsed.is_empty());
    // Folding changes nothing in the listing.
    assert_eq!(groups_of(&home, &data), ["Beta", "Play", "Zoo"]);
}

#[test]
fn unreadable_group_choices_leave_the_profiles_alone() {
    let (_root, home, data) = with_groups("tolerant", &THREE);
    let text = fs::read_to_string(data.join(FILE)).expect("the saved choices");
    let mut saved: serde_json::Value = serde_json::from_str(&text).expect("json");
    saved["group_order"] = serde_json::json!(7);
    saved["collapsed_groups"] = serde_json::json!({"a": 1});
    fs::write(data.join(FILE), saved.to_string()).expect("written");
    assert_eq!(groups_of(&home, &data), ["Beta", "Play", "Zoo"]);
    assert_eq!(group_choices_in(&data), GroupChoices::default());
}

#[test]
fn a_move_writes_the_order_beside_the_profiles_choices() {
    let (_root, home, data) = with_groups("survive", &THREE);
    move_group_in(&home, &data, "Zoo", true).expect("moves up");
    let saved = fs::read_to_string(data.join(FILE)).expect("the saved choices");
    assert!(saved.contains("group_order"));
}
