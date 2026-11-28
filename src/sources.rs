//! Where the player's own Minecraft files may already live: the folder they chose, the official
//! launcher's, or another launcher's (Prism Launcher, MultiMC, Modrinth App, ATLauncher,
//! GDLauncher, Dawn Client), #434.
//! Preparing always assembles the pinned version into Sunrise's own folder (`data_dir()/minecraft`)
//! from the build's pinned version file, copying files from these folders only when their hash
//! matches and downloading the rest from Mojang.

pub(crate) use crate::hashing::{sha1_file, sha1_hex};
use crate::i18n::{tr, trf};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// What a version's research-lock.json pins about the game files.
pub struct Pins {
    pub version: String,
    pub client_sha1: String,
    /// The version JSON's sha1, then the earlier ones Mojang republished.
    pub manifests: Vec<String>,
}

impl Pins {
    pub(crate) fn version_in(lock: &Value) -> Option<&str> {
        crate::browser::safe_name(lock["minecraft"]["version"].as_str()?)
    }

    pub fn read(lock: &Value) -> Option<Pins> {
        let game = &lock["minecraft"];
        let text = |key: &str| game[key].as_str().map(String::from);
        let mut manifests = vec![text("manifest_sha1")?];
        if let Some(list) = game["accepted_manifest_sha1"].as_array() {
            manifests.extend(list.iter().filter_map(Value::as_str).map(String::from));
        }
        Some(Pins {
            version: Self::version_in(lock)?.to_owned(),
            client_sha1: text("client_sha1")?,
            manifests,
        })
    }

    /// Any version from Mojang's version list, by its id and its version file's sha1 (the
    /// list's): the client's sha1 then comes from that checked version file.
    pub fn for_version(id: &str, manifest_sha1: &str) -> Pins {
        Pins {
            version: id.to_string(),
            client_sha1: String::new(),
            manifests: vec![manifest_sha1.to_string()],
        }
    }

    /// The primary version JSON's sha1 pinned in `research-lock.json`.
    pub(crate) fn primary_manifest_sha1(&self) -> &str {
        &self.manifests[0]
    }

    pub(crate) fn manifest_is_primary(&self, path: &Path) -> bool {
        sha1_file(path).as_deref() == Some(self.primary_manifest_sha1())
    }

    pub(crate) fn manifest_accepted(&self, path: &Path) -> bool {
        sha1_file(path).is_some_and(|sha1| self.manifests.contains(&sha1))
    }
}

/// Where a build's pinned Minecraft files are assembled: Sunrise's own folder (`dir`) and any
/// other launchers' base folders (`sources`) to copy hash-matching files from before downloading.
pub struct Store<'a> {
    pub dir: &'a Path,
    pub sources: &'a [PathBuf],
}

pub struct Folders {
    pub home: PathBuf,
    /// `dirs::data_dir()`: ~/.local/share (XDG_DATA_HOME), ~/Library/Application Support,
    /// %APPDATA%.
    pub data: PathBuf,
    /// `dirs::config_dir()`: ~/.config (XDG_CONFIG_HOME), the same as `data` elsewhere.
    pub config: PathBuf,
}

