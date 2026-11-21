//! Native mods installed from a GitHub link (docs/modding/src/how-to/native-mods.md, "Install from GitHub"), so an
//! author can ship a version without a registry pull request. The link names a repository (its
//! latest release), a release tag, a branch or tag (`tree/`) or a commit. It is resolved through
//! GitHub's public API to an exact **release asset** (a `.smod`, kept by its SHA-256) or an exact
//! **commit** (the archive of that commit, whose mod folder is packed into a `.smod`), never to a
//! branch that moves. The result passes `sunrise_launcher_smod::index::native::Bundle::read`, the launcher's
//! own read of a native mod, before anything is installed, and the origin is
//! kept beside the file (`sunrise-native-mods/<id>.origin.json`) for "check for updates".
//!
//! Nothing here runs the mod's code, and nothing here trusts it: the registry has not reviewed it. The game's
//! content fence is not run here either: the game runs it when it loads the mod.
//! Callers show [`WARNING`] and ask first (`--trust-native-code`).
use crate::backend;
use crate::i18n::{tr, trf};
use crate::native_mods::{self, Bundle, INSTALLED};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

mod source;
pub(crate) use source::{forward, pack_pasted, pack_pin};
pub use source::{SourceOrigin, SourceTrack};

/// What a player is told before a mod from GitHub is installed.
pub const WARNING: &str = "Not reviewed by the Sunrise registry. Native mods run with full access to this computer. Read the source first.";

/// The most a release asset or a mod's archive may be.
const MAX_ASSET: u64 = 64 << 20;
const MAX_ARCHIVE: u64 = 256 << 20;
/// How deep below the archive's root a mod folder is looked for.
const DEPTH: usize = 6;

/// What a link points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// The repository: its latest release, else its default branch.
    Latest,
    /// The head of the default branch (what a mod installed from a commit is updated to).
    Head,
    /// `releases/tag/<tag>`: that release, else the tag itself.
    Tag(String),
    /// `tree/<ref>`: a branch, tag or commit.
    Ref(String),
    /// `commit/<sha>`.
    Commit(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub owner: String,
    pub repo: String,
    pub target: Target,
}

pub(crate) fn plain(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// A ref (branch, tag, sha) as GitHub names them: no spaces, no `..`, nothing a URL would read.
fn ref_ok(r: &str) -> bool {
    !r.is_empty()
        && !r.contains("..")
        && !r.starts_with('/')
        && !r.ends_with('/')
        && r.bytes().all(|b| {
            b.is_ascii_graphic()
                && !matches!(
                    b,
                    b'?' | b'#' | b'%' | b'\\' | b'*' | b'^' | b'~' | b':' | b'[' | b'@'
                )
        })
}

/// Parses `https://github.com/<owner>/<repo>[/releases/tag/<tag> | /tree/<ref> | /commit/<sha>]`.
pub fn parse(text: &str) -> Result<Link, String> {
    let bad = || {
        format!(
            "{:?} is not a GitHub link: use https://github.com/<owner>/<repo>, or .../releases/tag/<tag>, .../tree/<ref> or .../commit/<sha>.",
            text.trim()
        )
    };
    let text = text.trim();
    let rest = text
        .strip_prefix("https://github.com/")
        .or_else(|| text.strip_prefix("https://www.github.com/"))
        .ok_or_else(bad)?;
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let mut parts = rest.trim_end_matches('/').splitn(3, '/');
    let owner = parts.next().unwrap_or_default();
    let repo = parts.next().unwrap_or_default();
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    if !plain(owner) || !plain(repo) {
        return Err(bad());
    }
    let target = match parts.next() {
        None => Target::Latest,
        Some(tail) => {
            let tail = tail.trim_end_matches('/');
            let value = |prefix: &str| {
                tail.strip_prefix(prefix)
                    .filter(|v| ref_ok(v))
                    .map(str::to_owned)
            };
            if let Some(tag) = value("releases/tag/") {
                Target::Tag(tag)
            } else if let Some(r) = value("tree/") {
                Target::Ref(r)
            } else if let Some(sha) = value("commit/")
                .filter(|s| (7..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                Target::Commit(sha.to_ascii_lowercase())
            } else if matches!(tail, "releases" | "releases/latest") {
                Target::Latest
            } else {
                return Err(bad());
            }
        }
    };
    Ok(Link {
        owner: owner.into(),
        repo: repo.into(),
        target,
    })
}

/// A release's file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    /// The SHA-256 GitHub reports for it (`digest`), when it does: the download must match.
    pub sha256: Option<String>,
}

/// A link resolved to something exact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub owner: String,
    pub repo: String,
    /// The release it came from, if any.
    pub tag: Option<String>,
    /// The full commit: what the source link names, and what a commit install fetches.
    pub commit: String,
    /// The release's `.smod` assets, installed as they are.
    pub assets: Vec<Asset>,
}

