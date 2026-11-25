//! The types v2 installs, updates and provider switches pass around: plans, options, hooks and the questions they ask.

use super::*;
use crate::managed_play::V2Ownership;

/// Injectable byte fetcher used by internal `_with` entrypoints and unit tests.
pub(crate) type BytesFetcher<'a> = &'a dyn Fn(&str, u64) -> Result<Vec<u8>, String>;

/// Injectable offline attestation verifier (production always passes `attestation::verify_release`).
pub(crate) type ReleaseVerifier<'a> = &'a dyn Fn(
    &[u8],
    [u8; 32],
    &ExpectedRelease<'_>,
) -> Result<VerifiedRelease, attestation::Error>;

/// One installable v2 project offering a fitting release for an ambiguous dependency slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderCandidate {
    pub project_id: String,
    pub slug: String,
    pub name: String,
    pub repository_name: String,
    pub version: String,
}

/// Asks the player which project provides a dependency several projects offer; `None` declines.
pub type ChooseProvider<'a> = dyn Fn(&ProviderChoiceNeeded) -> Option<String> + 'a;

/// Returned when a required dependency names only an in-game `mod_id` slot and several v2 projects
/// provide fitting releases for the profile's build. The planner never chooses one silently.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderChoiceNeeded {
    pub mod_id: String,
    pub required_by: String,
    pub candidates: Vec<ProviderCandidate>,
}

/// Returned when a requested project or required dependency would collide with a different provider
/// (another v2 project or an unadopted/local mod) already occupying the same in-game `mod_id` slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderConflict {
    pub mod_id: String,
    pub installed_project: Option<String>,
    pub installed_name: String,
    pub requested_project: String,
    pub requested_name: String,
}

/// One scanned package in a profile together with its verified v2 ownership.
#[derive(Clone, Debug)]
pub struct InstalledV2Mod {
    pub package: ScannedPackage,
    pub digest: String,
    pub size: u64,
    pub enabled: bool,
    pub ownership: V2Ownership,
}

/// Options controlling v2 planning, version overrides, explicit provider choices, and publisher pin
/// confirmation.
#[derive(Clone, Debug, Default)]
pub struct V2InstallOptions {
    /// Explicit provider selections for ambiguous dependency slots: `mod_id -> project_id`.
    pub provider_choices: BTreeMap<String, String>,
    /// Explicit version overrides per project ID (takes precedence over `chosen-versions.json`).
    pub versions: BTreeMap<String, String>,
    /// Project IDs for which the player has already confirmed a verified publisher change.
    pub accepted_publisher_changes: BTreeSet<String>,
    /// Override directory for `publisher-pins.json` (`None` uses `backend::data_dir()`).
    pub pin_dir: Option<PathBuf>,
    /// The profile's original-jar cache, `<home>/jars`, which the game reads. Installs, updates
    /// and provider switches fill it from their profile when it is `None`; a planner given `None`
    /// counts no jar as cached.
    pub jar_cache: Option<PathBuf>,
}

/// One concrete release selected by the v2 planner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedV2Release {
    pub project_id: String,
    pub slug: String,
    pub mod_id: String,
    pub name: String,
    pub kind: V2Kind,
    pub version: String,
    pub sha256: String,
    pub size: u64,
    pub url: String,
    pub attestation_url: String,
    pub workflow: String,
    pub repository_id: u64,
    pub repository_name: String,
    pub repository_path: String,
    pub dependency: bool,
}

/// Result of planning a v2 install or update batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct V2Plan {
    pub steps: Vec<PlannedV2Release>,
    /// Optional dependencies declared by the planned releases (never auto-installed).
    pub optional_dependencies: Vec<index_v2::Dependency>,
}

/// An enabled dependent in the profile that would be affected by switching a slot's provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct V2AffectedDependent {
    pub mod_id: String,
    pub name: String,
    pub reason: String,
}

/// Preview presented to the player before committing an explicit provider switch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderSwitchPreview {
    pub mod_id: String,
    pub from_project: Option<String>,
    pub from_name: String,
    pub to_project: String,
    pub to_name: String,
    pub to_version: String,
    pub worlds: Vec<crate::worlds::WorldRisk>,
}

/// What the player is asked when a release's verified publisher differs from this computer's
/// remembered pin for its project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublisherChangeQuestion {
    pub mod_name: String,
    /// The version installed in the profile now; `None` when this would be its first install there.
    pub current_version: Option<String>,
    pub incoming_version: String,
    pub change: PublisherChange,
}

/// Asked before a release from a changed publisher is staged; `true` accepts the new publisher.
pub type ConfirmPublisherChange<'a> = &'a dyn Fn(&PublisherChangeQuestion) -> bool;

/// Callbacks for v2 install and update operations.
#[derive(Clone, Copy)]
pub struct V2Hooks<'a> {
    pub install: InstallHooks<'a>,
    /// Prompt when a required dependency slot has several fitting providers and no pre-selected
    /// entry in `V2InstallOptions::provider_choices`. Returning `None` aborts with
    /// [`InstallError::AmbiguousProvider`].
    pub choose_provider: Option<&'a ChooseProvider<'a>>,
    /// Prompt when a project's attested publisher (owner, or for an old pin repository) or workflow
    /// path changed from this computer's remembered pin. Returning `false` (or `None`) aborts with
    /// [`InstallError::PublisherChanged`].
    pub confirm_publisher_change: Option<ConfirmPublisherChange<'a>>,
}

/// Callbacks for explicit v2 provider switching.
#[derive(Clone, Copy)]
pub struct V2ProviderSwitchHooks<'a> {
    pub install: V2Hooks<'a>,
    pub approve_switch: &'a dyn Fn(&ProviderSwitchPreview) -> bool,
}

#[derive(Debug)]
pub(crate) struct PendingPin {
    pub(super) project_id: String,
    pub(super) pin: PublisherPin,
    pub(super) accepted_change: Option<PublisherChange>,
}
