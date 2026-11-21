//! Anonymous GitHub inspection and pinned source transfer. No code here runs mod code.
use super::{bounded, digest, readme, safe_path, safe_ref, Source, Tracker, FRESH};
use crate::i18n::tr;

mod pasted;
pub(crate) use pasted::{branch_ahead, pasted_source};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::{Duration, Instant, SystemTime},
};
use sunrise_launcher_smod::manifest::{self, Manifest};
use url::Url;

const MAX_API: u64 = 256 << 10;
const MAX_MANIFEST: u64 = 128 << 10;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_BACKOFF: Duration = Duration::from_secs(60);

/// A failed anonymous inspection, separated from any fit or compilation result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    RepositoryUnavailable,
    RefUnavailable,
    RateLimited { retry_at: Option<SystemTime> },
    NoManifest,
    Offline(String),
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Release {
    tag: String,
    published_at: String,
}

#[derive(Deserialize)]
struct Repository {
    id: u64,
    full_name: String,
    private: bool,
    default_branch: Option<String>,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    published_at: Option<String>,
    draft: bool,
    prerelease: bool,
}

impl GithubRelease {
    fn published(self) -> Result<Release, Failure> {
        let published_at = self
            .published_at
            .filter(|time| chrono::DateTime::parse_from_rfc3339(time).is_ok())
            .ok_or(Failure::RefUnavailable)?;
        if self.draft || !safe_ref(&self.tag_name) {
            return Err(Failure::RefUnavailable);
        }
        Ok(Release {
            tag: self.tag_name,
            published_at,
        })
    }
}

pub(super) fn valid_repo(repo: &str) -> bool {
    let url = format!("https://github.com/{repo}");
    crate::browser::sunrise::github_repo(&url).as_deref() == Some(url.as_str())
}

#[derive(Clone, Copy)]
enum RefKind {
    Head,
    Tag,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Pin {
    sha: String,
    release: Option<Release>,
}

/// The exact commit inspected for one listing, independent of where a branch moves next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceCommit {
    source: Source,
    pin: Pin,
    github_url: String,
}

impl SourceCommit {
    pub fn source(&self) -> &Source {
        &self.source
    }
    pub fn sha(&self) -> &str {
        &self.pin.sha
    }
    pub fn release_tag(&self) -> Option<&str> {
        self.pin.release.as_ref().map(|r| r.tag.as_str())
    }
    pub fn published_at(&self) -> Option<&str> {
        self.pin.release.as_ref().map(|r| r.published_at.as_str())
    }
    pub fn github_url(&self) -> &str {
        &self.github_url
    }
}

#[derive(Clone)]
pub struct Details {
    pub commit: Option<SourceCommit>,
    pub readme: Option<readme::Readme>,
    /// A saved commit and README can still be shown, but cannot authorize a new build.
    pub error: Option<Failure>,
}

#[derive(Clone)]
struct Lookup {
    when: Instant,
    pin: Option<Pin>,
    failure: Option<Failure>,
}

#[derive(Default)]
pub(super) struct State {
    backoff: HashMap<PathBuf, (SystemTime, Option<SystemTime>)>,
    resolves: HashMap<PathBuf, Lookup>,
    manifests: HashMap<(PathBuf, String, String), bool>,
    missing_readmes: HashSet<(PathBuf, String)>,
}

pub(super) static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| Mutex::new(State::default()));

