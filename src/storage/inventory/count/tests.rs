use super::*;
use crate::storage::tests::Home;
use crate::storage::{access_calls, entries_calls, walks};

#[test]
fn a_count_discovers_once_walks_each_unit_once_and_classifies_nothing() {
    let home = Home::new();
    home.seed("rust/old/payload");
    home.seed("native/source/rev/lib.rs");
    home.seed("native/target/debug/output");
    let storage = Storage::at(&home.0).unwrap();
    let found = storage.entries().unwrap().len();
    let before = (walks(), entries_calls(), access_calls());
    let mut handed_over = Vec::new();
    let report = storage
        .count(&Cancel::default(), |report| {
            handed_over.push(report.rows.len());
        })
        .unwrap()
        .unwrap();
    let after = storage.entries().unwrap().len();
    assert_eq!(entries_calls() - before.1, 2, "one for the count, one here");
    assert!(
        (found.min(after)..=found.max(after)).contains(&(walks() - before.0)),
        "each unit is walked once, not again for its notes"
    );
    assert_eq!(access_calls(), before.2, "counting never classifies a unit");
    assert_eq!(handed_over, (1..=Category::ALL.len()).collect::<Vec<_>>());
    assert!(report.is_complete());
    assert!(report.rows.iter().all(|row| row.freed_bytes == 0));
    assert!(report.total_bytes > 0);
}

#[test]
fn a_count_hands_over_each_row_as_it_finishes() {
    let home = Home::new();
    home.seed("rust/old/payload");
    let storage = Storage::at(&home.0).unwrap();
    let cancel = Cancel::default();
    let mut order = Vec::new();
    let report = storage
        .count(&cancel, |report| {
            order.push(report.rows.last().unwrap().category);
            assert!(report.is_from(&cancel));
            assert!(!report.is_from(&Cancel::default()));
        })
        .unwrap()
        .unwrap();
    assert_eq!(order, Category::ALL);
    assert_eq!(report.rows.len(), Category::ALL.len());
}

#[test]
fn cancelling_a_count_stops_it_before_the_next_row() {
    let home = Home::new();
    home.seed("native/source/rev/lib.rs");
    home.seed("rust/old/payload");
    let storage = Storage::at(&home.0).unwrap();
    let cancel = Cancel::default();
    let mut handed_over = 0;
    let result = storage
        .count(&cancel, |_| {
            handed_over += 1;
            cancel.cancel();
        })
        .unwrap();
    assert!(result.is_none(), "a cancelled count has no whole report");
    assert_eq!(handed_over, 1);
}

#[test]
fn a_count_cancelled_before_it_starts_discovers_and_walks_nothing() {
    let home = Home::new();
    home.seed("rust/old/payload");
    let storage = Storage::at(&home.0).unwrap();
    let cancel = Cancel::default();
    cancel.cancel();
    let before = (walks(), entries_calls());
    assert!(storage
        .count(&cancel, |_| panic!("no row"))
        .unwrap()
        .is_none());
    assert_eq!((walks(), entries_calls()), before);
}

#[test]
fn a_walk_cancelled_part_way_stops_descending() {
    let home = Home::new();
    home.seed("rust/a/b/c/payload");
    let storage = Storage::at(&home.0).unwrap();
    let cancel = Cancel::default();
    let stop = cancel.clone();
    STATTED.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |path| {
            assert!(!path.ends_with("payload"));
            if path.ends_with("rust/a/b") {
                stop.cancel();
            }
        }));
    });
    let result = storage.count(&cancel, |report| {
        assert!(report
            .rows
            .iter()
            .all(|row| row.category != Category::RustCompiler));
    });
    STATTED.with(|hook| hook.borrow_mut().take());
    assert!(cancel.is_cancelled());
    assert!(result.unwrap().is_none());
}

