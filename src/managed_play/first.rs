//! First Play freezes the candidate and its proof before downloading or preparing anything.
use super::{
    at, inputs, scan_packages, unsettled, Active, Artifact, Generation, ReferencePublication,
    Resources,
};
use crate::{
    backend::{
        self,
        installation::{BoundVariant, GameDownload, GameProof},
    },
    native_mods,
    process::leases::ProfileHandoff,
    profile_txn::{self, Admission, Bootstrap, Intent, Staging},
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Command,
};
use sunrise_launcher_smod::install_lock::{lock_home, HomeLockGuard};

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum Selection {
    Cached { base: Artifact, preferred: Artifact },
    Download { archive: GameDownload },
}

/// The candidate resolved on the first admitted Play. A committed pointer or an
/// interrupted binding wins over this chooser on every later Play.
pub struct FirstBuild {
    base: String,
    image: String,
    selection: Selection,
}

impl FirstBuild {
    pub fn installed(image: &str) -> Result<Self, String> {
        let base = if native_mods::is_native(image) {
            let data = native_mods::about(image).ok_or_else(unsettled)?;
            if data.target != native_mods::Target::Client || native_mods::is_native(&data.base) {
                return Err(unsettled());
            }
            data.base
        } else {
            image.to_owned()
        };
        let primary = Artifact::capture(&base)?;
        let preferred = if base == image {
            primary.clone()
        } else {
            Artifact::capture(image)?
        };
        Ok(Self {
            base,
            image: image.to_owned(),
            selection: Selection::Cached {
                base: primary,
                preferred,
            },
        })
    }

    pub fn download(release: &backend::Release) -> Result<Self, String> {
        let archive = GameDownload::select(release)?;
        Ok(Self {
            base: release.tag.clone(),
            image: release.tag.clone(),
            selection: Selection::Download { archive },
        })
    }
}

struct Bound {
    base: String,
    image: String,
    profile: [u8; 16],
    inputs: [u8; 32],
    selection: Selection,
    proof: Option<GameProof>,
}

impl Bound {
    fn from_record(record: Bootstrap) -> Result<Self, String> {
        let selection = serde_json::from_value(record.selection).map_err(|e| e.to_string())?;
        let proof = match &selection {
            Selection::Cached { base, preferred } => {
                if record.base != base.tag
                    || record.image != preferred.tag
                    || record.proof.is_some()
                {
                    return Err(unsettled());
                }
                super::validate_tag(&base.tag)?;
                if preferred.tag != base.tag {
                    super::validate_tag(&preferred.tag)?;
                } else if base != preferred {
                    return Err(unsettled());
                }
                None
            }
            Selection::Download { archive } => {
                if record.base != record.image {
                    return Err(unsettled());
                }
                archive.validate(&record.base)?;
                record.proof.map(GameProof::saved).transpose()?
            }
        };
        Ok(Self {
            base: record.base,
            image: record.image,
            profile: record.profile,
            inputs: record.inputs,
            selection,
            proof,
        })
    }

    fn verify_image(&self) -> Result<(), String> {
        match &self.selection {
            Selection::Cached { base, preferred } => {
                if Artifact::capture(&base.tag)? == *base
                    && (base == preferred || Artifact::capture(&preferred.tag)? == *preferred)
                {
                    return Ok(());
                }
            }
            Selection::Download { archive } => {
                if self
                    .proof
                    .as_ref()
                    .is_some_and(|proof| archive.is_installed(&self.base, proof))
                {
                    return Ok(());
                }
            }
        }
        Err(unsettled())
    }
}

pub struct Profile(Admission);

impl Profile {
    /// Acquire profile, then game manifest, then cache publication. Cache scans never
    /// take either profile lock. Keep the same admission through the game handoff.
    pub fn open(home: &Path, external_owner: Option<&Path>) -> Result<Self, String> {
        Self::open_checked(home, external_owner).map_err(Into::into)
    }

    pub(super) fn open_checked(
        home: &Path,
        external_owner: Option<&Path>,
    ) -> Result<Self, super::client::StartError> {
        Self::open_for(home, external_owner, None)
    }

    pub fn open_for(
        home: &Path,
        external_owner: Option<&Path>,
        identity: Option<profile_txn::Identity>,
    ) -> Result<Self, super::client::StartError> {
        let profile = Admission::open_fenced_for(home, external_owner, identity, || {
            let manifest = game_lock(home)?;
            let references = ReferencePublication::begin(home)?;
            Ok((manifest, references))
        })?;
        let _manifest = game_lock(home)?;
        crate::browser::installed::recover_game_install(home)
            .map_err(super::client::StartError::Journal)?;
        Ok(Self(profile))
    }

    pub fn home(&self) -> &Path {
        self.0.home()
    }

