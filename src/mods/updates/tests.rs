use super::*;

fn found(id: &str, current: &str, update: Option<&str>) -> Found {
    Found {
        id: id.into(),
        name: id.to_uppercase(),
        current: current.into(),
        update: update.map(String::from),
        ..Found::default()
    }
}

#[test]
fn only_mods_with_a_newer_registry_release_are_listed_by_name() {
    let list = vec![
        found("rain", "1.0", Some("1.1")),
        found("lantern", "2.0", None),
        Found {
            local: true,
            ..found("trail", "1.0", Some("9.9"))
        },
        found("ash", "0.3", Some("0.4")),
    ];
    let offer = offer(list, &Skips::default());
    let ids: Vec<_> = offer.items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(ids, ["ash", "rain"]);
    assert_eq!(offer.hidden, 0);
    assert_eq!(
        (
            offer.items[1].current.as_str(),
            offer.items[1].next.as_str()
        ),
        ("1.0", "1.1")
    );
}

#[test]
fn a_skipped_version_is_left_out_but_the_next_one_is_offered() {
    let mut skips = Skips::default();
    skips.skip("rain", "1.1");
    let first = offer(vec![found("rain", "1.0", Some("1.1"))], &skips);
    assert!(first.items.is_empty());
    assert_eq!(first.hidden, 1);
    let later = offer(vec![found("rain", "1.0", Some("1.2"))], &skips);
    assert_eq!(later.items.len(), 1);
    assert_eq!(later.hidden, 0);
}

#[test]
fn a_chosen_or_taken_down_mod_is_listed_but_never_ticked() {
    let list = vec![
        Found {
            pinned: true,
            ..found("chosen", "1.0", Some("1.1"))
        },
        Found {
            taken_down: true,
            ..found("gone", "1.0", Some("1.1"))
        },
        found("plain", "1.0", Some("1.1")),
    ];
    let offer = offer(list, &Skips::default());
    assert_eq!(offer.items.len(), 3);
    let picks = default_picks(&offer.items);
    assert_eq!(picks, BTreeSet::from(["plain".to_owned()]));
}

#[test]
fn the_sheet_opens_with_everything_ticked_and_the_controls_move_the_ticks() {
    let mut check = Check::default();
    let generation = check.begin();
    assert!(check.accepts(generation));
    let list = vec![
        found("a", "1", Some("2")),
        found("b", "1", Some("2")),
        Found {
            pinned: true,
            ..found("c", "1", Some("2"))
        },
    ];
    check.show(offer(list, &Skips::default()), Instant::now());
    assert_eq!(check.picks.len(), 2);
    assert_eq!(check.tickable(), 2);
    check.toggle("a");
    assert!(!check.picks.contains("a"));
    check.toggle("c");
    assert!(!check.picks.contains("c"), "a blocked row can't be ticked");
    check.select_all();
    assert_eq!(check.picks.len(), 2);
    check.select_none();
    assert!(check.picks.is_empty());
    check.select_all();
    check.drop_item("b");
    assert_eq!(
        (check.items.len(), check.picks.len(), check.hidden),
        (2, 1, 1)
    );
}

#[test]
fn a_check_left_behind_is_not_taken() {
    let mut check = Check::default();
    let first = check.begin();
    let second = check.begin();
    assert!(!check.accepts(first));
    assert!(check.accepts(second));
}

#[test]
fn nothing_found_says_up_to_date_for_a_few_seconds() {
    let mut check = Check::default();
    check.begin();
    let now = Instant::now();
    check.show(Offer::default(), now);
    assert!(check.up_to_date(now));
    assert!(!check.up_to_date(now + UP_TO_DATE_FOR + Duration::from_millis(1)));
    assert!(check.items.is_empty());
}

#[test]
fn skips_are_kept_in_a_file_and_cleared() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!Skips::load(dir.path()).is_skipped("rain", "1.1"));
    let mut skips = Skips::default();
    skips.skip("rain", "1.1");
    skips.skip("ash", "0.4");
    skips.save(dir.path()).unwrap();
    let again = Skips::load(dir.path());
    assert!(again.is_skipped("rain", "1.1"));
    assert!(!again.is_skipped("rain", "1.2"));
    assert!(again.is_skipped("ash", "0.4"));
    let mut cleared = again;
    cleared.clear();
    cleared.save(dir.path()).unwrap();
    assert!(!Skips::load(dir.path()).is_skipped("rain", "1.1"));
}

#[test]
fn an_unreadable_skip_file_means_no_skips() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("skipped-updates.json"), b"{ not json").unwrap();
    assert!(!Skips::load(dir.path()).is_skipped("rain", "1.1"));
}

#[test]
fn sizes_read_in_the_largest_unit() {
    assert_eq!(size_label(900), "900 B");
    assert_eq!(size_label(2048), "2.0 KB");
    assert_eq!(size_label(5 * 1024 * 1024 + 512 * 1024), "5.5 MB");
}
