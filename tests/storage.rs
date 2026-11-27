//! Real CLI regressions use separate launcher/game/config roots and never touch player data.
use std::{fs, path::PathBuf, process::Command};

struct Home(tempfile::TempDir);
impl Home {
    fn new() -> Self {
        Self(tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap())
    }
    fn file(&self, relative: &str, contents: &[u8]) -> PathBuf {
        let path = self.0.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }
    fn run(&self, args: &[&str]) -> std::process::Output {
        self.run_in("game", args)
    }
    fn run_in(&self, game: &str, args: &[&str]) -> std::process::Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sunrise-launcher"));
        command.args(args);
        command
            .current_dir(self.0.path())
            .env("SUNRISE_HOME", self.0.path().join(game))
            .env("SUNRISE_LAUNCHER_HOME", self.0.path().join("launcher"))
            .env("XDG_DATA_HOME", self.0.path().join("data"))
            .env("XDG_CONFIG_HOME", self.0.path().join("config"))
            .env("SUNRISE_LAUNCHER_TRASH", self.0.path().join("trash"))
            .env(
                "SUNRISE_LAUNCHER_CLIPBOARD",
                self.0.path().join("clipboard"),
            )
            .env("SUNRISE_LAUNCHER_DESKTOP", self.0.path().join("desktop"))
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("SUNRISE_NO_AUDIO", "1");
        #[cfg(target_os = "linux")]
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(|| {
                let limit = libc::rlimit {
                    rlim_cur: 256,
                    rlim_max: 256,
                };
                if libc::setrlimit(libc::RLIMIT_NOFILE, &limit) == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            });
        }
        command.output().unwrap()
    }
    fn clear(&self, row: &str) -> serde_json::Value {
        let output = self.run(&["storage", "--clear", row]);
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

#[test]
fn storage_clears_retained_compiler_attempts_on_explicit_clear() {
    let home = Home::new();
    home.file("launcher/native/.nb-old/compiler/w", &[9; 16384]);
    home.file(
        "launcher/native/.native-attempt-old/compiler/work",
        &[9; 16384],
    );
    let report = home.clear("build-files");
    let row = report["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["category"] == "build-files")
        .unwrap();
    assert!(row["kept"].as_array().unwrap().is_empty());
    assert!(row["freed_bytes"].as_u64().unwrap() >= 32768);
    assert_eq!(row["remaining_bytes"].as_u64().unwrap(), 0);
    assert!(!home.0.path().join("launcher/native/.nb-old").exists());
    assert!(!home
        .0
        .path()
        .join("launcher/native/.native-attempt-old")
        .exists());
}

#[test]
fn build_files_clear_without_touching_player_data() {
    let home = Home::new();
    let world = home.file("launcher/worlds/precious/level.dat", b"world");
    let game_world = home.file("game/worlds/precious/level.dat", b"world");
    let output = home.file("launcher/native/target/fast/output", &[7; 8192]);
    let source = home.file("launcher/native/source/rev/lib.rs", b"source");
    let prebuilt = home.file("launcher/native/prebuilt/rev/game", b"prebuilt");
    let clear = home.run(&["storage", "--clear", "build-files"]);
    assert_eq!(clear.status.code(), Some(1), "{clear:?}");
    home.clear("prebuilt-games");
    assert!(!prebuilt.exists());
    assert!(output.exists(), "incomplete native target was not kept");
    assert!(!source.exists());
    assert_eq!(fs::read(world).unwrap(), b"world");
    assert_eq!(fs::read(game_world).unwrap(), b"world");
}

#[cfg(unix)]
#[test]
fn running_unplayed_version_is_excluded_even_after_its_process_exits() {
    use std::process::Stdio;
    let home = Home::new();
    home.file("launcher/versions/v0.0.1/.installed", b"");
    home.file("launcher/settings.json", br#"{"selected":"v0.0.1"}"#);
    home.file("launcher/versions/native-played/.installed", b"");
    let played = home.file(
        "launcher/versions/native-played/game",
        b"instance-pinned native game",
    );
    home.file(
        "launcher/instances.json",
        br#"{"instances":{"sunrise":{"version":"native-played"}}}"#,
    );
    home.file("launcher/versions/native-live/.installed", b"");
    let game = home.file("launcher/versions/native-live/game", b"running");
    // ps sees the version path while read waits on the retained pipe. No game data is needed.
    let mut process = Command::new("sh")
        .args(["-c", "read -r ignored", "storage-fixture"])
        .arg(&game)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    home.clear("spare-game-builds");
    assert!(game.exists());
    assert!(
        played.exists(),
        "instance-pinned native version was selected"
    );
    process.kill().unwrap();
    process.wait().unwrap();
    home.clear("spare-game-builds");
    assert!(!game.exists());
    home.clear("all");
    assert!(
        played.exists(),
        "Clear all selected an instance-pinned native version"
    );
}

impl Home {
    fn installed_version(&self) {
        self.file("launcher/versions/v0.0.1/.installed", b"");
        self.file("launcher/settings.json", br#"{"selected":"v0.0.1"}"#);
    }
}

#[cfg(unix)]
#[test]
fn instance_caches_are_preserved_while_running_and_cleared_when_idle() {
    use std::process::Stdio;
    let home = Home::new();
    home.installed_version();
    let game = home.file("launcher/versions/v0.0.1/game", b"running");
    let caches = [
        home.file(
            "game/.sunrise-cache/behaviours-v8.bin",
            b"active instance cache",
        ),
        home.file(
            "game/.sunrise-cache/behaviours-v7.bin",
            b"active instance cache",
        ),
    ];
    let mut process = Command::new("sh")
        .args(["-c", "read -r ignored", "storage-instance"])
        .arg(game)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    home.clear("other-caches");
    for cache in &caches {
        assert_eq!(
            fs::read(cache).unwrap(),
            b"active instance cache",
            "running instance cache was selected"
        );
    }
    process.kill().unwrap();
    process.wait().unwrap();
    home.clear("other-caches");
    for cache in caches {
        assert!(!cache.exists());
    }
}

#[test]
fn unresolved_instance_caches_are_never_selected() {
    let home = Home::new();
    home.file(
        "launcher/settings.json",
        br#"{"selected":"missing-version"}"#,
    );
    let cache = home.file(
        "game/.sunrise-cache/behaviours-v8.bin",
        b"unresolved instance cache",
    );
    home.clear("other-caches");
    assert_eq!(
        fs::read(cache).unwrap(),
        b"unresolved instance cache",
        "an unresolved instance cannot prove it is idle"
    );
}

#[test]
fn report_cleanup_removes_only_old_non_latest_reports() {
    use std::time::{Duration, SystemTime};
    let home = Home::new();
    home.installed_version();
    for (name, half_days) in [
        ("logs/old.log", 29),
        ("logs/recent.log", 27),
        ("logs/latest.log", 0),
        ("crash-reports/old.txt", 40),
        ("crash-reports/newest.txt", 32),
    ] {
        let path = home.file(&format!("game/{name}"), b"report");
        let modified = SystemTime::now() - Duration::from_secs(half_days * 43200);
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
    }
    // Inventory still selects only the two old, non-newest reports.
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        let output = home.run(&["storage"]);
        assert!(output.status.success(), "{output:?}");
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let row = report["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["category"] == "other-caches")
            .unwrap();
        let selected_bytes: u64 = ["logs/old.log", "crash-reports/old.txt"]
            .iter()
            .map(|name| {
                fs::metadata(home.0.path().join("game").join(name))
                    .unwrap()
                    .blocks()
                    * 512
            })
            .sum();
        assert!(selected_bytes > 0);
        assert_eq!(row["bytes"], selected_bytes);
    }
    home.clear("other-caches");
    assert!(!home.0.path().join("game/logs/old.log").exists());
    assert!(
        home.0.path().join("game/logs/recent.log").exists(),
        "13.5-day log was cleared"
    );
    assert!(
        home.0.path().join("game/logs/latest.log").exists(),
        "latest log was cleared"
    );
    assert!(!home.0.path().join("game/crash-reports/old.txt").exists());
    assert!(
        home.0.path().join("game/crash-reports/newest.txt").exists(),
        "newest crash report must survive even after 14 days"
    );
}

#[test]
fn all_preserves_unknown_caches_worlds_and_mod_packages() {
    let home = Home::new();
    home.installed_version();
    let cache = home.file("game/.sunrise-cache/output", b"cache");
    let world = home.file("game/worlds/precious/level.dat", b"world");
    let package = home.file("game/sunrise-mods/precious.smod", b"mod package");
    home.clear("all");
    assert_eq!(fs::read(cache).unwrap(), b"cache");
    assert_eq!(
        fs::read(world).unwrap(),
        b"world",
        "worlds selected as instance cache"
    );
    assert_eq!(
        fs::read(package).unwrap(),
        b"mod package",
        "installed packages selected as instance cache"
    );
}

#[cfg(unix)]
#[test]
fn symlink_named_instance_is_refused_without_touching_its_outside_target() {
    let home = Home::new();
    home.installed_version();
    let outside = Home::new();
    let precious = outside.file(".sunrise-cache/decoy", b"outside cache");
    fs::create_dir_all(home.0.path().join("game/instances")).unwrap();
    std::os::unix::fs::symlink(
        outside.0.path(),
        home.0.path().join("game/instances/linked"),
    )
    .unwrap();
    let _ = home.run(&["storage", "--clear", "other-caches"]);
    assert_eq!(
        fs::read(precious).unwrap(),
        b"outside cache",
        "outside instance target was touched"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn hundreds_of_reports_and_jars_fit_the_descriptor_budget() {
    let home = Home::new();
    home.installed_version();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(15 * 86400);
    for index in 0..400 {
        let path = home.file(&format!("game/logs/{index}.log"), b"old log");
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(old))
            .unwrap();
        home.file(&format!("game/jars/{index}.jar"), b"unused jar");
    }
    let report = home.run(&["storage"]);
    assert!(
        report.status.success(),
        "read-only report exceeded 256 FDs: {report:?}"
    );
    home.clear("other-caches");
    home.clear("other-caches");
    assert!(!home.0.path().join("game/jars/399.jar").exists());
    assert_eq!(
        fs::read_dir(home.0.path().join("game/logs"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "log"))
            .count(),
        1
    );
    assert_eq!(
        fs::read_dir(home.0.path().join("game/jars"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn private_build_tools_still_clear_and_report_freed_bytes() {
    let home = Home::new();
    // Cleanup retains directory ancestry, not just the three cache roots. Establish the
    // complete empty skeleton using the same paths before measuring its allocated size.
    // Clearing zero-length placeholders also gives NTFS the same directory-entry history.
    let paths = [
        "launcher/rust/toolchain/bin/rustc",
        "launcher/cargo/registry/cache/output",
        "launcher/tmp/output",
    ];
    for path in paths {
        home.file(path, b"");
    }
    let empty = home.clear("rust-compiler");
    for path in paths {
        assert!(!home.0.path().join(path).exists(), "{path}");
    }
    let empty_row = empty["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["category"] == "rust-compiler")
        .unwrap();
    let retained_bytes = empty_row["remaining_bytes"].as_u64().unwrap();
    let rust = home.file("launcher/rust/toolchain/bin/rustc", &[1; 8192]);
    let cargo = home.file("launcher/cargo/registry/cache/output", &[2; 8192]);
    let scratch = home.file("launcher/tmp/output", &[3; 8192]);
    let lock_path = home.file("launcher/cargo/.package-cache", b"");
    let lock = fs::File::options()
        .read(true)
        .write(true)
        .open(lock_path)
        .unwrap();
    lock.lock().unwrap();
    let busy = home.run(&["storage", "--clear", "rust-compiler"]);
    assert!(!busy.status.success(), "{busy:?}");
    assert_eq!(fs::read(&cargo).unwrap(), vec![2; 8192]);
    drop(lock);
    // The busy attempt may clear Rust before encountering Cargo's lock. Refill it so
    // the successful clear must report freeing all three payloads, not just two.
    fs::write(&rust, [1; 8192]).unwrap();
    let report = home.clear("rust-compiler");
    for path in [rust, cargo, scratch] {
        assert!(!path.exists(), "{path:?}");
    }
    let row = report["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["category"] == "rust-compiler")
        .unwrap();
    assert!(row["freed_bytes"].as_u64().unwrap() >= 3 * 8192, "{row}");
    assert_eq!(row["remaining_bytes"], retained_bytes);
}

#[test]
fn explicit_removal_deletes_an_idle_version_from_before_runtime_leases() {
    let home = Home::new();
    let marker = home.file("launcher/versions/spare/.installed", b"installed");
    let game = home.file("launcher/versions/spare/game", b"game binary");
    let output = home.run(&["remove", "spare"]);
    assert!(output.status.success(), "{output:?}");
    assert!(!marker.exists());
    assert!(!game.exists());
}

#[test]
fn markerless_profile_clears_only_the_named_game_caches() {
    let home = Home::new();
    home.installed_version();
    home.file(
        &format!(
            "launcher/native/server-builds/server/sunrise-engine{}",
            std::env::consts::EXE_SUFFIX
        ),
        b"markerless",
    );
    let paths = [
        "game/.sunrise-cache/behaviours-v8.bin",
        "game/.sunrise-cache/behaviours-v7.bin",
        "game/.sunrise-cache/registry-v1.bin",
        "game/.sunrise-cache/mod-content/overlay",
        "game/.sunrise-cache/resourcepacks/overlay",
        "game/cache/dx12-shaders.bin",
    ]
    .map(|path| home.file(path, b"rebuildable"));
    let canaries = [
        "game/.sunrise-cache/behaviours-v9.bin",
        "game/.sunrise-cache/precious",
        "game/.sunrise-cache/.trash-123-456/precious",
        "game/cache/precious",
        "game/cache/private/precious",
        "game/saves/world/level.dat",
        "game/options.txt",
        "game/screenshots/precious",
        "game/resourcepacks/precious",
        "game/mods/precious",
        "game/mod-browser/active-build.json",
    ]
    .map(|path| home.file(path, b"player data"));
    home.clear("other-caches");
    assert!(paths.iter().all(|path| !path.exists()));
    for path in canaries {
        assert_eq!(fs::read(path).unwrap(), b"player data");
    }
}

#[cfg(any(unix, windows))]
#[test]
fn profile_cache_links_never_delete_outside_targets() {
    let home = Home::new();
    home.installed_version();
    let canary = home.file("outside/player-data", b"keep");
    let target = canary.parent().unwrap();
    for relative in [
        "game/.sunrise-cache/behaviours-v8.bin",
        "game/.sunrise-cache/behaviours-v7.bin",
        "game/.sunrise-cache/registry-v1.bin",
        "game/.sunrise-cache/mod-content",
        "game/.sunrise-cache/mod-content/linked",
        "game/.sunrise-cache/resourcepacks",
        "game/.sunrise-cache/resourcepacks/linked",
        "game/cache/dx12-shaders.bin",
    ] {
        let link = home.0.path().join(relative);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, &link).unwrap();
        #[cfg(windows)]
        assert!(Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(
                link.as_os_str()
                    .to_string_lossy()
                    .trim_start_matches(r"\\?\")
            )
            .arg(
                target
                    .as_os_str()
                    .to_string_lossy()
                    .trim_start_matches(r"\\?\")
            )
            .status()
            .unwrap()
            .success());
        let report = home.run(&["storage", "--clear", "other-caches"]);
        assert_eq!(report.status.code(), Some(1), "{relative}: {report:?}");
        assert_eq!(fs::read(&canary).unwrap(), b"keep", "{relative}");
        #[cfg(unix)]
        fs::remove_file(&link).unwrap();
        #[cfg(windows)]
        fs::remove_dir(&link).unwrap();
    }
}

#[cfg(any(unix, windows))]
#[test]
fn profile_name_gate_keeps_caches_until_released() {
    let home = Home::new();
    home.installed_version();
    let gate = home.file("game/.install-txn.lock", b"");
    let cache = home.file("game/cache/dx12-shaders.bin", b"cache");
    let mut options = fs::File::options();
    options.read(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let gate = options.open(gate).unwrap();
    #[cfg(unix)]
    gate.lock().unwrap();
    let output = home.run(&["storage", "--clear", "other-caches"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(cache.exists());
    drop(gate);
    home.clear("other-caches");
    assert!(!cache.exists());
}

#[test]
fn detached_trash_does_not_sweep_a_profile_that_shares_the_launcher_root() {
    let home = Home::new();
    home.installed_version();
    let unrelated = home.file("launcher/.trash-123-456/player-data", b"keep");
    let unrelated_file = home.file("launcher/.trash-123-457-1", b"keep");
    let transaction = home.file("launcher/mod-browser/install-txn.json", b"keep");
    let caches = [
        home.file("launcher/.sunrise-cache/behaviours-v8.bin", b"rebuildable"),
        home.file("launcher/.sunrise-cache/behaviours-v7.bin", b"rebuildable"),
    ];
    let output = home.run_in("launcher", &["storage", "--clear", "other-caches"]);
    assert!(output.status.success(), "{output:?}");
    for path in [unrelated, unrelated_file, transaction] {
        assert_eq!(fs::read(path).unwrap(), b"keep");
    }
    for cache in caches {
        assert!(!cache.exists());
    }
}

#[test]
fn private_stage_directories_under_versions_are_excluded_from_spare_game_builds() {
    let home = Home::new();
    home.installed_version();
    let build_stage = home.file("launcher/versions/.build-stage-active/payload", &[5; 16384]);
    let repair_stage = home.file("launcher/versions/.repair-active/payload", &[6; 16384]);
    let incomplete = home.file("launcher/versions/v0.0.0-incomplete/payload", &[7; 16384]);

    let output = home.run(&["--json", "storage"]);
    assert!(output.status.success(), "{output:?}");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let row = report["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["category"] == "spare-game-builds")
        .unwrap();
    let expected = row["remaining_bytes"].as_u64().unwrap();
    assert!(expected > 0, "incomplete build must still be reported");

    let cleared = home.clear("spare-game-builds");
    let cleared_row = cleared["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["category"] == "spare-game-builds")
        .unwrap();
    assert_eq!(cleared_row["freed_bytes"].as_u64().unwrap(), expected);
    assert_eq!(cleared_row["remaining_bytes"].as_u64().unwrap(), 0);
    assert!(cleared_row["kept"].as_array().unwrap().is_empty());
    assert!(!incomplete.exists());
    assert_eq!(fs::read(build_stage).unwrap(), vec![5; 16384]);
    assert_eq!(fs::read(repair_stage).unwrap(), vec![6; 16384]);
}
