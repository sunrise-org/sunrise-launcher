//! The no-change fast path (sunrise-org/sunrise-launcher#593): a set of mods built and installed
//! before is found again before its SDK check, so a Play with nothing changed skips the SDK tool's
//! check of every mod.
//!
//! The installed build is named by its version ([`version_for`]), and that is the one identity
//! the lookup trusts. From a private copy of the mods (a stable view of them), it works the
//! version out as planning does, but from the SDK indexes already kept on disk and nothing
//! fetched, and then proves the build installed under it is the one that version names:
//!
//! - the build's [`EVIDENCE`], first sealed by the planned path right after its verified outputs
//!   were published, holds the plan's inputs that change the build without being in its version
//!   ([`Image`]: the pins, the Cargo and dispatch orders, the mods left out, the digest of each
//!   SDK index read and the identity of the SDK the build compiles against ([`BuildSdk`])).
//!   They must be the ones the lookup works out. An installed build with no evidence is never
//!   sealed afterwards: a capture proves what is on disk now, not what built it.
//! - The rules that admitted the plan without changing the build ([`Admission`]: the launcher's
//!   own checks, the base, an exact profile build, the SDK base) must be the lookup's.
//! - The build's record (`native-mods.json`) and its installed mods must be the plan's
//!   ([`Contents`]).
//! - The installed game must still be the one the evidence sealed ([`Artifact`]: the executable
//!   and every file it loads beside itself).
//!
//! Then the profile is read again, and the mods' folder is hashed as the version hashes the mods
//! ([`hash_bundles`], in order, uses tables included) and compared with the copy as it was taken:
//! a change while the build was proved misses. Anything else, or a missing, torn or unknown evidence file, plans
//! the build as before.
use super::*;
use crate::managed_play::Artifact;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Beside an installed client build: what the fast path proves it with ([`Evidence`]).
pub(super) const EVIDENCE: &str = ".native-reuse.json";

/// What the launcher before this one kept beside a build. Never read, and never carried into
/// a new build, where an older launcher could take it for the new build's evidence.
pub(super) const OLDER_EVIDENCE: &str = ".native-inputs";

/// The evidence's layout. An unknown layout is unreadable, not missing.
const SCHEMA: u32 = 2;

/// How many admissions one build keeps (each caller and base that planned it has one).
const ADMISSIONS_KEPT: usize = 8;

/// What a build used beyond its version: the version hashes the revision, the horizon-mixins
/// key, the profile, the target, the platform key, the requested mods' files and the mods left
/// out, and nothing of these.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Image {
    /// Each kept mod's horizon-mixins pin, by id: what its copy's manifest was given.
    pins: BTreeMap<String, String>,
    /// The kept mods in Cargo link order: the order the build tool was given them in.
    link: Vec<String>,
    /// The kept mods in the dispatch order the instance's overrides allow.
    dispatch: Vec<String>,
    /// The mods left out, sorted.
    left_out: Vec<String>,
    /// The SHA-256 of each signed SDK index the plan read, by build commit.
    sdk: BTreeMap<String, String>,
    /// The SHA-256 of the developer SDK's index, when a pin was read from it.
    local_sdk: Option<String>,
    /// The SDK the build compiles against, whether or not any pin was read from it.
    build_sdk: BuildSdk,
    mixins_from_source: bool,
    tool: CompilerTool,
}

/// The identity of the SDK a build compiles against.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub(super) enum BuildSdk {
    /// A signed SDK release: the build commit and the SHA-256 of its index as signed.
    Signed { commit: String, sha256: String },
    /// A developer's prebuilt game: the SHA-256 of its `sunrise-prebuilt.json`, which lists the
    /// digest of every file of the package.
    Local { sha256: String },
}

/// The selected compiler for the installed image, independent of its SDK inputs.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
enum CompilerTool {
    Signed,
    Local { sha256: String },
    Unavailable,
}