#[cfg(test)]
thread_local! {
    static REQUESTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn test_requests() -> usize {
    REQUESTS.get()
}

pub(super) struct Github {
    pub(super) cache: PathBuf,
    fixture: Option<PathBuf>,
}

impl Github {
    pub(super) fn current() -> Self {
        let fixture = if cfg!(debug_assertions) {
            std::env::var_os("SUNRISE_SOURCE_FIXTURES").map(PathBuf::from)
        } else {
            None
        };
        let cache =
            crate::backend::data_dir()
                .join("mod-browser")
                .join(fixture.as_ref().map_or_else(
                    || "sources".to_owned(),
                    |root| {
                        format!(
                            "sources-fixture-{}",
                            digest(root.as_os_str().as_encoded_bytes())
                        )
                    },
                ));
        Self { cache, fixture }
    }

    pub(super) fn save(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        crate::profile_txn::durable::replace(
            self.cache.parent().expect("source cache parent"),
            path,
            bytes,
        )
    }

    fn ready(&self, state: &mut State, now: SystemTime) -> Result<(), Failure> {
        if let Some(&(until, retry_at)) = state.backoff.get(&self.cache) {
            if now < until {
                return Err(Failure::RateLimited { retry_at });
            }
            state.backoff.remove(&self.cache);
        }
        Ok(())
    }

    fn fetch(
        &self,
        state: &mut State,
        url: &str,
        fixture: &Path,
        max: u64,
    ) -> Result<Option<Vec<u8>>, Failure> {
        self.request(
            state,
            url,
            fixture,
            REQUEST_TIMEOUT,
            |status| match status {
                // GitHub redirects moved repositories. Do not follow a different identity.
                301 | 302 | 307 | 308 | 403 | 410 | 451 => Failure::RepositoryUnavailable,
                _ => unexpected_status(status),
            },
        )?
        .map(|body| bounded(body, max).map_err(other))
        .transpose()
    }

    fn request(
        &self,
        state: &mut State,
        url: &str,
        fixture: &Path,
        timeout: Duration,
        unexpected: impl FnOnce(u16) -> Failure,
    ) -> Result<Option<Box<dyn io::Read>>, Failure> {
        self.ready(state, SystemTime::now())?;
        #[cfg(test)]
        REQUESTS.set(REQUESTS.get() + 1);
        if let Some(root) = &self.fixture {
            return match fs::read_to_string(root.join(fixture.with_extension("status"))) {
                Ok(status) => {
                    let mut lines = status.lines();
                    match lines.next() {
                        Some("offline") => {
                            Err(Failure::Offline(tr("GitHub is unreachable.").into()))
                        }
                        Some(code) => {
                            let get = |name: &str| {
                                lines.clone().find_map(|line| {
                                    let (key, value) = line.split_once(':')?;
                                    key.eq_ignore_ascii_case(name).then(|| value.trim())
                                })
                            };
                            let status = code.parse().map_err(|_| {
                                Failure::Other(tr("Invalid GitHub test response.").into())
                            })?;
                            self.status(state, status, get, unexpected).map(|()| None)
                        }
                        None => Err(Failure::Other(tr("Invalid GitHub test response.").into())),
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    match fs::File::open(root.join(fixture)) {
                        Ok(file) => Ok(Some(Box::new(file))),
                        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                        Err(error) => Err(other(error)),
                    }
                }
                Err(error) => Err(other(error)),
            };
        }
        let response = match crate::net::anonymous_get_no_redirect(url)
            .timeout(timeout)
            .call()
        {
            Ok(response) | Err(ureq::Error::Status(_, response)) => response,
            Err(error) => return Err(Failure::Offline(error.to_string())),
        };
        if response.status() == 200 {
            Ok(Some(response.into_reader()))
        } else {
            self.status(
                state,
                response.status(),
                |name| response.header(name),
                unexpected,
            )
            .map(|()| None)
        }
    }

    fn archive(&self, repo: &str, sha: &str) -> Result<impl io::Read, Failure> {
        let url = format!("https://codeload.github.com/{repo}/tar.gz/{sha}");
        let fixture = PathBuf::from(format!("codeload/{repo}/{sha}.tgz"));
        // Serialize only the request through its response status, not the archive stream.
        // Another GitHub request must see a cooldown before making its own call.
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        self.request(
            &mut state,
            &url,
            &fixture,
            Duration::from_secs(30),
            unexpected_status,
        )?
        .ok_or(Failure::RefUnavailable)
    }

    fn status<'a>(
        &self,
        state: &mut State,
        status: u16,
        header: impl Fn(&str) -> Option<&'a str>,
        unexpected: impl FnOnce(u16) -> Failure,
    ) -> Result<(), Failure> {
        match status {
            404 => Ok(()),
            429 | 403
                if status == 429
                    || header("Retry-After").is_some()
                    || header("X-RateLimit-Remaining") == Some("0") =>
            {
                let now = SystemTime::now();
                let retry_at = retry_time(now, header("Retry-After"), header("X-RateLimit-Reset"));
                state.backoff.insert(
                    self.cache.clone(),
                    (
                        retry_at
                            .filter(|t| *t > now)
                            .unwrap_or(now + DEFAULT_BACKOFF),
                        retry_at,
                    ),
                );
                Err(Failure::RateLimited { retry_at })
            }
            _ => Err(unexpected(status)),
        }
    }

    fn api<T: DeserializeOwned>(
        &self,
        state: &mut State,
        repo: &str,
        tail: &[&str],
        readme_at: Option<&str>,
    ) -> Result<Option<T>, Failure> {
        let mut url = Url::parse("https://api.github.com/").expect("constant URL");
        let mut path = PathBuf::from("api/repos");
        {
            let mut parts = url.path_segments_mut().expect("HTTPS URL");
            parts.pop_if_empty().push("repos");
            for part in repo
                .split('/')
                .chain(tail.iter().flat_map(|p| p.split('/')))
            {
                parts.push(part);
                path.push(part);
            }
        }
        if let Some(sha) = readme_at {
            url.query_pairs_mut().append_pair("ref", sha);
            path.push(sha);
        }
        // Append the extension; a legal repo/ref can itself contain dots.
        path.as_mut_os_string().push(".json");
        self.fetch(state, url.as_str(), &path, MAX_API)?
            .as_deref()
            .map(serde_json::from_slice)
            .transpose()
            .map_err(other)
    }

    pub(super) fn raw(
        &self,
        state: &mut State,
        repo: &str,
        sha: &str,
        path: &str,
        max: u64,
    ) -> Result<Option<Vec<u8>>, Failure> {
        let mut url = Url::parse("https://raw.githubusercontent.com/").expect("constant URL");
        {
            let mut parts = url.path_segments_mut().expect("HTTPS URL");
            parts
                .pop_if_empty()
                .extend(repo.split('/'))
                .push(sha)
                .extend(path.split('/'));
        }
        self.fetch(
            state,
            url.as_str(),
            &Path::new("raw").join(repo).join(sha).join(path),
            max,
        )
    }

    fn repository(&self, state: &mut State, repo: &str) -> Result<Repository, Failure> {
        let shown: Repository = self
            .api(state, repo, &[], None)?
            .ok_or(Failure::RepositoryUnavailable)?;
        if shown.private || shown.id == 0 || !shown.full_name.eq_ignore_ascii_case(repo) {
            return Err(Failure::RepositoryUnavailable);
        }
        Ok(shown)
    }

    fn resolve_ref(&self, state: &mut State, source: &Source) -> Result<Pin, Failure> {
        let repo = source.repo();
        let shown = self.repository(state, repo)?;
        if shown.id != source.repo_id() || shown.full_name != repo {
            return Err(Failure::RepositoryUnavailable);
        }
        let (sha, release) = match source.tracker() {
            Tracker::Branch(name) => (self.ref_commit(state, repo, RefKind::Head, name)?, None),
            Tracker::Release => {
                let latest: GithubRelease = self
                    .api(state, repo, &["releases", "latest"], None)?
                    .ok_or(Failure::RefUnavailable)?;
                if latest.prerelease {
                    return Err(Failure::RefUnavailable);
                }
                let release = latest.published()?;
                let sha = self.ref_commit(state, repo, RefKind::Tag, &release.tag)?;
                (sha, Some(release))
            }
        };
        Ok(Pin {
            sha: sha.ok_or(Failure::RefUnavailable)?,
            release,
        })
    }

    /// `None` is an absent exact ref, not a malformed response or an unprovable object.
    fn ref_commit(
        &self,
        state: &mut State,
        repo: &str,
        kind: RefKind,
        name: &str,
    ) -> Result<Option<String>, Failure> {
        let under = match kind {
            RefKind::Head => "heads",
            RefKind::Tag => "tags",
        };
        #[derive(Deserialize)]
        struct Object {
            sha: String,
            #[serde(rename = "type")]
            kind: String,
        }
        #[derive(Deserialize)]
        struct Ref {
            #[serde(rename = "ref")]
            name: String,
            object: Object,
        }
        let Some(found): Option<Ref> = self.api(state, repo, &["git", "ref", under, name], None)?
        else {
            return Ok(None);
        };
        if found.name != format!("refs/{under}/{name}") {
            return Err(Failure::RefUnavailable);
        }
        let mut object = found.object;
        // A release tag may be annotated. A branch never resolves through a tag object.
        for _ in 0..8 {
            if !manifest::revision(&object.sha) {
                break;
            }
            if object.kind == "commit" {
                return Ok(Some(object.sha));
            }
            if !matches!(kind, RefKind::Tag) || object.kind != "tag" {
                break;
            }
            #[derive(Deserialize)]
            struct Tag {
                object: Object,
            }
            let tag: Tag = self
                .api(state, repo, &["git", "tags", &object.sha], None)?
                .ok_or(Failure::RefUnavailable)?;
            object = tag.object;
        }
        Err(Failure::RefUnavailable)
    }

    fn lookup(&self, state: &mut State, source: &Source, refresh: Option<Instant>) -> Lookup {
        let key = match source.tracker() {
            Tracker::Branch(branch) => {
                format!("{}:{}:heads:{branch}", source.repo_id(), source.repo())
            }
            Tracker::Release => format!("{}:{}:releases", source.repo_id(), source.repo()),
        };
        let path = self
            .cache
            .join(format!("ref-{}.json", digest(key.as_bytes())));
        let last = state.resolves.get(&path);
        if let Some(last) = last.filter(|last| {
            last.when.elapsed() < FRESH
                && refresh.is_none_or(|requested| last.when >= requested)
                && !matches!(last.failure, Some(Failure::RateLimited { .. }))
        }) {
            return last.clone();
        }
        let cached = last.and_then(|last| last.pin.clone());
        let fresh = self.resolve_ref(state, source);
        let (pin, failure) = match fresh {
            Ok(pin) => {
                if let Ok(bytes) = serde_json::to_vec(&pin) {
                    if let Err(error) = self.save(&path, &bytes) {
                        eprintln!("Save tracked source: {error}");
                    }
                }
                (Some(pin), None)
            }
            Err(problem @ (Failure::RepositoryUnavailable | Failure::RefUnavailable)) => {
                (None, Some(problem))
            }
            Err(problem) => {
                let cached = cached.or_else(|| {
                    let bytes = bounded(fs::File::open(&path).ok()?, 4096).ok()?;
                    let pin: Pin = serde_json::from_slice(&bytes).ok()?;
                    (manifest::revision(&pin.sha)
                        && pin.release.as_ref().is_none_or(|release| {
                            safe_ref(&release.tag)
                                && chrono::DateTime::parse_from_rfc3339(&release.published_at)
                                    .is_ok()
                        }))
                    .then_some(pin)
                });
                (cached, Some(problem))
            }
        };
        let lookup = Lookup {
            when: Instant::now(),
            pin,
            failure,
        };
        state.resolves.insert(path, lookup.clone());
        lookup
    }

    fn manifest(&self, state: &mut State, source: &Source, sha: &str) -> Result<(), Failure> {
        let key = (
            self.cache.clone(),
            format!("{}/{sha}/{}", source.repo(), source.manifest()),
            source.id().to_owned(),
        );
        if let Some(&found) = state.manifests.get(&key) {
            return found.then_some(()).ok_or(Failure::NoManifest);
        }
        let found = match self.raw(state, source.repo(), sha, source.manifest(), MAX_MANIFEST)? {
            Some(bytes) => std::str::from_utf8(&bytes)
                .ok()
                .and_then(|text| Manifest::parse(text).ok())
                .is_some_and(|m| m.id == source.id() && m.native.is_some()),
            None => false,
        };
        state.manifests.insert(key, found);
        found.then_some(()).ok_or(Failure::NoManifest)
    }

    fn readme(
        &self,
        state: &mut State,
        source: &Source,
        sha: &str,
        refresh: bool,
    ) -> Result<Option<(String, Vec<u8>)>, Failure> {
        #[derive(Deserialize)]
        struct Metadata {
            path: String,
            size: u64,
            #[serde(rename = "type")]
            kind: String,
        }
        let key = (self.cache.clone(), format!("{}/{sha}", source.repo()));
        if !refresh && state.missing_readmes.contains(&key) {
            return Ok(None);
        }
        let found: Option<Metadata> = self.api(state, source.repo(), &["readme"], Some(sha))?;
        if let Some(found) = found.filter(|found| {
            found.kind == "file"
                && safe_path(&found.path)
                && readme::markdown_file(&found.path)
                && found.size <= readme::MAX_README
        }) {
            if let Some(bytes) =
                self.raw(state, source.repo(), sha, &found.path, readme::MAX_README)?
            {
                return Ok(Some((found.path, bytes)));
            }
        }
        state.missing_readmes.insert(key);
        Ok(None)
    }
}

fn other(error: impl std::fmt::Display) -> Failure {
    Failure::Other(error.to_string())
}

fn unexpected_status(status: u16) -> Failure {
    Failure::Other(crate::i18n::trf(
        "GitHub answered {status}.",
        &[("status", &status.to_string())],
    ))
}

fn retry_time(now: SystemTime, retry: Option<&str>, reset: Option<&str>) -> Option<SystemTime> {
    let unix = |s| SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(s));
    let retry = retry.and_then(|value| {
        value
            .parse()
            .ok()
            .and_then(|seconds| now.checked_add(Duration::from_secs(seconds)))
            .or_else(|| parse_http_date(value).and_then(unix))
    });
    let reset = reset.and_then(|value| value.parse().ok()).and_then(unix);
    retry.or(reset)
}

