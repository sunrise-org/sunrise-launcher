//! Dedicated launches are separate from profile Play, including when the build tags match.
use super::*;
use std::io;
use sunrise_launcher_smod::selection::{LaunchArguments, ModSelectionContext};

/// Resolve a not-yet-created directory using its nearest existing ancestor. A loop, access
/// error or dangling symlink is not permission to create or write through that path.
fn canonical_candidate(path: &Path) -> Result<PathBuf, String> {
    use std::path::Component;
    let absolute = std::path::absolute(path).map_err(|e| e.to_string())?;
    if absolute.components().any(|c| c == Component::ParentDir) {
        return Err(refused());
    }
    let mut tail = Vec::new();
    let mut ancestor = absolute.as_path();
    loop {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => {
                let mut resolved = ancestor.canonicalize().map_err(|e| e.to_string())?;
                for name in tail.into_iter().rev() {
                    resolved.push(name);
                }
                return Ok(resolved);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                tail.push(ancestor.file_name().ok_or_else(refused)?);
                ancestor = ancestor.parent().ok_or_else(refused)?;
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn refused() -> String {
    crate::i18n::tr("A dedicated server needs its own folder and verified launch settings, separate from a game profile.").into()
}

/// A server may use a player-chosen directory, but not a profile, child or ancestor of one.
/// Compare resolved paths; a symlink or junction cannot turn an alias into a server root.
pub(crate) fn root(folder: &Path) -> Result<PathBuf, String> {
    let root = canonical_candidate(folder)?;
    let game = canonical_candidate(&game_home())?;
    if root.starts_with(&game) || game.starts_with(&root) {
        return Err(refused());
    }
    // The default profile contains the owned-profile and pack containers. A linked profile
    // can resolve outside that tree, so also check the known homes in those containers.
    for home in crate::instances::profile_homes_strict(&game)? {
        let home = canonical_candidate(&home)?;
        if root.starts_with(&home) || home.starts_with(&root) {
            return Err(refused());
        }
    }
    Ok(root)
}

pub(super) fn command(tag: &str, folder: &Path) -> Result<Command, String> {
    crate::browser::sunrise::txn::Target::legacy(tag)?;
    let root = root(folder)?;
    let mut command = Command::new(
        std::path::absolute(version_dir(tag).join(CLIENT)).map_err(|e| e.to_string())?,
    );
    if crate::developer_install::uses_developer(tag) {
        command.env("SUNRISE_TELEMETRY_URL", "off");
    }
    crate::run_leases::mark(&mut command, &version_dir(tag));
    // Even launching from a shell with SUNRISE_HOME set cannot aim the server at Play's data.
    command
        .current_dir(&root)
        .env("SUNRISE_HOME", &root)
        .env_remove(crate::run_leases::GAME)
        .env_remove("SUNRISE_ACCOUNT_SESSIONS");
    for key in [
        "LD_PRELOAD",
        "LD_AUDIT",
        "LD_LIBRARY_PATH",
        "DYLD_INSERT_LIBRARIES",
        "DYLD_LIBRARY_PATH",
        "DYLD_FALLBACK_LIBRARY_PATH",
    ] {
        command.env_remove(key);
    }
    command
        .arg("--resources")
        .arg(std::path::absolute(resources(tag)).map_err(|e| e.to_string())?)
        .args(["--dedicated", "--nogui"]);
    Ok(command)
}

/// Validate the exact effective command at the CLI or MCP handoff, after jar preparation.
pub(super) fn verify(tag: &str, folder: &Path, actual: &Command) -> Result<(), String> {
    let expected = command(tag, folder)?;
    if actual.get_program() != expected.get_program()
        || actual.get_args().ne(expected.get_args())
        || actual.get_current_dir() != expected.get_current_dir()
        || actual.get_envs().ne(expected.get_envs())
    {
        return Err(refused());
    }
    let exe = Path::new(actual.get_program());
    let installed = version_dir(tag).canonicalize().map_err(|e| e.to_string())?;
    let resources = resources(tag).canonicalize().map_err(|e| e.to_string())?;
    if exe.canonicalize().map_err(|e| e.to_string())?.parent() != Some(&installed)
        || !resources.starts_with(installed.join(".local"))
    {
        return Err(refused());
    }
    let LaunchArguments {
        context:
            ModSelectionContext {
                game_dir,
                home,
                dev_mods,
            },
        ..
    } = LaunchArguments::for_command(actual).map_err(|e| e.to_string())?;
    if game_dir != home || !dev_mods.is_empty() {
        return Err(refused());
    }
    Ok(())
}