impl Resolved {
    pub fn name(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    /// `owner/repo @ tag` for a release, `owner/repo @ 7-digit sha` for a commit.
    pub fn label(&self) -> String {
        label(&self.name(), self.tag.as_deref(), &self.commit)
    }

    /// Where to read exactly what would be installed.
    pub fn source_url(&self) -> String {
        source_url(&self.owner, &self.repo, &self.commit)
    }
}

fn label(name: &str, tag: Option<&str>, commit: &str) -> String {
    match tag {
        Some(tag) => format!("{name} @ {tag}"),
        None => format!("{name} @ {}", &commit[..commit.len().min(7)]),
    }
}

pub fn source_url(owner: &str, repo: &str, commit: &str) -> String {
    format!("https://github.com/{owner}/{repo}/tree/{commit}")
}

/// GitHub's JSON for an API path (`repos/...`); `None` when it answers 404.
pub type Api<'a> = &'a dyn Fn(&str) -> Result<Option<serde_json::Value>, String>;

/// The real API, unauthenticated (`SUNRISE_GITHUB_TOKEN` is used when set, for CI).
pub fn api(path: &str) -> Result<Option<serde_json::Value>, String> {
    #[cfg(debug_assertions)]
    if let Some(result) = crate::browser::sunrise::sources::fixture_link_api(path) {
        return result;
    }
    let url = format!("https://api.github.com/{path}");
    match backend::get_unless_missing(&url) {
        Ok(Some(response)) => response.into_json().map(Some).map_err(|e| e.to_string()),
        Ok(None) => Ok(None),
        Err(e) if e.contains("rate limit") => Err(RATE_LIMIT.into()),
        Err(e) => Err(e),
    }
}

const RATE_LIMIT: &str =
    "GitHub's rate limit for anonymous requests was reached. Try again in a few minutes.";

/// Percent-encodes a ref for a URL path, keeping its `/`.
fn encode(r: &str) -> String {
    r.bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn commit_of(api: Api, link: &Link, reference: &str) -> Result<String, ResolveError> {
    let path = format!(
        "repos/{}/{}/commits/{}",
        link.owner,
        link.repo,
        encode(reference)
    );
    let found = api(&path)?.ok_or_else(|| {
        ResolveError::Lookup(trf(
            "{repo} has no commit, tag or branch {reference}.",
            &[
                ("repo", &format!("{}/{}", link.owner, link.repo)),
                ("reference", reference),
            ],
        ))
    })?;
    found["sha"]
        .as_str()
        .filter(|s| sunrise_launcher_smod::manifest::revision(s))
        .map(str::to_owned)
        .ok_or_else(|| {
            ResolveError::Lookup(trf(
                "GitHub did not name the commit of {reference}.",
                &[("reference", reference)],
            ))
        })
}

fn default_head(api: Api, link: &Link) -> Result<String, ResolveError> {
    let name = format!("{}/{}", link.owner, link.repo);
    let repo = api(&format!("repos/{name}"))?.ok_or_else(|| {
        ResolveError::Lookup(trf(
            "{repo} doesn't exist, or is private.",
            &[("repo", &name)],
        ))
    })?;
    let branch = repo["default_branch"].as_str().ok_or_else(|| {
        ResolveError::Lookup(tr("GitHub did not name the default branch.").into())
    })?;
    commit_of(api, link, branch)
}

/// The release's mod packages, validated after download.
fn assets(release: &serde_json::Value, link: &Link) -> Vec<Asset> {
    let prefix = format!(
        "https://github.com/{}/{}/releases/download/",
        link.owner, link.repo
    );
    let list = release["assets"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    list.iter()
        .filter(|a| a["name"].as_str().is_some_and(|n| n.ends_with(".smod")))
        .filter_map(|a| {
            let url = a["browser_download_url"].as_str()?.to_owned();
            let name = a["name"].as_str()?.to_owned();
            // Only this repository's own release files.
            url.get(..prefix.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(&prefix))
                .then(|| Asset {
                    name,
                    url,
                    sha256: a["digest"]
                        .as_str()
                        .and_then(|d| d.strip_prefix("sha256:"))
                        .filter(|d| d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit()))
                        .map(str::to_ascii_lowercase),
                })
        })
        .collect()
}

fn from_release(
    api: Api,
    link: &Link,
    release: &serde_json::Value,
) -> Result<Resolved, ResolveError> {
    let tag = release["tag_name"]
        .as_str()
        .ok_or_else(|| ResolveError::Lookup(tr("GitHub did not name the release’s tag.").into()))?
        .to_owned();
    Ok(Resolved {
        owner: link.owner.clone(),
        repo: link.repo.clone(),
        commit: commit_of(api, link, &tag)?,
        tag: Some(tag),
        assets: assets(release, link),
    })
}

fn at_commit(commit: String, link: &Link) -> Resolved {
    Resolved {
        owner: link.owner.clone(),
        repo: link.repo.clone(),
        tag: None,
        commit,
        assets: Vec::new(),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ResolveError {
    NoRelease(String),
    Lookup(String),
    Transfer(String),
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoRelease(path) => {
                f.write_str(&trf("GitHub returned 404 for {path}.", &[("path", path)]))
            }
            Self::Lookup(why) | Self::Transfer(why) => f.write_str(why),
        }
    }
}

impl From<String> for ResolveError {
    fn from(why: String) -> Self {
        Self::Transfer(why)
    }
}

impl From<ResolveError> for String {
    fn from(error: ResolveError) -> Self {
        error.to_string()
    }
}

/// Resolves `link` to exact release assets or a commit, through `api`.
pub fn resolve(link: &Link, api: Api) -> Result<Resolved, ResolveError> {
    let base = format!("repos/{}/{}", link.owner, link.repo);
    let release = |path: String| {
        api(&path)?
            .ok_or(ResolveError::NoRelease(path))
            .and_then(|release| from_release(api, link, &release))
    };
    match &link.target {
        Target::Latest => match api(&format!("{base}/releases/latest"))? {
            Some(release) => from_release(api, link, &release),
            None => Ok(at_commit(default_head(api, link)?, link)),
        },
        Target::Tag(tag) => match release(format!("{base}/releases/tags/{}", encode(tag))) {
            Err(ResolveError::NoRelease(path)) => match commit_of(api, link, tag) {
                Ok(commit) => Ok(at_commit(commit, link)),
                Err(ResolveError::Lookup(_)) => Err(ResolveError::NoRelease(path)),
                Err(error) => Err(error),
            },
            result => result,
        },
        Target::Head => Ok(at_commit(default_head(api, link)?, link)),
        Target::Ref(r) | Target::Commit(r) => Ok(at_commit(commit_of(api, link, r)?, link)),
    }
}

/// Where a mod came from, kept beside it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
    pub id: String,
    pub version: String,
    pub owner: String,
    pub repo: String,
    /// The release it came from, when it is a release's asset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    pub commit: String,
    /// The release asset's name, when it came from one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    /// The SHA-256 of the installed `.smod`.
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceOrigin>,
}

impl Origin {
    /// "from GitHub: owner/repo @ tag" (or the commit's first 7 digits).
    pub fn label(&self) -> String {
        label(
            &format!("{}/{}", self.owner, self.repo),
            self.tag.as_deref(),
            &self.commit,
        )
    }

