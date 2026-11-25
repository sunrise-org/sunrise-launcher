//! Signed v2 mod registry catalogue reader (`v2/index.signed`: the minisign signature's four lines,
//! then `index.json` exactly, published as one file so the two can never come from different
//! publishes).
//!
//! Verifies the minisign signature over the exact fetched (or cached) bytes against the built-in
//! registry key list before parsing, persists the last verified copy in its own cache namespace
//! (`mod-browser/registry-v2/`), refuses any lower `serial` across both network and cached copies,
//! warns without failing on a stale `generated` timestamp, and rejects malformed payloads whole
//! with no salvage.
use crate::i18n::{tr, trf};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use sunrise_launcher_smod::index_v2::{self, Index, VerifyError};

/// Where the signed v2 registry publishes its index (GitHub Pages of sunrise-org/sunrise-mods).
pub const INDEX_URL: &str = sunrise_launcher_addresses::MOD_INDEX_V2;

/// Environment variable overriding [`INDEX_URL`] (HTTPS URL, local path, or debug loopback URL).
pub const ENV_OVERRIDE: &str = "SUNRISE_MOD_INDEX_V2";

/// How long an in-memory verified v2 index is reused before fetching again.
const FRESH: Duration = Duration::from_secs(600);

/// Dedicated v2 cache directory under the game home (isolated from v1's `sunrise-index.json`).
const CACHE_DIR: &str = "mod-browser/registry-v2";
const CACHE_INDEX: &str = "index.json";
const CACHE_SIG: &str = "index.json.minisig";
const STATE_FILE: &str = "state.json";

/// The registry's index signing key, published as `registry/index-key.pub` in
/// sunrise-org/sunrise-mods.
pub const PRODUCTION_KEY: &str = "RWTNJaFJ4AekHG/RgVrgmJpvy9B2EaeqbdSNuseKUx2XPxaOZkChLV0q";

/// Built-in minisign public keys trusted to sign the v2 mod registry catalogue.
#[cfg(not(test))]
pub const REGISTRY_KEYS: &[&str] = &[PRODUCTION_KEY];

/// Disposable test-only minisign public key; never compiled into non-test builds.
#[cfg(test)]
pub const REGISTRY_KEYS: &[&str] = &["RWSM0IpnOTNhKGM+MxjUjuctYPUzgBRDpKUFyXXIpJH6gHwgkpEL3mN8"];

/// A verified v2 registry catalogue loaded from the network or from the verified disk cache.
#[derive(Clone, Debug)]
pub struct Catalog {
    pub index: Arc<Index>,
    /// True when loaded from the verified last-good disk cache because the live fetch failed.
    pub from_cache: bool,
    /// Non-blocking warning when `index.generated` is stale; never prevents using the catalogue.
    pub stale_warning: Option<String>,
}

/// Why loading the signed v2 registry catalogue failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// No registry public key is configured in this build.
    KeyNotConfigured,
    /// Fetching `index.signed` failed and no valid cached copy could be used.
    Fetch(String),
    /// The minisign signature over `index.json` failed verification.
    BadSignature(String),
    /// The signed `index.json` payload was malformed or violated schema-2 rules (no salvage).
    Malformed(String),
    /// The verified index carried a lower `serial` than the highest previously accepted serial.
    Rollback { got: u64, highest: u64 },
    /// The persisted rollback state file exists on disk but could not be read or parsed.
    CorruptState(String),
}

impl From<VerifyError> for LoadError {
    fn from(error: VerifyError) -> Self {
        match error {
            VerifyError::KeyNotConfigured => Self::KeyNotConfigured,
            VerifyError::BadSignature(reason) => Self::BadSignature(reason),
            VerifyError::Malformed(reason) => Self::Malformed(reason),
        }
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KeyNotConfigured => f.write_str(tr("Sunrise mod registry key not configured.")),
            Self::Fetch(reason) => f.write_str(reason),
            Self::BadSignature(_) => {
                f.write_str(tr("The mod registry index signature could not be verified."))
            }
            Self::Malformed(reason) => f.write_str(&trf(
                "The mod registry index is malformed: {reason}",
                &[("reason", reason)],
            )),
            Self::Rollback { got, highest } => f.write_str(&trf(
                "The mod registry returned an older index (serial {got}, expected at least {highest}).",
                &[("got", &got.to_string()), ("highest", &highest.to_string())],
            )),
            Self::CorruptState(reason) => f.write_str(&trf(
                "The mod registry state file could not be read: {reason}",
                &[("reason", reason)],
            )),
        }
    }
}

