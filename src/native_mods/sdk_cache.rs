//! The mod SDK of one game build, as the launcher fetches and keeps it (SDK design §1, §3, §4,
//! §14): its signed index, `sdk.json`, first; then only the slices the instance's route needs,
//! each unpacked into a folder of its own by [`sdk::unpack`] and used again after a cheap stamp
//! check; and the small files the fit test and planning read (`surface.json`, `build-data`).
//!
//! Where it comes from: `<base>/sdk-<build id>/<file>`, the base being this repository's release
//! downloads, or `SUNRISE_SDK_BASE` ([`BASE_VAR`]): a URL (a mirror) or a folder (a copy for
//! offline use). Every base is checked the same way: `SHA256SUMS` signed by a release key for the
//! tag `sdk-<build id>`, `sdk.json`'s SHA-256 in it, then each file's size and SHA-256 in
//! `sdk.json`. A game release carries a copy of its SDK's `sdk.json` under the same signature
//! scheme, and that copy is where the game's build id comes from ([`build_of`]).
//!
//! What is kept, under the launcher's native build folder:
//! - `sdk/<build id>/`: `sdk.json` (as verified), `surface.json`, `build-data/`; a few MB, kept,
//!   so a mod's fit is told without a download once a build was seen.
//! - `sdk/versions/<game version>`: the build id that version's release named.
//! - `prebuilt/<build id>-<target>[-server]/`: the compiled slices (`runtime/`, `api/`,
//!   `engine/`), each with `<slice>.stamp` beside it, written while the build lease is held; the
//!   folder is one storage unit, evicted whole with what an unpack left in it (`.partial`,
//!   `.lock`, a download).
use super::Target;
use crate::i18n::{tr, trf};
use crate::rust_toolchain;
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use sunrise_launcher_release_verify::{PublicKey, SIGNATURE, SIGNATURE_V2, SUMS};
use sunrise_launcher_sdk::{self as sdk, Asset, Index, Kind, Side, INDEX};

mod game;
pub(crate) use game::{check_game_offer, offers_game, stage_game, GameSlices};

/// The SDK's three compiled slices of one build, each in its own folder (`runtime`, `api`, `engine`).
#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    pub runtime: PathBuf,
    pub api: PathBuf,
    pub engine: PathBuf,
}

impl Package {
    /// The slices under the build folder `build`.
    pub fn at(build: &Path) -> Self {
        Self {
            runtime: build.join("runtime"),
            api: build.join("api"),
            engine: build.join("engine"),
        }
    }
}

/// Replaces the base SDK releases are read from: a URL, or a folder holding `sdk-<build id>/`
/// folders (and game releases' folders, for their build ids).
pub const BASE_VAR: &str = "SUNRISE_SDK_BASE";
/// Where SDK releases are read from by default.
const DEFAULT_BASE: &str = sunrise_launcher_addresses::SDK_RELEASES;
/// The most an index, checksum list or signature file is read to, in bytes.
const SMALL: u64 = 1 << 20;
/// The most a slice or the build data unpacks to: a corrupt or hostile archive stops there.
const UNPACKED_LIMIT: u64 = 8 << 30;
/// What [`sdk::unpack`]'s destination gets beside it once it is unpacked and listed.
const STAMP_SUFFIX: &str = ".stamp";

/// Why an SDK can't be used. Every reader of downloaded bytes here ends in one of these.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The build has no mod SDK (its SDK build failed or is still running).
    NoSdk { build: String },
    /// The base can't be reached and what is needed isn't kept: `bytes` to download, when known.
    Offline { build: String, bytes: Option<u64> },
    /// The SDK's signature or checksums don't hold: never used.
    Unsigned { build: String, why: String },
    /// The build's dated tag now serves another build of that day, signed for that build: the
    /// requested one's mod tools are gone, not tampered with. The signature can't say which of the
    /// two is newer.
    Superseded { build: String },
    /// The SDK is newer than this launcher reads (`what`: its schema, package format or Rust).
    UpdateLauncher { build: String, what: String },
    /// The SDK was made with an older Rust than this launcher compiles with.
    OlderRust { build: String, rust: String },
    /// A file of the SDK was refused: why, by name.
    Refused { build: String, why: String },
    /// No SDK is published for this computer.
    Host,
    /// Reading or writing the launcher's copy failed.
    Cache(String),
}

/// What a player can do about a refusal other than waiting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WayOn {
    /// Play another game build (instance settings, Installation).
    OtherBuild,
    /// The problem is Sunrise's: report it.
    Report,
    /// Nothing is wrong but the connection: build again once it is back.
    Retry,
}