#[cfg(unix)]
#[test]
fn a_link_and_a_root_vanishing_after_stat_are_absent() {
    let home = Home::new();
    let path = home.seed("rust/kept");
    let folder = path.parent().unwrap();
    let link = folder.join("vanishing-link");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    let vanished = link.clone();
    STATTED.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |path| {
            if path == vanished {
                fs::remove_file(path).unwrap();
            }
        }));
    });
    let result = allocated_bytes(folder);
    STATTED.with(|hook| hook.borrow_mut().take());
    assert!(!link.exists());
    assert_eq!(result.unwrap(), allocated_bytes(folder).unwrap());
    let vanish = folder.to_owned();
    CANONICALIZING.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |path| {
            if path == vanish {
                fs::remove_dir_all(path).unwrap();
            }
        }));
    });
    let result = Root::open(folder);
    CANONICALIZING.with(|hook| hook.borrow_mut().take());
    assert!(result.unwrap().directory.is_none());
}

#[test]
fn clear_asks_for_no_count_and_survives_a_cancelled_one() {
    let home = Home::new();
    let payload = home.seed("rust/old/payload");
    let storage = Storage::at(&home.0).unwrap();
    let cancel = Cancel::default();
    cancel.cancel();
    assert!(storage.count(&cancel, |_| {}).unwrap().is_none());
    let report = storage.report(Some(Category::RustCompiler), false).unwrap();
    let row = report
        .rows
        .iter()
        .find(|row| row.category == Category::RustCompiler)
        .unwrap();
    assert_eq!(row.category, Category::RustCompiler);
    assert!(row.freed_bytes > 0, "Clear measures what it frees itself");
    assert!(!payload.exists());
}

#[test]
fn a_count_reports_a_refused_unit_on_its_row_without_holding_up_the_rest() {
    let home = Home::new();
    let kept = home.seed("rust/old/payload");
    home.seed("worlds/keep");
    #[cfg(unix)]
    {
        home.seed("native/source/rev/lib.rs");
        std::os::unix::fs::symlink(home.0.join("worlds"), home.0.join("native/source/rev/link"))
            .unwrap();
    }
    let storage = Storage::at(&home.0).unwrap();
    let report = storage.report(None, false).unwrap();
    assert!(
        report
            .rows
            .iter()
            .find(|row| row.category == Category::RustCompiler)
            .unwrap()
            .bytes
            > 0
    );
    #[cfg(unix)]
    assert_eq!(
        report
            .rows
            .iter()
            .find(|row| row.category == Category::BuildFiles)
            .unwrap()
            .kept
            .len(),
        1
    );
    assert!(kept.exists());
    assert!(home.0.join("worlds/keep").exists());
}

#[test]
fn explicit_clear_removes_retained_attempts_under_the_build_lease() {
    let home = Home::new();
    let old_worker = home.seed("native/.nb-old/compiler/w");
    let older_launchers = home.seed("native/.native-attempt-older/compiler/work");
    let storage = Storage::at(&home.0).unwrap();
    let lease = storage.build_lease().unwrap();
    let report = storage.report(None, false).unwrap();
    let built = report
        .rows
        .iter()
        .find(|row| row.category == Category::BuildFiles)
        .unwrap();
    let bytes = allocated_bytes(&home.0.join("native/.nb-old")).unwrap()
        + allocated_bytes(&home.0.join("native/.native-attempt-older")).unwrap();
    assert!(bytes > 0);
    assert_eq!(built.clearable_bytes(), bytes);
    assert!(built.kept.is_empty());
    assert!(matches!(
        storage.report(Some(Category::BuildFiles), false),
        Err(Error::Busy)
    ));
    assert!(old_worker.exists() && older_launchers.exists());
    drop(lease);
    let manual = storage.report(Some(Category::BuildFiles), false).unwrap();
    let built = manual
        .rows
        .iter()
        .find(|row| row.category == Category::BuildFiles)
        .unwrap();
    assert_eq!(built.freed_bytes, bytes);
    assert_eq!(built.remaining_bytes, 0);
    assert!(!home.0.join("native/.nb-old").exists());
    assert!(!home.0.join("native/.native-attempt-older").exists());
}

