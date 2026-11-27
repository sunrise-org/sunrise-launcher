use super::*;
use crate::storage::tests::Home;

#[cfg(unix)]
#[test]
fn actor_starting_after_the_last_process_observation_cannot_authorize_cleanup() {
    use std::{
        io::{Read, Write},
        process::{Command, Stdio},
    };
    let home = Home::new();
    let cache = home.seed("cache/output");
    let storage = Storage::at(&home.0).unwrap();
    let entries = vec![Entry::fixed(
        &Root::open(&home.0).unwrap(),
        Path::new("cache"),
        Category::OtherCaches,
    )
    .unwrap()];
    let tag = format!(
        "last-scan-{}",
        home.0.file_name().unwrap().to_string_lossy()
    );
    // This is the LAST process observation: the actor starts only after its result exists.
    assert!(!backend::in_use(&tag));
    let mut actor = Command::new("sh")
        .args([
            "-c",
            "exec 3<>\"$1\"; printf ready; read -r stop",
            "late-cache-actor",
        ])
        .arg(&cache)
        .arg(backend::version_dir(&tag).join(backend::CLIENT))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut ready = [0; 5];
    actor.stdout.take().unwrap().read_exact(&mut ready).unwrap();
    assert_eq!(&ready, b"ready");
    let result = storage.apply(entries, Some(Category::OtherCaches), false);
    let retained = cache.exists();
    actor.stdin.take().unwrap().write_all(b"stop\n").unwrap();
    assert!(actor.wait().unwrap().success());
    assert!(matches!(result, Err(Error::UnprovenExclusion)));
    assert!(
        retained,
        "actor started after the final scan lost its cache"
    );
    // A later absence observation is not mutual exclusion either.
    assert!(!backend::in_use(&tag));
    let entry = Entry::fixed(
        &Root::open(&home.0).unwrap(),
        Path::new("cache"),
        Category::OtherCaches,
    )
    .unwrap();
    assert!(matches!(
        entry.clear(&home.0),
        Err(Error::UnprovenExclusion)
    ));
    assert!(cache.exists());
}

#[cfg(unix)]
#[test]
fn entry_uses_held_directory_after_root_path_is_redirected() {
    let home = Home::new();
    let outside = Home::new();
    home.seed("cache/output");
    let precious = outside.seed("cache/precious");
    let storage = Storage::at(&home.0).unwrap();
    let entry = Entry::fixed(
        &Root::open(&home.0).unwrap(),
        Path::new("cache"),
        Category::RustCompiler,
    )
    .unwrap();
    let moved = home.0.with_extension("original");
    fs::rename(&home.0, &moved).unwrap();
    std::os::unix::fs::symlink(&outside.0, &home.0).unwrap();
    storage
        .apply(vec![entry], Some(Category::RustCompiler), false)
        .unwrap();
    assert!(precious.exists());
    assert!(!moved.join("cache/output").exists());
    fs::remove_file(&home.0).unwrap();
    fs::rename(moved, &home.0).unwrap();
}

#[cfg(unix)]
#[test]
fn hard_linked_outputs_across_entries_count_once() {
    use std::os::unix::fs::MetadataExt;
    let home = Home::new();
    let output = home.seed("native/source/first");
    fs::create_dir_all(home.0.join("native/patched")).unwrap();
    fs::hard_link(&output, home.0.join("native/patched/second")).unwrap();
    let expected = [
        output,
        home.0.join("native/source"),
        home.0.join("native/patched"),
    ]
    .into_iter()
    .map(|path| fs::metadata(path).unwrap().blocks() * 512)
    .sum::<u64>();
    let storage = Storage::at(&home.0).unwrap();
    let entries = ["native/source", "native/patched"]
        .into_iter()
        .map(|path| {
            Entry::fixed(
                &Root::open(&home.0).unwrap(),
                Path::new(path),
                Category::BuildFiles,
            )
            .unwrap()
        })
        .collect();
    let report = storage.apply(entries, None, false).unwrap();
    let row = report
        .rows
        .iter()
        .find(|row| row.category == Category::BuildFiles)
        .unwrap();
    assert_eq!(row.bytes, expected);
}

