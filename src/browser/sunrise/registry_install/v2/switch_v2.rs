//! Schema-2 game-build switching: preserves each installed mod's chosen project, relies solely on
//! authoritative `release.fits` / `release.does_not_fit` verdicts (`Fits` / `DoesNotFit` ->
//! `NoFit` / `Unknown` -> `Unproved`), verifies SHA-256 + offline Sigstore attestation + publisher
//! pin for every replacement archive, honors `chosen-versions.json` pins, and keeps verified
//! project identity in `HeldSource::Registry`.

use super::{
    check_staged_v2_candidate, chosen_version_pin, commit_pending_pins, compare_versions,
    fetch_release_bytes_cancellable, inventory_v2, revoked_for_package,
    stage_and_verify_v2_release, BytesFetcher, InstalledV2Mod, PendingPin, PlannedV2Release,
    ReleaseVerifier, V2Ownership, V2Transfer,
};
use crate::{
    attestation::{self, PublisherChange},
    backend,
    browser::{
        mod_list,
        sunrise::{
            chosen,
            registry_install::{
                at, build_progress, switch::sha256_bytes, BuildChoice, Document, InstallError,
                InstallProgress, SwitchDecision, SwitchHooks, SwitchQuestion, Switched,
            },
            v2 as catalogue_v2,
        },
    },
    i18n::tr,
    managed_play::{Held, HeldReason, HeldRoot, HeldSource, Profile, Target},
    profile_txn::Identity,
    worlds::switch::{Fence, TargetAge},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use sunrise_launcher_smod::index_v2::{FitVerdict, Index, Kind as V2Kind, Project, Release};

/// Callbacks and pin directory override for v2 build switching.
pub struct V2SwitchHooks<'a> {
    pub switch: SwitchHooks<'a>,
    pub confirm_publisher_change: Option<super::ConfirmPublisherChange<'a>>,
    pub pin_dir: Option<PathBuf>,
}

/// Read-only check: true when every enabled v2-owned mod in `home` has a non-revoked release of
/// its SAME chosen project whose authoritative `fits` list names exact build `tag` (respecting any
/// `chosen-versions.json` pin).
pub fn checked_switch_fit_v2(index: &Index, home: &Path, tag: &str) -> bool {
    let Ok(installed) = inventory_v2(home, index) else {
        return false;
    };
    let pins = chosen(home);
    for (slot, item) in &installed {
        if !item.enabled {
            continue;
        }
        if item.package.directory {
            return false;
        }
        if revoked_for_package(index, &item.ownership, &item.digest).is_some() {
            return false;
        }
        let Some(project_id) = item.ownership.project_id() else {
            // Unadopted/local mod cannot be proved against `tag` from the registry alone.
            return false;
        };
        let Some(project) = index.project(project_id) else {
            return false;
        };
        if index.revoked(&project.id, None).is_some() {
            return false;
        }
        let pin = chosen_version_pin(&pins, &project.id, slot);
        let fits = project.releases.iter().any(|release| {
            pin.is_none_or(|p| release.version == p)
                && release.fit_verdict(tag) == FitVerdict::Fits
                && index
                    .revoked(&project.id, Some(&release.file.sha256))
                    .is_none()
        });
        if !fits {
            return false;
        }
    }
    true
}

/// Switch `instance` to exact prepared Sunrise build `tag` using the signed v2 registry.
pub fn switch_build_v2(
    target: &Target,
    identity: Identity,
    tag: &str,
    hooks: V2SwitchHooks<'_>,
) -> Result<Switched, String> {
    switch_profile_build_v2(target, identity, tag, None, hooks)
}

/// Switch `instance` to exact prepared Sunrise build `tag` and persist `choice` atomically.
pub fn switch_profile_build_v2(
    target: &Target,
    identity: Identity,
    tag: &str,
    choice: Option<BuildChoice>,
    hooks: V2SwitchHooks<'_>,
) -> Result<Switched, String> {
    let stop = hooks.switch.cancellation;
    let fetch = |url: &str, max: u64| fetch_release_bytes_cancellable(url, max, Some(stop));
    switch_profile_build_v2_with(
        target,
        identity,
        tag,
        choice,
        hooks,
        || {
            catalogue_v2::refreshed_index()
                .map(|c| c.index)
                .map_err(|e| e.to_string())
        },
        &fetch,
        &attestation::verify_release,
    )
}