impl Folders {
    pub fn current() -> Option<Folders> {
        Some(Folders {
            home: dirs::home_dir()?,
            data: dirs::data_dir()?,
            config: dirs::config_dir()?,
        })
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

const OFFICIAL: &str = "the official launcher";

/// Every launcher's default game folder on `os`, the official launcher's first. MultiMC on
/// macOS and Windows has none (it lives where it was unzipped): it is chosen by hand.
pub fn known_folders(os: Os, f: &Folders) -> Vec<(&'static str, PathBuf)> {
    let flatpak = |app: &str, rest: &str| f.home.join(".var/app").join(app).join(rest);
    let mut list = match os {
        Os::Linux => vec![
            (OFFICIAL, f.home.join(".minecraft")),
            (OFFICIAL, flatpak("com.mojang.Minecraft", ".minecraft")),
            ("Prism Launcher", f.data.join("PrismLauncher")),
            (
                "Prism Launcher",
                flatpak("org.prismlauncher.PrismLauncher", "data/PrismLauncher"),
            ),
            ("MultiMC", f.data.join("multimc")),
            ("Modrinth App", f.data.join("ModrinthApp")),
            (
                "Modrinth App",
                flatpak("com.modrinth.ModrinthApp", "data/ModrinthApp"),
            ),
            ("ATLauncher", f.data.join("atlauncher")),
            (
                "ATLauncher",
                flatpak("com.atlauncher.ATLauncher", "data/atlauncher"),
            ),
            ("Dawn Client", f.home.join(".dawn")),
        ],
        Os::MacOs => vec![
            (OFFICIAL, f.data.join("minecraft")),
            ("Prism Launcher", f.data.join("PrismLauncher")),
            ("MultiMC", f.data.join("MultiMC")),
            ("Modrinth App", f.data.join("ModrinthApp")),
            ("ATLauncher", f.data.join("ATLauncher")),
            ("Dawn Client", f.data.join(".dawn")),
        ],
        Os::Windows => vec![
            (OFFICIAL, f.data.join(".minecraft")),
            ("Prism Launcher", f.data.join("PrismLauncher")),
            ("Modrinth App", f.data.join("ModrinthApp")),
            ("ATLauncher", f.data.join("ATLauncher")),
            ("Dawn Client", f.data.join(".dawn")),
        ],
    };
    // Names every system shares: the Modrinth App before its rename, GDLauncher and its
    // previous version (GDLauncher Next).
    list.extend([
        ("Modrinth App", f.data.join("com.modrinth.theseus")),
        ("GDLauncher", f.data.join("gdlauncher_carbon")),
        ("GDLauncher", f.config.join("gdlauncher_next")),
    ]);
    list
}

/// Folders that hold only a launcher's Java runtimes, not a game directory: searched for
/// Java and nowhere else (never the default game folder or an import source). The Microsoft
/// Store's Minecraft Launcher keeps its runtimes under its package's local cache.
pub fn java_only_folders(os: Os, f: &Folders) -> Vec<PathBuf> {
    match os {
        Os::Windows => vec![f
            .home
            .join("AppData/Local/Packages/Microsoft.4297127D64EC6_8wekyb3d8bbwe/LocalCache/Local")],
        _ => Vec::new(),
    }
}

/// The client jar's path in a Maven repository: `com.mojang:minecraft:26.3:client` is
/// `com/mojang/minecraft/26.3/minecraft-26.3-client.jar`.
pub fn maven_path(name: &str) -> Option<String> {
    let (name, extension) = name.split_once('@').unwrap_or((name, "jar"));
    let parts: Vec<&str> = name.split(':').collect();
    let (group, artifact, version) = match parts[..] {
        [group, artifact, version] | [group, artifact, version, _] => (group, artifact, version),
        _ => return None,
    };
    let classifier = parts.get(3).map(|c| format!("-{c}")).unwrap_or_default();
    let unsafe_part = |p: &str| p.is_empty() || p.contains(['/', '\\']) || p.contains("..");
    if parts.iter().any(|p| unsafe_part(p)) || unsafe_part(extension) {
        return None;
    }
    Some(format!(
        "{}/{artifact}/{version}/{artifact}-{version}{classifier}.{extension}",
        group.replace('.', "/")
    ))
}

/// The client jar Prism Launcher's and MultiMC's metadata name (`meta/net.minecraft/<v>.json`,
/// their own format, `mainJar` a Maven name in `libraries`).
fn meta_client_jar(base: &Path, version: &str) -> Option<String> {
    let meta = fs::read_to_string(base.join(format!("meta/net.minecraft/{version}.json"))).ok()?;
    let meta: Value = serde_json::from_str(&meta).ok()?;
    maven_path(meta["mainJar"]["name"].as_str()?).map(|path| format!("libraries/{path}"))
}

/// Where launchers keep the client jar, relative to the folder holding their `libraries`.
pub(crate) fn client_jars(base: &Path, version: &str) -> Vec<String> {
    let v = version;
    let mut jars: Vec<String> = meta_client_jar(base, v).into_iter().collect();
    for jar in [
        // The official launcher; the Modrinth App, in meta/.
        format!("versions/{v}/{v}.jar"),
        // Prism Launcher and MultiMC.
        format!("libraries/com/mojang/minecraft/{v}/minecraft-{v}-client.jar"),
        // ATLauncher and GDLauncher.
        format!("libraries/net/minecraft/client/{v}/client-{v}.jar"),
    ] {
        if !jars.contains(&jar) {
            jars.push(jar);
        }
    }
    jars.extend(loader_jars(base, version));
    jars
}

/// The client jars the Modrinth App keeps for a mod loader's profile: `versions/<v>-<loader>/`,
/// the vanilla jar under the loader's name, when only a Fabric or Quilt profile of the version
/// was ever played (#1783). The jar is checked against the pin like the others.
fn loader_jars(base: &Path, version: &str) -> Vec<String> {
    let prefix = format!("{version}-");
    let mut jars: Vec<String> = fs::read_dir(base.join("versions"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with(&prefix))
        .map(|name| format!("versions/{name}/{name}.jar"))
        .collect();
    jars.sort();
    jars
}

/// The Prism or MultiMC launcher root above `dir` (#693). Their instances live in
/// `instances/<name>/` (Prism's game folder is `minecraft`, MultiMC's `.minecraft`) and the
/// shared `libraries` and assets sit in the launcher root above `instances`, two or three
/// folders above what players naturally pick.
fn instance_root(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|a| a.file_name() == Some(std::ffi::OsStr::new("instances")))
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

/// The Dawn Client root above `dir`: Dawn stores its shared official cache in `cache/minecraft/`
/// directly below `.dawn/`.
fn dawn_root(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|a| a.file_name() == Some(std::ffi::OsStr::new(".dawn")))
        .map(Path::to_path_buf)
}

/// Maps an official native library relative path under `libraries/` to Prism Launcher's layout:
/// `<group>/<artifact>/<version>/<artifact>-<version>-natives-<os>.jar` becomes
/// `<group>/<artifact>-natives-<os>/<version>/<artifact>-natives-<os>-<version>.jar`.
pub(crate) fn prism_native_path(rel: &str) -> Option<String> {
    let (dir, file) = rel.rsplit_once('/')?;
    let (artifact_dir, version) = dir.rsplit_once('/')?;
    let (group, artifact) = artifact_dir.rsplit_once('/')?;
    let prefix = format!("{artifact}-{version}-natives-");
    let os = file.strip_prefix(&prefix)?.strip_suffix(".jar")?;
    if os.is_empty() {
        return None;
    }
    Some(format!(
        "{group}/{artifact}-natives-{os}/{version}/{artifact}-natives-{os}-{version}.jar"
    ))
}

/// Every existing base folder (holding `versions`, `libraries` or `assets`) under `chosen` (its
/// ancestors and any Prism/MultiMC or Dawn root above it) and `known`, excluding `own` so Sunrise
/// never copies from its own store into itself.
pub fn candidate_bases(
    chosen: Option<&Path>,
    known: &[(&'static str, PathBuf)],
    own: &Path,
) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(dir) = chosen {
        roots.extend(dir.ancestors().take(4).map(Path::to_path_buf));
        if let Some(root) = instance_root(dir).or_else(|| dawn_root(dir)) {
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
    }
    roots.extend(known.iter().map(|(_, dir)| dir.clone()));
    let same = |a: &Path, b: &Path| {
        a == b
            || fs::canonicalize(a)
                .ok()
                .zip(fs::canonicalize(b).ok())
                .is_some_and(|(x, y)| x == y)
    };
    let mut bases: Vec<PathBuf> = Vec::new();
    for root in roots.into_iter().filter(|d| d.is_dir()) {
        for base in [root.clone()]
            .into_iter()
            .chain(["meta", "data", "datastore", "cache/minecraft"].map(|sub| root.join(sub)))
        {
            if base.is_dir() && !same(&base, own) && !bases.iter().any(|seen| same(seen, &base)) {
                bases.push(base);
            }
        }
    }
    bases
}

pub(crate) fn missing_version_file(path: &Path) -> String {
    trf(
        "Minecraft version file not found: {path}",
        &[("path", &path.to_string_lossy())],
    )
}

pub(crate) fn prepare_advice() -> String {
    format!(
        "\n  • {}\n  • {}",
        tr("Choose an existing Minecraft folder with --minecraft PATH on the command line, or Minecraft folder › Choose… in the app."),
        tr("On a computer with no Minecraft install, run sunrise-launcher account sign-in, then sunrise-launcher prepare --download."),
    )
}

/// Fetches a URL's body (from several threads for a download).
pub type Fetch<'a> = &'a (dyn Fn(&str) -> Result<Vec<u8>, String> + Sync);

/// The primary pinned version JSON at `path`, copied from `sources` when one matches the primary
/// hash, else fetched from piston-meta by its primary sha1.
pub(crate) fn fetch_manifest(
    pins: &Pins,
    path: &Path,
    sources: &[PathBuf],
    fetch: Fetch,
    log: &dyn Fn(String),
) -> Result<(), String> {
    if pins.manifest_is_primary(path) {
        return Ok(());
    }
    let (v, sha1) = (&pins.version, pins.primary_manifest_sha1());
    for base in sources {
        let candidate = base.join("versions").join(v).join(format!("{v}.json"));
        if copy_checked(&candidate, path, sha1, None)? {
            return Ok(());
        }
    }
    log(trf(
        "Downloading the {version} version file from Mojang",
        &[("version", v)],
    ));
    let url = format!("https://piston-meta.mojang.com/v1/packages/{sha1}/{v}.json");
    let body = fetch_checked(fetch, &url, sha1).map_err(|e| {
        trf(
            "Mojang's {version} version file {error}",
            &[("version", v), ("error", &e.to_string())],
        )
    })?;
    write_file(path, &body)
}

/// A download checked against its sha1; the error says how it differs.
pub(crate) fn fetch_checked(fetch: Fetch, url: &str, sha1: &str) -> Result<Vec<u8>, String> {
    let body = fetch(url)?;
    match sha1_hex(&body) {
        actual if actual == sha1 => Ok(body),
        actual => Err(format!("is not the one Sunrise pins (sha1 {actual})")),
    }
}

/// Writes a file whole through a unique sibling temporary file: a partial one never takes its
/// place and concurrent preparations never collide on a fixed `.partial` name.
pub(crate) fn write_file(path: &Path, data: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let error = |e: std::io::Error| format!("{}: {e}", path.display());
    let not_a_file = || {
        trf(
            "{path}: not a file path",
            &[("path", &path.display().to_string())],
        )
    };
    let (folder, name) = path.parent().zip(path.file_name()).ok_or_else(not_a_file)?;
    fs::create_dir_all(folder).map_err(error)?;
    let mut partial = tempfile::Builder::new()
        .prefix(&format!(".{}.", name.to_string_lossy()))
        .suffix(".partial")
        .tempfile_in(folder)
        .map_err(error)?;
    partial.write_all(data).map_err(error)?;
    partial
        .persist(path)
        .map(|_| ())
        .map_err(|e| error(e.error))
}

/// Copies `from` to `to` in one pass through a unique sibling temporary file, persisting only when
/// both `expected_size` (if given) and `expected_sha1` match. Returns `Ok(false)` when `from` is
/// absent, unreadable or does not match; returns `Err` only on destination write failure.
pub(crate) fn copy_checked(
    from: &Path,
    to: &Path,
    expected_sha1: &str,
    expected_size: Option<u64>,
) -> Result<bool, String> {
    use sha1::{Digest, Sha1};
    use std::io::{Read, Write};
    let Ok(meta) = fs::metadata(from) else {
        return Ok(false);
    };
    if !meta.is_file() || expected_size.is_some_and(|want| meta.len() != want) {
        return Ok(false);
    }
    let Ok(mut src) = fs::File::open(from) else {
        return Ok(false);
    };
    let error = |e: std::io::Error| format!("{}: {e}", to.display());
    let not_a_file = || {
        trf(
            "{path}: not a file path",
            &[("path", &to.display().to_string())],
        )
    };
    let (folder, name) = to.parent().zip(to.file_name()).ok_or_else(not_a_file)?;
    fs::create_dir_all(folder).map_err(error)?;
    let mut partial = tempfile::Builder::new()
        .prefix(&format!(".{}.", name.to_string_lossy()))
        .suffix(".partial")
        .tempfile_in(folder)
        .map_err(error)?;
    let mut hasher = Sha1::new();
    let mut buf = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let Ok(n) = src.read(&mut buf) else {
            return Ok(false);
        };
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        partial.write_all(&buf[..n]).map_err(error)?;
        total += n as u64;
    }
    if expected_size.is_some_and(|want| total != want) {
        return Ok(false);
    }
    let actual: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != expected_sha1 {
        return Ok(false);
    }
    partial
        .persist(to)
        .map(|_| true)
        .map_err(|e| error(e.error))
}

/// Safely removes a legacy `linked-minecraft` folder left by an earlier launcher version,
/// unlinking `libraries` and `assets` symlinks/junctions first so target launcher folders are
/// never touched.
pub(crate) fn remove_legacy_linked(linked: &Path) {
    if fs::symlink_metadata(linked).is_err() {
        return;
    }
    for dir in ["libraries", "assets"] {
        let _ = unlink(&linked.join(dir));
    }
    let _ = fs::remove_dir_all(linked);
}

/// Removes a link left by an earlier preparation; never a real folder's contents.
fn unlink(path: &Path) -> Result<(), String> {
    if fs::symlink_metadata(path).is_err() {
        return Ok(());
    }
    // A Windows junction, or a directory symlink, is removed as a directory.
    fs::remove_file(path)
        .or_else(|_| fs::remove_dir(path))
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(any(windows, test))]
const JUNCTION_LINK: &str = "SUNRISE_JUNCTION_LINK";
#[cfg(any(windows, test))]
const JUNCTION_TARGET: &str = "SUNRISE_JUNCTION_TARGET";

/// What `cmd` is run with to make a junction: the arguments cmd gets as they are (`plain`),
/// the ones written to its command line unquoted-by-Rust (`raw`) and the environment.
///
/// `/D` skips cmd's AutoRun registry command (for example, ANSICon), which can otherwise
/// change or fail the junction command before it runs. The paths travel in environment
/// variables, never in the command line: cmd reads `&`, `^` or `(` in a path (a user folder
/// may have them) as its own syntax, and expands `%NAME%` even inside quotes, but it does
/// not read what a variable holds again.
#[cfg(any(windows, test))]
struct JunctionCommand {
    plain: Vec<String>,
    raw: Vec<String>,
    env: Vec<(&'static str, PathBuf)>,
}

#[cfg(any(windows, test))]
fn junction_command(target: &Path, link: &Path) -> JunctionCommand {
    JunctionCommand {
        plain: ["/D", "/C", "mklink", "/J"].map(String::from).into(),
        raw: vec![
            format!("\"%{JUNCTION_LINK}%\""),
            format!("\"%{JUNCTION_TARGET}%\""),
        ],
        env: vec![
            (JUNCTION_LINK, link.to_path_buf()),
            (JUNCTION_TARGET, target.to_path_buf()),
        ],
    }
}

/// Makes `link` a junction to the folder `target` (no admin rights needed, unlike a symlink). The
/// error carries cmd's own message ("Access is denied", "The system cannot find the path…").
#[cfg(windows)]
pub fn junction(target: &Path, link: &Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    let line = junction_command(target, link);
    let mut cmd = crate::process::command("cmd");
    cmd.args(&line.plain);
    for arg in &line.raw {
        cmd.raw_arg(arg);
    }
    let out = cmd.envs(line.env).output()?;
    if out.status.success() {
        return Ok(());
    }
    let why = String::from_utf8_lossy(&out.stderr);
    Err(std::io::Error::other(match why.trim() {
        "" => "mklink /J failed".to_string(),
        why => format!("mklink /J failed: {why}"),
    }))
}

#[cfg(test)]
mod tests;
