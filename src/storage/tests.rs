use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) struct Home(pub(super) PathBuf);
impl Home {
    pub(super) fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sunrise-storage-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        // Canonical, as Storage::at stores it: Windows temp paths are 8.3 short names.
        Self(crate::backend::plain(path.canonicalize().unwrap()))
    }
    pub(super) fn seed(&self, path: &str) -> PathBuf {
        let path = path
            .split('/')
            .filter(|part| !part.is_empty())
            .fold(self.0.clone(), |path, part| path.join(part));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, vec![7; 8192]).unwrap();
        path
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn build_tools_remain_clearable_without_claiming_game_absence() {
    let home = Home::new();
    let storage = Storage::at(&home.0).unwrap();
    for (cache, directory) in [
        (BuildCache::KeptMods, "native/kept-mods"),
        (BuildCache::Rust, "rust"),
        (BuildCache::Cargo, "cargo"),
        (BuildCache::Scratch, "tmp"),
    ] {
        let output = home.seed(&format!("{directory}/output"));
        assert!(storage.clear(cache).unwrap() > 0);
        assert!(!output.exists());
    }
}

#[test]
fn kept_mod_libraries_are_build_files_that_report_and_clear() {
    let home = Home::new();
    let blob = home.seed("native/kept-mods/blobs/0123");
    let index = home.seed("native/kept-mods/0123.json");
    let storage = Storage::at(&home.0).unwrap();
    assert_eq!(BuildCache::KeptMods.category(), Category::BuildFiles);
    assert!(storage.bytes(BuildCache::KeptMods).unwrap() > 0);
    let report = storage.report(None, false).unwrap();
    let row = report
        .rows
        .iter()
        .find(|row| row.category == Category::BuildFiles)
        .unwrap();
    assert!(row.bytes > 0, "Storage reports the kept libraries");
    assert!(storage.clear(BuildCache::KeptMods).unwrap() > 0);
    assert!(!blob.exists() && !index.exists());
}

#[test]
fn hard_links_count_once() {
    let home = Home::new();
    let file = home.seed("rust/a");
    let before = allocated_bytes(&home.0.join("rust")).unwrap();
    fs::hard_link(file, home.0.join("rust/b")).unwrap();
    assert_eq!(allocated_bytes(&home.0.join("rust")).unwrap(), before);
}

#[test]
fn clears_only_fixed_cache_and_clears_detached_tree() {
    let home = Home::new();
    home.seed("native/source/revision/lib.rs");
    home.seed("settings.json");
    home.seed("worlds/keep/level.dat");
    let storage = Storage::at(&home.0).unwrap();
    assert!(storage.bytes(BuildCache::Source).unwrap() > 0);
    assert!(
        storage
            .trash(
                &storage.path(BuildCache::Source).unwrap(),
                Access::BuildTools
            )
            .unwrap()
            > 0
    );
    assert!(!home.0.join("native/source/revision/lib.rs").exists());
    assert!(home.0.join("settings.json").exists());
    assert!(home.0.join("worlds/keep/level.dat").exists());
    home.seed(".trash-interrupted/file");
    Storage::clear_entry(
        &storage.directory,
        &storage.directory,
        Path::new(".trash-interrupted"),
    )
    .unwrap();
    assert!(!home.0.join(".trash-interrupted/file").exists());
}

#[test]
fn read_only_report_does_not_take_build_lease_while_manual_clear_does() {
    let home = Home::new();
    let output = home.seed("rust/old/payload");
    let storage = Storage::at(&home.0).unwrap();
    let held = Cache::acquire(&home.0).unwrap();
    let report = storage.report(None, false).unwrap();
    let row = report
        .rows
        .iter()
        .find(|row| row.category == Category::RustCompiler)
        .unwrap();
    assert!(row.bytes > 0);
    assert!(matches!(storage.clear(BuildCache::Rust), Err(Error::Busy)));
    assert!(matches!(
        storage.report(Some(Category::RustCompiler), false),
        Err(Error::Busy)
    ));
    assert!(matches!(storage.report(None, true), Err(Error::Busy)));
    assert!(output.exists());
    drop(held);
    let cleared = storage.report(Some(Category::RustCompiler), false).unwrap();
    let row = cleared
        .rows
        .iter()
        .find(|row| row.category == Category::RustCompiler)
        .unwrap();
    assert!(row.freed_bytes > 0);
    assert!(!output.exists());
}

