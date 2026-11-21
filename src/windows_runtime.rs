//! Keep the game's Windows executable path stable across installed versions.
//! A slot belongs to one game home; its lock is held until the child exits.
use sha2::{Digest, Sha256};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::{fs, io, io::Write, path::Path, process::Command};

use crate::run_leases::SOURCE;
pub(crate) const SLOT: &str = "SUNRISE_WINDOWS_RUNTIME_SLOT";

pub(crate) struct Guard {
    slot: fs::File,
    source: fs::File,
    game: std::path::PathBuf,
}

impl Guard {
    pub(crate) fn files(&self) -> [&fs::File; 2] {
        [&self.slot, &self.source]
    }

    pub(crate) fn game(&self) -> &Path {
        &self.game
    }
}

fn value<'a>(cmd: &'a Command, key: &str) -> Option<&'a std::ffi::OsStr> {
    cmd.get_envs().find(|(k, _)| *k == key).and_then(|(_, v)| v)
}

/// Called before adding a pre-launch command or wrapper, after applying the instance.
pub(crate) fn redirect(cmd: Command) -> Command {
    redirect_in(cmd, &crate::backend::data_dir().join("run"))
}

fn redirect_in(cmd: Command, root: &Path) -> Command {
    if value(&cmd, SOURCE).is_none() || value(&cmd, SLOT).is_some() {
        return cmd;
    }
    let home = value(&cmd, "SUNRISE_HOME")
        .map(Path::new)
        .or(cmd.get_current_dir())
        .expect("the game command has a home");
    let slot = slot_for_home(root, home);
    let mut out = Command::new(slot.join("game").join(crate::backend::CLIENT));
    out.args(cmd.get_args());
    if let Some(dir) = cmd.get_current_dir() {
        out.current_dir(dir);
    }
    for (key, value) in cmd.get_envs() {
        match value {
            Some(value) => out.env(key, value),
            None => out.env_remove(key),
        };
    }
    out.env(SLOT, slot);
    out
}

fn slot_for_home(root: &Path, home: &Path) -> std::path::PathBuf {
    let home = fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    root.join(format!(
        "{:x}",
        Sha256::digest(home.as_os_str().as_encoded_bytes())
    ))
}

/// Check the staged executable against the original dedicated command while its slot is held.
pub(crate) fn verify_dedicated(expected: &Command, actual: &Command) -> io::Result<()> {
    use sunrise_launcher_smod::index::sha256_file;

    let invalid = || io::Error::from(io::ErrorKind::InvalidInput);
    let data = crate::backend::data_dir();
    let slot = slot_for_home(
        &data.join("run"),
        expected.get_current_dir().expect("dedicated server cwd"),
    );
    let program = slot.join("game").join(crate::backend::CLIENT);
    if actual.get_program() != program.as_os_str()
        || value(actual, SLOT) != Some(slot.as_os_str())
        || actual
            .get_envs()
            .filter(|(name, _)| *name != SLOT)
            .ne(expected.get_envs())
    {
        return Err(invalid());
    }
    let trusted_slot = data
        .canonicalize()?
        .join("run")
        .join(slot.file_name().unwrap());
    if program.canonicalize()? != trusted_slot.join("game").join(crate::backend::CLIENT)
        || fs::symlink_metadata(&program)?.file_attributes() & 0x400 != 0
        || sha256_file(&program).map_err(io::Error::other)?
            != sha256_file(Path::new(expected.get_program())).map_err(io::Error::other)?
    {
        return Err(invalid());
    }
    Ok(())
}

/// Prepare under an exclusive lock. Never fall back to a version-specific executable:
/// a locked or incomplete slot is an error, rather than another firewall identity.
pub(crate) fn prepare(cmd: &Command) -> io::Result<Option<Guard>> {
    prepare_in(cmd, &crate::backend::data_dir().join("run"))
}

fn prepare_in(cmd: &Command, root: &Path) -> io::Result<Option<Guard>> {
    let (Some(source), Some(slot)) = (value(cmd, SOURCE), value(cmd, SLOT)) else {
        return Ok(None);
    };
    let slot = Path::new(slot);
    let valid_key = slot
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.len() == 64 && n.bytes().all(|b| b.is_ascii_hexdigit()));
    if slot.parent() != Some(root) || !valid_key {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    plain_directory(root)?;
    stage(Path::new(source), slot).map(Some)
}