#[test]
fn referenced_jar_and_uninstrumented_unused_jar_are_kept() {
    use sha2::{Digest, Sha512};
    use std::io::Write;
    use sunrise_launcher_smod::{manifest::JarRef, Package};
    let home = Home::new();
    fs::create_dir_all(home.0.join("jars")).unwrap();
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    archive
        .start_file(
            "assets/example/lang/en_us.json",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    archive.write_all(b"{}").unwrap();
    let bytes = archive.finish().unwrap().into_inner();
    let jar = JarRef::new(
        "original".into(),
        "1.0.0".into(),
        format!("{:x}", Sha512::digest(&bytes)),
    )
    .unwrap();
    let referenced = jar.cache_path(&home.0.join("jars"));
    fs::write(&referenced, bytes).unwrap();
    let manifest = home.seed("sunrise-mods/example/sunrise-mod.toml");
    fs::write(&manifest, format!("id='example'\nname='Example'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n[[jar]]\nid='original'\nversion='1.0.0'\nsha512='{}'\n", jar.sha512())).unwrap();
    Package::read(manifest.parent().unwrap()).unwrap();
    let unused = home.seed("jars/unused.jar");
    let storage = Storage::at(&home.0).unwrap();
    let mut entries = Vec::new();
    unused_jars(&mut entries, &Root::open(&home.0).unwrap()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, unused);
    assert!(matches!(
        storage.apply(entries, Some(Category::OtherCaches), false),
        Err(Error::UnprovenExclusion)
    ));
    assert!(referenced.exists());
    assert!(unused.exists());
}

#[test]
fn unreadable_mod_directory_retains_all_jars_instead_of_treating_references_as_empty() {
    let home = Home::new();
    let jar = home.seed("jars/referenced.jar");
    // A regular file where sunrise-mods/ is expected makes directory enumeration fail on every OS.
    home.seed("sunrise-mods");
    let mut entries = Vec::new();
    unused_jars(&mut entries, &Root::open(&home.0).unwrap()).unwrap();
    assert!(
        entries.is_empty(),
        "failed jar-reference discovery must retain all jars instead of selecting {} entries",
        entries.len()
    );
    assert!(jar.exists());
}

#[cfg(unix)]
#[test]
fn capable_units_delete_only_after_the_last_game_lease_and_mixed_builds_stay_residual() {
    crate::test_support::assert_holder_capability();
    use std::process::Command;
    let Some(root) = std::env::var_os("CLASSIFICATION_ROOT") else {
        let root = tempfile::tempdir().unwrap();
        let mut actor = Command::new(std::env::current_exe().unwrap());
        actor.args(["--exact", "storage::inventory::regressions::capable_units_delete_only_after_the_last_game_lease_and_mixed_builds_stay_residual"])
            .env("CLASSIFICATION_ROOT", root.path()).env("SUNRISE_LAUNCHER_HOME", root.path().join("data"))
            .env("SUNRISE_HOME", root.path().join("data/cache"))
            .env("XDG_DATA_HOME", root.path().join("xdg-data")).env("XDG_CONFIG_HOME", root.path().join("xdg-config"))
            .env("SUNRISE_LAUNCHER_TRASH", root.path().join("trash")).env("SUNRISE_LAUNCHER_CLIPBOARD", root.path().join("clipboard"))
            .env("SUNRISE_LAUNCHER_DESKTOP", root.path().join("desktop"))
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1").env("SUNRISE_NO_AUDIO", "1");
        assert!(actor.status().unwrap().success());
        return;
    };
    let root = PathBuf::from(root);
    if std::env::var_os("CLASSIFICATION_GAME").is_some() {
        sunrise_launcher_smod::holder::enter();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(std::env::args_os().skip(1));
        let selection =
            sunrise_launcher_smod::selection::LaunchArguments::for_command(&command).unwrap();
        let _capture = sunrise_launcher_smod::holder::acquire(&selection);
        fs::write(root.join("ready"), b"").unwrap();
        let end = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !root.join("stop").exists() {
            assert!(std::time::Instant::now() < end);
            std::thread::yield_now();
        }
        return;
    }
    let cache = root.join("data/cache");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("resource"), b"whole resource").unwrap();
    fs::create_dir(root.join("work")).unwrap();
    let install = backend::version_dir("modern");
    fs::create_dir_all(&install).unwrap();
    let image = install.join(backend::CLIENT);
    assert!(Command::new("cp")
        .arg(std::env::current_exe().unwrap())
        .arg(&image)
        .status()
        .unwrap()
        .success());
    fs::write(install.join(".installed"), b"").unwrap();
    crate::run_leases::publish(&install).unwrap();
    crate::run_leases::publish(&cache).unwrap();
    assert!(matches!(
        Access::for_unit(&cache).unwrap(),
        Access::LeasedGame
    ));
    let old = backend::version_dir("legacy");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join(".installed"), b"").unwrap();
    fs::write(old.join(backend::CLIENT), b"uninstrumented actor").unwrap();
    assert!(matches!(
        Access::for_unit(&cache).unwrap(),
        Access::UninstrumentedGame
    ));
    assert!(matches!(
        Access::for_unit(&install).unwrap(),
        Access::LeasedGame
    ));
    assert!(matches!(
        crate::cache_cleanup::delete_game_unit(&cache),
        Err(Error::UnprovenExclusion)
    ));
    assert_eq!(fs::read(cache.join("resource")).unwrap(), b"whole resource");
    fs::remove_dir_all(&old).unwrap();
    let command = || {
        let mut game = Command::new(&image);
        game.args(["--exact", "storage::inventory::regressions::capable_units_delete_only_after_the_last_game_lease_and_mixed_builds_stay_residual"])
            .env("CLASSIFICATION_GAME", "1").current_dir(root.join("work"));
        game
    };
    let mut game = command().spawn().unwrap();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !root.join("ready").exists() {
        assert!(std::time::Instant::now() < end);
        std::thread::yield_now();
    }
    assert!(matches!(
        crate::cache_cleanup::delete_game_unit(&cache),
        Err(Error::Busy)
    ));
    fs::write(root.join("stop"), b"").unwrap();
    assert!(game.wait().unwrap().success());
    crate::cache_cleanup::delete_game_unit(&cache).unwrap();
    assert!(!cache.exists());
    assert_eq!(command().status().unwrap().code(), Some(78));
    fs::remove_file(install.join(".installed")).unwrap();
    let storage = Storage::at(&root.join("data")).unwrap();
    assert!(
        storage
            .entries()
            .unwrap()
            .iter()
            .any(|entry| entry.path == install),
        "incomplete installs must stay visible for residual reporting and retry"
    );
}