fn parse_http_date(value: &str) -> Option<u64> {
    u64::try_from(
        chrono::DateTime::parse_from_rfc2822(value)
            .ok()?
            .timestamp(),
    )
    .ok()
}

fn inspect(
    github: &Github,
    source: &Source,
    refresh: Option<Instant>,
    include_readme: bool,
) -> Details {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let Lookup {
        pin, mut failure, ..
    } = github.lookup(&mut state, source, refresh);
    let Some(pin) = pin else {
        return Details {
            commit: None,
            readme: None,
            error: failure,
        };
    };
    if failure.is_none() {
        failure = github
            .ready(&mut state, SystemTime::now())
            .and_then(|()| github.manifest(&mut state, source, &pin.sha))
            .err();
    }
    let readme = if include_readme {
        // Never make extra requests from a saved ref. A missing/invalid manifest is different:
        // the commit is current and its README can still be read.
        let offline = failure
            .as_ref()
            .is_some_and(|failure| *failure != Failure::NoManifest);
        match readme::load(github, source.repo(), &pin.sha, offline, || {
            github.readme(&mut state, source, &pin.sha, refresh.is_some())
        }) {
            Ok(readme) => readme,
            Err(problem @ Failure::RateLimited { .. }) if failure.is_none() => {
                failure = Some(problem);
                None
            }
            Err(_) => None,
        }
    } else {
        None
    };
    let github_url = format!("https://github.com/{}/tree/{}", source.repo(), pin.sha);
    Details {
        commit: Some(SourceCommit {
            source: source.clone(),
            pin,
            github_url,
        }),
        readme,
        error: failure,
    }
}

