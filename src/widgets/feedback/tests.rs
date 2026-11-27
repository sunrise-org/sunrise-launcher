use super::*;

#[test]
fn a_bar_never_drops_below_a_sliver_and_never_passes_full() {
    assert_eq!(clamp_fraction(0.), 0.02);
    assert_eq!(clamp_fraction(0.5), 0.5);
    assert_eq!(clamp_fraction(3.), 1.);
}

#[test]
fn an_empty_state_line_loses_its_full_stop_only() {
    assert_eq!(
        label_line("Nothing here yet.".into()).as_ref(),
        "Nothing here yet"
    );
    assert_eq!(label_line("Nothing here。".into()).as_ref(), "Nothing here");
    assert_eq!(label_line("Add one.  ".into()).as_ref(), "Add one.  ");
}
