//! Assembling the pinned Minecraft version into Sunrise's own folder (`data_dir()/minecraft`):
//! the version JSON, the client jar, this system's libraries and the assets (the sounds), each
//! checked against its sha1, in the official launcher's layout. Files already present and whole
//! are kept; missing or mismatched files are copied from any other launcher's install when their
//! hash matches, and otherwise downloaded from Mojang for a signed-in account that owns Minecraft:
//! Java Edition (#434). Nothing of Mojang's is part of Sunrise.
//!
//! Each file is checked against the size and sha1 Mojang lists, and written whole through a unique
//! temporary file. A download stopped half way (cancelled, no network) keeps the files it finished:
//! the next one fetches only the rest.

use crate::backend::lock::recover;
use crate::i18n::{tr, trf};
use crate::sources::{
    client_jars, copy_checked, fetch_checked, fetch_manifest, missing_version_file,
    prism_native_path, sha1_file, sha1_hex, write_file, Fetch, Pins, Store,
};
use serde_json::Value;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::Duration;

const OBJECTS: &str = sunrise_launcher_addresses::MOJANG_RESOURCES;
/// The index of Mojang's Java runtimes, the one the official launcher reads.
const JAVA_RUNTIMES: &str = sunrise_launcher_addresses::MOJANG_JAVA_RUNTIMES;

/// Set to stop the download under way (the window's Cancel); cleared when a preparation starts.
pub static CANCEL: AtomicBool = AtomicBool::new(false);
/// The download under way: files done, of how many (0 when none runs).
static DONE: AtomicUsize = AtomicUsize::new(0);
static TOTAL: AtomicUsize = AtomicUsize::new(0);

/// How far the download under way is, files done of how many; none when none runs.
pub fn progress() -> Option<(usize, usize)> {
    let total = TOTAL.load(Relaxed);
    (total > 0).then(|| (DONE.load(Relaxed).min(total), total))
}

const CANCELLED: &str = "Download cancelled. Download again to go on where it stopped.";

/// Where another launcher's folder may already keep a pinned file.
enum CandidateKind {
    /// Client jar: checked across every launcher's client jar path for this version.
    Client(String),
    /// Library: official path under `libraries/` and Prism Launcher's native path.
    Library(String),
    /// Exact path relative to a launcher's base folder (logging config, asset index, object).
    Relative(String),
    /// No cross-launcher copy (Java runtime files).
    None,
}

/// One file to assemble, with the size Mojang lists for it when it lists one.
struct File {
    url: String,
    sha1: String,
    size: Option<u64>,
    path: PathBuf,
    candidates: CandidateKind,
}

fn file_ok(file: &File) -> bool {
    if let Some(size) = file.size {
        if fs::metadata(&file.path).ok().map(|m| m.len()) != Some(size) {
            return false;
        }
    }
    sha1_file(&file.path).as_ref() == Some(&file.sha1)
}

