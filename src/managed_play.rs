//! Exact managed Play state. The profile journal owns admission and publication;
//! this boundary owns the types, verification and interpretation of its saved pointers.
mod artifact;
mod catalogue;
mod client;
mod first;
mod game;
mod inputs;
mod resources;
mod roots;
pub use client::{headless_first, Client, Progress, StartError, Started};
pub(crate) use first::CandidateImage;
pub use first::{FirstAttempt, FirstBuild, Profile};
use inputs::inputs;
pub use inputs::{scan_packages, ScannedPackage};

/// Called only within the cache publication fence immediately before detaching a build.
pub(crate) fn authorize_cache_removal(tag: &str) -> Result<(), crate::native_cache::Error> {
    use crate::{backend, native_cache::Error};
    match roots::referenced(&backend::data_dir(), &backend::game_home(), tag) {
        Ok(false) => Ok(()),
        Ok(true) => Err(Error::ReferencedProfile),
        Err(error) => {
            eprintln!("profile artifact references: {error}");
            Err(Error::UnprovenReferences)
        }
    }
}

/// The profile gate is already owned. Holding this fence excludes both deletion and
/// another roster writer until the caller durably publishes its new reference.
// A repaint's shared snapshot is brief, but it can otherwise win the same-process
// race with Switch or receipt acknowledgement. Only serialize entry with the local
// reader; the kernel claim still controls exclusion with other processes and cache work.
static DISPLAY_PUBLICATION: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub struct ReferencePublication {
    _fence: sunrise_launcher_smod::runtime_leases::Publication,
}

impl ReferencePublication {
    pub fn begin(home: &Path) -> Result<Self, String> {
        use sunrise_launcher_smod::{
            locks::{LockMode, Release},
            runtime_leases::Registry,
        };
        let readers = DISPLAY_PUBLICATION.lock().map_err(|_| unsettled())?;
        let fence = Registry::with_release(&crate::backend::data_dir(), Release::Close)
            .and_then(|registry| registry.publication(LockMode::Exclusive))
            .map_err(|e| native_mods::cache_error(e.into()))?;
        drop(readers);
        roots::register(home)?;
        Ok(Self { _fence: fence })
    }
}

use crate::{backend::installation::BoundVariant, native_mods, native_status};
pub(crate) use artifact::Artifact;
use resources::Resources;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

fn unsettled() -> String {
    crate::i18n::tr("Sunrise couldn't verify the profile's saved game build.").into()
}

fn validate_tag(tag: &str) -> Result<(), String> {
    if !tag
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphanumeric)
        || tag.ends_with('.')
        || !tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.+@".contains(&b))
    {
        return Err(unsettled());
    }
    Ok(())
}

fn plain_file(meta: &fs::Metadata) -> bool {
    !crate::native_cache::linked(meta)
}

fn require_dir(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|error| at(path, error))?;
    if !meta.is_dir() || !plain_file(&meta) {
        return Err(at(path, io::Error::from(io::ErrorKind::InvalidData)));
    }
    Ok(())
}

fn open_if_file(path: &Path) -> Result<Option<fs::File>, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && plain_file(&meta) => {
            fs::File::open(path).map(Some).map_err(|e| at(path, e))
        }
        Ok(_) => Err(at(path, io::Error::from(io::ErrorKind::InvalidData))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(at(path, error)),
    }
}

fn digest_file(path: &Path) -> Result<Option<[u8; 32]>, String> {
    let Some(mut file) = open_if_file(path)? else {
        return Ok(None);
    };
    let mut hash = Sha256::new();
    io::copy(&mut file, &mut hash).map_err(|e| at(path, e))?;
    Ok(Some(hash.finalize().into()))
}

fn optional_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    use io::Read as _;
    let Some(mut file) = open_if_file(path)? else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|e| at(path, e))?;
    Ok(Some(bytes))
}

fn at(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}