    pub fn source_url(&self) -> String {
        source_url(&self.owner, &self.repo, &self.commit)
    }

    pub fn is_release_asset(&self) -> bool {
        self.source.is_none()
            && self.tag.as_deref().is_some_and(ref_ok)
            && self.asset.as_deref().is_some_and(asset_name)
    }

    pub(crate) fn matches_archive(&self, id: &str, version: &str, digest: &str) -> bool {
        self.id == id
            && self.version == version
            && plain(&self.owner)
            && plain(&self.repo)
            && sunrise_launcher_smod::manifest::revision(&self.commit)
            && self.sha256.eq_ignore_ascii_case(digest)
            && self.source.as_ref().is_none_or(|source| {
                source.validate()
                    && self.asset.is_none()
                    && match source.track {
                        SourceTrack::Release => self.tag.as_deref().is_some_and(ref_ok),
                        SourceTrack::Pinned | SourceTrack::Branch(_) => self.tag.is_none(),
                    }
            })
    }

    /// What its update is read from: the latest release for a mod from a release, the default
    /// branch's head for one from a commit.
    pub fn follows(&self) -> Link {
        Link {
            owner: self.owner.clone(),
            repo: self.repo.clone(),
            target: match self.tag {
                Some(_) => Target::Latest,
                None => Target::Head,
            },
        }
    }
}

/// Whether `latest` (what [`Origin::follows`] resolves to) is newer than what is installed.
pub fn is_update(origin: &Origin, latest: &Resolved) -> bool {
    match (&origin.tag, &latest.tag) {
        (Some(have), Some(new)) => have != new,
        // A release's mod whose repository now has no release asset to follow.
        (Some(_), None) => false,
        (None, _) => origin.commit != latest.commit,
    }
}

/// A mod ready to install: its `.smod` was read and checked, nothing run.
#[derive(Clone, Debug)]
pub struct Packed {
    pub id: String,
    pub version: String,
    pub name: String,
    pub file: PathBuf,
    pub sha256: String,
    checked: Option<(std::sync::Arc<Resolved>, Origin)>,
    _scratch: Option<std::sync::Arc<tempfile::TempDir>>,
}

impl Packed {
    fn origin(&self, found: &Resolved, asset: Option<&Asset>) -> Origin {
        Origin {
            id: self.id.clone(),
            version: self.version.clone(),
            owner: found.owner.clone(),
            repo: found.repo.clone(),
            tag: asset.and(found.tag.clone()),
            commit: found.commit.clone(),
            asset: asset.map(|a| a.name.clone()),
            sha256: self.sha256.clone(),
            source: None,
        }
    }