impl std::error::Error for LoadError {}

#[derive(Debug, Serialize)]
struct PersistedState {
    highest_serial: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    active: Option<ActiveCache>,
}

/// `state.json` as written. The serial floor is the part that guards against rollback, so it
/// must parse; the snapshot pointer is only a cache, read separately.
#[derive(Deserialize)]
struct StoredState {
    highest_serial: u64,
    #[serde(default)]
    active: Option<serde_json::Value>,
}

/// The committed snapshot, by its ID (`snapshot-<serial>-<16 hex digits>`); both file names are
/// derived from it, so a damaged state file can never name another file in the cache.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ActiveCache {
    snapshot: String,
}

impl ActiveCache {
    fn index_name(&self) -> String {
        format!("{}.json", self.snapshot)
    }

    fn signature_name(&self) -> String {
        format!("{}.json.minisig", self.snapshot)
    }
}

fn valid_snapshot_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("snapshot-") else {
        return false;
    };
    let Some((serial, nonce)) = rest.split_once('-') else {
        return false;
    };
    !serial.is_empty()
        && serial.bytes().all(|b| b.is_ascii_digit())
        && nonce.len() == 16
        && nonce
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PublishStep {
    Index,
    Signature,
    State,
}

struct LoadedState {
    at: Instant,
    source: String,
    catalog: Catalog,
    highest_serial: u64,
}

struct RefreshContext<'a> {
    cache_dir: &'a Path,
    keys: &'a [&'a str],
    source: &'a str,
    force_refresh: bool,
    now_unix: i64,
}

static LOADED: Mutex<Option<LoadedState>> = Mutex::new(None);
/// The verified snapshot on disk, read by [`read_saved_index`] while nothing is loaded yet.
static SAVED: Mutex<Option<Catalog>> = Mutex::new(None);
/// Serializes on-disk serial-floor checks and cache publication across stores.
/// Lock order is always `LOADED` (when held) before `COMMIT_LOCK`.
static COMMIT_LOCK: Mutex<()> = Mutex::new(());

/// Where the v2 index comes from: `SUNRISE_MOD_INDEX_V2` when set and non-empty, else
/// [`INDEX_URL`].
pub fn location() -> String {
    std::env::var(ENV_OVERRIDE)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| INDEX_URL.to_owned())
}

/// Splits `index.signed` after its fourth line: the minisign signature file, then the index bytes.
fn split_signed(mut bytes: Vec<u8>) -> Result<(Vec<u8>, Vec<u8>), String> {
    let end = bytes
        .iter()
        .enumerate()
        .filter(|&(_, &b)| b == b'\n')
        .nth(3)
        .map(|(i, _)| i + 1)
        .ok_or_else(|| tr("The mod registry file has no signature.").to_owned())?;
    let index = bytes.split_off(end);
    Ok((index, bytes))
}

/// The v2 catalogue cache folder under the game home.
pub fn cache_dir() -> PathBuf {
    crate::backend::game_home().join(CACHE_DIR)
}

/// Returns the last verified v2 catalogue held in memory without network or disk I/O.
pub fn known_index() -> Option<Catalog> {
    LOADED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|state| state.catalog.clone())
}

/// Reads the last verified catalogue on disk, when none is loaded this run, so views can show it
/// before the network read lands. Disk I/O: call it off the window thread.
pub fn read_saved_index() -> Option<Catalog> {
    if let Some(catalog) = known_index() {
        return Some(catalog);
    }
    let saved = Store::new(cache_dir(), REGISTRY_KEYS)
        .load_cached(now_unix_seconds())
        .ok()?;
    *SAVED.lock().unwrap_or_else(|e| e.into_inner()) = Some(saved.clone());
    Some(saved)
}

/// The catalogue to show without I/O: the one loaded this run, else the snapshot
/// [`read_saved_index`] read from disk.
pub fn shown_index() -> Option<Catalog> {
    known_index().or_else(|| SAVED.lock().unwrap_or_else(|e| e.into_inner()).clone())
}