/// Only this pointer selects a game's executable once its managed profile is initialized.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Active {
    version: u32,
    profile: [u8; 16],
    generation: u64,
    base: Artifact,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    variant: Option<Box<BoundVariant>>,
    executable: Artifact,
    resources: Resources,
    inputs: [u8; 32],
    receipts: Vec<Receipt>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    id: [u8; 16],
    waiting_for_update: Vec<Held>,
    removed: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    report: Option<ReceiptReport>,
    /// Acknowledging the notice must not discard the evidence an explicit update needs.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    acknowledged: bool,
}

/// Immutable display facts. An Update may retire a live hold without rewriting history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptReport {
    base: Option<String>,
    held: Vec<(String, bool)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    unproved: Vec<(String, bool)>,
}

impl ReceiptReport {
    fn new(base: Option<String>, held: &[Held]) -> Self {
        let mut report = Self {
            base,
            held: Vec::new(),
            unproved: Vec::new(),
        };
        for hold in held {
            let facts = (hold.title().into(), hold.enabled_before());
            if hold.reason().unproved() {
                report.unproved.push(facts);
            } else {
                report.held.push(facts);
            }
        }
        report
    }
}

impl Receipt {
    fn base(&self) -> Option<&str> {
        if let Some(report) = &self.report {
            return report.base.as_deref();
        }
        // Legacy reports with a hold recorded its target build on the hold itself.
        let build = self.waiting_for_update.first()?.reason().build();
        self.waiting_for_update
            .iter()
            .all(|hold| hold.reason().build() == build)
            .then_some(build)
    }

    fn has_report(&self) -> bool {
        !self.removed.is_empty()
            || !self.waiting_for_update.is_empty()
            || self
                .report
                .as_ref()
                .is_some_and(|report| !report.held.is_empty() || !report.unproved.is_empty())
    }
}

/// An exact package left installed but held off after Switch. The receipt explains why;
/// its metadata alone cannot authorize replacing the archive.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Held {
    id: String,
    title: String,
    digest: [u8; 32],
    root: HeldRoot,
    leaf: String,
    source: HeldSource,
    enabled_before: bool,
    reason: HeldReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeldRoot {
    Native,
    Content,
}

