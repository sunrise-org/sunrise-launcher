//! Registry and collection requests produce one admitted, fully proved same-base generation.
use super::installed;
use crate::{
    browser::{mod_list, Kind, SourceId},
    i18n::{tr, trf},
    managed_play::{self, FirstBuild, Profile, ScannedPackage},
    native_mods,
};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use sunrise_launcher_smod::{
    index::{self, browser_state, resolve},
    manifest::{Manifest, Side},
};

mod source;
mod switch;
pub mod v2;
pub use crate::managed_play::Target;
pub(super) use crate::profile_txn::Identity;
pub use source::{
    add_sources, apply_sources, pack_sources, prepare_pasted_sources, prepare_source,
    read_pasted_sources, PastedSource, PreparedSource, SourceDependency, SourceError, SourceHooks,
    SourceProof, SourceRequest,
};
pub use switch::{BuildChoice, SwitchDecision, SwitchHooks, SwitchQuestion, Switched};
pub use v2::{
    checked_switch_fit_v2, classify_v2_ownership, install_registry_v2, inventory_v2,
    plan_v2_install, plan_v2_update, switch_build_v2, switch_profile_build_v2, switch_provider_v2,
    turn_off_revoked_v2, update_registry_v2, InstalledV2Mod, PlannedV2Release, ProviderCandidate,
    ProviderChoiceNeeded, ProviderConflict, ProviderSwitchPreview, PublisherChangeQuestion,
    V2AffectedDependent, V2Hooks, V2InstallOptions, V2Ownership, V2Plan, V2ProviderSwitchHooks,
    V2SwitchHooks,
};

// Advisory only. Never use these in place of a fresh, admitted worker proof. Key by the
// base build and verified archive hash, not the Minecraft version or catalogue version number.
static UNFIT_UPDATES: Mutex<BTreeMap<String, BTreeSet<String>>> = Mutex::new(BTreeMap::new());
static UNFIT_UPDATES_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn known_unfit_updates(base: &str) -> BTreeSet<String> {
    UNFIT_UPDATES
        .lock()
        .unwrap()
        .get(base)
        .cloned()
        .unwrap_or_default()
}

/// Monotonic revision bumped whenever [`remember_unfit_sha256`] records a new unfit update,
/// so views caching mod update status (such as Home) refresh even when the inventory and
/// registry index are unchanged.
pub fn unfit_updates_revision() -> u64 {
    UNFIT_UPDATES_REVISION.load(std::sync::atomic::Ordering::Relaxed)
}

fn remember_unfit_sha256(base: &str, sha256: &str) {
    if UNFIT_UPDATES
        .lock()
        .unwrap()
        .entry(base.into())
        .or_default()
        .insert(sha256.to_ascii_lowercase())
    {
        UNFIT_UPDATES_REVISION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InstallProgress {
    PreparingBuild,
    Checking,
    /// Another build or a cache cleanup holds the build folder: this install waits for it.
    Waiting,
    Downloading(f32),
    /// The game build's own download, before its Minecraft files are prepared.
    DownloadingBuild(f32),
    /// A game build's preparation: the step under way and, while Mojang's files download, how
    /// many of how many are done.
    Minecraft(crate::backend::PrepareStep, Option<(usize, usize)>),
    Preparing,
    Tools {
        done: u64,
        total: u64,
    },
    Rust(f32),
    Building(Option<f32>),
    Publishing,
    Reviewing,
}

#[derive(Debug)]
pub enum InstallError {
    Refused(String),
    Host(String),
    Unproved(String),
    Download(String),
    NotForVersion(String),
    GithubBuild(String),
    Source(SourceError),
    Compile(BuildNotice),
    PublisherChanged(Box<crate::attestation::PublisherChange>),
    AmbiguousProvider(Box<v2::ProviderChoiceNeeded>),
    ProviderConflict(Box<v2::ProviderConflict>),
    Cancelled,
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(text)
            | Self::Host(text)
            | Self::Unproved(text)
            | Self::NotForVersion(text)
            | Self::GithubBuild(text)
            | Self::Download(text) => f.write_str(text),
            Self::Compile(failure) => f.write_str(&failure.detail),
            Self::Source(error) => error.fmt(f),
            Self::PublisherChanged(change) => crate::attestation::Error::PublisherChanged(
                change.clone(),
            )
            .fmt(f),
            Self::AmbiguousProvider(need) => f.write_str(&trf(
                "{requester} needs {mod}, which is provided by several mods. Choose one to install.",
                &[("requester", &need.required_by), ("mod", &need.mod_id)],
            )),
            Self::ProviderConflict(conflict) => f.write_str(&trf(
                "{requested} cannot be installed while {installed} is already installed for {mod}. Switch provider instead.",
                &[
                    ("requested", &conflict.requested_name),
                    ("installed", &conflict.installed_name),
                    ("mod", &conflict.mod_id),
                ],
            )),
            Self::Cancelled => f.write_str(tr("Install cancelled.")),
        }
    }
}

