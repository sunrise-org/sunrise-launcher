//! Resolve a mod set's passive Java jars before a launcher-managed launch.
mod launch;
use crate::browser::installed::Installed;
use crate::i18n::{tr, trf};
pub use launch::{prepare_for_command, prepare_launch, LaunchError};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs, io,
    path::{Path, PathBuf},
};
use sunrise_launcher_jar_fetch::{Http, Modrinth};
use sunrise_launcher_smod::manifest::{JarRef, JarRequirement, MissingJar};
use ureq::OrAnyStatus;

/// Where jars are found and downloaded from: Modrinth's API, and only its CDN.
const MODRINTH: Modrinth<'static> = Modrinth {
    api: crate::browser::modrinth::API,
    cdn_host: "cdn.modrinth.com",
};

/// `sunrise-launcher-jar-fetch`'s transport: the launcher's proxy settings, and no hop off the
/// requested `https` host.
#[derive(Default)]
struct Transport {
    /// Checked before each request and each read, so a stopped install ends within a chunk.
    stop: Option<crate::native_mods::Cancellation>,
}

/// A body whose every read first checks the stop.
struct Stoppable<R>(crate::native_mods::Cancellation, R);

impl<R: io::Read> io::Read for Stoppable<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.check().map_err(io::Error::other)?;
        self.1.read(buf)
    }
}

impl Transport {
    fn guard(&self, body: Box<dyn io::Read>) -> Box<dyn io::Read> {
        match &self.stop {
            Some(stop) => Box::new(Stoppable(stop.clone(), body)),
            None => body,
        }
    }

    fn check(&self) -> Result<(), String> {
        self.stop.as_ref().map_or(Ok(()), |stop| stop.check())
    }
}

impl Http for Transport {
    fn get(&self, url: &str) -> Result<Option<Box<dyn io::Read>>, String> {
        self.check()?;
        // The CDN host check lives in sunrise_launcher_jar_fetch::download; this host list only
        // blocks redirects to another host.
        let host = url::Url::parse(url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .ok_or_else(|| format!("{url}: no host"))?;
        Ok(crate::net::get_from_hosts(url, &[&host])?
            .map(|response| self.guard(response.into_reader())))
    }

    fn post_json(&self, url: &str, body: &str) -> Result<Box<dyn io::Read>, String> {
        self.check()?;
        let response =
            crate::proxy::agent("jar-version-files", url, |builder| builder.redirects(0))?
                .post(url)
                .set("User-Agent", crate::proxy::USER_AGENT)
                .set("Content-Type", "application/json")
                .send_string(body)
                .or_any_status()
                .map_err(|e| format!("{url}: {}", e.kind()))?;
        let status = response.status();
        if status >= 300 {
            return Err(trf(
                "{url}: HTTP {status}",
                &[("url", url), ("status", &status.to_string())],
            ));
        }
        Ok(self.guard(response.into_reader()))
    }
}

/// Download consent for this mod set, never a permission to execute Java code. It decides only
/// jars that no installed Sunrise registry mod lists: those are fetched without asking.
#[non_exhaustive]
pub enum Permission<'a> {
    Refuse,
    Allow,
    Ask(&'a dyn Fn(&[&JarRef]) -> bool),
}

impl Permission<'static> {
    /// Headless managed launches never ask: downloading needs an explicit per-call opt-in.
    pub fn headless(allow_downloads: bool) -> Self {
        if allow_downloads {
            Self::Allow
        } else {
            Self::Refuse
        }
    }
}

/// Preparation failures retain their kind; callers never inspect error text.
#[derive(Debug)]
#[non_exhaustive]
pub enum PrepareError {
    Refused {
        missing: Vec<MissingJar>,
    },
    /// One metadata request failed for this complete missing set.
    Lookup {
        missing: Vec<MissingJar>,
        reason: String,
    },
    Download {
        jar: Box<JarRef>,
        path: PathBuf,
        reason: String,
    },
    /// Modrinth's exact-hash lookup returned no file for this jar.
    NoLongerOnModrinth {
        jar: Box<JarRef>,
        path: PathBuf,
    },
    HashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    Io {
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for PrepareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused { missing } => {
                f.write_str(tr("Jar downloads were not allowed. Supply the original files at these exact cache paths, or pass --allow-jar-downloads:\n"))?;
                write_missing_jars(f, missing)
            }
            Self::Lookup { missing, reason } => {
                f.write_str(&trf(
                    "Jar metadata lookup failed: {reason}. Put the original files at these exact cache paths, then retry:\n",
                    &[("reason", reason)],
                ))?;
                write_missing_jars(f, missing)
            }
            Self::Download { jar, path, reason } => f.write_str(&trf(
                "Jar {id} {version}: {reason}. Put the original file with this exact SHA-512 at {path}, then retry.",
                &[("id", jar.id()), ("version", jar.version()), ("reason", reason), ("path", &path.display().to_string())],
            )),
            Self::NoLongerOnModrinth { jar, path } => f.write_str(&trf(
                "The original file {id} {version} is no longer on Modrinth. Put the original file with this exact SHA-512 at {path} to use it.",
                &[("id", jar.id()), ("version", jar.version()), ("path", &path.display().to_string())],
            )),
            Self::HashMismatch { path, expected, actual } => f.write_str(&trf(
                "SHA-512 mismatch at {path}: expected {expected}, got {actual}. Replace this cache entry with the exact original file, or remove it and retry with --allow-jar-downloads.",
                &[("path", &path.display().to_string()), ("expected", expected), ("actual", actual)],
            )),
            Self::Io { path, source } => f.write_str(&trf(
                "Cannot read or write jar cache {path}: {error}. Check the file and its permissions before retrying.",
                &[("path", &path.display().to_string()), ("error", &source.to_string())],
            )),
        }
    }
}
fn write_missing_jars(f: &mut fmt::Formatter<'_>, missing: &[MissingJar]) -> fmt::Result {
    for entry in missing {
        writeln!(f, "{entry}")?;
    }
    Ok(())
}

