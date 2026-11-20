use super::*;

pub(super) fn native_permission(trust: bool) -> Result<(), String> {
    if trust {
        backend::Settings::load().allow_native_mods()?;
    }
    native_mods::require_consent().map_err(String::from)
}

pub(super) fn github_native(
    instance: &instances::Instance,
    found: github_mods::Resolved,
    only: Option<&str>,
    all: bool,
    trust: bool,
    update: Option<&github_mods::Origin>,
) -> Result<(), String> {
    println!("{}", found.label());
    println!(
        "{}",
        trf("Source, to read: {url}", &[("url", &found.source_url())])
    );
    eprintln!("{}", tr(github_mods::WARNING));
    native_permission(trust)?;
    let mut ready = github_mods::fetch(found)?;
    let selected = github_mods::choose(ready.mods(), only, all)?
        .into_iter()
        .map(|package| package.id.clone())
        .collect();
    ready.retain(&selected);
    let owner = Arc::new(sunrise_launcher::browser::sunrise::txn::Lease::admit(
        &instance.home,
    )?);
    let proposal = sunrise_launcher::browser::sunrise::txn::prepare_github_install(
        owner,
        instance,
        &ready,
        update,
        &|label, fraction| progress(label.to_owned())(fraction),
        &registry_consent,
    )?;
    if !switch::approve_world_change(proposal.question(), tr("Install anyway? [y/N]"))? {
        return Err(tr("Install cancelled. Nothing was changed.").into());
    }
    for o in proposal.commit(true)? {
        println!(
            "{}",
            trf(
                "Installed {mod} {version} from GitHub ({source}), SHA-256 {hash}.",
                &[
                    ("mod", &o.id),
                    ("version", &o.version),
                    ("source", &o.label()),
                    ("hash", &o.sha256),
                ]
            )
        );
    }
    Ok(())
}

pub(super) fn registry_consent(inventory: &native_mods::Consent) -> bool {
    eprintln!("{}\n{}", inventory.label(), inventory.detail());
    // The headless native route has already required explicit native-code permission.
    true
}
