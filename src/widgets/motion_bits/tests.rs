use super::*;

#[test]
fn a_chevron_turns_only_when_its_menu_opens_or_closes() {
    let mut folds = HashMap::new();
    // First drawn, closed or open, it is where it belongs, without turning.
    assert_eq!(turns_after(&mut folds, "a".into(), false), 0);
    assert_eq!(turns_after(&mut folds, "b".into(), true), 0);
    // Drawn again the same: still no turn.
    assert_eq!(turns_after(&mut folds, "a".into(), false), 0);
    // Each opening and closing is one turn, and each chevron counts its own.
    assert_eq!(turns_after(&mut folds, "a".into(), true), 1);
    assert_eq!(turns_after(&mut folds, "a".into(), true), 1);
    assert_eq!(turns_after(&mut folds, "a".into(), false), 2);
    assert_eq!(turns_after(&mut folds, "b".into(), true), 0);
    assert_eq!(turns_after(&mut folds, "b".into(), false), 1);
}

#[test]
fn a_copy_is_counted_under_its_text() {
    assert_eq!(copy_key("copy", "one"), copy_key("copy", "one"));
    assert_ne!(copy_key("copy", "one"), copy_key("copy", "two"));
    assert_ne!(copy_key("copy", "one"), copy_key("other", "one"));
}
