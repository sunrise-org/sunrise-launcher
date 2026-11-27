use super::*;

#[test]
fn a_set_is_off_on_or_some_by_how_much_of_it_is_ticked() {
    assert_eq!(tick_of(0, 3), Tick::Off);
    assert_eq!(tick_of(1, 3), Tick::Some);
    assert_eq!(tick_of(2, 3), Tick::Some);
    assert_eq!(tick_of(3, 3), Tick::On);
    assert_eq!(
        tick_of(0, 0),
        Tick::Off,
        "an empty set is ticked by nothing"
    );
}