/// The project an installed registry row belongs to. A row from the old registry (no `mod_id`)
/// belongs to the one project with a release of the same mod, version and archive hash, as
/// `classify_v2_ownership` adopts it; otherwise it has none.
pub fn installed_project<'a>(
    index: &'a Index,
    entry: &crate::browser::installed::Entry,
) -> Option<&'a index_v2::Project> {
    if entry.mod_id.is_some() {
        return index.project(&entry.project);
    }
    let hash = entry.checksum.strip_prefix("SHA-256:")?;
    match index
        .releases_by_sha256(&entry.project, hash)
        .into_iter()
        .filter(|(_, release)| release.version == entry.version)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [(project, _)] => Some(project),
        _ => None,
    }
}

/// The newest release of `project` newer than the `installed` version that fits `build`: not
/// taken down and not already proved not to fit (`unfit`, lowercase SHA-256s).
pub fn newer_release<'a>(
    index: &Index,
    project: &'a index_v2::Project,
    installed: &str,
    build: &str,
    unfit: &std::collections::BTreeSet<String>,
) -> Option<&'a index_v2::Release> {
    use sunrise_launcher_smod::dependencies::parse_version;
    let installed = parse_version(installed).ok()?;
    project
        .releases
        .iter()
        .filter(|r| {
            r.fits_build(build)
                && index.revoked(&project.id, Some(&r.file.sha256)).is_none()
                && !unfit.contains(&r.file.sha256.to_ascii_lowercase())
        })
        .filter_map(|r| Some((parse_version(&r.version).ok()?, r)))
        .filter(|(v, _)| *v > installed)
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, r)| r)
}

/// Returns the verified v2 catalogue, reusing an in-memory copy from the last ten minutes for the
/// same endpoint, otherwise fetching and verifying against [`REGISTRY_KEYS`].
/// The verified catalogue's index, when one can be read.
pub fn current_index() -> Option<Arc<Index>> {
    index().ok().map(|catalog| catalog.index)
}

pub fn index() -> Result<Catalog, LoadError> {
    load_process_index(false)
}

/// Forces a fresh fetch and verification of the v2 catalogue against [`REGISTRY_KEYS`].
pub fn refreshed_index() -> Result<Catalog, LoadError> {
    load_process_index(true)
}

/// The verified catalogue an install, update or provider switch plans against: the in-memory copy
/// while it is fresh, else a new fetch. When that fetch fails for any reason, the last verified
/// copy (in memory, then on disk) serves instead, so a passing registry or network fault never
/// blocks a change Discover already showed.
pub fn change_index() -> Result<Catalog, LoadError> {
    let source = location();
    let cache = cache_dir();
    let ctx = RefreshContext {
        cache_dir: &cache,
        keys: REGISTRY_KEYS,
        source: &source,
        force_refresh: false,
        now_unix: now_unix_seconds(),
    };
    change_index_in(&LOADED, &ctx, fetch_raw)
}

fn change_index_in<F>(
    loaded: &Mutex<Option<LoadedState>>,
    ctx: &RefreshContext<'_>,
    fetch: F,
) -> Result<Catalog, LoadError>
where
    F: FnOnce(&str) -> Result<(Vec<u8>, Vec<u8>), String>,
{
    refresh_in(loaded, ctx, fetch).or_else(|error| {
        let known = loaded
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .filter(|state| state.source == ctx.source)
            .map(|state| state.catalog.clone());
        match known {
            Some(catalog) => Ok(Catalog {
                from_cache: true,
                ..catalog
            }),
            None => Store::new(ctx.cache_dir.to_path_buf(), ctx.keys)
                .load_cached(ctx.now_unix)
                .map_err(|_| error),
        }
    })
}

fn load_process_index(force_refresh: bool) -> Result<Catalog, LoadError> {
    let source = location();
    let cache = cache_dir();
    let ctx = RefreshContext {
        cache_dir: &cache,
        keys: REGISTRY_KEYS,
        source: &source,
        force_refresh,
        now_unix: now_unix_seconds(),
    };
    refresh_in(&LOADED, &ctx, fetch_raw)
}