    pub(crate) fn checked_origin(&self, found: &Resolved) -> Option<&Origin> {
        let (source, origin) = self.checked.as_ref()?;
        (source.as_ref() == found
            && self.id == origin.id
            && self.version == origin.version
            && self.sha256 == origin.sha256)
            .then_some(origin)
    }

    pub(crate) fn release_origin(&self, found: &Resolved) -> Option<&Origin> {
        self.checked_origin(found)
            .filter(|origin| origin.is_release_asset())
    }

    pub(crate) fn copy_verified(&self, dest: &Path) -> Result<(), String> {
        fs::copy(&self.file, dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        if sunrise_launcher_smod::index::sha256_file(dest)? != self.sha256 {
            return Err(trf(
                "The downloaded package for {mod} changed. Fetch it again.",
                &[("mod", &self.id)],
            ));
        }
        Ok(())
    }
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Reads `bytes` as `.smod`, with the registry's checks, into a file `Packed` names.
fn checked(bytes: &[u8], dir: &Path, asset: Option<&Asset>) -> Result<Packed, String> {
    let sum = sha256(bytes);
    if let Some(asset) = asset.filter(|asset| {
        asset
            .sha256
            .as_ref()
            .is_some_and(|expected| *expected != sum)
    }) {
        return Err(format!(
            "{} does not match the checksum GitHub lists for it.",
            asset.name
        ));
    }
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = dir.join(format!("{}.smod", &sum[..16]));
    fs::write(&file, bytes).map_err(|e| format!("{}: {e}", file.display()))?;
    let manifest = sunrise_launcher_smod::index::native::Bundle::read(&file)
        .inspect_err(|_| {
            let _ = fs::remove_file(&file);
        })?
        .manifest;
    Ok(Packed {
        id: manifest.id,
        version: manifest.version,
        name: manifest.name,
        file,
        sha256: sum,
        checked: None,
        _scratch: None,
    })
}

/// Every flat native project under `root`, using the same reader as native-build.
/// A mod's folder is not searched further.
pub fn find_mods(root: &Path) -> Result<Vec<Bundle>, String> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<Bundle>) -> Result<(), String> {
        if let Some(found) = native_mods::bundle_in(dir)? {
            out.push(found);
            return Ok(());
        }
        if depth == 0 {
            return Ok(());
        }
        let mut dirs: Vec<PathBuf> = fs::read_dir(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                !name.starts_with('.') && name != "target" && name != "node_modules"
            })
            .map(|e| e.path())
            .collect();
        dirs.sort();
        for sub in dirs {
            walk(&sub, depth - 1, out)?;
        }
        Ok(())
    }
    let mut found = Vec::new();
    walk(root, DEPTH, &mut found)?;
    found.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    Ok(found)
}

