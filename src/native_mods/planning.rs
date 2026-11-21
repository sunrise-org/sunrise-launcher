//! Read-only classification of a native mod set, without executing mod or game code.
use super::*;

/// Plan enabled native mods for an instance. Worker-only filesystem and release reads.
pub fn routes(home: &Path, base: Option<&str>) -> Result<Routes, Failed> {
    require_consent()?;
    let lease = crate::native_cache::Cache::acquire(&backend::build_dir()).map_err(cache_error)?;
    let Some(folder) = stage_installed(home, root().join("installed.routes"))? else {
        return Ok(Routes::default());
    };
    classify(&folder, base, &lease, &Cancellation::default())
}

/// Share classification with an actual build under its existing cache lease.
pub(super) fn classify(
    folder: &Path,
    base: Option<&str>,
    // Held while the SDK's index, surface and build data are read and kept.
    _lease: &crate::native_cache::Cache,
    cancellation: &Cancellation,
) -> Result<Routes, Failed> {
    let found: Vec<Bundle> = scan_bundles(folder)?
        .into_iter()
        .filter(|bundle| bundle.manifest.native.is_some())
        .collect();
    let mut result = Routes::default();
    for bundle in &found {
        result
            .mods
            .push((bundle.manifest.id.clone(), Route::Prebuilt));
    }
    if found.is_empty() {
        return Ok(result);
    }
    let unchecked = |result: &mut Routes, why: &str| {
        for (id, route) in &mut result.mods {
            *route = Route::Unchecked {
                reason: trf(
                    "Can't check {mod}: its SDK isn't available. {why}",
                    &[("mod", id), ("why", why)],
                ),
            };
        }
    };
    let Some(base) = base else {
        unchecked(&mut result, tr("Install Sunrise first."));
        return Ok(result);
    };
    let built_on = checked_about(base)?;
    let origin = built_on.as_ref().map_or(base, |about| about.base.as_str());
    let revision = build_revision(base, built_on.as_ref())?;
    // A developer install's checkout has no published surface: its mods are made for it exactly.
    if crate::developer_install::is_developer(origin) {
        if let Some(list) = mismatch(&found, &revision) {
            return Err(mismatch_error(&list).into());
        }
        return Ok(result);
    }
    let sdk_base = sdk_cache::Base::from_env();
    let keys = sunrise_launcher_release_verify::sdk_keys_for(backend::GAME_REPO);
    let sdk = match sdk_cache::index(&sdk_base, &revision, &keys) {
        Ok(sdk) => sdk,
        Err(refusal) => {
            unchecked(&mut result, &refusal.to_string());
            result.unavailable = Some(refusal);
            return Ok(result);
        }
    };
    let shown = sdk.shown();
    let mut said = |id: &str, route: Route| {
        let (_, slot) = result
            .mods
            .iter_mut()
            .find(|(mod_id, _)| mod_id == id)
            .expect("parsed project has a route");
        if matches!(slot, Route::Prebuilt) {
            *slot = route;
        }
    };
    match check_projects(folder, &found, &sdk, &sdk_base, cancellation) {
        Ok(checks) => {
            for (id, why) in checks.refusals {
                let reason = refusal_text(&id, &why);
                said(id.as_str(), Route::Invalid { reason });
            }
            for (id, verdict) in checks.verdicts {
                let route = match &verdict {
                    fit::Verdict::MadeFor => continue,
                    fit::Verdict::Fits { .. } => Route::Fits {
                        line: fit::line(&id, &verdict, &shown),
                    },
                    _ => Route::Unfit {
                        headline: fit::headline(&id, &verdict),
                        line: fit::line(&id, &verdict, &shown),
                        actions: fit::actions(&id),
                    },
                };
                said(id.as_str(), route);
            }
        }
        // Nothing is guessed: without the build's tool and files, no mod is checked.
        Err(refusal) => unchecked(&mut result, &refusal.to_string()),
    }
    Ok(result)
}

/// Check a read-only set of projects. Older SDK tools need an explicit mixins pin, so resolve
/// omitted pins in a temporary copy rather than modifying these projects.
fn check_projects(
    folder: &Path,
    mods: &[Bundle],
    sdk: &sdk_cache::Sdk,
    base: &sdk_cache::Base,
    cancellation: &Cancellation,
) -> Result<Checks, sdk_cache::Refusal> {
    let ids: Vec<&str> = mods.iter().map(|b| b.manifest.id.as_str()).collect();
    let scratch;
    let folder = if mods.iter().any(|b| b.native().mixins.is_none()) {
        let refused = |why| sdk_cache::Refusal::Refused {
            build: sdk.shown(),
            why,
        };
        let pins = mixins::pins(mods, Some(sdk), base, None).map_err(refused)?;
        scratch = mixins::check_inputs(mods, &pins).map_err(refused)?;
        scratch.path()
    } else {
        folder
    };
    check_all(folder, &ids, sdk, base, cancellation, None)
}