impl HeldRoot {
    pub fn directory(self) -> &'static str {
        use sunrise_launcher_smod::mod_list::{DIR, NATIVE_DIR};
        match self {
            Self::Native => NATIVE_DIR,
            Self::Content => DIR,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HeldSource {
    Registry {
        project: String,
        version: String,
    },
    Github {
        owner: String,
        repo: String,
        commit: String,
        version: String,
    },
    Local {
        version: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HeldReason {
    NoFit { build: String },
    Unproved { build: String },
}

impl HeldReason {
    fn build(&self) -> &str {
        match self {
            Self::NoFit { build } | Self::Unproved { build } => build,
        }
    }

    pub fn unproved(&self) -> bool {
        matches!(self, Self::Unproved { .. })
    }

    pub fn label(&self) -> &'static str {
        if self.unproved() {
            crate::i18n::tr("Fit could not be verified")
        } else {
            crate::i18n::tr("Needs an update from its author")
        }
    }
}

impl Held {
    pub fn new(
        id: String,
        title: String,
        digest: [u8; 32],
        (root, leaf): (HeldRoot, String),
        source: HeldSource,
        enabled_before: bool,
        reason: HeldReason,
    ) -> Result<Self, String> {
        let held = Self {
            id,
            title,
            digest,
            root,
            leaf,
            source,
            enabled_before,
            reason,
        };
        held.validate()?;
        Ok(held)
    }

    fn validate(&self) -> Result<String, String> {
        use sunrise_launcher_smod::{index::valid_id, manifest::revision};
        if !crate::profile_txn::valid_smod_leaf(&self.leaf) {
            return Err(unsettled());
        }
        // The journal's portable key is the folded leaf, even across the two roots.
        let key = self.leaf.to_ascii_lowercase();
        if !valid_id(&self.id)
            || self.title.trim().is_empty()
            || self.title.chars().any(char::is_control)
        {
            return Err(unsettled());
        }
        let version = match &self.source {
            HeldSource::Registry { project, version } if project == &self.id => version,
            HeldSource::Github {
                owner,
                repo,
                commit,
                version,
            } if crate::github_mods::plain(owner)
                && crate::github_mods::plain(repo)
                && revision(commit) =>
            {
                version
            }
            HeldSource::Local { version } => version,
            _ => return Err(unsettled()),
        };
        if version.is_empty() || version.chars().any(char::is_control) {
            return Err(unsettled());
        }
        validate_tag(self.reason.build())?;
        Ok(key)
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn matches_archive(&self, home: &Path) -> bool {
        digest_file(&home.join(self.path())).is_ok_and(|hash| hash.as_ref() == Some(&self.digest))
    }

    pub fn digest(&self) -> &[u8; 32] {
        &self.digest
    }
    pub fn root(&self) -> HeldRoot {
        self.root
    }
    pub fn path(&self) -> PathBuf {
        Path::new(self.root.directory()).join(&self.leaf)
    }
    pub fn source(&self) -> &HeldSource {
        &self.source
    }
    pub fn enabled_before(&self) -> bool {
        self.enabled_before
    }
    pub fn reason(&self) -> &HeldReason {
        &self.reason
    }
}

enum Generation<'a> {
    First {
        profile: [u8; 16],
        resources: Resources,
        variant: Option<Box<BoundVariant>>,
    },
    Advance(&'a Active),
    Switch {
        previous: &'a Active,
        resources: Resources,
        variant: Option<Box<BoundVariant>>,
    },
}

pub enum DisplayActiveError {
    Publishing,
    Read(String),
}

/// Read only the last settled pointer for display. The Installed list must distinguish a
/// temporarily busy reader from a settled profile with no holds.
pub fn try_display_active(home: &Path) -> Result<Option<Active>, DisplayActiveError> {
    use sunrise_launcher_smod::{
        locks::LockMode,
        runtime_leases::{Error, Registry},
    };
    let _reader = match DISPLAY_PUBLICATION.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::WouldBlock) => return Err(DisplayActiveError::Publishing),
        Err(std::sync::TryLockError::Poisoned(_)) => {
            return Err(DisplayActiveError::Read(unsettled()))
        }
    };
    let _publication = match Registry::existing(&crate::backend::data_dir())
        .and_then(|registry| registry.publication(LockMode::Shared))
    {
        Ok(lock) => Some(lock),
        Err(Error::Io(error)) if error.kind() == io::ErrorKind::NotFound => None,
        Err(Error::Busy) => return Err(DisplayActiveError::Publishing),
        Err(error) => {
            return Err(DisplayActiveError::Read(native_mods::cache_error(
                error.into(),
            )))
        }
    };
    crate::profile_txn::display_pointer(home)
        .and_then(|active| active.as_deref().map(Active::parse).transpose())
        .map_err(DisplayActiveError::Read)
}

/// A display snapshot never authorizes mutation; those callers still claim profile admission.
pub fn display_active(home: &Path) -> Result<Option<Active>, String> {
    try_display_active(home).map_err(|error| match error {
        DisplayActiveError::Publishing => {
            native_mods::cache_error(sunrise_launcher_smod::runtime_leases::Error::Busy.into())
        }
        DisplayActiveError::Read(error) => error,
    })
}

/// A paint can request an admitted read, but this hint never authorizes an action.
#[derive(Debug, PartialEq, Eq)]
pub enum ReceiptHint {
    Recovery,
    Pending { id: [u8; 16], generation: u64 },
}

pub fn build_changes_hint(home: &Path) -> Result<Option<ReceiptHint>, String> {
    if crate::profile_txn::recovery_pending(home)? {
        return Ok(Some(ReceiptHint::Recovery));
    }
    Ok(display_active(home)?.and_then(|active| {
        active
            .pending_changes()
            .map(|pending| ReceiptHint::Pending {
                id: pending.id,
                generation: active.generation(),
            })
    }))
}

/// Home and the report sheet use this admitted snapshot. Settling a committed journal
/// brings its receipt forward before the player can read or acknowledge it.
pub fn read_build_changes(
    home: &Path,
) -> Result<Option<(crate::profile_txn::Identity, Active)>, String> {
    let identity = crate::profile_txn::Identity::of(home)?;
    let profile = Profile::open_for(home, None, Some(identity)).map_err(String::from)?;
    Ok(profile
        .active()?
        .filter(|active| active.pending_changes().is_some())
        .map(|active| (identity, active)))
}

pub struct PendingChanges<'a> {
    pub id: [u8; 16],
    pub removed: &'a [(String, String)],
    receipt: &'a Receipt,
}

impl<'a> PendingChanges<'a> {
    pub fn base(&self) -> Option<&'a str> {
        self.receipt.base()
    }