fn plain_directory(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    // A replaced runtime slot or game folder must never redirect cleanup to player data.
    if fs::symlink_metadata(path)?.file_attributes() & 0x400 != 0 {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    Ok(())
}

fn stage(source: &Path, slot: &Path) -> io::Result<Guard> {
    plain_directory(slot)?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        // Exclusive sharing follows the file object through DuplicateHandle. The creator's
        // exit, unlike a Win32 byte-range lock, does not release the child's exclusion.
        .share_mode(0)
        .open(slot.join("run.lock"))?;
    // Retain the original exe's protection against deletion/rebuild while playing.
    let source_file = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(source.join(crate::backend::CLIENT))?;
    let game = slot.join("game");
    plain_directory(&game)?;
    let runtime = game.join(crate::backend::CLIENT);
    match fs::symlink_metadata(&runtime) {
        Ok(meta) if meta.is_file() && !crate::native_cache::linked(&meta) => (),
        Ok(_) => return Err(io::ErrorKind::InvalidInput.into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => (),
        Err(error) => return Err(error),
    }
    // Replace the directory entry, never write through an existing link. Windows refuses
    // to replace an executing image even if its launcher (and lock) has already exited.
    let staged = tempfile::NamedTempFile::new_in(&game)?.into_temp_path();
    fs::copy(source.join(crate::backend::CLIENT), &staged)?;
    staged.persist(runtime)?;
    for entry in fs::read_dir(&game)? {
        let entry = entry?;
        if entry.file_name() == crate::backend::CLIENT {
            continue;
        }
        if entry.file_type()?.is_dir() {
            fs::remove_dir_all(entry.path())?;
        } else {
            fs::remove_file(entry.path())?;
        }
    }
    // Runtime files (DLLs, native-data and native-mods.json) stay beside the exe.
    // --resources still names the installed version's own resources.
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if entry.file_name() == crate::backend::CLIENT || entry.file_name() == ".local" {
            continue;
        }
        let to = game.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            crate::backend::copy_dir(&entry.path(), &to)?;
        } else {
            fs::copy(entry.path(), to)?;
        }
    }
    let mut marker = tempfile::NamedTempFile::new_in(slot)?;
    marker.write_all(&serde_json::to_vec(&source.canonicalize()?)?)?;
    marker.persist(slot.join("source.json"))?;
    // The game runs from this copy and, finding the launcher's lease registry above it, locks
    // the copy and its folders up to the data folder: they need lease entries, as an installed
    // version's have.
    crate::run_leases::publish(&game)
        .map_err(|error| io::Error::other(crate::native_mods::cache_error(error)))?;
    Ok(Guard {
        slot: lock,
        source: source_file,
        game,
    })
}

pub(crate) fn in_use(source: &Path) -> bool {
    in_use_in(source, &crate::backend::data_dir().join("run"))
}

/// Whether a program in the version's own folder is running: `sunrise-ctl` under the launcher's
/// MCP server, `sunrise-server`, `sunrise-prepare`. They start from there, not from a runtime
/// slot, and Windows refuses both to open a running image for writing and to delete it.
fn runs_program(dir: &Path) -> bool {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| {
            let path = entry.path();
            path.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
                && fs::OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .is_err_and(|e| e.raw_os_error() == Some(ERROR_SHARING_VIOLATION))
        })
}