/// The way on a refusal's `text` offers, found from the sentences it was written with (a build
/// reaches the player as text only).
pub fn way_on(text: &str) -> Option<WayOn> {
    // An offline refusal's size, when known, sits after the build: its opening says it all.
    let offline_opening = |text: &str| {
        let probe = Refusal::Offline {
            build: "\0".to_owned(),
            bytes: None,
        }
        .to_string();
        probe
            .split('\0')
            .next()
            .is_some_and(|opening| text.contains(opening))
    };
    let said = |refusal: Refusal| {
        let probe = refusal.to_string();
        probe.split('\0').all(|piece| text.contains(piece))
    };
    let mark = || "\0".to_owned();
    if said(Refusal::NoSdk { build: mark() })
        || said(Refusal::Superseded { build: mark() })
        || said(Refusal::OlderRust {
            build: mark(),
            rust: mark(),
        })
    {
        Some(WayOn::OtherBuild)
    } else if said(Refusal::Unsigned {
        build: mark(),
        why: mark(),
    }) {
        Some(WayOn::Report)
    } else if offline_opening(text) {
        Some(WayOn::Retry)
    } else {
        None
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::NoSdk { build } => trf(
                "The mod tools for {build} aren't ready yet. Pick another version, or try again later.",
                &[("build", build)],
            ),
            Self::Offline { build, bytes: Some(bytes) } => trf(
                "The launcher is offline and the mod tools for {build} aren't downloaded yet ({size}). Connect to the internet and try again.",
                &[("build", build), ("size", &megabytes(*bytes))],
            ),
            Self::Offline { build, bytes: None } => trf(
                "The launcher is offline and the mod tools for {build} aren't downloaded yet. Connect to the internet and try again.",
                &[("build", build)],
            ),
            Self::Unsigned { build, .. } => trf(
                "The mod tools for {build} didn't pass Sunrise's safety check, so nothing was installed and your game is untouched. Try again later; if it keeps happening, please report it.",
                &[("build", build)],
            ),
            Self::Superseded { build } => trf(
                "The mod tools for {build} aren't available any more: another build of that day replaced it. Pick another version.",
                &[("build", build)],
            ),
            Self::UpdateLauncher { build, what } => trf(
                "The mod tools for {build} need a newer launcher. Update the launcher, then build again.\n({what})",
                &[("build", build), ("what", what)],
            ),
            Self::OlderRust { build, rust } => trf(
                "The mod tools for {build} are older than this launcher can build with. Pick a newer version.\n(Rust {rust}; this launcher builds with {ours})",
                &[("build", build), ("rust", rust), ("ours", rust_toolchain::VERSION)],
            ),
            Self::Refused { build, why } => trf(
                "The mod tools for {build} were refused: {why}",
                &[("build", build), ("why", why)],
            ),
            Self::Host => {
                tr("Native mods can't be built on this computer: Sunrise has no game package for its system yet.").into()
            }
            Self::Cache(why) => trf(
                "Could not keep the mod tools: {why}",
                &[("why", why)],
            ),
        };
        f.write_str(&text)
    }
}

/// `bytes` as people read a download's size: whole megabytes, at least 1.
fn megabytes(bytes: u64) -> String {
    trf(
        "{count} MB",
        &[("count", &bytes.div_ceil(1_000_000).max(1).to_string())],
    )
}

/// A build as people read it: using the Nightly’s update name ("the Nov 13 update"),
/// found from an installed version when the index has no name, else "your Sunrise version"
/// (an imported build or a Dev build), never a commit. Anything but a build id (a mod's manifest
/// may say anything) is shown as it is, cut where a character ends.
pub fn shown(commit: &str, nightly: Option<&str>) -> String {
    let name = |name: &str| crate::nightly::update_name(name).unwrap_or_else(|| name.to_owned());
    match nightly.map(str::to_owned).or_else(|| nightly_of(commit)) {
        Some(nightly) => name(&nightly),
        None if super::is_revision(commit) => crate::i18n::tr("your Sunrise version").to_string(),
        None => commit.get(..7).unwrap_or(commit).to_owned(),
    }
}

/// Build `commit`'s Nightly name, from its kept index or an installed version made from it.
pub(super) fn nightly_of(commit: &str) -> Option<String> {
    if !super::is_revision(commit) {
        return None;
    }
    fs::read(root().join(commit).join(INDEX))
        .ok()
        .and_then(|bytes| Index::build_of(&bytes).ok())
        .and_then(|build| build.nightly)
        .or_else(|| {
            crate::backend::installed()
                .into_iter()
                .find(|tag| tag.starts_with("nightly-") && tag.ends_with(&commit[..7]))
        })
}

/// Build `commit`'s display name when it is known here (a kept index, or
/// an installed version). `commit` may come from a mod's manifest: anything but a build id is
/// never looked up.
pub fn name_of(commit: &str) -> Option<String> {
    nightly_of(commit).map(|name| shown(commit, Some(&name)))
}

/// Build `commit` as players read it, for a mod's fit: [`name_of`], else "another Sunrise
/// version", never a commit.
pub fn known_name(commit: &str) -> String {
    match name_of(commit) {
        Some(name) => name,
        None if super::is_revision(commit) => {
            crate::i18n::tr("another Sunrise version").to_string()
        }
        None => commit.get(..7).unwrap_or(commit).to_owned(),
    }
}

/// Where SDK releases are read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Base {
    /// `<url>/<tag>/<file>`.
    Url(String),
    /// `<folder>/<tag>/<file>`.
    Folder(PathBuf),
}

/// Why a file couldn't be read from a base.
#[derive(Debug, PartialEq, Eq)]
enum Unread {
    /// No connection.
    Offline,
    /// Local SDK mirror I/O, not evidence that a file is missing.
    Local(String),
    /// Any other failure, said.
    Failed(String),
}

struct Opened {
    reader: Box<dyn Read>,
    remote: bool,
}

impl Base {
    /// [`BASE_VAR`]'s base, else this repository's releases.
    pub(crate) fn from_env() -> Self {
        Self::parse(std::env::var_os(BASE_VAR))
    }

    /// An `http://` or `https://` value is a URL (without a trailing `/`); any other value a
    /// folder; nothing, or an empty value, the default base.
    fn parse(value: Option<std::ffi::OsString>) -> Self {
        let Some(value) = value.filter(|value| !value.is_empty()) else {
            return Self::Url(DEFAULT_BASE.into());
        };
        match value.to_str() {
            Some(text) if text.starts_with("https://") || text.starts_with("http://") => {
                Self::Url(text.trim_end_matches('/').to_owned())
            }
            _ => Self::Folder(PathBuf::from(value)),
        }
    }