/// What the developer's prebuilt game folder ([`sdk_cache::DEVELOPER_SDK`]) names its package by
/// (`tools/prebuilt_game.py build`).
const PREBUILT_IDENTITY: &str = "sunrise-prebuilt.json";

/// The SDK selected for a build, carried through its plan: the one its identity is recorded
/// from is the one it is built with.
#[derive(Clone, Debug)]
pub(super) enum Chosen {
    Signed(Box<sdk_cache::Sdk>),
    Local { folder: PathBuf, sha256: String },
}

impl Chosen {
    /// A developer base's own prebuilt game in `local` when it has one, else `release`, the
    /// signed SDK of the build, else the one `published` gives (a developer commit's release).
    pub(super) fn select(
        local: Option<&Path>,
        release: Option<sdk_cache::Sdk>,
        published: impl FnOnce() -> Result<sdk_cache::Sdk, Failed>,
    ) -> Result<Self, Failed> {
        if let Some(folder) = local {
            let path = folder.join(PREBUILT_IDENTITY);
            let bytes = fs::read(&path).map_err(|error| {
                trf(
                    "Could not read the developer SDK manifest {manifest}: {kind}.",
                    &[
                        ("manifest", PREBUILT_IDENTITY),
                        ("kind", &error.kind().to_string()),
                    ],
                )
            })?;
            return Ok(Self::Local {
                folder: folder.to_owned(),
                sha256: sunrise_launcher_release_verify::sha256_hex(&bytes),
            });
        }
        match release {
            Some(sdk) => Ok(Self::Signed(Box::new(sdk))),
            None => published().map(|sdk| Self::Signed(Box::new(sdk))),
        }
    }

    fn identity(&self) -> BuildSdk {
        match self {
            Self::Signed(sdk) => BuildSdk::Signed {
                commit: sdk.commit().to_owned(),
                sha256: sdk.signed().to_owned(),
            },
            Self::Local { sha256, .. } => BuildSdk::Local {
                sha256: sha256.clone(),
            },
        }
    }
}

impl Image {
    /// `mods` (the kept mods, with their `pins`), in `link` order and `dispatch` order, with
    /// `left_out` left out, the indexes `read` and the SDK `chosen` to compile against.
    pub(super) fn new(
        mods: &[Bundle],
        pins: &[String],
        link: &[Bundle],
        dispatch: &[String],
        left_out: &[LeftOut],
        read: &mixins::Read,
        chosen: &Chosen,
    ) -> Self {
        let mut left: Vec<String> = left_out.iter().map(|left| left.id.clone()).collect();
        left.sort_unstable();
        Self {
            pins: mods
                .iter()
                .zip(pins)
                .map(|(bundle, pin)| (bundle.manifest.id.clone(), pin.clone()))
                .collect(),
            link: link
                .iter()
                .map(|bundle| bundle.manifest.id.clone())
                .collect(),
            dispatch: dispatch.to_vec(),
            left_out: left,
            sdk: read.sdk.clone(),
            local_sdk: read.local.clone(),
            build_sdk: chosen.identity(),
            mixins_from_source: MIXINS_FROM_SOURCE.load(std::sync::atomic::Ordering::Relaxed),
            tool: match chosen {
                Chosen::Signed(_) => CompilerTool::Signed,
                Chosen::Local { .. } => sdk_cache::developer_tool(true)
                    .and_then(|tool| sunrise_launcher_smod::index::sha256_file(&tool).ok())
                    .map_or(CompilerTool::Unavailable, |sha256| CompilerTool::Local {
                        sha256,
                    }),
            },
        }
    }
}

/// What admitted a plan without changing what it built: the launcher whose checks passed it,
/// the version it started from, whether it was a profile's exact build, and where SDK indexes
/// come from.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Admission {
    launcher: String,
    base: String,
    origin: String,
    exact: bool,
    /// A fingerprint of [`sdk_cache::BASE_VAR`] ([`sdk_base_fingerprint`]), never the setting:
    /// a mirror's address can carry its credentials.
    sdk_base: Option<String>,
}