impl From<String> for InstallError {
    fn from(error: String) -> Self {
        Self::Unproved(error)
    }
}

#[derive(Clone, Debug)]
pub struct BuildNotice {
    pub changes: Vec<BuildChange>,
    /// Raw compiler/proof error for the headless route, not the visible sheet.
    pub detail: String,
    pub log: Option<Arc<str>>,
}

#[derive(Clone, Debug)]
pub struct BuildChange {
    pub project: String,
    pub title: String,
    pub reason: installed::InstallHold,
    pub predecessor: Option<String>,
}

impl BuildChange {
    pub fn disposition(&self) -> String {
        match &self.predecessor {
            Some(version) => trf("Keep installed version {version}", &[("version", version)]),
            None => tr("Keep installed, but off").into(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct InstalledBatch {
    pub added: Vec<installed::Entry>,
    pub updated: Vec<(String, installed::Entry)>,
    pub failure: Option<BuildNotice>,
}

#[derive(Clone, Copy)]
pub struct InstallHooks<'a> {
    pub progress: &'a dyn Fn(InstallProgress),
    pub continue_after_failure: &'a dyn Fn(&BuildNotice) -> bool,
    /// Optional cancellation token observed during build-tool downloads and native builds.
    pub cancellation: Option<&'a native_mods::Cancellation>,
}

pub enum UpdateRequest {
    /// Every named root must advance, even if it is the only one requested.
    Strict(Vec<String>),
    /// The bulk action can retain roots with no usable newer release proved by this attempt.
    Bulk(Vec<String>),
}

/// An error that arrives while `stop` is asked is the stop itself, not a failure to report.
fn stopped_or(
    stop: Option<&native_mods::Cancellation>,
    error: impl Into<InstallError>,
) -> InstallError {
    if stop.is_some_and(native_mods::Cancellation::is_stopped) {
        InstallError::Cancelled
    } else {
        error.into()
    }
}

fn prove_edit(
    edit: &managed_play::Edit<'_>,
    candidate: &Path,
    target: &Target,
    cancellation: &native_mods::Cancellation,
    hooks: &InstallHooks<'_>,
) -> Result<managed_play::CandidateImage, native_mods::Failed> {
    edit.prove(
        candidate,
        target.native_profile(),
        cancellation,
        |step| build_progress(step, hooks.progress),
        None,
    )
}

/// A line from the first preparation of a profile's game build, which runs only when the build
/// isn't prepared yet: the install says the step it is at and Mojang's files done, as the Switch
/// dialog does, instead of "Getting ready…" for all of it.
fn preparation_line(progress: &dyn Fn(InstallProgress)) -> impl Fn(String) + '_ {
    move |_| {
        progress(InstallProgress::Minecraft(
            crate::backend::prepare_step(),
            crate::mojang::progress(),
        ))
    }
}

fn build_progress(step: native_mods::Step, progress: &dyn Fn(InstallProgress)) {
    progress(match step {
        // Registry actions already opened their progress before asking for build proof.
        native_mods::Step::Starting => return,
        native_mods::Step::Waiting => InstallProgress::Waiting,
        native_mods::Step::Rust(fraction) => InstallProgress::Rust(fraction),
        native_mods::Step::Tools { done, total, .. } => InstallProgress::Tools { done, total },
        native_mods::Step::Install | native_mods::Step::Done { .. } => {
            InstallProgress::Building(Some(1.))
        }
        _ => match step.progress() {
            native_mods::Progress {
                stage: native_mods::Stage::Compile,
                fraction,
            } => InstallProgress::Building(fraction),
            _ => InstallProgress::Preparing,
        },
    });
}

struct SeenPackage {
    package: ScannedPackage,
    digest: Option<String>,
    registry_claim: bool,
}

impl SeenPackage {
    fn matches_hold(&self, home: &Path, base: &str, hold: &managed_play::Held) -> bool {
        if hold.reason().build() != base
            || self.package.path != home.join(hold.path())
            || !self.digest.as_ref().is_some_and(|hash| {
                let digest = sha2::digest::Output::<sha2::Sha256>::from(*hold.digest());
                hash.eq_ignore_ascii_case(&format!("{digest:x}"))
            })
        {
            return false;
        }
        match hold.source() {
            managed_play::HeldSource::Registry { version, .. }
            | managed_play::HeldSource::Local { version } => {
                version == &self.package.manifest.version
            }
            managed_play::HeldSource::Github {
                owner,
                repo,
                commit,
                version,
            } => self
                .verified_github_origin(home)
                .ok()
                .flatten()
                .is_some_and(|row| {
                    &row.owner == owner
                        && &row.repo == repo
                        && &row.commit == commit
                        && &row.version == version
                }),
        }
    }

    fn github_origin(&self, home: &Path) -> Option<crate::github_mods::Origin> {
        let id = &self.package.manifest.id;
        let target = home.join(native_mods::INSTALLED).join(format!("{id}.smod"));
        if self.package.path != target || self.package.manifest.native.is_none() {
            return None;
        }
        crate::github_mods::origin(home, id).filter(|row| {
            row.local_folder_path().is_none()
                && self.digest.as_ref().is_some_and(|digest| {
                    row.matches_archive(id, &self.package.manifest.version, digest)
                })
        })
    }

    /// Reads and verifies the `.origin.json` receipt beside this archive (either a GitHub
    /// receipt in `sunrise-native-mods/` or a local-folder receipt beside a native or content
    /// archive). Returns `Err(())` when an origin sidecar exists at the canonical path but
    /// cannot be verified against this archive.
    fn verified_origin_sidecar(
        &self,
        home: &Path,
    ) -> Result<Option<(PathBuf, crate::github_mods::Origin)>, ()> {
        let id = &self.package.manifest.id;
        let native = self.package.manifest.native.is_some();
        let origin_at = if native {
            crate::github_mods::origin_path(home, id)
        } else {
            home.join(sunrise_launcher_smod::mod_list::DIR)
                .join(format!("{id}.origin.json"))
        };
        let origin = match fs::symlink_metadata(&origin_at) {
            Ok(meta) if meta.is_file() => {
                crate::github_mods::read_origin_at(&origin_at).ok_or(())?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            _ => return Err(()),
        };
        let canonical = home
            .join(if native {
                native_mods::INSTALLED
            } else {
                sunrise_launcher_smod::mod_list::DIR
            })
            .join(format!("{id}.smod"));
        // A predecessor kept under a name that differs only by case is still this receipt's leaf.
        let at_canonical = self.package.path.parent() == canonical.parent()
            && self
                .package
                .path
                .file_name()
                .zip(canonical.file_name())
                .is_some_and(|(leaf, expected)| leaf.eq_ignore_ascii_case(expected));
        let matches = if origin.local_folder_path().is_some() {
            at_canonical
                && self.digest.as_deref().is_some_and(|digest| {
                    origin.matches_archive(id, &self.package.manifest.version, digest)
                })
        } else if !native {
            false
        } else if origin.source.is_some() {
            self.github_origin(home).as_ref() == Some(&origin)
        } else {
            origin.id == *id
                && origin.version == self.package.manifest.version
                && self
                    .digest
                    .as_deref()
                    .is_some_and(|digest| origin.sha256.eq_ignore_ascii_case(digest))
        };
        if matches {
            Ok(Some((origin_at, origin)))
        } else {
            Err(())
        }
    }

    /// Reads and verifies a GitHub receipt at `origin_path(home, id)` when present,
    /// excluding local-folder receipts.
    fn verified_github_origin(
        &self,
        home: &Path,
    ) -> Result<Option<crate::github_mods::Origin>, ()> {
        Ok(self
            .verified_origin_sidecar(home)?
            .map(|(_, origin)| origin)
            .filter(|origin| origin.local_folder_path().is_none()))
    }
}

fn portable_row_path(home: &Path, file: &str) -> Option<(String, PathBuf)> {
    let normalized = file.replace('\\', "/");
    installed::in_folders(&normalized).then(|| {
        let path = home.join(&normalized);
        (normalized, path)
    })
}

type Inventory = BTreeMap<String, SeenPackage>;

fn inventory(home: &Path) -> Result<Inventory, String> {
    let doc = Document::read(home)?;
    managed_play::scan_packages(home)?
        .into_iter()
        .map(|p| {
            let digest = (!p.directory)
                .then(|| index::sha256_file(&p.path))
                .transpose()?;
            let id = p.manifest.id.clone();
            let row: Option<installed::Entry> = doc
                .row(&id)?
                .and_then(|row| serde_json::from_value(row.clone()).ok());
            let claim = row
                .as_ref()
                .zip(digest.as_deref())
                .and_then(|(row, digest)| {
                    let (normalized, path) = portable_row_path(home, &row.file)?;
                    (row.source == SourceId::Sunrise && row.kind == Kind::Mod && path == p.path)
                        .then_some((normalized, digest))
                });
            let registry_claim = claim.is_some();
            if let Some((normalized, _)) = claim {
                doc.rows
                    .check_other_claims(home, &id, &normalized)
                    .map_err(|error| error.render(trf))?;
            }
            Ok((
                id,
                SeenPackage {
                    package: p,
                    digest,
                    registry_claim,
                },
            ))
        })
        .collect()
}

fn missing_required<'a>(
    manifest: &'a Manifest,
    enabled: &BTreeMap<&str, &Manifest>,
) -> Option<&'a str> {
    manifest.dependencies.iter().find_map(|(id, dependency)| {
        let need = dependency.details();
        (!need.optional
            && need.side != Side::Server
            && !enabled.get(id.as_str()).is_some_and(|other| {
                sunrise_launcher_smod::dependencies::parse_version(&other.version)
                    .is_ok_and(|v| need.version.matches(&v))
            }))
        .then_some(id.as_str())
    })
}

fn manifest_conflict(
    home: &Path,
    side: Side,
    staged: impl Fn(&str) -> bool,
) -> Result<Option<resolve::ManifestConflict>, InstallError> {
    let packages = inventory(home)?;
    let overrides = (side == Side::Client)
        .then(|| {
            sunrise_launcher_smod::dependencies::Overrides::read(&home.join(mod_list::MODS_DIR))
        })
        .transpose()?;
    let enabled: BTreeMap<_, _> = packages
        .iter()
        .filter(|(id, _)| mod_list::is_enabled(home, id))
        .map(|(id, p)| (id.as_str(), &p.package.manifest))
        .collect();
    let file_for = |id: &str| {
        let package = &packages[id];
        resolve::InstalledMod {
            id: id.to_owned(),
            version: package.package.manifest.version.clone(),
            // A freshly checked download names a resolver choice, not replacement authority.
            sha256: staged(id).then(|| package.digest.clone()).flatten(),
        }
    };
    let mut namespaces = BTreeMap::new();
    for (&id, &manifest) in &enabled {
        if missing_required(manifest, &enabled).is_some() {
            return Err(InstallError::Refused(trf(
                "The proposed mods don't work with {mod}. Nothing was installed.",
                &[("mod", &manifest.name)],
            )));
        }
        for namespace in manifest.owned_namespaces() {
            if let Some(peer) = namespaces.insert(namespace, id) {
                return Ok(Some(resolve::ManifestConflict {
                    files: vec![file_for(peer), file_for(id)],
                    reason: trf(
                        "{first} and {second} both own the {namespace} namespace.",
                        &[
                            ("first", &enabled[peer].name),
                            ("second", &manifest.name),
                            ("namespace", namespace),
                        ],
                    ),
                }));
            }
        }
        for (peer, rule) in &manifest.incompatible {
            let details = rule.details();
            if side != Side::Both && details.side != Side::Both && details.side != side {
                continue;
            }
            if overrides
                .as_ref()
                .is_some_and(|overrides| overrides.get(id, peer).is_some())
            {
                continue;
            }
            if let Some(other) = enabled.get(peer.as_str()) {
                let version = sunrise_launcher_smod::dependencies::parse_version(&other.version)
                    .map_err(|error| InstallError::Unproved(error.to_string()))?;
                if details.version.matches(&version) {
                    return Ok(Some(resolve::ManifestConflict {
                        files: vec![file_for(id), file_for(peer)],
                        reason: trf(
                            "{first} {version} doesn't work with {second} {other_version}.",
                            &[
                                ("first", &manifest.name),
                                ("version", &manifest.version),
                                ("second", &other.name),
                                ("other_version", &other.version),
                            ],
                        ),
                    }));
                }
            }
        }
    }
    Ok(None)
}

fn guard_replacements<'a>(
    home: &Path,
    suspect: &crate::worlds::SuspectMods,
    replacements: impl Iterator<Item = &'a str>,
) -> Result<Vec<crate::worlds::WorldRisk>, String> {
    let reviewed = mutations::world_evidence(home, suspect)?;
    if reviewed.iter().any(|world| !world.mods.is_empty()) {
        let names: Vec<_> = replacements.collect();
        return Err(match names.as_slice() {
            [name] => trf(
                "{mod} is already installed. Update it from Installed.",
                &[("mod", name)],
            ),
            _ => trf(
                "These mods are already installed: {mods}. Update them from Installed.",
                &[("mods", &names.join(", "))],
            ),
        });
    }
    Ok(reviewed)
}

