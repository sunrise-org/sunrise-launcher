//! `sunrise-mod.toml`, a mod's manifest, and the `.smod` file around it.
//!
//! Native mixin schema adapted from horizon-mixins (Copyright (c) 2026 levzzz),
//! MIT licensed; see `HORIZON_MIXINS_LICENSE` and `THIRD_PARTY_NOTICES.md`.
mod jar;
pub mod loops;
pub mod mixin;
mod runtime_files;
pub use jar::{
    JarConflict, JarInputsError, JarRef, JarRequirement, MissingJar, collect_jar_requirements,
    missing_jars,
};
pub use runtime_files::{
    MAX_RUNTIME_FILE, MAX_RUNTIME_FILES, MAX_RUNTIME_TOTAL, RUNTIME_DIR, RuntimeData, RuntimeTable,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};
use std::{
    fmt,
    io::Read,
    path::{Path, PathBuf},
};

/// Checks a SHA-512 written as 128 lowercase hex digits, the form `[[jar]]` uses too.
pub(crate) fn sha512_hex(value: &str) -> Result<(), &'static str> {
    if value.len() == 128
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(())
    } else {
        Err("must be 128 lowercase hex digits")
    }
}

/// The lowercase SHA-512 of `bytes`.
pub fn sha512_of(bytes: &[u8]) -> String {
    Sha512::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Ids no mod or pack may take: the mod site's own pages under `/mods/` (a project's page is
/// `/mods/<id>`), and the names of the pages below an id.
pub const RESERVED_IDS: [&str; 7] = [
    "account",
    "edit",
    "index",
    "mine",
    "policy",
    "registry-page",
    "submit",
];

/// A native mod's uses table (`mod_surface::uses::USES_FILE` in the game repository).
pub const USES_FILE: &str = "uses.json";
/// The largest uses table ([`USES_FILE`]) a package may carry: what the surface crate reads.
pub const MAX_USES: u64 = 4 << 20;
/// The content manifest API this build loads.
pub const API: &str = "0.2";

fn api() -> String {
    API.to_owned()
}
/// Namespaces no mod may claim: vanilla's, the convention tags' (mods add
/// to those, docs/modding/src/how-to/content-mods.md) and the game's own.
const RESERVED_NAMESPACES: [&str; 3] = ["minecraft", "c", "sunrise"];
/// A mod's saved settings, shared by native mods and modpack overrides.
pub const STORAGE_DIR: &str = ".sunrise-mod-data";
/// The largest manifest this build reads.
/// Maximum manifest bytes accepted by package readers and developer tools.
pub const MAX_MANIFEST: u64 = 32 << 20;

/// The most source files a package carries (every [`source_file`] of a native package): as many as
/// its content files, so a port laid out one file per class fits. The packers and the readers that
/// inflate source (`index::native::Bundle::read`) check it from the archive's directory before
/// inflating any; [`Package::read_zip`] never reads a source file.
pub const MAX_SOURCE_FILES: usize = crate::files::MAX_FILES;
/// The most bytes those source files take, unpacked.
pub const MAX_SOURCE_BYTES: u64 = 64 << 20;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// `[a-z0-9_]`, 1 to 64 characters: the mod's folder, log and settings key.
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub authors: Vec<String>,
    /// The Mods screen's `License:` line (a name such as `MIT`); empty shows it as not given.
    #[serde(default)]
    pub license: String,
    /// Where the license text is: the screen makes its name a link.
    #[serde(default)]
    pub license_url: Option<String>,
    /// The Mods screen's `Credits:` line.
    #[serde(default)]
    pub credits: String,
    /// The mod's home page, the Mods screen's Homepage button.
    #[serde(default)]
    pub homepage: Option<String>,
    /// Where to report problems, the Mods screen's Issues button.
    #[serde(default)]
    pub issues: Option<String>,
    /// The content API line; the tool writes it into the packed package. Absent, it is [`API`].
    #[serde(default = "api")]
    pub api: String,
    #[serde(default)]
    pub side: Side,
    /// Absent, this carries the same label as a manifest written by the mod tool.
    #[serde(default = "Capability::derived")]
    pub capabilities: Vec<Capability>,
    /// Namespaces besides `id` that the mod's `assets/` and `data/` use, for
    /// a content mod whose files sit under other names than its id (Towns
    /// and Towers ships `kaisyn` and `towns_and_towers`). No two content mods
    /// that are on may share one (`content::clash`).
    #[serde(default)]
    pub namespaces: Vec<String>,
    /// Native runtime mod that serves code-side parts and registrations for
    /// this content package's namespaces; refused on `[native]` packages.
    #[serde(default)]
    pub served_by: Option<String>,
    /// Other native mods whose hooks must be available before this one.
    #[serde(default)]
    pub dependencies: std::collections::BTreeMap<String, Dependency>,
    /// Mods this one breaks with: the game refuses to start while one of them is loaded at a
    /// version the rule matches (NeoForge's `DependencyType.INCOMPATIBLE`).
    #[serde(default)]
    pub incompatible: std::collections::BTreeMap<String, Rule>,
    /// Mods this one works badly with: they load, with a warning (`DependencyType.DISCOURAGED`).
    #[serde(default)]
    pub discouraged: std::collections::BTreeMap<String, Rule>,
    /// Mods whose hooks run after this one's when they are loaded, without depending on them
    /// (`Ordering.BEFORE`).
    #[serde(default)]
    pub load_before: Vec<String>,
    /// Original Java jars, layered in list order below this mod's own files.
    #[serde(default)]
    pub jar: Vec<JarRef>,
    /// Exact build pins and support modules for a code mod.
    pub native: Option<Native>,
    /// A native mod's `runtime/` files, each with its SHA-512. Not pack data.
    #[serde(default)]
    pub runtime: Option<RuntimeTable>,
    /// Game internals used by this mod, fingerprinted against its pinned build.
    #[serde(default)]
    pub internals: Vec<Internal>,
    /// Free-form values other mods read through `PackageInfo::property`.
    #[serde(default)]
    pub properties: std::collections::BTreeMap<String, Property>,
}

/// The most `[properties]` keys a manifest may declare.
pub const MAX_PROPERTIES: usize = 64;
/// The most `[properties]` bytes: keys and strings by length, 8 per number, 1 per boolean.
pub const MAX_PROPERTY_BYTES: usize = 8 << 10;

/// One `[properties]` value: a table or array is refused.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Property {
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(String),
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Dependency {
    Version(semver::VersionReq),
    Detailed(DependencyDetails),
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DependencyDetails {
    pub version: semver::VersionReq,
    #[serde(default)]
    pub optional: bool,
    /// The installation on which this dependency declaration applies.
    #[serde(default = "both_sides")]
    pub side: Side,
    /// A mod-provided explanation, at most 512 bytes.
    #[serde(default)]
    pub reason: Option<String>,
    /// An HTTPS link with more information.
    #[serde(default)]
    pub url: Option<String>,
    /// Dispatch ordering only; `none` does not remove a code link requirement.
    #[serde(default)]
    pub order: DependencyOrder,
}

impl Dependency {
    /// The dependency's typed declaration, with bare-version defaults filled in.
    pub fn details(&self) -> DependencyDetails {
        match self {
            Self::Version(version) => DependencyDetails {
                version: version.clone(),
                optional: false,
                side: Side::Both,
                reason: None,
                url: None,
                order: DependencyOrder::After,
            },
            Self::Detailed(details) => details.clone(),
        }
    }
}

/// An `[incompatible]` or `[discouraged]` entry: the versions of the other mod it matches
/// (`"<2"`), or a table that also gives the player a reason and a link.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Rule {
    Version(semver::VersionReq),
    Detailed(RuleDetails),
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RuleDetails {
    /// Every version when left out.
    #[serde(default = "any_version")]
    pub version: semver::VersionReq,
    /// Why, shown when the rule fails: at most 512 bytes.
    #[serde(default)]
    pub reason: Option<String>,
    /// Where to read more, shown with the reason: an `https://` link.
    #[serde(default)]
    pub url: Option<String>,
    /// Where the rule holds: both sides, or only the client or the dedicated server.
    #[serde(default = "both_sides")]
    pub side: Side,
}

fn both_sides() -> Side {
    Side::Both
}

fn any_version() -> semver::VersionReq {
    semver::VersionReq::STAR
}

impl Rule {
    /// The versions, reason, link and side, the bare form's as defaults.
    pub fn details(&self) -> RuleDetails {
        match self {
            Self::Version(version) => RuleDetails {
                version: version.clone(),
                reason: None,
                url: None,
                side: Side::Both,
            },
            Self::Detailed(details) => details.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Native {
    pub game: String,
    pub sunrise: String,
    /// When omitted, the mod uses the horizon-mixins revision in its pinned SDK's `sdk.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mixins: Option<String>,
    /// Whether this native mod opts in to serving content packages that name it in `served_by`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub serves_content: bool,
    /// Source-level edits of game functions, native only: the one form that compiles game code.
    #[serde(default)]
    pub mixin: Vec<mixin::Mixin>,
    #[serde(default)]
    pub module: Vec<Module>,
    /// Accessor and invoker bridges, forwarded as horizon-mixins' `[[bridge]]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bridge: Vec<mixin::Forwarded>,
    /// State fields added to game types, forwarded as horizon-mixins' `[[state]]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub state: Vec<mixin::Forwarded>,
    /// Slots of published game loops, native only (`mod_api::loops`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub r#loop: Vec<loops::Loop>,
}

/// One declared game internal, as listed in the pinned game's `internals.json`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Internal {
    pub path: String,
    /// The catalog's member kind, required when several kinds share the path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Optional pinning: the catalog fingerprint `internals add` writes. Absent, the build takes
    /// it from the pinned game's catalog, which `[native] sunrise` fixes exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

/// A support module merged into a game crate, forwarded as horizon-mixins'
/// `[[module]]`: self-contained Rust items, emitted into `file` as
/// `mod name`, for helpers that need more than one function.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Module {
    pub id: String,
    /// The game source file that receives the module, as the pin indexes it.
    pub file: String,
    /// The module's Rust name under the destination file's top level.
    pub name: String,
    /// The payload's file in this package, under `src/`.
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
}

/// Which installations must carry a package: the game's mod-vocab `Side`, in the same serde form.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Side {
    /// Local content, not compared with the server's mods when joining.
    #[default]
    Client,
    /// Both installations; the server tells clients which version it expects.
    Both,
    /// As a package's side only the server needs it; as a dependency's, the dedicated server.
    Server,
}

/// A dependency's dispatch ordering: the game's mod-vocab `DependencyOrder`, in the same serde form.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum DependencyOrder {
    /// The dependency dispatches before its owner (the default).
    #[default]
    After,
    /// The owner dispatches before the dependency.
    Before,
    /// Do not impose a dispatch edge.
    None,
}

/// What a content mod adds; the player sees it before turning the mod on.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    /// New blocks, items and mob effects from the mod's `data/` (API 0.2).
    Registries,
}
impl Capability {
    fn derived() -> Vec<Self> {
        vec![Self::Registries]
    }

