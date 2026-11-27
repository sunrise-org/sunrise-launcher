use super::*;

#[test]
fn arrow_keys_step_to_the_neighbour_and_wrap_round() {
    assert_eq!(next_choice(3, 0, "right"), Some(1));
    assert_eq!(next_choice(3, 0, "down"), Some(1));
    assert_eq!(next_choice(3, 2, "right"), Some(0));
    assert_eq!(next_choice(3, 0, "left"), Some(2));
    assert_eq!(next_choice(3, 0, "up"), Some(2));
    assert_eq!(next_choice(3, 2, "left"), Some(1));
}

#[test]
fn other_keys_and_an_empty_group_choose_nothing() {
    assert_eq!(next_choice(3, 1, "enter"), None);
    assert_eq!(next_choice(3, 1, "a"), None);
    assert_eq!(next_choice(0, 0, "right"), None);
}

#[test]
fn a_single_choice_stays_on_itself() {
    assert_eq!(next_choice(1, 0, "right"), Some(0));
    assert_eq!(next_choice(1, 0, "left"), Some(0));
}