    /// The file `name` of the release `tag`, open, with its length when the base says it;
    /// `Ok(None)` when the base has no such file. `tag` and `name` are plain names: a tag this
    /// module makes, and a name `sdk.json` lists (`Asset::parts_of` accepts only plain ones).
    fn open(&self, tag: &str, name: &str) -> Result<Option<Opened>, Unread> {
        let file = |path: PathBuf| match fs::File::open(&path) {
            Ok(file) => Ok(Some(Opened {
                reader: Box::new(file),
                remote: false,
            })),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(Unread::Local(format!("{}: {error}", path.display()))),
        };
        match self {
            Self::Url(url) => {
                let address = format!("{url}/{tag}/{name}");
                if let Some(path) = crate::draft_releases::local(&address).map_err(Unread::Local)? {
                    return file(path);
                }
                match crate::backend::get_unless_missing(&address) {
                    Ok(Some(response)) => Ok(Some(Opened {
                        reader: response.into_reader(),
                        remote: true,
                    })),
                    Ok(None) => Ok(None),
                    Err(why) if why.starts_with("No connection") => Err(Unread::Offline),
                    Err(why) => Err(Unread::Failed(why)),
                }
            }
            Self::Folder(folder) => file(folder.join(tag).join(name)),
        }
    }

    /// A small file whole (at most [`SMALL`] bytes; a larger one is refused, never cut).
    fn small(&self, tag: &str, name: &str) -> Result<Option<Vec<u8>>, Unread> {
        let Some(Opened { reader, .. }) = self.open(tag, name)? else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        reader
            .take(SMALL + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| {
                let why = format!("{name}: {error}");
                match self {
                    Self::Folder(_) => Unread::Local(why),
                    Self::Url(_) => Unread::Failed(why),
                }
            })?;
        if bytes.len() as u64 > SMALL {
            return Err(Unread::Failed(trf(
                "{name} is larger than {size}",
                &[("name", name), ("size", &megabytes(SMALL))],
            )));
        }
        Ok(Some(bytes))
    }

    pub(crate) fn release_file(
        &self,
        tag: &str,
        version: &str,
        name: &str,
    ) -> Result<Option<Vec<u8>>, Refusal> {
        self.small(tag, name).map_err(|unread| match unread {
            Unread::Offline => Refusal::Offline {
                build: version.to_owned(),
                bytes: None,
            },
            Unread::Local(why) => Refusal::Cache(why),
            Unread::Failed(why) => Refusal::Refused {
                build: version.to_owned(),
                why,
            },
        })
    }

    /// Where the base keeps the release `tag`'s file `name` on this computer, when it does.
    fn local(&self, tag: &str, name: &str) -> Option<PathBuf> {
        match self {
            Self::Folder(folder) => Some(folder.join(tag).join(name)),
            Self::Url(_) => None,
        }
    }
}

/// The SDK release's tag for `commit`: what its checksums are signed for.
fn tag(commit: &str) -> String {
    format!("sdk-{commit}")
}

/// The launcher's SDK folder: `<build dir>/native/sdk`.
fn root() -> PathBuf {
    super::root().join("sdk")
}

/// Where the launcher keeps what it reads of build `commit`'s SDK.
pub(super) fn build_folder(commit: &str) -> PathBuf {
    root().join(commit)
}

/// The index [`index`] keeps for build `commit`, read as [`index`] reads a kept one; None when
/// none is kept or it doesn't read. Nothing is fetched: what the fast path (`reuse`) compares
/// with the digest a build recorded of the index it was planned from.
pub(super) fn kept(commit: &str) -> Option<Sdk> {
    if !super::is_revision(commit) {
        return None;
    }
    let dir = build_folder(commit);
    let bytes = fs::read(dir.join(INDEX)).ok()?;
    checked(&bytes, commit, dir).ok()
}

/// Write `bytes` to `path` whole or not at all: beside it first, then renamed into place.
pub(super) fn write_whole(path: &Path, bytes: &[u8]) -> Result<(), Refusal> {
    let cache = |error: std::io::Error| Refusal::Cache(format!("{}: {error}", path.display()));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(cache)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}.new", std::process::id()));
    let temporary = PathBuf::from(temporary);
    let written = fs::write(&temporary, bytes).and_then(|()| fs::rename(&temporary, path));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written.map_err(cache)
}

fn sha256(bytes: &[u8]) -> String {
    sunrise_launcher_release_verify::sha256_hex(bytes)
}

/// One build's verified index.
#[derive(Clone, Debug)]
pub(super) struct Sdk {
    pub(super) index: Index,
    /// The SHA-256 of `sdk.json` as signed: what a stamp names.
    sha256: String,
    /// `sdk/<build id>/`.
    dir: PathBuf,
}

impl Sdk {
    pub(super) fn commit(&self) -> &str {
        &self.index.build.commit
    }

    /// The SHA-256 of `sdk.json` as signed, which names every slice's digest.
    pub(super) fn signed(&self) -> &str {
        &self.sha256
    }

    /// The build as people read it.
    pub(super) fn shown(&self) -> String {
        shown(self.commit(), self.index.build.nightly.as_deref())
    }
}

/// The verified index of build `commit`: the one kept when there is one, else fetched from
/// `base` and checked against the release's signed checksums with `keys`, then kept.
pub(super) fn index(base: &Base, commit: &str, keys: &[PublicKey]) -> Result<Sdk, Refusal> {
    index_in(&root(), base, commit, keys)
}

