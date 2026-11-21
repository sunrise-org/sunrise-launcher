//! A Sunrise build's first start. The launcher before it (Sunrise's own, then the arismoko
//! fork's: `identity::EARLIER`) left the player's settings, accounts and instances; they are
//! copied into Sunrise's own folders, never moved, so the player keeps them and the earlier
//! launcher keeps working. The earlier launcher's folders are only read.
use crate::data_location::{self, Roots};
use std::fs;
use std::path::Path;

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

/// The file that says the stage holds the whole copy, written before the first rename.
const COMPLETE: &str = "complete";

/// Runs [`carry_over`] on the player's folders before anything reads them, unless a home
/// override or a moved Sunrise location decides where the data is. A failure is reported and
/// leaves the launcher to start empty: the earlier launcher's data is untouched either way.
pub fn at_start() {
    let overridden = ["SUNRISE_LAUNCHER_HOME", "SUNRISE_HOME"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|v| !v.is_empty()));
    if crate::identity::EARLIER.is_empty()
        || overridden
        || !matches!(data_location::configured_roots(), Ok(None))
    {
        return;
    }
    let (Some(data), Some(config)) = (dirs::data_dir(), dirs::config_dir()) else {
        return;
    };
    if let Err(error) = carry_over(&data, &config) {
        eprintln!("Could not copy the earlier launcher's data: {error}");
    }
}

/// Copies the data of the first earlier launcher that has any into this launcher's folders under
/// `data`, when neither this launcher's data folder nor its game folder exists yet. Earlier
/// launchers' folders are looked for under `data`, or where their `location.json` under `config`
/// moved them. Returns whether anything was copied.
///
/// Everything is copied into a folder beside the targets first and renamed into place at the
/// end, so a start that stops halfway copies again, from the beginning, next time.
pub fn carry_over(data: &Path, config: &Path) -> Result<bool, String> {
    let launcher = crate::identity::launcher_folder_in(data);
    let game = crate::identity::game_folder_in(data);
    let stage = launcher.with_file_name(".carry-over");
    if launcher.exists() || game.exists() {
        // A copy that was finished but stopped between its two renames: the one folder that is
        // still in the stage goes where the game reads it.
        if stage.join(COMPLETE).is_file() || first_rename_done(&stage, &launcher, &game) {
            return resume(&stage, &launcher, &game);
        }
        // A copy that reached both targets but could not remove its stage (a file appearing in
        // it mid-delete) left it beside them; nothing reads it, so it goes on a later start.
        if launcher.exists() && game.exists() && stage.exists() {
            settle(&stage);
        }
        return Ok(false);
    }
    let Some(source) = crate::identity::EARLIER
        .iter()
        .find_map(|&(launcher, game, location)| {
            earlier_roots(data, config, launcher, game, location)
        })
    else {
        return Ok(false);
    };
    if stage.exists() {
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
    data_location::copy_tree(&source.game, &staged.game, &[], &mut quiet)?;
    let target = Roots {
        launcher: launcher.clone(),
        game: game.clone(),
        build: launcher.clone(),
    };
    crate::relocation::prepare_paths(&staged, &source, &target)?;
    // From here the stage holds the whole copy: a start that finds it with one target in place
    // finishes the renames (`resume`) instead of leaving the other folder in it.
    // The marker is what a later start trusts: it must be on disk before a rename is.
    let marker = stage.join(COMPLETE);
    fs::File::create(&marker)
        .and_then(|file| file.sync_all())
        .map_err(|e| format!("{}: {e}", marker.display()))?;
    data_location::sync_dir(&stage)?;
    fs::rename(&staged.launcher, &launcher).map_err(|e| format!("{}: {e}", launcher.display()))?;
    fs::rename(&staged.game, &game).map_err(|e| format!("{}: {e}", game.display()))?;
    discard(&stage);
    Ok(true)
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
fn resume(stage: &Path, launcher: &Path, game: &Path) -> Result<bool, String> {
    let mut moved = false;
    for (part, target) in [("launcher", launcher), ("game", game)] {
        let from = stage.join(part);
        if !target.exists() && from.is_dir() {
            fs::rename(&from, target).map_err(|e| format!("{}: {e}", target.display()))?;
            moved = true;
        }
    }
    if launcher.exists() && game.exists() {
        settle(stage);
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
        Ok(()) => eprintln!(
            "Kept {} ({}): the data folder it was copied for already existed, so it was not replaced.",
            kept.display(),
            left.join(", ")
        ),
        Err(error) => eprintln!("Could not keep {}: {error}", stage.display()),
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
    }
}

/// An earlier launcher's folders, when its launcher or game folder exists: where its
/// `location.json` moved them, else its default folders under `data`.
fn earlier_roots(
    data: &Path,
    config: &Path,
    launcher: &str,
    game: &str,
    location: &str,
) -> Option<Roots> {
    let moved = data_location::resolve_at(&config.join(location).join("location.json"))
        .ok()
        .flatten();
    let roots = moved.unwrap_or_else(|| Roots {
        launcher: data.join(launcher),
        game: data.join(game),
        build: data.join(launcher),
    });
    (roots.launcher.is_dir() || roots.game.is_dir()).then_some(roots)
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
    fn a_copy_stopped_before_either_rename_copies_again_and_the_saves_end_in_place() {
        let Some(&(old_launcher, old_game, _)) = crate::identity::EARLIER.first() else {
            return;
        };
        let (data, config, base) = scratch("again");
        let (stage, launcher, game) = finished_stage(&data);
        // The first rename failed: nothing is in place, the stage is complete.
        let from_game = data.join(old_game);
        fs::create_dir_all(from_game.join("saves/World")).unwrap();
        fs::write(from_game.join("saves/World/level.dat"), [4, 5]).unwrap();
        fs::create_dir_all(data.join(old_launcher)).unwrap();
        assert_eq!(carry_over(&data, &config), Ok(true));
        assert_eq!(
            fs::read(game.join("saves/World/level.dat")).unwrap(),
            [4, 5]
        );
        assert!(launcher.is_dir() && !stage.exists());
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
}
