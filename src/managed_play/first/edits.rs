//! Package edits and explicit Switch use the exact profile journal and staged compiler.
use super::*;
use crate::{
    i18n::tr,
    managed_play::{digest_file, Held, Receipt, ReceiptReport},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
};
use sunrise_launcher_smod::mod_list;

pub(crate) struct Edit<'a> {
    basis: Basis<'a>,
    home: PathBuf,
    before: [u8; 32],
}

enum Basis<'a> {
    Active(Active, Staging<'a>),
    First(FirstAttempt<'a>),
    Pending(Staging<'a>),
    Switch {
        old: Option<Active>,
        target: Box<FirstAttempt<'a>>,
        resources: Resources,
    },
}

/// Receipt and build-choice metadata committed together when publishing a switch edit.
#[derive(Default)]
pub(crate) struct SwitchPublication {
    pub(crate) holds: Vec<Held>,
    pub(crate) removed: BTreeMap<String, (String, String)>,
    /// Earlier holds this switch decided again: re-held ones are among `holds`.
    pub(crate) retired: BTreeSet<String>,
    pub(crate) build_choice: Option<crate::managed_play::CommittedBuildChoice>,
}

pub(crate) struct CandidateImage {
    candidate: PathBuf,
    tag: String,
    inputs: [u8; 32],
}

impl Profile {
    pub(crate) fn begin_edit(
        &mut self,
        choose: impl FnOnce() -> Result<FirstBuild, String>,
        download: impl Fn(f32),
    ) -> Result<Edit<'_>, String> {
        let home = self.home().to_owned();
        let manifest = game_lock(&home)?;
        let before = inputs(&home)?;
        let basis = match self.active()? {
            Some(active) => {
                active.verify_artifacts()?;
                let _references = ReferencePublication::begin(&home)?;
                let targets = [active.base().to_owned(), active.executable().to_owned()]
                    .into_iter()
                    .collect();
                Basis::Active(active, self.0.reserve(Intent::Install, targets)?)
            }
            None => {
                let first = if self.0.has_bootstrap()? {
                    // An interrupted Play already accepted this build; honor it.
                    drop(manifest);
                    self.begin_first(choose, download)?
                } else {
                    // A cancelled Install must not bind later Play to its attempted build.
                    // Reserve the images before transfer without creating a resumable Play selection.
                    let choice = choose()?;
                    let proof = match &choice.selection {
                        Selection::Download { archive } => Some(archive.prove(&choice.base)?),
                        Selection::Cached { .. } => None,
                    };
                    let bound = Bound {
                        base: choice.base,
                        image: choice.image,
                        profile: profile_identity()?,
                        inputs: before,
                        selection: choice.selection,
                        proof,
                    };
                    let stage = {
                        let _references = ReferencePublication::begin(&home)?;
                        if matches!(bound.selection, Selection::Cached { .. }) {
                            bound.verify_image()?;
                        }
                        let targets = [bound.base.clone(), bound.image.clone()]
                            .into_iter()
                            .collect();
                        self.0.reserve(Intent::Install, targets)?
                    };
                    drop(manifest);
                    if let Selection::Download { archive } = &bound.selection {
                        archive.install(
                            &bound.base,
                            bound.proof.as_ref().expect("proved download"),
                            download,
                        )?;
                    }
                    bound.verify_image()?;
                    FirstAttempt {
                        home: home.clone(),
                        bound,
                        stage,
                    }
                };
                if before != first.bound.inputs {
                    return Err(unsettled!());
                }
                Basis::First(first)
            }
        };
        Ok(Edit {
            basis,
            home,
            before,
        })
    }

    /// Stage a files-only edit (archives, sidecars, enabled list) without needing a game build
    /// to be selected or downloaded. Refuses if a first Play bootstrap is unsettled.
    pub(crate) fn begin_pending_edit(&mut self) -> Result<Edit<'_>, String> {
        let home = self.home().to_owned();
        let _manifest = game_lock(&home)?;
        let before = inputs(&home)?;
        let basis = match self.active()? {
            Some(active) => {
                active.verify_artifacts()?;
                let _references = ReferencePublication::begin(&home)?;
                let targets = [active.base().to_owned(), active.executable().to_owned()]
                    .into_iter()
                    .collect();
                Basis::Active(active, self.0.reserve(Intent::Install, targets)?)
            }
            None => {
                if self.0.has_bootstrap()? {
                    return Err(unsettled!());
                }
                let _references = ReferencePublication::begin(&home)?;
                Basis::Pending(self.0.reserve(Intent::Install, BTreeSet::new())?)
            }
        };
        Ok(Edit {
            basis,
            home,
            before,
        })
    }

    /// Reserve one exact, already-prepared target. Download and Prepare never change a profile.
    #[cfg(test)]
    pub(crate) fn begin_switch(&mut self, tag: &str) -> Result<Option<Edit<'_>>, String> {
        self.begin_switch_cancellable(tag, None)
    }

    /// [`Self::begin_switch`], whose resource capture honours `stop` (a switch's Cancel).
    pub(crate) fn begin_switch_cancellable(
        &mut self,
        tag: &str,
        stop: Option<&crate::native_mods::Cancellation>,
    ) -> Result<Option<Edit<'_>>, String> {
        let home = self.home().to_owned();
        let manifest = game_lock(&home)?;
        let before = inputs(&home)?;
        let old = self.active()?;
        if let Some(active) = &old {
            active.verify_artifacts()?;
            if active.base() == tag {
                return Ok(None);
            }
        } else if self.0.has_bootstrap()? {
            return Err(tr("Finish the profile's first Play before switching builds.").into());
        }
        let choice = FirstBuild::installed(tag)?;
        if choice.base != tag || choice.image != tag {
            return Err(tr("Choose a game build, not a compiled mod image.").into());
        }
        let bound = Bound {
            base: choice.base,
            image: choice.image,
            profile: old
                .as_ref()
                .map(|a| a.profile)
                .map_or_else(profile_identity, Ok)?,
            inputs: before,
            selection: choice.selection,
            proof: None,
        };
        let stage = {
            let _references = ReferencePublication::begin(&home)?;
            bound.verify_image()?;
            let mut targets = BTreeSet::from([bound.base.clone()]);
            if let Some(active) = &old {
                targets.extend([active.base().to_owned(), active.executable().to_owned()]);
            }
            self.0.reserve(Intent::Switch, targets)?
        };
        drop(manifest);
        let resources = Resources::capture_cancellable(tag, &stage, stop)?;
        let target = FirstAttempt {
            home: home.clone(),
            bound,
            stage,
        };
        Ok(Some(Edit {
            basis: Basis::Switch {
                old,
                target: Box::new(target),
                resources,
            },
            home,
            before,
        }))
    }
}

