//! Nightly listings describe unreviewed source. The catalog never contacts GitHub's API;
//! only explicit detail and build requests resolve source, and only to immutable commits.
use crate::i18n::{tr, trf};
use serde::{Deserialize, Deserializer};
use std::{
    collections::HashSet,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};
use sunrise_launcher_smod::{index::Collection, manifest};

mod github;
#[cfg(debug_assertions)]
pub(crate) use github::fixture_link_api;
pub(crate) use github::{branch_ahead, github_archive, pasted_source};
pub use github::{
    details, resolve, resolve_batch, resolve_each, Details, Failure, IconMisses, SourceCommit,
    SourceIcon,
};

/// Proven origin of a manual GitHub source install, separate from release assets.
pub(crate) struct PastedPin {
    pub(crate) repo: String,
    pub(crate) repo_id: u64,
    pub(crate) track: crate::github_mods::SourceTrack,
    pub(crate) published_at: Option<String>,
    pub(crate) tag: Option<String>,
}

const FRESH: Duration = Duration::from_secs(600);
const MAX_CATALOG: u64 = 2 << 20;

/// A validated listing. Its tracker is a hint for discovery, not evidence of fit or authority.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(try_from = "WireSource")]
pub struct Source {
    id: String,
    name: String,
    summary: String,
    repo: String,
    repo_id: u64,
    manifest: String,
    tracker: Tracker,
    icon: Option<String>,
}

/// A listing as published. Fields added to the feed later are ignored, so a newer list never
/// stops an older launcher from reading the fields it knows.
#[derive(Deserialize)]
struct WireSource {
    id: String,
    name: String,
    summary: String,
    repo: String,
    repo_id: u64,
    manifest: String,
    #[serde(deserialize_with = "tracker")]
    track: Tracker,
    icon: Option<String>,
}

impl TryFrom<WireSource> for Source {
    type Error = String;