impl Admission {
    pub(super) fn new(base: &str, origin: &str, exact: bool) -> Self {
        Self::with_sdk_base(
            base,
            origin,
            exact,
            std::env::var_os(sdk_cache::BASE_VAR).as_deref(),
        )
    }

    /// [`Self::new`], with `sdk_base` as the SDK base setting.
    pub(super) fn with_sdk_base(
        base: &str,
        origin: &str,
        exact: bool,
        sdk_base: Option<&std::ffi::OsStr>,
    ) -> Self {
        Self {
            launcher: backend::VERSION.to_owned(),
            base: base.to_owned(),
            origin: origin.to_owned(),
            exact,
            sdk_base: sdk_base.map(sdk_base_fingerprint),
        }
    }
}

/// What the evidence keeps of the SDK base setting: enough to tell another setting from this
/// one, as #622's key hashed it, and nothing to read it back from.
fn sdk_base_fingerprint(setting: &std::ffi::OsStr) -> String {
    let mut hash = Sha256::new();
    hash.update(b"sunrise-launcher sdk base\0");
    hash.update(setting.as_encoded_bytes());
    format!("{:x}", hash.finalize())
}

/// The file beside a build ([`EVIDENCE`]).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Evidence {
    schema: u32,
    version: String,
    image: Image,
    /// The installed game when the evidence was written.
    seal: Artifact,
    /// Newest last.
    admitted: Vec<Admission>,
}

impl Evidence {
    /// Missing evidence is `None`; unreadable evidence is an error.
    fn read(dir: &Path) -> Result<Option<Self>, ()> {
        let bytes = match fs::read(dir.join(EVIDENCE)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(()),
        };
        let evidence: Self = serde_json::from_slice(&bytes).map_err(|_| ())?;
        if evidence.schema != SCHEMA {
            return Err(());
        }
        Ok(Some(evidence))
    }

    /// Prove the installed mods and game are this plan's: the build's own evidence names this
    /// image and its game is still the sealed one. A build with no evidence is never proved, and
    /// never sealed here.
    fn prove(version: &str, contents: &Contents<'_>, image: &Image) -> Option<Self> {
        if image.tool == CompilerTool::Unavailable || !contents.installed_in(version) {
            return None;
        }
        let dir = Target::Client.directory(version);
        let evidence = Self::read(&dir).ok()??;
        if evidence.version != version || evidence.image != *image {
            return None;
        }
        let seal = Artifact::capture_at(version, &dir).ok()?;
        (evidence.seal == seal).then_some(evidence)
    }

    /// The first seal of a build this launcher has just published: its installed mods are the
    /// plan's, it has no evidence yet, and its game can be captured.
    fn seal(version: &str, contents: &Contents<'_>, image: &Image) -> Option<Self> {
        if image.tool == CompilerTool::Unavailable || !contents.installed_in(version) {
            return None;
        }
        let dir = Target::Client.directory(version);
        if !matches!(Self::read(&dir), Ok(None)) {
            return None;
        }
        Some(Self {
            schema: SCHEMA,
            version: version.to_owned(),
            image: image.clone(),
            seal: Artifact::capture_at(version, &dir).ok()?,
            admitted: Vec::new(),
        })
    }
}

/// What an installed build of a plan holds: its record and its mods.
pub(super) struct Contents<'a> {
    pub(super) revision: &'a str,
    pub(super) origin: &'a str,
    pub(super) mixins: &'a str,
    pub(super) profile: Profile,
    /// The kept mods, with each one's source digest in `digests`.
    pub(super) kept: &'a [Bundle],
    pub(super) digests: &'a HashMap<String, Option<String>>,
    pub(super) left_out: &'a [LeftOut],
}