fn in_use_in(source: &Path, root: &Path) -> bool {
    if runs_program(source) {
        return true;
    }
    let source = fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
    let slots = match fs::read_dir(root) {
        Ok(slots) => slots,
        Err(e) => return e.kind() != io::ErrorKind::NotFound,
    };
    slots.filter_map(Result::ok).any(|slot| {
        let recorded = fs::read(slot.path().join("source.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<std::path::PathBuf>(&bytes).ok());
        if recorded.as_ref() != Some(&source) {
            return false;
        }
        let Ok(_slot) = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(slot.path().join("run.lock"))
        else {
            return true;
        };
        // If a legacy launcher exited first, Windows still protects the running image.
        fs::OpenOptions::new()
            .write(true)
            .open(slot.path().join("game").join(crate::backend::CLIENT))
            .is_err()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_running_from_the_version_folder_keeps_it_in_use() {
        let root = std::env::temp_dir().join(format!("sunrise-program-{}", std::process::id()));
        let version = root.join("version");
        fs::create_dir_all(&version).unwrap();
        let ctl = version.join("sunrise-ctl.exe");
        fs::copy(
            Path::new(&std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe"),
            &ctl,
        )
        .unwrap();
        // No game ever ran from it: no runtime slot names this version.
        let slots = root.join("run");
        assert!(!in_use_in(&version, &slots));
        let mut cmd = Command::new(&ctl);
        cmd.current_dir(&root).args(["/D", "/C", "set /p answer="]);
        crate::process::hide_console(&mut cmd);
        let mut child = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let running = child.try_wait().unwrap().is_none();
        let protected = in_use_in(&version, &slots);
        let _ = child.kill();
        child.wait().unwrap();
        assert!(running && protected);
        assert!(!in_use_in(&version, &slots));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_orphaned_process_keeps_its_runtime_and_source_version_protected() {
        let root = std::env::temp_dir().join(format!("sunrise-orphan-{}", std::process::id()));
        let source = root.join("version");
        fs::create_dir_all(&source).unwrap();
        fs::copy(
            Path::new(&std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe"),
            source.join(crate::backend::CLIENT),
        )
        .unwrap();
        let mut cmd = Command::new(source.join(crate::backend::CLIENT));
        cmd.current_dir(&root).args(["/D", "/C", "set /p answer="]);
        crate::run_leases::mark(&mut cmd, &source);
        let mut cmd = redirect_in(cmd, &root.join("run"));
        let guard = prepare_in(&cmd, &root.join("run")).unwrap().unwrap();
        crate::process::hide_console(&mut cmd);
        let mut child = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        drop(guard); // Simulate a launcher exiting before its game.
        let running = child.try_wait().unwrap().is_none();
        let protected = in_use_in(&source, &root.join("run"));
        let replacement = prepare_in(&cmd, &root.join("run"));
        let _ = child.kill();
        child.wait().unwrap();
        assert!(running && protected);
        assert!(replacement.is_err());
        assert!(!in_use_in(&source, &root.join("run")));
        drop(prepare_in(&cmd, &root.join("run")).unwrap());
        // Windows can keep the killed child's image mapped for a moment after `wait`; the
        // temporary folder is only cleanup, not part of what this test checks.
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_game_finds_every_folder_it_locks_leased_when_played_from_its_slot() {
        use sunrise_launcher_smod::runtime_leases::{open_directory, Registry};
        let data = crate::backend::data_dir();
        let name = format!("slot-lease-{}", std::process::id());
        let source = data.join("versions").join(&name);
        let home = data.join("instances").join(&name);
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&home).unwrap();
        fs::write(source.join(crate::backend::CLIENT), "game").unwrap();
        fs::write(source.join(".installed"), "").unwrap();
        crate::run_leases::publish(&source).unwrap();
        let mut cmd = Command::new(source.join(crate::backend::CLIENT));
        cmd.current_dir(&home).env("SUNRISE_HOME", &home);
        crate::run_leases::mark(&mut cmd, &source);
        // Play's own preparation: the leases, the slot and its copy of the game.
        let (cmd, _runtime) = crate::backend::prepare_command(redirect(cmd)).unwrap();
        let executable = Path::new(cmd.get_program());
        assert!(executable.starts_with(data.join("run")));
        // The lease check the game makes at start, on what it would lock.
        let selection =
            sunrise_launcher_smod::selection::LaunchArguments::for_command(&cmd).unwrap();
        let data = data.canonicalize().unwrap();
        let paths =
            sunrise_launcher_smod::holder::lease_paths(executable, &selection, &data, &home)
                .unwrap();
        let registry = Registry::existing(&data).unwrap();
        for (path, _) in paths {
            assert!(
                registry
                    .lock(
                        &open_directory(&path).unwrap(),
                        sunrise_launcher_smod::locks::LockMode::Shared
                    )
                    .is_ok(),
                "{} has no lease",
                path.display()
            );
        }
    }

    #[test]
    fn versions_reuse_a_slot_keep_native_files_and_cannot_replace_a_running_game() {
        let root = std::env::temp_dir().join(format!("sunrise-runtime-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let command = |version: &str| {
            let source = root.join(version);
            fs::create_dir_all(source.join("native-data/mod")).unwrap();
            fs::write(source.join(crate::backend::CLIENT), version).unwrap();
            fs::write(source.join("native-data/mod/table"), version).unwrap();
            let mut cmd = Command::new(source.join(crate::backend::CLIENT));
            cmd.current_dir(&home)
                .env("SUNRISE_HOME", &home)
                .arg("--offscreen");
            crate::run_leases::mark(&mut cmd, &source);
            redirect_in(cmd, &root.join("run"))
        };
        let first = command("v1");
        let lock = prepare_in(&first, &root.join("run")).unwrap().unwrap();
        assert!(in_use_in(&root.join("v1"), &root.join("run")));
        assert!(fs::remove_file(root.join("v1").join(crate::backend::CLIENT)).is_err());
        let game = Path::new(first.get_program()).parent().unwrap();
        fs::write(game.join("removed.dll"), "old").unwrap();
        let second = command("v2");
        assert_eq!(first.get_program(), second.get_program());
        assert!(prepare_in(&second, &root.join("run")).is_err());
        assert_eq!(fs::read_to_string(first.get_program()).unwrap(), "v1");
        drop(lock);
        assert!(!in_use_in(&root.join("v1"), &root.join("run")));
        let lock = prepare_in(&second, &root.join("run")).unwrap().unwrap();
        assert!(!in_use_in(&root.join("v1"), &root.join("run")));
        assert!(in_use_in(&root.join("v2"), &root.join("run")));
        assert_eq!(fs::read_to_string(second.get_program()).unwrap(), "v2");
        assert_eq!(
            fs::read_to_string(game.join("native-data/mod/table")).unwrap(),
            "v2"
        );
        assert!(!game.join("removed.dll").exists());
        assert_eq!(second.get_current_dir(), Some(home.as_path()));
        assert_eq!(second.get_args().collect::<Vec<_>>(), ["--offscreen"]);
        drop(lock);
        let unrelated = root.join("unrelated");
        fs::create_dir_all(&unrelated).unwrap();
        fs::write(unrelated.join("keep.txt"), "player data").unwrap();
        let mut injected = command("v3");
        injected.env(SLOT, &unrelated);
        assert!(prepare_in(&injected, &root.join("run")).is_err());
        assert_eq!(
            fs::read_to_string(unrelated.join("keep.txt")).unwrap(),
            "player data"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