struct Document {
    map: Map<String, Value>,
    rows: browser_state::Installed,
}
impl Document {
    fn read(home: &Path) -> Result<Self, String> {
        let path = home.join(installed::MANIFEST);
        let mut map: Map<String, Value> = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| at(&path, e))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Map::from_iter([("entries".into(), Value::Array(Vec::new()))])
            }
            Err(error) => return Err(at(&path, error)),
        };
        let Some(Value::Array(entries)) = map.remove("entries") else {
            return Err(unproved_mods());
        };
        Ok(Self {
            map,
            rows: browser_state::Installed { entries },
        })
    }
    fn position(&self, id: &str) -> Result<Option<usize>, String> {
        self.rows.single(id).map_err(|e| e.render(trf))
    }
    fn row(&self, id: &str) -> Result<Option<&Value>, String> {
        Ok(self.position(id)?.map(|i| &self.rows.entries[i]))
    }
    fn set(&mut self, id: &str, value: Value) -> Result<(), String> {
        if let Some(i) = self.position(id)? {
            self.rows.entries[i] = value;
        } else {
            self.rows.entries.push(value);
        }
        Ok(())
    }
    fn remove(&mut self, id: &str) -> Result<(), String> {
        if let Some(i) = self.position(id)? {
            self.rows.entries.remove(i);
        }
        Ok(())
    }
    fn replace(&mut self, entry: &installed::Entry) -> Result<(), String> {
        let key = entry.in_game_mod_id().unwrap_or(&entry.project);
        let encoded = serde_json::to_value(entry).map_err(|e| e.to_string())?;
        let mut fields = match self.row(key)? {
            Some(Value::Object(previous)) => previous.clone(),
            Some(_) => return Err(unproved_mods()),
            None => Map::new(),
        };
        fields.remove("launcher_install_hold");
        fields.remove("verdict");
        fields.remove("mod_id");
        fields.remove("repository");
        let Value::Object(encoded) = encoded else {
            unreachable!("Entry serializes as an object")
        };
        fields.extend(encoded);
        self.set(key, Value::Object(fields))
    }
    fn write(&self, home: &Path) -> Result<(), String> {
        let mut map = self.map.clone();
        map.insert("entries".into(), Value::Array(self.rows.entries.clone()));
        let bytes = serde_json::to_vec(&map).map_err(|e| e.to_string())?;
        let path = home.join(installed::MANIFEST);
        fs::write(&path, bytes).map_err(|e| at(&path, e))
    }
}

fn held_change_refusal(id: &str) -> InstallError {
    InstallError::Refused(trf(
        "Couldn't verify a usable replacement for the installed hold on {mod}. Nothing was changed.",
        &[("mod", id)],
    ))
}

fn unproved_mods() -> String {
    tr("Sunrise couldn't verify the installed mod set. Nothing was installed.").into()
}
fn at(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}

mod github;
mod local;
mod mutations;
#[cfg(test)]
mod tests;
#[cfg(all(test, debug_assertions, unix))]
mod update_tests;
pub use github::{add_github, install_github_manual, update_github, ManualRelease};
pub use local::{install_local, install_local_at, restore_held};
pub(crate) use mutations::check_activation;
pub use mutations::{remove_mods, ChangeImpact};