fn index_in(root: &Path, base: &Base, commit: &str, keys: &[PublicKey]) -> Result<Sdk, Refusal> {
    let build = shown(commit, None);
    if !super::is_revision(commit) {
        return Err(Refusal::Refused {
            build,
            why: trf("{commit} is not a build id", &[("commit", commit)]),
        });
    }
    let dir = root.join(commit);
    let kept = dir.join(INDEX);
    match fs::read(&kept) {
        Ok(bytes) => {
            // Written only once verified: read as the same checks would, without the signature.
            if let Ok(sdk) = checked(&bytes, commit, dir.clone()) {
                return Ok(sdk);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(Refusal::Cache(format!("{}: {error}", kept.display()))),
    }
    let bytes = signed_sdk_index(base, commit, keys)?;
    let sdk = checked(&bytes, commit, dir)?;
    write_whole(&kept, &bytes)?;
    Ok(sdk)
}

fn signed_sdk_index(base: &Base, commit: &str, keys: &[PublicKey]) -> Result<Vec<u8>, Refusal> {
    let build = shown(commit, None);
    let tag = tag(commit);
    let read = |name: &str| base.release_file(&tag, &build, name);
    // The index first: a build without one has no SDK, whatever else its release holds.
    let bytes = read(INDEX)?.ok_or_else(|| Refusal::NoSdk {
        build: build.clone(),
    })?;
    let sums = read(SUMS)?;
    let signature = read(SIGNATURE)?.map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    let unsigned = |why: String| Refusal::Unsigned {
        build: build.clone(),
        why,
    };
    // Signed for exactly `sdk-<build id>`: neither a game release's sums nor another build's SDK
    // pass for it.
    let signed = sunrise_launcher_release_verify::sdk_digest(
        commit,
        INDEX,
        sums.as_deref(),
        signature.as_deref(),
        keys,
    )
    .map_err(unsigned)?;
    sunrise_launcher_release_verify::check(INDEX, &signed, &sha256(&bytes)).map_err(unsigned)?;
    Ok(bytes)
}

/// `bytes` read as build `commit`'s index, its toolchain this launcher's.
fn checked(bytes: &[u8], commit: &str, dir: PathBuf) -> Result<Sdk, Refusal> {
    let build = shown(commit, None);
    let index = Index::read(bytes, commit).map_err(|refusal| match refusal {
        sdk::Refusal::Schema(schema) => Refusal::UpdateLauncher {
            build: build.clone(),
            what: trf("SDK schema {schema}", &[("schema", &schema.to_string())]),
        },
        sdk::Refusal::PackageFormat(format) => Refusal::UpdateLauncher {
            build: build.clone(),
            what: trf(
                "package format {format}",
                &[("format", &format.to_string())],
            ),
        },
        other => Refusal::Refused {
            build: build.clone(),
            why: other.to_string(),
        },
    })?;
    let build = shown(commit, index.build.nightly.as_deref());
    let rust = &index.toolchain.rust;
    match compare_versions(rust, rust_toolchain::VERSION) {
        Some(std::cmp::Ordering::Equal) => {}
        Some(std::cmp::Ordering::Greater) | None => {
            return Err(Refusal::UpdateLauncher {
                build,
                what: trf("Rust {rust}", &[("rust", rust)]),
            });
        }
        Some(std::cmp::Ordering::Less) => {
            return Err(Refusal::OlderRust {
                build,
                rust: rust.clone(),
            });
        }
    }
    Ok(Sdk {
        index,
        sha256: sha256(bytes),
        dir,
    })
}

/// Two dotted version numbers (`1.95.0`) compared part by part; `None` when either is not one.
fn compare_versions(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    let parts = |text: &str| -> Option<Vec<u64>> {
        text.split('.').map(|part| part.parse().ok()).collect()
    };
    Some(parts(a)?.cmp(&parts(b)?))
}

/// The verified `surface.json` of `sdk`'s build: reused offline if it still matches the index,
/// otherwise fetched from `base`, verified and kept.
pub(super) fn surface_file(sdk: &Sdk, base: &Base) -> Result<PathBuf, Refusal> {
    const NAME: &str = "surface.json";
    let build = sdk.shown();
    let refused = |why: String| Refusal::Refused {
        build: build.clone(),
        why,
    };
    let kept = sdk.dir.join(NAME);
    let listed = sdk.index.asset(NAME).map_err(|r| refused(r.to_string()))?;
    match fs::read(&kept) {
        Ok(bytes) if sdk.index.verify(NAME, bytes.as_slice()).is_ok() => return Ok(kept),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(Refusal::Cache(format!("{}: {error}", kept.display())));
        }
        _ => {
            let tag = tag(sdk.commit());
            let Opened { reader, .. } = base
                .open(&tag, NAME)
                .map_err(|unread| match unread {
                    Unread::Offline => Refusal::Offline {
                        build: build.clone(),
                        bytes: Some(listed.size),
                    },
                    Unread::Local(why) => Refusal::Cache(why),
                    Unread::Failed(why) => refused(why),
                })?
                .ok_or_else(|| refused(sdk::Refusal::Missing(vec![NAME.into()]).to_string()))?;
            let mut bytes = Vec::new();
            reader
                .take(listed.size.saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(|error| match base {
                    Base::Folder(_) => Refusal::Cache(format!("{NAME}: {error}")),
                    Base::Url(_) => refused(error.to_string()),
                })?;
            sdk.index
                .verify(NAME, bytes.as_slice())
                .map_err(|r| refused(r.to_string()))?;
            write_whole(&kept, &bytes)?;
        }
    }
    Ok(kept)
}

/// The folder of `sdk`'s build data (`internals.json`, `loops.json`,
/// `functions.json`): unpacked once, then used again on its stamp.
pub(super) fn build_data(sdk: &Sdk, base: &Base) -> Result<PathBuf, Refusal> {
    const NAME: &str = "build-data.tar.xz";
    let into = sdk.dir.join("build-data");
    let mut fetched = Fetched::default();
    fetch_into(sdk, base, NAME, &into, &mut fetched, &|_| {})?;
    Ok(into)
}

/// What a download reports as it goes: bytes done of the whole route's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Fetched {
    pub(super) done: u64,
    pub(super) total: u64,
}

/// The slices `route` needs of `sdk`'s build for this computer and `target`, each kept in its
/// own folder ([`Package`]): one whose stamp holds is used as it is; any other is fetched from
/// `base`, unpacked by [`sdk::unpack`] (which checks it against the index first) and stamped.
/// `lease` is the build lease: the slices and their stamps change only while it is held.
/// `progress` hears the bytes fetched of all that must be fetched, before and while.
pub(super) fn slices(
    _lease: &crate::native_cache::Cache,
    sdk: &Sdk,
    base: &Base,
    route: &str,
    target: Target,
    progress: &dyn Fn(Fetched),
) -> Result<Package, Refusal> {
    let host = rust_toolchain::host().ok_or(Refusal::Host)?;
    let side = side(target);
    let prebuilt = super::root().join("prebuilt");
    let package = slices_in(&prebuilt, sdk, base, route, host.target, side, progress)?;
    Ok(package)
}

/// The SDK route (`api`, `engine`) a build route the tool names (`prebuilt-api`, `prebuilt`) fetches.
fn route_inputs(route: &str) -> Option<&'static str> {
    match route {
        "prebuilt-api" => Some("api"),
        "prebuilt" => Some("engine"),
        _ => None,
    }
}