#[cfg(unix)]
#[test]
fn refuses_linked_ancestor_and_descendant_before_deletion() {
    use std::os::unix::fs::symlink;
    let home = Home::new();
    home.seed("worlds/keep");
    fs::create_dir_all(home.0.join("native")).unwrap();
    let storage = Storage::at(&home.0).unwrap();
    symlink(home.0.join("worlds"), home.0.join("native/source")).unwrap();
    assert!(storage.clear(BuildCache::Source).is_err());
    fs::remove_file(home.0.join("native/source")).unwrap();
    home.seed("native/source/good");
    symlink(home.0.join("worlds"), home.0.join("native/source/bad")).unwrap();
    assert!(storage.clear(BuildCache::Source).is_err());
    assert!(home.0.join("native/source/good").exists());
    assert!(home.0.join("worlds/keep").exists());
    fs::rename(home.0.join("native"), home.0.join("moved")).unwrap();
    symlink(home.0.join("moved"), home.0.join("native")).unwrap();
    assert!(storage.clear(BuildCache::Source).is_err());
}

/// The game's prepare step fills an instance's mod-content cache with links into the installed
/// build. They are the cache's normal shape: nothing to count, removed as links, targets kept.
#[cfg(unix)]
#[test]
fn links_into_an_installed_build_count_as_nothing_and_are_removed_as_links() {
    use std::os::unix::fs::symlink;
    let home = Home::new();
    let build = crate::backend::data_dir()
        .join("versions")
        .join(home.0.file_name().unwrap());
    fs::create_dir_all(build.join("prepared/assets")).unwrap();
    fs::write(build.join("prepared/asset-index.json"), vec![7; 1 << 20]).unwrap();
    fs::write(build.join("prepared/assets/texture"), b"texture").unwrap();
    home.seed("cache/mod-content/e677/own");
    let cache = home.0.join("cache");
    let before = allocated_bytes(&cache).unwrap();
    let entry = cache.join("mod-content/e677");
    symlink(
        build.join("prepared/asset-index.json"),
        entry.join("asset-index.json"),
    )
    .unwrap();
    symlink(build.join("prepared/assets"), entry.join("assets")).unwrap();
    assert!(allocated_bytes(&cache).unwrap() < before + (1 << 20));
    let storage = Storage::at(&home.0).unwrap();
    Storage::clear_entry(&storage.directory, &storage.directory, Path::new("cache")).unwrap();
    for name in ["own", "asset-index.json", "assets"] {
        assert!(fs::symlink_metadata(entry.join(name)).is_err(), "{name}");
    }
    assert!(build.join("prepared/asset-index.json").exists());
    assert!(build.join("prepared/assets/texture").exists());
    // A link that only passes through the build, or leaves the launcher's builds, is refused.
    let kept = home.seed("cache/kept");
    home.seed("worlds/keep");
    for target in [
        home.0.join("worlds"),
        build.join("../../../worlds"),
        PathBuf::from("prepared"),
    ] {
        symlink(&target, cache.join("bad")).unwrap();
        assert!(matches!(allocated_bytes(&cache), Err(Error::Unsafe(_))));
        assert!(matches!(
            Storage::clear_entry(&storage.directory, &storage.directory, Path::new("cache")),
            Err(Error::Unsafe(_))
        ));
        assert!(kept.exists());
        fs::remove_file(cache.join("bad")).unwrap();
    }
    assert!(home.0.join("worlds/keep").exists());
    fs::remove_dir_all(build).unwrap();
}

#[test]
fn read_only_file_can_be_removed() {
    let home = Home::new();
    let file = home.seed("native/tools/tool");
    let mut permissions = fs::metadata(&file).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(file, permissions).unwrap();
    let storage = Storage::at(&home.0).unwrap();
    storage
        .trash(
            &storage.path(BuildCache::Tools).unwrap(),
            Access::BuildTools,
        )
        .unwrap();
    assert!(!home.0.join("native/tools/tool").exists());
}

#[cfg(windows)]
#[test]
fn held_open_file_is_kept_until_released() {
    use std::os::windows::fs::OpenOptionsExt;
    let home = Home::new();
    let file = home.seed("native/source/file");
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(file)
        .unwrap();
    let storage = Storage::at(&home.0).unwrap();
    assert!(storage.clear(BuildCache::Source).is_err());
    drop(held);
    storage
        .trash(
            &storage.path(BuildCache::Source).unwrap(),
            Access::BuildTools,
        )
        .unwrap();
}

#[cfg(unix)]
#[test]
fn root_replacement_cannot_redirect_clear() {
    let home = Home::new();
    let outside = Home::new();
    outside.seed(".trash-precious/world");
    let storage = Storage::at(&home.0).unwrap();
    let moved = home.0.with_extension("moved");
    fs::rename(&home.0, &moved).unwrap();
    std::os::unix::fs::symlink(&outside.0, &home.0).unwrap();
    assert!(storage.clear(BuildCache::Source).is_err());
    assert!(outside.0.join(".trash-precious/world").exists());
    fs::remove_file(&home.0).unwrap();
    fs::rename(moved, &home.0).unwrap();
}

