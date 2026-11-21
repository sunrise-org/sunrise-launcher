use super::*;
use sunrise_launcher::browser::sunrise;

pub(super) fn native_permission(trust: bool) -> Result<(), String> {
    if trust {
        backend::Settings::load().allow_native_mods()?;
    }
    native_mods::require_consent().map_err(String::from)
}

pub(super) struct GithubInstall<'a> {
    pub instance: &'a instances::Instance,
    pub found: &'a github_mods::Resolved,
    pub only: Option<&'a str>,
    pub all: bool,
    pub update: bool,
    pub trust: bool,
    pub confirm_world_loss: bool,
}

pub(super) fn github_native(request: GithubInstall<'_>) -> Result<(), String> {
    github_native_from(request, github_mods::fetch)
}

fn github_native_from(
    request: GithubInstall<'_>,
    fetch: impl FnOnce(&github_mods::Resolved) -> Result<Vec<github_mods::Packed>, String>,
) -> Result<(), String> {
    let GithubInstall {
        instance,
        found,
        only,
        all,
        update,
        trust,
        confirm_world_loss,
    } = request;
    if instance.kind == instances::Kind::Pack {
        return Err(tr("This pack controls which mods it installs.").into());
    }
    let identity = sunrise::install_target(instance)?;
    println!("{}", found.label());
    println!("Source, to read: {}", found.source_url());
    eprintln!("{}", github_mods::WARNING);
    native_permission(trust)?;
    let mods = fetch(found)?;
    let mut fresh = Vec::new();
    let mut replacements = Vec::new();
    for package in github_mods::choose(&mods, only, all)? {
        let target = instance
            .home
            .join(native_mods::INSTALLED)
            .join(format!("{}.smod", package.id));
        if update {
            replacements.push(package);
        } else {
            match std::fs::symlink_metadata(&target) {
                Ok(_) => replacements.push(package),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    fresh.push(package.clone());
                }
                Err(error) => return Err(format!("{}: {error}", target.display())),
            }
        }
    }
    let choose = || sunrise_launcher::managed_play::headless_first(instance);
    let hooks = sunrise::InstallHooks {
        progress: &|_| {},
        native: &|_| {
            native_mods::require_consent()
                .map(|_| true)
                .map_err(String::from)
        },
        registry_crates: Some(&registry_consent),
        continue_after_failure: &|_| false,
        resolve: None,
    };
    let report = |action, origin: github_mods::Origin| {
        println!(
            "{action} {} {} from GitHub ({}), sha256 {}.",
            origin.id,
            origin.version,
            origin.label(),
            origin.sha256
        );
    };
    if !fresh.is_empty() {
        let installed = sunrise::add_github(instance, identity, found, &fresh, None, choose, hooks)
            .map_err(|error| error.to_string())?;
        for origin in installed {
            report("Installed", origin);
        }
    }
    for package in replacements {
        let origin = sunrise::update_github(
            instance,
            identity,
            found,
            package,
            choose,
            hooks,
            &|impact| approve_mod_change(impact, confirm_world_loss),
        )
        .map_err(|error| match error {
            sunrise::InstallError::Cancelled => tr("Nothing was updated.").into(),
            e => e.to_string(),
        })?;
        report("Updated", origin);
    }
    Ok(())
}

#[cfg(all(unix, test))]
#[path = "cli_native_permission/tests.rs"]
mod tests;

pub(super) fn registry_consent(inventory: &native_mods::Consent) -> bool {
    eprintln!("{}\n{}", inventory.label(), inventory.detail());
    // The headless native route has already required explicit native-code permission.
    true
}
