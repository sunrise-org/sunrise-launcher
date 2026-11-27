use super::super::tests::{scratch, world};
use super::*;

#[test]
fn the_seed_is_read_from_either_place_the_game_keeps_it() {
    let home = scratch("seed");
    let new = world(
        &home,
        "new",
        vec![(
            "WorldGenSettings",
            Tag::compound([("seed", Tag::Long(-4172144997902289642))]),
        )],
    );
    let old = world(&home, "old", vec![("RandomSeed", Tag::Long(42))]);
    let none = world(&home, "none", vec![]);
    assert_eq!(seed(&new), Some(-4172144997902289642));
    assert_eq!(seed(&old), Some(42));
    assert_eq!(seed(&none), None);
    assert_eq!(seed(&home.join("saves/missing")), None);
    let _ = fs::remove_dir_all(home);
}