/// [`switch_profile_build_v2`] with the index read, the download and the attestation check
/// injected, so tests switch offline through the same path.
#[allow(clippy::too_many_arguments)]
pub(crate) fn switch_profile_build_v2_with(
    target: &Target,
    identity: Identity,
    tag: &str,
    choice: Option<BuildChoice>,
    hooks: V2SwitchHooks<'_>,
    fetch_index: impl FnOnce() -> Result<Arc<Index>, String>,
    fetch_bytes: BytesFetcher<'_>,
    verify_release: ReleaseVerifier<'_>,
) -> Result<Switched, String> {
    if target.is_pack() {
        return Err(tr("Choose an ordinary profile to switch builds.").into());
    }
    (hooks.switch.progress)(InstallProgress::PreparingBuild);
    let mut profile =
        Profile::open_for(target.home(), None, Some(identity)).map_err(String::from)?;
    let cancellation = hooks.switch.cancellation;
    cancellation.check()?;
    let previous_holds: Vec<Held> = profile
        .active()?
        .map(|active| active.holds().cloned().collect())
        .unwrap_or_default();
    let Some(edit) = profile.begin_switch(tag)? else {
        if let Some(choice) = choice {
            cancellation.begin_publication()?;
            target.save_build_choice(tag, choice)?;
        }
        return Ok(Switched::AlreadySelected);
    };
    let index = fetch_index()?;
    let result = switch_build_v2_edit(
        target,
        edit,
        &previous_holds,
        Arc::new(crate::native_mods::local_fit::filled(&index)),
        tag,
        choice,
        hooks,
        fetch_bytes,
        verify_release,
    )?;
    if let (Some(_), Switched::Changed(active)) = (choice, &result) {
        if let Some(committed) = active.build_choice() {
            target.sync_build_choice(committed);
        }
    }
    Ok(result)
}

pub(super) enum SlotPlan<'a> {
    /// Keep the exact on-disk archive as-is (current archive already fits `tag`, or local mod).
    Keep,
    /// Replace with a fitting release of the SAME chosen project.
    Replace(&'a Project, &'a Release),
    /// Hold the archive off because the chosen project has no fitting release for `tag`.
    Hold(HeldReason),
    /// Remove/withhold because revoked by the signed v2 index.
    Revoked { reason: String, owned: bool },
}

