//! Sunrise's own registry: mods reviewed in sunrise-org/sunrise-mods, read from the signed index
//! ([`v2`]); installs, updates and build switches check every download against it, and mods it
//! says were taken down are turned off before play.
use super::{installed, mod_list, Kind, SourceId};
use crate::i18n::{tr, trf};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use sunrise_launcher_smod::index_v2;

/// The registry's pages on the website, `<SITE>/<slug>`.
pub(crate) const SITE: &str = sunrise_launcher_addresses::MOD_PAGES;

/// A GitHub login as GitHub allows it (letters, digits, single dashes, at most 39), so an index
/// entry can't turn a link into something else.
fn github_login(login: &str) -> bool {
    (1..=39).contains(&login.len())
        && login
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !login.starts_with('-')
        && !login.ends_with('-')
        && !login.contains("--")
}

/// The GitHub repository a project's `source` is in, `https://github.com/<owner>/<repo>`: a
/// deeper path (a folder of a monorepo) or a trailing `.git` names the same repository. None
/// when the source isn't on GitHub, or its owner or name isn't one GitHub allows.
pub fn github_repo(source: &str) -> Option<String> {
    let path = source.strip_prefix("https://github.com/")?;
    let mut parts = path.split(['/', '?', '#']);
    let owner = parts.next().filter(|o| github_login(o))?;
    let repo = parts.next()?;
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    let allowed = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.');
    if repo.is_empty() || repo.len() > 100 || repo.starts_with('.') || !repo.bytes().all(allowed) {
        return None;
    }
    Some(format!("https://github.com/{owner}/{repo}"))
}

