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
pub struct ReferencePublication {
    _fence: sunrise_launcher_smod::runtime_leases::Publication,
}

impl ReferencePublication {
    pub fn begin(home: &Path) -> Result<Self, String> {
        use sunrise_launcher_smod::{
            locks::{LockMode, Release},
            runtime_leases::Registry,
        };
        let fence = Registry::with_release(&crate::backend::data_dir(), Release::Close)
            .and_then(|registry| registry.publication(LockMode::Exclusive))
            .map_err(|e| native_mods::cache_error(e.into()))?;
        roots::register(home)?;
        Ok(Self { _fence: fence })
    }
}

use crate::{backend::installation::BoundVariant, native_mods, native_status};
pub(crate) use artifact::Artifact;
use resources::Resources;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, io, path::Path};

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
    /// Acknowledging the notice must not discard the evidence an explicit update needs.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    acknowledged: bool,
}

/// An exact native package left installed but held off by a proved no-fit on Switch.
/// This is update authority, not the notice's display text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Held {
    id: String,
    digest: [u8; 32],
    path: String,
    source: HeldSource,
    enabled_before: bool,
    reason: HeldReason,
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
}

impl Held {
    pub fn new(
        id: String,
        digest: [u8; 32],
        path: String,
        source: HeldSource,
        enabled_before: bool,
        reason: HeldReason,
    ) -> Result<Self, String> {
        let held = Self {
            id,
            digest,
            path,
            source,
            enabled_before,
            reason,
        };
        held.validate()?;
        Ok(held)
    }

    fn validate(&self) -> Result<String, String> {
        use sunrise_launcher_smod::{index::valid_id, manifest::revision};
        let key = crate::profile_txn::native_package_key(&self.path).ok_or_else(unsettled)?;
        if !valid_id(&self.id) {
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
        let HeldReason::NoFit { build } = &self.reason;
        validate_tag(build)?;
        Ok(key)
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn digest(&self) -> &[u8; 32] {
        &self.digest
    }
    pub fn path(&self) -> &str {
        &self.path
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
                || (receipt.waiting_for_update.is_empty()
                    && (receipt.acknowledged || receipt.removed.is_empty()))
            {
                return Err(unsettled());
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

    pub fn base(&self) -> &str {
        &self.base.tag
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
    /// the chosen image. A previous generation is only advanced on the same exact base.
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