impl Contents<'_> {
    /// Whether installed client build `version` is this: its record names the revision, the
    /// release it started from, the horizon-mixins key, the profile and the mods left out, it
    /// includes exactly the kept mods at their versions and digests (as Play checks them,
    /// `native_status::included`), and holds exactly their copies.
    fn installed_in(&self, version: &str) -> bool {
        let dir = Target::Client.directory(version);
        let Ok(Some(about)) = checked_about(version) else {
            return false;
        };
        let mut left: Vec<&str> = about.left_out.iter().map(|l| l.id.as_str()).collect();
        let mut planned: Vec<&str> = self.left_out.iter().map(|l| l.id.as_str()).collect();
        left.sort_unstable();
        planned.sort_unstable();
        if about.target != Target::Client
            || about.sunrise != self.revision
            || about.base != self.origin
            || left != planned
        {
            return false;
        }
        let Some(record) = fs::read(dir.join(RECORD))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        else {
            return false;
        };
        if record["horizon_mixins"].as_str() != Some(self.mixins)
            || record["profile"].as_str() != Some(self.profile.name())
        {
            return false;
        }
        let Some(expected) = self
            .kept
            .iter()
            .map(|bundle| {
                let digest = self.digests.get(&bundle.manifest.id)?.as_ref()?;
                let id = bundle.manifest.id.clone();
                Some((
                    id,
                    bundle.manifest.version.clone(),
                    digest.to_ascii_lowercase(),
                ))
            })
            .collect::<Option<BTreeSet<_>>>()
        else {
            return false;
        };
        let Ok(included) = crate::native_status::included(version) else {
            return false;
        };
        let held: BTreeSet<_> = included
            .into_iter()
            .map(|m| (m.id, m.version, m.sha256.to_ascii_lowercase()))
            .collect();
        let copies: Option<BTreeSet<String>> =
            fs::read_dir(dir.join(sunrise_launcher_smod::mod_list::NATIVE_DIR))
                .ok()
                .map(|entries| {
                    entries
                        .flatten()
                        .filter_map(|entry| entry.file_name().into_string().ok())
                        .collect()
                });
        let kept: BTreeSet<String> = self.kept.iter().map(|b| b.manifest.id.clone()).collect();
        held == expected && copies.as_ref() == Some(&kept)
    }
}

/// How many versions a plan tries ([`versions`]) before it gives up.
const VERSIONS_TRIED: usize = 8;

/// The versions a client plan installs as, in order: its own (`version_for`), then one after
/// another. A build installed under one that doesn't hold the plan ([`Evidence::prove`]) is never
/// used for the plan, sealed, or replaced while something may still run it: the plan goes on to
/// the next version in the same sequence.
pub(super) fn versions(version: &str) -> impl Iterator<Item = String> + '_ {
    (0..VERSIONS_TRIED).map(move |n| {
        if n == 0 {
            return version.to_owned();
        }
        let mut hash = Sha256::new();
        hash.update(b"in place of\0");
        hash.update(version.as_bytes());
        hash.update((n as u64).to_le_bytes());
        let key: String = hash.finalize()[..6]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let head = version
            .get(..version.len().saturating_sub(12))
            .unwrap_or(version);
        format!("{head}{key}")
    })
}

/// Where a plan goes in the version sequence.
enum Slot {
    /// An installed build that holds the plan.
    Proved(String, Box<Evidence>),
    /// A path nothing is at: publication can take it.
    Vacant(String),
}

/// The installed build that holds this plan, wherever it is in the version sequence; failing
/// that, the first vacant path. A path that is anything at all (a directory without its
/// completion marker, a file, a link, one that can't be read) is not vacant: publication
/// refuses it.
fn slot(version: &str, contents: &Contents<'_>, image: &Image) -> Option<Slot> {
    let mut vacant = None;
    for candidate in versions(version) {
        let path = Target::Client.directory(&candidate);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                vacant.get_or_insert(candidate);
            }
            _ if installed(&candidate) => {
                if let Some(evidence) = Evidence::prove(&candidate, contents, image) {
                    return Some(Slot::Proved(candidate, Box::new(evidence)));
                }
            }
            _ => {}
        }
    }
    vacant.map(Slot::Vacant)
}

