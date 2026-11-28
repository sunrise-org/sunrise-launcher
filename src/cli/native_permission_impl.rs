use super::*;
use sunrise_launcher::browser::sunrise;

/// Code the registry didn't review runs only when this run says so: nothing is remembered.
pub(super) fn native_permission(trust: bool) -> Result<(), String> {
    if trust {
        Ok(())
    } else {
        Err(tr("This native code isn't reviewed by the Sunrise registry. Read its source, then pass --trust-native-code to run it.").into())
    }
}

/// Lets this run's builds take a profile's native mods that neither came verified from the
/// registry nor were confirmed, such as files copied into it by hand. Nothing is remembered.
pub(super) fn trust_unreviewed_builds() {
    native_mods::ask_unreviewed_with(|found| {
        let names: Vec<&str> = found.iter().map(|p| p.name.as_str()).collect();
        sunrise_launcher::elog!(
            "{}",
            trf(
                "Building native code nobody reviewed: {mods}",
                &[("mods", &names.join(", "))]
            )
        );
        true
    });
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
    let target = sunrise::Target::of(instance)?;
    let identity = target.identity();
    println!("{}", found.label());
    println!("Source, to read: {}", found.source_url());
    sunrise_launcher::elog!("{}", github_mods::WARNING);
    native_permission(trust)?;
    let mods = fetch(found)?;
    let mut fresh = Vec::new();
    let mut replacements = Vec::new();
    for package in github_mods::choose(&mods, only, all)? {
        let path = instance
            .home
            .join(native_mods::INSTALLED)
            .join(format!("{}.smod", package.id));
        if update {
            replacements.push(package);
        } else {
            match std::fs::symlink_metadata(&path) {
                Ok(_) => replacements.push(package),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    fresh.push(package.clone());
                }
                Err(error) => return Err(format!("{}: {error}", path.display())),
            }
        }
    }
    let choose = || sunrise_launcher::managed_play::headless_first(instance);
    let hooks = sunrise::InstallHooks {
        progress: &|_| {},
        continue_after_failure: &|_| false,
        cancellation: None,
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
        let installed = sunrise::add_github(&target, identity, found, &fresh, None, choose, hooks)
            .map_err(|error| error.to_string())?;
        for origin in installed {
            report("Installed", origin);
        }
    }
    for package in replacements {
        let origin = sunrise::update_github(
            &target,
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
mod tests;
