//! Schema-2 signed registry planning, verification, staging, installation, updates, and explicit
//! provider switching.
//!
//! Every v2 operation keys mods by permanent 8-character project ID, treats `project.mod_id` as an
//! in-game slot with at most one enabled provider per profile, relies solely on authoritative
//! `release.fits` / `release.does_not_fit` verdicts against the exact Sunrise build ID, verifies
//! every archive's SHA-256 and offline Sigstore attestation against the repository the release
//! was signed from and `release.workflow`, pins the publisher's owner and workflow per computer,
//! and refuses to silently pick among competing providers or silently substitute a different
//! provider.

use super::{
    at, guard_replacements, held_change_refusal, known_unfit_updates, manifest_conflict,
    missing_required, mutations, prove_edit, remember_unfit_sha256, ChangeImpact, Document,
    InstallError, InstallHooks, InstallProgress, InstalledBatch, UpdateRequest,
};
use crate::{
    attestation::{self, ExpectedRelease, PublisherChange, PublisherPin, VerifiedRelease},
    backend,
    browser::{
        download, installed, mod_list,
        sunrise::{chosen, v2 as catalogue_v2, SITE},
        Kind, SourceId,
    },
    i18n::{tr, trf},
    jars,
    managed_play::{self, FirstBuild, Profile, ScannedPackage, Target},
    native_mods,
    profile_txn::Identity,
};
use semver::VersionReq;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    io::Read as _,
    path::{Path, PathBuf},
    sync::Arc,
};
use sunrise_launcher_smod::{
    dependencies::parse_version,
    index::{self, MAX_FILE},
    index_v2::{self, FitVerdict, Index, Kind as V2Kind, Project, Release},
    manifest::{collect_jar_requirements, JarRef, Side},
};

mod switch_v2;
#[cfg(all(test, target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
pub(crate) use switch_v2::switch_profile_build_v2_with;
pub use switch_v2::{
    checked_switch_fit_v2, switch_build_v2, switch_profile_build_v2, V2SwitchHooks,
};
mod ownership;
mod plan;
mod queue;
mod types;
pub use crate::managed_play::{
    classify_v2_ownership, revoked_for_installed_package, revoked_for_package, V2Ownership,
};
pub use ownership::*;
pub use plan::*;
use queue::*;
pub use types::*;
// edit and provider_switch call this as `super::preparation_line`.
use super::preparation_line;
mod edit;
mod provider_switch;
mod stage;
pub use edit::*;
pub use provider_switch::*;
pub(crate) use stage::*;

#[cfg(test)]
mod tests;