/// The version the client plan installs as ([`versions`]), with proof if an installed build
/// holds it.
pub(super) fn installable(
    version: &str,
    contents: &Contents<'_>,
    image: &Image,
) -> Result<(String, Option<Evidence>), Failed> {
    match slot(version, contents, image) {
        Some(Slot::Proved(version, evidence)) => Ok((version, Some(*evidence))),
        Some(Slot::Vacant(version)) => Ok((version, None)),
        None => {
            crate::launcher_log::line(
                "Native build refused: no vacant or proved installed image slot",
            );
            Err(Failed::Other(trf(
                "The installed builds for these mods ({version}) hold different mods or were changed after they were installed. Remove them, then build again.",
                &[("version", version)],
            )))
        }
    }
}

fn installed(version: &str) -> bool {
    Target::Client
        .directory(version)
        .join(".installed")
        .is_file()
}

/// The first seal of a build just published for the plan, kept with its admission. Only the
/// verified publication of this launcher's own outputs calls it. A failure costs only the next
/// build's planning.
pub(super) fn seal_published(
    version: &str,
    contents: &Contents<'_>,
    image: &Image,
    admission: Admission,
) {
    if let Some(evidence) = Evidence::seal(version, contents, image) {
        keep(evidence, admission);
    }
}

/// The admission added to the proved evidence of a build the plan uses again.
pub(super) fn admit(evidence: Evidence, admission: Admission) {
    keep(evidence, admission);
}

fn keep(mut evidence: Evidence, admission: Admission) {
    let admitted = &mut evidence.admitted;
    admitted.retain(|old| *old != admission);
    admitted.push(admission);
    let drop = admitted.len().saturating_sub(ADMISSIONS_KEPT);
    admitted.drain(..drop);
    if let Ok(bytes) = serde_json::to_vec(&evidence) {
        let dir = Target::Client.directory(&evidence.version);
        let _ = sdk_cache::write_whole(&dir.join(EVIDENCE), &bytes);
    }
}

/// What `pipeline` looks a build up with: its own arguments.
pub(super) struct Request<'a> {
    pub(super) base: &'a str,
    pub(super) folder: &'a Path,
    pub(super) built_on: Option<&'a About>,
    pub(super) platform: BuildPlatform,
    pub(super) profile: Profile,
    pub(super) target: Target,
    pub(super) options: &'a BuildOptions<'a>,
    /// The developer base's own prebuilt game ([`sdk_cache::DEVELOPER_SDK`]), for a developer base.
    pub(super) local_sdk: Option<&'a Path>,
}

/// What planning would make of a request, worked out without fetching or checking anything.
#[derive(Debug, PartialEq)]
struct Derived {
    version: String,
    image: Image,
    admission: Admission,
    revision: String,
    origin: String,
    mixins: String,
    mods: Vec<Bundle>,
    ordered: Vec<Bundle>,
    left_out: Vec<LeftOut>,
}

/// The pin of each of `mods` from the SDK indexes kept on disk only.
fn kept_pins(
    mods: &[Bundle],
    build_sdk: Option<&sdk_cache::Sdk>,
    local: Option<&Path>,
) -> Option<(Vec<String>, mixins::Read)> {
    mixins::read_pins(mods, build_sdk, local, |pin| {
        sdk_cache::kept(pin).ok_or_else(|| Failed::Other(String::new()))
    })
    .ok()
}

/// The build id a request compiles against, from what is kept: a release's is fetched by the
/// planner, never by a lookup.
fn kept_revision(base: &str, built_on: Option<&About>, developer: bool) -> Option<String> {
    match built_on {
        Some(about) => Some(about.sunrise.clone()),
        None if developer => crate::developer_install::revision(base).ok(),
        None => sdk_cache::kept_build_of(base),
    }
}