fn try_copy_from_sources(file: &File, sources: &[PathBuf]) -> Result<bool, String> {
    match &file.candidates {
        CandidateKind::None => Ok(false),
        CandidateKind::Relative(rel) => {
            for base in sources {
                if copy_checked(&base.join(rel), &file.path, &file.sha1, file.size)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        CandidateKind::Library(rel) => {
            let prism = prism_native_path(rel);
            for base in sources {
                if copy_checked(
                    &base.join("libraries").join(rel),
                    &file.path,
                    &file.sha1,
                    file.size,
                )? {
                    return Ok(true);
                }
                if let Some(alt) = &prism {
                    if copy_checked(
                        &base.join("libraries").join(alt),
                        &file.path,
                        &file.sha1,
                        file.size,
                    )? {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        }
        CandidateKind::Client(version) => {
            for base in sources {
                for jar in client_jars(base, version) {
                    if copy_checked(&base.join(jar), &file.path, &file.sha1, file.size)? {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        }
    }
}

fn game_files(manifest: &Value, pins: &Pins, dir: &Path, os: &str) -> Result<Vec<File>, String> {
    let v = &pins.version;
    let versions = dir.join("versions").join(v);
    let client = &manifest["downloads"]["client"];
    // A version Sunrise doesn't pin takes its client's sha1 from its version file, itself
    // checked against the sha1 Mojang's version list gives (`Pins::for_version`).
    let client_sha1 = match pins.client_sha1.as_str() {
        "" => text(client, "sha1")?,
        pinned => pinned.to_string(),
    };
    let mut game = vec![File {
        url: text(client, "url")?,
        sha1: client_sha1,
        size: client["size"].as_u64(),
        path: versions.join(format!("{v}.jar")),
        candidates: CandidateKind::Client(v.clone()),
    }];
    let libraries = manifest["libraries"]
        .as_array()
        .ok_or("The version file has no libraries")?;
    for library in libraries.iter().filter(|l| allowed(l, os)) {
        // Before 1.19 a library's natives for this system are a classifier of its own.
        let native = native_classifier(library, os)
            .map(|key| &library["downloads"]["classifiers"][key.as_str()]);
        for artifact in [Some(&library["downloads"]["artifact"]), native]
            .into_iter()
            .flatten()
            .filter(|a| !a.is_null())
        {
            let rel_str = text(artifact, "path")?;
            let rel = relative(&rel_str)?;
            game.push(File {
                url: text(artifact, "url")?,
                sha1: text(artifact, "sha1")?,
                size: artifact["size"].as_u64(),
                path: dir.join("libraries").join(rel),
                candidates: CandidateKind::Library(rel_str),
            });
        }
    }
    // The logging setup the game is started with (`logging.client.argument`): for 1.7 to 1.18,
    // Mojang's fixed one is what keeps Log4Shell out.
    let logging = &manifest["logging"]["client"]["file"];
    if !logging.is_null() {
        let id = text(logging, "id")?;
        let rel = relative(&id)?;
        game.push(File {
            url: text(logging, "url")?,
            sha1: text(logging, "sha1")?,
            size: logging["size"].as_u64(),
            path: dir.join("assets/log_configs").join(rel),
            candidates: CandidateKind::Relative(format!("assets/log_configs/{id}")),
        });
    }
    Ok(game)
}

fn asset_index_file(manifest: &Value, dir: &Path) -> Result<File, String> {
    let index = &manifest["assetIndex"];
    let id = text(index, "id")?;
    let rel_name = format!("{id}.json");
    Ok(File {
        url: text(index, "url")?,
        sha1: text(index, "sha1")?,
        size: index["size"].as_u64(),
        path: dir.join("assets/indexes").join(relative(&rel_name)?),
        candidates: CandidateKind::Relative(format!("assets/indexes/{rel_name}")),
    })
}

/// Assembles the pinned version into `store.dir`, `jobs` files at a time: files already in
/// `store.dir` and whole are kept, missing or mismatched files are copied from `store.sources`
/// when a hash-matching copy exists, and the rest are downloaded from Mojang. Requires the
/// primary pinned version manifest. An asset that fails is a line in the log (the game is then
/// partly silent), anything else an error. `cancel` stops it between files.
pub fn download(
    pins: &Pins,
    store: &Store<'_>,
    os: &str,
    jobs: usize,
    fetch: Fetch,
    cancel: &AtomicBool,
    log: &dyn Fn(String),
) -> Result<(), String> {
    let v = &pins.version;
    let manifest_path = store.dir.join("versions").join(v).join(format!("{v}.json"));
    fetch_manifest(pins, &manifest_path, store.sources, fetch, log)?;
    let manifest = read_json(&manifest_path)?;
    let game = game_files(&manifest, pins, store.dir, os)?;
    let libraries = game.len() - 1;
    log(trf(
        "Downloading Minecraft {version} from Mojang: the game and {libraries} libraries",
        &[
            ("version", &v.to_string()),
            ("libraries", &libraries.to_string()),
        ],
    ));
    fetch_all(&game, store.sources, jobs, fetch, cancel, log)?;
    let index = asset_index_file(&manifest, store.dir)?;
    let assets = fetch_all(
        std::slice::from_ref(&index),
        store.sources,
        1,
        fetch,
        cancel,
        log,
    )
    .and_then(|_| objects(&read_json(&index.path)?, store.dir))
    .and_then(|objects| {
        log(trf(
            "Downloading the sounds ({count} files)",
            &[("count", &objects.len().to_string())],
        ));
        // A connection that fails now and then is tried again, and a file that still fails
        // does not stop the others: the next download fetches what is left.
        let retrying = |url: &str| {
            let mut got = fetch(url);
            for _ in 0..2 {
                if got.is_ok() {
                    break;
                }
                got = fetch(url);
            }
            got
        };
        fetch_each(&objects, store.sources, jobs, &retrying, cancel, log, false)
    });
    match assets {
        Err(e) if e == CANCELLED => return Err(e),
        Err(e) => log(trf(
            "The sounds could not all be downloaded: {error}",
            &[("error", &e.to_string())],
        )),
        Ok(()) => {}
    }
    Ok(())
}

/// Assembles the pinned version into `store.dir` purely from local `store.sources` (no network),
/// used when no signed-in account that owns Minecraft is available. Copies every hash-matching
/// file (including assets when present) and returns an error if the manifest or any required game
/// file is missing locally.
pub(crate) fn assemble_local(
    pins: &Pins,
    store: &Store<'_>,
    os: &str,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let v = &pins.version;
    let manifest_path = store.dir.join("versions").join(v).join(format!("{v}.json"));
    ensure_local_manifest(pins, &manifest_path, store.sources)?;
    let manifest = read_json(&manifest_path)?;
    let game = game_files(&manifest, pins, store.dir, os)?;
    let mut missing = None;
    for file in &game {
        if cancel.load(Relaxed) {
            return Err(CANCELLED.to_string());
        }
        if !file_ok(file) && !try_copy_from_sources(file, store.sources)? {
            missing.get_or_insert_with(|| file.path.clone());
        }
    }
    copy_local_assets(&manifest, store, cancel)?;
    if let Some(path) = missing {
        return Err(format!(
            "Missing pinned Minecraft {v} file: {}",
            path.display()
        ));
    }
    Ok(())
}

fn copy_local_assets(
    manifest: &Value,
    store: &Store<'_>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let Ok(index) = asset_index_file(manifest, store.dir) else {
        return Ok(());
    };
    if !file_ok(&index) && !try_copy_from_sources(&index, store.sources)? {
        return Ok(());
    }
    let Ok(objs) = read_json(&index.path).and_then(|j| objects(&j, store.dir)) else {
        return Ok(());
    };
    for file in &objs {
        if cancel.load(Relaxed) {
            return Err(CANCELLED.to_string());
        }
        if !file_ok(file) {
            let _ = try_copy_from_sources(file, store.sources);
        }
    }
    Ok(())
}

fn ensure_local_manifest(pins: &Pins, path: &Path, sources: &[PathBuf]) -> Result<(), String> {
    if pins.manifest_is_primary(path) {
        return Ok(());
    }
    let v = &pins.version;
    let primary = pins.primary_manifest_sha1();
    for base in sources {
        let candidate = base.join("versions").join(v).join(format!("{v}.json"));
        if copy_checked(&candidate, path, primary, None)? {
            return Ok(());
        }
    }
    if pins.manifest_accepted(path) {
        return Ok(());
    }
    for sha1 in pins.manifests.iter().skip(1) {
        for base in sources {
            let candidate = base.join("versions").join(v).join(format!("{v}.json"));
            if copy_checked(&candidate, path, sha1, None)? {
                return Ok(());
            }
        }
    }
    Err(missing_version_file(path))
}

/// Mojang's name for this system in its Java runtime index.
pub(crate) fn java_platform() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "mac-os-arm64",
        ("macos", "x86_64") => "mac-os",
        ("linux", "x86_64") => "linux",
        ("linux", "x86") => "linux-i386",
        ("windows", "x86_64") => "windows-x64",
        ("windows", "aarch64") => "windows-arm64",
        ("windows", "x86") => "windows-x86",
        _ => return None,
    })
}

fn no_runtime(component: &str, platform: &str) -> String {
    trf(
        "Mojang has no {component} for {platform}",
        &[("component", component), ("platform", platform)],
    )
}

fn unreadable_manifest(component: &str, e: &serde_json::Error) -> String {
    trf(
        "Mojang's {component} manifest: {error}",
        &[("component", component), ("error", &e.to_string())],
    )
}

fn downloading_java(name: &str, count: usize) -> String {
    trf(
        "Downloading Minecraft's Java {name} from Mojang ({count} files)",
        &[("name", name), ("count", &count.to_string())],
    )
}

/// Downloads Mojang's Java runtime `component` (the version JSON's `javaVersion.component`, e.g.
/// `java-runtime-epsilon`) into the game directory's `runtime` folder, as the official launcher
/// lays it out: `<component>/<platform>/<component>/`. Each file is checked against its sha1;
/// files already there and whole are kept. These also include javac for older release preparers;
/// current releases can use an already installed runtime without it.
pub fn java_runtime(
    component: &str,
    runtime: &Path,
    jobs: usize,
    fetch: Fetch,
    cancel: &AtomicBool,
    log: &dyn Fn(String),
) -> Result<PathBuf, String> {
    let platform = java_platform().ok_or(tr("Mojang has no Java for this system"))?;
    let index: Value = serde_json::from_slice(&fetch(JAVA_RUNTIMES)?)
        .map_err(|e| trf("Mojang's Java index: {error}", &[("error", &e.to_string())]))?;
    let entry = &index[platform][component][0];
    if entry.is_null() {
        return Err(no_runtime(component, platform));
    }
    let manifest = &entry["manifest"];
    let manifest: Value = serde_json::from_slice(&fetch_checked(
        fetch,
        &text(manifest, "url")?,
        &text(manifest, "sha1")?,
    )?)
    .map_err(|e| unreadable_manifest(component, &e))?;
    let files = manifest["files"]
        .as_object()
        .ok_or_else(|| format!("Mojang's {component} manifest lists no files"))?;
    let home = runtime.join(component).join(platform).join(component);
    let mut downloads = Vec::new();
    let mut executables = Vec::new();
    let mut links = Vec::new();
    for (name, file) in files {
        let path = home.join(relative(name)?);
        match file["type"].as_str() {
            Some("file") => {
                let raw = &file["downloads"]["raw"];
                if file["executable"] == true {
                    executables.push(path.clone());
                }
                downloads.push(File {
                    url: text(raw, "url")?,
                    sha1: text(raw, "sha1")?,
                    size: raw["size"].as_u64(),
                    path,
                    candidates: CandidateKind::None,
                });
            }
            Some("link") => links.push((path, text(file, "target")?)),
            _ => {}
        }
    }
    let name = entry["version"]["name"].as_str().unwrap_or(component);
    log(downloading_java(name, downloads.len()));
    fetch_all(&downloads, &[], jobs, fetch, cancel, log)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in &executables {
            fs::set_permissions(path, fs::Permissions::from_mode(0o755))
                .map_err(|e| format!("{}: {e}", path.display()))?;
        }
        for (path, target) in &links {
            if !link_stays_inside(name_in(&home, path), target) {
                return Err(format!(
                    "Unsafe link {target:?} in Mojang's {component} manifest"
                ));
            }
            let _ = fs::remove_file(path);
            let folder = path
                .parent()
                .ok_or_else(|| format!("{}: not a file path", path.display()))?;
            fs::create_dir_all(folder)
                .and_then(|_| std::os::unix::fs::symlink(target, path))
                .map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    #[cfg(not(unix))]
    let _ = (executables, links);
    Ok(home)
}

// Used by the unix link step below; the path-traversal check itself is tested on every
// platform, so these stay compiled and are allowed to look unused elsewhere.
#[cfg_attr(not(unix), allow(dead_code))]
fn name_in<'a>(home: &Path, path: &'a Path) -> &'a Path {
    path.strip_prefix(home).unwrap_or(path)
}

/// Whether a link at `link` (relative to the runtime's home) pointing at `target` stays in the
/// home.
#[cfg_attr(not(unix), allow(dead_code))]
fn link_stays_inside(link: &Path, target: &str) -> bool {
    let mut depth = link.components().count().saturating_sub(1);
    for c in Path::new(target).components() {
        match c {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir if depth > 0 => depth -= 1,
            _ => return false,
        }
    }
    true
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn text(v: &Value, key: &str) -> Result<String, String> {
    v[key]
        .as_str()
        .map(String::from)
        .ok_or_else(|| format!("The version file has no {key} in {v}"))
}

/// A path from the version file, refused if it could leave the folder.
pub(crate) fn relative(path: &str) -> Result<PathBuf, String> {
    let safe = !path.contains('\\')
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)));
    safe.then(|| PathBuf::from(path))
        .ok_or_else(|| format!("Unsafe path {path:?} in the version file"))
}

/// Whether a library's rules let this system (`osx`, `windows`, `linux`) use it: the last
/// rule that matches decides.
fn allowed(library: &Value, os: &str) -> bool {
    let Some(rules) = library["rules"].as_array() else {
        return true;
    };
    rules
        .iter()
        .rfind(|rule| rule["os"]["name"].as_str().is_none_or(|name| name == os))
        .is_some_and(|rule| rule["action"] == "allow")
}

/// The classifier holding a library's natives for `os` (`natives-osx`), from its `natives` map;
/// `${arch}` is this system's word size, as old versions name Windows' natives.
pub(crate) fn native_classifier(library: &Value, os: &str) -> Option<String> {
    let arch = if cfg!(target_pointer_width = "64") {
        "64"
    } else {
        "32"
    };
    library["natives"][os]
        .as_str()
        .map(|key| key.replace("${arch}", arch))
}

/// The asset index's objects, once each.
fn objects(index: &Value, dir: &Path) -> Result<Vec<File>, String> {
    let objects = index["objects"]
        .as_object()
        .ok_or("The asset index has no objects")?;
    let sizes: std::collections::HashMap<&str, u64> = objects
        .values()
        .filter_map(|o| Some((o["hash"].as_str()?, o["size"].as_u64()?)))
        .collect();
    let mut hashes: Vec<&str> = objects
        .values()
        .filter_map(|o| o["hash"].as_str())
        .collect();
    hashes.sort_unstable();
    hashes.dedup();
    hashes
        .into_iter()
        .map(|hash| {
            if hash.len() != 40 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!("Bad asset hash {hash:?} in the asset index"));
            }
            let path = format!("{}/{hash}", &hash[..2]);
            Ok(File {
                url: format!("{OBJECTS}/{path}"),
                sha1: hash.to_string(),
                size: sizes.get(hash).copied(),
                path: dir.join("assets/objects").join(&path),
                candidates: CandidateKind::Relative(format!("assets/objects/{path}")),
            })
        })
        .collect()
}