    /// What the player reads before turning the mod on: "May <this>".
    pub fn describe(self) -> &'static str {
        match self {
            Capability::Registries => {
                "add blocks, items, mob effects and entity types to your worlds"
            }
        }
    }
}
impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Capability::Registries => f.write_str("registries"),
        }
    }
}

/// Whether a source pin is an exact lowercase Git object id.
pub fn revision(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The grammar shared by manifest IDs and source listings, including portable file names.
pub fn valid_mod_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && !reserved_mod_id(id)
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn reserved_mod_id(id: &str) -> bool {
    matches!(id, "con" | "prn" | "aux" | "nul")
        || (id.len() == 4
            && (id.starts_with("com") || id.starts_with("lpt"))
            && matches!(id.as_bytes()[3], b'1'..=b'9'))
}

fn rust_ident(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !name.as_bytes()[0].is_ascii_digit()
}

impl Internal {
    fn check_all(internals: &[Self]) -> Result<(), String> {
        let mut paths = std::collections::HashSet::new();
        for internal in internals {
            if internal.path.is_empty()
                || internal.hash.as_ref().is_some_and(String::is_empty)
                || internal.kind.as_ref().is_some_and(|kind| kind.is_empty())
                || !paths.insert((&internal.path, &internal.kind))
            {
                return Err(format!(
                    "{MANIFEST}: empty internal path, kind or hash, or duplicate internal path and kind"
                ));
            }
        }
        Ok(())
    }
}

impl Manifest {
    /// Dependencies that must be present for a client installation.
    pub fn required_client_dependencies(&self) -> impl Iterator<Item = (&String, &Dependency)> {
        self.dependencies.iter().filter(|(_, need)| match need {
            Dependency::Version(_) => true,
            Dependency::Detailed(need) => !need.optional && need.side != Side::Server,
        })
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let m: Manifest = toml::from_str(text).map_err(|e| format!("{MANIFEST}: {e}"))?;

        Internal::check_all(&m.internals)?;
        if !m.internals.is_empty() && m.native.is_none() {
            return Err(format!("{MANIFEST}: internals need a [native] mod"));
        }
        if m.runtime.is_some() && m.native.is_none() {
            return Err(format!(
                "{MANIFEST}: a content mod has no code, so no [runtime]"
            ));
        }
        if reserved_mod_id(&m.id) {
            return Err(format!(
                "{MANIFEST}: id {:?} is a reserved Windows device name",
                m.id
            ));
        }
        if !valid_mod_id(&m.id) {
            return Err(format!(
                "{MANIFEST}: id {:?} is not [a-z0-9_]{{1,64}}",
                m.id
            ));
        }
        if let Some(host) = &m.served_by {
            if m.native.is_some() {
                return Err(format!(
                    "{MANIFEST}: a [native] mod cannot declare served_by"
                ));
            }
            if RESERVED_NAMESPACES.contains(&host.as_str()) {
                return Err(format!(
                    "{MANIFEST}: served_by {host:?} is a reserved namespace, not a mod id"
                ));
            }
            if RESERVED_IDS.contains(&host.as_str()) {
                return Err(format!(
                    "{MANIFEST}: served_by {host:?} is a page of the mod site, not a mod id"
                ));
            }
            if reserved_mod_id(host) {
                return Err(format!(
                    "{MANIFEST}: served_by {host:?} is a reserved Windows device name"
                ));
            }
            if !valid_mod_id(host) {
                return Err(format!(
                    "{MANIFEST}: served_by {host:?} is not [a-z0-9_]{{1,64}}"
                ));
            }
            if host == &m.id {
                return Err(format!(
                    "{MANIFEST}: served_by cannot name this package's own id {host:?}"
                ));
            }
        }
        let rules = m.incompatible.iter().chain(&m.discouraged);
        for id in m
            .dependencies
            .keys()
            .chain(rules.clone().map(|(id, _)| id))
            .chain(&m.load_before)
        {
            if !valid_mod_id(id) {
                return Err(format!("{MANIFEST}: invalid dependency id {id:?}"));
            }
        }
        let rule_notes = rules.map(|(id, rule)| {
            let details = rule.details();
            (id, details.reason, details.url)
        });
        let dependency_notes = m.dependencies.iter().map(|(id, dependency)| {
            let details = dependency.details();
            (id, details.reason, details.url)
        });
        for (id, reason, url) in rule_notes.chain(dependency_notes) {
            if reason.as_ref().is_some_and(|reason| reason.len() > 512) {
                return Err(format!("{MANIFEST}: rule for {id}: reason over 512 bytes"));
            }
            if url.as_ref().is_some_and(|url| !url.starts_with("https://")) {
                return Err(format!("{MANIFEST}: rule for {id}: url is not https://"));
            }
        }
        mixin::Mixin::check_all(
            m.native
                .as_ref()
                .map(|n| n.mixin.as_slice())
                .unwrap_or_default(),
            &m.id,
        )
        .map_err(|e| format!("{MANIFEST}: {e}"))?;
        loops::Loop::check_all(
            m.native
                .as_ref()
                .map(|n| n.r#loop.as_slice())
                .unwrap_or_default(),
            &m.id,
        )
        .map_err(|e| format!("{MANIFEST}: {e}"))?;
        if let Some(entry) = m
            .native
            .as_ref()
            .map(|n| n.mixin.as_slice())
            .unwrap_or_default()
            .iter()
            .find(|entry| {
                m.dependencies.keys().any(|id| {
                    entry.target.split("::").next() == Some(id.replace('-', "_").as_str())
                })
            })
        {
            return Err(format!(
                "{MANIFEST}: {}",
                mixin::MixinError::Invalid {
                    id: format!("{}:{}", m.id, entry.id),
                    field: "target",
                    fix: "a mixin targets a game function",
                }
            ));
        }
        if let Some(native) = &m.native {
            // Native source edits share one id space.
            let mut ids = std::collections::HashSet::new();
            for id in native
                .mixin
                .iter()
                .map(|entry| &entry.id)
                .chain(native.bridge.iter().chain(&native.state).map(|row| &row.id))
                .chain(native.r#loop.iter().map(|row| &row.id))
            {
                if id.trim().is_empty() || !ids.insert(id) {
                    return Err(format!(
                        "{MANIFEST}: empty or duplicate mixin, bridge, state or loop id"
                    ));
                }
            }
            // A bridge's id becomes part of its accessor's name: `-` reads as `_`, and case is
            // dropped, so the id is that alphabet and no two ids may give one name.
            let mut accessors = std::collections::HashMap::new();
            for bridge in &native.bridge {
                let id = &bridge.id;
                if !id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                {
                    return Err(format!(
                        "{MANIFEST}: E_MIXIN_INVALID: {}:{id}: invalid id; a bridge id is ASCII letters, digits, `_` and `-`",
                        m.id
                    ));
                }
                if let Some(other) = accessors.insert(id.replace('-', "_").to_ascii_lowercase(), id)
                {
                    return Err(format!(
                        "{MANIFEST}: E_MIXIN_INVALID: {}:{id}: invalid id; it gives the same accessor name as bridge `{other}`",
                        m.id
                    ));
                }
            }
            for module in &native.module {
                if module.id.trim().is_empty() || !ids.insert(&module.id) {
                    return Err(format!("{MANIFEST}: empty or duplicate module id"));
                }
                if !plain_path(&module.file) {
                    return Err(format!("{MANIFEST}: module file must be a plain path"));
                }
                if !rust_ident(&module.name) {
                    return Err(format!("{MANIFEST}: module name must be an identifier"));
                }
                if !plain_path(&module.source) || !module.source.starts_with("src/") {
                    return Err(format!(
                        "{MANIFEST}: module source must be a file under src/"
                    ));
                }
                if let Some(visibility) = &module.visibility
                    && !matches!(visibility.as_str(), "pub" | "pub(crate)")
                {
                    return Err(format!(
                        "{MANIFEST}: module visibility must be pub or pub(crate)"
                    ));
                }
            }
            if m.version.is_empty()
                || native.game.is_empty()
                || !revision(&native.sunrise)
                || native.mixins.as_deref().is_some_and(|pin| !revision(pin))
            {
                return Err(format!(
                    "{MANIFEST}: native requires game and exact revisions"
                ));
            }
        }
        if m.api != API {
            return Err(format!(
                "unsupported content API {}; this Sunrise reads {API}",
                m.api
            ));
        }
        let property_bytes: usize = m
            .properties
            .iter()
            .map(|(key, value)| {
                key.len()
                    + match value {
                        Property::Boolean(_) => 1,
                        Property::Integer(_) | Property::Float(_) => 8,
                        Property::String(text) => text.len(),
                    }
            })
            .sum();
        if m.properties.len() > MAX_PROPERTIES || property_bytes > MAX_PROPERTY_BYTES {
            return Err(format!(
                "{MANIFEST}: [properties] holds at most {MAX_PROPERTIES} keys and {MAX_PROPERTY_BYTES} \
                 bytes; this one has {} keys and {property_bytes} bytes",
                m.properties.len()
            ));
        }
        if m.capabilities != [Capability::Registries] {
            return Err(format!(
                "{MANIFEST}: a content mod has capabilities = [\"registries\"]"
            ));
        }
        for (field, url) in [
            ("license_url", &m.license_url),
            ("homepage", &m.homepage),
            ("issues", &m.issues),
        ] {
            if let Some(url) = url
                && (url.len() > 2048
                    || !(url.starts_with("https://") || url.starts_with("http://")))
            {
                return Err(format!(
                    "{MANIFEST}: {field} is an http(s) link of at most 2048 characters"
                ));
            }
        }
        for (i, ns) in m.namespaces.iter().enumerate() {
            // Vanilla's namespace characters, at most as long as an id.
            let chars_ok = (1..=64).contains(&ns.len())
                && ns
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b));
            if !chars_ok {
                return Err(format!(
                    "{MANIFEST}: namespace {ns:?} is not [a-z0-9_.-]{{1,64}}"
                ));
            }
            if RESERVED_NAMESPACES.contains(&ns.as_str()) {
                return Err(format!(
                    "{MANIFEST}: namespace {ns} is not a mod's to claim (only to add to its tags)"
                ));
            }
            if *ns == m.id || m.namespaces[..i].contains(ns) {
                return Err(format!("{MANIFEST}: namespace {ns} is listed twice"));
            }
        }
        Ok(m)
    }
    /// The namespaces the mod's content owns: its id, then `namespaces`.
    pub fn owned_namespaces(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.id.as_str()).chain(self.namespaces.iter().map(String::as_str))
    }
    /// The refusal of a `side = "server"` mod whose code declares registry content (one of
    /// `declared`, each its registry kind and id, in a namespace it owns), naming the mod, the
    /// kind and the id: every client must have that content.
    pub fn server_only_declaration<'a>(
        &self,
        mut declared: impl Iterator<Item = (&'a str, &'a str)>,
    ) -> Option<String> {
        if self.side != Side::Server {
            return None;
        }
        let (kind, id) = declared.find(|(_, id)| {
            let ns = id.split_once(':').map_or("", |(ns, _)| ns);
            self.owned_namespaces().any(|owned| owned == ns)
        })?;
        Some(format!(
            "{}: a side = \"server\" mod declares the {kind} {id}, which every client must have; \
             declare side = \"both\"",
            self.id
        ))
    }
}