pub(super) fn plan_slot_for_build<'a>(
    index: &'a Index,
    slot: &str,
    item: &InstalledV2Mod,
    tag: &str,
    pins: &BTreeMap<String, String>,
) -> SlotPlan<'a> {
    if let Some(revoked) = revoked_for_package(index, &item.ownership, &item.digest) {
        return SlotPlan::Revoked {
            reason: revoked.reason.clone(),
            owned: item.ownership.is_owned(),
        };
    }
    let Some(project_id) = item.ownership.project_id() else {
        // Local or unadopted v1 mod: keep its exact bytes as-is; NEVER substitute a v2 project
        // that happens to share the same `mod_id`.
        return SlotPlan::Keep;
    };
    let Some(project) = index.project(project_id) else {
        return SlotPlan::Hold(HeldReason::Unproved {
            build: tag.to_owned(),
            detail: None,
        });
    };
    if let Some(revoked) = index.revoked(&project.id, None) {
        return SlotPlan::Revoked {
            reason: revoked.reason.clone(),
            owned: true,
        };
    }

    let pin = chosen_version_pin(pins, &project.id, slot);
    // If the currently installed exact archive already fits `tag` and matches any pin, keep it.
    let current_release = project
        .releases
        .iter()
        .find(|r| r.file.sha256.eq_ignore_ascii_case(&item.digest));
    if current_release.is_some_and(|current| {
        pin.is_none_or(|p| p == current.version) && current.fit_verdict(tag) == FitVerdict::Fits
    }) {
        return SlotPlan::Keep;
    }

    // If pinned to an explicit version in chosen-versions.json, check ONLY that pinned version.
    if let Some(pinned_ver) = pin {
        let Some(release) = project.releases.iter().find(|r| r.version == pinned_ver) else {
            return SlotPlan::Hold(HeldReason::Unproved {
                build: tag.to_owned(),
                detail: None,
            });
        };
        if index
            .revoked(&project.id, Some(&release.file.sha256))
            .is_some()
        {
            return SlotPlan::Hold(HeldReason::NoFit {
                build: tag.to_owned(),
                detail: None,
            });
        }
        return match release.fit_verdict(tag) {
            FitVerdict::Fits => SlotPlan::Replace(project, release),
            FitVerdict::DoesNotFit => SlotPlan::Hold(HeldReason::NoFit {
                build: tag.to_owned(),
                detail: None,
            }),
            FitVerdict::Unknown => SlotPlan::Hold(HeldReason::Unproved {
                build: tag.to_owned(),
                detail: None,
            }),
        };
    }

    // Otherwise search ONLY `project.releases` (NEVER another project for the same mod_id!).
    let mut fitting: Vec<&Release> = Vec::new();
    let mut saw_unknown = false;
    let mut saw_does_not_fit = false;
    for release in &project.releases {
        if index
            .revoked(&project.id, Some(&release.file.sha256))
            .is_some()
        {
            continue;
        }
        match release.fit_verdict(tag) {
            FitVerdict::Fits => fitting.push(release),
            FitVerdict::DoesNotFit => saw_does_not_fit = true,
            FitVerdict::Unknown => saw_unknown = true,
        }
    }
    if let Some(best) = fitting
        .into_iter()
        .max_by(|a, b| compare_versions(&a.version, &b.version))
    {
        return SlotPlan::Replace(project, best);
    }
    if !saw_unknown && saw_does_not_fit {
        SlotPlan::Hold(HeldReason::NoFit {
            build: tag.to_owned(),
            detail: None,
        })
    } else {
        SlotPlan::Hold(HeldReason::Unproved {
            build: tag.to_owned(),
            detail: None,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn switch_build_v2_edit(
    target: &Target,
    edit: crate::managed_play::Edit<'_>,
    previous_holds: &[Held],
    index: Arc<Index>,
    tag: &str,
    choice: Option<BuildChoice>,
    hooks: V2SwitchHooks<'_>,
    fetch_bytes: BytesFetcher<'_>,
    verify_release: ReleaseVerifier<'_>,
) -> Result<Switched, String> {
    (hooks.switch.progress)(InstallProgress::Checking);
    let old = edit.candidate()?;
    let before = inventory_v2(old.path(), &index)?;
    let pins = chosen(target.home());
    let pin_dir = hooks.pin_dir.clone().unwrap_or_else(backend::data_dir);

    let next = edit.candidate()?;
    let mut doc = Document::read(next.path())?;
    let mut pending_pins: Vec<PendingPin> = Vec::new();
    let mut holds: Vec<Held> = Vec::new();
    let mut removed: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut staged_ids: BTreeSet<String> = BTreeSet::new();
    let mut swaps: BTreeMap<String, StagedSwap> = BTreeMap::new();
    let mut dropped_ids: BTreeSet<String> = BTreeSet::new();
    let original_doc = Document::read(old.path())?;
    // A mod held again keeps the on/off state the player wanted before its first hold, not the
    // "off" the old hold left it in.
    let holder = Holder {
        home: old.path(),
        wanted: previous_holds
            .iter()
            .filter(|hold| hold.enabled_before())
            .map(|hold| hold.id().to_owned())
            .chain(
                before
                    .iter()
                    .filter(|(_, item)| item.enabled)
                    .map(|(slot, _)| slot.clone()),
            )
            .collect(),
    };

    // Re-offer previously held local or source mods whose exact bytes are still present and not
    // revoked; their old holds are retired, and any that still fail `prove` below are held again
    // with today's target build and reason.
    let rechecked = collect_rechecked_local_holds(previous_holds, &before, &index);
    for slot in &rechecked {
        mod_list::set_enabled(next.path(), slot, true)?;
    }

    for (slot, item) in &before {
        match plan_slot_for_build(&index, slot, item, tag, &pins) {
            SlotPlan::Keep => {}
            SlotPlan::Replace(project, release) => {
                // Stop between releases; the staged candidate is dropped with the edit.
                hooks.switch.cancellation.check()?;
                let step = PlannedV2Release {
                    project_id: project.id.clone(),
                    slug: project.slug.clone(),
                    mod_id: project.mod_id.clone(),
                    name: project.name.clone(),
                    kind: project.kind.clone(),
                    version: release.version.clone(),
                    sha256: release.file.sha256.clone(),
                    size: release.file.size,
                    url: release.file.url.clone(),
                    attestation_url: release.file.attestation.clone(),
                    workflow: release.workflow.clone(),
                    repository_id: project.release_repository(release),
                    repository_name: project.repository.name.clone(),
                    repository_path: project.repository.path.clone(),
                    dependency: doc
                        .row(slot)?
                        .is_some_and(|row| row["dependency"].as_bool() == Some(true)),
                };
                let (entry, pin) = stage_and_verify_v2_release(
                    &index,
                    next.path(),
                    &step,
                    &pin_dir,
                    &BTreeSet::new(),
                    hooks.confirm_publisher_change,
                    V2Transfer {
                        fetch_bytes,
                        verify_release,
                    },
                )
                .map_err(|error| match error {
                    InstallError::PublisherChanged(change) => change_message(&change),
                    other => other.to_string(),
                })?;
                doc.replace(&entry)?;
                staged_ids.insert(slot.clone());
                swaps.insert(
                    slot.clone(),
                    StagedSwap {
                        project_id: project.id.clone(),
                        archive: next
                            .path()
                            .join(staged_folder(&project.kind))
                            .join(format!("{}.smod", project.mod_id)),
                    },
                );
                pending_pins.push(pin);
            }
            SlotPlan::Hold(reason) => {
                mod_list::set_enabled(next.path(), slot, false)?;
                dropped_ids.insert(slot.clone());
                holds.push(holder.hold(slot, item, reason)?);
            }
            SlotPlan::Revoked { reason, owned } => {
                mod_list::set_enabled(next.path(), slot, false)?;
                dropped_ids.insert(slot.clone());
                if owned {
                    let rel = item
                        .package
                        .path
                        .strip_prefix(old.path())
                        .map_err(|e| at(&item.package.path, e))?;
                    let target = next.path().join(rel);
                    if target.exists() {
                        fs::remove_file(&target).map_err(|e| at(&target, e))?;
                    }
                    doc.remove(slot)?;
                    removed.insert(slot.clone(), (item.package.manifest.name.clone(), reason));
                } else {
                    holds.push(holder.hold(
                        slot,
                        item,
                        HeldReason::NoFit {
                            build: tag.to_owned(),
                            detail: Some(reason),
                        },
                    )?);
                }
            }
        }
    }

    // Hold any enabled dependent whose required dependency was held or removed.
    hold_missing_dependents(
        &holder,
        next.path(),
        &before,
        &mut dropped_ids,
        &mut holds,
        tag,
    )?;
    let undo = SwapUndo {
        old_home: old.path(),
        stage_home: next.path(),
        before: &before,
        original_doc: &original_doc,
    };
    undo.held(
        &holds,
        &mut swaps,
        &mut staged_ids,
        &mut doc,
        &mut pending_pins,
    )?;

    doc.write(next.path())?;
    check_staged_v2_candidate(next.path(), &index, |id| staged_ids.contains(id))
        .map_err(|e| e.to_string())?;

    // Prove the candidate image. If an enabled local, source, or unadopted mod fails to fit or
    // compile on `tag`, hold it (and its required dependents) and retry instead of aborting the
    // whole switch.
    let proof = loop {
        (hooks.switch.progress)(InstallProgress::Preparing);
        match edit.prove(
            next.path(),
            target.native_profile(),
            hooks.switch.cancellation,
            |step| build_progress(step, hooks.switch.progress),
            None,
        ) {
            Ok(proof) => break proof,
            Err(crate::native_mods::Failed::Mods {
                left, compiling, ..
            }) => {
                hooks.switch.cancellation.check()?;
                let blamed = classify_prove_failure(&left, compiling, tag);
                let mut held_any = false;
                for (slot, reason) in blamed {
                    if dropped_ids.contains(&slot) || !mod_list::is_enabled(next.path(), &slot) {
                        continue;
                    }
                    let Some(item) = before.get(&slot) else {
                        continue;
                    };
                    mod_list::set_enabled(next.path(), &slot, false)?;
                    dropped_ids.insert(slot.clone());
                    holds.push(holder.hold(&slot, item, reason)?);
                    held_any = true;
                }
                if !held_any {
                    return Err(
                        tr("Sunrise couldn't prove a working mod set for this build.").into(),
                    );
                }
                hold_missing_dependents(
                    &holder,
                    next.path(),
                    &before,
                    &mut dropped_ids,
                    &mut holds,
                    tag,
                )?;
                undo.held(
                    &holds,
                    &mut swaps,
                    &mut staged_ids,
                    &mut doc,
                    &mut pending_pins,
                )?;
                doc.write(next.path())?;
            }
            Err(error) => return Err(error.to_string()),
        }
    };

    // Compute world risks and SwitchQuestion.
    let mut suspect = crate::worlds::SuspectMods::new();
    for slot in &dropped_ids {
        if before.get(slot).is_some_and(|m| m.enabled) {
            suspect.insert(slot.clone(), BTreeSet::new());
        }
    }
    let replaced_packages: Vec<&crate::managed_play::ScannedPackage> = staged_ids
        .iter()
        .filter_map(|id| before.get(id).map(|m| &m.package))
        .collect();
    let (update_drops, _) = crate::browser::sunrise::registry_install::mutations::update_worlds(
        next.path(),
        replaced_packages,
    )?;
    for (id, ns) in update_drops {
        suspect.entry(id).or_default().extend(ns);
    }

    let resources = edit.switch_resources()?;
    let mut order = None;
    let mut compare = |commit: &str| {
        order
            .get_or_insert_with(|| backend::WorldBuildOrder::for_target(tag))
            .compare_saved(commit)
    };
    let (fence, worlds) = Fence::hold(
        target.home(),
        suspect,
        |id| {
            before
                .get(id)
                .map_or("", |item| item.package.manifest.name.as_str())
        },
        Some(TargetAge {
            data_version: backend::prepared_world_version(&resources),
            compare_saved: &mut compare,
        }),
    )?;
    let (held_names, unproved_names) = split_hold_names(&holds);
    let question = SwitchQuestion {
        build: tag.to_owned(),
        worlds,
        held: held_names,
        unproved: unproved_names,
        reasons: holds
            .iter()
            .filter_map(|hold| Some((hold.title().to_owned(), hold.reason().detail()?.to_owned())))
            .collect(),
        removed: removed.values().cloned().collect(),
    };

    let decision = if question.needed() {
        (hooks.switch.approve)(&question)?
    } else {
        SwitchDecision::TurnOff
    };
    if decision == SwitchDecision::Cancel {
        return Ok(Switched::Stayed);
    }

    let (final_proof, final_holds, final_removed) =
        if decision == SwitchDecision::Remove && !holds.is_empty() {
            let mut pruned_doc = Document::read(next.path())?;
            let mut removed_all = removed;
            for hold in &holds {
                let path = next.path().join(hold.path());
                if path.exists() {
                    fs::remove_file(&path).map_err(|e| at(&path, e))?;
                }
                pruned_doc.remove(hold.id())?;
                removed_all.insert(
                    hold.id().to_owned(),
                    (hold.title().to_owned(), hold.reason().label().to_owned()),
                );
            }
            pruned_doc.write(next.path())?;
            let proof = edit
                .prove(
                    next.path(),
                    target.native_profile(),
                    hooks.switch.cancellation,
                    |step| build_progress(step, hooks.switch.progress),
                    None,
                )
                .map_err(|e| e.to_string())?;
            (proof, Vec::new(), removed_all)
        } else {
            (proof, holds, removed)
        };

    let committed_choice = choice.map(|choice| choice.commit(tag)).transpose()?;

    let retired = retired_holds(rechecked, previous_holds, &final_holds);
    let active = edit.publish_switch(
        final_proof,
        hooks.switch.cancellation,
        crate::managed_play::SwitchPublication {
            holds: final_holds,
            removed: final_removed,
            retired,
            build_choice: committed_choice,
        },
        || fence.revalidate(target.home()),
        || {
            (hooks.switch.at_cutoff)()?;
            (hooks.switch.progress)(InstallProgress::Publishing);
            commit_pending_pins(&pin_dir, &pending_pins)?;
            Ok(())
        },
    )?;
    Ok(Switched::Changed(Box::new(active)))
}

/// The earlier holds this switch replaces: the re-offered ones, and every one that is held again,
/// which today's hold replaces so the profile never carries two holds for one mod.
fn retired_holds(
    rechecked: BTreeSet<String>,
    previous_holds: &[Held],
    final_holds: &[Held],
) -> BTreeSet<String> {
    let mut retired = rechecked;
    retired.extend(
        final_holds
            .iter()
            .filter(|hold| previous_holds.iter().any(|old| old.id() == hold.id()))
            .map(|hold| hold.id().to_owned()),
    );
    retired
}

/// Makes holds for one switch: `home` is the profile the mods were read from, and `wanted` the mods
/// the player had on, now or before an earlier hold.
struct Holder<'a> {
    home: &'a Path,
    wanted: BTreeSet<String>,
}

impl Holder<'_> {
    fn hold(&self, slot: &str, item: &InstalledV2Mod, reason: HeldReason) -> Result<Held, String> {
        make_v2_hold(self.home, slot, item, self.wanted.contains(slot), reason)
    }
}

/// A registry replacement staged into the candidate, kept so it can be undone if the mod is held.
struct StagedSwap {
    project_id: String,
    archive: PathBuf,
}

fn staged_folder(kind: &V2Kind) -> &'static str {
    match kind {
        V2Kind::NativeMod => crate::native_mods::INSTALLED,
        _ => mod_list::MODS_DIR,
    }
}