fn refresh_in<F>(
    loaded: &Mutex<Option<LoadedState>>,
    ctx: &RefreshContext<'_>,
    fetch: F,
) -> Result<Catalog, LoadError>
where
    F: FnOnce(&str) -> Result<(Vec<u8>, Vec<u8>), String>,
{
    if !has_configured_key(ctx.keys) {
        return Err(LoadError::KeyNotConfigured);
    }
    {
        let guard = loaded.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = guard.as_ref() {
            if !ctx.force_refresh && state.source == ctx.source && state.at.elapsed() < FRESH {
                return Ok(state.catalog.clone());
            }
        }
    }

    // Keep network and file I/O outside the lock so concurrent refreshes overlap freely.
    let fetched = fetch(ctx.source);

    // Serialize the serial-floor check, disk commit, and in-memory update together so a slower
    // response can never overwrite a newer catalogue on disk.
    let mut guard = loaded.lock().unwrap_or_else(|e| e.into_inner());
    let prev_highest = guard.as_ref().map(|state| state.highest_serial);
    let mut store = Store::with_highest_serial(ctx.cache_dir.to_path_buf(), ctx.keys, prev_highest);
    let catalog = store.load_from_fetch(fetched, ctx.now_unix)?;
    let highest_serial = guard.as_ref().map_or(catalog.index.serial, |current| {
        current.highest_serial.max(catalog.index.serial)
    });
    *guard = Some(LoadedState {
        at: Instant::now(),
        source: ctx.source.to_owned(),
        catalog: catalog.clone(),
        highest_serial,
    });
    Ok(catalog)
}

/// Self-contained v2 catalogue verifier, cache, and rollback-state store.
///
/// Owns a dedicated cache directory and trusted key list, making verification and rollback checks
/// independent of process-global state.
pub struct Store {
    cache_dir: PathBuf,
    keys: Vec<String>,
    highest_serial: Option<u64>,
}

impl Store {
    /// Creates a store rooted at `cache_dir` trusting `keys`.
    pub fn new(cache_dir: PathBuf, keys: &[&str]) -> Self {
        Self::with_highest_serial(cache_dir, keys, None)
    }

    /// Creates a store seeded with an in-memory `highest_serial` floor.
    pub fn with_highest_serial(
        cache_dir: PathBuf,
        keys: &[&str],
        highest_serial: Option<u64>,
    ) -> Self {
        Self {
            cache_dir,
            keys: keys.iter().map(|key| (*key).to_owned()).collect(),
            highest_serial,
        }
    }

    /// Returns the highest accepted serial tracked in memory or on disk.
    pub fn highest_serial(&self) -> Result<Option<u64>, LoadError> {
        let disk_serial = self.read_state()?.map(|state| state.highest_serial);
        Ok(combine_serial(self.highest_serial, disk_serial))
    }

    /// Fetches `index.signed` from `source`, verifies the signature and
    /// serial, caches the verified copy, and falls back to the verified cache only on fetch error.
    pub fn fetch_and_load(&mut self, source: &str, now_unix: i64) -> Result<Catalog, LoadError> {
        self.require_keys()?;
        let fetched = fetch_raw(source);
        self.load_from_fetch(fetched, now_unix)
    }

    /// Applies a completed fetch result against this store's keys, rollback state, and cache.
    ///
    /// Verification, invalid signature encoding, malformed-payload, and rollback errors on a
    /// fetched response are returned directly and never masked by a cached copy. Only a fetch
    /// failure falls back to the verified last-good cache, which is still signature- and
    /// serial-checked.
    pub fn load_from_fetch(
        &mut self,
        fetched: Result<(Vec<u8>, Vec<u8>), String>,
        now_unix: i64,
    ) -> Result<Catalog, LoadError> {
        self.require_keys()?;
        match fetched {
            Ok((bytes, sig_bytes)) => self.accept_raw_bytes(&bytes, &sig_bytes, now_unix),
            Err(fetch_error) => match self.load_cached(now_unix) {
                Ok(catalog) => Ok(catalog),
                Err(
                    error @ (LoadError::KeyNotConfigured
                    | LoadError::Rollback { .. }
                    | LoadError::CorruptState(_)),
                ) => Err(error),
                Err(_) => Err(LoadError::Fetch(fetch_error)),
            },
        }
    }