    fn try_from(wire: WireSource) -> Result<Self, String> {
        let plain = |text: &str, limit| {
            !text.trim().is_empty()
                && text.chars().count() <= limit
                && !text.chars().any(char::is_control)
        };
        if !plain(&wire.name, 80) || !plain(&wire.summary, 160) {
            return Err(trf(
                "Invalid Nightly source listing: {id}",
                &[("id", &wire.id)],
            ));
        }
        let source = Self {
            id: wire.id,
            name: wire.name,
            summary: wire.summary,
            repo: wire.repo,
            repo_id: wire.repo_id,
            manifest: wire.manifest,
            tracker: wire.track,
            icon: wire.icon,
        };
        source.validate_identity()?;
        Ok(source)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Tracker {
    Branch(String),
    Release,
}

fn tracker<'de, D: Deserializer<'de>>(input: D) -> Result<Tracker, D::Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Branch {
        branch: String,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Release {
        release: bool,
    }
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Wire {
        Branch(Branch),
        Release(Release),
    }
    match Wire::deserialize(input)? {
        Wire::Branch(branch) => Ok(Tracker::Branch(branch.branch)),
        Wire::Release(Release { release: true }) => Ok(Tracker::Release),
        Wire::Release(_) => Err(serde::de::Error::custom("release must be true")),
    }
}

impl Source {
    /// Reconstitute a tracked installed origin, even if no listing still names it.
    pub fn for_origin(
        id: &str,
        repo: &str,
        repo_id: u64,
        manifest: &str,
        tracker: Tracker,
    ) -> Result<Self, Failure> {
        let source = Self {
            id: id.into(),
            name: id.into(),
            summary: String::new(),
            repo: repo.into(),
            repo_id,
            manifest: manifest.into(),
            tracker,
            icon: None,
        };
        source.validate_identity().map_err(Failure::Other)?;
        Ok(source)
    }

    fn validate_identity(&self) -> Result<(), String> {
        if !manifest::valid_mod_id(&self.id)
            || self.repo_id == 0
            || !github::valid_repo(&self.repo)
            || !safe_path(&self.manifest)
            || self.manifest.rsplit('/').next() != Some(manifest::MANIFEST)
            || self
                .icon
                .as_ref()
                .is_some_and(|path| !path.starts_with("icons/") || !safe_path(path))
            || matches!(&self.tracker, Tracker::Branch(branch) if !safe_ref(branch) || branch.starts_with('-'))
        {
            return Err(trf(
                "Invalid Nightly source listing: {id}",
                &[("id", &self.id)],
            ));
        }
        Ok(())
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn summary(&self) -> &str {
        &self.summary
    }
    pub fn repo(&self) -> &str {
        &self.repo
    }
    pub fn repo_id(&self) -> u64 {
        self.repo_id
    }
    pub fn owner(&self) -> &str {
        self.repo.split_once('/').expect("validated owner/repo").0
    }
    pub fn manifest(&self) -> &str {
        &self.manifest
    }
    pub fn tracker(&self) -> &Tracker {
        &self.tracker
    }
    fn icon(&self) -> Option<&str> {
        self.icon.as_deref()
    }
}

fn safe_path(path: &str) -> bool {
    !path.contains(['\\', '%', ':', '?', '#'])
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|part| !matches!(part, "" | "." | "..") && part.trim() == part)
}

fn safe_ref(name: &str) -> bool {
    safe_path(name)
        && name != "@"
        && !name.contains("..")
        && !name.ends_with('.')
        && name
            .bytes()
            .all(|c| c.is_ascii_graphic() && !matches!(c, b'~' | b'^' | b'*' | b'['))
        && !name.contains("@{")
        && name
            .split('/')
            .all(|part| !part.starts_with('.') && !part.ends_with(".lock"))
}

/// The parsed list: its sources, and the collections whose every member it lists.
#[derive(Clone, Default)]
struct Feed {
    sources: Arc<[Source]>,
    collections: Arc<[Collection]>,
}

fn parse(bytes: &[u8]) -> Result<Feed, String> {
    #[derive(Deserialize)]
    struct Wire {
        format: u32,
        sources: Vec<Source>,
        #[serde(default)]
        collections: Vec<Collection>,
    }
    if bytes.len() as u64 > MAX_CATALOG {
        return Err(tr("Nightly list is larger than 2 MiB.").into());
    }
    let Wire {
        format,
        mut sources,
        mut collections,
    } = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if format != 1 {
        return Err(tr("This Nightly list format is not supported.").into());
    }
    let mut paths = HashSet::new();
    sources.sort_unstable_by(|a, b| a.id.cmp(&b.id));
    let mut last = None;
    for source in &sources {
        if last == Some(source.id()) || !paths.insert((source.repo_id, source.manifest())) {
            return Err(trf(
                "Duplicate Nightly source listing: {id}",
                &[("id", &source.id)],
            ));
        }
        last = Some(source.id());
    }
    // A collection shows only when every member is listed once, and its icon is one of the
    // list's own Pages files.
    let mut ids = HashSet::new();
    collections.retain(|collection| {
        let mut members = HashSet::new();
        ids.insert(collection.id.clone())
            && !collection.members.is_empty()
            && collection.icon.starts_with("icons/")
            && safe_path(&collection.icon)
            && collection.members.iter().all(|member| {
                members.insert(&member.id)
                    && sources
                        .binary_search_by(|source| source.id.as_str().cmp(&member.id))
                        .is_ok()
            })
    });
    Ok(Feed {
        sources: sources.into(),
        collections: collections.into(),
    })
}

#[derive(Clone)]
pub struct Catalogue {
    pub sources: Arc<[Source]>,
    pub collections: Arc<[Collection]>,
    pub saved_at: Option<SystemTime>,
    pub checked_at: SystemTime,
    /// Failure of the latest check. An offline snapshot cannot authorize a new build.
    pub error: Option<String>,
}

/// Persist the validator and timestamp with the bytes; a bad response updates neither.
/// One newline-delimited header keeps the original JSON within its own size bound.
#[derive(serde::Serialize, Deserialize)]
struct Header {
    saved_at: SystemTime,
    etag: Option<String>,
}

struct Saved {
    header: Header,
    json: Vec<u8>,
    feed: Feed,
}

fn bounded(mut body: impl Read, max: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    body.by_ref().take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "response too large",
        ));
    }
    Ok(bytes)
}

fn etag(value: &str) -> bool {
    value.len() <= 512 && !value.is_empty() && value.bytes().all(|b| b.is_ascii_graphic())
}

impl Saved {
    fn read(path: &Path) -> Option<Self> {
        let bytes = bounded(fs::File::open(path).ok()?, MAX_CATALOG + 1024).ok()?;
        let split = bytes.iter().position(|&b| b == b'\n')?;
        let header: Header = serde_json::from_slice(&bytes[..split]).ok()?;
        let json = &bytes[split + 1..];
        let feed = parse(json).ok()?;
        Some(Self {
            header,
            feed,
            json: json.into(),
        })
    }

    fn write(&self, path: &Path) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(&self.header).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        bytes.extend(&self.json);
        // The target is under `mod-browser`; flush that directory and its parent.
        crate::profile_txn::durable::replace(
            path.parent()
                .and_then(Path::parent)
                .expect("mod-browser cache path"),
            path,
            &bytes,
        )
        .map_err(|e| e.to_string())
    }
}

