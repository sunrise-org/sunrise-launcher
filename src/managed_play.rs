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
mod target;
pub(crate) use catalogue::has_source_origin;
pub use catalogue::{
    classify_v2_ownership, revoked_for_installed_package, revoked_for_package, V2Ownership,
};
pub use client::{headless_first, Client, FailedBuild, ModRecovery, Progress, StartError, Started};
pub(crate) use first::{changed_files, copy_install_inputs};
pub(crate) use first::{CandidateImage, Edit, SwitchPublication};
pub use first::{FirstAttempt, FirstBuild, Profile};
pub(crate) use inputs::inputs;
pub use inputs::{scan_packages, ScannedPackage};
pub use journal::{recover_game_install, GameInstallProblem, ModLock, VerifiedRepository};
pub(crate) use roots::remap_relocated_root;
pub use target::{BuildChoice, Target, TargetError};

/// Check pending and committed references under the exclusive cache publication fence
/// before detaching a build or publishing an ordinary download at its name.
pub(crate) fn require_unreferenced_build(tag: &str) -> Result<(), crate::native_cache::Error> {
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
        Self::begin_at(
            &crate::backend::data_dir(),
            &crate::backend::game_home(),
            home,
        )
    }

    pub(crate) fn begin_at(cache: &Path, game: &Path, home: &Path) -> Result<Self, String> {
        use sunrise_launcher_smod::{
            locks::{LockMode, Release},
            runtime_leases::Registry,
        };
        let readers = DISPLAY_PUBLICATION
            .lock()
            .map_err(|error| unsettled!(error))?;
        // This guard is never handed off. Release it explicitly even if an unrelated
        // concurrent fork still has a transient copy of the file descriptor.
        let fence = Registry::with_release(cache, Release::Unlock)
            .and_then(|registry| registry.publication(LockMode::Exclusive))
            .map_err(|e| native_mods::cache_error(e.into()))?;
        roots::register(cache, game, home)?;
        drop(readers);
        Ok(Self { _fence: fence })
    }
}

use crate::{backend::installation::BoundVariant, native_mods, native_status};

pub(crate) use artifact::Artifact;
pub(crate) use resources::Resources;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// Refuses a saved game build, naming the check that refused it (`file:line`) and the error it
/// had in hand, if any. Printed to stderr as well: about a hundred checks say the same sentence,
/// and the player's report must still tell them apart.
macro_rules! unsettled {
    () => {
        $crate::managed_play::unsettled_at(concat!(file!(), ":", line!()), None)
    };
    ($error:expr) => {
        $crate::managed_play::unsettled_at(
            concat!(file!(), ":", line!()),
            Some(&$error as &dyn std::fmt::Display),
        )
    };
}
pub(crate) use unsettled;
mod journal;