pub use crate::package::{MANIFEST, NOT_PACKAGED, binary_kind, source_file};

fn plain_path(name: &str) -> bool {
    !name.starts_with('/')
        && !name.contains(['\\', ':'])
        && !name.split('/').any(|part| {
            // A Windows extraction drops a trailing dot or space, so `a.` would land as `a`.
            part.is_empty() || part == "." || part == ".." || part.ends_with(['.', ' '])
        })
}

/// A mod as found on disk.
pub struct Package {
    pub manifest: Manifest,
    pub icon: Option<crate::icon::Icon>,
    pub layouts: crate::layouts::Layouts,
}
impl Package {
    /// The kind of this validated package, independent of its install folder.
    pub fn is_native(&self) -> bool {
        self.manifest.native.is_some()
    }

    /// Read a flat `.smod` or project folder, without executing native code.
    pub fn read(path: &Path) -> Result<Self, String> {
        let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        let mut icon = None;
        let mut has_cargo = false;
        let mut has_source = false;
        let toml = if metadata.is_dir() {
            if std::fs::symlink_metadata(path.join("Cargo.toml")).is_ok() {
                check_folder_file(path, "Cargo.toml")?;
                has_cargo = true;
            }
            if let Ok(source) = std::fs::symlink_metadata(path.join("src")) {
                if !source.is_dir() {
                    return Err("src: unsupported package path".into());
                }
                has_source = true;
            }
            let icon_path = path.join("icon.png");
            match std::fs::symlink_metadata(&icon_path) {
                Ok(metadata) => {
                    check_folder_file(path, "icon.png")?;
                    if metadata.len() > crate::icon::MAX_ICON as u64 {
                        return Err("icon.png exceeds 256 KiB".into());
                    }
                    let mut bytes = Vec::new();
                    std::fs::File::open(&icon_path)
                        .map_err(|e| format!("icon.png: {e}"))?
                        .take(crate::icon::MAX_ICON as u64 + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|e| format!("icon.png: {e}"))?;
                    icon = Some(crate::icon::Icon::parse(bytes)?);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("icon.png: {e}")),
            }
            check_folder_file(path, MANIFEST)?;
            for (name, _) in crate::files::folder_files(path)? {
                check_folder_file(path, &name)?;
            }
            let mut text = String::new();
            std::fs::File::open(path.join(MANIFEST))
                .map_err(|e| format!("{MANIFEST}: {e}"))?
                .take(MAX_MANIFEST + 1)
                .read_to_string(&mut text)
                .map_err(|e| format!("{MANIFEST}: {e}"))?;
            if text.len() as u64 > MAX_MANIFEST {
                return Err(format!("{MANIFEST}: over {} MiB", MAX_MANIFEST >> 20));
            }
            text
        } else {
            if !metadata.is_file() {
                return Err(format!(
                    "{}: unsupported file type in a content mod",
                    path.display()
                ));
            }
            #[cfg(target_arch = "wasm32")]
            return Err("archive I/O is unavailable on wasm32".into());
            #[cfg(not(target_arch = "wasm32"))]
            {
                let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
                let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
                return Self::read_zip(&mut zip);
            }
        };
        let manifest = Manifest::parse(&toml)?;
        if has_cargo != manifest.native.is_some() || has_source != has_cargo {
            return Err("inconsistent code declaration ([native]) and source".into());
        }
        let mut package = Package {
            manifest,
            icon,
            layouts: Default::default(),
        };
        package.check_runtime_layout(&crate::files::runtime_folder_files(path)?)?;
        match std::fs::File::open(path.join(crate::layouts::FILE)) {
            Ok(file) => {
                check_folder_file(path, crate::layouts::FILE)?;
                package.layouts = crate::layouts::Layouts::read(file)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("layouts.json: {e}")),
        }
        Ok(package)
    }