/// The SDK's native build tool for this computer (its `tool` route), fetched beside the slices and kept: the program to run.
pub(super) fn tool(sdk: &Sdk, base: &Base, progress: &dyn Fn(Fetched)) -> Result<PathBuf, Refusal> {
    let host = rust_toolchain::host().ok_or(Refusal::Host)?;
    tool_in(
        &super::root().join("prebuilt"),
        sdk,
        base,
        host.target,
        progress,
    )
}

fn tool_in(
    prebuilt: &Path,
    sdk: &Sdk,
    base: &Base,
    triple: &str,
    progress: &dyn Fn(Fetched),
) -> Result<PathBuf, Refusal> {
    let build = sdk.shown();
    let refused = |why: String| Refusal::Refused {
        build: build.clone(),
        why,
    };
    let names = sdk
        .index
        .route_assets("tool", triple, Side::Client)
        .map_err(|r| match r {
            sdk::Refusal::Missing(_) => Refusal::Host,
            sdk::Refusal::Route(_) => {
                refused(tr("its SDK has no native build tool: install a newer version").into())
            }
            other => refused(other.to_string()),
        })?;
    let into = prebuilt.join(format!("{}-{triple}-tool", sdk.commit()));
    let mut fetched = Fetched::default();
    for name in &names {
        if !stamp_holds(sdk, name, &into) {
            fetched.total += sdk
                .index
                .asset(name)
                .map_err(|r| refused(r.to_string()))?
                .size;
        }
    }
    progress(fetched);
    for name in &names {
        fetch_into(sdk, base, name, &into, &mut fetched, progress)?;
    }
    Ok(into.join(super::tool::PROGRAM))
}

/// Names a developer's own `cargo-sunrise-mod`, built from the same checkout as [`DEVELOPER_SDK`]'s folder: the tool its builds run.
pub(super) const DEVELOPER_TOOL: &str = "SUNRISE_LAUNCHER_DEVELOPER_TOOL";

/// The tool [`DEVELOPER_TOOL`] names, honoured as the SDK key override is (a debug launcher), or for a developer base's own SDK.
pub(super) fn developer_tool(developer_base: bool) -> Option<PathBuf> {
    (cfg!(debug_assertions) || developer_base)
        .then(|| std::env::var_os(DEVELOPER_TOOL))
        .flatten()
        .map(PathBuf::from)
        .filter(|program| program.is_file())
}

/// Names the folder of a prebuilt game built from a developer install's checkout
/// (`tools/prebuilt_game.py build`). Read for developer installs only: their commit has no release.
pub(super) const DEVELOPER_SDK: &str = "SUNRISE_LAUNCHER_DEVELOPER_SDK";

/// A developer install's own prebuilt game, the folder `local` ([`DEVELOPER_SDK`]): checked to be
/// `revision`'s for this computer and `target`, then laid out afresh as its slices by links, so
/// the developer's folder is never changed. A release's game comes from its SDK instead
/// ([`slices`]).
pub(super) fn developer(
    program: &Path,
    revision: &str,
    target: Target,
    local: &Path,
    cancellation: &super::Cancellation,
) -> Result<Package, String> {
    let host = rust_toolchain::host().ok_or_else(|| Refusal::Host.to_string())?;
    let unit = unit(revision, host.target, side(target));
    let out = super::root()
        .join("prebuilt")
        .join(format!("{unit}.developer"));
    let reply = super::tool::ask(
        crate::process::command(program),
        "native-layout",
        &serde_json::json!({
            "prebuilt": local, "build": revision, "host": host.target,
            "target": target.name(), "out": out,
        }),
        cancellation,
    )?;
    let folder = |slice: &str| {
        reply["package"][slice]
            .as_str()
            .map(PathBuf::from)
            .ok_or_else(|| tr("The native build tool laid out no package.").to_owned())
    };
    Ok(Package {
        runtime: folder("runtime")?,
        api: folder("api")?,
        engine: folder("engine")?,
    })
}

fn side(target: Target) -> Side {
    match target {
        Target::Client => Side::Client,
        Target::Server => Side::Server,
    }
}