    /// Verifies exact `bytes` and raw `signature_bytes` (returning [`LoadError::BadSignature`] if
    /// the signature is not valid UTF-8), enforces monotonic `serial`, writes the verified copy
    /// and updated serial atomically to the v2 cache namespace, and returns the accepted catalogue.
    pub fn accept_raw_bytes(
        &mut self,
        bytes: &[u8],
        signature_bytes: &[u8],
        now_unix: i64,
    ) -> Result<Catalog, LoadError> {
        self.require_keys()?;
        let signature = std::str::from_utf8(signature_bytes)
            .map_err(|e| LoadError::BadSignature(format!("invalid UTF-8 signature: {e}")))?;
        self.accept_bytes(bytes, signature, now_unix)
    }

    /// Verifies exact `bytes` and `signature`, enforces monotonic `serial`, publishes the verified
    /// index, signature, and serial together in the v2 cache namespace, and returns the accepted
    /// catalogue.
    pub fn accept_bytes(
        &mut self,
        bytes: &[u8],
        signature: &str,
        now_unix: i64,
    ) -> Result<Catalog, LoadError> {
        self.accept_bytes_inner(bytes, signature, now_unix, |_, path, data| {
            write_atomic(path, data)
        })
    }

    #[cfg(test)]
    fn accept_bytes_with_writer<W>(
        &mut self,
        bytes: &[u8],
        signature: &str,
        now_unix: i64,
        write_file: W,
    ) -> Result<Catalog, LoadError>
    where
        W: FnMut(PublishStep, &Path, &[u8]) -> Result<(), String>,
    {
        self.accept_bytes_inner(bytes, signature, now_unix, write_file)
    }

    fn accept_bytes_inner<W>(
        &mut self,
        bytes: &[u8],
        signature: &str,
        now_unix: i64,
        write_file: W,
    ) -> Result<Catalog, LoadError>
    where
        W: FnMut(PublishStep, &Path, &[u8]) -> Result<(), String>,
    {
        self.require_keys()?;
        let key_refs: Vec<&str> = self.keys.iter().map(String::as_str).collect();
        let index = index_v2::verify_and_parse(bytes, signature, &key_refs)?;

        let _commit = COMMIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let state = self.read_state()?;
        let highest = combine_serial(
            self.highest_serial,
            state.as_ref().map(|s| s.highest_serial),
        );
        if let Some(prev) = highest.filter(|&prev| index.serial < prev) {
            return Err(LoadError::Rollback {
                got: index.serial,
                highest: prev,
            });
        }
        let next_serial = highest.map_or(index.serial, |prev| prev.max(index.serial));
        let previous_active = state.and_then(|s| s.active);
        self.persist_verified_with(
            bytes,
            signature,
            next_serial,
            previous_active.as_ref(),
            write_file,
        )?;
        self.highest_serial = Some(next_serial);
        let stale_warning = stale_warning_at(&index, now_unix);
        Ok(Catalog {
            index: Arc::new(index),
            from_cache: false,
            stale_warning,
        })
    }

    /// Loads and re-verifies the cached v2 copy from this store's namespace, enforcing both
    /// signature verification and the highest accepted `serial`.
    pub fn load_cached(&mut self, now_unix: i64) -> Result<Catalog, LoadError> {
        self.require_keys()?;
        let _commit = COMMIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let state = self.read_state()?;
        let (index_path, sig_path) = match state.as_ref().and_then(|s| s.active.as_ref()) {
            Some(active) => (
                self.cache_dir.join(active.index_name()),
                self.cache_dir.join(active.signature_name()),
            ),
            None => (
                self.cache_dir.join(CACHE_INDEX),
                self.cache_dir.join(CACHE_SIG),
            ),
        };
        let bytes = read_bounded_file(
            &index_path,
            index_v2::MAX_INDEX,
            tr("The mod registry index is too large.").into(),
        )
        .map_err(LoadError::Fetch)?;
        let sig_bytes = read_bounded_file(
            &sig_path,
            index_v2::MAX_SIGNATURE,
            tr("The mod registry signature is too large.").into(),
        )
        .map_err(LoadError::Fetch)?;
        let signature = String::from_utf8(sig_bytes)
            .map_err(|e| LoadError::BadSignature(format!("invalid UTF-8 signature: {e}")))?;

        let key_refs: Vec<&str> = self.keys.iter().map(String::as_str).collect();
        let index = index_v2::verify_and_parse(&bytes, &signature, &key_refs)?;
        let highest = combine_serial(
            self.highest_serial,
            state.as_ref().map(|s| s.highest_serial),
        );
        if let Some(prev) = highest.filter(|&prev| index.serial < prev) {
            return Err(LoadError::Rollback {
                got: index.serial,
                highest: prev,
            });
        }
        self.highest_serial = Some(highest.map_or(index.serial, |prev| prev.max(index.serial)));
        let stale_warning = stale_warning_at(&index, now_unix);
        Ok(Catalog {
            index: Arc::new(index),
            from_cache: true,
            stale_warning,
        })
    }

