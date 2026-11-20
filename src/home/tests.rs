use super::*;

#[test]
fn a_fact_starts_with_a_capital() {
    assert_eq!(sentence("yesterday"), "Yesterday");
    assert_eq!(sentence(""), "");
}

#[test]
fn a_profile_says_its_play_time_only_once_it_was_played() {
    assert_eq!(played_line(Duration::from_secs(600), None), None);
    let now = std::time::SystemTime::now();
    assert_eq!(
        played_line(Duration::from_secs(3 * 3600 + 300), Some(now)),
        Some(("3 h 05 min played".into(), "Last played today".into()))
    );
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

#[test]
fn the_launcher_update_notice_shows_only_when_there_is_something_to_do() {
    let release = Release {
        tag: "launcher-v1.1.3".into(),
        name: String::new(),
        body: String::new(),
        date: String::new(),
        time: String::new(),
        asset: None,
        prerelease: false,
        nightly: None,
    };
    let show = |hidden, state: &SelfUpdate, found: Option<&Release>, allowed| {
        launcher_notice(hidden, state, found, allowed)
            .map(|(version, get)| (version, get.is_some()))
    };
    // Downloaded and ready: Update applies it.
    assert_eq!(
        show(false, &SelfUpdate::Ready("1.1.3".into()), None, true),
        Some(("1.1.3".into(), false))
    );
    // Found by a check, not yet downloaded: Update starts the download.
    let found = show(false, &SelfUpdate::Idle, Some(&release), true).unwrap();
    assert!(found.1);
    // Nothing found, dismissed, a copy that can't replace itself, mid-download and failure.
    assert_eq!(show(false, &SelfUpdate::Idle, None, true), None);
    assert_eq!(
        show(true, &SelfUpdate::Ready("1.1.3".into()), None, true),
        None
    );
    assert_eq!(show(false, &SelfUpdate::Idle, Some(&release), false), None);
    assert_eq!(
        show(
            false,
            &SelfUpdate::Downloading("1.1.3".into(), 0.5),
            Some(&release),
            true
        ),
        None
    );
    assert_eq!(
        show(false, &SelfUpdate::Failed("x".into()), Some(&release), true),
        None
    );
}
