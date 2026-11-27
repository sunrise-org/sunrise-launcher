//! A Sunrise build's first start. The launcher before it (Sunrise's own, then the arismoko
//! fork's: `identity::EARLIER`) left the player's settings, accounts and instances; they are
//! copied into Sunrise's own folders, never moved, so the player keeps them and the earlier
//! launcher keeps working. The earlier launcher's folders are only read.
use crate::data_location::{self, Roots};
use std::fs;
use std::path::Path;
use std::sync::Mutex;

static STARTUP_ERROR: Mutex<Option<String>> = Mutex::new(None);

/// The error from [`at_start`], if carrying over an earlier launcher's data failed.
pub fn startup_error() -> Option<String> {
    STARTUP_ERROR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// What a launcher folder holds that is the player's: settings, accounts (with the secrets file
/// of a computer without a keychain), the instances' names and icons, the servers made in the
/// launcher. Game builds, downloads, caches and logs are made again.
const LAUNCHER_STATE: [&str; 7] = [
    "settings.json",
    "accounts.json",
    crate::account::SECRETS_FILE,
    "account-secrets.key",
    "instances.json",
    "servers.json",
    "servers",
];

/// What a game folder holds that the game makes again: its content cache (under the earlier
/// launcher's name or this one's), compiled scripts and downloads. Not copied: a Horizon game
/// folder's `.horizon-cache` is most of its files, and its entries are links into the earlier
/// launcher's game builds, which a copy refuses.
const GAME_CACHES: [&str; 4] = [".horizon-cache", ".sunrise-cache", "wasm-cache", "cache"];

/// The file that says the stage holds the whole copy, written before the first rename.
const COMPLETE: &str = "complete";

/// All a start whose carry-over failed makes in the launcher folder: the instances list's lock
/// and the caches. A launcher folder holding anything else is the player's and is never replaced,
/// even something this list doesn't know (a theme, a saved skin, a publisher pin).
const LEFT_BY_A_FAILED_START: [&str; 2] = [".instances.lock", "cache"];

/// Where a launcher folder holding only [`LEFT_BY_A_FAILED_START`] goes while the copy takes
/// its place.
const LEFTOVER: &str = ".carry-over-leftover";

/// Runs [`carry_over`] on the player's folders before anything reads them, unless a home
/// override or a moved Sunrise location decides where the data is. A failure is remembered in
/// [`startup_error`] and reported to stderr so startup skips creating an empty root and the
/// player can repair the earlier location and restart to retry.
pub fn at_start() -> Result<(), String> {
    *STARTUP_ERROR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    let overridden = ["SUNRISE_LAUNCHER_HOME", "SUNRISE_HOME"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|v| !v.is_empty()));
    if crate::identity::EARLIER.is_empty()
        || overridden
        || !matches!(data_location::configured_roots(), Ok(None))
    {
        return Ok(());
    }
    let (Some(data), Some(config)) = (dirs::data_dir(), dirs::config_dir()) else {
        return Ok(());
    };
    if let Err(error) = carry_over(&data, &config) {
        log_startup_failure(&error);
        *STARTUP_ERROR
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(error.clone());
        return Err(error);
    }
    Ok(())
}

/// Reports a startup carry-over failure locally on stderr and records only a path-free summary
/// in the launcher log (carry-over runs before `launcher_log::init` registers private roots).
fn log_startup_failure(error: &str) {
    eprintln!("Could not copy the earlier launcher's data: {error}");
    crate::launcher_log::line(format!(
        "Could not copy the earlier launcher's data: {}",
        crate::launcher_log::safe_error(error)
    ));
}