pub(crate) fn copy_install_inputs(home: &Path, destination: &Path) -> Result<(), String> {
    for name in [mod_list::DIR, mod_list::NATIVE_DIR] {
        let dir = home.join(name);
        match fs::symlink_metadata(&dir) {
            Ok(_) => super::super::resources::copy_tree(&dir, &destination.join(name), false, 0)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(at(&dir, error)),
        }
    }
    for name in [
        sunrise_launcher_smod::index::browser_state::MANIFEST,
        mod_list::FILE,
    ] {
        if let Some(bytes) = super::super::optional_bytes(&home.join(name))? {
            fs::write(destination.join(name), bytes).map_err(|e| at(destination, e))?;
        }
    }
    Ok(())
}

impl Edit<'_> {
    pub(crate) fn base(&self) -> &str {
        match &self.basis {
            Basis::Active(active, _) => active.base(),
            Basis::First(first) => first.base(),
            Basis::Pending(_) => "",
            Basis::Switch { target, .. } => target.base(),
        }
    }

    pub(crate) fn held(&self, id: &str) -> Option<&crate::managed_play::Held> {
        self.current()?.holds().find(|held| held.id() == id)
    }

    /// A first edit has no committed comparison base; it cannot clear save risk.
    pub(crate) fn current(&self) -> Option<&Active> {
        match &self.basis {
            Basis::Active(active, _) => Some(active),
            Basis::First(_) | Basis::Pending(_) => None,
            Basis::Switch { old, .. } => old.as_ref(),
        }
    }

    pub(crate) fn switch_resources(&self) -> Result<PathBuf, String> {
        match &self.basis {
            Basis::Switch {
                target, resources, ..
            } => Ok(resources.path(target.base())),
            _ => Err(unsettled!()),
        }
    }

    fn stage(&self) -> &Staging<'_> {
        match &self.basis {
            Basis::Active(_, stage) | Basis::Pending(stage) => stage,
            Basis::First(first) => &first.stage,
            Basis::Switch { target, .. } => &target.stage,
        }
    }

    /// Each resolver trial starts with the exact old inputs, including files and rows the
    /// launcher does not understand. Candidate paths never escape this journal attempt.
    pub(crate) fn candidate(&self) -> Result<tempfile::TempDir, String> {
        let _manifest = game_lock(&self.home)?;
        let candidate = tempfile::Builder::new()
            .prefix("candidate-")
            .tempdir_in(self.stage().path())
            .map_err(|e| at(self.stage().path(), e))?;
        copy_install_inputs(&self.home, candidate.path())?;
        if inputs(candidate.path())? != self.before || inputs(&self.home)? != self.before {
            return Err(unsettled!());
        }
        Ok(candidate)
    }

    pub(crate) fn prepare_first(&self, log: &dyn Fn(String)) -> Result<(), String> {
        if let Basis::First(first) = &self.basis {
            first.prepare(log)?;
        }
        Ok(())
    }

    pub(crate) fn publish_pending<G>(
        self,
        candidate: &Path,
        checked_inputs: [u8; 32],
        cancellation: &native_mods::Cancellation,
        authorize: impl FnOnce() -> Result<G, String>,
    ) -> Result<(), String> {
        let _manifest = game_lock(&self.home)?;
        crate::managed_play::recover_game_install(&self.home)?;
        let _references = ReferencePublication::begin(&self.home)?;
        let stage = match self.basis {
            Basis::Active(_, stage) | Basis::Pending(stage) => stage,
            Basis::First(first) => {
                if first.stage.resumes_bootstrap() {
                    return Err(unsettled!());
                }
                first.bound.verify_image()?;
                first.stage
            }
            Basis::Switch { .. } => return Err(unsettled!()),
        };
        if inputs(&self.home)? != self.before || inputs(candidate)? != checked_inputs {
            return Err(unsettled!());
        }
        let files = changed_files(&self.home, candidate)?;
        if files.is_empty() {
            return Ok(());
        }
        // Hold world evidence through the renames and through any journal undo.
        let mut authorization = None;
        stage.seal_files(None, &files)?.publish(|| {
            authorization = Some(authorize()?);
            if inputs(&self.home)? != self.before || inputs(candidate)? != checked_inputs {
                return Err(unsettled!());
            }
            cancellation.begin_publication()
        })
    }

    pub(crate) fn prove(
        &self,
        candidate: &Path,
        mode: native_mods::Profile,
        cancellation: &native_mods::Cancellation,
        progress: impl Fn(native_mods::Step),
        log: Option<&dyn Fn(std::sync::Arc<str>)>,
    ) -> Result<CandidateImage, native_mods::Failed> {
        let before = inputs(candidate)?;
        let preferred = match &self.basis {
            Basis::Active(old, _) => old.executable(),
            Basis::First(first) => first.preferred_image(),
            Basis::Pending(_) => return Err(unsettled!().into()),
            Basis::Switch { target, .. } => target.preferred_image(),
        };
        let tag = image_for_stage(
            self.base(),
            preferred,
            candidate,
            self.stage(),
            mode,
            cancellation,
            progress,
            log,
        )?;
        if inputs(candidate)? != before {
            return Err(unsettled!().into());
        }
        Ok(CandidateImage {
            candidate: candidate.into(),
            tag,
            inputs: before,
        })
    }

    /// Verify the exact live and candidate images plus any extra evidence at cutoff.
    /// Keep the returned authorization guard alive through publication.
    pub(crate) fn publish_install<G>(
        self,
        proof: CandidateImage,
        cancellation: &native_mods::Cancellation,
        restored: &BTreeSet<String>,
        authorize: impl FnOnce() -> Result<G, String>,
    ) -> Result<Active, String> {
        self.publish_install_at(proof, cancellation, restored, authorize, || {})
    }

    pub(crate) fn publish_install_at<G>(
        self,
        proof: CandidateImage,
        cancellation: &native_mods::Cancellation,
        restored: &BTreeSet<String>,
        authorize: impl FnOnce() -> Result<G, String>,
        finishing: impl FnOnce(),
    ) -> Result<Active, String> {
        self.publish_inner(
            proof,
            cancellation,
            (None, restored, None),
            authorize,
            || {
                finishing();
                Ok(())
            },
            |_, _| Ok(()),
        )
    }

    pub(crate) fn publish_switch(
        self,
        proof: CandidateImage,
        cancellation: &native_mods::Cancellation,
        publication: SwitchPublication,
        authorize: impl FnOnce() -> Result<(), String>,
        at_cutoff: impl FnOnce() -> Result<(), String>,
    ) -> Result<Active, String> {
        if !matches!(self.basis, Basis::Switch { .. }) {
            return Err(unsettled!());
        }
        let SwitchPublication {
            holds,
            removed,
            mut retired,
            build_choice,
        } = publication;
        retired.extend(removed.keys().cloned());
        let receipt = if holds.is_empty() && removed.is_empty() {
            None
        } else {
            Some(Receipt {
                id: profile_identity()?,
                report: Some(ReceiptReport::new(Some(self.base().into()), &holds)),
                waiting_for_update: holds,
                removed: removed.into_values().collect(),
                acknowledged: false,
            })
        };
        self.publish_inner(
            proof,
            cancellation,
            (receipt, &retired, build_choice),
            authorize,
            at_cutoff,
            |_, _| Ok(()),
        )
    }

    fn publish_inner<G>(
        self,
        proof: CandidateImage,
        cancellation: &native_mods::Cancellation,
        (receipt, retired_holds, build_choice): (
            Option<Receipt>,
            &BTreeSet<String>,
            Option<crate::managed_play::CommittedBuildChoice>,
        ),
        authorize: impl FnOnce() -> Result<G, String>,
        at_cutoff: impl FnOnce() -> Result<(), String>,
        observe: impl FnMut(usize, profile_txn::Boundary) -> Result<(), String>,
    ) -> Result<Active, String> {
        let resources = match &self.basis {
            Basis::First(first) => Some(Resources::capture(first.base(), &first.stage)?),
            Basis::Active(..) | Basis::Pending(_) | Basis::Switch { .. } => None,
        };
        let _manifest = game_lock(&self.home)?;
        crate::managed_play::recover_game_install(&self.home)?;
        let _references = ReferencePublication::begin(&self.home)?;
        if inputs(&self.home)? != self.before {
            return Err(unsettled!());
        }
        let (mut active, stage) = match self.basis {
            Basis::Pending(_) => return Err(unsettled!()),
            Basis::Active(old, stage) => {
                old.verify_artifacts()?;
                let active = Active::capture(
                    old.base(),
                    &proof.tag,
                    &proof.candidate,
                    proof.inputs,
                    Generation::Advance {
                        previous: &old,
                        resources: None,
                    },
                )?;
                (active, stage)
            }
            Basis::First(first) => {
                first.bound.verify_image()?;
                let generation = Generation::First {
                    profile: first.bound.profile,
                    resources: resources.expect("first resources captured"),
                    variant: first.variant().map(Box::new),
                };
                let active = Active::capture(
                    first.base(),
                    &proof.tag,
                    &proof.candidate,
                    proof.inputs,
                    generation,
                )?;
                (active, first.stage)
            }
            Basis::Switch {
                old,
                target,
                resources,
            } => {
                target.bound.verify_image()?;
                let variant = target.variant().map(Box::new);
                let generation = match &old {
                    Some(previous) => Generation::Switch {
                        previous,
                        resources,
                        variant,
                    },
                    None => Generation::First {
                        profile: target.bound.profile,
                        resources,
                        variant,
                    },
                };
                let active = Active::capture(
                    target.base(),
                    &proof.tag,
                    &proof.candidate,
                    proof.inputs,
                    generation,
                )?;
                (active, target.stage)
            }
        };
        if !retired_holds.is_empty() {
            active.receipts.retain_mut(|receipt| {
                if !receipt.acknowledged
                    && receipt.report.is_none()
                    && receipt
                        .waiting_for_update
                        .iter()
                        .any(|hold| retired_holds.contains(hold.id()))
                {
                    receipt.report = Some(ReceiptReport::new(
                        receipt.base().map(str::to_owned),
                        &receipt.waiting_for_update,
                    ));
                }
                receipt
                    .waiting_for_update
                    .retain(|held| !retired_holds.contains(held.id()));
                !receipt.waiting_for_update.is_empty() || !receipt.acknowledged
            });
        }
        if let Some(receipt) = receipt {
            active.receipts.push(receipt);
        }
        if let Some(choice) = build_choice {
            active.build_choice = Some(choice);
        }
        let files = changed_files(&self.home, &proof.candidate)?;
        let pointer = serde_json::to_vec(&active).map_err(|e| e.to_string())?;
        Active::parse(&pointer)?;
        // The journal drops the cutoff guard before the first live rename. Save locks
        // returned by authorization must also cover the renames and any journal undo.
        let mut authorization = None;
        let result = stage.seal(pointer, &files)?.publish_observed(
            || {
                authorization = Some(authorize()?);
                active.verify_artifacts()?;
                if inputs(&self.home)? != self.before || inputs(&proof.candidate)? != proof.inputs {
                    return Err(unsettled!());
                }
                cancellation.begin_publication()?;
                at_cutoff()
            },
            observe,
        );
        drop(authorization);
        result?;

        Ok(active)
    }
}