/// Puts back what a staged replacement changed when that mod ends up held, so the hold, which
/// names the old archive, matches the bytes and `installed.json` row on disk.
struct SwapUndo<'a> {
    old_home: &'a Path,
    stage_home: &'a Path,
    before: &'a BTreeMap<String, InstalledV2Mod>,
    original_doc: &'a Document,
}

impl SwapUndo<'_> {
    fn held(
        &self,
        holds: &[Held],
        swaps: &mut BTreeMap<String, StagedSwap>,
        staged_ids: &mut BTreeSet<String>,
        doc: &mut Document,
        pending_pins: &mut Vec<PendingPin>,
    ) -> Result<(), String> {
        for hold in holds {
            let Some(swap) = swaps.remove(hold.id()) else {
                continue;
            };
            let Some(item) = self.before.get(hold.id()) else {
                continue;
            };
            let rel = item
                .package
                .path
                .strip_prefix(self.old_home)
                .map_err(|e| at(&item.package.path, e))?;
            let back = self.stage_home.join(rel);
            if swap.archive != back && swap.archive.exists() {
                fs::remove_file(&swap.archive).map_err(|e| at(&swap.archive, e))?;
            }
            if let Some(parent) = back.parent() {
                fs::create_dir_all(parent).map_err(|e| at(parent, e))?;
            }
            fs::copy(&item.package.path, &back).map_err(|e| at(&back, e))?;
            match self.original_doc.row(hold.id())? {
                Some(row) => doc.set(hold.id(), row.clone())?,
                None => doc.remove(hold.id())?,
            }
            staged_ids.remove(hold.id());
            pending_pins.retain(|pin| pin.project_id != swap.project_id);
        }
        Ok(())
    }
}