    /// Read an archive: its manifest, icon, layouts, content files and uses table, and the
    /// `runtime/` files it lists. Any other entry, a native package's source among them, is
    /// skipped without being read; every entry's path and file type are still checked.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read_zip<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>) -> Result<Self, String> {
        let mut icon = None;
        let mut layouts = None;
        let mut uses = false;
        let mut runtime = Vec::new();
        let mut resources = Vec::new();
        let mut names = std::collections::HashSet::new();
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
            let name = entry.name().map_err(|e| e.to_string())?.into_owned();
            let directory = entry.is_dir();
            let path = if directory {
                name.trim_end_matches('/')
            } else {
                &name
            };
            if !plain_path(path) || !names.insert(name.clone()) {
                return Err(format!("invalid or duplicate package path: {name}"));
            }
            let mode = entry.unix_mode().unwrap_or_default();
            let kind = mode & 0o170000;
            if !matches!(kind, 0 | 0o100000) && !(directory && kind == 0o040000) {
                return Err(format!("{name}: unsupported file type"));
            }
            if directory {
                continue;
            }
            if name == crate::layouts::FILE {
                layouts = Some(crate::layouts::Layouts::read(&mut entry)?);
            } else if let Some(below) = name
                .strip_prefix(RUNTIME_DIR)
                .and_then(|n| n.strip_prefix('/'))
            {
                // Any file type, but a file: its bytes are checked against `[runtime]` below.
                runtime.push((below.to_owned(), entry.size()));
            } else if name == USES_FILE {
                if entry.size() > MAX_USES {
                    return Err(format!("{USES_FILE} exceeds {} MiB", MAX_USES >> 20));
                }
                uses = true;
            } else if name == "icon.png" {
                check_file(&name, mode, || Ok(&mut entry)).map_err(|e| e.to_string())?;
                if entry.size() > crate::icon::MAX_ICON as u64 {
                    return Err("icon.png exceeds 256 KiB".into());
                }
                let mut bytes = Vec::new();
                drop(entry);
                zip.by_index(i)
                    .map_err(|e| e.to_string())?
                    .take(crate::icon::MAX_ICON as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| format!("icon.png: {e}"))?;
                icon = Some(crate::icon::Icon::parse(bytes)?);
            } else if name == MANIFEST {
                check_file(&name, mode, || Ok(&mut entry)).map_err(|e| e.to_string())?;
            } else if name.starts_with("assets/") || name.starts_with("data/") {
                check_file(&name, mode, || Ok(&mut entry)).map_err(|e| e.to_string())?;
                resources.push((name, entry.size()));
            }
        }
        let mut toml = String::new();
        zip.by_name(MANIFEST)
            .map_err(|e| format!("{MANIFEST}: {e}"))?
            .take(MAX_MANIFEST + 1)
            .read_to_string(&mut toml)
            .map_err(|e| format!("{MANIFEST}: {e}"))?;
        if toml.len() as u64 > MAX_MANIFEST {
            return Err(format!("{MANIFEST}: over {} MiB", MAX_MANIFEST >> 20));
        }
        let package = Package {
            manifest: Manifest::parse(&toml)?,
            icon,
            layouts: layouts.unwrap_or_default(),
        };
        if uses && !package.is_native() {
            return Err(format!("a content mod has no code, so no {USES_FILE}"));
        }
        package.check_runtime_layout(&runtime)?;
        if package.is_native() {
            let mut resource_bytes = 0u64;
            for (name, size) in &resources {
                crate::files::check_path(name)?;
                crate::files::check_namespace(name, &package.manifest)?;
                if *size > crate::files::MAX_FILE {
                    return Err(format!("{name} exceeds the content file size limit"));
                }
                resource_bytes = resource_bytes
                    .checked_add(*size)
                    .ok_or("content too large")?;
            }
            if resources.len() > crate::files::MAX_FILES || resource_bytes > crate::files::MAX_TOTAL
            {
                return Err("content exceeds its entry or size limit".into());
            }
        }
        Ok(package)
    }

    /// Refuse an archive whose source entries are too many or too large: they are counted, and
    /// their declared sizes summed from the archive's directory, before any is inflated, as the
    /// packers bound them.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn check_source_bounds<R: Read + std::io::Seek>(
        zip: &mut zip::ZipArchive<R>,
    ) -> Result<(), String> {
        let (mut count, mut bytes) = (0usize, 0u64);
        for i in 0..zip.len() {
            let entry = zip.by_index_raw(i).map_err(|e| e.to_string())?;
            if entry.is_dir() || !entry.name().is_ok_and(|name| source_file(&name)) {
                continue;
            }
            count += 1;
            bytes = bytes
                .checked_add(entry.size())
                .ok_or("native package too large")?;
            if count > MAX_SOURCE_FILES || bytes > MAX_SOURCE_BYTES {
                return Err("native package exceeds its entry or size limit".into());
            }
        }
        Ok(())
    }

    /// Refuse a `runtime/` folder that is not the one `[runtime]` lists: an extra, a missing or an
    /// oversized file, each named. `found` is each file below `runtime/` and its size.
    fn check_runtime_layout(&self, found: &[(String, u64)]) -> Result<(), String> {
        if !found.is_empty() && !self.is_native() {
            return Err("a content mod has no code, so no runtime/ folder".into());
        }
        RuntimeTable::check_layout(self.manifest.runtime.as_ref(), found)
    }

    /// The package's runtime files, read from `path` (a folder or a `.smod`) and each checked
    /// against its `[runtime]` SHA-512: the one place their bytes are hashed. A mismatch is
    /// refused by name. `None` for a package with no `[runtime]`.
    pub fn read_runtime(&self, path: &Path) -> Result<Option<RuntimeData>, String> {
        let Some(table) = &self.manifest.runtime else {
            return Ok(None);
        };
        let mut files = std::collections::BTreeMap::new();
        let folder = path.is_dir();
        #[cfg(not(target_arch = "wasm32"))]
        let mut zip = if folder {
            None
        } else {
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            Some(zip::ZipArchive::new(file).map_err(|e| e.to_string())?)
        };
        for (name, _) in table.files() {
            let entry = format!("{RUNTIME_DIR}/{name}");
            let mut bytes = Vec::new();
            // At most one byte past the cap, so an oversized file is refused unread.
            let read = if folder {
                std::fs::File::open(path.join(&entry))
                    .map_err(|e| format!("{entry}: {e}"))?
                    .take(MAX_RUNTIME_FILE + 1)
                    .read_to_end(&mut bytes)
            } else {
                #[cfg(target_arch = "wasm32")]
                return Err("archive I/O is unavailable on wasm32".into());
                #[cfg(not(target_arch = "wasm32"))]
                zip.as_mut()
                    .expect("an archive")
                    .by_name(&entry)
                    .map_err(|e| format!("{entry}: {e}"))?
                    .take(MAX_RUNTIME_FILE + 1)
                    .read_to_end(&mut bytes)
            };
            read.map_err(|e| format!("{entry}: {e}"))?;
            table.check(name, &bytes)?;
            files.insert(name.to_owned(), std::sync::Arc::from(bytes));
        }
        Ok(Some(RuntimeData::new(
            files,
            table.digest(),
            folder.then(|| path.to_owned()),
        )))
    }

    /// Whether the mod at `path` (a `.smod` or a folder) has a [`MANIFEST`].
    pub fn has_manifest(path: &Path) -> bool {
        if path.is_dir() {
            return path.join(MANIFEST).is_file();
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            std::fs::File::open(path)
                .ok()
                .and_then(|f| zip::ZipArchive::new(f).ok())
                .is_some_and(|mut zip| zip.by_name(MANIFEST).is_ok())
        }
        #[cfg(target_arch = "wasm32")]
        false
    }
    /// Select and validate a flat project's source and resource files deterministically. The
    /// source is what `cargo package` would put in the crate ([`crate::package::source_files`]): a
    /// file outside it (one under `.cargo/`, one `[package] exclude` or `.gitignore` names) is left
    /// out unread, and one inside it is refused when its bytes are a prebuilt binary
    /// ([`binary_kind`]) or it is not a plain file.
    pub fn directory_files(
        &self,
        root: &Path,
    ) -> Result<std::collections::BTreeMap<String, Vec<u8>>, String> {
        let mut files = std::collections::BTreeMap::new();
        for name in crate::files::content_files(root)? {
            let bytes = crate::files::read_file(root, &name)?;
            files.insert(name, bytes);
        }
        if let Ok(file) = std::fs::File::open(root.join(crate::layouts::FILE)) {
            let mut bytes = Vec::new();
            file.take((1 << 20) + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            crate::layouts::Layouts::read(bytes.as_slice())?;
            files.insert(crate::layouts::FILE.into(), bytes);
        }
        if let Some(icon) = &self.icon {
            files.insert("icon.png".to_string(), icon.bytes().to_vec());
        }
        if let Some(runtime) = self.read_runtime(root)? {
            for name in runtime.names() {
                files.insert(
                    format!("{RUNTIME_DIR}/{name}"),
                    runtime.file(name).expect("a listed file").to_vec(),
                );
            }
        }
        if self.is_native() {
            let (mut source_bytes, mut source_count) = (0u64, 0usize);
            for name in crate::package::source_files(root).map_err(|e| e.to_string())? {
                let path = root.join(&name);
                let mode = folder_mode(root, &name).map_err(|e| e.to_string())?;
                check_source_type(&name, mode).map_err(|e| e.to_string())?;
                let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
                source_bytes = source_bytes
                    .checked_add(size)
                    .ok_or("native package too large")?;
                source_count += 1;
                if source_bytes > MAX_SOURCE_BYTES || source_count > MAX_SOURCE_FILES {
                    return Err("native package exceeds its entry or size limit".into());
                }
                // Read once: the bytes checked are the bytes packed.
                let mut bytes = Vec::new();
                std::fs::File::open(&path)
                    .map_err(|e| format!("{name}: {e}"))?
                    .take(size)
                    .read_to_end(&mut bytes)
                    .map_err(|e| format!("{name}: {e}"))?;
                refuse_binary(&name, &bytes).map_err(|e| e.to_string())?;
                files.insert(name, bytes);
            }
        }
        files.insert(
            MANIFEST.to_owned(),
            std::fs::read(root.join(MANIFEST)).map_err(|e| e.to_string())?,
        );
        Ok(files)
    }

    /// Pack a flat project, selecting its source and resource files deterministically.
    /// Carries a valid [`USES_FILE`] from `root` for native mods when present.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn pack_directory(root: &Path) -> Result<Vec<u8>, String> {
        let package = Self::read(root)?;
        let uses = if package.is_native() {
            read_folder_uses(root)?
        } else {
            None
        };
        Self::pack_directory_with(root, uses.as_deref())
    }

    /// The bytes of entry `name` in the `.smod` at `path`, refused past `most` bytes; `None` when
    /// it has no such entry.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn entry(path: &Path, name: &str, most: u64) -> Result<Option<Vec<u8>>, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", path.display()))?;
        let entry = match zip.by_name(name) {
            Ok(entry) => entry,
            Err(zip::result::ZipError::FileNotFound) => return Ok(None),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let mut bytes = Vec::new();
        entry
            .take(most + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("{name}: {e}"))?;
        if bytes.len() as u64 > most {
            return Err(format!("{name}: over {most} bytes"));
        }
        Ok(Some(bytes))
    }

    /// [`Package::pack_directory`], carrying `uses` as the package's [`USES_FILE`] table when given.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn pack_directory_with(root: &Path, uses: Option<&[u8]>) -> Result<Vec<u8>, String> {
        let package = Self::read(root)?;
        let mut files = package.directory_files(root)?;
        if let Some(uses) = uses {
            files.insert(USES_FILE.to_owned(), uses.to_vec());
        }
        let text = String::from_utf8(files.remove(MANIFEST).expect("selected manifest"))
            .map_err(|e| e.to_string())?;
        let files: Vec<_> = files
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        Self::write_files(&text, &files)
    }

    /// The `.smod` bytes for `manifest_text` and selected project files.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn write_files(manifest_text: &str, files: &[(&str, &[u8])]) -> Result<Vec<u8>, String> {
        use std::io::Write;
        let manifest = Manifest::parse(manifest_text)?;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        let err = |e: zip::result::ZipError| e.to_string();
        let mut put = |name: &str, bytes: &[u8]| -> Result<(), String> {
            zip.start_file(name, options).map_err(err)?;
            zip.write_all(bytes).map_err(|e| e.to_string())
        };
        put(MANIFEST, manifest_text.as_bytes())?;
        let mut lowercase = std::collections::HashMap::from([(MANIFEST.to_lowercase(), MANIFEST)]);
        let code = manifest.native.is_some();
        let (mut source_bytes, mut source_count) = (0u64, 0usize);
        for (name, bytes) in files {
            if let Some(other) = lowercase.insert(name.to_lowercase(), *name) {
                return Err(format!(
                    "{other} and {name} differ only by case, which a Windows or macOS extraction cannot keep apart"
                ));
            }
            let source = code && source_file(name) && plain_path(name);
            if *name == crate::layouts::FILE {
                crate::layouts::Layouts::read(*bytes)?;
                put(name, bytes)?;
                continue;
            }
            if let Some(below) = name.strip_prefix("runtime/") {
                let table = manifest
                    .runtime
                    .as_ref()
                    .ok_or(format!("{name}: not listed in [runtime]"))?;
                table.check(below, bytes)?;
                put(name, bytes)?;
                continue;
            }
            if *name == USES_FILE && code {
                if bytes.len() as u64 > MAX_USES {
                    return Err(format!("{USES_FILE} exceeds {} MiB", MAX_USES >> 20));
                }
                put(name, bytes)?;
                continue;
            }
            if source {
                // A source file has no name or mode rule: only its bytes may not be a binary.
                refuse_binary(name, bytes).map_err(|e| e.to_string())?;
                source_bytes += bytes.len() as u64;
                source_count += 1;
                if source_bytes > MAX_SOURCE_BYTES || source_count > MAX_SOURCE_FILES {
                    return Err("native package exceeds its entry or size limit".into());
                }
                put(name, bytes)?;
                continue;
            }
            if *name != "icon.png" {
                crate::files::check_path(name)?;
            }
            check_file(name, 0, || Ok(*bytes)).map_err(|e| e.to_string())?;
            put(name, bytes)?;
        }
        let bytes = zip.finish().map_err(err)?.into_inner();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).map_err(err)?;
        Self::read_zip(&mut archive)?;
        Ok(bytes)
    }
}

/// A content entry that cannot be safely loaded as data.
#[derive(Debug)]
pub enum ContentFileError {
    UnsupportedType(String),
    Executable(String),
    /// A native package's source file whose bytes are prebuilt object code of this kind
    /// ([`binary_kind`]), whatever its name. A content file with such a header is `Executable`.
    Binary {
        name: String,
        kind: &'static str,
    },
    Read {
        name: String,
        source: std::io::Error,
    },
}

impl std::fmt::Display for ContentFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedType(name) => {
                write!(f, "{name}: unsupported file type in a content mod")
            }
            Self::Executable(name) => {
                write!(f, "{name}: unsupported executable file in a content mod")
            }
            Self::Binary { name, kind } => write!(
                f,
                "{name}: unsupported executable file ({kind}); a package carries source, never prebuilt binaries"
            ),
            Self::Read { name, source } => write!(f, "{name}: {source}"),
        }
    }
}

impl std::error::Error for ContentFileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// The first bytes of a file, up to 8, for the content checks.
fn file_head<R: Read>(
    name: &str,
    open: impl FnOnce() -> std::io::Result<R>,
) -> Result<([u8; 8], usize), ContentFileError> {
    let error = |source| ContentFileError::Read {
        name: name.into(),
        source,
    };
    let mut file = open().map_err(error)?;
    let mut head = [0; 8];
    let mut read = 0;
    while read < head.len() {
        let size = file.read(&mut head[read..]).map_err(error)?;
        if size == 0 {
            break;
        }
        read += size;
    }
    Ok((head, read))
}

/// The refusal of a source file whose bytes are prebuilt object code ([`binary_kind`]), whatever
/// its name. Inspects the full bounded `bytes` buffer so a PE signature past the first 1 KiB is
/// still caught.
pub(crate) fn refuse_binary(name: &str, bytes: &[u8]) -> Result<(), ContentFileError> {
    match binary_kind(bytes) {
        Some(kind) => Err(ContentFileError::Binary {
            name: name.into(),
            kind,
        }),
        None => Ok(()),
    }
}