    pub fn active(&self) -> Result<Option<Active>, String> {
        profile_txn::references(self.home())?
            .pointer
            .as_deref()
            .map(Active::parse)
            .transpose()
    }

    pub fn handoff(&self) -> ProfileHandoff<'_> {
        self.0.handoff()
    }

    /// Acknowledge one receipt from the generation shown to the player. A retry after
    /// successful publication is a no-op; a stale view cannot dismiss a different generation.
    pub fn acknowledge_receipt(&mut self, shown: &Active, id: [u8; 16]) -> Result<bool, String> {
        self.acknowledge_receipt_at(shown, id, |_, _| Ok(()))
    }

    fn acknowledge_receipt_at(
        &mut self,
        shown: &Active,
        id: [u8; 16],
        observe: impl FnMut(usize, profile_txn::Boundary) -> Result<(), String>,
    ) -> Result<bool, String> {
        let home = self.home().to_owned();
        let _manifest = game_lock(&home)?;
        let _references = ReferencePublication::begin(&home)?;
        let before = profile_txn::references(&home)?
            .pointer
            .ok_or_else(unsettled)?;
        let mut current = Active::parse(&before)?;
        let Some(position) = current
            .receipts
            .iter()
            .position(|receipt| receipt.id == id && !receipt.acknowledged)
        else {
            return Ok(false);
        };
        if current != *shown {
            return Err(unsettled());
        }
        if current.receipts[position].waiting_for_update.is_empty() {
            current.receipts.remove(position);
        } else {
            current.receipts[position].acknowledged = true;
        }
        current.generation = current.generation.checked_add(1).ok_or_else(unsettled)?;
        let next = serde_json::to_vec(&current).map_err(|error| error.to_string())?;
        let targets = [current.base().to_owned(), current.executable().to_owned()]
            .into_iter()
            .collect();
        self.0
            .reserve(Intent::Install, targets)?
            .seal_pointer_replacing(&before, next)?
            .publish_observed(|| Ok(()), observe)?;
        Ok(true)
    }

    /// Bring an already committed profile forward on the same base. The game may have
    /// changed packages and the enable list; failed fits never change the launcher pointer.
    pub fn refresh(
        &mut self,
        mode: native_mods::Profile,
        cancellation: &native_mods::Cancellation,
        progress: impl Fn(native_mods::Step),
        consent: Option<&dyn Fn(&native_mods::Consent) -> bool>,
        registry: Option<&sunrise_launcher_smod::index::Index>,
    ) -> Result<Option<Active>, native_mods::Failed> {
        let home = self.home().to_owned();
        let (old, epoch, repair, stage) = {
            let _manifest = game_lock(&home)?;
            crate::browser::installed::recover_game_install(&home).map_err(String::from)?;
            let Some(old) = self.active()? else {
                return Ok(None);
            };
            old.verify_artifacts()?;
            let epoch = inputs(&home, None)?;
            let repair = super::inputs::browser_repair(&home, &scan_packages(&home)?, registry)?;
            if old.inputs == epoch && repair.is_none() {
                super::verify_included(old.base(), old.executable(), &home)?;
                if native_mods::is_native(old.executable()) {
                    cancellation.admit_native_work()?;
                }
                return Ok(Some(old));
            }
            let _references = ReferencePublication::begin(&home)?;
            let tags = [old.base().to_owned(), old.executable().to_owned()]
                .into_iter()
                .collect();
            (old, epoch, repair, self.0.reserve(Intent::Install, tags)?)
        };
        let image = image_for_stage(
            old.base(),
            old.executable(),
            &home,
            &stage,
            mode,
            cancellation,
            progress,
            consent,
        )?;
        let _manifest = game_lock(&home)?;
        crate::browser::installed::recover_game_install(&home).map_err(String::from)?;
        let _references = ReferencePublication::begin(&home)?;
        let active = Active::capture(old.base(), &image, &home, epoch, Generation::Advance(&old))?;
        Ok(Some(publish_generation(
            stage,
            active,
            &home,
            epoch,
            repair,
            cancellation,
        )?))
    }

    /// Recheck the actual committed generation immediately before creating its child.
    /// The same profile admission, game-manifest lock and native grant span the cutover.
    pub fn verified_handoff<T, E: From<String>>(
        &self,
        active: &Active,
        cancellation: &native_mods::Cancellation,
        spawn: impl FnOnce(ProfileHandoff<'_>) -> Result<T, E>,
    ) -> Result<T, E> {
        let home = self.home();
        let _manifest = game_lock(home)?;
        crate::browser::installed::recover_game_install(home).map_err(String::from)?;
        if self.active()?.as_ref() != Some(active) || inputs(home, None)? != active.inputs {
            return Err(unsettled().into());
        }
        active.verify_artifacts()?;
        super::verify_included(active.base(), active.executable(), home)?;
        if native_mods::is_native(active.executable()) {
            cancellation.admit_native_work().map_err(String::from)?;
        }
        let _permission = cancellation.publication_guard()?;
        spawn(self.handoff())
    }

    pub fn begin_first(
        &mut self,
        choose: impl FnOnce() -> Result<FirstBuild, String>,
        progress: impl Fn(f32),
    ) -> Result<FirstAttempt<'_>, String> {
        self.begin_requested(None, choose, progress)
    }

    pub(super) fn begin_requested(
        &mut self,
        requested: Option<&str>,
        choose: impl FnOnce() -> Result<FirstBuild, String>,
        progress: impl Fn(f32),
    ) -> Result<FirstAttempt<'_>, String> {
        let home = self.home().to_path_buf();
        let mut admitted = None;
        let initial = {
            let _manifest = game_lock(&home)?;
            scan_packages(&home)?;
            let epoch = inputs(&home, None)?;
            let initial = self.0.bind_bootstrap(
                || {
                    let choice = choose()?;
                    assert_requested(requested, &choice.base, &choice.image)?;
                    let profile = profile_identity()?;
                    let selection =
                        serde_json::to_value(&choice.selection).map_err(|e| e.to_string())?;
                    Ok(Bootstrap::new(
                        choice.base,
                        choice.image,
                        profile,
                        epoch,
                        selection,
                    ))
                },
                |record| {
                    let guard = ReferencePublication::begin(&home)?;
                    let bound = Bound::from_record(record.clone())?;
                    if matches!(bound.selection, Selection::Cached { .. }) {
                        bound.verify_image()?;
                    }
                    admitted = Some(bound);
                    Ok(guard)
                },
            )?;
            if initial.inputs != epoch {
                return Err(unsettled());
            }
            initial
        };
        let mut bound = admitted.map_or_else(|| Bound::from_record(initial), Ok)?;
        assert_requested(requested, &bound.base, &bound.image)?;
        if let Selection::Download { archive } = &bound.selection {
            if bound.proof.is_none() {
                let proof = archive.prove(&bound.base)?;
                let _references = ReferencePublication::begin(&home)?;
                self.0
                    .prove_bootstrap(|_| serde_json::to_value(&proof).map_err(|e| e.to_string()))?;
                bound.proof = Some(proof);
            }
            archive.install(
                &bound.base,
                bound.proof.as_ref().expect("saved download proof"),
                progress,
            )?;
        }
        let stage = {
            let _manifest = game_lock(&home)?;
            let _references = ReferencePublication::begin(&home)?;
            if inputs(&home, None)? != bound.inputs {
                return Err(unsettled());
            }
            bound.verify_image()?;
            self.0.resume_bootstrap()?
        };
        Ok(FirstAttempt { home, bound, stage })
    }
}