pub(super) fn make_v2_hold(
    home: &Path,
    slot: &str,
    item: &InstalledV2Mod,
    wanted: bool,
    reason: HeldReason,
) -> Result<Held, String> {
    let root = if item.package.manifest.native.is_some() {
        HeldRoot::Native
    } else {
        HeldRoot::Content
    };
    let leaf = item
        .package
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| tr("Sunrise couldn't identify the mod archive filename.").to_owned())?
        .to_owned();
    let digest = sha256_bytes(&item.digest)?;
    let source = match &item.ownership {
        V2Ownership::Verified {
            project_id,
            repository,
            ..
        }
        | V2Ownership::AdoptedV1 {
            project_id,
            repository,
            ..
        } => HeldSource::Registry {
            project: project_id.clone(),
            version: item.package.manifest.version.clone(),
            repository: Some(repository.clone()),
        },
        // A mod added from GitHub has no registry project, but its verified receipt says where it
        // came from, and updating it from there later needs that.
        V2Ownership::Local => match github_origin(home, item) {
            Some(origin) => HeldSource::Github {
                owner: origin.owner,
                repo: origin.repo,
                commit: origin.commit,
                version: origin.version,
            },
            None => HeldSource::Local {
                version: item.package.manifest.version.clone(),
            },
        },
    };
    Held::new(
        slot.to_owned(),
        item.package.manifest.name.clone(),
        digest,
        (root, leaf),
        source,
        wanted,
        reason,
    )
}