pub(crate) fn unsettled_at(check: &str, error: Option<&dyn std::fmt::Display>) -> String {
    // `file!()` uses the host's separators: the same text on every platform.
    let check = check.replace('\\', "/");
    let check = check.strip_prefix("src/").unwrap_or(&check);
    let reason = match error {
        Some(error) => crate::i18n::trf(
            "Sunrise couldn't verify the profile's saved game build ({check}: {error}).",
            &[("check", check), ("error", &error.to_string())],
        ),
        None => crate::i18n::trf(
            "Sunrise couldn't verify the profile's saved game build ({check}).",
            &[("check", check)],
        ),
    };
    eprintln!("{reason}");
    reason
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
        return Err(unsettled!());
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

/// The channel or exact pin committed alongside an `Active` switch generation.
/// Synced into `instances.json` after publication; recovered from here if that write fails
/// or the launcher exits between the two writes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommittedBuildChoice {
    pub id: [u8; 16],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<crate::instances::Channel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

impl CommittedBuildChoice {
    pub(crate) fn new(
        channel: Option<crate::instances::Channel>,
        version: Option<String>,
    ) -> Result<Self, String> {
        let mut id = [0u8; 16];
        getrandom::fill(&mut id).map_err(|e| e.to_string())?;
        if id == [0; 16] {
            return Err(unsettled!());
        }
        Ok(Self {
            id,
            channel,
            version,
        })
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    build_choice: Option<CommittedBuildChoice>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    mod_locks: Vec<ModLock>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repository: Option<VerifiedRepository>,
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
    NoFit {
        build: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    Unproved {
        build: String,
        /// What the fit check said about these exact bytes, when it said anything.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    BuildFailed {
        build: String,
    },
}

impl HeldReason {
    /// The build against which this hold was recorded.
    pub fn build(&self) -> &str {
        match self {
            Self::NoFit { build, .. }
            | Self::Unproved { build, .. }
            | Self::BuildFailed { build } => build,
        }
    }

    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::NoFit { detail, .. } | Self::Unproved { detail, .. } => detail.as_deref(),
            Self::BuildFailed { .. } => Some(crate::i18n::tr("Couldn't build on this computer.")),
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
            return Err(unsettled!());
        }
        // The journal's portable key is the folded leaf, even across the two roots.
        let key = self.leaf.to_ascii_lowercase();
        if !valid_id(&self.id)
            || self.title.trim().is_empty()
            || self.title.chars().any(char::is_control)
        {
            return Err(unsettled!());
        }
        let version = match &self.source {
            HeldSource::Registry {
                project,
                version,
                repository,
            } if (repository.is_none() && project == &self.id)
                || (repository.as_ref().is_some_and(VerifiedRepository::valid)
                    && sunrise_launcher_smod::index_v2::valid_project_id(project)) =>
            {
                version
            }
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
            _ => return Err(unsettled!()),
        };
        if version.is_empty() || version.chars().any(char::is_control) {
            return Err(unsettled!());
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
    Advance {
        previous: &'a Active,
        resources: Option<Resources>,
    },
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
    try_display_active_at(&crate::backend::data_dir(), home)
}

pub(crate) fn try_display_active_at(
    data: &Path,
    home: &Path,
) -> Result<Option<Active>, DisplayActiveError> {
    use sunrise_launcher_smod::{
        locks::LockMode,
        runtime_leases::{Error, Registry},
    };
    let _reader = match DISPLAY_PUBLICATION.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::WouldBlock) => return Err(DisplayActiveError::Publishing),
        Err(std::sync::TryLockError::Poisoned(_)) => {
            return Err(DisplayActiveError::Read(unsettled!()))
        }
    };
    let _publication = match Registry::existing(data)
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
        let holds = &self.receipt.waiting_for_update;
        self.held().map(move |(title, enabled, unknown)| {
            let line = crate::i18n::trf(
                match (enabled, unknown) {
                    (true, false) => "{mod} was turned off. Needs an update from its author.",
                    (false, false) => "{mod} stays off. Needs an update from its author.",
                    (true, true) => "{mod} was turned off. Fit could not be verified.",
                    (false, true) => "{mod} stays off. Fit could not be verified.",
                },
                &[("mod", title)],
            );
            // A hold still waiting says what the check found; a retired one has moved on.
            match holds
                .iter()
                .find(|hold| hold.title() == title && hold.reason().unproved() == unknown)
                .and_then(|hold| hold.reason().detail())
            {
                Some(reason) => format!("{line} {reason}"),
                None => line,
            }
        })
    }
}