/// The boundary of a content file (an asset, a data file, the manifest, the icon). Refuse
/// type/mode before opening a file, then inspect its header even when its name has no executable
/// extension.
pub(crate) fn check_file<R: Read>(
    name: &str,
    mode: u32,
    open: impl FnOnce() -> std::io::Result<R>,
) -> Result<(), ContentFileError> {
    if !matches!(mode & 0o170000, 0 | 0o100000) {
        return Err(ContentFileError::UnsupportedType(name.into()));
    }
    let extension = name
        .rsplit(['/', '\\'])
        .next()
        .and_then(|n| n.rsplit_once('.'));
    let executable = extension.is_some_and(|(_, ext)| {
        [
            "exe", "dll", "so", "dylib", "class", "jar", "com", "bat", "cmd", "ps1", "wasm",
        ]
        .iter()
        .any(|kind| ext.eq_ignore_ascii_case(kind))
    });
    if executable || mode & 0o111 != 0 {
        return Err(ContentFileError::Executable(name.into()));
    }
    let (head, read) = file_head(name, open)?;
    let head = &head[..read];
    // Any `MZ` is refused here, not only a PE: a content file has no use for it.
    if binary_kind(head).is_some()
        || head.starts_with(b"MZ")
        || (head.starts_with(b"#!") && head.get(2) != Some(&b'['))
    {
        return Err(ContentFileError::Executable(name.into()));
    }
    Ok(())
}

/// The type check of a native package's source file, before it is opened: a plain file. Its name
/// and mode play no part, so a C source tree may carry a `configure` script, a `.bat` or a file
/// with the executable bit; its bytes are checked by [`refuse_binary`].
fn check_source_type(name: &str, mode: u32) -> Result<(), ContentFileError> {
    if matches!(mode & 0o170000, 0 | 0o100000) {
        Ok(())
    } else {
        Err(ContentFileError::UnsupportedType(name.into()))
    }
}

/// The file mode of `name` in a mod folder: the mode itself for a file (0 off Unix), a link's for
/// anything else, so that nothing but a plain file passes the content checks.
fn folder_mode(root: &Path, name: &str) -> Result<u32, ContentFileError> {
    let metadata =
        std::fs::symlink_metadata(root.join(name)).map_err(|source| ContentFileError::Read {
            name: name.into(),
            source,
        })?;
    Ok(if metadata.is_file() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            metadata.mode()
        }
        #[cfg(not(unix))]
        {
            0
        }
    } else {
        0o120000
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn read_folder_uses(root: &Path) -> Result<Option<Vec<u8>>, String> {
    let path = root.join(USES_FILE);
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{USES_FILE}: {e}")),
        Ok(_) => {}
    }
    check_folder_file(root, USES_FILE)?;
    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .map_err(|e| format!("{USES_FILE}: {e}"))?
        .take(MAX_USES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{USES_FILE}: {e}"))?;
    if bytes.len() as u64 > MAX_USES {
        return Err(format!("{USES_FILE} exceeds {} MiB", MAX_USES >> 20));
    }
    serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(&bytes)
        .map_err(|e| format!("{USES_FILE}: {e}"))?;
    Ok(Some(bytes))
}

fn check_folder_file(root: &Path, name: &str) -> Result<(), String> {
    let path = root.join(name);
    let mode = folder_mode(root, name).map_err(|e| e.to_string())?;
    check_file(name, mode, || std::fs::File::open(&path)).map_err(|e| e.to_string())
}

/// The `.smod` files and mod folders in `dir`, by name.
pub fn find(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "smod") || p.join(MANIFEST).is_file())
        .collect();
    found.sort();
    found
}

#[cfg(test)]
#[path = "manifest/runtime_files_tests.rs"]
mod runtime_files_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// A content package names the native mod that serves its code; the host opts in.
    #[test]
    fn served_by_and_serves_content_read_and_served_by_is_refused_on_native_mods() {
        let head = "id='runes'\nname='Runes'\nversion='1.0.0'\napi='0.2'\n";
        let native = "[native]\ngame='26.3'\nsunrise='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'\n";
        let served = Manifest::parse(&format!("{head}served_by='java_host'\n")).unwrap();
        assert_eq!(served.served_by.as_deref(), Some("java_host"));
        let host = Manifest::parse(&format!("{head}{native}serves_content=true\n")).unwrap();
        assert!(host.native.unwrap().serves_content);
        let both = Manifest::parse(&format!("{head}served_by='java_host'\n{native}")).unwrap_err();
        assert!(both.contains("cannot declare served_by"), "{both}");
        let own = Manifest::parse(&format!("{head}served_by='runes'\n")).unwrap_err();
        assert!(own.contains("own id"), "{own}");
    }

    /// The game's `mod_vocab` forms of a side and an ordering, which manifests and the index carry.
    #[test]
    fn sides_and_orders_keep_the_games_serde_forms() {
        for (text, side) in [
            ("client", Side::Client),
            ("both", Side::Both),
            ("server", Side::Server),
        ] {
            assert_eq!(
                serde_json::from_str::<Side>(&format!("{text:?}")).unwrap(),
                side
            );
            assert_eq!(serde_json::to_string(&side).unwrap(), format!("{text:?}"));
        }
        for (text, order) in [
            ("after", DependencyOrder::After),
            ("before", DependencyOrder::Before),
            ("none", DependencyOrder::None),
        ] {
            assert_eq!(
                serde_json::from_str::<DependencyOrder>(&format!("{text:?}")).unwrap(),
                order
            );
            assert_eq!(serde_json::to_string(&order).unwrap(), format!("{text:?}"));
        }
        assert_eq!(
            (Side::default(), DependencyOrder::default()),
            (Side::Client, DependencyOrder::After)
        );
        assert!(serde_json::from_str::<Side>("\"Both\"").is_err());
    }

    /// The mod tool writes the API line into the package, so a project's manifest may leave it out.
    #[test]
    fn a_manifest_without_an_api_line_reads_the_api_this_build_loads() {
        let manifest = Manifest::parse(
            "id='runes'\nname='Runes'\nversion='1.0.0'\n[native]\ngame='26.3'\nsunrise='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'\n",
        )
        .unwrap();
        assert_eq!(manifest.api, "0.2");
        let other = Manifest::parse("id='runes'\nname='Runes'\nversion='1.0.0'\napi='0.1'\n");
        assert!(other.unwrap_err().contains("unsupported content API"));
    }

    #[test]
    fn windows_device_mod_ids_are_refused() {
        for id in ["con", "prn", "aux", "nul", "com1", "com9", "lpt1", "lpt9"] {
            assert!(
                Manifest::parse(&EXAMPLE.replace("ruby", id))
                    .unwrap_err()
                    .contains("reserved Windows device name"),
                "{id}"
            );
        }
        for id in ["console", "com0", "com10", "lpt0", "con_tool"] {
            assert!(
                Manifest::parse(&EXAMPLE.replace("ruby", id)).is_ok(),
                "{id}"
            );
        }
    }

    const EXAMPLE: &str = r#"