/// The GitHub receipt of a native mod installed at its usual place, when it matches the archive's
/// exact bytes. A receipt for a mod added from a local folder doesn't count.
fn github_origin(home: &Path, item: &InstalledV2Mod) -> Option<crate::github_mods::Origin> {
    let id = &item.package.manifest.id;
    let canonical = home
        .join(crate::native_mods::INSTALLED)
        .join(format!("{id}.smod"));
    if item.package.path != canonical || item.package.manifest.native.is_none() {
        return None;
    }
    crate::github_mods::origin(home, id).filter(|origin| {
        origin.local_folder_path().is_none()
            && origin.matches_archive(id, &item.package.manifest.version, &item.digest)
    })
}

fn split_hold_names(holds: &[Held]) -> (Vec<String>, Vec<String>) {
    let mut held = Vec::new();
    let mut unproved = Vec::new();
    for hold in holds {
        if hold.reason().unproved() {
            unproved.push(hold.title().to_owned());
        } else {
            held.push(hold.title().to_owned());
        }
    }
    held.sort();
    unproved.sort();
    (held, unproved)
}

fn change_message(change: &PublisherChange) -> String {
    attestation::Error::PublisherChanged(Box::new(change.clone())).to_string()
}

/// Disable any enabled mod whose required dependency was held or removed, recording a hold for it.
fn hold_missing_dependents(
    holder: &Holder<'_>,
    stage_home: &Path,
    before: &BTreeMap<String, InstalledV2Mod>,
    dropped_ids: &mut BTreeSet<String>,
    holds: &mut Vec<Held>,
    tag: &str,
) -> Result<(), String> {
    loop {
        let mut newly_held = Vec::new();
        for (slot, item) in before {
            if dropped_ids.contains(slot) || !mod_list::is_enabled(stage_home, slot) {
                continue;
            }
            let missing_dep = item
                .package
                .manifest
                .dependencies
                .iter()
                .any(|(dep_id, dep)| !dep.details().optional && dropped_ids.contains(dep_id));
            if missing_dep {
                newly_held.push(slot.clone());
            }
        }
        if newly_held.is_empty() {
            break;
        }
        for slot in newly_held {
            mod_list::set_enabled(stage_home, &slot, false)?;
            dropped_ids.insert(slot.clone());
            if let Some(item) = before.get(&slot) {
                holds.push(holder.hold(
                    &slot,
                    item,
                    HeldReason::NoFit {
                        build: tag.to_owned(),
                        detail: None,
                    },
                )?);
            }
        }
    }
    Ok(())
}