/// The folder a build's slices for `triple` and `side` are kept in.
pub(super) fn unit(commit: &str, triple: &str, side: Side) -> String {
    match side {
        Side::Client => format!("{commit}-{triple}"),
        Side::Server => format!("{commit}-{triple}-server"),
    }
}

fn slices_in(
    prebuilt: &Path,
    sdk: &Sdk,
    base: &Base,
    route: &str,
    triple: &str,
    side: Side,
    progress: &dyn Fn(Fetched),
) -> Result<Package, Refusal> {
    let build = sdk.shown();
    let refused = |why: String| Refusal::Refused {
        build: build.clone(),
        why,
    };
    let names = sdk
        .index
        .route_assets(route_inputs(route).ok_or_else(|| refused(trf("its tool names the route {route}, which this launcher doesn't know: update the launcher", &[("route", route)])))?, triple, side)
        .map_err(|r| match r {
            sdk::Refusal::Missing(_) => Refusal::Host,
            other => refused(other.to_string()),
        })?;
    let dir = prebuilt.join(unit(sdk.commit(), triple, side));
    let package = Package::at(&dir);
    let folder = |kind: Kind| match kind {
        Kind::Runtime => Ok(package.runtime.clone()),
        Kind::Api => Ok(package.api.clone()),
        Kind::Engine => Ok(package.engine.clone()),
        other => Err(refused(trf(
            "the route lists {kind}, which is not a slice",
            &[("kind", other.name())],
        ))),
    };
    let mut wanted = Vec::new();
    for name in &names {
        let kind = Asset::parts_of(name)
            .map(|(kind, ..)| kind)
            .ok_or_else(|| refused(sdk::Refusal::AssetName(name.clone()).to_string()))?;
        wanted.push((name.as_str(), folder(kind)?));
    }
    let mut fetched = Fetched::default();
    for (name, into) in &wanted {
        if !stamp_holds(sdk, name, into) {
            fetched.total += sdk
                .index
                .asset(name)
                .map_err(|r| refused(r.to_string()))?
                .size;
        }
    }
    progress(fetched);
    for (name, into) in &wanted {
        fetch_into(sdk, base, name, into, &mut fetched, progress)?;
    }
    Ok(package)
}

/// What a stamp says: the index that listed the archive, the archive, and every file it
/// unpacked to with its size.
#[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq)]
struct Stamp {
    index: String,
    asset: String,
    files: BTreeMap<String, u64>,
}

fn stamp_path(into: &Path) -> PathBuf {
    let mut path = into.as_os_str().to_owned();
    path.push(STAMP_SUFFIX);
    PathBuf::from(path)
}

/// Every file under `dir`, `/`-separated relative to it, with its size; `None` when it can't
/// be read or holds anything but files and folders.
fn listing(dir: &Path) -> Option<BTreeMap<String, u64>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, u64>) -> Option<()> {
        for entry in fs::read_dir(dir).ok()? {
            let entry = entry.ok()?;
            let kind = entry.file_type().ok()?;
            let path = entry.path();
            if kind.is_dir() {
                walk(root, &path, out)?;
            } else if kind.is_file() {
                let relative = path.strip_prefix(root).ok()?;
                let name = relative
                    .components()
                    .map(|part| part.as_os_str().to_str())
                    .collect::<Option<Vec<_>>>()?
                    .join("/");
                out.insert(name, entry.metadata().ok()?.len());
            } else {
                return None;
            }
        }
        Some(())
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out)?;
    Some(out)
}

/// Whether `into` is `name` as `sdk`'s index lists it: its stamp names this index and this
/// archive, and the folder holds exactly the files the stamp lists, each at its size. No file is
/// hashed: the unpack checked the archive before the stamp was written.
fn stamp_holds(sdk: &Sdk, name: &str, into: &Path) -> bool {
    let Ok(text) = fs::read(stamp_path(into)) else {
        return false;
    };
    let Ok(stamp) = serde_json::from_slice::<Stamp>(&text) else {
        return false;
    };
    stamp.index == sdk.sha256 && stamp.asset == name && listing(into) == Some(stamp.files)
}

/// `name` unpacked at `into`: kept when its stamp holds; otherwise what is there is removed,
/// the archive read from `base` (downloaded beside `into` first when the base is not a folder),
/// unpacked, and stamped.
fn fetch_into(
    sdk: &Sdk,
    base: &Base,
    name: &str,
    into: &Path,
    fetched: &mut Fetched,
    progress: &dyn Fn(Fetched),
) -> Result<(), Refusal> {
    let start = fetched.done;
    fetch_into_cancellable(sdk, base, name, into, fetched, None, &|phase| {
        if let SliceProgress::Downloading(bytes) = phase {
            progress(bytes);
        }
    })?;
    if fetched.done != start {
        progress(*fetched);
    }
    Ok(())
}

enum SliceProgress {
    Downloading(Fetched),
    Unpacking,
}