#[test]
fn child_cargo_locks_prevent_clear_and_survive_idle_cleanup() {
    for (cache, expected) in [
        (BuildCache::Source, "native/source"),
        (BuildCache::Tools, "native/tools"),
        (BuildCache::PrebuiltOutput, "native/prebuilt-build"),
        (BuildCache::InternalsSync, "native/internals-sync"),
        (BuildCache::Prebuilt, "native/prebuilt"),
        (BuildCache::ServerBuilds, "native/server-builds"),
        (BuildCache::KeptMods, "native/kept-mods"),
        (BuildCache::Rust, "rust"),
        (BuildCache::Cargo, "cargo"),
        (BuildCache::Scratch, "tmp"),
    ] {
        for depth in ["", "target", "rev/target/debug"] {
            let directory = format!("{expected}/{depth}");
            let home = Home::new();
            let lock_path = home.seed(&format!("{directory}/.cargo-lock"));
            let output = home.seed(&format!("{directory}/output"));
            let held = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&lock_path)
                .unwrap();
            held.lock().unwrap();
            let world = home.seed("worlds/precious/level.dat");
            let storage = Storage::at(&home.0).unwrap();
            assert!(matches!(
                storage.trash(&storage.path(cache).unwrap(), Access::BuildTools),
                Err(Error::Busy)
            ));
            assert!(output.exists());
            held.unlock().unwrap();
            if matches!(
                cache.access(&home.0.join(cache.relative())).unwrap(),
                Access::UninstrumentedGame
            ) {
                assert!(matches!(
                    storage.clear(cache),
                    Err(Error::UnprovenExclusion)
                ));
                assert!(output.exists());
            }
            storage
                .trash(&storage.path(cache).unwrap(), Access::BuildTools)
                .unwrap();
            assert!(!output.exists());
            assert!(lock_path.exists());
            assert!(world.exists());
        }
    }
}

#[cfg(windows)]
#[test]
fn held_root_cannot_be_renamed_away_on_windows() {
    let home = Home::new();
    let precious = home.seed("native/source/world");
    let _storage = Storage::at(&home.0).unwrap();
    assert!(fs::rename(&home.0, home.0.with_extension("held")).is_err());
    assert!(precious.exists());
}

#[cfg(unix)]
#[test]
fn same_path_with_different_root_identity_is_refused() {
    let home = Home::new();
    let storage = Storage::at(&home.0).unwrap();
    let moved = home.0.with_extension("held");
    fs::rename(&home.0, &moved).unwrap();
    let precious = home.seed("native/source/world");
    home.seed(".trash-precious/world");
    assert!(storage.clear(BuildCache::Source).is_err());
    assert!(precious.exists());
    assert!(home.0.join(".trash-precious/world").exists());
    fs::remove_dir_all(&home.0).unwrap();
    fs::rename(moved, &home.0).unwrap();
}

#[test]
fn freed_bytes_exclude_the_retained_lock_skeleton() {
    let home = Home::new();
    home.seed("native/source/rev/output");
    home.seed("native/source/rev/.cargo-lock");
    let storage = Storage::at(&home.0).unwrap();
    let before = storage.bytes(BuildCache::Source).unwrap();
    let freed = storage
        .trash(
            &storage.path(BuildCache::Source).unwrap(),
            Access::BuildTools,
        )
        .unwrap();
    let after = storage.bytes(BuildCache::Source).unwrap();
    assert!(after > 0);
    assert_eq!(freed, before.saturating_sub(after));
    assert!(freed < before);
}

/// Use checkout storage rather than /tmp (which may report zero directory blocks).
#[cfg(unix)]
#[test]
fn allocated_total_includes_directory_metadata() {
    use std::os::unix::fs::MetadataExt;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/storage-allocation-tests");
    fs::create_dir_all(&root).unwrap();
    let path = root.join(format!("directory-{}", std::process::id()));
    fs::create_dir(&path).unwrap();
    // Home removes the fixture on drop, including when an assertion unwinds.
    let home = Home(path);
    for index in 0..300 {
        fs::write(home.0.join(format!("{index:04}-{}", "x".repeat(180))), []).unwrap();
    }
    let directory_bytes = fs::metadata(&home.0).unwrap().blocks() * 512;
    let entries_bytes: u64 = fs::read_dir(&home.0)
        .unwrap()
        .map(|entry| {
            fs::symlink_metadata(entry.unwrap().path())
                .unwrap()
                .blocks()
                * 512
        })
        .sum();
    assert_eq!(
        allocated_bytes(&home.0).unwrap(),
        directory_bytes + entries_bytes
    );
    eprintln!("independent directory allocation: {directory_bytes} bytes");
}