    pub fn held(&self) -> impl Iterator<Item = (&'a str, bool, bool)> {
        let recorded = self.receipt.report.as_ref();
        let original = recorded.into_iter().flat_map(|report| {
            report
                .held
                .iter()
                .map(|(title, enabled)| (title.as_str(), *enabled, false))
                .chain(
                    report
                        .unproved
                        .iter()
                        .map(|(title, enabled)| (title.as_str(), *enabled, true)),
                )
        });
        let legacy = recorded
            .is_none()
            .then_some(&self.receipt.waiting_for_update)
            .into_iter()
            .flatten()
            .map(|hold| {
                (
                    hold.title(),
                    hold.enabled_before(),
                    hold.reason().unproved(),
                )
            });
        original.chain(legacy)
    }

    pub fn held_lines(&self) -> impl Iterator<Item = String> + 'a {
        self.held().map(|(title, enabled, unknown)| {
            crate::i18n::trf(
                match (enabled, unknown) {
                    (true, false) => "{mod} was turned off. Needs an update from its author.",
                    (false, false) => "{mod} stays off. Needs an update from its author.",
                    (true, true) => "{mod} was turned off. Fit could not be verified.",
                    (false, true) => "{mod} stays off. Fit could not be verified.",
                },
                &[("mod", title)],
            )
        })
    }
}

