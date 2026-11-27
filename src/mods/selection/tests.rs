use super::*;

fn order(keys: &[&str]) -> Vec<String> {
    keys.iter().map(|key| (*key).to_owned()).collect()
}

fn picked(selection: &Selection) -> Vec<&str> {
    selection.keys().iter().map(String::as_str).collect()
}

#[test]
fn a_click_flips_one_row() {
    let rows = order(&["a", "b", "c"]);
    let mut selection = Selection::default();
    selection.click(&rows, "b", false);
    assert_eq!(picked(&selection), ["b"]);
    selection.click(&rows, "b", false);
    assert!(selection.is_empty());
}

#[test]
fn a_range_runs_from_the_last_row_pressed_either_way() {
    let rows = order(&["a", "b", "c", "d", "e"]);
    let mut selection = Selection::default();
    selection.click(&rows, "b", false);
    selection.click(&rows, "d", true);
    assert_eq!(picked(&selection), ["b", "c", "d"]);
    selection.click(&rows, "a", true);
    assert_eq!(picked(&selection), ["a", "b", "c", "d"]);
}

#[test]
fn a_range_without_a_row_pressed_before_flips_just_that_row() {
    let rows = order(&["a", "b", "c"]);
    let mut selection = Selection::default();
    selection.click(&rows, "c", true);
    assert_eq!(picked(&selection), ["c"]);
}

#[test]
fn a_range_from_a_row_the_filter_hides_flips_just_that_row() {
    let mut selection = Selection::default();
    selection.click(&order(&["a", "b", "c"]), "a", false);
    selection.click(&order(&["b", "c"]), "c", true);
    assert_eq!(picked(&selection), ["a", "c"]);
}

#[test]
fn a_key_that_is_not_shown_changes_nothing() {
    let mut selection = Selection::default();
    selection.click(&order(&["a"]), "z", false);
    assert!(selection.is_empty());
}

#[test]
fn select_all_takes_only_what_is_shown() {
    let mut selection = Selection::default();
    selection.click(&order(&["a", "b", "c"]), "c", false);
    // A search narrowed the list to a and b.
    let shown = order(&["a", "b"]);
    selection.select_all(&shown);
    assert_eq!(picked(&selection), ["a", "b"]);
    assert!(selection.all_of(&shown));
}

#[test]
fn the_toggle_selects_all_then_clears() {
    let rows = order(&["a", "b"]);
    let mut selection = Selection::default();
    selection.click(&rows, "a", false);
    selection.toggle_all(&rows);
    assert_eq!(selection.len(), 2);
    selection.toggle_all(&rows);
    assert!(selection.is_empty());
}

#[test]
fn nothing_shown_is_never_all_selected() {
    assert!(!Selection::default().all_of(&[]));
}

#[test]
fn retain_drops_rows_that_went_away() {
    let mut selection = Selection::default();
    selection.select_all(&order(&["a", "b", "c"]));
    selection.retain(&order(&["a", "c", "d"]));
    assert_eq!(picked(&selection), ["a", "c"]);
}

#[test]
fn clear_forgets_the_range_start() {
    let rows = order(&["a", "b", "c"]);
    let mut selection = Selection::default();
    selection.click(&rows, "a", false);
    selection.clear();
    selection.click(&rows, "c", true);
    assert_eq!(picked(&selection), ["c"]);
}

#[test]
fn a_removal_is_confirmed_only_for_the_selection_it_was_asked_for() {
    let rows = order(&["a", "b", "c"]);
    let mut selection = Selection::default();
    selection.select_all(&rows);
    assert!(!selection.confirm_removal());
    assert!(selection.removal_armed());
    // Another row joins: the question was about a different set.
    selection.clear();
    selection.select_all(&order(&["a", "b"]));
    assert!(!selection.removal_armed());
    assert!(!selection.confirm_removal());
    assert!(selection.confirm_removal());
    assert!(!selection.removal_armed());
}

#[test]
fn modifiers_make_gestures() {
    let ctrl = gpui::Modifiers {
        control: true,
        ..Default::default()
    };
    assert!(Gesture::of(&ctrl).toggle && Gesture::of(&ctrl).selects());
    let shift = gpui::Modifiers {
        shift: true,
        ..Default::default()
    };
    assert!(Gesture::of(&shift).range);
    assert!(!Gesture::of(&gpui::Modifiers::default()).selects());
}