/// `request`'s version and plan inputs, as `pipeline` works them out, from `requested` (the
/// mods' private copies, which take their pins as planning gives them). None when planning
/// would need anything not on disk, or would stop.
fn derive(request: &Request<'_>, requested: &[Bundle]) -> Option<Derived> {
    let Request {
        base,
        folder,
        built_on,
        platform,
        profile,
        target,
        options,
        local_sdk,
    } = *request;
    let side = side_of(target);
    let (left_out, mods, declared) = kept_mods(requested, options.skip, side);
    // A profile's exact build never installs a set with mods left out.
    if mods.is_empty() || (options.exact_home.is_some() && !left_out.is_empty()) {
        return None;
    }
    let instance = instance_of(folder, options);
    let overrides =
        dependencies::Overrides::read(&instance.join(sunrise_launcher_smod::mod_list::DIR)).ok()?;
    let (dispatch, links) =
        resolve_build_dependencies(instance, &mods, &declared, side, &overrides).ok()?;
    let ordered = links
        .order
        .iter()
        .map(|&i| mods.get(i).cloned())
        .collect::<Option<Vec<_>>>()?;
    let origin = built_on.map_or(base, |about| about.base.as_str());
    let developer = crate::developer_install::is_developer(origin);
    let revision = kept_revision(base, built_on, developer)?;
    let sdk = match developer {
        true if mismatch(&mods, &revision).is_some() => return None,
        true => None,
        false => Some(sdk_cache::kept(&revision)?),
    };
    let (pins, mut read) = kept_pins(&mods, sdk.as_ref(), local_sdk)?;
    if let Some(sdk) = &sdk {
        read.sdk.insert(revision.clone(), sdk.signed().to_owned());
    }
    let chosen = Chosen::select(local_sdk, sdk, || {
        sdk_cache::kept(&revision).ok_or_else(|| Failed::Other(String::new()))
    })
    .ok()?;
    for (bundle, pin) in mods.iter().zip(&pins) {
        mixins::materialize(bundle, &bundle.dir, pin).ok()?;
    }
    let mixins = mixins::for_key(&pins, |pin| {
        remembered(
            &root().join(DATES),
            "horizon-rust/horizon-mixins",
            pin,
            |_, _| Err(String::new()),
        )
    })
    .ok()?;
    let version = version_for(
        &revision,
        &mixins,
        profile,
        target,
        platform_key(developer, platform, origin).as_deref(),
        requested,
        &left_out,
    )
    .ok()?;
    Some(Derived {
        image: Image::new(&mods, &pins, &ordered, &dispatch, &left_out, &read, &chosen),
        admission: Admission::new(base, origin, options.exact_home.is_some()),
        version,
        revision,
        origin: origin.to_owned(),
        mixins,
        mods,
        ordered,
        left_out,
    })
}

/// An installed build found for a request: its version, and its mods in dependency order.
#[derive(Debug)]
pub(super) struct Hit {
    pub(super) version: String,
    ordered: Vec<Bundle>,
}

/// The installed client build `request` would plan, proved as the module says, from
/// `requested`: private copies of `request`'s mods not yet given their pins.
pub(super) fn lookup(request: &Request<'_>, requested: &[Bundle]) -> Option<Hit> {
    if request.target != Target::Client {
        return None;
    }
    let digests: HashMap<String, Option<String>> = requested
        .iter()
        .map(|bundle| (bundle.manifest.id.clone(), source_digest(bundle)))
        .collect();
    // The mods as the version hashes them, before the copies are given their pins.
    let inputs = |bundles: &[Bundle]| {
        let mut hash = Sha256::new();
        hash_bundles(&mut hash, bundles).ok()?;
        Some(hash.finalize())
    };
    let raw = inputs(requested)?;
    let derived = derive(request, requested)?;
    let contents = Contents {
        revision: &derived.revision,
        origin: &derived.origin,
        mixins: &derived.mixins,
        profile: request.profile,
        kept: &derived.mods,
        digests: &digests,
        left_out: &derived.left_out,
    };
    // Check admission on the same evidence that proved the image and seal.
    let Slot::Proved(version, evidence) = slot(&derived.version, &contents, &derived.image)? else {
        return None;
    };
    if !evidence.admitted.contains(&derived.admission) {
        return None;
    }
    // Read again: a profile, override or kept index that changed meanwhile, or a mod changed in
    // the folder since it was copied, is another build.
    let unchanged = || {
        derive(request, requested).as_ref() == Some(&derived)
            && scan_bundles(request.folder)
                .ok()
                .and_then(|found| inputs(&found))
                .is_some_and(|now| now == raw)
    };
    unchanged().then(|| Hit {
        version,
        ordered: derived.ordered,
    })
}

