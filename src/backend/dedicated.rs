//! Dedicated launches are separate from profile Play, even when their build tags match.
use super::{game_home, resources, version_dir, versions_dir, CLIENT};
use crate::i18n::tr;
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
    process::Command,
};

fn refused() -> String {
    tr("A dedicated server needs its own folder and verified launch settings, separate from a game profile.").into()
}

/// Resolve even a not-yet-created folder through its nearest existing ancestor. A dangling
/// link, an inaccessible path, or `..` in the missing suffix is not permission to create it.
fn canonical_folder(path: &Path) -> Result<PathBuf, String> {
    let absolute = std::path::absolute(path).map_err(|error| error.to_string())?;
    let mut tail = Vec::new();
    let mut ancestor = absolute.as_path();
    loop {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => {
                let mut resolved = ancestor.canonicalize().map_err(|error| error.to_string())?;
                if !fs::metadata(&resolved)
                    .map_err(|error| error.to_string())?
                    .is_dir()
                {
                    return Err(refused());
                }
                resolved.extend(tail.into_iter().rev());
                return Ok(resolved);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let Some(Component::Normal(name)) = ancestor.components().next_back() else {
                    return Err(refused());
                };
                tail.push(name);
                ancestor = ancestor.parent().ok_or_else(refused)?;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

/// Check the resolved paths, including profile entries that link outside the default tree.
/// This does not call the profile list, which may write back to the profile store.
pub(crate) fn root(folder: &Path) -> Result<PathBuf, String> {
    let root = canonical_folder(folder)?;
    let game = canonical_folder(&game_home())?;
    let overlaps = |home: &Path| root.starts_with(home) || home.starts_with(&root);
    if overlaps(&game) {
        return Err(refused());
    }
    for home in crate::instances::profile_homes_strict(&game)? {
        if overlaps(&canonical_folder(&home)?) {
            return Err(refused());
        }
    }
    Ok(root)
}

pub(super) fn command(tag: &str, folder: &Path) -> Result<Command, String> {
    let root = root(folder)?;
    let mut parts = Path::new(tag).components();
    if !matches!(parts.next(), Some(Component::Normal(name)) if name == tag)
        || parts.next().is_some()
    {
        return Err(refused());
    }
    if crate::native_mods::is_native(tag) {
        return Err(tr("A dedicated server cannot use a profile’s native client build.").into());
    }
    let installed = std::path::absolute(version_dir(tag)).map_err(|error| error.to_string())?;
    let mut command = Command::new(installed.join(CLIENT));
    if crate::developer_install::uses_developer(tag) {
        command.env("SUNRISE_TELEMETRY_URL", "off");
    }
    crate::run_leases::mark(&mut command, &installed);
    command.current_dir(&root).env("SUNRISE_HOME", &root);
    for name in [
        crate::run_leases::GAME,
        "SUNRISE_ACCOUNT_SESSIONS",
        "LD_PRELOAD",
        "LD_AUDIT",
        "LD_LIBRARY_PATH",
        "DYLD_INSERT_LIBRARIES",
        "DYLD_LIBRARY_PATH",
        "DYLD_FALLBACK_LIBRARY_PATH",
    ] {
        command.env_remove(name);
    }
    command
        .arg("--resources")
        .arg(std::path::absolute(resources(tag)).map_err(|error| error.to_string())?)
        .args(["--dedicated", "--nogui"]);
    Ok(command)
}

pub(super) fn verify_paths(tag: &str, command: &Command) -> Result<(), String> {
    let versions = versions_dir()
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let installed = version_dir(tag)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let program = Path::new(command.get_program())
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let resources = Path::new(
        command
            .get_args()
            .nth(1)
            .expect("dedicated command owns its resources argument"),
    )
    .canonicalize()
    .map_err(|error| error.to_string())?;
    if installed != versions.join(tag)
        || program != installed.join(CLIENT)
        || !resources.starts_with(installed.join(".local"))
    {
        return Err(refused());
    }
    Ok(())
}

/// A direct launch uses only the dedicated argv, the isolated root, and the dedicated
/// overrides. Windows may replace the installed program with that root's prepared copy.
pub(super) fn verify(
    tag: &str,
    folder: &Path,
    actual: &Command,
    effective: bool,
) -> Result<(), String> {
    let expected = command(tag, folder)?;
    verify_paths(tag, &expected)?;
    if actual.get_args().ne(expected.get_args())
        || actual.get_current_dir() != expected.get_current_dir()
    {
        return Err(refused());
    }
    #[cfg(windows)]
    if effective {
        return crate::windows_runtime::verify_dedicated(&expected, actual).map_err(|_| refused());
    }
    #[cfg(not(windows))]
    let _ = effective;
    if actual.get_program() != expected.get_program() || actual.get_envs().ne(expected.get_envs()) {
        return Err(refused());
    }
    Ok(())
}