    #[cfg(test)]
    fn active_cache_paths(&self) -> Result<Option<(PathBuf, PathBuf)>, LoadError> {
        let Some(state) = self.read_state()? else {
            return Ok(None);
        };
        let Some(active) = state.active else {
            return Ok(None);
        };
        Ok(Some((
            self.cache_dir.join(active.index_name()),
            self.cache_dir.join(active.signature_name()),
        )))
    }

    fn require_keys(&self) -> Result<(), LoadError> {
        if !self.keys.iter().any(|key| !key.trim().is_empty()) {
            return Err(LoadError::KeyNotConfigured);
        }
        Ok(())
    }

    fn read_state(&self) -> Result<Option<PersistedState>, LoadError> {
        let state_path = self.cache_dir.join(STATE_FILE);
        match fs::read_to_string(&state_path) {
            Ok(text) => {
                let stored: StoredState = serde_json::from_str(&text)
                    .map_err(|e| LoadError::CorruptState(e.to_string()))?;
                // A pointer this build can't read, or one naming a file outside the snapshot
                // pattern, leaves no cached copy: the next fetch replaces it, nothing it names is
                // deleted, and the serial floor stays.
                let active = stored
                    .active
                    .and_then(|value| serde_json::from_value::<ActiveCache>(value).ok())
                    .filter(|active| valid_snapshot_id(&active.snapshot));
                Ok(Some(PersistedState {
                    highest_serial: stored.highest_serial,
                    active,
                }))
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(LoadError::CorruptState(error.to_string())),
        }
    }

    fn persist_verified_with<W>(
        &self,
        bytes: &[u8],
        signature: &str,
        serial: u64,
        previous_active: Option<&ActiveCache>,
        mut write_file: W,
    ) -> Result<(), LoadError>
    where
        W: FnMut(PublishStep, &Path, &[u8]) -> Result<(), String>,
    {
        fs::create_dir_all(&self.cache_dir)
            .map_err(|e| LoadError::Fetch(format!("{}: {e}", self.cache_dir.display())))?;
        let next = ActiveCache {
            snapshot: next_snapshot_id(serial).map_err(LoadError::Fetch)?,
        };
        let index_name = next.index_name();
        let sig_name = next.signature_name();
        let index_path = self.cache_dir.join(&index_name);
        let sig_path = self.cache_dir.join(&sig_name);
        let state_path = self.cache_dir.join(STATE_FILE);

        if let Err(err) = write_file(PublishStep::Index, &index_path, bytes) {
            let _ = fs::remove_file(&index_path);
            return Err(LoadError::Fetch(err));
        }
        if let Err(err) = write_file(PublishStep::Signature, &sig_path, signature.as_bytes()) {
            let _ = fs::remove_file(&index_path);
            let _ = fs::remove_file(&sig_path);
            return Err(LoadError::Fetch(err));
        }
        let state_json = serde_json::to_vec(&PersistedState {
            highest_serial: serial,
            active: Some(next.clone()),
        })
        .map_err(|e| LoadError::CorruptState(e.to_string()))?;
        if let Err(err) = write_file(PublishStep::State, &state_path, &state_json) {
            let _ = fs::remove_file(&index_path);
            let _ = fs::remove_file(&sig_path);
            return Err(LoadError::Fetch(err));
        }

        // Remove the superseded snapshot only after state.json atomically published the new one.
        if let Some(old) = previous_active.filter(|old| **old != next) {
            let _ = fs::remove_file(self.cache_dir.join(old.index_name()));
            let _ = fs::remove_file(self.cache_dir.join(old.signature_name()));
        }
        Ok(())
    }
}

fn combine_serial(memory: Option<u64>, disk: Option<u64>) -> Option<u64> {
    match (memory, disk) {
        (Some(mem), Some(disk)) => Some(mem.max(disk)),
        (Some(mem), None) => Some(mem),
        (None, Some(disk)) => Some(disk),
        (None, None) => None,
    }
}

fn next_snapshot_id(serial: u64) -> Result<String, String> {
    let mut nonce = [0u8; 8];
    getrandom::fill(&mut nonce).map_err(|e| format!("random snapshot id: {e}"))?;
    let hex: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("snapshot-{serial}-{hex}"))
}