#[cfg(windows)]
#[test]
fn clear_retries_a_retained_attempt_after_its_locked_file_closes() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
    let home = Home::new();
    let path = home.seed("native/.nb-old/locked");
    let storage = Storage::at(&home.0).unwrap();
    let kept_open = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&path)
        .unwrap();
    let report = storage.report(Some(Category::BuildFiles), false).unwrap();
    let row = report
        .rows
        .iter()
        .find(|row| row.category == Category::BuildFiles)
        .unwrap();
    assert_eq!(row.kept.len(), 1);
    assert!(row.remaining_bytes > 0);
    assert!(path.is_file());
    drop(kept_open);
    storage.report(Some(Category::BuildFiles), false).unwrap();
    assert!(!path.exists());
}

#[cfg(windows)]
#[test]
fn clear_keeps_an_attempt_that_denies_read_sharing_and_removes_other_attempts() {
    use std::os::windows::fs::OpenOptionsExt;
    let home = Home::new();
    let locked = home.seed("native/.nb-locked/output");
    let free = home.seed("native/.nb-free/output");
    let handle = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&locked)
        .unwrap();
    let storage = Storage::at(&home.0).unwrap();
    let report = storage.report(Some(Category::BuildFiles), false).unwrap();
    let row = report
        .rows
        .iter()
        .find(|row| row.category == Category::BuildFiles)
        .unwrap();
    assert_eq!(row.kept.len(), 1);
    assert!(row.freed_bytes > 0);
    assert!(!free.exists());
    drop(handle);
    assert!(locked.exists());
}

#[test]
fn mod_build_cache_counts_slots_and_detached_trash_then_clears_under_the_build_lease() {
    let home = Home::new();
    let slot = home.seed("native/w/slot/target/out");
    let detached = home.seed("native/w/.trash-slot/target/out");
    let other = home.seed("native/.nb-old/out");
    let storage = Storage::at(&home.0).unwrap();
    let work = home.0.join(BuildCache::ModBuildCache.relative());
    let expected = allocated_bytes(&work.join("slot")).unwrap()
        + allocated_bytes(&work.join(".trash-slot")).unwrap();
    assert!(expected > 0);
    let lease = storage.build_lease().unwrap();
    let report = storage.report(None, false).unwrap();
    let row = report
        .rows
        .iter()
        .find(|row| row.category == Category::ModBuildCache)
        .unwrap();
    assert_eq!(row.clearable_bytes(), expected);
    assert!(matches!(
        storage.report(Some(Category::ModBuildCache), false),
        Err(Error::Busy)
    ));
    assert!(slot.exists() && detached.exists());
    drop(lease);
    let report = storage
        .report(Some(Category::ModBuildCache), false)
        .unwrap();
    let row = report
        .rows
        .iter()
        .find(|row| row.category == Category::ModBuildCache)
        .unwrap();
    assert_eq!(row.freed_bytes, expected);
    assert_eq!(row.remaining_bytes, 0);
    assert!(!work.join("slot").exists());
    assert!(!work.join(".trash-slot").exists());
    assert!(other.exists());
    assert!(home.0.join("native/.build.lock").exists());
}

/// #649: without the build lease, a folder can be detached between its parent's listing and
/// its own, which skips it instead of failing the report.
#[test]
fn a_cross_target_directory_that_vanishes_does_not_fail_a_read_only_report() {
    let home = Home::new();
    let triple = home.0.join("native/target/x86_64-unknown-test");
    home.seed("native/target/x86_64-unknown-test/debug/out");
    home.seed("native/target/debug/kept");
    let storage = Storage::at(&home.0).unwrap();
    let vanishing = triple.clone();
    LISTED.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |folder| {
            if folder.ends_with("native/target") {
                let _ = fs::remove_dir_all(&vanishing);
            }
        }));
    });
    let report = storage.report(None, false);
    LISTED.with(|hook| hook.borrow_mut().take());
    let report = report.unwrap();
    assert!(!triple.exists());
    assert!(
        report
            .rows
            .iter()
            .find(|row| row.category == Category::BuildFiles)
            .unwrap()
            .bytes
            > 0,
        "the unit that stayed is counted"
    );
    assert!(children(&home.0.join("gone")).unwrap().is_empty());
}