id = "ruby"
name = "Ruby"
version = "0.1.0"
api = "0.2"
capabilities = ["registries"]
"#;

    #[test]
    fn refuses_bad_ids_and_retired_manifests() {
        let bad_id = EXAMPLE.replace("ruby", "Ruby!");
        assert!(Manifest::parse(&bad_id).unwrap_err().contains("is not"));
        for api in ["0.1", "0.3", "0.4"] {
            let text = EXAMPLE.replace("api = \"0.2\"", &format!("api = \"{api}\""));
            assert!(
                Manifest::parse(&text)
                    .unwrap_err()
                    .contains("unsupported content API")
            );
        }
        for caps in ["[]", "[\"hud\"]", "[\"registries\", \"registries\"]"] {
            let text = EXAMPLE.replace("[\"registries\"]", caps);
            assert!(Manifest::parse(&text).is_err());
        }
        for field in ["network = true", "experimental = []", "side = \"local\""] {
            assert!(Manifest::parse(&format!("{EXAMPLE}{field}\n")).is_err());
        }
        assert_eq!(Manifest::parse(EXAMPLE).unwrap().side, Side::Client);
        assert_eq!(
            Manifest::parse(&format!("{EXAMPLE}side = \"both\"\n"))
                .unwrap()
                .side,
            Side::Both
        );
        let server = Manifest::parse(&format!("{EXAMPLE}side = \"server\"\n")).unwrap();
        assert_eq!(server.side, Side::Server);
        let id = &server.id;
        assert_eq!(
            server.server_only_declaration(
                [
                    ("block", "minecraft:stone"),
                    ("item", &format!("{id}:marker")[..])
                ]
                .into_iter()
            ),
            Some(format!(
                "{id}: a side = \"server\" mod declares the item {id}:marker, which every client \
                 must have; declare side = \"both\""
            ))
        );
        assert_eq!(
            server.server_only_declaration([("block", "minecraft:stone")].into_iter()),
            None
        );
    }

    #[test]
    fn native_manifest_requires_exact_pins_and_known_fields() {
        let native = format!(
            "[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        Manifest::parse(&format!("{EXAMPLE}{native}")).unwrap();
        assert!(
            Manifest::parse(&format!(
                "{EXAMPLE}{}",
                native.replace(&"a".repeat(40), "main")
            ))
            .is_err()
        );
        for unsupported in ["unexpected = true", "[[native.post_build]]\nbin = 'bake'"] {
            assert!(Manifest::parse(&format!("{EXAMPLE}{native}{unsupported}\n")).is_err());
        }
    }

    #[test]
    fn native_unknown_fields_are_refused_at_the_owning_boundary() {
        let pin = "a".repeat(40);
        let base = format!("{EXAMPLE}[native]\ngame='26.3'\nsunrise='{pin}'\nmixins='{pin}'\n");
        let mut accepted = Vec::new();
        for field in [
            "injection=[]",
            "[[native.injection]]\nid='tick'\ntarget='game::tick'\nkind='HEAD'\nraw='tick'",
            "internals=[]",
            "[[native.internals]]\npath='Game'\nhash='h'",
            "unexpected=true",
        ] {
            let text = format!("{base}{field}\n");
            match Manifest::parse(&text) {
                Ok(_) => accepted.push(field),
                Err(error) => assert!(error.contains("unknown field"), "{error}"),
            }
            // The same owning serde boundary applies outside Manifest::parse.
            if toml::from_str::<Native>(text.split("[native]\n").nth(1).unwrap()).is_ok() {
                accepted.push(field);
            }
        }
        assert!(
            accepted.is_empty(),
            "accepted unknown native fields: {accepted:?}"
        );
    }

    #[test]
    fn internal_kinds_distinguish_members_and_round_trip() {
        let base = format!(
            "{EXAMPLE}[native]\ngame='26.3'\nsunrise='{}'\nmixins='{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let declaration = "internals=[{path='Round::listening',kind='field',hash='1234'}, {path='Round::listening',kind='method',hash='5678'}]\n";
        let manifest =
            Manifest::parse(&base.replace("[native]", &format!("{declaration}[native]"))).unwrap();
        assert_eq!(manifest.internals[1].kind.as_deref(), Some("method"));
        let row = toml::to_string(&manifest.internals[1]).unwrap();
        assert_eq!(
            toml::from_str::<Internal>(&row).unwrap(),
            manifest.internals[1]
        );
        let native = manifest.native.unwrap();
        let text = toml::to_string(&native).unwrap();
        assert_eq!(toml::from_str::<Native>(&text).unwrap(), native);
        for kind in ["field", ""] {
            let invalid = declaration.replace("kind='method'", &format!("kind='{kind}'"));
            assert!(
                Manifest::parse(&base.replace("[native]", &format!("{invalid}[native]"))).is_err()
            );
        }
        let untyped = Internal {
            path: "Game".into(),
            kind: None,
            hash: Some("1234".into()),
        };
        assert!(!toml::to_string(&untyped).unwrap().contains("kind"));
    }

    #[test]
    fn native_modules_are_validated() {
        let native = format!(
            "[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let module = "[[native.module]]\nid = 'runtime'\nfile = 'crates/sunrise-client/src/game.rs'\nname = 'runtime'\nsource = 'src/runtime.rs'\n";
        let manifest = Manifest::parse(&format!("{EXAMPLE}{native}{module}")).unwrap();
        let module = &manifest.native.as_ref().unwrap().module[0];
        assert_eq!(module.name, "runtime");
        assert_eq!(module.visibility, None);
        // Support modules still forward whole to horizon-mixins.
        let forwarded = toml::to_string(manifest.native.as_ref().unwrap()).unwrap();
        assert!(forwarded.contains("[[module]]"));
        assert!(forwarded.contains("id = \"runtime\""));
        let valid = "[[native.module]]\nid = 'runtime'\nfile = 'crates/sunrise-client/src/game.rs'\nname = 'runtime'\nsource = 'src/runtime.rs'\n";
        for bad in [
            // escapes the package's src/
            valid.replace("source = 'src/runtime.rs'", "source = '../escape.rs'"),
            // unknown field
            format!("{valid}order = 1\n"),
            // duplicate module id
            format!("{valid}{valid}"),
            format!(
                "{valid}[[native.module]]\nid='runtime'\nfile='game.rs'\nname='other'\nsource='src/other.rs'\n"
            ),
            // not an identifier
            valid.replace("name = 'runtime'", "name = 'run time'"),
            // file must stay a plain path
            valid.replace(
                "file = 'crates/sunrise-client/src/game.rs'",
                "file = '/abs.rs'",
            ),
            // horizon-mixins accepts exactly pub and pub(crate)
            valid.to_owned() + "visibility = 'public'\n",
        ] {
            assert!(
                Manifest::parse(&format!("{EXAMPLE}{native}{bad}")).is_err(),
                "accepted: {bad}"
            );
        }
        let public = valid.to_owned() + "visibility = 'pub(crate)'\n";
        assert!(Manifest::parse(&format!("{EXAMPLE}{native}{public}")).is_ok());
    }

    #[test]
    fn properties_are_typed_and_bounded() {
        let properties = |rows: &str| Manifest::parse(&format!("{EXAMPLE}[properties]\n{rows}"));
        let manifest = properties("tier = 3\nratio = 0.5\nlabel = 'x'\nfast = true\n").unwrap();
        assert_eq!(manifest.properties["tier"], Property::Integer(3));
        assert_eq!(manifest.properties["ratio"], Property::Float(0.5));
        assert_eq!(manifest.properties["label"], Property::String("x".into()));
        assert_eq!(manifest.properties["fast"], Property::Boolean(true));
        let many: String = (0..65).map(|i| format!("k{i} = {i}\n")).collect();
        let error = properties(&many).unwrap_err();
        assert!(error.contains("at most 64 keys"), "{error}");
        assert!(properties(&format!("text = '{}'\n", "a".repeat(8 << 10))).is_err());
        assert!(properties("list = [1]\n").is_err());
    }

    #[test]
    fn dependencies_are_typed_for_content_and_code_mods() {
        let deps =
            "[dependencies]\ncreate = '^6.0'\naddon = { version = '^1.2', optional = true }\n";
        let content = Manifest::parse(&format!("{EXAMPLE}{deps}")).unwrap();
        assert_eq!(content.dependencies.len(), 2);
        let native = format!(
            "[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let text = format!("{EXAMPLE}{deps}{native}");
        let manifest = Manifest::parse(&text).unwrap();
        assert!(
            matches!(&manifest.dependencies["addon"], Dependency::Detailed(details) if details.optional)
        );
        assert!(matches!(
            &manifest.dependencies["create"],
            Dependency::Version(_)
        ));
        for bad in [
            text.replace("^6.0", "not semver"),
            text.replace("create =", "\"not-valid\" ="),
            text.replace("optional = true", "unexpected = true"),
        ] {
            assert!(Manifest::parse(&bad).is_err());
        }
    }

    #[test]
    fn a_code_mod_carries_one_bounded_uses_table_and_a_content_mod_none() {
        let native = format!(
            "{EXAMPLE}[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let cargo = b"[package]\nname = 'ruby'\n".as_slice();
        let lib = b"pub fn f() {}\n".as_slice();
        let table = br#"{"schema":1,"made_for":"x","entries":[]}"#.as_slice();
        let bytes = Package::write_files(
            &native,
            &[
                ("Cargo.toml", cargo),
                ("src/lib.rs", lib),
                (USES_FILE, table),
            ],
        )
        .unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        Package::read_zip(&mut zip).unwrap();
        let mut read = Vec::new();
        zip.by_name(USES_FILE)
            .unwrap()
            .read_to_end(&mut read)
            .unwrap();
        assert_eq!(read, table);
        let large = vec![b' '; MAX_USES as usize + 1];
        assert!(
            Package::write_files(
                &native,
                &[
                    ("Cargo.toml", cargo),
                    ("src/lib.rs", lib),
                    (USES_FILE, &large)
                ],
            )
            .unwrap_err()
            .contains(USES_FILE)
        );
        // A content mod has no code to scan, so no table either, written or read.
        assert!(Package::write_files(EXAMPLE, &[(USES_FILE, table)]).is_err());
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, bytes) in [(MANIFEST, EXAMPLE.as_bytes()), (USES_FILE, table)] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        let mut archive = zip::ZipArchive::new(zip.finish().unwrap()).unwrap();
        let error = Package::read_zip(&mut archive)
            .err()
            .expect("a content package's uses.json is refused");
        assert!(error.contains(USES_FILE), "{error}");
    }

    #[test]
    fn layout_metadata_round_trips_and_malformed_archives_are_refused() {
        let layouts = br#"{"ruby":{"ruby::T":"0123456789abcdef"}}"#;
        let bytes = Package::write_files(EXAMPLE, &[("layouts.json", layouts)]).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let package = Package::read_zip(&mut zip).unwrap();
        let changed =
            crate::layouts::Layouts::read(br#"{"ruby":{"ruby::T":"1123456789abcdef"}}"#.as_slice())
                .unwrap();
        assert_eq!(
            package.layouts.first_difference("ruby", &changed),
            Some("ruby::T")
        );
        assert!(
            Package::write_files(EXAMPLE, &[("layouts.json", b"{")])
                .unwrap_err()
                .contains("layouts.json")
        );
    }

    #[test]
    fn flat_native_archive_round_trips_through_shared_writer() {
        let manifest = format!(
            "{EXAMPLE}[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let files: &[(&str, &[u8])] = &[
            (
                "Cargo.toml",
                b"[package]\nname = 'other'\nversion = '2.0.0'\n",
            ),
            ("src/lib.rs", b"pub fn hooks() {}"),
        ];
        let bytes = Package::write_files(&manifest, files).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let package = Package::read_zip(&mut zip).unwrap();
        assert_eq!(package.manifest.id, "ruby");
        assert_eq!(package.manifest.version, "0.1.0");
        assert_eq!(package.manifest.native.unwrap().game, "26.3");
        assert!(
            Package::write_files(&manifest, &[("Cargo.toml", b""), ("src/../bad.rs", b"")])
                .is_err()
        );
        assert!(
            Package::write_files(
                &manifest,
                &[
                    ("Cargo.toml", b""),
                    ("src/lib.rs", b""),
                    ("src/lib.rs", b"")
                ]
            )
            .is_err()
        );
    }

    #[test]
    fn a_native_project_folder_packs_the_files_cargo_package_would_carry() {
        let root = std::env::temp_dir().join(format!("sunrise-flat-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("vendor/zlib")).unwrap();
        std::fs::create_dir_all(root.join(".git/hooks")).unwrap();
        std::fs::create_dir_all(root.join("target/generated")).unwrap();
        std::fs::create_dir_all(root.join(".cargo")).unwrap();
        let manifest = format!(
            "{EXAMPLE}[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        std::fs::write(root.join(MANIFEST), manifest).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = 'other'\nversion = '2.0.0'\n[dependencies]\nhelper = { path = 'crates/helper' }\n",
        )
        .unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn hooks() {}\n").unwrap();
        std::fs::write(
            root.join("vendor/zlib/inflate.c"),
            "int inflate(void) { return 0; }\n",
        )
        .unwrap();
        std::fs::write(root.join("configure"), "#!/bin/sh\necho ok\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                root.join("configure"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        std::fs::create_dir_all(root.join("crates/helper/src")).unwrap();
        std::fs::write(
            root.join("crates/helper/Cargo.toml"),
            "[package]\nname = 'helper'\nversion = '0.1.0'\n",
        )
        .unwrap();
        std::fs::write(root.join("crates/helper/src/lib.rs"), "pub fn h() {}\n").unwrap();
        std::fs::create_dir_all(root.join("tools/unused/src")).unwrap();
        std::fs::write(
            root.join("tools/unused/Cargo.toml"),
            "[package]\nname = 'unused'\nversion = '0.1.0'\n",
        )
        .unwrap();
        std::fs::write(root.join("tools/unused/src/lib.rs"), "fn u() {}\n").unwrap();
        std::fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(root.join("ignored.txt"), "skip me").unwrap();
        let registry_lock = "version = 4\n[[package]]\nname = 'itoa'\nversion = '1.0.18'\n";
        std::fs::write(root.join("Cargo.lock"), registry_lock).unwrap();
        std::fs::write(root.join(".git/hooks/pre-commit"), "#!/bin/sh\n").unwrap();
        std::fs::write(root.join("target/generated/bad.rs"), "#!/bin/sh\n").unwrap();
        std::fs::write(root.join(".cargo/config.toml"), "[build]\n").unwrap();
        let sdk = root.join(".sunrise/sdk/api/crates/sunrise-mod-api/src");
        std::fs::create_dir_all(&sdk).unwrap();
        std::fs::write(sdk.join("lib.rs"), "pub fn api() {}\n").unwrap();
        std::fs::write(root.join(".sunrise/sdk/sdk.json"), "{}").unwrap();
        let bytes = Package::pack_directory(&root).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(Package::read_zip(&mut zip).unwrap().manifest.id, "ruby");
        for kept in [
            "Cargo.toml",
            "Cargo.lock",
            "src/lib.rs",
            "vendor/zlib/inflate.c",
            "configure",
            "crates/helper/Cargo.toml",
            "crates/helper/src/lib.rs",
        ] {
            assert!(zip.by_name(kept).is_ok(), "missing {kept}");
        }
        let mut packed_lock = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("Cargo.lock").unwrap(), &mut packed_lock)
            .unwrap();
        assert_eq!(packed_lock, registry_lock);
        for skipped in [
            ".git/hooks/pre-commit",
            "target/generated/bad.rs",
            ".cargo/config.toml",
            "ignored.txt",
            "tools/unused/Cargo.toml",
            "tools/unused/src/lib.rs",
        ] {
            assert!(zip.by_name(skipped).is_err(), "unexpected {skipped}");
        }
        assert!(
            zip.file_names_raw()
                .all(|name| !name.starts_with(b".sunrise")),
            "{:?}",
            zip.file_names_raw()
                .map(String::from_utf8_lossy)
                .collect::<Vec<_>>()
        );
        assert!(!source_file(
            ".sunrise/sdk/api/crates/sunrise-mod-api/src/lib.rs"
        ));
        // A name packing cannot encode is left out, and is no bar.
        // macOS's file systems refuse a name that is not UTF-8, so only Linux can make one.
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::ffi::OsStrExt;
            let bad = root.join(std::ffi::OsStr::from_bytes(b"bad-\xff.c"));
            std::fs::write(&bad, b"int x;\n").unwrap();
        }
        // On Unix a `\` is part of a file name, never a separator.
        #[cfg(unix)]
        std::fs::write(root.join("src").join("a\\b.rs"), "").unwrap();
        let bytes = Package::pack_directory(&root).unwrap();
        let zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert!(
            zip.file_names_raw()
                .all(|name| !name.starts_with(b"bad-") && !name.contains(&b'\\')),
            "unencodable names must be left out"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_prebuilt_binary_named_as_source_is_refused_when_packed_and_skipped_when_read() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("sunrise-bin-src-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        let manifest = format!(
            "{EXAMPLE}[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        std::fs::write(root.join(MANIFEST), &manifest).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = 'ruby'\nversion = '0.1.0'\n",
        )
        .unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn hooks() {}\n").unwrap();
        let mut pe = vec![0u8; 0x48];
        pe[..2].copy_from_slice(b"MZ");
        pe[0x3c] = 0x40;
        pe[0x40..0x44].copy_from_slice(b"PE\0\0");
        let mut pe_past_1k = vec![0u8; 0x508];
        pe_past_1k[..2].copy_from_slice(b"MZ");
        pe_past_1k[0x3c..0x40].copy_from_slice(&0x500u32.to_le_bytes());
        pe_past_1k[0x500..0x504].copy_from_slice(b"PE\0\0");
        for (bytes, kind) in [
            (&b"\x7fELF\x02\x01\x01\0"[..], "ELF"),
            (b"\xcf\xfa\xed\xfe\x07\0\0\x01", "Mach-O"),
            (&pe, "PE"),
            (&pe_past_1k, "PE"),
            (b"!<arch>\nfoo.o/", "ar archive"),
        ] {
            std::fs::write(root.join("src/lib.rs"), bytes).unwrap();
            let error = Package::pack_directory(&root).unwrap_err();
            assert!(
                error.contains(kind) && error.contains("prebuilt binaries"),
                "{error}"
            );
            let error = Package::write_files(&manifest, &[("src/lib.rs", bytes)]).unwrap_err();
            assert!(
                error.contains(kind) && error.contains("prebuilt binaries"),
                "{error}"
            );
            // Package::read_zip skips source entries unread.
            let archive = root.join("skip.smod");
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file(MANIFEST, options).unwrap();
            zip.write_all(manifest.as_bytes()).unwrap();
            zip.start_file("src/lib.rs", options).unwrap();
            zip.write_all(bytes).unwrap();
            zip.finish().unwrap();
            assert_eq!(Package::read(&archive).unwrap().manifest.id, "ruby");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_native_archive_with_any_other_entry_loads() {
        let manifest = format!(
            "{EXAMPLE}[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let bytes = Package::write_files(
            &manifest,
            &[
                ("Cargo.toml", b"[package]\nname='ruby'\nversion='1'\n"),
                ("src/lib.rs", b"pub fn hooks() {}"),
                ("vendor/zlib/inflate.c", b"int inflate(void) { return 0; }"),
                ("sunrise-native.json", b"{}"),
            ],
        )
        .unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(Package::read_zip(&mut zip).unwrap().manifest.id, "ruby");
    }

    #[test]
    fn an_archive_entry_that_escapes_or_links_is_refused_by_name() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("sunrise-escape-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let archive = root.join("bad.smod");
        for bad in ["../escape.c", "/abs/path.c", "src/a."] {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file(MANIFEST, options).unwrap();
            zip.write_all(EXAMPLE.as_bytes()).unwrap();
            zip.start_file(bad, options).unwrap();
            zip.write_all(b"x").unwrap();
            zip.finish().unwrap();
            let error = Package::read(&archive).err().unwrap();
            assert!(error.contains(bad), "{bad}: {error}");
        }
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        let options = zip::write::SimpleFileOptions::default().system(zip::System::Unix);
        zip.start_file(MANIFEST, options).unwrap();
        zip.write_all(EXAMPLE.as_bytes()).unwrap();
        zip.add_symlink("vendor/link.c", "../outside", options)
            .unwrap();
        zip.finish().unwrap();
        let error = Package::read(&archive).err().unwrap();
        assert!(error.contains("vendor/link.c"), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_native_project_of_many_source_files_packs_and_reads() {
        let manifest = format!(
            "{EXAMPLE}[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let names: Vec<String> = (0..600).map(|i| format!("vendor/f{i}.c")).collect();
        let mut entries: Vec<(&str, &[u8])> = vec![
            ("Cargo.toml", b"[package]\nname='ruby'\nversion='1'\n"),
            ("src/lib.rs", b"pub fn hooks() {}"),
        ];
        entries.extend(names.iter().map(|n| (n.as_str(), &b"int x;\n"[..])));
        let bytes = Package::write_files(&manifest, &entries).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(Package::read_zip(&mut zip).unwrap().manifest.id, "ruby");
        Package::check_source_bounds(&mut zip).unwrap();
    }

    #[test]
    fn a_project_reader_ignores_unrelated_root_paths() {
        let root = std::env::temp_dir().join(format!("sunrise-unrelated-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        let manifest = format!(
            "{EXAMPLE}[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        std::fs::write(root.join(MANIFEST), manifest).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = 'ruby'\nversion = '0.1.0'\n",
        )
        .unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn hooks() {}\n").unwrap();
        for name in ["sunrise-native.json", "horizon-mixin.toml"] {
            std::fs::write(root.join(name), "").unwrap();
        }
        std::fs::create_dir(root.join("native")).unwrap();
        std::fs::create_dir(root.join("rust")).unwrap();
        assert_eq!(Package::read(&root).unwrap().manifest.id, "ruby");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_packer_refuses_names_that_differ_only_by_case_and_parts_a_windows_extraction_renames() {
        let manifest = format!(
            "{EXAMPLE}[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let error = Package::write_files(
            &manifest,
            &[("build.rs", b"".as_slice()), ("Build.rs", b"".as_slice())],
        )
        .unwrap_err();
        assert!(
            error.contains("Build.rs") && error.contains("build.rs"),
            "{error}"
        );
        assert!(plain_path("src/a.c"));
        assert!(!plain_path("src/a."));
        assert!(!plain_path("src/a /b.c"));
    }

    #[test]
    fn package_kind_requires_matching_source_and_native_declaration() {
        let root =
            std::env::temp_dir().join(format!("sunrise-package-kind-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(MANIFEST), EXAMPLE).unwrap();
        assert!(Package::read(&root).is_ok());
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = 'unrelated'\nversion = '7.0.0'\n",
        )
        .unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        assert!(Package::read(&root).is_err());
        let native = format!(
            "[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        std::fs::write(root.join(MANIFEST), format!("{EXAMPLE}{native}")).unwrap();
        assert_eq!(Package::read(&root).unwrap().manifest.id, "ruby");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn checks_extra_namespaces() {
        let with = |list: &str| Manifest::parse(&format!("{EXAMPLE}namespaces = [{list}]\n"));
        let m = with("\"kaisyn\", \"towns_and_towers\"").unwrap();
        let owned: Vec<&str> = m.owned_namespaces().collect();
        assert_eq!(owned, ["ruby", "kaisyn", "towns_and_towers"]);
        for reserved in ["minecraft", "c", "sunrise"] {
            let e = with(&format!("\"{reserved}\"")).unwrap_err();
            assert!(e.contains("not a mod's to claim"), "{e}");
        }
        assert!(with("\"ruby\"").unwrap_err().contains("twice"));
        assert!(with("\"a\", \"a\"").unwrap_err().contains("twice"));
        assert!(with("\"Kaisyn\"").unwrap_err().contains("is not"));
        assert!(with("\"\"").unwrap_err().contains("is not"));
    }

    #[test]
    fn the_mods_screen_fields_are_optional_and_links_must_be_http() {
        let m = Manifest::parse(EXAMPLE).unwrap();
        assert!(m.license.is_empty() && m.credits.is_empty());
        assert!(m.license_url.is_none() && m.homepage.is_none() && m.issues.is_none());
        let full = format!(
            "{EXAMPLE}license = \"MIT\"
credits = \"Alex\"
homepage = \"https://example.org\"
issues = \"http://example.org/issues\"
"
        );
        let m = Manifest::parse(&full).unwrap();
        assert_eq!((m.license.as_str(), m.credits.as_str()), ("MIT", "Alex"));
        assert_eq!(m.homepage.as_deref(), Some("https://example.org"));
        for field in ["license_url", "homepage", "issues"] {
            let e = Manifest::parse(&format!(
                "{EXAMPLE}{field} = \"file:///etc/passwd\"
"
            ))
            .unwrap_err();
            assert!(e.contains(field), "{e}");
        }
    }

    #[test]
    fn content_round_trips_and_executable_files_are_refused() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("sunrise-mods-{}", std::process::id()));
        let dir = root.join("folder");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST), EXAMPLE).unwrap();
        let archive = root.join("ruby.smod");
        std::fs::write(&archive, Package::write_files(EXAMPLE, &[]).unwrap()).unwrap();
        assert_eq!(Package::read(&archive).unwrap().manifest.id, "ruby");
        assert_eq!(Package::read(&dir).unwrap().manifest.id, "ruby");
        let cases: &[(&str, &[u8])] = &[
            ("mod.exe", b""),
            ("./mod.exe", b"MZ"),
            ("MOD.EXE", b""),
            ("sub/mod.exe", b""),
            ("assets/ruby/CODE.ExE", b""),
            ("assets/ruby/CODE.WaSm", b""),
            ("assets/ruby/model.bin", b"\0asm"),
            ("assets/ruby/native.bin", b"\x7fELF"),
            ("assets/ruby/windows.bin", b"MZ"),
            ("assets/ruby/mac.bin", b"\xcf\xfa\xed\xfe"),
            ("assets/ruby/script", b"#!/bin/sh"),
            ("assets/ruby/program.EXE", b""),
        ];
        for (name, bytes) in cases {
            let file = dir.join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, bytes).unwrap();
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file(MANIFEST, options).unwrap();
            zip.write_all(EXAMPLE.as_bytes()).unwrap();
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
            zip.finish().unwrap();
            if name.starts_with("assets/") {
                let error = Package::read(&archive).err().unwrap();
                assert!(
                    error.contains("unsupported executable file"),
                    "{name}: {error}"
                );
                let error = Package::read(&dir).err().unwrap();
                assert!(
                    error.contains("unsupported executable file"),
                    "{name}: {error}"
                );
            } else {
                // Not a content file: the reader skips it, in an archive as in a folder.
                assert!(Package::read(&dir).is_ok(), "unpackaged {name}");
                if *name != "./mod.exe" {
                    assert!(Package::read(&archive).is_ok(), "skipped {name}");
                }
            }
            std::fs::remove_file(file).unwrap();
        }
        for (name, bytes) in cases.iter().filter(|(name, _)| name.starts_with("assets/")) {
            assert!(
                Package::write_files(EXAMPLE, &[(name, bytes)]).is_err(),
                "{name}"
            );
        }
        // Native bundle sources can start with Rust inner attributes, not a script shebang.
        assert!(
            Package::write_files(
                EXAMPLE,
                &[("assets/ruby/source.txt", b"#![allow(dead_code)]")]
            )
            .is_ok()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn project_folder_loads_without_packaging_git_hooks() {
        let root = std::env::temp_dir().join(format!("sunrise-mod-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(root.join(MANIFEST), EXAMPLE).unwrap();
        std::fs::create_dir_all(root.join(".git/hooks")).unwrap();
        std::fs::write(root.join(".git/hooks/pre-commit"), b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                root.join(".git/hooks/pre-commit"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        std::fs::create_dir_all(root.join("assets/ruby")).unwrap();
        std::fs::write(root.join("assets/ruby/icon.png"), b"image").unwrap();
        assert_eq!(Package::read(&root).unwrap().manifest.id, "ruby");
        std::fs::write(root.join("assets/ruby/icon.png"), b"\0asm").unwrap();
        assert!(
            Package::read(&root)
                .err()
                .unwrap()
                .contains("unsupported executable")
        );
        std::fs::remove_file(root.join("assets/ruby/icon.png")).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn structure_nbt_is_data_but_not_an_executable_exception() {
        let name = "data/ruby/structure/hut.nbt";
        // An uncompressed, named NBT root containing an empty compound.
        let nbt = [10u8, 0, 0, 0];
        assert!(check_file(name, 0o100644, || Ok(std::io::Cursor::new(nbt))).is_ok());
        assert!(matches!(
            check_file(name, 0o100755, || Ok(std::io::Cursor::new(nbt))),
            Err(ContentFileError::Executable(_))
        ));
        for (header, expected) in [
            (&b"\x7fELF"[..], "ELF"),
            (b"MZ!!", "PE"),
            (b"\0asm", "WebAssembly"),
            (b"\xcf\xfa\xed\xfe", "Mach-O"),
            (
                b"\xca\xfe\xba\xbe",
                "Mach-O universal binary or Java class file",
            ),
            (b"!<arch>\n", "ar archive"),
            // A file that is only a shebang's first two bytes.
            (b"#!", "script"),
        ] {
            assert!(
                matches!(
                    check_file(name, 0o100644, || Ok(std::io::Cursor::new(header))),
                    Err(ContentFileError::Executable(_))
                ),
                "{expected}"
            );
        }
    }

    #[test]
    fn archive_links_and_executable_modes_are_refused() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("sunrise-mods-kinds-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("ruby.smod");
        for link in [false, true] {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
            // Made by a Unix tool on every OS: the writer's default on Windows is DOS attributes,
            // which hold no mode to refuse.
            let options = zip::write::SimpleFileOptions::default().system(zip::System::Unix);
            zip.start_file(MANIFEST, options).unwrap();
            zip.write_all(EXAMPLE.as_bytes()).unwrap();
            if link {
                zip.add_symlink("assets/ruby/link", "missing", options)
                    .unwrap();
            } else {
                zip.start_file("assets/ruby/program", options.unix_permissions(0o755))
                    .unwrap();
                zip.write_all(b"ordinary bytes").unwrap();
            }
            zip.finish().unwrap();
            let error = Package::read(&path).err().unwrap();
            assert!(
                error.contains(if link {
                    "unsupported file type"
                } else {
                    "unsupported executable"
                }),
                "{error}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn folder_links_and_executable_modes_are_refused() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root =
            std::env::temp_dir().join(format!("sunrise-mods-folder-kinds-{}", std::process::id()));
        let dir = root.join("folder");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST), EXAMPLE).unwrap();
        std::fs::create_dir_all(dir.join("assets/ruby")).unwrap();
        let file = dir.join("assets/ruby/program");
        std::fs::write(&file, b"ordinary bytes").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = Package::read(&dir).err().unwrap();
        assert!(error.contains("unsupported executable"), "{error}");
        std::fs::remove_file(&file).unwrap();
        for name in ["assets/ruby/mod.wasm", "assets/ruby/nested/link"] {
            let link = dir.join(name);
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            symlink("missing", &link).unwrap();
            let error = Package::read(&dir).err().unwrap();
            assert!(error.contains("unsupported file type"), "{error}");
            std::fs::remove_file(link).unwrap();
        }
        std::fs::remove_dir_all(dir.join("assets")).unwrap();
        symlink("missing", dir.join("assets")).unwrap();
        assert!(
            Package::read(&dir)
                .err()
                .unwrap()
                .contains("unsupported file type")
        );
        let link = root.join("linked-folder");
        symlink(&dir, &link).unwrap();
        let error = Package::read(&link).err().unwrap();
        assert!(error.contains("unsupported file type"), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pack_directory_carries_valid_uses_json_and_refuses_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let native = format!(
            "{EXAMPLE}[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join(MANIFEST), &native).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = 'ruby'\nversion = '0.1.0'\nedition = '2024'\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn f() {}\n").unwrap();
        let table = br#"{"schema":1,"made_for":"x","entries":[]}"#;
        std::fs::write(dir.path().join(USES_FILE), table).unwrap();

        let bytes = Package::pack_directory(dir.path()).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut read = Vec::new();
        zip.by_name(USES_FILE)
            .unwrap()
            .read_to_end(&mut read)
            .unwrap();
        assert_eq!(read, table);

        // Malformed JSON is refused instead of silently dropped.
        std::fs::write(dir.path().join(USES_FILE), b"{bad json").unwrap();
        let error = Package::pack_directory(dir.path()).unwrap_err();
        assert!(error.contains(USES_FILE), "{error}");

        // Non-object JSON is refused.
        std::fs::write(dir.path().join(USES_FILE), b"[]").unwrap();
        let error = Package::pack_directory(dir.path()).unwrap_err();
        assert!(error.contains(USES_FILE), "{error}");
    }

    #[test]
    fn include_overrides_gitignore_and_cargo_manifests_are_checked_and_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let native = format!(
            "{EXAMPLE}[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("out")).unwrap();
        std::fs::write(root.join(MANIFEST), &native).unwrap();
        std::fs::write(root.join(".gitignore"), "generated.c\nout/\n").unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = 'ruby'\nversion = '0.1.0'\ninclude = ['src/**', 'generated.c', 'out/shim.c']\n",
        )
        .unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn hooks() {}\n").unwrap();
        std::fs::write(root.join("generated.c"), "int g(void) { return 1; }\n").unwrap();
        std::fs::write(root.join("out/shim.c"), "int s(void) { return 2; }\n").unwrap();
        std::fs::write(root.join("out/ignored.c"), "int i(void) { return 3; }\n").unwrap();

        let listed = crate::package::source_files(root).unwrap();
        assert_eq!(
            listed,
            ["Cargo.toml", "generated.c", "out/shim.c", "src/lib.rs"]
        );
        let packed = Package::pack_directory(root).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(packed)).unwrap();
        assert!(zip.by_name("generated.c").is_ok());
        assert!(zip.by_name("out/shim.c").is_ok());
        assert!(zip.by_name("out/ignored.c").is_err());

        // Oversized root and nested Cargo manifests are refused before reading into memory.
        let huge = tempfile::tempdir().unwrap();
        let root_cargo = std::fs::File::create(huge.path().join("Cargo.toml")).unwrap();
        root_cargo.set_len(MAX_SOURCE_BYTES + 1).unwrap();
        let error = crate::package::source_files(huge.path())
            .unwrap_err()
            .to_string();
        assert!(error.contains("exceeds its entry or size limit"), "{error}");

        std::fs::write(
            huge.path().join("Cargo.toml"),
            "[package]\nname = 'ruby'\n[dependencies]\nhelper = { path = 'crates/helper' }\n",
        )
        .unwrap();
        std::fs::create_dir_all(huge.path().join("crates/helper")).unwrap();
        let nested_cargo =
            std::fs::File::create(huge.path().join("crates/helper/Cargo.toml")).unwrap();
        nested_cargo.set_len(MAX_SOURCE_BYTES + 1).unwrap();
        let error = crate::package::source_files(huge.path())
            .unwrap_err()
            .to_string();
        assert!(error.contains("exceeds its entry or size limit"), "{error}");

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let outside = dir.path().join("outside.toml");
            std::fs::write(&outside, "[package]\nname = 'outside'\n").unwrap();

            let linked_nested = tempfile::tempdir().unwrap();
            std::fs::write(
                linked_nested.path().join("Cargo.toml"),
                "[package]\nname = 'ruby'\n[dependencies]\nhelper = { path = 'crates/helper' }\n",
            )
            .unwrap();
            std::fs::create_dir_all(linked_nested.path().join("crates/helper")).unwrap();
            symlink(
                &outside,
                linked_nested.path().join("crates/helper/Cargo.toml"),
            )
            .unwrap();
            let error = crate::package::source_files(linked_nested.path())
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("crates/helper/Cargo.toml")
                    && error.contains("unsupported file type"),
                "{error}"
            );

            let linked_root = tempfile::tempdir().unwrap();
            symlink(&outside, linked_root.path().join("Cargo.toml")).unwrap();
            let error = crate::package::source_files(linked_root.path())
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("Cargo.toml") && error.contains("unsupported file type"),
                "{error}"
            );
        }
    }
}