/// The mods an update leaves out: the unfit, then every mod that needs one of them. A mod the tool refuses can't be built on any build, so it stops the update as it stops a build, by name.
pub(super) fn update_left_out(found: &[Bundle], checks: &Checks) -> Result<Vec<LeftOut>, String> {
    if let Some((id, why)) = checks.refusals.first() {
        return Err(refusal_text(id, why));
    }
    let unfit: Vec<LeftOut> = checks
        .verdicts
        .iter()
        .filter(|(_, verdict)| !verdict.usable())
        .filter_map(|(id, verdict)| {
            let bundle = found.iter().find(|b| &b.manifest.id == id)?;
            Some(LeftOut::new(
                bundle,
                isolate::Why::Unfit(fit::short(verdict)),
            ))
        })
        .collect();
    let declared: Vec<Vec<dependencies::Dependency>> = found
        .iter()
        .map(|bundle| {
            dependencies::from_manifest(
                &bundle.manifest.dependencies,
                sunrise_launcher_smod::manifest::Side::Both,
            )
        })
        .collect();
    let pairs: Vec<(&Bundle, &[dependencies::Dependency])> = found
        .iter()
        .zip(&declared)
        .map(|(b, d)| (b, d.as_slice()))
        .collect();
    Ok(isolate::with_dependents(&unfit, &pairs))
}

/// What updating an instance's native mods to a newer release or Nightly would do, told before
/// anything is downloaded but the build's small SDK files (design-fit §10.1).
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateCheck {
    /// The newer installed release or Nightly the update builds on.
    pub to: String,
    /// What the player reads: how many mods fit, and each one switched off and why.
    pub summary: String,
    /// The worlds saved with a mod the update would switch off: updating anyway loses what the
    /// mod added to them, so staying is the other choice.
    pub worlds: Vec<String>,
    /// A mod the tool refuses on any build: `summary` names it, and there is no update to take until it is fixed or removed.
    pub blocked: bool,
    /// The mods the update would switch off, by id, each with why (`summary` says the same in
    /// sentences).
    pub off: Vec<(String, String)>,
}

/// [`UpdateCheck`] for the native mods turned on in game folder `home`, on the installed release
/// or Nightly `to`. Worker-only: it reads releases.
pub fn update_check(home: &Path, to: &str) -> Result<UpdateCheck, Failed> {
    require_consent()?;
    let _lease = crate::native_cache::Cache::acquire(&backend::build_dir()).map_err(cache_error)?;
    let folder = stage_installed(home, root().join("installed.routes"))?;
    let found = match &folder {
        Some(folder) => scan_bundles(folder)?,
        None => Vec::new(),
    };
    let found: Vec<Bundle> = found
        .into_iter()
        .filter(|bundle| bundle.manifest.native.is_some())
        .collect();
    let revision = build_revision(to, None)?;
    let sdk_base = sdk_cache::Base::from_env();
    let keys = sunrise_launcher_release_verify::sdk_keys_for(backend::GAME_REPO);
    let sdk = sdk_cache::index(&sdk_base, &revision, &keys).map_err(|r| r.to_string())?;
    let checks = match &folder {
        Some(folder) => check_projects(folder, &found, &sdk, &sdk_base, &Cancellation::default())
            .map_err(|r| r.to_string())?,
        None => Checks::default(),
    };
    let shown = sdk.shown();
    let left = match update_left_out(&found, &checks) {
        Ok(left) => left,
        Err(refusal) => {
            return Ok(UpdateCheck {
                to: to.to_owned(),
                summary: refusal,
                worlds: Vec::new(),
                blocked: true,
                off: Vec::new(),
            })
        }
    };
    let off: Vec<(String, String)> = left
        .iter()
        .map(|l| {
            let why = match &l.why {
                isolate::Why::Unfit(why) => why.clone(),
                isolate::Why::Needs(library) => trf(
                    "it needs {library}, which doesn't fit",
                    &[("library", library)],
                ),
                _ => String::new(),
            };
            (l.id.clone(), why)
        })
        .collect();
    let ids: Vec<String> = left.iter().map(|l| l.id.clone()).collect();
    let all = crate::worlds::list(home);
    let worlds = crate::worlds::saved_with(&all, &ids)
        .into_iter()
        .map(|(world, _)| world.name.clone())
        .collect();
    Ok(UpdateCheck {
        to: to.to_owned(),
        summary: fit::update_summary(&shown, found.len(), &off),
        worlds,
        blocked: false,
        off,
    })
}