/// One file from Mojang, refused unless it has the size and sha1 Mojang lists.
fn fetch_file(fetch: Fetch, file: &File) -> Result<Vec<u8>, String> {
    let body = fetch(&file.url)?;
    if let Some(size) = file.size.filter(|&size| size != body.len() as u64) {
        return Err(trf(
            "is {actual} bytes, not the {expected} Mojang lists",
            &[
                ("actual", &body.len().to_string()),
                ("expected", &size.to_string()),
            ],
        ));
    }
    match sha1_hex(&body) {
        actual if actual == file.sha1 => Ok(body),
        actual => Err(trf(
            "is not the one Sunrise pins (sha1 {actual})",
            &[("actual", actual.as_str())],
        )),
    }
}

/// Downloads the files not already there (or copied from `sources`), `jobs` at a time, with a
/// line every second; stops at the first error, or between files once `cancel` is set.
fn fetch_all(
    files: &[File],
    sources: &[PathBuf],
    jobs: usize,
    fetch: Fetch,
    cancel: &AtomicBool,
    log: &dyn Fn(String),
) -> Result<(), String> {
    fetch_each(files, sources, jobs, fetch, cancel, log, true)
}

/// `fetch_all`, which with `stop_on_error` false goes on past a file that fails and reports how
/// many did, with the first one's error, once the rest are done.
fn fetch_each(
    files: &[File],
    sources: &[PathBuf],
    jobs: usize,
    fetch: Fetch,
    cancel: &AtomicBool,
    log: &dyn Fn(String),
    stop_on_error: bool,
) -> Result<(), String> {
    let errors = AtomicUsize::new(0);
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let failed = Mutex::new(None);
    DONE.store(0, Relaxed);
    TOTAL.store(files.len(), Relaxed);
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..jobs.clamp(1, files.len().max(1)))
            .map(|_| {
                scope.spawn(|| {
                    while let Some(file) = files.get(next.fetch_add(1, Relaxed)) {
                        if stop_on_error && recover(&failed).is_some() {
                            break;
                        }
                        if cancel.load(Relaxed) {
                            recover(&failed).get_or_insert(CANCELLED.to_string());
                            break;
                        }
                        if !file_ok(file) {
                            let got = match try_copy_from_sources(file, sources) {
                                Ok(true) => Ok(()),
                                Ok(false) => fetch_file(fetch, file)
                                    .map_err(|e| format!("{} {e}", file.url))
                                    .and_then(|body| write_file(&file.path, &body)),
                                Err(e) => Err(e),
                            };
                            if let Err(e) = got {
                                errors.fetch_add(1, Relaxed);
                                recover(&failed).get_or_insert(e);
                            }
                        }
                        DONE.store(done.fetch_add(1, Relaxed) + 1, Relaxed);
                    }
                })
            })
            .collect();
        let mut waited = 0;
        while !workers.iter().all(|w| w.is_finished()) {
            std::thread::sleep(Duration::from_millis(100));
            waited += 1;
            if waited % 10 == 0 {
                log(trf(
                    "{done} of {total} files",
                    &[
                        ("done", &done.load(Relaxed).to_string()),
                        ("total", &files.len().to_string()),
                    ],
                ));
            }
        }
    });
    TOTAL.store(0, Relaxed);
    // As recover() does for the workers: a worker that panicked leaves at most its error here.
    match failed.into_inner().unwrap_or_else(|e| e.into_inner()) {
        Some(e) if !stop_on_error && e != CANCELLED => Err(trf(
            "{failed} of {total} files failed, the first: {error}",
            &[
                ("failed", &errors.load(Relaxed).to_string()),
                ("total", &files.len().to_string()),
                ("error", e.as_str()),
            ],
        )),
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests;