#[cfg(any(unix, windows))]
#[test]
fn clear_mod_build_slots_unlinks_directory_links_without_touching_their_target() {
    let home = Home::new();
    let outside = Home::new();
    let canary = outside.seed("sentinel/canary");
    home.seed("native/w/slot/output");
    let work = home.0.join("native").join("w");
    let bytes = allocated_bytes(&work.join("slot")).unwrap();
    let sentinel = outside.0.join("sentinel");
    for link in [work.join("slot").join("link"), work.join("linked-slot")] {
        #[cfg(unix)]
        std::os::unix::fs::symlink(&sentinel, &link).unwrap();
        #[cfg(windows)]
        assert!(std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(crate::backend::plain(link))
            .arg(crate::backend::plain(sentinel.clone()))
            .status()
            .unwrap()
            .success());
    }
    let storage = Storage::at(&home.0).unwrap();
    let report = storage.report(None, false).unwrap();
    let row = report
        .rows
        .iter()
        .find(|row| row.category == Category::ModBuildCache)
        .unwrap();
    assert_eq!(row.bytes, bytes);
    assert!(bytes > 0);
    let report = storage
        .report(Some(Category::ModBuildCache), false)
        .unwrap();
    let row = report
        .rows
        .iter()
        .find(|row| row.category == Category::ModBuildCache)
        .unwrap();
    assert_eq!(row.remaining_bytes, 0);
    assert_eq!(row.freed_bytes, bytes);
    assert!(!work.join("slot").exists());
    assert!(fs::symlink_metadata(work.join("linked-slot")).is_err());
    assert_eq!(fs::read(canary).unwrap(), vec![7; 8192]);
}

#[cfg(unix)]
#[test]
fn rows_distinguish_zero_byte_candidates_from_wholly_refused_and_mixed_reasons() {
    let home = Home::new();
    let canary = home.seed("outside/player-data");
    fs::create_dir_all(home.0.join("native/w")).unwrap();
    let slot = home.0.join("native/w/only-link");
    std::os::unix::fs::symlink(&canary, &slot).unwrap();
    let broken = home.seed("rust/broken/output");
    std::os::unix::fs::symlink(&canary, broken.with_file_name("unexpected-link")).unwrap();
    let storage = Storage::at(&home.0).unwrap();
    let count = storage.report(None, false).unwrap();
    let row = |category| {
        count
            .rows
            .iter()
            .find(|row| row.category == category)
            .unwrap()
    };
    let link_only = row(Category::ModBuildCache);
    assert_eq!(link_only.measured_bytes(), Some(0));
    assert_eq!(link_only.eligibility(), Eligibility::Candidate);
    let refused = row(Category::RustCompiler);
    assert_eq!(refused.measured_bytes(), Some(0));
    assert_eq!(refused.eligibility(), Eligibility::WhollyRefused);
    assert!(refused.only_kept_kind(KeptKind::Unsafe));

    let lockpath = home.seed("cargo/.package-cache");
    let lock = fs::File::options()
        .read(true)
        .write(true)
        .open(lockpath)
        .unwrap();
    lock.lock().unwrap();
    let cleared = storage.report(Some(Category::RustCompiler), false).unwrap();
    let mixed = cleared
        .rows
        .iter()
        .find(|row| row.category == Category::RustCompiler)
        .unwrap();
    assert!(mixed.kept.iter().any(|unit| unit.kind == KeptKind::Unsafe));
    assert!(mixed.kept.iter().any(|unit| unit.kind == KeptKind::Busy));
    assert!(!mixed.only_kept_kind(KeptKind::Unsafe));
    assert!(!mixed.older_build_kept);
    let unselected = cleared
        .rows
        .iter()
        .find(|row| row.category == Category::ModBuildCache)
        .unwrap();
    assert_eq!(unselected.measured_bytes(), None);
    assert_eq!(unselected.eligibility(), Eligibility::NotMeasured);
    storage
        .report(Some(Category::ModBuildCache), false)
        .unwrap();
    assert!(fs::symlink_metadata(slot).is_err());
    assert!(canary.is_file());
}

#[test]
fn storage_io_error_message_stays_plain_while_log_detail_keeps_kind_and_code() {
    let coded = Error::Io(std::io::Error::from_raw_os_error(13));
    let detail = error_log_detail(&coded).expect("I/O detail");
    assert!(detail.contains("(os error 13)"), "{detail}");
    assert!(
        detail.contains(&std::io::Error::from_raw_os_error(13).kind().to_string()),
        "{detail}"
    );
    let plain = error_message(coded);
    assert_eq!(
        plain,
        "Sunrise couldn't access a file or folder. Close any programs using it and try again."
    );
    assert!(!plain.contains("os error"));

    let kind_only = Error::Io(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
    assert_eq!(
        error_log_detail(&kind_only).as_deref(),
        Some("permission denied")
    );
    assert_eq!(error_log_detail(&Error::Busy), None);
}