fn fetch_into_cancellable(
    sdk: &Sdk,
    base: &Base,
    name: &str,
    into: &Path,
    fetched: &mut Fetched,
    stop: Option<&super::Cancellation>,
    progress: &dyn Fn(SliceProgress),
) -> Result<(), Refusal> {
    let build = sdk.shown();
    let refused = |why: String| Refusal::Refused {
        build: build.clone(),
        why,
    };
    if let Some(stop) = stop {
        stop.check().map_err(refused)?;
    }
    if stamp_holds(sdk, name, into) {
        return Ok(());
    }
    let cache = |error: std::io::Error| Refusal::Cache(format!("{}: {error}", into.display()));
    let size = sdk
        .index
        .asset(name)
        .map_err(|r| refused(r.to_string()))?
        .size;
    let stamp = stamp_path(into);
    match fs::remove_file(&stamp) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(cache(error)),
    }
    match fs::symlink_metadata(into) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(into).map_err(cache)?,
        Ok(_) => fs::remove_file(into).map_err(cache)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(cache(error)),
    }
    let parent = into.parent().expect("a slice folder has a parent");
    fs::create_dir_all(parent).map_err(cache)?;
    let tag = tag(sdk.commit());
    let start = fetched.done;
    let (archive, downloaded) = match base.local(&tag, name) {
        Some(path) => match path.try_exists() {
            Ok(true) => (path, None),
            Ok(false) => {
                return Err(refused(
                    sdk::Refusal::Missing(vec![name.into()]).to_string(),
                ))
            }
            Err(error) => return Err(Refusal::Cache(format!("{}: {error}", path.display()))),
        },
        None => {
            let Opened { reader, remote } = base
                .open(&tag, name)
                .map_err(|unread| match unread {
                    Unread::Offline => Refusal::Offline {
                        build: build.clone(),
                        bytes: Some(fetched.total.saturating_sub(fetched.done).max(size)),
                    },
                    Unread::Local(why) => Refusal::Cache(why),
                    Unread::Failed(why) => refused(why),
                })?
                .ok_or_else(|| refused(sdk::Refusal::Missing(vec![name.into()]).to_string()))?;
            let file = parent.join(format!("{name}.download"));
            let listed = sdk.index.asset(name).map_err(|r| refused(r.to_string()))?;
            let total = fetched.total;
            let report_progress = |done: u64, _| {
                if remote {
                    progress(SliceProgress::Downloading(Fetched {
                        done: start.saturating_add(done.min(size)),
                        total,
                    }));
                }
            };
            let saved = match stop {
                Some(stop) => crate::backend::save_checked_bytes(
                    stop.reader(reader),
                    size,
                    name,
                    &listed.sha256,
                    &file,
                    &report_progress,
                ),
                None => crate::backend::save_checked_bytes(
                    reader,
                    size,
                    name,
                    &listed.sha256,
                    &file,
                    &report_progress,
                ),
            };
            saved.map_err(|error| {
                if let Some(stop) = stop {
                    if let Err(why) = stop.check() {
                        return refused(why);
                    }
                }
                match error {
                    crate::backend::SaveCheckedError::Download(error) => refused(error),
                    crate::backend::SaveCheckedError::Local(error) => cache(error),
                }
            })?;
            (file.clone(), Some(file))
        }
    };
    if let Some(stop) = stop {
        if let Err(why) = stop.check() {
            if let Some(file) = downloaded {
                let _ = fs::remove_file(file);
            }
            return Err(refused(why));
        }
    }
    progress(SliceProgress::Unpacking);
    let unpacked = sdk::unpack(&sdk.index, name, &archive, into, UNPACKED_LIMIT);
    if let Some(file) = downloaded {
        let _ = fs::remove_file(file);
    }
    unpacked.map_err(|refusal| match refusal {
        // The SDK extractor also uses Archive for destination I/O during extraction.
        // Do not publish a reduced Switch candidate after an ambiguous unpack error.
        sdk::Refusal::Archive { .. }
        | sdk::Refusal::Io(_)
        | sdk::Refusal::Busy(_)
        | sdk::Refusal::NoLocks(_)
        | sdk::Refusal::Exists(_)
        | sdk::Refusal::Rename { .. } => Refusal::Cache(refusal.to_string()),
        other => refused(other.to_string()),
    })?;
    let files = listing(into).ok_or_else(|| {
        Refusal::Cache(trf(
            "{folder} holds something other than files and folders",
            &[("folder", &into.display().to_string())],
        ))
    })?;
    let stamp_text = serde_json::to_vec(&Stamp {
        index: sdk.sha256.clone(),
        asset: name.to_owned(),
        files,
    })
    .expect("a stamp serializes");
    write_whole(&stamp, &stamp_text)?;
    fetched.done = start.saturating_add(size);
    Ok(())
}

/// The build id the game release `version` was made from, as its copy of its SDK's index names
/// it (signed with the release): kept once read, so it is known offline after that. A Nightly or
/// Dev index must be signed for the exact build version, and its full commit must match that
/// version's commit prefix. A stable release's index is signed for its own tag.
pub(super) fn build_of(version: &str, base: &Base, keys: &[PublicKey]) -> Result<String, Refusal> {
    build_of_in(&root().join("versions"), version, base, keys)
}

/// The build id [`build_of`] kept for game version `version`, without reaching the network.
pub(super) fn kept_build_of(version: &str) -> Option<String> {
    let text = plain_version(version)
        .then(|| fs::read_to_string(root().join("versions").join(version)).ok())??;
    super::is_revision(text.trim()).then(|| text.trim().to_owned())
}

/// Every game version [`build_of`] kept a build id for, with that id.
pub(super) fn kept_builds() -> Vec<(String, String)> {
    let Ok(entries) = fs::read_dir(root().join("versions")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let version = entry.file_name().into_string().ok()?;
            let commit = kept_build_of(&version)?;
            Some((version, commit))
        })
        .collect()
}

