//! Propose the smallest provable same-base change. Turning off is not deleting an archive.
use super::*;
use sunrise_launcher_smod::dependencies::parse_version;

#[derive(Clone, Debug)]
pub struct ResolveQuestion {
    pub requested: Vec<String>,
    pub blockers: Vec<(String, String)>,
    pub worlds: Question,
}

pub(super) fn minimal<'a>(
    catalogue: &index::Index,
    roots: &[resolve::Request],
    installed: &'a [resolve::InstalledMod],
    game: &str,
    conflicts: &[resolve::ManifestConflict],
    native_fits: impl Fn(&index::Project, &index::Version) -> bool,
) -> Result<Option<(resolve::InstallPlan, Vec<&'a str>)>, InstallError> {
    let eligible: Vec<_> = installed
        .iter()
        .filter(|old| !roots.iter().any(|root| root.id == old.id))
        .map(|old| old.id.as_str())
        .collect();
    let mut tried = 0;
    for count in 1..=eligible.len() {
        let mut positions: Vec<_> = (0..count).collect();
        loop {
            tried += 1;
            if tried > MAX_TRIALS {
                return Err(InstallError::Unproved(
                    tr("Too many combinations to verify. Nothing was installed.").into(),
                ));
            }
            let disabled: Vec<_> = positions.iter().map(|&i| eligible[i]).collect();
            let retain: Vec<_> = installed
                .iter()
                .filter(|old| !disabled.contains(&old.id.as_str()))
                .cloned()
                .collect();
            // A disabled old project cannot immediately return as a required download.
            let mut available = catalogue.clone();
            available
                .projects
                .retain(|p| !disabled.contains(&p.id.as_str()));
            match resolve::install_whole(
                &available,
                roots,
                &retain,
                game,
                Side::Both,
                resolve::WholeOptions {
                    conflicts,
                    native_fits: &native_fits,
                    messages: trf,
                },
            ) {
                Ok(plan) => return Ok(Some((plan, disabled))),
                Err(resolve::WholeRefusal::Unproved(reason)) => {
                    return Err(InstallError::Unproved(reason))
                }
                Err(resolve::WholeRefusal::Inconsistent(_)) => (),
            }
            let Some(i) = (0..count).rfind(|&i| positions[i] < eligible.len() - count + i) else {
                break;
            };
            positions[i] += 1;
            for j in i + 1..count {
                positions[j] = positions[j - 1] + 1;
            }
        }
    }
    Ok(None)
}

pub(super) fn question(
    catalogue: &index::Index,
    roots: &[resolve::Request],
    before: &Inventory,
    candidate: &Path,
    disabled: &[&str],
) -> Result<ResolveQuestion, String> {
    let packages = inventory(candidate, None)?;
    let enabled: BTreeMap<_, _> = packages
        .iter()
        .filter(|(id, _)| mod_list::is_enabled(candidate, id))
        .map(|(id, p)| (id.as_str(), &p.package.manifest))
        .collect();
    let mut blockers = Vec::new();
    for &id in disabled {
        let old = &before[id].package.manifest;
        // The resolver uses catalogue metadata. Require a concrete blocker in the staged
        // manifests too, so a missing row or unsupported version cannot disable a real mod.
        let requirement = old.dependencies.iter().find_map(|(id, dependency)| {
            let need = dependency.details();
            if need.optional || need.side == Side::Server {
                return None;
            }
            match enabled.get(id.as_str()) {
                Some(other)
                    if parse_version(&other.version).is_ok_and(|v| need.version.matches(&v)) =>
                {
                    None
                }
                Some(other) => Some(trf(
                    "Needs {mod} {range}; this install uses {version}.",
                    &[
                        ("mod", &other.name),
                        ("range", &need.version.to_string()),
                        ("version", &other.version),
                    ],
                )),
                None if disabled.contains(&id.as_str()) => Some(trf(
                    "Needs {mod}, which would also be turned off.",
                    &[("mod", &before[id].package.manifest.name)],
                )),
                None => None,
            }
        });
        let incompatible = |a: &Manifest, b: &Manifest| {
            let conflict = a.incompatible.get(&b.id).is_some_and(|rule| {
                parse_version(&b.version).is_ok_and(|v| rule.details().version.matches(&v))
            });
            conflict
                || a.owned_namespaces()
                    .any(|ns| b.owned_namespaces().any(|other| other == ns))
        };
        let reason = requirement
            .or_else(|| {
                enabled
                    .values()
                    .find(|m| incompatible(old, m) || incompatible(m, old))
                    .map(|m| {
                        trf(
                            "Doesn't work with {mod} {version}.",
                            &[("mod", &m.name), ("version", &m.version)],
                        )
                    })
            })
            .ok_or_else(unproved_mods)?;
        blockers.push((old.name.clone(), reason));
    }
    let requested = roots
        .iter()
        .map(|r| {
            catalogue
                .project(&r.id)
                .map_or_else(|| r.id.clone(), |p| p.name.clone())
        })
        .collect();
    Ok(ResolveQuestion {
        requested,
        blockers,
        worlds: Question::default(),
    })
}