/// Copies the data of the first earlier launcher that has any into this launcher's folders under
/// `data`, when neither this launcher's data folder nor its game folder exists yet. Earlier
/// launchers' folders are looked for under `data`, or where their `location.json` under `config`
/// moved them. Returns whether anything was copied.
///
/// Everything is copied into a stage beside the targets before publication. A later start
/// resumes a finished stage and copies an unfinished one again from the beginning.
pub fn carry_over(data: &Path, config: &Path) -> Result<bool, String> {
    let launcher = crate::identity::launcher_folder_in(data);
    let game = crate::identity::game_folder_in(data);
    let stage = launcher.with_file_name(".carry-over");
    let leftover = launcher.with_file_name(LEFTOVER);
    // Lock-free fast paths so steady-state launches and clean installs with no earlier launcher
    // never leave a `.carry-over.lock` dotfile behind. Check `game`/`launcher_in_use` before
    // `!present(&stage)` so an in-progress publish that already renamed one target still takes
    // the lock and rechecks everything underneath it.
    if (present(&game)? || launcher_in_use(&launcher)?) && !present(&stage)? {
        return Ok(false);
    }
    if !present(&stage)? && find_earlier_roots(data, config)?.is_none() {
        return Ok(false);
    }
    // Shared startup leases allow two first-run launchers to enter carry-over together.
    // Serialize the shared staging directory across read, copy, resume, publish, and cleanup.
    let _lock =
        data_location::lock_exclusive_blocking(&launcher.with_file_name(".carry-over.lock"))?;
    if refuse_link(&stage)?.is_some() {
        let marker = refuse_link(&stage.join(COMPLETE))?;
        refuse_link(&stage.join("launcher"))?;
        refuse_link(&stage.join("game"))?;
        if marker.is_some_and(|m| m.is_file()) || first_rename_done(&stage, &launcher, &game) {
            return resume(&stage, &launcher, &game, &leftover);
        }
        // A finished copy may have left an unmarked stage it couldn't remove.
        if launcher.exists() && game.exists() {
            settle(&stage);
        }
    }
    if present(&game)? || launcher_in_use(&launcher)? {
        return Ok(false);
    }
    let Some(source) = find_earlier_roots(data, config)? else {
        return Ok(false);
    };
    if refuse_link(&stage)?.is_some() {
        fs::remove_dir_all(&stage).map_err(|e| format!("{}: {e}", stage.display()))?;
    }
    let staged = Roots {
        launcher: stage.join("launcher"),
        game: stage.join("game"),
        build: stage.join("launcher"),
    };
    fs::create_dir_all(&staged.launcher).map_err(|e| format!("{}: {e}", stage.display()))?;
    let mut quiet = |_: &str| {};
    for name in LAUNCHER_STATE {
        let from = source.launcher.join(name);
        if from.exists() {
            data_location::copy_tree(&from, &staged.launcher.join(name), &[], &mut quiet)?;
        }
    }
    data_location::copy_tree_except(&source.game, &staged.game, &[], &GAME_CACHES, &mut quiet)?;
    let target = Roots {
        launcher: launcher.clone(),
        game: game.clone(),
        build: launcher.clone(),
    };
    crate::relocation::prepare_paths(&staged, &source, &target)?;
    // What a failed start left in the launcher folder goes aside before the marker, so a stage
    // marked complete never meets a launcher folder it would have to step around. Another window
    // can write the player's files there while the copy runs: then the copy is dropped.
    let set_aside = match set_aside(&launcher, &leftover)? {
        Some(moved) => moved,
        None => {
            discard(&stage);
            return Ok(false);
        }
    };
    // From here the stage holds the whole copy: a later start finishes the renames (`resume`)
    // even if the original folders are no longer available.
    // The marker is what a later start trusts: it must be on disk before a rename is.
    let marker = stage.join(COMPLETE);
    fs::File::create(&marker)
        .and_then(|file| file.sync_all())
        .map_err(|e| format!("{}: {e}", marker.display()))?;
    data_location::sync_dir(&stage)?;
    fs::rename(&staged.launcher, &launcher).map_err(|e| format!("{}: {e}", launcher.display()))?;
    fs::rename(&staged.game, &game).map_err(|e| format!("{}: {e}", game.display()))?;
    discard(&stage);
    if set_aside {
        remove_leftover(&leftover);
    }
    Ok(true)
}