/// The versions the player picked in game folder `home`, by mod id.
pub fn chosen(home: &Path) -> std::collections::BTreeMap<String, String> {
    fs::read(home.join("mod-browser/chosen-versions.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// The mod id of an installed mod (`sunrise-mods/<id>`, `sunrise-mods/<id>.smod` or
/// `sunrise-native-mods/<id>.smod`).
pub fn mod_id(entry: &installed::Entry) -> Option<&str> {
    if let Some(rest) = entry.file.strip_prefix(crate::native_mods::INSTALLED) {
        return rest.strip_prefix('/')?.strip_suffix(".smod");
    }
    let rest = entry
        .file
        .strip_prefix(mod_list::MODS_DIR)?
        .strip_prefix('/')?;
    Some(rest.strip_suffix(".smod").unwrap_or(rest))
}

/// Check the actual launch profile, after its folder has been applied and before starting it:
/// mods the signed registry took down are turned off, and a build that has one compiled in is
/// refused. May contact the registry; when no verified copy of it can be read, play goes on with
/// a notice, not a clean verdict.
pub fn before_play(
    cmd: &mut std::process::Command,
    tag: &str,
    launch: &crate::launch::Launch,
) -> Result<Vec<String>, String> {
    use sunrise_launcher_smod::selection::LaunchArguments;

    crate::launch::apply_environment(cmd, launch);
    let args = LaunchArguments::for_command(cmd).map_err(|e| e.to_string())?;
    if args.is_information_request() {
        return Ok(Vec::new());
    }
    let included = crate::native_status::included(tag)?;
    // The mod list lives with the game’s packages, not the separate cache/data home.
    let notices = check_before_play(&args.context.game_dir, &included, || {
        v2::change_index().ok().map(|catalog| catalog.index)
    })?;
    crate::native_status::check_compiled_profile(&included, &args.context.game_dir)?;
    Ok(notices)
}

fn check_before_play(
    home: &Path,
    included: &[crate::native_status::Included],
    load: impl FnOnce() -> Option<Arc<index_v2::Index>>,
) -> Result<Vec<String>, String> {
    let entries = {
        // A writer can already hold the lock even though its pending file isn't visible yet.
        let _lock = super::lock_home(home).map_err(|error| error.to_string())?;
        installed::recover_game_install(home)?;
        installed::Installed::read(home)?.entries
    };
    if included.is_empty() && !entries.iter().any(|e| e.source == SourceId::Sunrise) {
        return Ok(Vec::new());
    }
    let Some(index) = load() else {
        return Ok(vec![
            tr("Mods were not checked for registry takedowns.").into()
        ]);
    };
    let lines = turn_off_revoked_v2(&index, home)?;
    // Switching the mod off in this profile cannot remove code from the selected binary.
    for crate::native_status::Included { id, sha256, .. } in included {
        let revoked = index.revoked_by_sha256(sha256).or_else(|| {
            index
                .releases_by_sha256(id, sha256)
                .into_iter()
                .find_map(|(project, _)| index.revoked(&project.id, Some(sha256)))
        });
        if let Some(revoked) = revoked {
            return Err(trf(
                "This build includes a mod the registry took down. Choose another game build.\n{mod}: {reason}",
                &[("mod", id), ("reason", &revoked.reason)],
            ));
        }
    }
    Ok(lines)
}
/// Pre-Play takedown checks on the profile's own mod folder, before the profile journal freezes
/// it: a mod the signed registry took down must be off (or gone) before Play.
pub fn pre_play_notices(home: &Path) -> Result<Vec<String>, String> {
    use crate::managed_play::scan_packages;
    let packages = scan_packages(home)?;
    let rows = installed::Installed::read(home)?;
    if !rows
        .entries
        .iter()
        .any(|row| row.source == SourceId::Sunrise)
        && !packages.iter().any(|package| {
            package.manifest.native.is_some() && mod_list::is_enabled(home, &package.manifest.id)
        })
    {
        return Ok(Vec::new());
    }
    let Some(index) = v2::index().ok().map(|catalog| catalog.index) else {
        return Ok(vec![
            tr("Mods were not checked for registry takedowns.").into()
        ]);
    };
    for package in &packages {
        let id = &package.manifest.id;
        if package.directory || !mod_list::is_enabled(home, id) {
            continue;
        }
        let hash = sunrise_launcher_smod::index::sha256_file(&package.path)?;
        if let Some(revoked) = revoked_for_installed_package(&index, &rows, home, package, &hash) {
            return Err(trf(
                "{mod} was taken down. Turn it off before playing.\n{reason}",
                &[("mod", &package.manifest.name), ("reason", &revoked.reason)],
            ));
        }
    }
    let mut notices = Vec::new();
    for entry in rows.entries {
        if entry.source != SourceId::Sunrise
            || !matches!(entry.kind, Kind::ResourcePack | Kind::Shader)
        {
            continue;
        }
        let revoked = if index_v2::valid_project_id(&entry.project) {
            index.revoked(&entry.project, entry.sha256())
        } else {
            entry
                .sha256()
                .and_then(|hash| index.revoked_by_sha256(hash))
        };
        if let Some(revoked) = revoked {
            notices.push(trf(
                "{mod} was taken down: {reason} Remove it.",
                &[("mod", &entry.title), ("reason", &revoked.reason)],
            ));
        }
    }
    Ok(notices)
}

/// Check these mods may be on for this active build, then turn them on together.
pub fn activate(
    home: &Path,
    active: Option<&crate::managed_play::Active>,
    ids: &[&str],
) -> Result<(), String> {
    check_activation(home, active, ids)?;
    mod_list::set_all_enabled(home, ids, true)
}

pub mod auto;
pub mod pack;
pub mod ratings;
mod registry_install;
pub mod sources;
pub mod v2;
pub(crate) use registry_install::check_activation;
pub use registry_install::v2::revoked_for_package;
pub(crate) use registry_install::v2::{fetch_release_bytes, revoked_for_installed_package};
pub use registry_install::{
    add_github, add_sources, apply_sources, checked_switch_fit_v2, classify_v2_ownership,
    install_github_manual, install_local, install_local_at, install_registry_v2, inventory_v2,
    known_unfit_updates, pack_sources, plan_v2_install, plan_v2_update, prepare_pasted_sources,
    prepare_source, read_pasted_sources, remove_mods, restore_held, switch_build_v2,
    switch_profile_build_v2, switch_provider_v2, turn_off_revoked_v2, unfit_updates_revision,
    update_github, update_registry_v2, BuildChange, BuildChoice, BuildNotice, ChangeImpact,
    InstallError, InstallHooks, InstallProgress, InstalledBatch, InstalledV2Mod, ManualRelease,
    PastedSource, PlannedV2Release, PreparedSource, ProviderCandidate, ProviderChoiceNeeded,
    ProviderConflict, ProviderSwitchPreview, PublisherChangeQuestion, SourceDependency,
    SourceError, SourceHooks, SourceProof, SourceRequest, SwitchDecision, SwitchHooks,
    SwitchQuestion, Switched, Target, UpdateRequest, V2AffectedDependent, V2Hooks,
    V2InstallOptions, V2Ownership, V2Plan, V2ProviderSwitchHooks, V2SwitchHooks,
};
#[cfg(test)]
mod tests;