impl std::error::Error for PrepareError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// In-game ids of the mods installed from the Sunrise registry, read from the installed rows of
/// the game folder `game_dir`. An unreadable list names none, so every jar then asks as before.
pub fn registry_mods(game_dir: &Path) -> BTreeSet<String> {
    Installed::load(game_dir)
        .entries
        .iter()
        .filter_map(|row| row.in_game_mod_id().map(str::to_owned))
        .collect()
}

/// Cache hits work offline. Missing jars are resolved by exact SHA-512. A missing jar listed by a
/// mod in `registry_mods` is downloaded without asking; the rest are downloaded only after one
/// consent decision for their complete set.
pub fn prepare(
    home: &Path,
    jars: &[JarRequirement],
    registry_mods: &BTreeSet<String>,
    permission: Permission<'_>,
) -> Result<(), PrepareError> {
    prepare_cancellable(home, jars, registry_mods, permission, None)
}

/// [`prepare`], whose downloads stop at the next chunk once `stop` is asked. A half-read jar is
/// never kept.
pub fn prepare_cancellable(
    home: &Path,
    jars: &[JarRequirement],
    registry_mods: &BTreeSet<String>,
    permission: Permission<'_>,
    stop: Option<&crate::native_mods::Cancellation>,
) -> Result<(), PrepareError> {
    let transport = Transport {
        stop: stop.cloned(),
    };
    prepare_with(
        home,
        jars,
        registry_mods,
        permission,
        |missing| {
            let hashes: Vec<&str> = missing.iter().map(|jar| jar.sha512()).collect();
            sunrise_launcher_jar_fetch::file_urls(&transport, MODRINTH, &hashes)
        },
        |url, jar, dest| {
            sunrise_launcher_jar_fetch::download(&transport, MODRINTH, url, jar.sha512(), dest)
        },
    )
}

fn jar_owners(jars: &[JarRequirement]) -> BTreeSet<String> {
    jars.iter()
        .flat_map(|requirement| requirement.owners.iter().cloned())
        .collect()
}

/// Fetches every jar of `jars` into the cache for an install or update of registry mods, which
/// asks nothing: each listing mod is a reviewed registry release. A jar Modrinth no longer has
/// fails with [`PrepareError::NoLongerOnModrinth`].
pub fn fetch_listed(home: &Path, jars: &[JarRequirement]) -> Result<(), PrepareError> {
    fetch_listed_cancellable(home, jars, None)
}

/// [`fetch_listed`], which stops at the next chunk once `stop` is asked.
pub fn fetch_listed_cancellable(
    home: &Path,
    jars: &[JarRequirement],
    stop: Option<&crate::native_mods::Cancellation>,
) -> Result<(), PrepareError> {
    prepare_cancellable(home, jars, &jar_owners(jars), Permission::Refuse, stop)
}

#[cfg(test)]
fn fetch_listed_with(
    home: &Path,
    jars: &[JarRequirement],
    lookup: impl FnMut(&[&JarRef]) -> Result<BTreeMap<String, String>, String>,
    download: impl FnMut(&str, &JarRef, &Path) -> Result<(), String>,
) -> Result<(), PrepareError> {
    prepare_with(
        home,
        jars,
        &jar_owners(jars),
        Permission::Refuse,
        lookup,
        download,
    )
}

/// Prepares `jars` like [`prepare`], then copies each into `to`'s own `jars` folder: the home a
/// server exported there reads them from. The export is the operator's own copy.
pub fn export(
    home: &Path,
    to: &Path,
    jars: &[JarRequirement],
    registry_mods: &BTreeSet<String>,
    permission: Permission<'_>,
) -> Result<(), PrepareError> {
    prepare(home, jars, registry_mods, permission)?;
    copy_exported(home, to, jars)
}

