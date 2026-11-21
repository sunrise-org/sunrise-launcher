//! Same-base package edits use the exact profile journal and the existing staged compiler.
use super::*;
use crate::{browser::installed::MANIFEST, managed_play::digest_file};
use std::{collections::BTreeSet, fs, io};
use sunrise_launcher_smod::mod_list;

pub(crate) struct Edit<'a> {
    basis: Basis<'a>,
    home: PathBuf,
    before: [u8; 32],
}

enum Basis<'a> {
    Active(Active, Staging<'a>),
    First(FirstAttempt<'a>),
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
        log: &dyn Fn(String),
    ) -> Result<Edit<'_>, String> {
        let home = self.home().to_owned();
        let manifest = game_lock(&home)?;
        let before = inputs(&home, None)?;
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
                    return Err(unsettled());
                }
                first.prepare(log)?;
                Basis::First(first)
            }
        };
        Ok(Edit {
            basis,
            home,
            before,
        })
    }
}

impl Edit<'_> {
    pub(crate) fn base(&self) -> &str {
        match &self.basis {
            Basis::Active(active, _) => active.base(),
            Basis::First(first) => first.base(),
        }
    }

    pub(crate) fn held(&self, id: &str) -> Option<&crate::managed_play::Held> {
        self.active_basis()?.holds().find(|held| held.id() == id)
    }

    /// A first edit has no committed comparison base; it cannot clear save risk.
    pub(crate) fn active_basis(&self) -> Option<&Active> {
        match &self.basis {
            Basis::Active(active, _) => Some(active),
            Basis::First(_) => None,
        }
    }

    fn stage(&self) -> &Staging<'_> {
        match &self.basis {
            Basis::Active(_, stage) => stage,
            Basis::First(first) => &first.stage,
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
        for name in [mod_list::DIR, mod_list::NATIVE_DIR] {
            let dir = self.home.join(name);
            match fs::symlink_metadata(&dir) {
                Ok(_) => super::super::resources::copy_tree(
                    &dir,
                    &candidate.path().join(name),
                    false,
                    0,
                )?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                Err(error) => return Err(at(&dir, error)),
            }
        }
        for name in [MANIFEST, mod_list::FILE] {
            if let Some(bytes) = super::super::optional_bytes(&self.home.join(name))? {
                fs::write(candidate.path().join(name), bytes)
                    .map_err(|e| at(candidate.path(), e))?;
            }
        }
        if inputs(candidate.path(), None)? != self.before
            || inputs(&self.home, None)? != self.before
        {
            return Err(unsettled());
        }
        let registry = crate::browser::sunrise::fetched_index();
        if let Some(repaired) = super::super::inputs::browser_repair(
            candidate.path(),
            &scan_packages(candidate.path())?,
            registry.as_deref(),
        )? {
            fs::write(candidate.path().join(MANIFEST), repaired)
                .map_err(|e| at(candidate.path(), e))?;
        }
        Ok(candidate)
    }

    pub(crate) fn prove(
        &self,
        candidate: &Path,
        mode: native_mods::Profile,
        cancellation: &native_mods::Cancellation,
        progress: impl Fn(native_mods::Step),
        consent: Option<&dyn Fn(&native_mods::Consent) -> bool>,
    ) -> Result<CandidateImage, native_mods::Failed> {
        let before = inputs(candidate, None)?;
        let preferred = match &self.basis {
            Basis::Active(old, _) => old.executable(),
            Basis::First(first) => first.preferred_image(),
        };
        let tag = image_for_stage(
            self.base(),
            preferred,
            candidate,
            self.stage(),
            mode,
            cancellation,
            progress,
            consent,
        )?;
        if inputs(candidate, None)? != before {
            return Err(unsettled().into());
        }
        Ok(CandidateImage {
            candidate: candidate.into(),
            tag,
            inputs: before,
        })
    }

    /// Keep one authoritative commit. Verify the same live and candidate bytes at cutoff,
    /// including any other approval (e.g. registry evidence or a world question).
    pub(crate) fn publish<G>(
        self,
        proof: CandidateImage,
        cancellation: &native_mods::Cancellation,
        retire_holds: &BTreeSet<String>,
        authorize: impl FnOnce() -> Result<G, String>,
    ) -> Result<Active, String> {
        self.publish_observed(proof, cancellation, retire_holds, authorize, |_, _| Ok(()))
    }

    fn publish_observed<G>(
        self,
        proof: CandidateImage,
        cancellation: &native_mods::Cancellation,
        retire_holds: &BTreeSet<String>,
        authorize: impl FnOnce() -> Result<G, String>,
        observe: impl FnMut(usize, profile_txn::Boundary) -> Result<(), String>,
    ) -> Result<Active, String> {
        let resources = match &self.basis {
            Basis::First(first) => Some(Resources::capture(first.base(), &first.stage)?),
            Basis::Active(..) => None,
        };
        let _manifest = game_lock(&self.home)?;
        crate::browser::installed::recover_game_install(&self.home)?;
        let _references = ReferencePublication::begin(&self.home)?;
        if inputs(&self.home, None)? != self.before {
            return Err(unsettled());
        }
        let (mut active, stage) = match self.basis {
            Basis::Active(old, stage) => {
                old.verify_artifacts()?;
                let active = Active::capture(
                    old.base(),
                    &proof.tag,
                    &proof.candidate,
                    proof.inputs,
                    Generation::Advance(&old),
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
        };
        active.receipts.retain_mut(|receipt| {
            receipt
                .waiting_for_update
                .retain(|held| !retire_holds.contains(held.id()));
            !receipt.waiting_for_update.is_empty()
                || (!receipt.acknowledged && !receipt.removed.is_empty())
        });
        let files = changed_files(&self.home, &proof.candidate)?;
        let pointer = serde_json::to_vec(&active).map_err(|e| e.to_string())?;
        // The journal drops the cutoff guard before the first live rename. Save locks
        // returned by authorization must also cover the renames and any journal undo.
        let mut authorization = None;
        let result = stage.seal(pointer, &files)?.publish_observed(
            || {
                authorization = Some(authorize()?);
                active.verify_artifacts()?;
                if inputs(&self.home, None)? != self.before
                    || inputs(&proof.candidate, None)? != proof.inputs
                {
                    return Err(unsettled());
                }
                cancellation.publication_guard()
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
fn changed_files(home: &Path, candidate: &Path) -> Result<Vec<(PathBuf, Option<PathBuf>)>, String> {
    let mut names = BTreeSet::from([PathBuf::from(MANIFEST), PathBuf::from(mod_list::FILE)]);
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
                return Err(at(&name, unsettled()));
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