fn profile_identity() -> Result<[u8; 16], String> {
    let mut profile = [0; 16];
    while profile == [0; 16] {
        getrandom::fill(&mut profile).map_err(|e| e.to_string())?;
    }
    Ok(profile)
}

pub(super) fn assert_requested(
    requested: Option<&str>,
    base: &str,
    image: &str,
) -> Result<(), String> {
    if let Some(requested) = requested.filter(|tag| *tag != base && *tag != image) {
        return Err(crate::i18n::trf(
            "This profile is bound to {build}, not {requested}. Change the profile's build before playing another one.",
            &[("build", base), ("requested", requested)],
        ));
    }
    Ok(())
}

pub struct FirstAttempt<'a> {
    home: std::path::PathBuf,
    bound: Bound,
    stage: Staging<'a>,
}

impl FirstAttempt<'_> {
    pub fn base(&self) -> &str {
        &self.bound.base
    }
    pub fn preferred_image(&self) -> &str {
        &self.bound.image
    }

    pub fn handoff(&self) -> ProfileHandoff<'_> {
        self.stage.handoff()
    }

    pub(super) fn variant(&self) -> Option<BoundVariant> {
        match &self.bound.selection {
            Selection::Cached { .. } => None,
            Selection::Download { archive } => archive.variant(self.bound.proof.as_ref()?),
        }
    }

    /// Verify under the game-manifest lock before checking or compiling this first set.
    pub(super) fn verify_inputs(&self) -> Result<(), String> {
        if inputs(&self.home, None)? != self.bound.inputs {
            return Err(unsettled());
        }
        Ok(())
    }

    pub fn prepare(&self, log: &dyn Fn(String)) -> Result<(), String> {
        let base = self.base();
        if backend::check_prepared(base) {
            return Ok(());
        }
        let preparation = backend::prepare_lock(base)?;
        backend::reuse_prepared_for_play(base);
        if backend::check_prepared(base) {
            return Ok(());
        }
        let minecraft = backend::Settings::load()
            .minecraft_dir
            .unwrap_or_else(|| crate::preparer::minecraft_dir(None));
        let handoff = preparation.handoff(Some(self.handoff()));
        let (command, minecraft) = backend::preparer_staged(
            base,
            &minecraft,
            backend::MinecraftFrom::Install,
            log,
            &handoff,
        )?;
        backend::sounds_staged(base, &minecraft, log, &handoff);
        self.run_prepare(command, &preparation, log)
    }

    fn run_prepare(
        &self,
        command: Command,
        preparation: &crate::preparer::Preparation,
        log: &dyn Fn(String),
    ) -> Result<(), String> {
        crate::preparer::run_staged(command, preparation.handoff(Some(self.handoff())), log)?;
        backend::forget_prepared();
        backend::need_prepared_locked(self.base(), preparation)
    }

    pub fn exact_image(
        &self,
        mode: native_mods::Profile,
        cancellation: &native_mods::Cancellation,
        progress: impl Fn(native_mods::Step),
        consent: Option<&dyn Fn(&native_mods::Consent) -> bool>,
    ) -> Result<String, native_mods::Failed> {
        image_for_stage(
            self.base(),
            self.preferred_image(),
            &self.home,
            &self.stage,
            mode,
            cancellation,
            progress,
            consent,
        )
    }

    /// No failed or partially compiled output enters a generation. Keep the worker's
    /// original native permission revision through the last check and publication.
    pub fn commit(
        self,
        image: &str,
        cancellation: &native_mods::Cancellation,
    ) -> Result<Active, String> {
        if native_mods::is_native(image) {
            cancellation.admit_native_work().map_err(String::from)?;
        }
        // The pending binding already reserves the base. Do not hold the cache-wide
        // publication fence while waiting on a preparer child for this one version.
        self.bound.verify_image()?;
        let resources = Resources::capture(self.base(), &self.stage)?;
        let _manifest = game_lock(&self.home)?;
        crate::browser::installed::recover_game_install(&self.home)?;
        let _references = ReferencePublication::begin(&self.home)?;
        self.bound.verify_image()?;
        let active = Active::capture(
            &self.bound.base,
            image,
            &self.home,
            self.bound.inputs,
            Generation::First {
                profile: self.bound.profile,
                resources,
                variant: self.variant().map(Box::new),
            },
        )?;
        let registry = crate::browser::sunrise::fetched_index();
        let repair = super::inputs::browser_repair(
            &self.home,
            &scan_packages(&self.home)?,
            registry.as_deref(),
        )?;
        publish_generation(
            self.stage,
            active,
            &self.home,
            self.bound.inputs,
            repair,
            cancellation,
        )
    }
}