/// Whether `version` can name a kept file: letters, digits, `.`, `-` and `_` only.
fn plain_version(version: &str) -> bool {
    !version.is_empty()
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

fn build_of_in(
    kept: &Path,
    version: &str,
    base: &Base,
    keys: &[PublicKey],
) -> Result<String, Refusal> {
    let refused = |why: String| Refusal::Refused {
        build: version.to_owned(),
        why,
    };
    if !plain_version(version) {
        return Err(refused(trf(
            "{version} is not a game version",
            &[("version", version)],
        )));
    }
    let file = kept.join(version);
    match fs::read_to_string(&file) {
        Ok(text) if super::is_revision(text.trim()) => return Ok(text.trim().to_owned()),
        Err(error)
            if !matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::InvalidData
            ) =>
        {
            return Err(Refusal::Cache(format!("{}: {error}", file.display())));
        }
        _ => {}
    }
    // Only the full build version, never the release title, identifies a Nightly or Dev index.
    let channel = sunrise_launcher_release_verify::Channel::of(version)
        .filter(|channel| sunrise_launcher_release_verify::is_build(*channel, version));
    // The Nightly being served is published as `nightly` and renamed `nightly-<date>` when the
    // next one goes out: the current tag is tried first. Its v2 signature binds the exact
    // version, so whichever tag serves it, only that version's index is accepted. A Dev build is
    // served under its version alone.
    let tags: Vec<String> = match (
        channel,
        sunrise_launcher_release_verify::nightly_date(version),
    ) {
        (Some(sunrise_launcher_release_verify::Channel::Nightly), Some(date)) => vec![
            sunrise_launcher_release_verify::NIGHTLY.to_owned(),
            format!("nightly-{date}"),
        ],
        _ => vec![version.to_owned()],
    };
    let mut bytes = None;
    let mut last = Refusal::NoSdk {
        build: version.to_owned(),
    };
    for tag in &tags {
        match signed_game_index(base, tag, version, channel, keys) {
            Ok(found) => {
                bytes = Some(found);
                break;
            }
            // Neither being offline nor local I/O failure can be resolved by another tag.
            Err(refusal @ (Refusal::Offline { .. } | Refusal::Cache(_))) => return Err(refusal),
            Err(refusal) => last = refusal,
        }
    }
    let Some(bytes) = bytes else {
        return Err(last);
    };
    let commit = game_commit(version, channel, &bytes)?;
    write_whole(&file, commit.as_bytes())?;
    Ok(commit)
}

fn game_commit(
    version: &str,
    channel: Option<sunrise_launcher_release_verify::Channel>,
    bytes: &[u8],
) -> Result<String, Refusal> {
    let refused = |why| Refusal::Refused {
        build: version.to_owned(),
        why,
    };
    let build = Index::build_of(bytes).map_err(|refusal| match refusal {
        sdk::Refusal::Schema(schema) => Refusal::UpdateLauncher {
            build: version.to_owned(),
            what: trf("SDK schema {schema}", &[("schema", &schema.to_string())]),
        },
        other => refused(other.to_string()),
    })?;
    let commit = build.commit;
    let named = version.rsplit('-').next().unwrap_or_default();
    if !super::is_revision(&commit) || (channel.is_some() && !commit.starts_with(named)) {
        return Err(refused(trf(
            "its index names build {commit}, not {version}'s",
            &[("commit", &commit), ("version", version)],
        )));
    }
    Ok(commit)
}

/// The game release `tag`'s copy of its SDK index, checked against the release's signed sums:
/// for a build of `channel` (a Nightly or a Dev build), signed for exactly `version` on that
/// channel (`SHA256SUMS.v2.sig`); for a release, for `tag`.
fn signed_game_index(
    base: &Base,
    tag: &str,
    version: &str,
    channel: Option<sunrise_launcher_release_verify::Channel>,
    keys: &[PublicKey],
) -> Result<Vec<u8>, Refusal> {
    let read = |name: &str| base.release_file(tag, version, name);
    let bytes = read(INDEX)?.ok_or_else(|| Refusal::NoSdk {
        build: version.to_owned(),
    })?;
    let sums = read(SUMS)?;
    let unsigned = |why: String| Refusal::Unsigned {
        build: version.to_owned(),
        why,
    };
    let signed = if let Some(channel) = channel {
        let signature = read(SIGNATURE_V2)?.map(|b| String::from_utf8_lossy(&b).into_owned());
        let digest = |version: &str| {
            sunrise_launcher_release_verify::build_digest(
                channel,
                version,
                INDEX,
                sums.as_deref(),
                signature.as_deref(),
                keys,
            )
        };
        // A dated tag republished with another build of the same day holds that build's index,
        // validly signed for it: the build asked for is gone, which is no safety failure. The
        // index names that build only as a guess to check; the signature decides.
        digest(version).map_err(|why| {
            let named = version.rsplit('-').next().unwrap_or_default();
            let same_day = Index::build_of(&bytes)
                .ok()
                .map(|build| build.commit)
                .filter(|commit| super::is_revision(commit) && !commit.starts_with(named))
                .map(|commit| {
                    let day = &version[..version.len() - named.len()];
                    format!("{day}{}", &commit[..named.len()])
                });
            match same_day {
                Some(same_day)
                    if digest(&same_day).is_ok_and(|signed| {
                        sunrise_launcher_release_verify::check(INDEX, &signed, &sha256(&bytes))
                            .is_ok()
                    }) =>
                {
                    Refusal::Superseded {
                        build: version.to_owned(),
                    }
                }
                _ => unsigned(why),
            }
        })
    } else {
        let signature = read(SIGNATURE)?.map(|b| String::from_utf8_lossy(&b).into_owned());
        sunrise_launcher_release_verify::signed_digest(
            tag,
            INDEX,
            sums.as_deref(),
            signature.as_deref(),
            keys,
        )
        .map_err(unsigned)
    }?;
    sunrise_launcher_release_verify::check(INDEX, &signed, &sha256(&bytes)).map_err(unsigned)?;
    Ok(bytes)
}

#[cfg(test)]
#[path = "sdk_cache_tests.rs"]
pub(crate) mod tests;