/// Changes only to journal-approved leaves can be published. A directory and its opaque local
/// contents may be used by a proof, never adopted as writable package destinations.
pub(crate) fn changed_files(
    home: &Path,
    candidate: &Path,
) -> Result<Vec<(PathBuf, Option<PathBuf>)>, String> {
    let mut names = BTreeSet::from([
        PathBuf::from(sunrise_launcher_smod::index::browser_state::MANIFEST),
        PathBuf::from(mod_list::FILE),
    ]);
    for root in [mod_list::DIR, mod_list::NATIVE_DIR] {
        for profile in [home, candidate] {
            let dir = profile.join(root);
            let entries = match fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(at(&dir, error)),
            };
            for entry in entries {
                let entry = entry.map_err(|e| at(&dir, e))?;
                names.insert(Path::new(root).join(entry.file_name()));
            }
        }
    }
    let mut changes = Vec::new();
    for name in names {
        let old = home.join(&name);
        let next = candidate.join(&name);
        if old.is_dir() || next.is_dir() {
            if super::super::artifact::tree(&old, false)?
                != super::super::artifact::tree(&next, false)?
            {
                return Err(at(&name, unsettled!()));
            }
            continue;
        }
        let before = digest_file(&old)?;
        let after = digest_file(&next)?;
        if before != after {
            changes.push((name, after.map(|_| next)));
        }
    }
    Ok(changes)
}

#[cfg(test)]
mod tests;