/// Previously held local or source mods whose exact archive is still on disk (currently disabled)
/// and not revoked are re-offered on the next build switch; their old holds are retired.
fn collect_rechecked_local_holds(
    previous_holds: &[Held],
    before: &BTreeMap<String, InstalledV2Mod>,
    index: &Index,
) -> BTreeSet<String> {
    let mut rechecked = BTreeSet::new();
    for hold in previous_holds {
        if !matches!(
            hold.source(),
            HeldSource::Local { .. } | HeldSource::Github { .. }
        ) {
            continue;
        }
        let Some(item) = before.get(hold.id()) else {
            continue;
        };
        if item.enabled || !hold.enabled_before() {
            continue;
        }
        let Ok(digest) = sha256_bytes(&item.digest) else {
            continue;
        };
        if &digest != hold.digest() {
            continue;
        }
        if revoked_for_package(index, &item.ownership, &item.digest).is_some() {
            continue;
        }
        rechecked.insert(hold.id().to_owned());
    }
    rechecked
}

/// Convert `native_mods::Failed::Mods` omissions into per-mod `HeldReason`s so a failing local,
/// source, or unadopted mod is held instead of aborting the entire switch.
fn classify_prove_failure(
    left: &[crate::native_mods::LeftOut],
    compiling: bool,
    tag: &str,
) -> Vec<(String, HeldReason)> {
    use crate::native_mods::Why;
    let mut blamed = Vec::new();
    for omission in left {
        let reason = match &omission.why {
            Why::Unfit(detail) if !compiling => HeldReason::NoFit {
                build: tag.to_owned(),
                detail: Some(detail.clone()),
            },
            Why::Unproved(detail) if !compiling => HeldReason::Unproved {
                build: tag.to_owned(),
                detail: Some(detail.clone()),
            },
            Why::DidNotBuild if compiling => HeldReason::BuildFailed {
                build: tag.to_owned(),
            },
            Why::OtherVersion => HeldReason::NoFit {
                build: tag.to_owned(),
                detail: None,
            },
            Why::Clash(other) => HeldReason::NoFit {
                build: tag.to_owned(),
                detail: Some(other.clone()),
            },
            Why::Needs(_) | Why::Unfit(_) | Why::Unproved(_) | Why::DidNotBuild => continue,
        };
        blamed.push((omission.id.clone(), reason));
    }
    blamed
}

#[cfg(test)]
mod tests;