/// [`lookup`], timed in the build log, and the build it finds used again ([`reused`]).
pub(super) fn find(
    request: &Request<'_>,
    requested: &[Bundle],
    progress: &impl Fn(Step),
    started: std::time::Instant,
) -> Result<Option<Built>, Failed> {
    if request.target != Target::Client {
        return Ok(None);
    }
    let looking = std::time::Instant::now();
    let hit = lookup(request, requested);
    timed(tr("Looking up the installed build"), looking)?;
    hit.map(|hit| {
        reused(
            hit,
            request.profile,
            request.target,
            request.options,
            progress,
            started,
        )
    })
    .transpose()
}

/// [`find`] for a build of `folder`'s mods on `base`, before anything of the build is planned or
/// checked: what [`build`] tries before it checks the mods to choose a profile. Whatever stops
/// this (an unprepared base, a duplicate mod) finds nothing, and the build then says it as before.
pub(super) fn before_planning(
    base: &str,
    folder: &Path,
    profile: Profile,
    options: &BuildOptions<'_>,
    progress: &impl Fn(Step),
) -> Result<Option<Built>, Failed> {
    let started = std::time::Instant::now();
    let (Ok(built_on), Ok(()), Ok(platform)) = (
        checked_about(base),
        backend::need_prepared(base),
        platform_for_game(base),
    ) else {
        return Ok(None);
    };
    let Ok(found) = scan_bundles(folder) else {
        return Ok(None);
    };
    let mut ids = std::collections::HashSet::new();
    if found.is_empty() || !found.iter().all(|b| ids.insert(&b.manifest.id)) {
        return Ok(None);
    }
    fs::create_dir_all(root()).map_err(|e| e.to_string())?;
    let copies = tempfile::Builder::new()
        .prefix(".native-input-")
        .tempdir_in(root())
        .map_err(|e| e.to_string())?;
    let mut requested = Vec::new();
    for bundle in found {
        let dir = copies.path().join(&bundle.manifest.id);
        if copy_bundle(&bundle, &dir).is_err() {
            return Ok(None);
        }
        requested.push(Bundle { dir, ..bundle });
    }
    let origin = built_on.as_ref().map_or(base, |about| about.base.as_str());
    let local_sdk = developer_sdk(crate::developer_install::is_developer(origin));
    let request = Request {
        base,
        folder,
        built_on: built_on.as_ref(),
        platform,
        profile,
        target: Target::Client,
        options,
        local_sdk: local_sdk.as_deref(),
    };
    find(&request, &requested, progress, started)
}