/// Deterministically package a validated flat mod project.
pub fn pack(bundle: &Bundle) -> Result<Vec<u8>, String> {
    sunrise_launcher_smod::Package::pack_directory(&bundle.dir)
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    backend::get(url)?
        .into_reader()
        .take(MAX_ASSET + 1)
        .read_to_end(&mut body)
        .map_err(|e| format!("Download interrupted: {e}"))?;
    Ok(body)
}

fn asset_name(name: &str) -> bool {
    name.ends_with(".smod") && !name.contains(['/', '\\']) && !name.chars().any(char::is_control)
}

#[derive(Debug)]
pub enum FetchError {
    NoMods { release: bool, detail: String },
    Package(String),
    Transfer(String),
}

impl From<FetchError> for String {
    fn from(error: FetchError) -> Self {
        match error {
            FetchError::NoMods { detail, .. }
            | FetchError::Package(detail)
            | FetchError::Transfer(detail) => detail,
        }
    }
}

/// Read all of this exact release's packages, or pack the mods in the resolved commit.
/// Returned packages own their temporary files; nothing is installed or executed here.
pub fn fetch(found: &Resolved) -> Result<Vec<Packed>, String> {
    fetch_checked(found).map_err(String::from)
}

pub fn fetch_checked(found: &Resolved) -> Result<Vec<Packed>, FetchError> {
    fetch_packages(found, download)
}