/// The caller holds the profile, game manifest and reference fence. A stale row is a postimage
/// of this transaction, not a standalone restore. Its published bytes are what the pointer hashes.
fn publish_generation(
    stage: Staging<'_>,
    mut active: Active,
    home: &Path,
    epoch: [u8; 32],
    repair: Option<Vec<u8>>,
    cancellation: &native_mods::Cancellation,
) -> Result<Active, String> {
    let mut files = Vec::new();
    if let Some(bytes) = repair {
        active.inputs = inputs(home, Some(&bytes))?;
        let candidate = stage.path().join("browser-repair");
        profile_txn::durable::replace(stage.path(), &candidate, &bytes)
            .map_err(|error| at(&candidate, error))?;
        files.push((
            PathBuf::from(crate::browser::installed::MANIFEST),
            Some(candidate),
        ));
    }
    let pointer = serde_json::to_vec(&active).map_err(|e| e.to_string())?;
    stage.seal(pointer, &files)?.publish(|| {
        active.verify_artifacts()?;
        if inputs(home, None)? != epoch {
            return Err(unsettled());
        }
        cancellation.publication_guard()
    })?;
    Ok(active)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn image_for_stage(
    base: &str,
    preferred: &str,
    home: &Path,
    stage: &Staging<'_>,
    mode: native_mods::Profile,
    cancellation: &native_mods::Cancellation,
    progress: impl Fn(native_mods::Step),
    consent: Option<&dyn Fn(&native_mods::Consent) -> bool>,
) -> Result<String, native_mods::Failed> {
    if super::verify_included(base, preferred, home).is_ok() {
        if native_mods::is_native(preferred) {
            cancellation.admit_native_work()?;
        }
        return Ok(preferred.to_owned());
    }
    if crate::native_status::enabled(home).is_empty() {
        return Ok(base.to_owned());
    }
    native_mods::build_exact_client(base, home, mode, stage, cancellation, progress, consent)
}

pub(super) fn game_lock(home: &Path) -> Result<HomeLockGuard, String> {
    lock_home(home).map_err(|e| at(home, e))
}

mod edits;
pub(crate) use edits::{CandidateImage, Edit};

#[cfg(test)]
mod tests;