/// Whether Play's exact-profile fast path (`build_exact_client` -> `pipeline` -> `reuse::find` ->
/// [`lookup`]) already has a proved, admitted installed client build for the enabled native mods
/// in `home` on `base` with build `profile`. Reads only kept SDK indexes and installed files on
/// disk; fetches nothing and writes only inside a transient temp directory that drops on return.
pub(crate) fn exact_client_reusable(base: &str, home: &Path, profile: Profile) -> bool {
    if backend::native_mod_architecture_error().is_some() {
        return false;
    }
    let (Ok(built_on), Ok(()), Ok(platform)) = (
        checked_about(base),
        backend::need_prepared(base),
        platform_for_game(base),
    ) else {
        return false;
    };
    let Ok(temp) = tempfile::Builder::new()
        .prefix("sunrise-native-reuse-")
        .tempdir()
    else {
        return false;
    };
    let staged = temp.path().join("staged");
    if !stage_enabled_readonly(home, &staged) {
        return false;
    }
    let Ok(found) = scan_bundles(&staged) else {
        return false;
    };
    let mut ids = std::collections::HashSet::new();
    if found.is_empty() || !found.iter().all(|b| ids.insert(&b.manifest.id)) {
        return false;
    }
    let incoming = temp.path().join("incoming");
    let mut requested = Vec::with_capacity(found.len());
    for bundle in found {
        let dir = incoming.join(&bundle.manifest.id);
        if copy_bundle(&bundle, &dir).is_err() {
            return false;
        }
        requested.push(Bundle { dir, ..bundle });
    }
    let origin = built_on.as_ref().map_or(base, |about| about.base.as_str());
    let local_sdk = developer_sdk(crate::developer_install::is_developer(origin));
    let options = BuildOptions {
        exact_home: Some(home),
        ..BuildOptions::default()
    };
    let request = Request {
        base,
        folder: &staged,
        built_on: built_on.as_ref(),
        platform,
        profile,
        target: Target::Client,
        options: &options,
        local_sdk: local_sdk.as_deref(),
    };
    lookup(&request, &requested).is_some()
}

/// Read-only staging of enabled native `.smod` archives in `home` into a transient folder,
/// matching `stage_installed` without locking or mutating `home`.
fn stage_enabled_readonly(home: &Path, out: &Path) -> bool {
    let mut paths: Vec<PathBuf> = fs::read_dir(home.join(INSTALLED))
        .map(|dir| dir.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    paths.retain(|path| path.extension().is_some_and(|ext| ext == "smod") && path.is_file());
    paths.sort();
    let mut seen = std::collections::HashSet::new();
    let mut count = 0usize;
    for path in paths {
        let Ok(bundle) = sunrise_launcher_smod::index::native::Bundle::read(&path) else {
            continue;
        };
        if !crate::browser::mod_list::is_enabled(home, &bundle.manifest.id) {
            continue;
        }
        if !seen.insert(bundle.manifest.id.clone()) {
            return false;
        }
        let dir = out.join(&bundle.manifest.id);
        for (name, bytes) in &bundle.files {
            let to = dir.join(name);
            if fs::create_dir_all(to.parent().unwrap_or(&dir)).is_err()
                || fs::write(&to, bytes).is_err()
            {
                return false;
            }
        }
        count += 1;
    }
    count > 0
}

/// The found build, used again as the planned path uses an installed one: kept for a staged
/// profile, said to be up to date, and counted as the newest of the builds kept (`prune`).
pub(super) fn reused(
    hit: Hit,
    profile: Profile,
    target: Target,
    options: &BuildOptions<'_>,
    progress: &impl Fn(Step),
    started: std::time::Instant,
) -> Result<Built, Failed> {
    log_internals(&hit.ordered)?;
    if let Some(stage) = options.staging {
        let _publication = crate::run_leases::installation().map_err(cache_error)?;
        stage.retain_target(&hit.version)?;
    }
    timed(tr("Planning the build"), started)?;
    options.cancellation.check()?;
    crate::launcher_log::line(
        "Native build reused: unchanged inputs match a sealed installed image",
    );
    progress(Step::Plan(Plan::UpToDate, profile));
    let _ = fs::write(target.directory(&hit.version).join(".installed"), "");
    Ok(Built {
        left_out: about(&hit.version)
            .map(|about| about.left_out)
            .unwrap_or_default(),
        version: hit.version,
        plan: Plan::UpToDate,
        profile,
        secs: started.elapsed().as_secs(),
    })
}

#[cfg(test)]
mod tests;
