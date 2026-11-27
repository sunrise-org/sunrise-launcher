use super::tail::{load_tail, Tail};
use super::*;

#[test]
fn a_line_is_as_bad_as_its_level() {
    assert_eq!(severity("[12:00:01] [main/INFO]: Starting"), 0);
    assert_eq!(severity("[12:00:02] [main/WARN]: Slow tick"), 1);
    assert_eq!(
        severity("[12:00:03] [Render thread/ERROR]: Shader failed"),
        2
    );
    assert_eq!(severity("ERROR: no display"), 2);
}

#[test]
fn the_tail_is_bounded_and_tells_absence_from_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("latest.log");
    assert_eq!(load_tail(&path, None), Some((None, Tail::Absent)));

    let line = "x".repeat(99) + "\n";
    std::fs::write(&path, line.repeat(60_000)).unwrap(); // about 6 MB
    let (stamp, tail) = load_tail(&path, None).unwrap();
    let Tail::Lines(lines) = tail else {
        panic!("the log was read")
    };
    assert_eq!(lines.len(), crate::LOG_LINES);
    assert!(lines.iter().all(|l| l.len() == 99));
    assert_eq!(load_tail(&path, stamp), None);

    // A folder where the file should be is not "no log yet".
    let folder = dir.path().join("dir.log");
    std::fs::create_dir(&folder).unwrap();
    assert!(matches!(
        load_tail(&folder, None),
        Some((None, Tail::Failed(_)))
    ));
}