/// Same checks as the production route, with an injected release transfer for tests.
pub fn fetch_with(
    found: &Resolved,
    download_release: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<Vec<Packed>, String> {
    fetch_packages(found, download_release).map_err(String::from)
}

fn fetch_packages(
    found: &Resolved,
    mut download_release: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<Vec<Packed>, FetchError> {
    if !plain(&found.owner)
        || !plain(&found.repo)
        || !sunrise_launcher_smod::manifest::revision(&found.commit)
    {
        return Err(FetchError::Package(
            tr("Couldn't verify the resolved GitHub repository and commit.").into(),
        ));
    }
    let scratch =
        std::sync::Arc::new(tempfile::tempdir().map_err(|e| FetchError::Package(e.to_string()))?);
    let source = std::sync::Arc::new(found.clone());
    let dir = scratch.path();
    let mut mods = Vec::new();
    if found.assets.is_empty() {
        let url = format!(
            "https://codeload.github.com/{}/{}/tar.gz/{}",
            found.owner, found.repo, found.commit
        );
        let top = unpack_archive(&url, &dir.join("source"), MAX_ARCHIVE)?;
        for bundle in find_mods(&top).map_err(FetchError::Package)? {
            let bytes = pack(&bundle).map_err(FetchError::Package)?;
            let mut package = checked(&bytes, dir, None).map_err(FetchError::Package)?;
            package.checked = Some((source.clone(), package.origin(found, None)));
            mods.push(package);
        }
    } else {
        let mut remaining = MAX_ARCHIVE;
        for asset in &found.assets {
            let prefix = format!("/{}/{}/releases/download/", found.owner, found.repo);
            let from_release = url::Url::parse(&asset.url).is_ok_and(|url| {
                url.scheme() == "https"
                    && url.host_str() == Some("github.com")
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.port().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
                    && url
                        .path()
                        .split_at_checked(prefix.len())
                        .is_some_and(|(head, tail)| {
                            head.eq_ignore_ascii_case(&prefix)
                                && tail
                                    .rsplit_once('/')
                                    .is_some_and(|(tag, name)| !tag.is_empty() && !name.is_empty())
                        })
            });
            if !from_release
                || !asset_name(&asset.name)
                || !found.tag.as_deref().is_some_and(ref_ok)
            {
                return Err(FetchError::Package(
                    tr("Couldn't verify the GitHub release asset.").into(),
                ));
            }
            let bytes = download_release(&asset.url).map_err(FetchError::Transfer)?;
            if bytes.len() as u64 > remaining.min(MAX_ASSET) {
                return Err(FetchError::Package(
                    tr("The GitHub mod download is too large.").into(),
                ));
            }
            remaining -= bytes.len() as u64;
            let mut package = checked(&bytes, dir, Some(asset)).map_err(FetchError::Package)?;
            package.checked = Some((source.clone(), package.origin(found, Some(asset))));
            mods.push(package);
        }
    }
    if mods.is_empty() {
        return Err(FetchError::NoMods {
            release: found.tag.is_some(),
            detail: trf(
                "No Sunrise mod packages were found at {source}.",
                &[("source", &found.label())],
            ),
        });
    }
    mods.sort_by(|a, b| a.id.cmp(&b.id));
    if let Some(pair) = mods.windows(2).find(|pair| pair[0].id == pair[1].id) {
        return Err(FetchError::Package(trf(
            "The GitHub download has more than one package for {mod}.",
            &[("mod", &pair[0].id)],
        )));
    }
    for package in &mut mods {
        package._scratch = Some(scratch.clone());
    }
    Ok(mods)
}

/// Unpack one fresh, bounded tar tree. Never materialize links, duplicate leaves or path aliases.
fn unpack_bounded(
    reader: impl Read,
    partial: &Path,
    limit: u64,
    cancellation: Option<&native_mods::Cancellation>,
) -> Result<PathBuf, String> {
    fs::create_dir_all(partial).map_err(|e| e.to_string())?;
    let mut archive = tar::Archive::new(reader.take(limit + 1));
    let mut remaining = limit;
    let mut root = None::<PathBuf>;
    let mut seen = std::collections::BTreeSet::new();
    let invalid =
        || tr("The GitHub source archive contains an invalid or duplicate path.").to_owned();
    let too_large = || tr("The GitHub mod download is too large.").to_owned();
    for entry in archive.entries().map_err(|e| e.to_string())? {
        if let Some(cancellation) = cancellation {
            cancellation.check()?;
        }
        let mut entry =
            entry.map_err(|e| trf("Source interrupted: {error}", &[("error", &e.to_string())]))?;
        remaining = remaining
            .checked_sub(entry.size().saturating_add(512))
            .ok_or_else(too_large)?;
        let kind = entry.header().entry_type();
        // Ignore links entirely as the legacy importer does; no write may traverse one.
        if !(kind.is_file() || kind.is_dir()) {
            continue;
        }
        let path = entry.path().map_err(|_| invalid())?;
        let path = path.to_str().ok_or_else(invalid)?.trim_end_matches('/');
        let mut parts = path.split('/');
        let top = parts.next().ok_or_else(invalid)?;
        if root.as_ref().is_some_and(|old| old.as_os_str() != top)
            || !path.split('/').all(archive_segment)
            || (kind.is_file() && parts.next().is_none())
            || !seen.insert(path.to_lowercase())
        {
            return Err(invalid());
        }
        root.get_or_insert_with(|| top.into());
        if !entry.unpack_in(partial).map_err(|e| {
            trf(
                "Could not unpack the source: {error}",
                &[("error", &e.to_string())],
            )
        })? {
            return Err(invalid());
        }
    }
    // `Take` alone can make a truncated stream look like ordinary tar EOF. Drain the remaining
    // compressed member through its checksum and detect a limit hit before accepting it.
    let mut reader = archive.into_inner();
    std::io::copy(&mut reader, &mut std::io::sink()).map_err(|e| e.to_string())?;
    if reader.limit() == 0 {
        return Err(too_large());
    }
    root.map(|name| partial.join(name))
        .ok_or_else(|| tr("The source archive is empty.").into())
}

fn archive_segment(part: &str) -> bool {
    let device = part
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    !part.is_empty()
        && !part.ends_with(['.', ' '])
        && !part
            .chars()
            .any(|c| c.is_control() || matches!(c, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        && !matches!(
            device.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        )
        && !["COM", "LPT"].iter().any(|prefix| {
            device.strip_prefix(prefix).is_some_and(|n| {
                matches!(
                    n,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
}

fn unpack_archive(url: &str, partial: &Path, limit: u64) -> Result<PathBuf, FetchError> {
    let response = backend::get(url).map_err(FetchError::Transfer)?;
    unpack_bounded(
        flate2::read::GzDecoder::new(response.into_reader()),
        partial,
        limit,
        None,
    )
    .map_err(FetchError::Package)
}

/// Which of `found` to install: the one named, all of them, or the only one. Several with no
/// choice is an error that lists them.
pub fn choose<'a>(
    found: &'a [Packed],
    id: Option<&str>,
    all: bool,
) -> Result<Vec<&'a Packed>, String> {
    if let Some(id) = id {
        return found
            .iter()
            .find(|p| p.id == id)
            .map(|p| vec![p])
            .ok_or_else(|| format!("No mod {id} there. It holds: {}.", ids(found)));
    }
    match found {
        [one] => Ok(vec![one]),
        _ if all => Ok(found.iter().collect()),
        _ => Err(format!(
            "It holds {} mods: {}. Pick one (--mod ID) or install them all (--all).",
            found.len(),
            ids(found)
        )),
    }
}

fn ids(found: &[Packed]) -> String {
    found
        .iter()
        .map(|p| p.id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn origin_path(home: &Path, id: &str) -> PathBuf {
    home.join(INSTALLED).join(format!("{id}.origin.json"))
}

/// The origin of mod `id` installed in `home`, if it came from GitHub.
pub fn origin(home: &Path, id: &str) -> Option<Origin> {
    serde_json::from_str(&fs::read_to_string(origin_path(home, id)).ok()?).ok()
}

/// Every mod of `home` installed from GitHub.
pub fn origins(home: &Path) -> Vec<Origin> {
    let mut found: Vec<Origin> = fs::read_dir(home.join(INSTALLED))
        .map(|dir| {
            dir.flatten()
                .filter(|e| e.file_name().to_string_lossy().ends_with(".origin.json"))
                .filter_map(|e| serde_json::from_str(&fs::read_to_string(e.path()).ok()?).ok())
                .collect()
        })
        .unwrap_or_default();
    found.sort_by(|a: &Origin, b| a.id.cmp(&b.id));
    found
}

#[cfg(test)]
mod tests;
