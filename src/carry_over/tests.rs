use super::*;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Every file under `root`, with its bytes.
fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                todo.push(path);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(&path).unwrap(),
                );
            }
        }
    }
    out
}

#[test]
fn the_earlier_launchers_data_is_copied_once_and_left_untouched() {
    let Some(&(old_launcher, old_game, _)) = crate::identity::EARLIER.first() else {
        return;
    };
    let base = std::env::temp_dir().join(format!("hz-carry-over-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let (data, config) = (base.join("data"), base.join("config"));
    let (from_launcher, from_game) = (data.join(old_launcher), data.join(old_game));
    let background = from_game.join("screenshots/a.png");
    fs::create_dir_all(from_launcher.join("cache")).unwrap();
    fs::create_dir_all(from_game.join("saves/World")).unwrap();
    fs::create_dir_all(from_game.join("instances/modded")).unwrap();
    let settings = serde_json::json!({ "background": background, "language": "fr" });
    fs::write(from_launcher.join("settings.json"), settings.to_string()).unwrap();
    fs::write(from_launcher.join("accounts.json"), r#"{"accounts":[]}"#).unwrap();
    fs::write(from_launcher.join("cache/big.bin"), [7; 64]).unwrap();
    fs::write(from_game.join("saves/World/level.dat"), [1, 2, 3]).unwrap();
    fs::write(from_game.join("instances/modded/options.txt"), "fov:90").unwrap();
    // A Horizon game folder's cache: files and links into the earlier launcher's builds.
    fs::create_dir_all(from_game.join(".horizon-cache/mod-content")).unwrap();
    fs::write(
        from_game.join(".horizon-cache/mod-content/blocks.json"),
        "{}",
    )
    .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        from_launcher.join("settings.json"),
        from_game.join(".horizon-cache/mod-content/link.json"),
    )
    .unwrap();
    // Only the game folder's own caches stay behind: a folder of that name deeper is copied.
    fs::create_dir_all(from_game.join("instances/modded/cache")).unwrap();
    fs::write(from_game.join("instances/modded/cache/kept.txt"), "kept").unwrap();
    let before = (files(&from_launcher), files(&from_game));

    assert_eq!(carry_over(&data, &config), Ok(true));
    assert_eq!(
        (files(&from_launcher), files(&from_game)),
        before,
        "the earlier launcher's folders are unchanged"
    );
    let launcher = crate::identity::launcher_folder_in(&data);
    let game = crate::identity::game_folder_in(&data);
    assert_eq!(
        fs::read(game.join("saves/World/level.dat")).unwrap(),
        [1, 2, 3]
    );
    assert_eq!(
        fs::read_to_string(game.join("instances/modded/options.txt")).unwrap(),
        "fov:90"
    );
    assert!(launcher.join("accounts.json").is_file());
    assert!(!launcher.join("cache").exists(), "caches are made again");
    assert!(
        !game.join(".horizon-cache").exists(),
        "the game's cache is made again"
    );
    assert_eq!(
        fs::read_to_string(game.join("instances/modded/cache/kept.txt")).unwrap(),
        "kept"
    );
    // A path into the earlier game folder now names the copy.
    let copied: serde_json::Value =
        serde_json::from_slice(&fs::read(launcher.join("settings.json")).unwrap()).unwrap();
    assert_eq!(
        copied["background"].as_str().map(PathBuf::from),
        Some(game.join("screenshots/a.png"))
    );
    assert_eq!(copied["language"], "fr");

    // Only the first start copies: this launcher's own data is never replaced.
    fs::write(from_game.join("saves/World/level.dat"), [9]).unwrap();
    assert_eq!(carry_over(&data, &config), Ok(false));
    assert_eq!(
        fs::read(game.join("saves/World/level.dat")).unwrap(),
        [1, 2, 3]
    );
    let _ = fs::remove_dir_all(&base);
}

/// A finished stage (marker, launcher and game folders) beside `data`, with no target yet.
fn finished_stage(data: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let launcher = crate::identity::launcher_folder_in(data);
    let game = crate::identity::game_folder_in(data);
    let stage = launcher.with_file_name(".carry-over");
    fs::create_dir_all(stage.join("launcher")).unwrap();
    fs::write(stage.join("launcher/accounts.json"), "{}").unwrap();
    fs::create_dir_all(stage.join("game/saves/World")).unwrap();
    fs::write(stage.join("game/saves/World/level.dat"), [1, 2, 3]).unwrap();
    fs::write(stage.join(COMPLETE), b"").unwrap();
    (stage, launcher, game)
}

fn scratch(name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!("hz-carry-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    (base.join("data"), base.join("config"), base)
}

#[test]
fn a_copy_stopped_between_its_renames_is_finished_on_the_next_start() {
    let (data, config, base) = scratch("resume");
    let (stage, launcher, game) = finished_stage(&data);
    // The second rename failed: the launcher folder is in place, the saves still staged.
    fs::rename(stage.join("launcher"), &launcher).unwrap();
    assert_eq!(carry_over(&data, &config), Ok(true));
    assert_eq!(
        fs::read(game.join("saves/World/level.dat")).unwrap(),
        [1, 2, 3]
    );
    assert!(launcher.join("accounts.json").is_file());
    assert!(!stage.exists());
    assert_eq!(carry_over(&data, &config), Ok(false));
    let _ = fs::remove_dir_all(&base);
}

/// The `.carry-over-kept-*` folders beside the data folders.
fn kept(data: &Path) -> Vec<PathBuf> {
    let beside = crate::identity::launcher_folder_in(data);
    fs::read_dir(beside.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(".carry-over-kept-"))
        })
        .collect()
}

#[test]
fn a_part_that_was_skipped_is_kept_not_deleted() {
    let (data, config, base) = scratch("kept");
    let (stage, launcher, game) = finished_stage(&data);
    // One part is placed (the launcher's), one skipped (the game folder exists).
    fs::create_dir_all(game.join("saves")).unwrap();
    fs::write(game.join("saves/mine.dat"), [9]).unwrap();
    assert_eq!(carry_over(&data, &config), Ok(true));
    assert!(launcher.join("accounts.json").is_file());
    assert!(!stage.exists());
    let kept = kept(&data);
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(
        fs::read(kept[0].join("game/saves/World/level.dat")).unwrap(),
        [1, 2, 3]
    );
    assert!(
        !kept[0].join("launcher").exists(),
        "the placed part is not kept"
    );
    assert_eq!(fs::read(game.join("saves/mine.dat")).unwrap(), [9]);
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn a_stage_with_everything_in_place_is_removed_and_nothing_is_kept() {
    let (data, config, base) = scratch("nothing-kept");
    let (stage, launcher, _game) = finished_stage(&data);
    fs::rename(stage.join("launcher"), &launcher).unwrap();
    assert_eq!(carry_over(&data, &config), Ok(true));
    assert!(!stage.exists() && kept(&data).is_empty());
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn a_complete_stage_resumes_without_source_or_either_target() {
    let (data, config, base) = scratch("resume-without-source");
    let (stage, launcher, game) = finished_stage(&data);
    assert_eq!(carry_over(&data, &config), Ok(true));
    assert_eq!(fs::read(launcher.join("accounts.json")).unwrap(), b"{}");
    assert_eq!(
        fs::read(game.join("saves/World/level.dat")).unwrap(),
        [1, 2, 3]
    );
    assert!(!stage.exists());
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn finishing_never_replaces_a_target_that_has_saves() {
    let (data, config, base) = scratch("keep");
    let (stage, launcher, game) = finished_stage(&data);
    // The game folder is already there with the player's own saves; the launcher's is not.
    fs::create_dir_all(game.join("saves")).unwrap();
    fs::write(game.join("saves/mine.dat"), [9]).unwrap();
    assert_eq!(carry_over(&data, &config), Ok(true));
    assert_eq!(fs::read(game.join("saves/mine.dat")).unwrap(), [9]);
    assert!(
        !game.join("saves/World").exists(),
        "the staged saves did not replace them"
    );
    assert!(launcher.join("accounts.json").is_file());
    assert!(!stage.exists());
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn the_layout_launcher_1_0_1_left_between_its_renames_is_finished() {
    let (data, config, base) = scratch("v101");
    let (stage, launcher, game) = finished_stage(&data);
    // 1.0.1 wrote no marker: it renamed `launcher` out of the stage and stopped.
    fs::remove_file(stage.join(COMPLETE)).unwrap();
    fs::rename(stage.join("launcher"), &launcher).unwrap();
    fs::write(launcher.join("settings.json"), "{}").unwrap();
    assert_eq!(carry_over(&data, &config), Ok(true));
    assert_eq!(
        fs::read(game.join("saves/World/level.dat")).unwrap(),
        [1, 2, 3]
    );
    assert!(launcher.join("settings.json").is_file());
    assert!(!stage.exists());

    // The same shape with a game folder already in place: it is not replaced.
    let (data, config, base2) = scratch("v101-keep");
    let (stage, launcher, game) = finished_stage(&data);
    fs::remove_file(stage.join(COMPLETE)).unwrap();
    fs::rename(stage.join("launcher"), &launcher).unwrap();
    fs::create_dir_all(game.join("saves")).unwrap();
    fs::write(game.join("saves/mine.dat"), [9]).unwrap();
    assert_eq!(carry_over(&data, &config), Ok(false));
    assert_eq!(fs::read(game.join("saves/mine.dat")).unwrap(), [9]);
    assert!(!stage.exists(), "both targets exist: the leftover goes");
    let _ = fs::remove_dir_all(&base);
    let _ = fs::remove_dir_all(&base2);
}

#[test]
fn a_stage_that_is_not_marked_complete_is_never_moved_into_place() {
    let (data, config, base) = scratch("partial");
    let (stage, launcher, game) = finished_stage(&data);
    fs::remove_file(stage.join(COMPLETE)).unwrap();
    // A copy that failed part way, after which the launcher started empty.
    fs::create_dir_all(&launcher).unwrap();
    assert_eq!(carry_over(&data, &config), Ok(false));
    assert!(!game.exists());
    assert!(stage.join("game/saves/World/level.dat").is_file());
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn a_stage_left_beside_a_finished_copy_is_removed_on_a_later_start() {
    let base = std::env::temp_dir().join(format!("hz-carry-stage-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let (data, config) = (base.join("data"), base.join("config"));
    let launcher = crate::identity::launcher_folder_in(&data);
    let game = crate::identity::game_folder_in(&data);
    let stage = launcher.with_file_name(".carry-over");
    fs::create_dir_all(stage.join("game/saves")).unwrap();
    fs::write(stage.join("game/saves/level.dat"), [1]).unwrap();
    // The stage still has its own `launcher/`: the copy cannot be shown to have finished.
    fs::create_dir_all(stage.join("launcher")).unwrap();
    fs::create_dir_all(&launcher).unwrap();
    fs::write(launcher.join("accounts.json"), "{}").unwrap();

    // Only the launcher folder: the copy may not have finished, so its stage stays.
    assert_eq!(carry_over(&data, &config), Ok(false));
    assert!(stage.join("game/saves/level.dat").is_file());

    fs::create_dir_all(game.join("saves")).unwrap();
    fs::write(game.join("saves/level.dat"), [2]).unwrap();
    assert_eq!(carry_over(&data, &config), Ok(false));
    assert!(!stage.exists());
    assert_eq!(fs::read(game.join("saves/level.dat")).unwrap(), [2]);
    assert_eq!(fs::read(launcher.join("accounts.json")).unwrap(), b"{}");
    let _ = fs::remove_dir_all(&base);
}

/// Links the folder `link` to the folder `target`: a symlink, or a junction on Windows.
#[cfg(any(unix, windows))]
fn link_dir(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    crate::sources::junction(target, link).unwrap();
}

/// Removes the link `link_dir` made, never what it points to.
#[cfg(any(unix, windows))]
fn unlink_dir(link: &Path) {
    #[cfg(unix)]
    fs::remove_file(link).unwrap();
    #[cfg(windows)]
    fs::remove_dir(link).unwrap();
}

#[cfg(any(unix, windows))]
#[test]
fn an_earlier_game_folder_that_is_a_link_is_refused_and_left_untouched() {
    let Some(&(old_launcher, old_game, _)) = crate::identity::EARLIER.first() else {
        return;
    };
    let (data, config, base) = scratch("linked-root");
    let elsewhere = base.join("elsewhere");
    fs::create_dir_all(elsewhere.join("saves/World")).unwrap();
    fs::write(elsewhere.join("saves/World/level.dat"), [1, 2, 3]).unwrap();
    fs::create_dir_all(data.join(old_launcher)).unwrap();
    fs::write(data.join(old_launcher).join("accounts.json"), "{}").unwrap();
    let linked = data.join(old_game);
    link_dir(&elsewhere, &linked);
    let before = files(&elsewhere);

    let error = carry_over(&data, &config).unwrap_err();
    assert!(
        error.contains("Symbolic links and reparse points"),
        "{error}"
    );
    assert!(error.contains(&linked.display().to_string()), "{error}");
    assert_eq!(files(&elsewhere), before, "the linked folder is unchanged");
    assert!(!crate::identity::launcher_folder_in(&data).exists());
    assert!(!crate::identity::game_folder_in(&data).exists());
    unlink_dir(&linked);
    let _ = fs::remove_dir_all(&base);
}

#[cfg(any(unix, windows))]
#[test]
fn a_link_above_the_earlier_folders_is_followed() {
    let Some(&(old_launcher, old_game, _)) = crate::identity::EARLIER.first() else {
        return;
    };
    let (_, config, base) = scratch("linked-parent");
    let real = base.join("real");
    fs::create_dir_all(real.join(old_launcher)).unwrap();
    fs::write(real.join(old_launcher).join("accounts.json"), "{}").unwrap();
    fs::create_dir_all(real.join(old_game).join("saves/World")).unwrap();
    fs::write(real.join(old_game).join("saves/World/level.dat"), [1, 2, 3]).unwrap();
    // The whole data folder is reached through a link, as when it lives on another disk.
    let data = base.join("data");
    link_dir(&real, &data);

    assert_eq!(carry_over(&data, &config), Ok(true));
    assert_eq!(
        fs::read(crate::identity::game_folder_in(&real).join("saves/World/level.dat")).unwrap(),
        [1, 2, 3]
    );
    unlink_dir(&data);
    let _ = fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn a_restricted_earlier_game_folder_stays_restricted() {
    use std::os::unix::fs::PermissionsExt;
    let Some(&(old_launcher, old_game, _)) = crate::identity::EARLIER.first() else {
        return;
    };
    let (data, config, base) = scratch("restricted");
    let from_game = data.join(old_game);
    fs::create_dir_all(data.join(old_launcher)).unwrap();
    fs::create_dir_all(from_game.join("saves")).unwrap();
    fs::write(from_game.join("saves/level.dat"), [1]).unwrap();
    fs::create_dir_all(from_game.join(".horizon-cache")).unwrap();
    fs::set_permissions(&from_game, fs::Permissions::from_mode(0o700)).unwrap();

    assert_eq!(carry_over(&data, &config), Ok(true));
    let game = crate::identity::game_folder_in(&data);
    assert_eq!(
        fs::metadata(&game).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn a_resumed_stage_with_a_linked_root_marker_or_part_is_refused() {
    let (data, config, base) = scratch("resumed-linked-stage");
    let launcher = crate::identity::launcher_folder_in(&data);
    let game = crate::identity::game_folder_in(&data);
    let stage = launcher.with_file_name(".carry-over");
    fs::create_dir_all(&launcher).unwrap();

    // 1. Linked stage root.
    let elsewhere = base.join("elsewhere");
    fs::create_dir_all(elsewhere.join("game/saves")).unwrap();
    fs::write(elsewhere.join(COMPLETE), []).unwrap();
    link_dir(&elsewhere, &stage);
    let error = carry_over(&data, &config).unwrap_err();
    assert!(
        error.contains("Symbolic links and reparse points"),
        "{error}"
    );
    assert!(!game.exists());
    unlink_dir(&stage);

    // 2. Linked staged child (`game`).
    fs::create_dir_all(&stage).unwrap();
    fs::write(stage.join(COMPLETE), []).unwrap();
    link_dir(&elsewhere.join("game"), &stage.join("game"));
    let error = carry_over(&data, &config).unwrap_err();
    assert!(
        error.contains("Symbolic links and reparse points"),
        "{error}"
    );
    assert!(!game.exists());
    unlink_dir(&stage.join("game"));

    // 3. Linked `complete` marker (Unix file symlink).
    #[cfg(unix)]
    {
        fs::remove_file(stage.join(COMPLETE)).unwrap();
        let marker_target = base.join("marker-target");
        fs::write(&marker_target, []).unwrap();
        std::os::unix::fs::symlink(&marker_target, stage.join(COMPLETE)).unwrap();
        let error = carry_over(&data, &config).unwrap_err();
        assert!(
            error.contains("Symbolic links and reparse points"),
            "{error}"
        );
        assert!(!game.exists());
        fs::remove_file(stage.join(COMPLETE)).unwrap();
    }

    // Still allow the platform data base itself to sit behind a link on resume.
    let real_data = base.join("real-data");
    let linked_data = base.join("linked-data");
    let real_launcher = crate::identity::launcher_folder_in(&real_data);
    let real_stage = real_launcher.with_file_name(".carry-over");
    fs::create_dir_all(&real_launcher).unwrap();
    fs::create_dir_all(real_stage.join("game/saves/World")).unwrap();
    fs::write(real_stage.join("game/saves/World/level.dat"), [1, 2, 3]).unwrap();
    fs::write(real_stage.join(COMPLETE), []).unwrap();
    link_dir(&real_data, &linked_data);
    assert_eq!(carry_over(&linked_data, &config), Ok(true));
    assert_eq!(
        fs::read(crate::identity::game_folder_in(&real_data).join("saves/World/level.dat"))
            .unwrap(),
        [1, 2, 3]
    );
    unlink_dir(&linked_data);
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn a_linked_earlier_location_is_refused_with_the_named_error() {
    let Some(&(_, _, location)) = crate::identity::EARLIER.first() else {
        return;
    };
    let (data, config, base) = scratch("linked-earlier-location");
    let real_launcher = base.join("real-launcher");
    let real_game = base.join("real-game");
    fs::create_dir_all(&real_launcher).unwrap();
    fs::create_dir_all(&real_game).unwrap();
    fs::write(real_launcher.join("settings.json"), r#"{"ok":true}"#).unwrap();
    let linked_launcher = base.join("linked-launcher");
    link_dir(&real_launcher, &linked_launcher);

    let loc_dir = config.join(location);
    fs::create_dir_all(&loc_dir).unwrap();
    let loc_file = loc_dir.join("location.json");
    let location_json = serde_json::json!({
        "version": 1,
        "roots": {
            "launcher": linked_launcher,
            "game": real_game,
            "build": linked_launcher,
        }
    });
    fs::write(&loc_file, location_json.to_string()).unwrap();

    let error = carry_over(&data, &config).unwrap_err();
    let linked_text = fs::canonicalize(&base)
        .unwrap()
        .join("linked-launcher")
        .display()
        .to_string();
    assert!(
        error.contains("Symbolic links and reparse points") && error.contains(&linked_text),
        "expected link refusal naming {linked_text}, got: {error}"
    );
    // Neither target root was created, so repairing the pointer allows carry_over to retry.
    assert!(!crate::identity::launcher_folder_in(&data).exists());
    assert!(!crate::identity::game_folder_in(&data).exists());

    let repaired_json = serde_json::json!({
        "version": 1,
        "roots": {
            "launcher": real_launcher,
            "game": real_game,
            "build": real_launcher,
        }
    });
    fs::write(&loc_file, repaired_json.to_string()).unwrap();
    assert!(carry_over(&data, &config).unwrap());
    let copied: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(crate::identity::launcher_folder_in(&data).join("settings.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(copied, serde_json::json!({ "ok": true }));
    unlink_dir(&linked_launcher);
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn a_failed_starts_locks_and_caches_do_not_stop_the_retry() {
    let Some(&(_, _, location)) = crate::identity::EARLIER.first() else {
        return;
    };
    let (data, config, base) = scratch("leftover-retry");
    let real_launcher = base.join("real-launcher");
    let real_game = base.join("real-game");
    fs::create_dir_all(&real_launcher).unwrap();
    fs::create_dir_all(real_game.join("saves/World")).unwrap();
    fs::write(real_launcher.join("settings.json"), r#"{"ok":true}"#).unwrap();
    fs::write(real_game.join("saves/World/level.dat"), [1, 2, 3]).unwrap();
    let linked_launcher = base.join("linked-launcher");
    link_dir(&real_launcher, &linked_launcher);
    let loc_dir = config.join(location);
    fs::create_dir_all(&loc_dir).unwrap();
    let pointer = |launcher: &Path| {
        serde_json::json!({
            "version": 1,
            "roots": { "launcher": launcher, "game": real_game, "build": launcher }
        })
        .to_string()
    };
    fs::write(loc_dir.join("location.json"), pointer(&linked_launcher)).unwrap();
    assert!(carry_over(&data, &config).is_err());

    // What the window of that failed start made: the instances lock and a GitHub cache.
    let launcher = crate::identity::launcher_folder_in(&data);
    fs::create_dir_all(launcher.join("cache/github")).unwrap();
    fs::write(launcher.join(".instances.lock"), b"").unwrap();
    fs::write(launcher.join("cache/github/answer.json"), b"{}").unwrap();

    fs::write(loc_dir.join("location.json"), pointer(&real_launcher)).unwrap();
    assert_eq!(carry_over(&data, &config), Ok(true));
    let copied: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(launcher.join("settings.json")).unwrap()).unwrap();
    assert_eq!(copied, serde_json::json!({ "ok": true }));
    assert!(!launcher.join(".instances.lock").exists());
    assert!(!launcher.join("cache").exists());
    assert!(!launcher.with_file_name(".carry-over-leftover").exists());
    assert_eq!(
        fs::read(crate::identity::game_folder_in(&data).join("saves/World/level.dat")).unwrap(),
        [1, 2, 3]
    );
    // The earlier launcher's own folders are only read.
    assert!(real_launcher.join("settings.json").is_file());
    unlink_dir(&linked_launcher);
    let _ = fs::remove_dir_all(&base);
}

/// What the window of a failed start makes in the launcher folder.
fn leave_a_failed_start(launcher: &Path) {
    fs::create_dir_all(launcher.join("cache/github")).unwrap();
    fs::write(launcher.join(".instances.lock"), b"").unwrap();
    fs::write(launcher.join("cache/github/answer.json"), b"{}").unwrap();
}

#[test]
fn a_complete_stage_beside_a_failed_starts_folder_goes_into_place() {
    let (data, config, base) = scratch("resume-leftover");
    let (stage, launcher, game) = finished_stage(&data);
    // The rename into place failed, and that start's window made the folder again.
    leave_a_failed_start(&launcher);
    assert_eq!(carry_over(&data, &config), Ok(true));
    assert_eq!(fs::read(launcher.join("accounts.json")).unwrap(), b"{}");
    assert!(!launcher.join(".instances.lock").exists());
    assert_eq!(
        fs::read(game.join("saves/World/level.dat")).unwrap(),
        [1, 2, 3]
    );
    assert!(!stage.exists());
    assert!(!launcher.with_file_name(LEFTOVER).exists());
    let kept: Vec<_> = fs::read_dir(launcher.parent().unwrap())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(".carry-over-kept")
        })
        .collect();
    assert!(kept.is_empty(), "the copy was parked in {kept:?}");
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn anything_but_a_failed_starts_lock_and_caches_is_the_players() {
    let Some(&(old_launcher, _, _)) = crate::identity::EARLIER.first() else {
        return;
    };
    for kept in [
        "themes/mine.json",
        "saved-skins/a.png",
        "publisher-pins.json",
        "unknown",
    ] {
        let (data, config, base) = scratch("players-own");
        fs::create_dir_all(data.join(old_launcher)).unwrap();
        fs::write(data.join(old_launcher).join("settings.json"), b"{}").unwrap();
        let launcher = crate::identity::launcher_folder_in(&data);
        leave_a_failed_start(&launcher);
        let path = launcher.join(kept);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"mine").unwrap();
        assert_eq!(carry_over(&data, &config), Ok(false), "{kept}");
        assert_eq!(fs::read(&path).unwrap(), b"mine", "{kept}");
        assert!(!launcher.join("settings.json").exists(), "{kept}");
        assert!(!launcher.with_file_name(".carry-over").exists(), "{kept}");
        let _ = fs::remove_dir_all(&base);
    }
}

#[test]
fn a_folder_that_gains_the_players_files_while_set_aside_goes_back() {
    let (_, _, base) = scratch("set-aside");
    let launcher = base.join("launcher");
    let leftover = base.join(LEFTOVER);
    leave_a_failed_start(&launcher);
    assert_eq!(set_aside(&launcher, &leftover), Ok(Some(true)));
    assert!(!launcher.exists() && leftover.join(".instances.lock").is_file());

    // Another window wrote the player's settings into it: it is theirs now, so it stays.
    fs::rename(&leftover, &launcher).unwrap();
    fs::write(launcher.join("settings.json"), b"{}").unwrap();
    assert_eq!(set_aside(&launcher, &leftover), Ok(None));
    assert!(launcher.join("settings.json").is_file());
    assert!(!leftover.exists());

    // A leftover holding the player's files is never removed to make room.
    fs::create_dir_all(&leftover).unwrap();
    fs::write(leftover.join("accounts.json"), b"[]").unwrap();
    fs::remove_file(launcher.join("settings.json")).unwrap();
    assert_eq!(set_aside(&launcher, &leftover), Ok(None));
    assert_eq!(fs::read(leftover.join("accounts.json")).unwrap(), b"[]");
    remove_leftover(&leftover);
    assert!(leftover.join("accounts.json").is_file());
    let _ = fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn a_broken_link_for_the_launcher_folder_stops_before_copying() {
    let Some(&(old_launcher, old_game, _)) = crate::identity::EARLIER.first() else {
        return;
    };
    let (data, config, base) = scratch("broken-link");
    fs::create_dir_all(data.join(old_launcher)).unwrap();
    fs::write(data.join(old_launcher).join("settings.json"), b"{}").unwrap();
    // A copy would stop on this link with an error: returning early never reads it.
    fs::create_dir_all(base.join("real-game")).unwrap();
    std::os::unix::fs::symlink(base.join("real-game"), data.join(old_game)).unwrap();
    let launcher = crate::identity::launcher_folder_in(&data);
    fs::create_dir_all(launcher.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(base.join("gone"), &launcher).unwrap();
    assert_eq!(carry_over(&data, &config), Ok(false));
    assert!(!launcher.with_file_name(".carry-over").exists());
    assert!(fs::symlink_metadata(&launcher)
        .unwrap()
        .file_type()
        .is_symlink());
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn a_launcher_folder_with_the_players_files_is_never_replaced() {
    let Some(&(old_launcher, _, _)) = crate::identity::EARLIER.first() else {
        return;
    };
    let (data, config, base) = scratch("in-use");
    fs::create_dir_all(data.join(old_launcher)).unwrap();
    fs::write(data.join(old_launcher).join("settings.json"), b"{}").unwrap();
    let launcher = crate::identity::launcher_folder_in(&data);
    fs::create_dir_all(&launcher).unwrap();
    fs::write(launcher.join("accounts.json"), b"[]").unwrap();
    assert_eq!(carry_over(&data, &config), Ok(false));
    assert_eq!(fs::read(launcher.join("accounts.json")).unwrap(), b"[]");
    assert!(!launcher.join("settings.json").exists());
    let _ = fs::remove_dir_all(&base);
}

#[cfg(windows)]
fn acl_script(path: &Path, script: &str) -> (bool, String) {
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(format!("$ErrorActionPreference = 'Stop'; {script}"))
        .env("SUNRISE_CARRY_ACL_PATH", path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "ACL fixture for {path:?}: {output:?}"
    );
    let sddl = String::from_utf8(output.stdout).unwrap();
    let sddl = sddl.trim();
    assert!(sddl.starts_with("D:"), "expected DACL SDDL, got {sddl:?}");
    // Compare protection and every ACE, but not the auto-inheritance bookkeeping flags.
    let (flags, rules) = sddl.split_at(sddl.find('(').unwrap_or(sddl.len()));
    (flags.contains('P'), rules.to_owned())
}

#[cfg(windows)]
fn access_rules(path: &Path) -> (bool, String) {
    acl_script(
        path,
        "(Get-Acl -LiteralPath $env:SUNRISE_CARRY_ACL_PATH).GetSecurityDescriptorSddlForm('Access')",
    )
}

#[cfg(windows)]
#[test]
fn a_restricted_earlier_game_folder_keeps_its_access_rules() {
    let Some(&(old_launcher, old_game, _)) = crate::identity::EARLIER.first() else {
        return;
    };
    let (data, config, base) = scratch("restricted-acl");
    let from_game = data.join(old_game);
    let default_sibling = data.join(old_launcher);
    fs::create_dir_all(&default_sibling).unwrap();
    fs::create_dir(&from_game).unwrap();
    // Replace the whole DACL: protected, with one inheritable grant to the current SID.
    let restricted = acl_script(
        &from_game,
        "$sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value; \
         if ($sid -in 'S-1-5-18', 'S-1-5-32-544') { throw 'Run the ACL fixture as a user account'; }; \
         $sddl = 'D:P(A;OICI;FA;;;' + $sid + ')'; \
         $acl = Get-Acl -LiteralPath $env:SUNRISE_CARRY_ACL_PATH; \
         $acl.SetSecurityDescriptorSddlForm($sddl, 'Access'); \
         Set-Acl -LiteralPath $env:SUNRISE_CARRY_ACL_PATH -AclObject $acl; \
         [System.Security.AccessControl.RawSecurityDescriptor]::new($sddl).GetSddlForm('Access')",
    );
    assert!(restricted.0, "fixture DACL must be protected");
    assert_eq!(access_rules(&from_game), restricted, "restricted source");
    assert_ne!(
        access_rules(&default_sibling).1,
        restricted.1,
        "source ACEs must differ from the default ACL"
    );
    fs::create_dir(from_game.join("saves")).unwrap();
    fs::write(from_game.join("saves/level.dat"), [1]).unwrap();
    fs::create_dir(from_game.join(".horizon-cache")).unwrap();
    let stage = data.join("acl-probe");
    let mut copied = 0;
    data_location::copy_tree_except(&from_game, &stage, &[], &GAME_CACHES, &mut |_| {
        copied += 1;
        assert_eq!(access_rules(&stage), restricted, "during the copy");
    })
    .unwrap();
    assert_eq!(copied, 1);

    assert_eq!(carry_over(&data, &config), Ok(true));
    let game = crate::identity::game_folder_in(&data);
    assert_eq!(access_rules(&game), restricted, "after carry-over rename");
    let _ = fs::remove_dir_all(&base);
}
