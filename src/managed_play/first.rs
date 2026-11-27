//! First Play freezes the candidate and its proof before downloading or preparing anything.
use super::{
    at, inputs, resources::PreparedResources, scan_packages, unsettled, Active, Artifact,
    Generation, HeldReason, ReferencePublication, Resources,
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
use std::path::{Path, PathBuf};
use sunrise_launcher_smod::install_lock::{lock_home, HomeLockGuard};

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum Selection {
    Cached { base: Artifact, preferred: Artifact },
    Download { archive: GameDownload },
}

/// The candidate resolved on the first admitted Play. A committed pointer or an
/// interrupted binding wins over this chooser on every later Play.
#[derive(Debug, PartialEq, Eq)]
pub struct FirstBuild {
    base: String,
    image: String,
    selection: Selection,
}

impl FirstBuild {
    pub(crate) fn base_tag(&self) -> &str {
        &self.base
    }

    /// Two cached choices prove the same exact base bytes, not merely the same tag.
    pub(crate) fn same_cached_base(&self, other: &Self) -> bool {
        self.base == other.base
            && matches!((&self.selection, &other.selection),
            (Selection::Cached { base: a, .. }, Selection::Cached { base: b, .. }) if a == b)
    }

    pub fn installed(image: &str) -> Result<Self, String> {
        let base = if native_mods::is_native(image) {
            let data = native_mods::about(image).ok_or_else(|| unsettled!())?;
            if data.target != native_mods::Target::Client || native_mods::is_native(&data.base) {
                return Err(unsettled!());
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
                    return Err(unsettled!());
                }
                super::validate_tag(&base.tag)?;
                if preferred.tag != base.tag {
                    super::validate_tag(&preferred.tag)?;
                } else if base != preferred {
                    return Err(unsettled!());
                }
                None
            }
            Selection::Download { archive } => {
                if record.base != record.image {
                    return Err(unsettled!());
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
        Err(unsettled!())
    }
}

pub struct Profile(Admission);

fn check_identity(
    actual: profile_txn::Identity,
    expected: profile_txn::Identity,
) -> Result<(), String> {
    if actual == expected {
        Ok(())
    } else {
        Err(crate::instances::lifecycle::changed())
    }
}

impl Profile {
    /// Acquire profile, then game manifest, then cache publication. Cache scans never
    /// take either profile lock. Keep the same admission through the game handoff.
    pub fn open(home: &Path, external_owner: Option<&Path>) -> Result<Self, String> {
        let expected = profile_txn::Identity::of(home)?;
        Self::open_using(
            &backend::game_home(),
            &backend::data_dir(),
            home,
            external_owner,
            |actual| check_identity(actual, expected),
        )
        .map_err(Into::into)
    }

    /// Admit the exact home accepted before an Install entered its queue.
    pub fn open_for(
        home: &Path,
        external_owner: Option<&Path>,
        expected: Option<profile_txn::Identity>,
    ) -> Result<Self, super::client::StartError> {
        let changed = || {
            crate::i18n::tr(
                "This profile was replaced while the request was waiting. Nothing was changed.",
            )
            .to_owned()
        };
        let expected = match expected {
            Some(id) if profile_txn::Identity::of(home).ok() == Some(id) => id,
            Some(_) => return Err(changed().into()),
            None => profile_txn::Identity::of(home)?,
        };
        Self::open_using(
            &backend::game_home(),
            &backend::data_dir(),
            home,
            external_owner,
            |actual| {
                if actual == expected {
                    Ok(())
                } else {
                    Err(changed())
                }
            },
        )
    }

    pub(crate) fn open_at(game: &Path, cache: &Path, home: &Path) -> Result<Self, String> {
        let expected = profile_txn::Identity::of(home)?;
        Self::open_using(game, cache, home, None, |actual| {
            check_identity(actual, expected)
        })
        .map_err(Into::into)
    }

    /// Admit a previously selected profile before a launcher operation touches its files.
    pub fn open_selected(instance: &crate::instances::Instance) -> Result<Self, String> {
        Self::open_instance(&backend::game_home(), &backend::data_dir(), instance)
            .map_err(Into::into)
    }

    pub(crate) fn open_instance(
        game: &Path,
        cache: &Path,
        instance: &crate::instances::Instance,
    ) -> Result<Self, super::client::StartError> {
        use crate::instances::{lifecycle::changed, Kind};
        let expected = match instance.incarnation {
            Some(id) => id,
            None if instance.kind == Kind::Default => {
                crate::instances::Incarnation::of(&instance.work, &instance.home)?
            }
            None => return Err(changed().into()),
        };
        Self::open_using(game, cache, &instance.home, None, |actual| {
            expected.check(instance, actual)
        })
    }

    fn open_using(
        game: &Path,
        cache: &Path,
        home: &Path,
        external_owner: Option<&Path>,
        selected: impl FnOnce(profile_txn::Identity) -> Result<(), String>,
    ) -> Result<Self, super::client::StartError> {
        let owner = crate::instances::lifecycle::owner_file(game, home)?;
        let profile =
            Admission::open_fenced(home, owner.as_deref().or(external_owner), |actual| {
                selected(actual)?;
                let manifest = game_lock(home)?;
                let references = ReferencePublication::begin_at(cache, game, home)?;
                Ok((manifest, references))
            })?;
        let _manifest = game_lock(home)?;
        crate::browser::installed::recover_game_install(home)
            .map_err(super::client::StartError::Journal)?;
        Ok(Self(profile))
    }

    pub(super) fn admitted_instance(
        &self,
        instance: &crate::instances::Instance,
    ) -> crate::instances::Instance {
        instance.after_admission(self.0.identity())
    }

    pub(crate) fn lock_inputs(&self) -> Result<HomeLockGuard, String> {
        game_lock(self.home())
    }

    pub(crate) fn has_accepted_bootstrap(&self) -> Result<bool, String> {
        self.0.has_bootstrap()
    }

    pub(crate) fn has_pending_bootstrap(&self) -> Result<bool, String> {
        // Admission already settled any completed or abandoned file operation.
        Ok(!profile_txn::references(self.home())?.targets.is_empty())
    }

    pub(crate) fn stage_copy(
        &self,
        mut active: Active,
        destination: &Path,
    ) -> Result<Active, String> {
        active.verify_artifacts()?;
        if inputs(self.home())? != active.inputs || inputs(destination)? != active.inputs {
            return Err(unsettled!());
        }
        getrandom::fill(&mut active.profile).map_err(|e| e.to_string())?;
        if active.profile == [0; 16] {
            return Err(unsettled!());
        }
        active.generation = 1;
        active.receipts.clear();
        active.build_choice = None;
        let bytes = serde_json::to_vec(&active).map_err(|e| e.to_string())?;
        profile_txn::write_copy_pointer(destination, &bytes)?;
        Ok(active)
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

    /// Enable only the requested mods that aren't held by Switch. An exact hold is retired by
    /// a proved replacement, not by turning on its old ID from a stale view or command.
    pub fn enable_unheld<'a>(&self, ids: &[&'a str]) -> Result<Vec<(&'a str, HeldReason)>, String> {
        let _manifest = game_lock(self.home())?;
        let active = self.active()?;
        let mut go = Vec::new();
        let mut skipped = Vec::new();
        for &id in ids {
            if let Some(hold) = active
                .as_ref()
                .and_then(|active| active.holds().find(|hold| hold.id() == id))
            {
                skipped.push((id, hold.reason().clone()));
            } else {
                go.push(id);
            }
        }
        if !go.is_empty() {
            crate::browser::sunrise::check_activation(self.home(), active.as_ref(), &go)?;
            crate::browser::mod_list::set_all_enabled(self.home(), &go, true)?;
        }
        Ok(skipped)
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
            .ok_or_else(|| unsettled!())?;
        let mut current = Active::parse(&before)?;
        let Some(position) = current
            .receipts
            .iter()
            .position(|receipt| receipt.id == id && !receipt.acknowledged)
        else {
            return Ok(false);
        };
        if current != *shown {
            return Err(unsettled!());
        }
        if current.receipts[position].waiting_for_update.is_empty() {
            current.receipts.remove(position);
        } else {
            current.receipts[position].acknowledged = true;
        }
        current.generation = current
            .generation
            .checked_add(1)
            .ok_or_else(|| unsettled!())?;
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
    pub(crate) fn refresh_for_play(
        &mut self,
        mode: native_mods::Profile,
        cancellation: &native_mods::Cancellation,
        progress: impl Fn(native_mods::Step),
        log: Option<&dyn Fn(std::sync::Arc<str>)>,
        replacement: Option<Resources>,
    ) -> Result<Option<(Active, bool)>, RefreshError> {
        let home = self.home().to_owned();
        let (old, epoch, stage, verified_resources) = {
            let _manifest = game_lock(&home)?;
            crate::browser::installed::recover_game_install(&home).map_err(String::from)?;
            let Some(old) = self.active()? else {
                return Ok(None);
            };
            // The prepared files are hashed once: before native compilation when refresh
            // would compile, otherwise at `verified_handoff`. A damaged snapshot is never
            // copied forward into a new digest.
            old.verify_binaries()?;
            let epoch = inputs(&home)?;
            super::inputs::check_browser_rows(&home, &scan_packages(&home)?)?;
            if old.inputs == epoch && replacement.is_none() {
                super::verify_included(old.base(), old.executable(), &home)?;
                return Ok(Some((old, false)));
            }
            let verified_resources = if replacement.is_none()
                && needs_native_build(old.base(), old.executable(), &home)
            {
                old.resources
                    .verify(old.base())
                    .map_err(RefreshError::Snapshot)?;
                true
            } else {
                false
            };
            let _references = ReferencePublication::begin(&home)?;
            let tags = [old.base().to_owned(), old.executable().to_owned()]
                .into_iter()
                .collect();
            (
                old,
                epoch,
                self.0.reserve(Intent::Install, tags)?,
                verified_resources,
            )
        };
        let image = image_for_stage(
            old.base(),
            old.executable(),
            &home,
            &stage,
            mode,
            cancellation,
            progress,
            log,
        )?;
        let _manifest = game_lock(&home)?;
        crate::browser::installed::recover_game_install(&home).map_err(String::from)?;
        let _references = ReferencePublication::begin(&home)?;
        let active = Active::capture(
            old.base(),
            &image,
            &home,
            epoch,
            Generation::Advance {
                previous: &old,
                resources: replacement,
            },
        )?;
        let published = publish_generation(stage, active, &home, epoch, cancellation)?;
        Ok(Some((published, verified_resources)))
    }

    /// Recheck the actual committed generation immediately before creating its child.
    /// The same profile admission, game-manifest lock and native grant span the cutover.
    pub fn verified_handoff<T, E: From<String>>(
        &self,
        active: &Active,
        cancellation: &native_mods::Cancellation,
        spawn: impl FnOnce(ProfileHandoff<'_>) -> Result<T, E>,
    ) -> Result<T, E> {
        self.verified_handoff_for(active, None, cancellation, spawn)
    }

    pub(super) fn verified_handoff_for<T, E: From<String>>(
        &self,
        active: &Active,
        proof: Option<VerifiedSnapshot>,
        cancellation: &native_mods::Cancellation,
        spawn: impl FnOnce(ProfileHandoff<'_>) -> Result<T, E>,
    ) -> Result<T, E> {
        let _manifest = self
            .verify_handoff(active, proof, cancellation)
            .map_err(|error| E::from(String::from(error)))?;
        spawn(self.handoff())
    }

    pub(super) fn verify_handoff(
        &self,
        active: &Active,
        proof: Option<VerifiedSnapshot>,
        cancellation: &native_mods::Cancellation,
    ) -> Result<HomeLockGuard, super::StartError> {
        let home = self.home();
        let checking = crate::launcher_log::step("Verifying the game and its prepared files");
        let manifest = game_lock(home)?;
        checking.note("the profile's lock is held");
        let verified: Result<(), super::StartError> = (|| {
            crate::browser::installed::recover_game_install(home).map_err(String::from)?;
            if self.active()?.as_ref() != Some(active) || inputs(home)? != active.inputs {
                return Err(unsettled!().into());
            }
            // First Play (or a fresh prepare seal) proved this exact snapshot, and the proof
            // still owns the game-manifest lock from that check through this handoff.
            match &proof {
                Some(proof) if &proof.active == active && proof.home.as_path() == home => {
                    active.verify_binaries()?;
                }
                Some(_) => return Err(unsettled!().into()),
                None => {
                    active.verify_binaries()?;
                    active
                        .resources
                        .verify(active.base())
                        .map_err(super::StartError::Snapshot)?;
                }
            }
            active
                .resources
                .check_play_output(active.base())
                .map_err(|issue| issue.message(|| unsettled!()))?;
            super::verify_included(active.base(), active.executable(), home)?;
            Ok(())
        })();
        // Drop `proof` while `manifest` holds the re-entrant lock so `manifest` is the sole
        // remaining guard on this thread.
        drop(proof);
        match &verified {
            Ok(()) => checking.done(""),
            Err(why) => checking.failed(match why {
                super::StartError::Snapshot(detail)
                | super::StartError::UnknownSpawn(detail)
                | super::StartError::Other(detail) => detail.as_str(),
                _ => "verification failed",
            }),
        }
        verified?;
        cancellation.check()?;
        Ok(manifest)
    }

    pub(super) fn release_failed_first_play(&self) -> Result<(), String> {
        let home = self.home();
        let _manifest = game_lock(home)?;
        let _references = ReferencePublication::begin(home)?;
        self.0.release_failed_bootstrap()
    }

    pub fn begin_first(
        &mut self,
        choose: impl FnOnce() -> Result<FirstBuild, String>,
        progress: impl Fn(f32),
    ) -> Result<FirstAttempt<'_>, String> {
        let home = self.home().to_path_buf();
        let mut admitted = None;
        let initial = {
            let _manifest = game_lock(&home)?;
            scan_packages(&home)?;
            let epoch = inputs(&home)?;
            let initial = self.0.bind_bootstrap(
                || {
                    let choice = choose()?;
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
                return Err(unsettled!());
            }
            initial
        };
        let mut bound = admitted.map_or_else(|| Bound::from_record(initial), Ok)?;
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
            if inputs(&home)? != bound.inputs {
                return Err(unsettled!());
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

pub struct FirstAttempt<'a> {
    home: std::path::PathBuf,
    bound: Bound,
    stage: Staging<'a>,
}

/// A committed generation and the manifest lock under which its snapshot was either
/// verified or freshly sealed in `prepare`. Never persisted: a later Play must verify
/// the saved resources again.
pub(super) struct VerifiedSnapshot {
    pub(super) active: Active,
    home: PathBuf,
    _manifest: HomeLockGuard,
}

impl VerifiedSnapshot {
    pub(super) fn new(active: Active, home: PathBuf, manifest: HomeLockGuard) -> Self {
        Self {
            active,
            home,
            _manifest: manifest,
        }
    }
}

impl std::fmt::Debug for VerifiedSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifiedSnapshot").finish_non_exhaustive()
    }
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

    pub(super) fn recovery_selection(&self) -> Result<FirstBuild, String> {
        self.bound.verify_image()?;
        FirstBuild::installed(self.preferred_image())
    }

    pub(super) fn variant(&self) -> Option<BoundVariant> {
        match &self.bound.selection {
            Selection::Cached { .. } => None,
            Selection::Download { archive } => archive.variant(self.bound.proof.as_ref()?),
        }
    }

    /// Verify under the game-manifest lock before checking or compiling this first set.
    pub(super) fn verify_inputs(&self) -> Result<(), String> {
        if inputs(&self.home)? != self.bound.inputs {
            return Err(unsettled!());
        }
        Ok(())
    }

    pub fn prepare(&self, log: &dyn Fn(String)) -> Result<(), String> {
        Resources::for_play(self.base(), self.handoff(), log).map(drop)
    }

    pub(super) fn prepare_with(
        &self,
        log: &dyn Fn(String),
        diagnostics: &dyn Fn(String),
    ) -> Result<PreparedResources, String> {
        Resources::for_play_with(self.base(), self.handoff(), log, diagnostics)
    }

    pub fn exact_image(
        &self,
        mode: native_mods::Profile,
        cancellation: &native_mods::Cancellation,
        progress: impl Fn(native_mods::Step),
        log: Option<&dyn Fn(std::sync::Arc<str>)>,
    ) -> Result<String, native_mods::Failed> {
        image_for_stage(
            self.base(),
            self.preferred_image(),
            &self.home,
            &self.stage,
            mode,
            cancellation,
            progress,
            log,
        )
    }

    /// No failed or partially compiled output enters a generation. Keep the worker's
    /// original native permission revision through the last check and publication.
    pub fn commit(
        self,
        image: &str,
        cancellation: &native_mods::Cancellation,
    ) -> Result<Active, String> {
        cancellation.check()?;
        // The pending binding already reserves the base. Do not hold the cache-wide
        // publication fence while waiting on a preparer child for this one version.
        self.bound.verify_image()?;
        let resources = Resources::capture(self.base(), &self.stage)?;
        Ok(self
            .commit_prepared(image, PreparedResources::reused(resources), cancellation)
            .map_err(String::from)?
            .active)
    }

    pub(super) fn commit_prepared(
        self,
        image: &str,
        prepared: PreparedResources,
        cancellation: &native_mods::Cancellation,
    ) -> Result<VerifiedSnapshot, super::StartError> {
        let manifest = game_lock(&self.home)?;
        crate::browser::installed::recover_game_install(&self.home)
            .map_err(super::StartError::Journal)?;
        // Refuse a damaged reused tree before it can bind an unbound profile.
        // A snapshot just sealed inside `prepare` is already proven by its seal.
        if !prepared.proven_by_seal {
            prepared
                .resources
                .verify(self.base())
                .map_err(super::StartError::Snapshot)?;
        }
        let _references = ReferencePublication::begin(&self.home)?;
        self.bound.verify_image()?;
        let active = Active::capture(
            &self.bound.base,
            image,
            &self.home,
            self.bound.inputs,
            Generation::First {
                profile: self.bound.profile,
                resources: prepared.resources,
                variant: self.variant().map(Box::new),
            },
        )?;
        super::inputs::check_browser_rows(&self.home, &scan_packages(&self.home)?)?;
        let active = publish_generation(
            self.stage,
            active,
            &self.home,
            self.bound.inputs,
            cancellation,
        )?;
        Ok(VerifiedSnapshot::new(active, self.home, manifest))
    }
}

/// The caller holds the profile, game manifest and reference fence.
fn publish_generation(
    stage: Staging<'_>,
    active: Active,
    home: &Path,
    epoch: [u8; 32],
    cancellation: &native_mods::Cancellation,
) -> Result<Active, String> {
    let pointer = serde_json::to_vec(&active).map_err(|e| e.to_string())?;
    stage.seal(pointer, &[])?.publish(|| {
        // First Play checks resources before publication; refresh keeps the saved digest
        // and checks resources at the handoff.
        active.verify_binaries()?;
        if inputs(home)? != epoch {
            return Err(unsettled!());
        }
        cancellation.check()
    })?;
    Ok(active)
}

#[derive(Debug)]
pub(crate) enum RefreshError {
    Native(native_mods::Failed),
    Snapshot(String),
}

impl From<String> for RefreshError {
    fn from(error: String) -> Self {
        Self::Native(error.into())
    }
}

impl From<native_mods::Failed> for RefreshError {
    fn from(error: native_mods::Failed) -> Self {
        Self::Native(error)
    }
}

fn needs_native_build(base: &str, preferred: &str, home: &Path) -> bool {
    super::verify_included(base, preferred, home).is_err()
        && !crate::native_status::enabled(home).is_empty()
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
    log: Option<&dyn Fn(std::sync::Arc<str>)>,
) -> Result<String, native_mods::Failed> {
    if super::verify_included(base, preferred, home).is_ok() {
        return Ok(preferred.to_owned());
    }
    if crate::native_status::enabled(home).is_empty() {
        return Ok(base.to_owned());
    }
    native_mods::build_exact_client(base, home, mode, stage, cancellation, progress, log)
}

pub(super) fn game_lock(home: &Path) -> Result<HomeLockGuard, String> {
    lock_home(home).map_err(|e| at(home, e))
}

mod edits;
pub(crate) use edits::{
    changed_files, copy_install_inputs, CandidateImage, Edit, SwitchPublication,
};

#[cfg(test)]
mod tests;