#[test]
fn shared_runtime_lease_on_profile_context_root_blocks_clearing_profile_caches() {
    const TEST: &str =
        "storage::inventory::regressions::shared_runtime_lease_on_profile_context_root_blocks_clearing_profile_caches";
    const ACTOR: &str = "STORAGE_CONTEXT_ROOT_LEASE_ACTOR";
    if std::env::var_os(ACTOR).is_none() {
        let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(ACTOR, "1")
            .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
            .env("SUNRISE_HOME", root.path().join("game"))
            .env(
                "SUNRISE_LAUNCHER_LOCATION_FILE",
                root.path().join("location"),
            )
            .env("XDG_DATA_HOME", root.path().join("xdg-data"))
            .env("XDG_CONFIG_HOME", root.path().join("xdg-config"))
            .env("SUNRISE_NO_KEYCHAIN", "1")
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("SUNRISE_NO_AUDIO", "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("test result: ok. 1 passed;"),
            "{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let data = backend::data_dir();
    let home = backend::game_home();
    let version = backend::version_dir("v0.0.1");
    fs::create_dir_all(&version).unwrap();
    fs::write(version.join(".installed"), b"").unwrap();
    fs::write(data.join("settings.json"), br#"{"selected":"v0.0.1"}"#).unwrap();
    let caches = [
        home.join(".sunrise-cache/behaviours-v8.bin"),
        home.join(".sunrise-cache/mod-content/overlay"),
        home.join("cache/dx12-shaders.bin"),
        home.join("jars/unused.jar"),
    ];
    for path in &caches {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"live cache").unwrap();
    }
    // Hold the exact shared runtime lease a running game holds on its profile context root,
    // without any matching version process name.
    let registry = crate::run_leases::Registry::open().unwrap();
    let home_dir = Dir::open_ambient_dir(&home, cap_std::ambient_authority()).unwrap();
    let lease = registry
        .lock(&home_dir, crate::native_cache::LockMode::Shared)
        .unwrap();
    let storage = Storage::at(&data).unwrap();
    let blocked = storage.report(Some(Category::OtherCaches), false).unwrap();
    let row = blocked
        .rows
        .iter()
        .find(|row| row.category == Category::OtherCaches)
        .unwrap();
    assert!(row.kept.iter().any(|unit| unit.kind == KeptKind::Busy));
    for path in &caches {
        assert_eq!(
            fs::read(path).unwrap(),
            b"live cache",
            "cache was removed while its profile context root held a shared runtime lease"
        );
    }
    drop(lease);
    drop(home_dir);
    let cleared = storage.report(Some(Category::OtherCaches), false).unwrap();
    let row = cleared
        .rows
        .iter()
        .find(|row| row.category == Category::OtherCaches)
        .unwrap();
    assert!(
        row.kept.is_empty(),
        "expected caches to clear once context lease dropped"
    );
    for path in &caches {
        assert!(
            !path.exists(),
            "cache still existed after context lease dropped"
        );
    }
}

#[test]
fn incomplete_build_retry_and_clear_recheck_enforce_ordinary_profile_pins() {
    const TEST: &str = "storage::inventory::regressions::incomplete_build_retry_and_clear_recheck_enforce_ordinary_profile_pins";
    const ACTOR: &str = "STORAGE_ORDINARY_PIN_RECHECK_ACTOR";
    if std::env::var_os(ACTOR).is_none() {
        let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(ACTOR, "1")
            .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
            .env("SUNRISE_HOME", root.path().join("game"))
            .env(
                "SUNRISE_LAUNCHER_LOCATION_FILE",
                root.path().join("location"),
            )
            .env("XDG_DATA_HOME", root.path().join("xdg-data"))
            .env("XDG_CONFIG_HOME", root.path().join("xdg-config"))
            .env("SUNRISE_NO_KEYCHAIN", "1")
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("SUNRISE_NO_AUDIO", "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("test result: ok. 1 passed;"),
            "{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let data = backend::data_dir();
    let base = backend::version_dir("v0.0.1");
    fs::create_dir_all(&base).unwrap();
    fs::write(base.join(".installed"), b"").unwrap();
    fs::write(data.join("settings.json"), br#"{"selected":"v0.0.1"}"#).unwrap();

    // 1. An incomplete build (no `.installed`) pinned by an ordinary profile must neither be
    // selected for retry nor removed under the publication fence.
    let incomplete = backend::version_dir("native-incomplete");
    fs::create_dir_all(&incomplete).unwrap();
    fs::write(incomplete.join("game"), b"pinned incomplete").unwrap();
    let pin = instances::create("Pinned", None).unwrap();
    instances::edit(&pin.id, |meta| {
        meta.version = Some("native-incomplete".into())
    })
    .unwrap();

    // 2. A spare native build pinned after Clear selects it must be refused under the fence.
    let raced = backend::version_dir("native-raced");
    fs::create_dir_all(&raced).unwrap();
    fs::write(raced.join(".installed"), b"").unwrap();
    fs::write(raced.join("game"), b"newly pinned").unwrap();
    let late = instances::create("Late Pin", None).unwrap();

    let storage = Storage::at(&data).unwrap();
    let entries = storage.entries().unwrap();
    assert!(
        !entries.iter().any(|entry| entry.path == incomplete),
        "incomplete build pinned by an ordinary profile was selected for retry"
    );
    assert!(
        entries.iter().any(|entry| entry.path == raced),
        "unpinned spare native build should be a candidate before the late pin"
    );

    let late_id = late.id.clone();
    let report = storage
        .clear_observed(Some(Category::SpareGameBuilds), false, |path| {
            if path == raced {
                instances::edit(&late_id, |meta| meta.version = Some("native-raced".into()))
                    .unwrap();
            }
        })
        .unwrap();
    let row = report
        .rows
        .iter()
        .find(|row| row.category == Category::SpareGameBuilds)
        .unwrap();
    assert!(row
        .kept
        .iter()
        .any(|unit| unit.kind == KeptKind::ReferencedProfile));
    assert_eq!(
        fs::read(raced.join("game")).unwrap(),
        b"newly pinned",
        "spare build pinned after selection was deleted"
    );
    assert_eq!(
        fs::read(incomplete.join("game")).unwrap(),
        b"pinned incomplete",
        "pinned incomplete build was deleted"
    );
    assert!(matches!(
        crate::cache_cleanup::clear_claimed_unit_with_contexts(&incomplete, &[], None),
        Err(Error::ReferencedProfile)
    ));

    // 3. With an unreadable instances.json, incomplete builds are neither selected nor removed.
    let orphan = backend::version_dir("native-unproven");
    fs::create_dir_all(&orphan).unwrap();
    fs::write(orphan.join("game"), b"unproven").unwrap();
    fs::write(data.join("instances.json"), b"{not valid json").unwrap();
    assert!(
        !storage
            .entries()
            .unwrap()
            .iter()
            .any(|entry| entry.path == orphan),
        "unreadable instances.json must suppress incomplete-build retry selection"
    );
    assert!(matches!(
        crate::cache_cleanup::clear_claimed_unit_with_contexts(&orphan, &[], None),
        Err(Error::UnprovenReferences)
    ));
    assert_eq!(fs::read(orphan.join("game")).unwrap(), b"unproven");
}

#[test]
fn clear_refuses_units_replaced_with_different_identity_after_selection() {
    const TEST: &str = "storage::inventory::regressions::clear_refuses_units_replaced_with_different_identity_after_selection";
    const ACTOR: &str = "STORAGE_IDENTITY_BIND_ACTOR";
    if std::env::var_os(ACTOR).is_none() {
        let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(ACTOR, "1")
            .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
            .env("SUNRISE_HOME", root.path().join("game"))
            .env(
                "SUNRISE_LAUNCHER_LOCATION_FILE",
                root.path().join("location"),
            )
            .env("XDG_DATA_HOME", root.path().join("xdg-data"))
            .env("XDG_CONFIG_HOME", root.path().join("xdg-config"))
            .env("SUNRISE_NO_KEYCHAIN", "1")
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("SUNRISE_NO_AUDIO", "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("test result: ok. 1 passed;"),
            "{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let data = backend::data_dir();
    let home = backend::game_home();
    let hold = data.parent().unwrap().join("held-originals");
    fs::create_dir_all(&hold).unwrap();

    let version = backend::version_dir("v0.0.1");
    fs::create_dir_all(&version).unwrap();
    fs::write(version.join(".installed"), b"").unwrap();
    fs::write(data.join("settings.json"), br#"{"selected":"v0.0.1"}"#).unwrap();

    // 1. Directory and file units cleared via Registry::remove_unit_with_contexts.
    let cache_dir = home.join(".sunrise-cache/mod-content");
    fs::create_dir_all(&cache_dir).unwrap();
    fs::write(cache_dir.join("original"), b"old dir").unwrap();
    let cache_file = home.join(".sunrise-cache/behaviours-v8.bin");
    fs::write(&cache_file, b"old file").unwrap();

    // 2. Build-tool directory cleared via Storage::clear_entry_bound.
    let rust_unit = data.join("rust/old");
    fs::create_dir_all(&rust_unit).unwrap();
    fs::write(rust_unit.join("payload"), b"old rust").unwrap();

    let storage = Storage::at(&data).unwrap();
    let report = storage
        .clear_observed(None, true, |path| {
            if path == cache_dir {
                // Keep the original alive under `hold` so the filesystem cannot reuse its inode.
                fs::rename(&cache_dir, hold.join("mod-content")).unwrap();
                fs::create_dir_all(&cache_dir).unwrap();
                fs::write(cache_dir.join("replacement"), b"new dir").unwrap();
            } else if path == cache_file {
                fs::rename(&cache_file, hold.join("behaviours-v8.bin")).unwrap();
                fs::write(&cache_file, b"new file").unwrap();
            } else if path == rust_unit {
                fs::rename(&rust_unit, hold.join("rust-old")).unwrap();
                fs::create_dir_all(&rust_unit).unwrap();
                fs::write(rust_unit.join("replacement"), b"new rust").unwrap();
            }
        })
        .unwrap();

    let other = report
        .rows
        .iter()
        .find(|row| row.category == Category::OtherCaches)
        .unwrap();
    assert_eq!(
        other
            .kept
            .iter()
            .filter(|unit| unit.kind == KeptKind::Unsafe)
            .count(),
        2,
        "both swapped profile cache units must be refused as Unsafe"
    );
    assert_eq!(fs::read(cache_dir.join("replacement")).unwrap(), b"new dir");
    assert_eq!(fs::read(&cache_file).unwrap(), b"new file");

    let rust_row = report
        .rows
        .iter()
        .find(|row| row.category == Category::RustCompiler)
        .unwrap();
    assert!(rust_row
        .kept
        .iter()
        .any(|unit| unit.kind == KeptKind::Unsafe));
    assert_eq!(
        fs::read(rust_unit.join("replacement")).unwrap(),
        b"new rust"
    );
}

#[cfg(unix)]
#[test]
fn explicit_storage_clear_keeps_version_started_after_discovery_as_busy() {
    use std::{
        io::Read,
        process::{Child, Command, Stdio},
    };
    const TEST: &str = "storage::inventory::regressions::explicit_storage_clear_keeps_version_started_after_discovery_as_busy";
    const ACTOR: &str = "STORAGE_VERSION_IN_USE_ACTOR";
    const TAG: &str = "v0.0.0-late-server";
    if std::env::var_os(ACTOR).is_none() {
        let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(ACTOR, "1")
            .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
            .env("SUNRISE_HOME", root.path().join("game"))
            .env(
                "SUNRISE_LAUNCHER_LOCATION_FILE",
                root.path().join("location"),
            )
            .env("XDG_DATA_HOME", root.path().join("xdg-data"))
            .env("XDG_CONFIG_HOME", root.path().join("xdg-config"))
            .env("SUNRISE_NO_KEYCHAIN", "1")
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("SUNRISE_NO_AUDIO", "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("test result: ok. 1 passed;"),
            "{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    struct StopChild(Option<Child>);
    impl Drop for StopChild {
        fn drop(&mut self) {
            if let Some(mut child) = self.0.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    let data = backend::data_dir();
    let selected = backend::version_dir("v0.0.1");
    fs::create_dir_all(&selected).unwrap();
    fs::write(selected.join(".installed"), b"").unwrap();
    fs::write(data.join("settings.json"), br#"{"selected":"v0.0.1"}"#).unwrap();

    // Unpinned incomplete version unit under versions/: selected by discovery as SpareGameBuilds.
    let spare = backend::version_dir(TAG);
    fs::create_dir_all(&spare).unwrap();
    fs::write(spare.join("payload"), b"running server build").unwrap();
    assert!(!backend::in_use(TAG));

    let storage = Storage::at(&data).unwrap();
    let mut running = StopChild(None);
    let report = storage
        .clear_observed(Some(Category::SpareGameBuilds), false, |path| {
            if path == spare {
                let marker = format!("{}/sunrise-server", spare.display());
                let mut child = Command::new("sh")
                    .args(["-c", "printf ready; read -r stop", "late-version-actor"])
                    .arg(&marker)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                let mut ready = [0u8; 5];
                child
                    .stdout
                    .as_mut()
                    .unwrap()
                    .read_exact(&mut ready)
                    .unwrap();
                assert_eq!(&ready, b"ready");
                assert!(backend::in_use(TAG));
                running.0 = Some(child);
            }
        })
        .unwrap();

    assert!(
        running.0.is_some(),
        "clear_observed did not visit the spare version unit"
    );
    let row = report
        .rows
        .iter()
        .find(|row| row.category == Category::SpareGameBuilds)
        .unwrap();
    assert!(
        row.kept.iter().any(|unit| unit.kind == KeptKind::Busy),
        "expected Busy keep for in-use version unit, got {:?}",
        row.kept
            .iter()
            .map(|u| (u.kind, u.reason.as_str()))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        fs::read(spare.join("payload")).unwrap(),
        b"running server build"
    );

    drop(running);
    assert!(!backend::in_use(TAG));
    let after = storage
        .report(Some(Category::SpareGameBuilds), false)
        .unwrap();
    let row = after
        .rows
        .iter()
        .find(|row| row.category == Category::SpareGameBuilds)
        .unwrap();
    assert!(
        row.kept.is_empty(),
        "unexpected kept units: {:?}",
        row.kept
            .iter()
            .map(|u| (u.kind, u.reason.as_str()))
            .collect::<Vec<_>>()
    );
    assert!(!spare.exists());
}

#[cfg(windows)]
fn noncanonical_windows_path(path: PathBuf) -> PathBuf {
    let plain = backend::plain(path);
    let text = plain.to_string_lossy();
    if text.contains('~') {
        return plain;
    }
    let bytes = text.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_uppercase() && bytes[1] == b':' {
        let mut lowered = String::with_capacity(text.len());
        lowered.push(bytes[0].to_ascii_lowercase() as char);
        lowered.push_str(&text[1..]);
        return PathBuf::from(lowered);
    }
    plain
}

#[test]
fn explicit_clear_under_noncanonical_short_path_keeps_leased_version() {
    const TEST: &str = "storage::inventory::regressions::explicit_clear_under_noncanonical_short_path_keeps_leased_version";
    const ACTOR: &str = "STORAGE_SHORT_PATH_LEASE_ACTOR";
    if std::env::var_os(ACTOR).is_none() {
        // Keep the raw temp dir (8.3 short path on Windows CI, e.g. C:\Users\RUNNER~1\...)
        // rather than pre-canonicalizing it, so boundary normalization is exercised end-to-end.
        let root = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        let launcher_home = noncanonical_windows_path(root.path().join("launcher"));
        #[cfg(not(windows))]
        let launcher_home = root.path().join("launcher");
        #[cfg(windows)]
        let game_home = noncanonical_windows_path(root.path().join("game"));
        #[cfg(not(windows))]
        let game_home = root.path().join("game");
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(ACTOR, "1")
            .env("SUNRISE_LAUNCHER_HOME", launcher_home)
            .env("SUNRISE_HOME", game_home)
            .env(
                "SUNRISE_LAUNCHER_LOCATION_FILE",
                root.path().join("location"),
            )
            .env("XDG_DATA_HOME", root.path().join("xdg-data"))
            .env("XDG_CONFIG_HOME", root.path().join("xdg-config"))
            .env("SUNRISE_NO_KEYCHAIN", "1")
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("SUNRISE_NO_AUDIO", "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("test result: ok. 1 passed;"),
            "{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    let data = backend::data_dir();
    let selected = backend::version_dir("v0.0.1");
    fs::create_dir_all(&selected).unwrap();
    fs::write(selected.join(".installed"), b"").unwrap();
    fs::write(data.join("settings.json"), br#"{"selected":"v0.0.1"}"#).unwrap();

    let spare = backend::version_dir("native-short-leased");
    fs::create_dir_all(&spare).unwrap();
    fs::write(spare.join(".installed"), b"").unwrap();
    fs::write(spare.join("native-mods.json"), b"{}").unwrap();
    fs::write(spare.join(backend::CLIENT), b"native game").unwrap();

    let tree = Dir::open_ambient_dir(&spare, cap_std::ambient_authority()).unwrap();
    let playing = crate::run_leases::Registry::open()
        .unwrap()
        .lock(&tree, crate::native_cache::LockMode::Shared)
        .unwrap();
    drop(tree);

    let storage = Storage::open().unwrap();
    let blocked = storage
        .report(Some(Category::SpareGameBuilds), false)
        .unwrap();
    let row = blocked
        .rows
        .iter()
        .find(|row| row.category == Category::SpareGameBuilds)
        .unwrap();
    assert!(
        row.kept.iter().any(|unit| unit.kind == KeptKind::Busy),
        "expected Busy keep for leased version under short/non-canonical home, got {:?}",
        row.kept
            .iter()
            .map(|u| (u.kind, u.reason.as_str()))
            .collect::<Vec<_>>()
    );
    assert_eq!(fs::read(spare.join("native-mods.json")).unwrap(), b"{}");
    assert_eq!(
        fs::read(spare.join(backend::CLIENT)).unwrap(),
        b"native game"
    );

    drop(playing);
    let cleared = storage
        .report(Some(Category::SpareGameBuilds), false)
        .unwrap();
    let row = cleared
        .rows
        .iter()
        .find(|row| row.category == Category::SpareGameBuilds)
        .unwrap();
    assert!(
        row.kept.is_empty(),
        "unexpected kept units after lease drop: {:?}",
        row.kept
            .iter()
            .map(|u| (u.kind, u.reason.as_str()))
            .collect::<Vec<_>>()
    );
    assert!(!spare.exists());
}

#[test]
fn entry_remove_refuses_path_outside_storage_and_data_roots() {
    let storage_home = Home::new();
    let outside = Home::new();
    let profile_home = Home::new();
    let outside_version = outside.seed("versions/precious/game");
    let outside_rust = outside.seed("rust/precious/bin");
    let storage = Storage::at(&storage_home.0).unwrap();
    let outside_root = Root::open(&outside.0).unwrap();

    let spare = Entry::fixed(
        &outside_root,
        Path::new("versions/precious"),
        Category::SpareGameBuilds,
    )
    .unwrap();
    assert!(matches!(
        spare.remove(&storage.root, true, spare.root_id),
        Err(Error::Unsafe(_))
    ));
    assert_eq!(fs::read(&outside_version).unwrap(), vec![7; 8192]);

    let compiler = Entry::fixed(
        &outside_root,
        Path::new("rust/precious"),
        Category::RustCompiler,
    )
    .unwrap();
    assert!(matches!(
        compiler.remove(&storage.root, true, compiler.root_id),
        Err(Error::Unsafe(_))
    ));
    assert_eq!(fs::read(&outside_rust).unwrap(), vec![7; 8192]);

    let mut mismatched_profile = Entry::fixed(
        &outside_root,
        Path::new("versions/precious"),
        Category::OtherCaches,
    )
    .unwrap();
    mismatched_profile.profile = Some(Arc::new(ProfileContext {
        home: profile_home.0.clone(),
        work: profile_home.0.clone(),
        version: "v0.0.1".into(),
    }));
    assert!(matches!(
        mismatched_profile.remove(&storage.root, true, mismatched_profile.root_id),
        Err(Error::Unsafe(_))
    ));
    assert_eq!(fs::read(&outside_version).unwrap(), vec![7; 8192]);

    let mut at_root = Entry::fixed(
        &Root::open(&storage_home.0).unwrap(),
        Path::new("rust"),
        Category::RustCompiler,
    )
    .unwrap();
    at_root.path = storage.root.clone();
    assert!(matches!(
        at_root.remove(&storage.root, true, at_root.root_id),
        Err(Error::Unsafe(_))
    ));
}
