use super::*;

#[test]
fn a_fact_starts_with_a_capital() {
    assert_eq!(sentence("yesterday"), "Yesterday");
    assert_eq!(sentence(""), "");
}

#[test]
fn a_long_list_builds_the_rows_near_the_view() {
    // 100 rows 56 apart, the page showing the list's first 400 points.
    assert_eq!(near(100, 1, 56., (0., 400.)), 0..11);
    // Scrolled 2000 points down: those in view, three before and a few after.
    assert_eq!(near(100, 1, 56., (2000., 2400.)), 32..46);
    // The list is far under the view, or far over it: nothing of it is built.
    assert_eq!(near(100, 1, 56., (-900., -500.)), 0..0);
    assert_eq!(near(100, 1, 56., (9000., 9400.)), 100..100);
    // Just under the view: its first rows are there for Tab to land on.
    assert_eq!(near(100, 1, 56., (-500., -100.)), 0..2);
    // A short list is whole.
    assert_eq!(near(6, 1, 56., (0., 400.)), 0..6);
    // The last row of a thousand still belongs to the active virtual range.
    assert_eq!(near(1_000, 1, ROW, (39_800., 40_000.)), 992..1_000);
    // Tiles go by whole bands of four.
    assert_eq!(near(30, 4, 196., (400., 800.)), 0..30);
    assert_eq!(near(300, 4, 196., (1960., 2360.)), 28..64);
}