impl Active {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let value: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if value.version != 1 || value.profile == [0; 16] || value.generation == 0 {
            return Err(unsettled!());
        }
        validate_tag(value.base())?;
        validate_tag(value.executable())?;
        if value.base() == value.executable() && value.base != value.executable {
            return Err(unsettled!());
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
                return Err(unsettled!());
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
                    return Err(unsettled!());
                }
            }
            for held in &receipt.waiting_for_update {
                if !paths.insert(held.validate()?) || !holds.insert(held.id()) {
                    return Err(unsettled!());
                }
            }
        }
        if let Some(choice) = &value.build_choice {
            if choice.id == [0; 16] || (choice.channel.is_none() && choice.version.is_none()) {
                return Err(unsettled!());
            }
            if let Some(version) = &choice.version {
                validate_tag(version)?;
            }
        }
        let mut locked_mods = std::collections::HashSet::new();
        let mut locked_projects = std::collections::HashSet::new();
        for lock in &value.mod_locks {
            lock.validate()?;
            if !locked_mods.insert(lock.mod_id.as_str())
                || !locked_projects.insert(lock.project.as_str())
            {
                return Err(unsettled!());
            }
        }
        Ok(value)
    }

    /// Read the currently published generation without recovering or changing the profile.
    pub fn read_pointer(home: &Path) -> Result<Option<Self>, String> {
        crate::profile_txn::references(home)?
            .pointer
            .as_deref()
            .map(Self::parse)
            .transpose()
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

    pub fn build_choice(&self) -> Option<&CommittedBuildChoice> {
        self.build_choice.as_ref()
    }

    pub fn mod_locks(&self) -> &[ModLock] {
        &self.mod_locks
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
            return Err(unsettled!());
        }
        Ok(())
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
            return Err(unsettled!());
        }
        self.resources.verify(self.base())
    }

    /// Capture a full immutable generation after owning the admitted profile and checking
    /// the chosen image. Only the journal's Switch arm may advance to a distinct base.
    /// Verify the bound resources at the publication cutoff, not while assembling the pointer.
    fn capture(
        base: &str,
        executable: &str,
        home: &Path,
        epoch: [u8; 32],
        previous: Generation<'_>,
    ) -> Result<Self, String> {
        scan_packages(home)?;
        verify_included(base, executable, home)?;
        if inputs(home)? != epoch {
            return Err(unsettled!());
        }
        let base = Artifact::capture(base)?;
        let executable = if executable == base.tag {
            base.clone()
        } else {
            Artifact::capture(executable)?
        };
        let (profile, generation, resources, receipts, variant, build_choice) = match previous {
            Generation::First {
                profile,
                resources,
                variant,
            } => (profile, 1, resources, Vec::new(), variant, None),
            Generation::Advance {
                previous,
                resources,
            } => {
                if base != previous.base {
                    return Err(unsettled!());
                }
                (
                    previous.profile,
                    previous
                        .generation
                        .checked_add(1)
                        .ok_or_else(|| unsettled!())?,
                    resources.unwrap_or_else(|| previous.resources.clone()),
                    previous.receipts.clone(),
                    previous.variant.clone(),
                    previous.build_choice.clone(),
                )
            }
            Generation::Switch {
                previous,
                resources,
                variant,
            } => {
                if base.tag == previous.base() {
                    return Err(unsettled!());
                }
                (
                    previous.profile,
                    previous
                        .generation
                        .checked_add(1)
                        .ok_or_else(|| unsettled!())?,
                    resources,
                    previous.receipts.clone(),
                    variant,
                    previous.build_choice.clone(),
                )
            }
        };
        let mod_locks = capture_mod_locks(home)?;
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
            build_choice,
            mod_locks,
        })
    }
}

fn capture_mod_locks(home: &Path) -> Result<Vec<ModLock>, String> {
    let path = home.join(sunrise_launcher_smod::index::browser_state::MANIFEST);
    let Some(bytes) = optional_bytes(&path)? else {
        return Ok(Vec::new());
    };
    let Ok(document) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Ok(Vec::new());
    };
    let Some(entries) = document
        .get("entries")
        .and_then(serde_json::Value::as_array)
    else {
        return Ok(Vec::new());
    };
    let mut locks = Vec::new();
    let mut locked_mods = std::collections::HashSet::new();
    let mut locked_projects = std::collections::HashSet::new();
    for raw in entries {
        if raw.get("source").and_then(serde_json::Value::as_str) != Some("sunrise")
            || raw
                .get("mod_id")
                .and_then(serde_json::Value::as_str)
                .is_none()
        {
            continue;
        }
        // A signed schema-2 row: Sunrise source and mod kind, an 8-character project ID
        // beside the in-game mod ID, a lowercase SHA-256, and a verified repository.
        let field = |name: &str| raw.get(name).and_then(serde_json::Value::as_str);
        let Some(project) = field("project") else {
            continue;
        };
        let Some(mod_id) = field("mod_id") else {
            continue;
        };
        if field("kind") != Some("mod")
            || !sunrise_launcher_smod::index_v2::valid_project_id(project)
        {
            continue;
        }
        let Some(version) = field("version").map(str::to_owned) else {
            continue;
        };
        let Some(sha256) = field("checksum")
            .and_then(|checksum| checksum.strip_prefix("SHA-256:"))
            .filter(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .map(str::to_ascii_lowercase)
        else {
            continue;
        };
        let Some(repository) = raw
            .get("repository")
            .and_then(|value| serde_json::from_value::<VerifiedRepository>(value.clone()).ok())
            .filter(VerifiedRepository::valid)
        else {
            continue;
        };
        let lock = ModLock {
            project: project.to_owned(),
            mod_id: mod_id.to_owned(),
            version,
            sha256,
            repository,
        };
        lock.validate()?;
        if !locked_mods.insert(lock.mod_id.clone()) || !locked_projects.insert(lock.project.clone())
        {
            return Err(unsettled!());
        }
        locks.push(lock);
    }
    locks.sort_by(|a, b| a.mod_id.cmp(&b.mod_id));
    Ok(locks)
}

fn verify_included(base: &str, image: &str, home: &Path) -> Result<(), String> {
    if image != base
        && native_mods::about(image)
            .is_none_or(|about| about.base != base || !about.left_out.is_empty())
    {
        return Err(unsettled!());
    }
    native_status::check_exact_compiled_profile(&native_status::included(image)?, home)
}
