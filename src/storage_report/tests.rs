use super::*;

fn work(clearing: bool) -> Work {
    Work {
        cancel: storage::Cancel::default(),
        clearing,
    }
}

#[test]
fn clear_is_offered_whatever_is_known_of_sizes() {
    assert!(clear_offered(None, false), "nothing counted yet");
    assert!(
        clear_offered(Some(&work(false)), false),
        "a count is in flight"
    );
    assert!(
        !clear_offered(Some(&work(true)), false),
        "a Clear is already running"
    );
    assert!(
        !clear_offered(None, true),
        "configured data folder is unavailable"
    );
    assert!(
        !clear_offered(Some(&work(false)), true),
        "unavailable location must not offer Clear during a count"
    );
    assert!(
        !clear_offered(Some(&work(true)), true),
        "unavailable location never offers Clear"
    );
}

#[test]
fn the_question_asks_without_a_size_when_the_rows_are_not_counted() {
    for asked in [None, Some(Category::OtherCaches)] {
        let unsized_question = clear_question(asked, None);
        assert!(unsized_question.contains("Close the game first."));
        assert!(!unsized_question.contains("Up to"), "{unsized_question}");
        assert!(!unsized_question.contains("{size}"), "{unsized_question}");
        let sized = clear_question(asked, Some(2_000_000));
        assert!(sized.contains("Close the game first."));
        assert!(sized.contains("2.0 MB"), "{sized}");
    }
}

#[cfg(unix)]
#[test]
#[ignore = "spawns an isolated process with scratch data roots"]
fn storage_row_actions_and_advice_use_real_inventory_rows() {
    use std::{fs, os::unix::fs::symlink, path::PathBuf, process::Command};

    // Storage opens the process's data roots. Keep the test's roots in a fresh process.
    const CHILD: &str = "SUNRISE_TEST_STORAGE_ROW_CHILD";
    const DONE: &str = "storage-row-fixture-passed";
    let Some(root) = std::env::var_os(CHILD).map(PathBuf::from) else {
        let home = tempfile::tempdir().unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--include-ignored",
                "--exact",
                "storage_report::tests::storage_row_actions_and_advice_use_real_inventory_rows",
                "--nocapture",
            ])
            .env(CHILD, home.path())
            .env("SUNRISE_LAUNCHER_HOME", home.path().join("launcher"))
            .env(
                "SUNRISE_LAUNCHER_LOCATION_FILE",
                home.path().join("location.json"),
            )
            .env("SUNRISE_HOME", home.path().join("game"))
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success()
                && stdout.contains("test result: ok. 1 passed;")
                && stdout.contains(DONE),
            "storage row child did not pass exactly one test:\n{stdout}{stderr}"
        );
        return;
    };
    let launcher = root.join("launcher");
    let target = launcher.join("native/target/debug");
    let source = launcher.join("native/source/revision");
    let work = launcher.join("native/w");
    for dir in [&target, &source, &work] {
        fs::create_dir_all(dir).unwrap();
    }
    let canary = root.join("private-path-canary");
    fs::write(&canary, [1; 8192]).unwrap();
    fs::write(target.join("output"), [1; 8192]).unwrap();
    symlink(&canary, source.join("private-path-canary")).unwrap();
    symlink(&canary, work.join("link-only")).unwrap();

    let storage = storage::Storage::open().unwrap();
    let mut report = storage.report(None, false).unwrap();
    let links = report
        .rows
        .iter()
        .find(|r| r.category == Category::ModBuildCache)
        .unwrap();
    assert_eq!(links.measured_bytes(), Some(0));
    assert!(row_clearable(Some(links)));
    assert_eq!(kept_line(links), None);

    let refused = report
        .rows
        .iter_mut()
        .find(|r| r.category == Category::BuildFiles)
        .unwrap();
    assert_eq!(refused.eligibility(), Eligibility::WhollyRefused);
    assert!(!row_clearable(Some(refused)));
    assert!(kept_line(refused).is_some());
    assert!(refused
        .kept
        .iter()
        .any(|unit| unit.kind == KeptKind::Unsafe));
    assert!(refused
        .kept
        .iter()
        .all(|unit| !unit.reason.contains("private-path-canary")));

    // Check a post-Clear report with both older-build and other refusals, then only older builds.
    refused.after_clear = true;
    refused.kept[0].kind = KeptKind::OlderBuild;
    let size = storage::format_bytes(refused.remaining_bytes);
    assert_eq!(
        kept_line(refused).unwrap(),
        format!("{size} stayed. See the launcher log for details.")
    );
    refused.kept.truncate(1);
    assert!(kept_line(refused)
        .unwrap()
        .contains("Remove older game builds"));
    println!("{DONE}");
}

#[test]
fn queued_count_error_cannot_settle_a_clear_or_reopened_count() {
    let stale = storage::Cancel::default();
    let error = Err("queued count error".into());
    let mut view = settings::SettingsView::default();
    view.storage_clear = Some(Err("previous clear error".into()));
    for clearing in [true, false] {
        let active = work(clearing);
        let token = active.cancel.clone();
        view.storage_work = Some(active);
        assert_eq!(view.accept_storage(&stale, &error), None);
        assert!(view.storage_busy());
        assert!(matches!(&view.storage_clear, Some(Err(error)) if error == "previous clear error"));
        assert_eq!(view.accept_storage(&token, &error), Some(clearing));
        assert!(!view.storage_busy());
    }
}