fn has_configured_key(keys: &[&str]) -> bool {
    keys.iter().any(|key| !key.trim().is_empty())
}

fn stale_warning_at(index: &Index, now_unix: i64) -> Option<String> {
    index.is_stale_at(now_unix).then(|| {
        trf(
            "The mod registry index was last updated on {date} and may be out of date.",
            &[("date", &index.generated)],
        )
    })
}

fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
}

fn fetch_raw(source: &str) -> Result<(Vec<u8>, Vec<u8>), String> {
    let loopback = is_debug_loopback(source);
    if source.starts_with("http://") && !loopback {
        return Err(tr("The mod registry requires HTTPS.").into());
    }
    let max = index_v2::MAX_SIGNATURE + index_v2::MAX_INDEX;
    let too_large = || tr("The mod registry index is too large.").to_owned();
    let bytes = if source.starts_with("https://") || loopback {
        fetch_http_bytes(source, loopback, source == INDEX_URL, max, too_large())?
    } else {
        read_bounded_file(Path::new(source), max, too_large())?
    };
    split_signed(bytes)
}

fn is_debug_loopback(source: &str) -> bool {
    cfg!(any(test, debug_assertions))
        && url::Url::parse(source).is_ok_and(|parsed| {
            parsed.scheme() == "http"
                && parsed.host_str() == Some("127.0.0.1")
                && parsed.port().is_some()
                && parsed.username().is_empty()
                && parsed.password().is_none()
        })
}

fn fetch_http_bytes(
    url: &str,
    loopback: bool,
    direct: bool,
    max_bytes: u64,
    too_large: String,
) -> Result<Vec<u8>, String> {
    let agent = if loopback {
        ureq::AgentBuilder::new()
            .try_proxy_from_env(false)
            .redirects(0)
            .timeout(Duration::from_secs(2))
            .build()
    } else {
        let slot = if direct {
            "sunrise-index-v2-direct"
        } else {
            "sunrise-index-v2"
        };
        crate::proxy::agent(slot, url, |builder| {
            let builder = builder.https_only(true).timeout(Duration::from_secs(10));
            if direct {
                builder.redirects(0)
            } else {
                builder
            }
        })?
    };

    let response = agent
        .get(url)
        .set("User-Agent", crate::proxy::USER_AGENT)
        .call()
        .map_err(|error| match error {
            ureq::Error::Status(code, _) => trf(
                "The mod registry answered {code}.",
                &[("code", &code.to_string())],
            ),
            ureq::Error::Transport(transport) => trf(
                "The mod registry can't be reached: {error}",
                &[("error", &transport.to_string())],
            ),
        })?;
    if response.status() != 200 || response.header("Content-Range").is_some() {
        return Err(tr("The mod registry didn't send the full index.").into());
    }
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max_bytes {
        return Err(too_large);
    }
    Ok(bytes)
}

fn read_bounded_file(path: &Path, max_bytes: u64, too_large: String) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() as u64 > max_bytes {
        return Err(too_large);
    }
    Ok(bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut temp = tempfile::Builder::new()
        .prefix(".sunrise-v2-")
        .tempfile_in(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    temp.write_all(bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    temp.as_file()
        .sync_all()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    temp.persist(path)
        .map_err(|e| format!("{}: {}", path.display(), e.error))?;
    Ok(())
}

#[cfg(test)]
mod tests;