/// Whether anything is at `path`, a broken link included.
fn present(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Whether `launcher` holds anything but failed-start leftovers or logs made by earlier versions.
/// A link there, even a broken one, is treated as player data.
fn launcher_in_use(launcher: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(launcher) {
        Ok(meta) if crate::native_cache::linked(&meta) || !meta.is_dir() => return Ok(true),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("{}: {e}", launcher.display())),
    }
    let entries = fs::read_dir(launcher).map_err(|e| format!("{}: {e}", launcher.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", launcher.display()))?;
        let name = entry.file_name();
        if name == "logs" {
            let path = entry.path();
            let meta =
                fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if !meta.is_dir() || crate::native_cache::linked(&meta) {
                return Ok(true);
            }
            let logs = fs::read_dir(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            for log in logs {
                let log = log.map_err(|e| format!("{}: {e}", path.display()))?;
                let log_path = log.path();
                let meta = fs::symlink_metadata(&log_path)
                    .map_err(|e| format!("{}: {e}", log_path.display()))?;
                if !meta.is_file()
                    || crate::native_cache::linked(&meta)
                    || !crate::launcher_log::is_rotation_file(&log.file_name())
                {
                    return Ok(true);
                }
            }
        } else if !LEFT_BY_A_FAILED_START
            .iter()
            .any(|allowed| name == *allowed)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Moves a launcher folder that holds only what a failed start left to `leftover`, so the copy
/// can take its place. `Some(true)` when it was moved, `Some(false)` when there was none, and
/// `None` when it is the player's: then it stays where it is. It is checked again once moved, so
/// a file written into it just before the move puts it back.
fn set_aside(launcher: &Path, leftover: &Path) -> Result<Option<bool>, String> {
    if !present(launcher)? {
        return Ok(Some(false));
    }
    if launcher_in_use(launcher)? {
        return Ok(None);
    }
    if present(leftover)? {
        // Only ever what an earlier start set aside; anything else there is left alone.
        if launcher_in_use(leftover)? {
            return Ok(None);
        }
        fs::remove_dir_all(leftover).map_err(|e| format!("{}: {e}", leftover.display()))?;
    }
    fs::rename(launcher, leftover).map_err(|e| format!("{}: {e}", launcher.display()))?;
    if launcher_in_use(leftover)? {
        fs::rename(leftover, launcher).map_err(|e| format!("{}: {e}", launcher.display()))?;
        return Ok(None);
    }
    Ok(Some(true))
}

/// Removes what [`set_aside`] moved, once the copy is in its place. It is checked again first,
/// and a failure leaves it for a later start.
fn remove_leftover(leftover: &Path) {
    match launcher_in_use(leftover) {
        Ok(false) => {
            if let Err(error) = fs::remove_dir_all(leftover) {
                eprintln!("Could not remove {}: {error}", leftover.display());
                crate::launcher_log::line(format!(
                    "Could not remove leftover carry-over folder: {}",
                    crate::launcher_log::safe_error(&error.to_string())
                ));
            }
        }
        Ok(true) => {
            eprintln!(
                "Kept {}: something besides caches was written to it.",
                leftover.display()
            );
            crate::launcher_log::line(
                "Kept leftover carry-over folder: something besides caches was written to it.",
            );
        }
        Err(error) => {
            eprintln!("{error}");
            crate::launcher_log::line(format!(
                "Could not inspect leftover carry-over folder: {}",
                crate::launcher_log::safe_error(&error)
            ));
        }
    }
}

/// Whether an unmarked stage (one a launcher before the marker, 1.0.0 or 1.0.1, left) is provably
/// a finished copy. Those launchers copied everything, and made `launcher/` in the stage first,
/// before renaming it into place and then `game/`: so a stage without `launcher/` but with
/// `game/`, beside a launcher folder and no game folder, stopped between the two renames.
/// Any other unmarked stage may be half a copy.
fn first_rename_done(stage: &Path, launcher: &Path, game: &Path) -> bool {
    launcher.is_dir()
        && !game.exists()
        && !stage.join("launcher").exists()
        && stage.join("game").is_dir()
}

/// Finishes a copy that stopped between its two renames: each folder still in the stage whose
/// target does not exist is renamed into place. A target that exists is never replaced, so saves
/// already there stay as they are, and what is left in the stage is removed once both are there.
fn resume(stage: &Path, launcher: &Path, game: &Path, leftover: &Path) -> Result<bool, String> {
    let mut moved = false;
    for (part, target) in [("launcher", launcher), ("game", game)] {
        let from = stage.join(part);
        if !from.is_dir() {
            continue;
        }
        // A launcher folder that a failed start made after the copy was marked complete holds
        // nothing of the player's: the copy goes in its place, not into a kept stage.
        if part == "launcher" {
            set_aside(target, leftover)?;
        }
        if !present(target)? {
            fs::rename(&from, target).map_err(|e| format!("{}: {e}", target.display()))?;
            moved = true;
        }
    }
    if launcher.exists() && game.exists() {
        settle(stage);
        if leftover.exists() {
            remove_leftover(leftover);
        }
    }
    Ok(moved)
}

/// Ends a stage whose copy is over. When something in it did not go into place (its target
/// already existed), the stage is kept as `.carry-over-kept-<UTC time>` beside it, with one line
/// saying where; only a stage with nothing left in it is removed.
fn settle(stage: &Path) {
    let left: Vec<&str> = ["launcher", "game"]
        .into_iter()
        .filter(|part| stage.join(part).is_dir())
        .collect();
    if left.is_empty() {
        return discard(stage);
    }
    let kept = stage.with_file_name(format!(
        ".carry-over-kept-{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    ));
    match fs::rename(stage, &kept) {
        Ok(()) => {
            eprintln!(
                "Kept {} ({}): the data folder it was copied for already existed, so it was not replaced.",
                kept.display(),
                left.join(", ")
            );
            crate::launcher_log::line(format!(
                "Kept carry-over stage ({}): the data folder it was copied for already existed, so it was not replaced.",
                left.join(", ")
            ));
        }
        Err(error) => {
            eprintln!("Could not keep {}: {error}", stage.display());
            crate::launcher_log::line(format!(
                "Could not keep carry-over stage: {}",
                crate::launcher_log::safe_error(&error.to_string())
            ));
        }
    }
}

/// Removes the stage of a finished copy. A failure is reported and leaves it for the next start.
fn discard(stage: &Path) {
    if let Err(error) = fs::remove_dir_all(stage) {
        eprintln!(
            "{}",
            crate::i18n::trf(
                "Could not remove {path}, left from copying the earlier launcher's data; the next start tries again: {error}",
                &[
                    ("path", &stage.display().to_string()),
                    ("error", &error.to_string()),
                ],
            )
        );
        crate::launcher_log::line(format!(
            "Could not remove carry-over stage; the next start tries again: {}",
            crate::launcher_log::safe_error(&error.to_string())
        ));
    }
}

/// Returns `path`'s own metadata (without following links), refusing symlinks and junctions.
fn refuse_link(path: &Path) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if crate::native_cache::linked(&meta) => Err(crate::i18n::trf(
            "Symbolic links and reparse points cannot be relocated: {path}",
            &[("path", &path.display().to_string())],
        )),
        Ok(meta) => Ok(Some(meta)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn find_earlier_roots(data: &Path, config: &Path) -> Result<Option<Roots>, String> {
    for &(old_launcher, old_game, location) in crate::identity::EARLIER {
        if let Some(roots) = earlier_roots(data, config, old_launcher, old_game, location)? {
            return Ok(Some(roots));
        }
    }
    Ok(None)
}

/// An earlier launcher's folders, when its launcher or game folder exists: where its
/// `location.json` moved them, else its default folders under `data`.
fn earlier_roots(
    data: &Path,
    config: &Path,
    launcher: &str,
    game: &str,
    location: &str,
) -> Result<Option<Roots>, String> {
    let moved = data_location::resolve_at(&config.join(location).join("location.json"))?;
    let roots = moved.unwrap_or_else(|| Roots {
        launcher: data.join(launcher),
        game: data.join(game),
        build: data.join(launcher),
    });
    Ok((roots.launcher.is_dir() || roots.game.is_dir()).then_some(roots))
}

#[cfg(test)]
mod tests {
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
            serde_json::from_str(&fs::read_to_string(launcher.join("settings.json")).unwrap())
                .unwrap();
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
    fn launcher_logs_do_not_block_carry_over_or_resume() {
        let &(old_launcher, _, _) = crate::identity::EARLIER.first().unwrap();
        for staged in [false, true] {
            let (data, config, base) = scratch("launcher-logs");
            let launcher = crate::identity::launcher_folder_in(&data);
            if staged {
                finished_stage(&data);
            } else {
                fs::create_dir_all(data.join(old_launcher)).unwrap();
                fs::write(data.join(old_launcher).join("accounts.json"), b"{}").unwrap();
            }
            fs::create_dir_all(launcher.join("logs")).unwrap();
            fs::write(launcher.join("logs/launcher.log"), b"old header").unwrap();
            fs::write(launcher.join("logs/launcher.1.log"), b"rotated").unwrap();
            assert_eq!(carry_over(&data, &config), Ok(true), "staged: {staged}");
            assert_eq!(fs::read(launcher.join("accounts.json")).unwrap(), b"{}");
            assert!(!launcher.join("logs").exists());
            let _ = fs::remove_dir_all(&base);
        }
    }

    #[test]
    fn player_entries_named_or_inside_logs_block_carry_over() {
        let &(old_launcher, _, _) = crate::identity::EARLIER.first().unwrap();
        for kept in ["logs", "logs/player-note.txt", "logs/launcher.log/note"] {
            let (data, config, base) = scratch("player-logs");
            fs::create_dir_all(data.join(old_launcher)).unwrap();
            fs::write(data.join(old_launcher).join("accounts.json"), b"{}").unwrap();
            let launcher = crate::identity::launcher_folder_in(&data);
            let path = launcher.join(kept);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"mine").unwrap();
            assert_eq!(carry_over(&data, &config), Ok(false), "{kept}");
            assert_eq!(fs::read(path).unwrap(), b"mine");
            assert!(!launcher.join("accounts.json").exists());
            let _ = fs::remove_dir_all(&base);
        }
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn a_link_named_logs_blocks_carry_over() {
        let &(old_launcher, _, _) = crate::identity::EARLIER.first().unwrap();
        let (data, config, base) = scratch("linked-logs");
        fs::create_dir_all(data.join(old_launcher)).unwrap();
        fs::write(data.join(old_launcher).join("accounts.json"), b"{}").unwrap();
        let launcher = crate::identity::launcher_folder_in(&data);
        let real = base.join("real-logs");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("launcher.log"), b"mine").unwrap();
        fs::create_dir_all(&launcher).unwrap();
        let link = launcher.join("logs");
        link_dir(&real, &link);
        assert_eq!(carry_over(&data, &config), Ok(false));
        assert_eq!(fs::read(link.join("launcher.log")).unwrap(), b"mine");
        assert!(!launcher.join("accounts.json").exists());
        unlink_dir(&link);
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

    #[test]
    fn concurrent_carry_over_starts_serialize_and_publish_once() {
        let Some(&(old_launcher, old_game, _)) = crate::identity::EARLIER.first() else {
            return;
        };
        let (data, config, base) = scratch("concurrent");
        let from_launcher = data.join(old_launcher);
        let from_game = data.join(old_game);
        fs::create_dir_all(&from_launcher).unwrap();
        fs::write(from_launcher.join("accounts.json"), r#"{"accounts":[]}"#).unwrap();
        fs::write(from_launcher.join("settings.json"), r#"{"language":"en"}"#).unwrap();
        fs::create_dir_all(from_game.join("saves/World")).unwrap();
        for index in 0..32 {
            fs::write(
                from_game.join(format!("saves/World/chunk-{index:02}.dat")),
                vec![index as u8; 4096],
            )
            .unwrap();
        }
        let expected_game = files(&from_game);

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let barrier = barrier.clone();
            let data = data.clone();
            let config = config.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                carry_over(&data, &config)
            }));
        }
        let mut results: Vec<bool> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap().unwrap())
            .collect();
        results.sort();
        assert_eq!(results, vec![false, true]);

        let launcher = crate::identity::launcher_folder_in(&data);
        let game = crate::identity::game_folder_in(&data);
        assert_eq!(
            fs::read_to_string(launcher.join("accounts.json")).unwrap(),
            r#"{"accounts":[]}"#
        );
        assert_eq!(
            fs::read_to_string(launcher.join("settings.json")).unwrap(),
            "{\n  \"language\": \"en\"\n}"
        );
        assert_eq!(files(&game), expected_game);
        assert!(!launcher.with_file_name(".carry-over").exists());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn steady_state_and_clean_starts_do_not_create_carry_over_lock() {
        let (data, config, base) = scratch("no-lock-fast-path");
        fs::create_dir_all(&data).unwrap();
        let launcher = crate::identity::launcher_folder_in(&data);
        let lock = launcher.with_file_name(".carry-over.lock");
        assert_eq!(carry_over(&data, &config), Ok(false));
        assert!(!lock.exists(), "clean install should not create {lock:?}");

        if let Some(&(old_launcher, _, _)) = crate::identity::EARLIER.first() {
            fs::create_dir_all(data.join(old_launcher)).unwrap();
            let game = crate::identity::game_folder_in(&data);
            fs::create_dir_all(&game).unwrap();
            assert_eq!(carry_over(&data, &config), Ok(false));
            assert!(
                !lock.exists(),
                "steady-state start should not create {lock:?}"
            );
        }
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn startup_failure_and_stage_cleanup_logs_omit_private_paths() {
        let base = std::env::temp_dir().join(format!(
            "hz-private-carry-over-secret-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        let stage = base.join(".carry-over");
        let leftover = base.join(".carry-over-leftover");
        fs::create_dir_all(stage.join("launcher")).unwrap();
        fs::create_dir_all(&leftover).unwrap();
        fs::write(leftover.join("player-note.txt"), "keep").unwrap();

        let (before, _) = crate::launcher_log::since(u64::MAX);
        log_startup_failure(
            "/mnt/private-nas/SecretClient/earlier: No such file or directory (os error 2)",
        );
        settle(&stage);
        remove_leftover(&leftover);

        let (_, lines) = crate::launcher_log::since(before);
        let expected_error = format!(
            "Could not copy the earlier launcher's data: {} (os error 2)",
            std::io::Error::from_raw_os_error(2).kind()
        );
        assert!(
            lines.iter().any(|line| line.contains(&expected_error)),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Kept carry-over stage (launcher):")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Kept leftover carry-over folder:")),
            "{lines:?}"
        );
        for secret in ["private-nas", "SecretClient", "private-carry-over-secret"] {
            assert!(
                lines.iter().all(|line| !line.contains(secret)),
                "{secret} leaked into launcher log: {lines:?}"
            );
        }
        let _ = fs::remove_dir_all(&base);
    }
}