impl Active {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let value: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if value.version != 1 || value.profile == [0; 16] || value.generation == 0 {
            return Err(unsettled());
        }
        validate_tag(value.base())?;
        validate_tag(value.executable())?;
        if value.base() == value.executable() && value.base != value.executable {
            return Err(unsettled());
        }
        let mut receipts = std::collections::HashSet::new();
        let mut holds = std::collections::HashSet::new();
        let mut paths = std::collections::HashSet::new();
        for receipt in &value.receipts {
            if receipt.id == [0; 16]
                || !receipts.insert(receipt.id)
                || (receipt.waiting_for_update.is_empty() && receipt.acknowledged)
                || !receipt.has_report()
            {
                return Err(unsettled());
            }
            if let Some(report) = &receipt.report {
                if let Some(base) = &report.base {
                    validate_tag(base)?;
                }
                if report
                    .held
                    .iter()
                    .chain(&report.unproved)
                    .any(|(title, _)| {
                        title.trim().is_empty() || title.chars().any(char::is_control)
                    })
                {
                    return Err(unsettled());
                }
            }
            for held in &receipt.waiting_for_update {
                if !paths.insert(held.validate()?) || !holds.insert(held.id()) {
                    return Err(unsettled());
                }
            }
        }
        Ok(value)
    }

    pub fn holds(&self) -> impl Iterator<Item = &Held> {
        self.receipts
            .iter()
            .flat_map(|receipt| &receipt.waiting_for_update)
    }

    /// The next unacknowledged receipt. Acknowledgment retains exact hold authority.
    pub fn pending_changes(&self) -> Option<PendingChanges<'_>> {
        self.receipts
            .iter()
            .find(|receipt| !receipt.acknowledged)
            .map(|receipt| PendingChanges {
                id: receipt.id,
                removed: &receipt.removed,
                receipt,
            })
    }

    pub fn base(&self) -> &str {
        &self.base.tag
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn same_base(&self, other: &Self) -> bool {
        self.base == other.base && self.variant == other.variant
    }

    pub fn executable(&self) -> &str {
        &self.executable.tag
    }

    pub fn resource_path(&self) -> std::path::PathBuf {
        self.resources.path(self.base())
    }

    pub fn allow_accelerator(&self) -> bool {
        self.resources.allow_accelerator(self.base())
    }

    pub fn verify_program(&self, path: &Path) -> Result<(), String> {
        self.executable.verify_program(path)
    }

    #[cfg(windows)]
    pub(crate) fn verify_runtime_copy(&self, directory: &Path) -> Result<(), String> {
        if Artifact::capture_at(self.executable(), directory)? != self.executable {
            return Err(unsettled());
        }
        Ok(())
    }

    /// Hash the actual image, including runtime dependencies and the signed archive marker.
    pub(crate) fn matches_inputs(&self, home: &Path) -> bool {
        inputs(home, None).is_ok_and(|input| input == self.inputs)
    }

    pub fn verify_artifacts(&self) -> Result<(), String> {
        let base = match &self.variant {
            Some(variant) => variant.verify(self.base())?,
            None => Artifact::capture(self.base())?,
        };
        if base != self.base
            || (self.executable != self.base
                && Artifact::capture(self.executable())? != self.executable)
        {
            return Err(unsettled());
        }
        self.resources.verify(self.base())
    }

    /// Capture a full immutable generation after owning the admitted profile and checking
    /// the chosen image. Only the journal's Switch arm may advance to a distinct base.
    fn capture(
        base: &str,
        executable: &str,
        home: &Path,
        epoch: [u8; 32],
        previous: Generation<'_>,
    ) -> Result<Self, String> {
        scan_packages(home)?;
        verify_included(base, executable, home)?;
        if inputs(home, None)? != epoch {
            return Err(unsettled());
        }
        let base = Artifact::capture(base)?;
        let executable = if executable == base.tag {
            base.clone()
        } else {
            Artifact::capture(executable)?
        };
        let (profile, generation, resources, receipts, variant) = match previous {
            Generation::First {
                profile,
                resources,
                variant,
            } => (profile, 1, resources, Vec::new(), variant),
            Generation::Advance(old) => {
                if base != old.base {
                    return Err(unsettled());
                }
                old.resources.verify(&base.tag)?;
                (
                    old.profile,
                    old.generation.checked_add(1).ok_or_else(unsettled)?,
                    old.resources.clone(),
                    old.receipts.clone(),
                    old.variant.clone(),
                )
            }
            Generation::Switch {
                previous,
                resources,
                variant,
            } => {
                if base.tag == previous.base() {
                    return Err(unsettled());
                }
                resources.verify(&base.tag)?;
                (
                    previous.profile,
                    previous.generation.checked_add(1).ok_or_else(unsettled)?,
                    resources,
                    previous.receipts.clone(),
                    variant,
                )
            }
        };
        Ok(Self {
            version: 1,
            profile,
            generation,
            base,
            variant,
            executable,
            resources,
            inputs: epoch,
            receipts,
        })
    }
}

fn verify_included(base: &str, image: &str, home: &Path) -> Result<(), String> {
    if image != base
        && native_mods::about(image)
            .is_none_or(|about| about.base != base || !about.left_out.is_empty())
    {
        return Err(unsettled());
    }
    native_status::check_exact_compiled_profile(&native_status::included(image)?, home)
}