enum Reply {
    Unchanged,
    New(Vec<u8>, Option<String>),
}

fn check_catalogue(
    path: &Path,
    saved: Option<Saved>,
    now: SystemTime,
    fetch: impl FnOnce(Option<&str>) -> Result<Reply, String>,
) -> (Option<Saved>, Catalogue) {
    let cached_etag = saved
        .as_ref()
        .and_then(|s| s.header.etag.as_deref())
        .filter(|tag| etag(tag));
    let fresh = fetch(cached_etag).and_then(|reply| match reply {
        Reply::Unchanged => saved
            .as_ref()
            .filter(|_| cached_etag.is_some())
            .map(|saved| Saved {
                header: Header {
                    saved_at: now,
                    etag: saved.header.etag.clone(),
                },
                json: saved.json.clone(),
                feed: saved.feed.clone(),
            })
            .ok_or_else(|| tr("The Nightly list had no saved copy to refresh.").into()),
        Reply::New(json, etag) => Ok(Saved {
            header: Header {
                saved_at: now,
                etag,
            },
            feed: parse(&json)?,
            json,
        }),
    });
    let (saved, error) = match fresh {
        Ok(fresh) => {
            if let Err(error) = fresh.write(path) {
                crate::elog!("Nightly catalogue cache: {error}");
            }
            (Some(fresh), None)
        }
        Err(error) => (saved, Some(error)),
    };
    let feed = saved.as_ref().map(|s| s.feed.clone()).unwrap_or_default();
    let view = Catalogue {
        sources: feed.sources,
        collections: feed.collections,
        saved_at: saved.as_ref().map(|s| s.header.saved_at),
        checked_at: now,
        error,
    };
    (saved, view)
}

struct Checked {
    path: PathBuf,
    when: Instant,
    saved: Option<Saved>,
    view: Catalogue,
}

static CHECKED: Mutex<Option<Checked>> = Mutex::new(None);

/// Call in a worker on Discover open or Retry. No listing ever resolves a tracker here.
pub fn catalogue(refresh: bool) -> Catalogue {
    let local = if cfg!(debug_assertions) {
        std::env::var_os("SUNRISE_MOD_SOURCES").map(PathBuf::from)
    } else {
        None
    };
    let cache = crate::backend::data_dir().join("mod-browser");
    let path = cache.join(local.as_deref().map_or_else(
        || "sunrise-sources".to_owned(),
        |path| {
            format!(
                "sunrise-sources-fixture-{}",
                digest(path.as_os_str().as_encoded_bytes())
            )
        },
    ));
    let mut last = CHECKED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(last) = last
        .as_ref()
        .filter(|last| last.path == path && !refresh && last.when.elapsed() < FRESH)
    {
        return last.view.clone();
    }
    let saved = last
        .take()
        .filter(|last| last.path == path)
        .and_then(|last| last.saved)
        .or_else(|| Saved::read(&path));
    let (saved, view) = check_catalogue(&path, saved, SystemTime::now(), |tag| {
        if let Some(path) = local {
            bounded(
                fs::File::open(path).map_err(|e| e.to_string())?,
                MAX_CATALOG,
            )
            .map(|bytes| Reply::New(bytes, None))
            .map_err(|e| e.to_string())
        } else {
            fetch_catalogue(tag)
        }
    });
    *last = Some(Checked {
        path,
        when: Instant::now(),
        saved,
        view: view.clone(),
    });
    view
}

fn fetch_catalogue(tag: Option<&str>) -> Result<Reply, String> {
    let mut request =
        crate::net::anonymous_get_no_redirect(sunrise_launcher_addresses::MOD_SOURCES)?;
    if let Some(tag) = tag {
        request = request.set("If-None-Match", tag);
    }
    let response = request.call().map_err(|e| e.to_string())?;
    match response.status() {
        304 => Ok(Reply::Unchanged),
        200 => {
            let tag = response
                .header("ETag")
                .filter(|tag| etag(tag))
                .map(str::to_owned);
            let bytes = bounded(response.into_reader(), MAX_CATALOG).map_err(|e| e.to_string())?;
            Ok(Reply::New(bytes, tag))
        }
        _ => Err(tr("The Nightly list did not return a catalog.").into()),
    }
}

fn digest(bytes: &[u8]) -> String {
    sunrise_launcher_release_verify::sha256_hex(bytes)
}

#[cfg(test)]
mod tests;