fn copy_exported(home: &Path, to: &Path, jars: &[JarRequirement]) -> Result<(), PrepareError> {
    if jars.is_empty() {
        return Ok(());
    }
    let (cache, folder) = (home.join("jars"), to.join("jars"));
    fs::create_dir_all(&folder).map_err(|source| PrepareError::Io {
        path: folder.clone(),
        source,
    })?;
    for requirement in jars {
        let path = requirement.jar.cache_path(&folder);
        fs::copy(requirement.jar.cache_path(&cache), &path)
            .map_err(|source| PrepareError::Io { path, source })?;
    }
    Ok(())
}

#[cfg(test)]
fn export_with(
    home: &Path,
    to: &Path,
    jars: &[JarRequirement],
    registry_mods: &BTreeSet<String>,
    permission: Permission<'_>,
    lookup: impl FnMut(&[&JarRef]) -> Result<BTreeMap<String, String>, String>,
    download: impl FnMut(&str, &JarRef, &Path) -> Result<(), String>,
) -> Result<(), PrepareError> {
    prepare_with(home, jars, registry_mods, permission, lookup, download)?;
    copy_exported(home, to, jars)
}

/// Deletes from the cache each jar in `removed` that no jar in `kept` names. A cache file that
/// `removed` does not list is left alone.
pub fn forget(home: &Path, removed: &[JarRef], kept: &[JarRef]) -> io::Result<()> {
    let cache = home.join("jars");
    for jar in removed {
        if kept.iter().any(|other| other.sha512() == jar.sha512()) {
            continue;
        }
        match fs::remove_file(jar.cache_path(&cache)) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
    }
    Ok(())
}

fn prepare_with(
    home: &Path,
    jars: &[JarRequirement],
    registry_mods: &BTreeSet<String>,
    permission: Permission<'_>,
    mut lookup: impl FnMut(&[&JarRef]) -> Result<BTreeMap<String, String>, String>,
    mut download: impl FnMut(&str, &JarRef, &Path) -> Result<(), String>,
) -> Result<(), PrepareError> {
    let cache = home.join("jars");
    let mut missing = Vec::new();
    for requirement in jars {
        let jar = &requirement.jar;
        let path = jar.cache_path(&cache);
        match fs::File::open(&path) {
            Ok(file) => {
                let hash =
                    sunrise_launcher_jar_fetch::hash_reader::<sha2::Sha512>(file, io::sink())
                        .map_err(|source| PrepareError::Io {
                            path: path.clone(),
                            source,
                        })?;
                let actual = format!("{hash:x}");
                if actual != jar.sha512() {
                    return Err(PrepareError::HashMismatch {
                        path,
                        expected: jar.sha512().into(),
                        actual,
                    });
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => missing.push(requirement),
            Err(source) => return Err(PrepareError::Io { path, source }),
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    let entries = |list: &[&JarRequirement]| -> Vec<MissingJar> {
        list.iter()
            .map(|requirement| MissingJar {
                owners: requirement.owners.clone(),
                jar: requirement.jar.clone(),
                path: requirement.jar.cache_path(&cache),
            })
            .collect()
    };
    let missing_entries = || entries(&missing);
    let missing_refs: Vec<_> = missing.iter().map(|requirement| &requirement.jar).collect();
    let asking: Vec<_> = missing
        .iter()
        .copied()
        .filter(|requirement| {
            !requirement
                .owners
                .iter()
                .any(|owner| registry_mods.contains(owner))
        })
        .collect();
    if !asking.is_empty() {
        match permission {
            Permission::Refuse => {
                return Err(PrepareError::Refused {
                    missing: entries(&asking),
                })
            }
            Permission::Allow => {}
            Permission::Ask(ask) => {
                let asking_refs: Vec<_> =
                    asking.iter().map(|requirement| &requirement.jar).collect();
                if !ask(&asking_refs) {
                    return Err(PrepareError::Refused {
                        missing: entries(&asking),
                    });
                }
            }
        }
    }
    let urls = lookup(&missing_refs).map_err(|reason| PrepareError::Lookup {
        missing: missing_entries(),
        reason,
    })?;
    let urls: Vec<_> = missing_refs
        .iter()
        .map(|jar| {
            urls.get(jar.sha512())
                .ok_or_else(|| PrepareError::NoLongerOnModrinth {
                    jar: Box::new((*jar).clone()),
                    path: jar.cache_path(&cache),
                })
        })
        .collect::<Result<_, _>>()?;
    fs::create_dir_all(&cache).map_err(|source| PrepareError::Io {
        path: cache.clone(),
        source,
    })?;
    for (jar, url) in missing_refs.into_iter().zip(urls) {
        let path = jar.cache_path(&cache);
        download(url, jar, &path).map_err(|reason| PrepareError::Download {
            jar: Box::new(jar.clone()),
            path,
            reason,
        })?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "jars_tests.rs"]
mod tests;