/// Stream a pinned source archive under the same cooldown as metadata and raw fetches.
pub(crate) fn github_archive(repo: &str, sha: &str) -> Result<impl io::Read, Failure> {
    Github::current().archive(repo, sha)
}

/// The unsigned launcher can drive the ordinary paste form from the same local snapshot
/// fixture as Nightly Discover. Releases still use the ordinary GitHub API.
#[cfg(debug_assertions)]
pub(crate) fn fixture_link_api(path: &str) -> Option<Result<Option<serde_json::Value>, String>> {
    let github = Github::current();
    github.fixture.as_ref()?;
    if !path.starts_with("repos/") || !safe_path(path) {
        return Some(Err(tr("This GitHub link could not be found.").into()));
    }
    let fixture = PathBuf::from(format!("api/{path}.json"));
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    Some(
        github
            .fetch(&mut state, "", &fixture, MAX_API)
            .and_then(|value| {
                value
                    .map(|bytes| serde_json::from_slice(&bytes).map_err(other))
                    .transpose()
            })
            .map_err(|error| format!("{error:?}")),
    )
}

pub fn details(source: &Source, refresh: bool) -> Details {
    inspect(&Github::current(), source, refresh.then(Instant::now), true)
}

/// Check a tracker and its manifest without loading its README or any images.
pub fn resolve(source: &Source, refresh: bool) -> Result<SourceCommit, Failure> {
    resolve_checked(&Github::current(), source, refresh.then(Instant::now))
}

/// One explicit check across many installed mods; repository/ref lookups deduplicate within it.
pub fn resolve_batch(sources: &[Source]) -> Vec<Result<SourceCommit, Failure>> {
    let github = Github::current();
    let requested = Some(Instant::now());
    sources
        .iter()
        .map(|source| resolve_checked(&github, source, requested))
        .collect()
}

fn resolve_checked(
    github: &Github,
    source: &Source,
    requested: Option<Instant>,
) -> Result<SourceCommit, Failure> {
    let details = inspect(github, source, requested, false);
    details
        .error
        .map_or_else(|| details.commit.ok_or(Failure::RefUnavailable), Err)
}

#[cfg(test)]
mod tests;
