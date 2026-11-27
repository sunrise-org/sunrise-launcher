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

fn touch(path: &std::path::Path, secs: u64) {
    std::fs::write(path, "x").unwrap();
    let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(at)
        .unwrap();
}

#[test]
fn crash_reports_list_newest_first_and_skip_other_files() {
    let dir = tempfile::tempdir().unwrap();
    touch(
        &dir.path().join("crash-2026-01-01_10.00.00-client.txt"),
        100,
    );
    touch(
        &dir.path().join("crash-2026-01-02_10.00.00-client.txt"),
        300,
    );
    touch(
        &dir.path().join("crash-2026-01-01_12.00.00-client.txt"),
        200,
    );
    touch(&dir.path().join("notes.txt"), 400);
    touch(&dir.path().join("launch-failure-1-2.log"), 500);
    let names: Vec<String> = sources::list_files(dir.path(), Source::Crashes)
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        [
            "crash-2026-01-02_10.00.00-client.txt",
            "crash-2026-01-01_12.00.00-client.txt",
            "crash-2026-01-01_10.00.00-client.txt",
        ]
    );
    let launcher = sources::list_files(dir.path(), Source::Launcher);
    assert_eq!(launcher.len(), 1);
    assert!(sources::list_files(&dir.path().join("missing"), Source::Crashes).is_empty());
}

#[test]
fn the_picked_file_stays_until_it_is_gone() {
    let files: Vec<std::path::PathBuf> = vec!["b.txt".into(), "a.txt".into()];
    assert_eq!(sources::pick_index(&files, None), Some(0));
    assert_eq!(sources::pick_index(&files, Some("a.txt".as_ref())), Some(1));
    assert_eq!(
        sources::pick_index(&files, Some("gone.txt".as_ref())),
        Some(0)
    );
    assert_eq!(sources::pick_index(&[], Some("a.txt".as_ref())), None);
}

#[test]
fn each_source_reads_its_own_file() {
    let home = std::path::Path::new("/profile");
    let mut logs = Logs::default();
    assert_eq!(logs.target(home), Some(home.join("logs/latest.log")));
    logs.switch(Source::Crashes);
    assert_eq!(logs.target(home), None);
    logs.files = vec!["/profile/crash-reports/crash-2.txt".into()];
    assert_eq!(
        logs.target(home),
        Some("/profile/crash-reports/crash-2.txt".into())
    );
    assert_eq!(Source::Crashes.folder(home), home.join("crash-reports"));
    // Switching away forgets the list.
    logs.switch(Source::Latest);
    assert!(logs.files.is_empty());
    assert_eq!(logs.target(home), Some(home.join("logs/latest.log")));
}
