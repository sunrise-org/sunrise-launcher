//! Everything that is not UI: GitHub, installs, resource preparation, the game's processes and
//! updates. The GUI, the CLI and the MCP server all go through this module. Functions here
//! block; the GUI runs them on threads and reads their messages.
use crate::process::child::Child;
use crate::{account, mojang, sources};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

/// Where the game's releases, Nightlies, snapshots and mod SDKs are published, and its bugs reported.
pub const GAME_REPO: &str = "sunrise-org/sunrise";
/// This launcher's own repository: its releases (what it updates itself from) and links.json.
pub const LAUNCHER_REPO: &str = sunrise_launcher_release_verify::LAUNCHER_REPOSITORY;

/// Where a crash is reported.
pub fn issues_url() -> String {
    format!("https://github.com/{GAME_REPO}/issues")
}
pub const README_URL: &str = "https://github.com/sunrise-org/sunrise-launcher/blob/main/README.md";
/// This launcher's version: the release workflow sets `SUNRISE_LAUNCHER_VERSION` from the
/// tag it builds (no version-bump commits); other builds say Cargo.toml's.
pub const VERSION: &str = version_or(option_env!("SUNRISE_LAUNCHER_VERSION"));
/// Launcher releases are tagged `vX.Y.Z` in [`LAUNCHER_REPO`]; before the launcher had its own repository they were `launcher@X.Y.Z` in the game's.
pub const LAUNCHER_TAG_PREFIX: &str = "v";

const fn version_or(injected: Option<&'static str>) -> &'static str {
    match injected {
        Some(v) if !v.is_empty() => v,
        _ => env!("CARGO_PKG_VERSION"),
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Release {
    pub tag: String,
    pub name: String,
    pub body: String,
    pub date: String,
    /// When it was published, `HH:MM` UTC (empty if GitHub didn't say).
    pub time: String,
    pub asset: Option<(String, u64)>,
    /// Listed and installed by hand, never as an update: a GitHub pre-release of the old Alpha
    /// channel (v0.2.1 on), or a Nightly older than the current one.
    pub prerelease: bool,
    /// A Nightly's GitHub tag (`nightly`, or `nightly-<date>` for the last week's): its files and
    /// signature are there, while `tag` is the version it installs as (`nightly-2026-10-28-0c02067`).
    /// None for a release.
    pub nightly: Option<String>,
}

mod look_settings;
mod play_log;
mod release_assets;
mod release_notes;
mod settings_io;
pub use release_assets::PLATFORMS;
pub use release_notes::{markdown_to_text, notes_summary};
pub(crate) use settings_io::NativeGrant;
pub use settings_io::SettingsBaseline;

/// `a == b`, in a const context.
const fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// This system's entry of `PLATFORMS`: the game's asset and the launcher's.
const fn platform_assets(os: &str, arch: &str) -> Option<(&'static str, &'static str)> {
    let mut i = 0;
    while i < PLATFORMS.len() {
        let (o, a, game, launcher) = PLATFORMS[i];
        if same(o, os) && same(a, arch) {
            return Some((game, launcher));
        }
        i += 1;
    }
    None
}

/// Both follow this launcher's own architecture. An x64 launcher on a Windows ARM64 PC (under
/// emulation) keeps updating itself, installing games and building native mods as x64, so a player
/// whose emulated setup works is never moved to an ARM64 build by an update. The ARM64 launcher is
/// what `sunrise-launcher-setup-arm64.exe` installs.
const OWN_ASSETS: Option<(&str, &str)> =
    platform_assets(std::env::consts::OS, std::env::consts::ARCH);
/// The game build this launcher installs, None where Sunrise has none.
pub const ASSET: Option<&str> = match OWN_ASSETS {
    Some((game, _)) => Some(game),
    None => None,
};
/// The launcher build this launcher updates itself from.
pub const LAUNCHER_ASSET: Option<&str> = match OWN_ASSETS {
    Some((_, launcher)) => Some(launcher),
    None => None,
};
/// The game build the ARM64 Windows launcher installs from a release that has no ARM64 one (every
/// release before it, or a Nightly whose ARM64 job failed): the x64 build, which Windows runs under
/// emulation.
const GAME_FALLBACK: Option<&str> = game_fallback_for(std::env::consts::OS, std::env::consts::ARCH);

/// The game architecture selected in Settings. Auto preserves the launcher's existing behavior:
/// it follows the process architecture, even when an x64 launcher runs on Windows ARM.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GameArchitecture {
    #[default]
    Auto,
    X64,
    Arm64,
}

impl GameArchitecture {
    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::X64 => "x64",
            Self::Arm64 => "arm64",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" | "default" => Some(Self::Auto),
            "x64" | "x86_64" | "amd64" => Some(Self::X64),
            "arm64" | "aarch64" => Some(Self::Arm64),
            _ => None,
        }
    }
}

pub(crate) const GAME_BUILD_ARCHITECTURE: &str = ".sunrise-game-architecture.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct InstalledGameArchitecture {
    /// The architecture requested when this version was installed.
    requested: GameArchitecture,
    /// The archive's actual architecture (x64 may be the safe ARM64 fallback).
    binary: GameArchitecture,
}

fn architecture_for_platform(architecture: &str) -> Option<GameArchitecture> {
    match architecture {
        "x86_64" => Some(GameArchitecture::X64),
        "aarch64" => Some(GameArchitecture::Arm64),
        _ => None,
    }
}

fn architecture_for_asset(asset: &str) -> Option<GameArchitecture> {
    match asset.rsplit('/').next()? {
        "sunrise-windows-x64.zip" => Some(GameArchitecture::X64),
        "sunrise-windows-arm64.zip" => Some(GameArchitecture::Arm64),
        _ => None,
    }
}

/// Detect the native Windows machine even if this launcher process is running under emulation.
#[cfg(windows)]
fn native_windows_architecture() -> Option<&'static str> {
    #[repr(C)]
    struct SystemInfo {
        processor_architecture: u16,
        _reserved: u16,
        _page_size: u32,
        _minimum_application_address: *mut std::ffi::c_void,
        _maximum_application_address: *mut std::ffi::c_void,
        _active_processor_mask: usize,
        _number_of_processors: u32,
        _processor_type: u32,
        _allocation_granularity: u32,
        _processor_level: u16,
        _processor_revision: u16,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn GetModuleHandleA(module_name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
        fn GetProcAddress(
            module: *mut std::ffi::c_void,
            name: *const std::ffi::c_char,
        ) -> Option<unsafe extern "system" fn()>;
        fn GetNativeSystemInfo(info: *mut SystemInfo);
    }

    type IsWow64Process2 =
        unsafe extern "system" fn(*mut std::ffi::c_void, *mut u16, *mut u16) -> i32;

    // IsWow64Process2 reports the ARM64 host for x64 emulation; GetNativeSystemInfo reports the
    // emulated processor for compatibility. Resolve it dynamically for older Windows versions.
    let kernel32 = unsafe { GetModuleHandleA(c"kernel32.dll".as_ptr()) };
    let is_wow64_process2 = if kernel32.is_null() {
        None
    } else {
        unsafe { GetProcAddress(kernel32, c"IsWow64Process2".as_ptr()) }
    };
    if let Some(function) = is_wow64_process2 {
        // SAFETY: GetProcAddress resolved this exact Kernel32 API, whose signature is declared
        // above with the documented system calling convention.
        let is_wow64_process2: IsWow64Process2 = unsafe { std::mem::transmute(function) };
        let (mut process_machine, mut native_machine) = (0u16, 0u16);
        // SAFETY: GetCurrentProcess returns a pseudo-handle accepted by IsWow64Process2, and both
        // output pointers refer to live USHORTs.
        if unsafe {
            is_wow64_process2(
                GetCurrentProcess(),
                &mut process_machine,
                &mut native_machine,
            )
        } != 0
        {
            return match native_machine {
                0x8664 => Some("x86_64"),  // IMAGE_FILE_MACHINE_AMD64
                0xaa64 => Some("aarch64"), // IMAGE_FILE_MACHINE_ARM64
                _ => None,
            };
        }
    }

    // SAFETY: GetNativeSystemInfo writes the documented SYSTEM_INFO layout into this repr(C)
    // buffer and has no failure return value. This fallback preserves existing behavior on older
    // Windows versions without IsWow64Process2.
    let mut info: SystemInfo = unsafe { std::mem::zeroed() };
    unsafe { GetNativeSystemInfo(&mut info) };
    match info.processor_architecture {
        9 => Some("x86_64"),   // PROCESSOR_ARCHITECTURE_AMD64
        12 => Some("aarch64"), // PROCESSOR_ARCHITECTURE_ARM64
        _ => None,
    }
}

#[cfg(not(windows))]
fn native_windows_architecture() -> Option<&'static str> {
    None
}

fn supports_game_architecture_choice(os: &str, native: Option<&str>) -> bool {
    os == "windows" && native == Some("aarch64")
}

/// Resolves the game's asset and fallback. Auto follows the process architecture, preserving
/// existing x64, Linux and macOS behavior. Manual choices are honored on native Windows ARM.
fn game_assets_for(
    os: &str,
    native: Option<&str>,
    process: &str,
    choice: GameArchitecture,
) -> Option<(&'static str, Option<&'static str>)> {
    let arch = if supports_game_architecture_choice(os, native) {
        match choice {
            GameArchitecture::Auto => process,
            GameArchitecture::X64 => "x86_64",
            GameArchitecture::Arm64 => "aarch64",
        }
    } else {
        process
    };
    let (asset, _) = platform_assets(os, arch)?;
    Some((asset, game_fallback_for(os, arch)))
}

fn selected_game_architecture(settings: &Settings) -> Option<GameArchitecture> {
    if std::env::consts::OS != "windows" {
        return None;
    }
    let arch = if supports_game_architecture_choice("windows", native_windows_architecture()) {
        match settings.game_architecture {
            GameArchitecture::Auto => std::env::consts::ARCH,
            GameArchitecture::X64 => "x86_64",
            GameArchitecture::Arm64 => "aarch64",
        }
    } else {
        std::env::consts::ARCH
    };
    architecture_for_platform(arch)
}

/// True for both native ARM64 launchers and x64 launchers under ARM64 emulation.
pub fn can_choose_game_architecture() -> bool {
    supports_game_architecture_choice("windows", native_windows_architecture())
}

/// A downloaded version records its requested and actual architectures. Old installs and imports
/// have no marker; their PE machine type is used for both.
fn installed_game_architecture(tag: &str) -> Option<InstalledGameArchitecture> {
    let dir = version_dir(tag);
    serde_json::from_slice(&fs::read(dir.join(GAME_BUILD_ARCHITECTURE)).ok()?)
        .ok()
        .or_else(|| {
            let binary = pe_architecture_file(&dir.join(CLIENT))?;
            Some(InstalledGameArchitecture {
                requested: binary,
                binary,
            })
        })
}

fn pe_architecture_file(path: &Path) -> Option<GameArchitecture> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = fs::File::open(path).ok()?;
    let mut dos = [0u8; 0x40];
    file.read_exact(&mut dos).ok()?;
    if &dos[..2] != b"MZ" {
        return None;
    }
    let pe = u32::from_le_bytes(dos[0x3c..0x40].try_into().ok()?);
    file.seek(SeekFrom::Start(u64::from(pe))).ok()?;
    let mut header = [0u8; 6];
    file.read_exact(&mut header).ok()?;
    (&header[..4] == b"PE\x00\x00")
        .then(|| architecture_for_pe_machine(u16::from_le_bytes([header[4], header[5]])))
        .flatten()
}

fn architecture_for_pe_machine(machine: u16) -> Option<GameArchitecture> {
    match machine {
        0x8664 => Some(GameArchitecture::X64),
        0xaa64 => Some(GameArchitecture::Arm64),
        _ => None,
    }
}

fn installation_matches_game_architecture(tag: &str, settings: &Settings) -> bool {
    if !can_choose_game_architecture() {
        return true;
    }
    let Some(target) = selected_game_architecture(settings) else {
        return true;
    };
    installed_game_architecture(tag)
        .is_some_and(|installed| installed_architecture_matches(target, installed))
}

fn installed_architecture_matches(
    target: GameArchitecture,
    installed: InstalledGameArchitecture,
) -> bool {
    installed.requested == target || installed.binary == target
}

fn game_architecture_install_needed(
    target: GameArchitecture,
    expected: GameArchitecture,
    installed: Option<InstalledGameArchitecture>,
) -> bool {
    installed.is_none_or(|installed| {
        (installed.requested != target && installed.binary != target)
            || installed.binary != expected
    })
}

/// Whether an installed version needs the selected architecture downloaded. This also refreshes
/// old installs with a same-version install when needed to record an ARM64 fallback choice.
pub fn needs_game_architecture_install(tag: &str, release: &Release) -> bool {
    if !can_choose_game_architecture() {
        return false;
    }
    let Some(expected) = release
        .asset
        .as_ref()
        .and_then(|(url, _)| architecture_for_asset(url))
    else {
        return false;
    };
    let settings = Settings::load();
    let Some(target) = selected_game_architecture(&settings) else {
        return false;
    };
    game_architecture_install_needed(target, expected, installed_game_architecture(tag))
}

fn native_mod_architecture_message(
    process: GameArchitecture,
    selected: GameArchitecture,
) -> Option<&'static str> {
    (process != selected).then_some(if process == GameArchitecture::X64 {
        "Native mods cannot build for the selected ARM64 game yet. The available compiler builds x64 only."
    } else {
        "Native mods cannot build for the selected x64 game yet. This ARM64 launcher has no matching compiler."
    })
}

pub(crate) fn native_mod_architecture_error() -> Option<String> {
    if !can_choose_game_architecture() {
        return None;
    }
    let process = architecture_for_platform(std::env::consts::ARCH)?;
    let selected = selected_game_architecture(&Settings::load())?;
    native_mod_architecture_message(process, selected)
        .map(|message| crate::i18n::tr(message).to_string())
}

/// Refuse to launch an explicit game architecture until this version has been installed for it.
pub fn game_architecture_launch_error(tag: &str) -> Option<String> {
    if !can_choose_game_architecture() {
        return None;
    }
    let settings = Settings::load();
    if settings.game_architecture == GameArchitecture::Auto
        || installation_matches_game_architecture(tag, &settings)
    {
        return None;
    }
    let target = selected_game_architecture(&settings)?;
    let installed = installed_game_architecture(tag).map(|a| a.binary);
    let label = |arch| match arch {
        GameArchitecture::X64 => "x64".to_owned(),
        GameArchitecture::Arm64 => "ARM64".to_owned(),
        GameArchitecture::Auto => crate::i18n::tr("Automatic").to_string(),
    };
    let installed = installed
        .map(label)
        .unwrap_or_else(|| crate::i18n::tr("Unknown").to_string());
    let selected = label(target);
    Some(crate::i18n::trf(
        "This version has a {installed} game build, but Settings selects {selected}. Install the selected build with the version picker beside Play on Home before launching.",
        &[("installed", &installed), ("selected", &selected)],
    ))
}

/// The asset a platform's launcher falls back to when a release has no build of its own.
const fn game_fallback_for(os: &str, arch: &str) -> Option<&'static str> {
    if same(os, "windows") && same(arch, "aarch64") {
        match platform_assets("windows", "x86_64") {
            Some((game, _)) => Some(game),
            None => None,
        }
    } else {
        None
    }
}

/// The launcher as an AppImage (`packaging/linux/appimage.sh`): the program and its libraries in
/// one file. A launcher running from one updates by replacing that file ([`appimage_path`]), so it
/// needs this archive rather than the platform's `.tar.gz`.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub const LAUNCHER_APPIMAGE_ASSET: Option<&str> = Some("sunrise-launcher-linux-x86_64.AppImage");
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub const LAUNCHER_APPIMAGE_ASSET: Option<&str> = None;

const LAUNCHER_BINARY: &str = if cfg!(windows) {
    crate::identity::WINDOWS_EXECUTABLE
} else {
    crate::identity::EXECUTABLE
};

pub(crate) const CLIENT: &str = if cfg!(windows) {
    "sunrise-client.exe"
} else {
    "sunrise-client"
};

/// The agent `url` is fetched with: the launcher's own user agent, through the player's proxy when
/// one is set (`proxy`).
pub(crate) fn agent(url: &str) -> ureq::Agent {
    crate::proxy::agent(
        ureq::AgentBuilder::new()
            .user_agent(&format!("sunrise-launcher/{VERSION}"))
            // A connection that goes quiet fails instead of waiting for ever.
            .timeout_connect(std::time::Duration::from_secs(15))
            .timeout_read(std::time::Duration::from_secs(60)),
        url,
    )
    .build()
}

/// A file's body, from the network as the launcher reaches it.
pub fn get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    get(url)?
        .into_reader()
        .read_to_end(&mut body)
        .map_err(|e| e.to_string())?;
    Ok(body)
}

pub(crate) fn get(url: &str) -> Result<ureq::Response, String> {
    get_unless_missing(url)?.ok_or_else(|| format!("{url} answered 404"))
}

/// [`get`], with `None` when the server answers 404: a file that may not exist (a prebuilt game).
pub(crate) fn get_unless_missing(url: &str) -> Result<Option<ureq::Response>, String> {
    let mut request = agent(url).get(url);
    // CI runners share addresses and hit GitHub's anonymous rate limit; they pass their own token.
    if let Some(token) = std::env::var("SUNRISE_GITHUB_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
    {
        if url.starts_with("https://api.github.com/") {
            request = request.set("Authorization", &format!("Bearer {token}"));
        }
    }
    match crate::download_retry::get(&request).map_err(|e| *e) {
        Ok(response) => Ok(Some(response)),
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(e) => Err(match e {
            ureq::Error::Status(code, r)
                if url.starts_with("https://api.github.com/")
                    && crate::github::rate_limited(code, |h| r.header(h).map(str::to_owned)) =>
            {
                "GitHub's rate limit was reached. Try again in a few minutes.".into()
            }
            ureq::Error::Status(code, _) => format!("{url} answered {code}"),
            ureq::Error::Transport(t) => format!("No connection: {t}"),
        }),
    }
}

/// A Sunrise game tag: `v` then a digit (`v2026.10.13`); a launcher `vX.Y.Z` matches too, but only the game repository's list is checked, and its launcher releases are `launcher@X.Y.Z`.
pub fn is_game_tag(tag: &str) -> bool {
    tag.strip_prefix('v')
        .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
}

/// The launcher version in a `vX.Y.Z` tag of [`LAUNCHER_REPO`], or None for any other tag: the verifier's own grammar, so discovery never takes a tag it would refuse.
pub fn launcher_version(tag: &str) -> Option<&str> {
    sunrise_launcher_release_verify::launcher_version(tag)
}

/// The tag this launcher was released as: release.yml gives it (`SUNRISE_LAUNCHER_TAG`); any other build is `v` and its version.
pub fn own_tag() -> String {
    own_tag_of(
        option_env!("SUNRISE_LAUNCHER_TAG"),
        option_env!("SUNRISE_LAUNCHER_VERSION"),
    )
}

fn own_tag_of(tag: Option<&str>, injected: Option<&str>) -> String {
    match (
        tag.filter(|t| !t.is_empty()),
        injected.filter(|v| !v.is_empty()),
    ) {
        (Some(tag), _) => tag.to_string(),
        (None, Some(version)) => format!("{LAUNCHER_TAG_PREFIX}{version}"),
        (None, None) => format!("{LAUNCHER_TAG_PREFIX}{}", env!("CARGO_PKG_VERSION")),
    }
}

/// Where a tag or version stands among Sunrise's releases (docs/versioning.md): the tags from
/// before 0.2 (`vYYYY.MM.DD[.N]`, `launcher-vX.Y.Z`) and bare versions first, compared number by
/// number with missing parts as zero and anything after a `-` ignored, as launchers always did;
/// then the 0.2 tags (`vX.Y.Z`, `launcher@X.Y.Z`) in semver's order, a pre-release
/// (`v0.3.0-rc.1`) before its release.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReleaseOrder {
    /// Numbers without trailing zeros, so 1.2 and 1.2.0 are equal.
    Before02(Vec<u64>),
    Semver([u64; 3], Prerelease),
    /// A Nightly (`nightly-<date>-<commit>`), by its date: newer than every release, since the
    /// Nightly is the only build Sunrise ships.
    Nightly(String),
}

/// (is a release, the pre-release's identifiers): a release follows every pre-release of its
/// version; identifiers compare one by one, numbers before words, a shorter list first.
pub type Prerelease = (bool, Vec<Identifier>);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Identifier {
    Number(u64),
    Word(String),
}

/// Semver's majors stay below this; a date tag's first number is a year.
const FIRST_YEAR: u64 = 1000;

pub fn release_order(tag: &str) -> ReleaseOrder {
    if let Some(date) = crate::nightly::date_of(tag) {
        return ReleaseOrder::Nightly(date.to_owned());
    }
    // The game repository's launcher tags (`launcher@X.Y.Z`, before that `launcher-vX.Y.Z`) still order among the rest.
    let semver = tag
        .strip_prefix("launcher@")
        .or_else(|| tag.strip_prefix(LAUNCHER_TAG_PREFIX))
        .and_then(semver_order);
    semver.unwrap_or_else(|| {
        let v = tag
            .strip_prefix("launcher-v")
            .or_else(|| tag.strip_prefix("launcher@"))
            .unwrap_or(tag);
        let v = v.strip_prefix('v').unwrap_or(v);
        let mut parts: Vec<u64> = v
            .split('-')
            .next()
            .unwrap_or("")
            .split('.')
            .map(|p| p.parse().unwrap_or(0))
            .collect();
        while parts.last() == Some(&0) {
            parts.pop();
        }
        ReleaseOrder::Before02(parts)
    })
}

/// `MAJOR.MINOR.PATCH[-PRE]` with a major below [`FIRST_YEAR`], or None.
fn semver_order(version: &str) -> Option<ReleaseOrder> {
    let (core, pre) = match version.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (version, None),
    };
    let numbers: Vec<u64> = core
        .split('.')
        .map(|n| n.parse().ok())
        .collect::<Option<_>>()?;
    let &[major, minor, patch] = numbers.as_slice() else {
        return None;
    };
    if major >= FIRST_YEAR {
        return None;
    }
    let pre = match pre {
        None => (true, Vec::new()),
        Some(pre) => (
            false,
            pre.split('.')
                .map(|id| {
                    (!id.is_empty()).then(|| {
                        id.parse()
                            .map_or_else(|_| Identifier::Word(id.to_string()), Identifier::Number)
                    })
                })
                .collect::<Option<_>>()?,
        ),
    };
    Some(ReleaseOrder::Semver([major, minor, patch], pre))
}

/// Compares two tags or versions by [`release_order`].
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    release_order(a).cmp(&release_order(b))
}

/// Every published release of the repository, both kinds, as GitHub's JSON (kept on disk with its
/// ETag, and read through a configured mirror past GitHub's rate limit: [`crate::github`]).
fn release_list() -> Result<serde_json::Value, String> {
    Ok(crate::github::json_optional(&format!(
        "https://api.github.com/repos/{GAME_REPO}/releases?per_page=100"
    ))?
    .unwrap_or_else(|| serde_json::json!([])))
}

/// The game's releases in `list` whose tag passes `keep`, newest first, with the asset named `asset`, downloaded from [`GAME_REPO`].
pub(crate) fn parse_releases(
    list: &serde_json::Value,
    asset: Option<&str>,
    keep: fn(&str) -> bool,
) -> Vec<Release> {
    parse_releases_from(list, asset, keep, crate::signed::asset_url)
}

/// This launcher's releases in `list` (from [`LAUNCHER_REPO`]), newest first, with the asset named `asset`, downloaded from there.
pub(crate) fn parse_launcher_releases(
    list: &serde_json::Value,
    asset: Option<&str>,
) -> Vec<Release> {
    parse_releases_from(
        list,
        asset,
        is_launcher_tag,
        crate::signed::launcher_asset_url,
    )
}

fn parse_releases_from(
    list: &serde_json::Value,
    asset: Option<&str>,
    keep: fn(&str) -> bool,
    url: fn(&str, &str) -> String,
) -> Vec<Release> {
    let mut out = Vec::new();
    for r in list.as_array().into_iter().flatten() {
        let tag = r["tag_name"].as_str().unwrap_or_default();
        if r["draft"].as_bool().unwrap_or(false) || !keep(tag) {
            continue;
        }
        let found = r["assets"].as_array().into_iter().flatten().find_map(|a| {
            let name = a["name"].as_str()?;
            (Some(name) == asset).then(|| Some((url(tag, name), a["size"].as_u64()?)))?
        });
        out.push(Release {
            tag: tag.into(),
            name: r["name"].as_str().unwrap_or_default().into(),
            body: r["body"].as_str().unwrap_or_default().replace("\r\n", "\n"),
            date: r["published_at"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(10)
                .collect(),
            time: r["published_at"]
                .as_str()
                .and_then(|d| d.get(11..16))
                .unwrap_or_default()
                .into(),
            asset: found,
            prerelease: r["prerelease"].as_bool().unwrap_or(false),
            nightly: None,
        });
    }
    out.sort_by(|a, b| compare_versions(&b.tag, &a.tag));
    out
}

fn is_launcher_tag(tag: &str) -> bool {
    launcher_version(tag).is_some()
}

/// Published Sunrise builds, newest first: the current Nightly (the only build Sunrise ships since
/// 2026-10-28, docs/release-flow.md), the last week's Nightlies, then the releases from before.
/// Drafts never reach the anonymous API, and launcher releases are left out.
pub fn releases() -> Result<Vec<Release>, String> {
    Ok(game_releases(&release_list()?))
}

/// The game's builds in `list`, newest first. A Nightly is listed under the version it installs as
/// ([`crate::nightly::version_of`]); only the current one (`nightly`) is an update.
pub(crate) fn game_releases(list: &serde_json::Value) -> Vec<Release> {
    let settings = Settings::load();
    let assets = game_assets_for(
        std::env::consts::OS,
        native_windows_architecture(),
        std::env::consts::ARCH,
        settings.game_architecture,
    );
    let (asset, fallback) = assets.map_or((ASSET, GAME_FALLBACK), |(asset, fallback)| {
        (Some(asset), fallback)
    });
    game_releases_with(list, asset, fallback)
}

/// [`game_releases`] for one platform's asset, and the one it installs from a release that has
/// none of its own ([`GAME_FALLBACK`]).
fn game_releases_with(
    list: &serde_json::Value,
    asset: Option<&str>,
    fallback: Option<&str>,
) -> Vec<Release> {
    let keep = |tag: &str| is_game_tag(tag) || sunrise_launcher_release_verify::is_nightly(tag);
    let mut out = parse_releases(list, asset, keep);
    if fallback.is_some() {
        // The same releases in the same order: only the asset looked for differs.
        for (r, other) in out.iter_mut().zip(parse_releases(list, fallback, keep)) {
            if r.asset.is_none() && r.tag == other.tag {
                r.asset = other.asset;
            }
        }
    }
    for r in &mut out {
        if sunrise_launcher_release_verify::is_nightly(&r.tag) {
            let version = crate::nightly::version_of(r);
            let tag = std::mem::replace(&mut r.tag, version);
            r.prerelease = tag != sunrise_launcher_release_verify::NIGHTLY;
            r.nightly = Some(tag);
        }
    }
    // Two Nightlies of one day: the current one first (the dated one is the day's earlier build).
    out.sort_by(|a, b| compare_versions(&b.tag, &a.tag).then(a.prerelease.cmp(&b.prerelease)));
    out
}

/// Check the launcher's own release feed. An explicit check must ask GitHub even if the
/// cache is fresh, and must not report old cached results as a successful check.
pub fn refresh_launcher_update() -> Result<Option<Release>, String> {
    Ok(launcher_update(&launcher_releases(
        crate::github::CachePolicy::Revalidate,
    )?))
}

/// This launcher's release feed, or why it couldn't be read.
fn launcher_releases(policy: crate::github::CachePolicy) -> Result<Vec<Release>, String> {
    launcher_releases_from(crate::github::json_optional_with_policy(
        &format!("https://api.github.com/repos/{LAUNCHER_REPO}/releases?per_page=100"),
        policy,
    ))
}

/// The releases in the launcher's release list: only a list counts as checked (a repository with no release answers an empty one), never a 404 or anything else.
fn launcher_releases_from(
    answer: Result<Option<serde_json::Value>, String>,
) -> Result<Vec<Release>, String> {
    match answer? {
        Some(list) if list.is_array() => Ok(parse_launcher_releases(&list, launcher_asset())),
        Some(_) => Err("The launcher's release list isn't a list.".into()),
        None => Err(format!("{LAUNCHER_REPO}'s release list is missing.")),
    }
}

/// The newest Sunrise release that has a build for this platform.
pub fn latest(releases: &[Release]) -> Option<&Release> {
    releases.iter().find(|r| r.asset.is_some())
}

/// The releases a launcher installs by itself: every one that is published as a release, so
/// GitHub's latest when it comes out. The pre-releases of the old Alpha channel are left out.
pub fn installable_updates(releases: &[Release]) -> Vec<Release> {
    releases.iter().filter(|r| !r.prerelease).cloned().collect()
}

/// How often a launcher left open looks for new releases: they come out several times a day.
pub const CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// The community and website links, from `launcher/links.json` on main, so they can change
/// without a launcher release. An empty address hides its button.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Links {
    pub discord: String,
    pub website: String,
    /// The fork project board.
    pub roadmap: String,
    /// The modding book.
    pub docs: String,
}

impl Default for Links {
    /// What the launcher uses until (or unless) it can read links.json.
    fn default() -> Self {
        serde_json::from_str(include_str!("../links.json"))
            .expect("embedded launcher links must match Links")
    }
}

impl Links {
    /// Reads links.json. A key that is missing keeps its default, an empty one hides the link,
    /// and text that is not a JSON object gives the defaults.
    pub fn parse(text: &str) -> Self {
        let mut links = Self::default();
        if let Ok(serde_json::Value::Object(map)) = serde_json::from_str(text) {
            for (key, slot) in [
                ("discord", &mut links.discord),
                ("website", &mut links.website),
                ("roadmap", &mut links.roadmap),
                ("docs", &mut links.docs),
            ] {
                if let Some(url) = map.get(key).and_then(|v| v.as_str()) {
                    *slot = url.trim().to_string();
                }
            }
        }
        links
    }
}

/// The links from main, or the built-in ones when offline or answered with anything but the file itself.
pub fn links() -> Links {
    let url = format!("https://raw.githubusercontent.com/{LAUNCHER_REPO}/main/links.json");
    // No redirect is followed: links.json comes from this host over HTTPS or not at all.
    let agent = crate::proxy::agent(
        ureq::AgentBuilder::new()
            .user_agent(&format!("sunrise-launcher/{VERSION}"))
            .timeout_connect(std::time::Duration::from_secs(15))
            .timeout_read(std::time::Duration::from_secs(60))
            .redirects(0),
        &url,
    )
    .build();
    agent
        .get(&url)
        .call()
        .ok()
        .filter(|r| r.status() == 200)
        .and_then(|r| r.into_string().ok())
        .map_or_else(Links::default, |text| Links::parse(&text))
}

/// The launcher's data folder; `SUNRISE_LAUNCHER_HOME` overrides it (for tests and scripts).
pub fn data_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("SUNRISE_LAUNCHER_HOME").filter(|h| !h.is_empty()) {
        return PathBuf::from(home);
    }
    if cfg!(test) {
        return crate::data_location::test_home().join("launcher");
    }
    // A location that does not open stands in as the default folders, as
    // `data_location::startup` makes it for the window: never a panic here.
    if let Ok(Some(roots)) = crate::data_location::configured_roots() {
        return roots.launcher;
    }
    crate::identity::launcher_folder_in(&dirs::data_dir().unwrap_or_else(std::env::temp_dir))
}

/// A canonical path without Windows' verbatim prefix: `canonicalize` gives `\\?\C:\...` there,
/// and some tools stat `\\?\C:` — the volume itself — and fail. A drive path loses its `\\?\`;
/// any other path is left as it is.
pub(crate) fn plain(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|p| p.strip_prefix(r"\\?\")) {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

/// `path` spelled only in ASCII: what Windows' MinGW-w64 tools can open (they read paths
/// through ANSI APIs, so a Cyrillic user name in %APPDATA% breaks them, #5189).
fn ascii(path: &Path) -> bool {
    path.as_os_str().to_str().is_some_and(str::is_ascii)
}

/// `path` as Windows' MinGW-w64 tools can take it: [`ascii`], and on Windows without a space.
/// `windres` hands its include folders to the preprocessor unquoted, so a user name with a space
/// in %APPDATA% splits there ("cc1.exe: fatal error: v\: No such file or directory").
fn tool_safe(path: &Path) -> bool {
    ascii(path) && !(cfg!(windows) && path.as_os_str().to_str().is_some_and(|p| p.contains(' ')))
}

/// Where builds happen — the Rust toolchain, its Cargo home and `native_mods::root` — as a
/// path every build tool can read: `data_dir()` resolved, so the tools are never handed a
/// link (horizon-mixins refuses one, and macOS's /tmp is one). On Windows, when the resolved
/// path isn't [`tool_safe`], [`windows_build_dir`] names the same folder a way MinGW-w64 can open.
pub fn build_dir() -> PathBuf {
    // Tests keep it inside their scratch launcher folder (`data_dir`).
    if !cfg!(test) && std::env::var_os("SUNRISE_LAUNCHER_HOME").is_none_or(|p| p.is_empty()) {
        // A location that does not open stands in as the default folders, as
        // `data_location::startup` makes it for the window: never a panic here.
        if let Ok(Some(roots)) = crate::data_location::configured_roots() {
            return roots.build;
        }
    }
    let data = data_dir();
    let _ = fs::create_dir_all(&data);
    let real = data.canonicalize().map(plain).unwrap_or(data);
    if tool_safe(&real) {
        return real;
    }
    #[cfg(windows)]
    return windows_build_dir(&real);
    #[cfg(not(windows))]
    real
}

/// [`build_dir`] when `real` isn't [`tool_safe`]: [`ascii_build_dir`], a real folder of
/// ProgramData's. Only when it can't be created, the data folder's short (8.3) name.
///
/// The short name came first until #6009, and wasn't enough: it is only another spelling of the
/// same folder, and rustc canonicalizes its own path to find its sysroot, which spells the long,
/// Cyrillic name again and hands it to the linker (`x86_64-w64-mingw32-gcc`: "No such file or
/// directory" on `C:/Users/\xca\xe8…`). A folder whose real name is ASCII stays ASCII however a
/// tool resolves it.
#[cfg(windows)]
fn windows_build_dir(real: &Path) -> PathBuf {
    let dir = ascii_build_dir(real, std::env::var_os("ProgramData").map(PathBuf::from));
    // ProgramData is writable by every account, so the folder may be another account's: its
    // `bin` and `cargo\config.toml` would run as this player. Only a folder made here, or owned
    // by this account, is used.
    // The parent is made with `create_dir_all` (ProgramData may be missing); only the final
    // component is made with `create_dir`, which fails on a folder that already exists.
    let claim = |dir: &Path| {
        if let Some(parent) = dir.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::create_dir(dir).is_ok() || owned_by_this_account(dir)
    };
    if let Some(dir) = first_claimed_dir(&dir, claim) {
        return dir;
    }
    // Never an unclaimed folder: the short name of the data folder, else a folder of this
    // process's own in the temporary folder.
    build_fallback(short_path(real), std::process::id(), &std::env::temp_dir())
}

/// The build folder when no numbered name could be claimed: `short` if it is [`tool_safe`],
/// else a folder named for the process under `temp`, which no other account made.
#[cfg_attr(not(windows), allow(dead_code))]
fn build_fallback(short: Option<PathBuf>, pid: u32, temp: &Path) -> PathBuf {
    short
        .filter(|p| tool_safe(p))
        .unwrap_or_else(|| temp.join(format!("sunrise-launcher-{pid}")))
}

/// `base`, else `base-2`, `base-3`… (up to `base-9`): the first one `claim` takes, which makes
/// the folder or finds it to be the caller's own. A folder someone else made is passed over.
#[cfg_attr(not(windows), allow(dead_code))]
fn first_claimed_dir(base: &Path, claim: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let name = base.file_name()?.to_os_string();
    std::iter::once(base.to_path_buf())
        .chain((2..=9).map(|n| {
            let mut next = name.clone();
            next.push(format!("-{n}"));
            base.with_file_name(next)
        }))
        .find(|dir| claim(dir))
}

/// Whether the folder `path` is owned by the account this process runs as: the owner a folder
/// it creates gets (the token's default owner). A folder the same player made in an elevated
/// session is owned by Administrators, so it is passed over and a numbered folder is made
/// instead: safe, but the toolchain is built again there.
#[cfg(windows)]
fn owned_by_this_account(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
    use windows_sys::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        EqualSid, GetTokenInformation, TokenOwner, OWNER_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR, PSID, TOKEN_OWNER, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: every pointer handed over is to a live local; the token and the descriptor
    // GetNamedSecurityInfoW allocates are released before returning, and the owner SID is only
    // read while that descriptor is alive.
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut buffer = vec![0u64; 128];
        let mut length = 0;
        let got = GetTokenInformation(
            token,
            TokenOwner,
            buffer.as_mut_ptr().cast(),
            (buffer.len() * 8) as u32,
            &mut length,
        );
        CloseHandle(token);
        if got == 0 {
            return false;
        }
        let ours: PSID = (*buffer.as_ptr().cast::<TOKEN_OWNER>()).Owner;
        let mut owner: PSID = std::ptr::null_mut();
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let status = GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut descriptor,
        );
        if status != 0 {
            return false;
        }
        let same = !owner.is_null() && EqualSid(owner, ours) != 0;
        LocalFree(descriptor);
        same
    }
}

/// The build folder for a data folder `real` whose path isn't [`ascii`]: in `program_data`
/// (`%ProgramData%`, `C:\ProgramData` when unset, relative or not ASCII), which every user may
/// create folders in, named for `real` so each user's is their own.
#[cfg_attr(not(windows), allow(dead_code))]
fn ascii_build_dir(real: &Path, program_data: Option<PathBuf>) -> PathBuf {
    use sha2::{Digest, Sha256};
    let public = program_data
        .filter(|p| p.is_absolute() && tool_safe(p))
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    let key = Sha256::digest(real.as_os_str().as_encoded_bytes());
    let key: String = key[..4].iter().map(|b| format!("{b:02x}")).collect();
    public.join(format!("sunrise-launcher-{key}"))
}

/// `GetShortPathNameW`: `path`'s short (8.3) name, or None on error. A folder with no short
/// name (the volume may keep none) answers with its long name again, which [`ascii`] refuses.
#[cfg(windows)]
fn short_path(path: &Path) -> Option<PathBuf> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    #[link(name = "kernel32")]
    extern "system" {
        fn GetShortPathNameW(long: *const u16, short: *mut u16, len: u32) -> u32;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let need = unsafe { GetShortPathNameW(wide.as_ptr(), std::ptr::null_mut(), 0) } as usize;
    if need == 0 {
        return None;
    }
    let mut buf = vec![0u16; need];
    let got = unsafe { GetShortPathNameW(wide.as_ptr(), buf.as_mut_ptr(), need as u32) } as usize;
    (got != 0 && got < need).then(|| PathBuf::from(std::ffi::OsString::from_wide(&buf[..got])))
}

fn versions_dir() -> PathBuf {
    data_dir().join("versions")
}

pub fn version_dir(tag: &str) -> PathBuf {
    versions_dir().join(tag)
}

const COMPLETE: &str = ".installed";

/// Installed versions, newest tag first.
pub fn installed() -> Vec<String> {
    installed_in(&versions_dir())
}

fn installed_in(versions: &Path) -> Vec<String> {
    let mut tags: Vec<String> = fs::read_dir(versions)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join(COMPLETE).exists())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    tags.sort_by(|a, b| compare_versions(b, a));
    tags
}

/// A small release file (its checksums or their signature), whole; None when it has none.
pub(crate) fn fetch_small(url: &str) -> Result<Option<Vec<u8>>, String> {
    let response = match get(url) {
        Ok(response) => response,
        Err(e) if e.ends_with("answered 404") => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut body = Vec::new();
    response
        .into_reader()
        .take(1 << 20)
        .read_to_end(&mut body)
        .map_err(|e| format!("Download interrupted: {e}"))?;
    Ok(Some(body))
}

/// The SHA-256 the release's signed checksums give for `asset`, or why it must not be installed.
fn signed_digest(tag: &str, asset: &str) -> Result<String, String> {
    crate::signed::digest(
        tag,
        asset,
        &sunrise_launcher_release_verify::trusted_keys_for(GAME_REPO),
        fetch_small,
    )
}

/// Downloads `url` into `to`; the file is kept only if its SHA-256 is `sha256`, the signed (or
/// pinned) one. A `size` of 0 takes the server's Content-Length for the progress.
pub(crate) fn download(
    url: &str,
    size: u64,
    sha256: &str,
    to: &Path,
    progress: &impl Fn(f32),
) -> Result<(), String> {
    let response = get(url)?;
    let size = match size {
        0 => response
            .header("Content-Length")
            .and_then(|n| n.parse().ok())
            .unwrap_or(0),
        n => n,
    };
    let name = url.rsplit('/').next().unwrap_or(url);
    save_checked(response.into_reader(), size, name, sha256, to, progress)
}

/// Saves `reader` as `to` once all of it has arrived and its SHA-256 is `sha256`: until then it
/// has a temporary name beside `to`, so `to` is never a partial or unchecked file.
pub(crate) fn save_checked(
    reader: impl Read,
    size: u64,
    name: &str,
    sha256: &str,
    to: &Path,
    progress: &impl Fn(f32),
) -> Result<(), String> {
    let mut temp = to.as_os_str().to_owned();
    temp.push(format!(".part-{}", std::process::id()));
    let temp = PathBuf::from(temp);
    let saved = write_checked(reader, size, name, sha256, &temp, progress)
        .and_then(|()| fs::rename(&temp, to).map_err(|e| e.to_string()));
    if saved.is_err() {
        let _ = fs::remove_file(&temp);
    }
    saved
}

fn write_checked(
    mut reader: impl Read,
    size: u64,
    name: &str,
    sha256: &str,
    to: &Path,
    progress: &impl Fn(f32),
) -> Result<(), String> {
    let mut file = fs::File::create(to).map_err(|e| e.to_string())?;
    let mut hash = sunrise_launcher_release_verify::Sha256::default();
    let (mut done, mut buf) = (0u64, vec![0u8; 1 << 16]);
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("Download interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        progress((done as f32 / size.max(1) as f32).min(1.0) * 0.9);
    }
    sunrise_launcher_release_verify::check(name, sha256, &hash.hex())
}

fn unpack(archive: &Path, url: &str, to: &Path) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    if url.ends_with(".tar.gz") {
        tar::Archive::new(flate2::read::GzDecoder::new(file))
            .unpack(to)
            .map_err(|e| e.to_string())
    } else {
        zip::ZipArchive::new(file)
            .and_then(|mut z| z.extract(to))
            .map_err(|e| e.to_string())
    }
}

/// Downloads one release, checks it against its signed checksums and unpacks it; `progress` gets
/// the fraction done.
pub fn install(release: &Release, progress: impl Fn(f32)) -> Result<(), String> {
    let download = GameDownload::select(release)?;
    let proof = download.prove(&release.tag)?;
    download.install(
        &release.tag,
        &proof,
        version_storage::Download::Requested,
        progress,
    )
}

/// The build's facts a launcher reads first (docs/api/release-file.md), signed beside its archives.
const RELEASE_FILE: &str = "sunrise-release.json";

/// [`RELEASE_FILE`]'s signed digest in `release`, published under the GitHub tag `source`.
fn release_file_digest(release: &Release, source: &str) -> Result<String, String> {
    match &release.nightly {
        Some(_) => crate::signed::build_digest(
            source,
            &release.tag,
            RELEASE_FILE,
            &sunrise_launcher_release_verify::trusted_keys_for(GAME_REPO),
            fetch_small,
        ),
        None => signed_digest(source, RELEASE_FILE),
    }
}

/// Keeps `bytes` as the installed build's [`RELEASE_FILE`] in `dir` when they are the signed `sha256`'s.
fn keep_release_file(dir: &Path, bytes: &[u8], sha256: &str) -> Result<(), String> {
    if sunrise_launcher_release_verify::sha256_hex(bytes) != sha256 {
        return Err(format!("{RELEASE_FILE}: not the signed file"));
    }
    let file = dir.join(RELEASE_FILE);
    let write = match fs::File::create_new(&file) {
        Ok(mut file) => file.write_all(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            if crate::native_cache::checked(&file, false).is_ok_and(|meta| meta.is_some())
                && fs::read(&file).is_ok_and(|archived| archived == bytes)
            {
                Ok(())
            } else {
                Err(e)
            }
        }
        Err(e) => Err(e),
    };
    write.map_err(|e| format!("{RELEASE_FILE}: {e}"))
}

/// The asset `release` installs from: the fallback when [`game_releases_with`] took it (its file
/// name ends the download address), so the signed checksum checked is that file's.
fn asset_installed<'a>(
    release: &Release,
    asset: Option<&'a str>,
    fallback: Option<&'a str>,
) -> Option<&'a str> {
    let file = release
        .asset
        .as_ref()
        .and_then(|(url, _)| url.rsplit('/').next());
    if fallback.is_some_and(|name| file == Some(name)) {
        fallback
    } else if asset.is_some_and(|name| file == Some(name)) {
        asset
    } else {
        None
    }
}

mod developer_play;
pub use developer_play::{client_as, managed_client_as};

/// Downloads `release`'s archive for this platform, checks it against `sha256` (its signed
/// checksum), and unpacks it as the installed version `tag`.
mod installation;
pub(crate) use installation::{GameDownload, GameProof, ARCHIVE_ID as BUILD_ARCHIVE_ID};

#[path = "backend/version_storage.rs"]
pub mod version_storage;
pub use version_storage::remove_version;

/// The Minecraft version a Sunrise build targets, from its research-lock.json.
pub fn game_version(tag: &str) -> String {
    game_version_at(&version_dir(tag))
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn game_version_at(dir: &Path) -> String {
    locked_game(dir).unwrap_or_else(|| PINNED_GAME.into())
}

fn locked_game(dir: &Path) -> Option<String> {
    read_json(&dir.join("research-lock.json"))?["minecraft"]["version"]
        .as_str()
        .map(String::from)
}

/// The Minecraft version played when no installed build says (the game's `VERSION_ID`).
pub const PINNED_GAME: &str = "26.3";

/// The Minecraft version the launcher plays (`WorldVersion.id()`), for the mod browser.
pub fn played_version_id() -> String {
    played("version_id")
}

/// The name of the Minecraft version the launcher plays (`WorldVersion.name()`).
pub fn played_vanilla() -> String {
    played("vanilla")
}

fn played(field: &str) -> String {
    played_by(None, field)
}

/// The build `selected` names when it is installed, else the newest installed: what the launcher plays, and what [`played_version_id`] reads.
pub fn launcher_build<'a>(selected: Option<&'a str>, installed: &'a [String]) -> Option<&'a str> {
    selected
        .filter(|s| installed.iter().any(|t| t == s))
        .or(installed.first().map(String::as_str))
}

/// The Minecraft version id the installed build `tag` plays (an instance's own build), else the launcher's.
pub fn played_version_id_by(tag: Option<&str>) -> String {
    played_by(tag, "version_id")
}

/// The Minecraft version id the build `tag` plays, from that build's own files; None when it
/// isn't installed (a build that isn't here says nothing about itself, and no other build
/// stands in for it).
pub fn game_of_build(tag: &str) -> Option<String> {
    game_in(&versions_dir(), tag)
}

fn game_in(versions: &Path, tag: &str) -> Option<String> {
    installed_in(versions)
        .iter()
        .any(|t| t == tag)
        .then_some(())?;
    let dir = versions.join(tag);
    read_json(&dir.join(RELEASE_FILE))
        .and_then(|release| release["version_id"].as_str().map(String::from))
        .or_else(|| locked_game(&dir))
}

fn played_by(tag: Option<&str>, field: &str) -> String {
    // Tests share one scratch launcher folder whose installed builds other tests change.
    if cfg!(test) {
        return PINNED_GAME.into();
    }
    let selected = Settings::load().selected;
    played_in(&versions_dir(), tag.or(selected.as_deref()), field)
}

/// The played build's sunrise-release.json `field`, else its locked Minecraft, else [`PINNED_GAME`].
fn played_in(versions: &Path, selected: Option<&str>, field: &str) -> String {
    let installed = installed_in(versions);
    launcher_build(selected, &installed)
        .map(|t| versions.join(t))
        .and_then(|dir| {
            read_json(&dir.join(RELEASE_FILE))
                .and_then(|release| release[field].as_str().map(String::from))
                .or_else(|| locked_game(&dir))
        })
        .unwrap_or_else(|| PINNED_GAME.into())
}

/// The version whose folder holds `tag`'s prepared resources: a native build shares its base's
/// (`copy_base` leaves `.local` out), any other version keeps its own.
pub fn resources_owner(tag: &str) -> String {
    crate::native_mods::about(tag).map_or_else(|| tag.to_owned(), |about| about.base)
}

pub fn resources(tag: &str) -> PathBuf {
    resources_at(&version_dir(&resources_owner(tag)))
}

fn resources_at(dir: &Path) -> PathBuf {
    dir.join(".local/sunrise-client").join(game_version_at(dir))
}

const RESOURCES_MANIFEST: &str = "sunrise-resources.json";

/// The target preparation records the world format of the verified client JAR it used.
pub(crate) fn prepared_world_version(resources: &Path) -> Option<i32> {
    let manifest = read_json(&resources.join(RESOURCES_MANIFEST))?;
    i32::try_from(manifest.get("data_version")?.as_i64()?).ok()
}

/// How long a drawn page believes an answer of [`is_prepared`] without looking at the disk again.
/// What the launcher itself changes (a preparation, an install, a removal) drops it at once
/// ([`forget_prepared`]); this only bounds how late a change made by another process shows.
const PREPARED_TTL: std::time::Duration = std::time::Duration::from_secs(5);

/// The answers of [`is_prepared`]: version folder, whether it was prepared, when that was checked.
#[derive(Default)]
struct PreparedCache(Vec<(PathBuf, bool, std::time::Instant)>);

impl PreparedCache {
    fn get(&self, dir: &Path, now: std::time::Instant) -> Option<bool> {
        let (_, prepared, at) = self.0.iter().find(|(d, ..)| d == dir)?;
        (now.saturating_duration_since(*at) < PREPARED_TTL).then_some(*prepared)
    }

    fn put(&mut self, dir: &Path, prepared: bool, now: std::time::Instant) {
        self.0.retain(|(d, ..)| d != dir);
        self.0.push((dir.to_path_buf(), prepared, now));
    }
}

static PREPARED: Mutex<PreparedCache> = Mutex::new(PreparedCache(Vec::new()));

fn lock_prepared(cache: &Mutex<PreparedCache>) -> std::sync::MutexGuard<'_, PreparedCache> {
    cache.lock().unwrap_or_else(|e| e.into_inner())
}

/// The answer kept for `dir`, else `check()` (run without the lock held), kept.
fn cached_prepared(
    cache: &Mutex<PreparedCache>,
    dir: &Path,
    now: std::time::Instant,
    check: impl FnOnce() -> bool,
) -> bool {
    if let Some(known) = lock_prepared(cache).get(dir, now) {
        return known;
    }
    let prepared = check();
    lock_prepared(cache).put(dir, prepared, now);
    prepared
}

/// Whether `tag` is prepared, as a page being drawn asks it (every frame, for each version it
/// shows): an answer stands for [`PREPARED_TTL`], so a frame does no file reads. Anything that
/// must not act on a stale answer (starting the game or a server) calls [`check_prepared`].
pub fn is_prepared(tag: &str) -> bool {
    cached_prepared(
        &PREPARED,
        &version_dir(tag),
        std::time::Instant::now(),
        || prepared_at(&resources(tag)),
    )
}

/// Whether `tag` is prepared, checked on the disk now: `research-lock.json`, the manifest and
/// every prepared table are read. The answer replaces the one [`is_prepared`] keeps.
pub fn check_prepared(tag: &str) -> bool {
    checked_prepared(
        &PREPARED,
        &version_dir(tag),
        &resources(tag),
        std::time::Instant::now(),
    )
}

fn checked_prepared(
    cache: &Mutex<PreparedCache>,
    dir: &Path,
    resources: &Path,
    now: std::time::Instant,
) -> bool {
    let prepared = prepared_at(resources);
    lock_prepared(cache).put(dir, prepared, now);
    prepared
}

/// Drops what [`is_prepared`] remembers: a preparation, install or removal changed the disk.
pub fn forget_prepared() {
    lock_prepared(&PREPARED).0.clear();
}

/// Whether the prepared folder `resources` is usable: its manifest is a JSON
/// document and no top-level table a killed preparation or an interrupted
/// copy left empty. Such a folder used to count as prepared, so Play started
/// the game on it and the loader crashed on the empty file.
pub(crate) fn prepared_at(resources: &Path) -> bool {
    if read_json(&resources.join(RESOURCES_MANIFEST)).is_none() {
        return false;
    }
    let Ok(entries) = fs::read_dir(resources) else {
        return false;
    };
    entries.flatten().all(|entry| {
        let path = entry.path();
        path.extension() != Some(std::ffi::OsStr::new("json"))
            || fs::metadata(&path).is_ok_and(|meta| meta.len() > 0)
    })
}

/// Whether a version's resources hold Minecraft's sounds, which `sounds` copies from the player's own
/// install beside the preparation. A version prepared while that copy failed (Mojang's republished 26.3
/// asset index, #1076), or one that reused such a version's resources, plays no sound at all (#1035).
pub fn has_sounds(tag: &str) -> bool {
    sounds_in(&resources(tag))
}

fn sounds_in(resources: &Path) -> bool {
    // What the game's sound registry reads (sunrise-client sound_events.rs).
    resources.join("assets/minecraft/sounds.json").is_file()
}

/// What the Play page says of a version's resources.
pub fn resources_summary(prepared: bool, sounds: bool, version: &str) -> String {
    use crate::i18n::trf;
    match (prepared, sounds) {
        (false, _) => trf(
            "Not prepared. Sunrise needs the files of your own Minecraft {version}.",
            &[("version", version)],
        ),
        (true, true) => trf("Prepared for Minecraft {version}", &[("version", version)]),
        (true, false) => trf(
            "Prepared for Minecraft {version}, without its sounds: the game will be silent. \
             Prepare again to copy them from your Minecraft install.",
            &[("version", version)],
        ),
    }
}

/// Whether the resources `from` prepared are the ones preparing `to` would make: both pin the same
/// Minecraft jars (research-lock.json) and their preparers make the same output. A preparer
/// declares what it makes with `OUTPUT_VERSION` (crates/sunrise-prepare/src/lib.rs, whose sources
/// ship in the release folder), bumped by hand when the output changes and checked by CI
/// (tools/prepare_output.py), so a release that changes the preparer's code but not its output
/// keeps the resources. Preparers from before the number are known by their sources
/// (`LEGACY_OUTPUT_VERSIONS`); older ones still are compared file by file (`same_preparer`).
fn resources_compatible(from: &Path, to: &Path, manifest: &serde_json::Value) -> bool {
    let pins = |dir: &Path| {
        read_json(&dir.join("research-lock.json")).map(|lock| {
            ["version", "client_sha1", "server_sha1"].map(|k| lock["minecraft"][k].clone())
        })
    };
    let (Some(a), Some(b)) = (pins(from), pins(to)) else {
        return false;
    };
    if a != b || a.iter().any(|v| !v.is_string()) {
        return false;
    }
    match (output_version_at(from), output_version_at(to)) {
        (Some(a), Some(b)) => a == b,
        (None, None) => same_preparer(from, to, manifest),
        _ => false,
    }
}

/// The `OUTPUT_VERSION` of a version folder's preparer: the constant in its sources, or for a
/// preparer from before it, the number `LEGACY_OUTPUT_VERSIONS` gives its fingerprint. None for
/// an unknown older preparer.
fn output_version_at(dir: &Path) -> Option<u32> {
    output_version_in(dir, &LEGACY_OUTPUT_VERSIONS)
}

fn output_version_in(dir: &Path, legacy: &[(&str, u32)]) -> Option<u32> {
    let crate_dir = dir.join(PREPARER_CRATE);
    let lib = fs::read_to_string(crate_dir.join("src/lib.rs")).ok()?;
    let declared = lib.lines().find_map(|line| {
        line.strip_prefix("pub const OUTPUT_VERSION: u32 = ")?
            .strip_suffix(';')?
            .parse()
            .ok()
    });
    if declared.is_some() {
        return declared;
    }
    let fingerprint = legacy_fingerprint(dir)?;
    legacy
        .iter()
        .find(|(known, _)| *known == fingerprint)
        .map(|&(_, version)| version)
}

/// Preparers from before `OUTPUT_VERSION`, by `legacy_fingerprint`, whose output
/// tools/prepare_output.py measured: what they prepare is version 1's.
const LEGACY_OUTPUT_VERSIONS: [(&str, u32); 2] = [
    // v0.2.52 to v0.2.64
    (
        "54218ade1c058b1f15ea4094bf8c1a7893adb37b1ed5cc803ddae8e950c31704",
        1,
    ),
    // v0.2.49 to v0.2.51
    (
        "da9ffe7558129c38b44abf43289e81946fe9f2512e5e07272eeae0cf0b4f1254",
        1,
    ),
];

/// The vanilla exporters those preparers ran (the `*_exporters_sha256` of their manifests), in
/// mods/sunrise/src/test/java/dev/sunriselab/parity.
const LEGACY_EXPORTERS: [&str; 18] = [
    "ParityData.java",
    "VanillaBlockBehaviourExporter.java",
    "VanillaBlockLandingOracle.java",
    "VanillaBlockStateBindingOracle.java",
    "VanillaCollisionOracle.java",
    "VanillaCommandParseOracle.java",
    "VanillaCreativeTabsExporter.java",
    "VanillaCuboidModelOracle.java",
    "VanillaDamageSourceOracle.java",
    "VanillaEntityAttributeOracle.java",
    "VanillaEntityModelExporter.java",
    "VanillaEntityTypesExporter.java",
    "VanillaFaceVisibilityOracle.java",
    "VanillaGameplayFactsExporter.java",
    "VanillaHeightmapOracle.java",
    "VanillaMobDataExporter.java",
    "VanillaMovementRegistryOracle.java",
    "VanillaPlayerHealthOracle.java",
];

/// The digest `sunrise_prepare::sources_sha256` gives a preparer crate's sources: `Cargo.toml`, `build.rs` and `src/*.rs`, CRs dropped.
fn preparer_sources_sha256(crate_dir: &Path) -> Option<String> {
    use sha2::Digest;
    let mut files = Vec::new();
    for name in ["Cargo.toml", "build.rs"] {
        files.push((name.to_string(), fs::read(crate_dir.join(name)).ok()?));
    }
    let mut sources: Vec<String> = fs::read_dir(crate_dir.join("src"))
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".rs"))
        .collect();
    sources.sort();
    for name in sources {
        let data = fs::read(crate_dir.join("src").join(&name)).ok()?;
        files.push((format!("src/{name}"), data));
    }
    let mut hash = sha2::Sha256::new();
    for (name, data) in files {
        let data: Vec<u8> = data.into_iter().filter(|b| *b != b'\r').collect();
        hash.update(format!("{name}\0{}\0", data.len()));
        hash.update(&data);
    }
    Some(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// A version folder's preparer as the launchers before `OUTPUT_VERSION` compared it: the digest
/// of its sources ([`preparer_sources_sha256`]) and of the exporters it ran, line endings
/// as checked out ignored.
fn legacy_fingerprint(dir: &Path) -> Option<String> {
    use sha2::Digest;
    let sources = preparer_sources_sha256(&dir.join(PREPARER_CRATE))?;
    let mut hash = sha2::Sha256::new();
    hash.update(sources.as_bytes());
    for name in LEGACY_EXPORTERS {
        let path = format!("mods/sunrise/src/test/java/dev/sunriselab/parity/{name}");
        let data: Vec<u8> = fs::read(dir.join(&path))
            .ok()?
            .into_iter()
            .filter(|b| *b != b'\r')
            .collect();
        hash.update(format!("{path}\0{}\0", data.len()));
        hash.update(&data);
    }
    Some(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// How launchers before `OUTPUT_VERSION` compared two preparers, for versions older than the
/// ones `LEGACY_OUTPUT_VERSIONS` knows. The manifest records the hashes of the vanilla exporters
/// the resources were made with (mods/sunrise/src/test/java/.../parity), and the preparer writes
/// every file and the manifest's `schema`, so both need the same preparer and the same exporter
/// sources. The preparer is crates/sunrise-prepare and tools/prepare_native_client.py (the
/// Python preparer of older versions, a wrapper of the binary since); a file only one of the two
/// versions has is a different preparer. The helpers the Python preparer imported (lab.py and
/// the test_*_parity.py modules) change between releases in how they find and check the
/// player's files, which the output does not depend on; they are not compared.
fn same_preparer(from: &Path, to: &Path, manifest: &serde_json::Value) -> bool {
    let Some(fields) = manifest.as_object() else {
        return false;
    };
    let mut preparer = vec![PathBuf::from("tools/prepare_native_client.py")];
    for dir in [from, to] {
        preparer.extend(files_under(dir, Path::new(PREPARER_CRATE)));
    }
    let same_or_absent = |path: &Path| match (fs::read(from.join(path)), fs::read(to.join(path))) {
        (Ok(a), Ok(b)) => a == b,
        (Err(a), Err(b)) => {
            a.kind() == std::io::ErrorKind::NotFound && b.kind() == std::io::ErrorKind::NotFound
        }
        _ => false,
    };
    if !preparer.iter().all(|p| same_or_absent(p)) {
        return false;
    }
    let mut sources = vec![
        "mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaHeightmapOracle.java".to_string(),
    ];
    for (_, value) in fields
        .iter()
        .filter(|(key, _)| key.ends_with("_exporters_sha256"))
    {
        let Some(files) = value.as_object() else {
            return false;
        };
        sources.extend(files.keys().cloned());
    }
    let same = |source: &str| {
        let path = Path::new(source);
        path.is_relative()
            && path
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
            && matches!((fs::read(from.join(path)), fs::read(to.join(path))), (Ok(a), Ok(b)) if a == b)
    };
    sources.iter().all(|s| same(s))
}

/// Where a version folder keeps the preparer's sources (the release package copies them).
const PREPARER_CRATE: &str = "crates/sunrise-prepare";

/// The files under `dir/relative`, as paths relative to `dir`.
fn files_under(dir: &Path, relative: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir.join(relative)) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = relative.join(entry.file_name());
        match entry.file_type() {
            Ok(t) if t.is_dir() => files.extend(files_under(dir, &path)),
            Ok(_) => files.push(path),
            Err(_) => {}
        }
    }
    files
}

pub(crate) fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    copy_dir_except(from, to, None)
}

/// Copy a folder, leaving out one directory or file when given.
pub(crate) fn copy_dir_except(from: &Path, to: &Path, skip: Option<&Path>) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        if Some(entry.path().as_path()) == skip {
            continue;
        }
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_except(&entry.path(), &target, skip)?;
        } else {
            crate::preparer::clone_or_copy(&entry.path(), &target)?;
        }
    }
    Ok(())
}

/// After an install: when `tag` is not prepared but another installed version made the same
/// resources, copies them (not hard links: preparing again rewrites files in place, which would
/// change both). The player then needs neither their Minecraft folder nor a new preparation to
/// play a release that only changed Sunrise. Returns the version they came from.
fn reuse_prepared(versions: &Path, tag: &str) -> Option<String> {
    let target = versions.join(tag);
    let to = resources_at(&target);
    if to.join(RESOURCES_MANIFEST).exists() {
        return None;
    }
    let version = game_version_at(&target);
    for other in installed_in(versions).into_iter().filter(|t| t != tag) {
        let source = versions.join(&other);
        if game_version_at(&source) != version {
            continue;
        }
        let from = resources_at(&source);
        let Some(manifest) = read_json(&from.join(RESOURCES_MANIFEST)) else {
            continue;
        };
        if !resources_compatible(&source, &target, &manifest) {
            continue;
        }
        // Copied beside it and renamed when complete: an interrupted copy is never "prepared".
        // The name is this call's own, as the window's start and an install can both copy.
        static COPIES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = COPIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let partial = to.with_file_name(format!(".{version}.partial-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&partial);
        // Without the data file: it names its own root, so the copy could not use it.
        let data_file = from.join(crate::preparer::DATA_FILE);
        let copied = copy_dir_except(&from, &partial, Some(&data_file)).and_then(|_| {
            if to.join(RESOURCES_MANIFEST).exists() {
                return Ok(false); // Prepared meanwhile, by another copy or the player.
            }
            let _ = fs::remove_dir_all(&to);
            fs::rename(&partial, &to).map(|_| true)
        });
        let _ = fs::remove_dir_all(&partial);
        match copied {
            Ok(true) => return Some(other),
            Ok(false) => return None,
            Err(_) => {}
        }
    }
    None
}

/// What an install does once its folder is in place (an imported build's too, `build_import`):
/// another installed version's resources if it has none, whatever its resources lack, then its
/// data file.
pub(crate) fn reuse_prepared_for(tag: &str) {
    let about = crate::native_mods::about(tag);
    let tag = about.as_ref().map_or(tag, |about| about.base.as_str());
    reuse_prepared(&versions_dir(), tag);
    complete_assets(tag);
    forget_prepared();
    let (root, resources) = (version_dir(tag), resources(tag));
    let spawned = std::thread::Builder::new()
        .name("Prepared data file".into())
        .spawn(move || {
            if let Err(e) = write_data_file(&root, &resources) {
                eprintln!("{}: not written: {e}", resources.display());
            }
        });
    if let Err(e) = spawned {
        eprintln!("{tag}: the data file was not written: {e}");
    }
}

/// Writes a prepared version's `sunrise-data.bin` so its first start already reads one file, not
/// thousands: over a minute under Windows Defender (#1961), and a player who updates would pay it
/// after every update. After `complete_assets`, as the file answers "missing" for what it lacks.
/// The version's own preparer writes it (`data-file`); the game still writes it when this didn't, or the preparer is too old to. Only looks for the file, as the game keeps it open for the whole run, which would stop a removal on Windows.
fn write_data_file(root: &Path, resources: &Path) -> Result<(), String> {
    if !prepared_at(resources) || resources.join(crate::preparer::DATA_FILE).exists() {
        return Ok(());
    }
    match crate::preparer::binary(root) {
        Some(binary) => crate::preparer::data_file(&binary, root, resources).map(drop),
        None => Ok(()),
    }
}

/// `reuse_prepared` for every installed version that is not prepared, oldest first, so a
/// version prepared this way can serve the next. Launchers before 0.1.4 installed new versions
/// without it; this catches those up. Returns each (version, where its resources came from).
/// Prepared versions also get the run-time reports (`complete_reports`) and the built-in packs,
/// panorama and sounds (`complete_assets`) they lack.
pub fn reuse_prepared_everywhere() -> Vec<(String, String)> {
    let versions = versions_dir();
    let reused = reuse_prepared_in(&versions);
    forget_prepared();
    for tag in installed_in(&versions) {
        // Here, on the start-up thread, and not in `complete_assets`, which Play also runs: a
        // download of thousands of files is not for the moment the game is waited for.
        refill_own_store(&tag);
        complete_assets(&tag);
    }
    reused
}

fn reuse_prepared_in(versions: &Path) -> Vec<(String, String)> {
    let mut tags = installed_in(versions);
    tags.reverse();
    let reused = tags
        .iter()
        .filter_map(|tag| reuse_prepared(versions, tag).map(|from| (tag.clone(), from)))
        .collect();
    for tag in &tags {
        complete_reports(versions, tag);
    }
    reused
}

/// The data generator reports the game reads at run time: the built-in registries' network ids
/// (without them no server can be joined or hosted) and the command tree.
const RUNTIME_REPORTS: [&str; 2] = ["registries.json", "commands.json"];

/// Puts the data generator reports the game reads at run time (`RUNTIME_REPORTS`) into a prepared version's
/// resources when they are missing. Preparers before they shipped there left them only in the
/// scratch output of the version that ran the data generator (`.local/datagen/out/reports`),
/// and resources copied from another version (`reuse_prepared`) came without them. They depend
/// only on the server jar, so the version's own scratch output or any installed version pinning
/// the same jar serves, and no new preparation is needed. Returns whether all are there.
fn complete_reports(versions: &Path, tag: &str) -> bool {
    let target = versions.join(tag);
    let to = resources_at(&target);
    if !to.join(RESOURCES_MANIFEST).exists() {
        return false;
    }
    let missing: Vec<_> = RUNTIME_REPORTS
        .iter()
        .filter(|name| !to.join(name).is_file())
        .collect();
    if missing.is_empty() {
        return true;
    }
    let server = |dir: &Path| {
        read_json(&dir.join("research-lock.json")).and_then(|lock| {
            let pins = ["version", "server_sha1"].map(|k| lock["minecraft"][k].clone());
            pins.iter().all(|v| v.is_string()).then_some(pins)
        })
    };
    let Some(pins) = server(&target) else {
        return false;
    };
    let reports = |dir: &Path| dir.join(".local/datagen/out/reports");
    let mut sources = vec![reports(&target)];
    for other in installed_in(versions).into_iter().filter(|t| t != tag) {
        let dir = versions.join(&other);
        if server(&dir).as_ref() == Some(&pins) {
            sources.push(resources_at(&dir));
            sources.push(reports(&dir));
        }
    }
    let mut complete = true;
    for name in missing {
        let Some(from) = sources.iter().map(|s| s.join(name)).find(|p| p.is_file()) else {
            complete = false;
            continue;
        };
        // Copied beside it and renamed: an interrupted copy never looks complete.
        // The name is this call's own, as the window's start and Play can both complete one version.
        static COPIES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = COPIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let partial = to.join(format!(".{name}.partial-{}-{n}", std::process::id()));
        let copied = fs::copy(&from, &partial).and_then(|_| fs::rename(&partial, to.join(name)));
        if copied.is_err() {
            let _ = fs::remove_file(&partial);
            complete = false;
        }
    }
    complete
}

/// Adds the extras a prepared version lacks (packs, panorama, fonts, languages, sounds), through its own
/// `sunrise-prepare extras`: a new version reuses the resources of the one before (`reuse_prepared`), which may
/// come from a preparer that left them out. Without them
/// in any Minecraft folder, one line each in the log, once per launcher run, and the version plays
/// as before. Nothing is prepared again.
pub fn complete_assets(tag: &str) {
    complete_assets_in(&versions_dir(), tag, minecraft_folders, &|line| {
        eprintln!("{line}")
    });
}

/// Whether Sunrise's own store is asked to download again: it has an asset index, the version
/// lacks some extra (any kind: the download fetches the whole asset index, which holds the
/// sounds, languages, fonts, panorama and built-in packs) and it was not asked this run yet.
fn refill_wanted(
    has_index: bool,
    missing: Option<&crate::preparer::Extras>,
    asked: &std::sync::atomic::AtomicBool,
) -> bool {
    has_index
        && missing.is_some_and(|m| !m.is_empty())
        && !asked.swap(true, std::sync::atomic::Ordering::Relaxed)
}

/// Sunrise's own download (an owner's) may have stopped before the sounds and the languages,
/// and the copies of `complete_assets` read them from its store: asked again, once per run, through
/// the same owner gate as any download from Mojang (`download_for_owner`), which keeps the files
/// already whole. Called where a download is expected (`reuse_prepared_everywhere`), never on Play.
fn refill_own_store(tag: &str) {
    static ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let own = data_dir().join("minecraft");
    let root = versions_dir().join(tag);
    let resources = resources_at(&root);
    let missing = crate::preparer::binary(&root).and_then(|binary| {
        crate::preparer::missing_extras(&binary, &root, &resources)
            .ok()
            .flatten()
    });
    if !refill_wanted(
        own.join("assets/indexes").is_dir(),
        missing.as_ref(),
        &ASKED,
    ) {
        return;
    }
    let Some(pins) = read_json(&version_dir(tag).join("research-lock.json"))
        .and_then(|l| sources::Pins::read(&l))
    else {
        return;
    };
    let fetch = |url: &str| -> Result<Vec<u8>, String> {
        let mut body = Vec::new();
        get(url)?
            .into_reader()
            .read_to_end(&mut body)
            .map_err(|e| e.to_string())?;
        Ok(body)
    };
    if let Err(e) = download_for_owner(&pins, &own, &fetch, &|line| eprintln!("{line}")) {
        eprintln!("{tag}: the missing assets were not downloaded: {e}");
    }
}

/// `folders`: where to look, asked only when the version lacks something.
fn complete_assets_in(
    versions: &Path,
    tag: &str,
    folders: impl Fn() -> Vec<PathBuf>,
    log: &dyn Fn(String),
) {
    // The versions tried this run. Held while adding: the start, an install and Play can complete one version at once.
    static TRIED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
    let root = versions.join(tag);
    let resources = resources_at(&root);
    if !resources.join(RESOURCES_MANIFEST).exists() {
        return;
    }
    // A version whose preparer has no `missing-extras` (from before it) keeps what it has.
    let Some(binary) = crate::preparer::binary(&root) else {
        return;
    };
    let lacking = match crate::preparer::missing_extras(&binary, &root, &resources) {
        Ok(Some(lacking)) if !lacking.is_empty() => lacking,
        Ok(_) => return,
        Err(e) => return log(format!("{tag}: {e}")),
    };
    let mut tried = TRIED.lock().unwrap_or_else(|e| e.into_inner());
    if tried.contains(&root) {
        return;
    }
    tried.push(root.clone());
    let kinds: Vec<String> = lacking.iter().map(|(kind, _)| kind.clone()).collect();
    match crate::preparer::add_extras(&binary, &root, &resources, &kinds, &folders(), log) {
        Ok(Some(added)) => {
            for (kind, error) in added.failed {
                let what = lacking
                    .iter()
                    .find(|(k, _)| *k == kind)
                    .map_or(kind.as_str(), |(_, what)| what.as_str());
                log(crate::i18n::trf(
                    "{version}: {what} not added: {error}",
                    &[("version", tag), ("what", what), ("error", &error)],
                ));
            }
        }
        Ok(None) => {}
        Err(e) => log(format!("{tag}: {e}")),
    }
}

/// The Minecraft folders whose asset store can hold what a prepared version lacks: the one
/// chosen in the launcher, the official launcher's (or SUNRISE_MINECRAFT_DIR), the folder
/// other launchers' files are linked into and Sunrise's own download, then every launcher's
/// default folder.
fn minecraft_folders() -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = Settings::load().minecraft_dir.into_iter().collect();
    folders.extend([
        crate::preparer::minecraft_dir(None),
        data_dir().join("linked-minecraft"),
        data_dir().join("minecraft"),
    ]);
    folders.extend(
        sources::Folders::current()
            .map(|f| sources::known_folders(sources::Os::current(), &f))
            .unwrap_or_default()
            .into_iter()
            .map(|(_, dir)| dir),
    );
    let mut found: Vec<PathBuf> = Vec::new();
    for folder in folders {
        if folder.is_dir() && !found.contains(&folder) {
            found.push(folder);
        }
    }
    found
}

/// The official launcher's game directory, else the first other launcher's found
/// (sources.rs); preparing looks in all of them anyway.
pub fn default_minecraft_dir() -> Option<PathBuf> {
    let folders = sources::Folders::current()?;
    sources::known_folders(sources::Os::current(), &folders)
        .into_iter()
        .map(|(_, dir)| dir)
        .find(|dir| dir.is_dir())
}

/// Downloads the pinned version from Mojang into `dir` when an account in the launcher owns
/// Minecraft (asked again now), else says what signing in would do, as the last line of the
/// not-found error.
fn download_for_owner(
    pins: &sources::Pins,
    dir: &Path,
    fetch: sources::Fetch,
    log: &dyn Fn(String),
) -> Result<(), String> {
    let mut accounts = account::Accounts::load();
    // Any account that owns the game may download it (`launch`), not only the one that plays.
    let pass = account::launch(
        &account::Endpoints::live(),
        account::client_id().as_deref(),
        account::store(),
        &mut accounts,
    );
    if let Err(e) = accounts.save_renewed() {
        log(format!("Sunrise: the accounts are not saved: {e}"));
    }
    download_for(pins, dir, pass, !accounts.accounts.is_empty(), fetch, log)
}

/// Downloads Minecraft `id` (any version on Mojang's list, its version file's sha1 from it) into
/// Sunrise's own game folder, in the official launcher's layout, for an account that owns the
/// game (asked again now). Returns that folder.
pub fn download_version_for_owner(
    id: &str,
    manifest_sha1: &str,
    log: &dyn Fn(String),
) -> Result<PathBuf, String> {
    let dir = data_dir().join("minecraft");
    let fetch = |url: &str| -> Result<Vec<u8>, String> {
        let mut body = Vec::new();
        get(url)?
            .into_reader()
            .read_to_end(&mut body)
            .map_err(|e| e.to_string())?;
        Ok(body)
    };
    let pins = sources::Pins::for_version(id, manifest_sha1);
    download_for_owner(&pins, &dir, &fetch, log)
        .map_err(|e| e.trim_start_matches("\n  • ").to_string())?;
    Ok(dir)
}

/// What a player with no account signed in can do: a sentence that stands alone, and reads as one
/// more bullet after a list of places Sunrise looked.
fn sign_in_to_download(version: &str) -> String {
    crate::i18n::trf(
        "Sign in with the Microsoft account that owns Minecraft: Sunrise then downloads {version} from Mojang itself.",
        &[("version", version)],
    )
}

/// What [`download_for_owner`] does with the owner gate's answer: Mojang's files only for its
/// `owner`, an account the services said owns the game just now.
fn download_for(
    pins: &sources::Pins,
    dir: &Path,
    pass: Result<account::Launch, account::NoOwner>,
    signed_in: bool,
    fetch: sources::Fetch,
    log: &dyn Fn(String),
) -> Result<(), String> {
    let v = &pins.version;
    match pass {
        Ok(account::Launch {
            owner: Some((owner, _)),
            ..
        }) => {
            let name = owner.name;
            log(format!(
                "{name} owns Minecraft: downloading {v} from Mojang"
            ));
            let os = match sources::Os::current() {
                sources::Os::Linux => "linux",
                sources::Os::MacOs => "osx",
                sources::Os::Windows => "windows",
            };
            mojang::download(pins, dir, os, 8, fetch, &mojang::CANCEL, log).map_err(|e| {
                format!("\n  • Sunrise tried to download {v} from Mojang for {name}, but: {e}")
            })
        }
        // An owner by its last answer, who can't be asked now.
        Ok(pass) => Err(format!(
            "\n  • Downloading {v} from Mojang needs your Microsoft account, which could not be \
             checked: {}",
            pass.offline.unwrap_or_default()
        )),
        Err(_) if signed_in => Err(
            "\n  • No signed-in account owns Minecraft: Java Edition, so Sunrise can't download it."
                .into(),
        ),
        Err(_) => Err(format!("\n  • {}", sign_in_to_download(v))),
    }
}

/// The Java home for this version: a runtime for precompiled exporters, a JDK for older
/// preparers or changed sources. Forward the selection to external preparers too.
/// Else try the official launcher's Java in `games`, then download the runtime named by
/// `manifest` into Sunrise's game folder (Mojang's runtimes are whole JDKs).
fn java(
    major: i64,
    compiler: bool,
    games: &[PathBuf],
    manifest: &Path,
    log: &dyn Fn(String),
) -> Result<PathBuf, String> {
    let find = if compiler {
        crate::jdk::find
    } else {
        crate::jdk::find_runtime
    };
    java_with(find, major, games, manifest, &|component, missing| {
        // Mojang builds its Linux Java for glibc: without glibc's loader (NixOS without nix-ld,
        // musl distributions) it cannot start, so don't download it.
        if cfg!(all(target_os = "linux", target_arch = "x86_64"))
            && !Path::new(GLIBC_LOADER).exists()
        {
            return Err(format!(
                "{missing}\n  • Minecraft's own Java from Mojang needs glibc ({GLIBC_LOADER} is missing; on NixOS, enable programs.nix-ld), so install compatible Java instead."
            ));
        }
        let fetch = |url: &str| -> Result<Vec<u8>, String> {
            let mut body = Vec::new();
            get(url)?
                .into_reader()
                .read_to_end(&mut body)
                .map_err(|e| e.to_string())?;
            Ok(body)
        };
        let runtime = data_dir().join("minecraft").join("runtime");
        mojang::java_runtime(component, &runtime, 8, &fetch, &mojang::CANCEL, log).map_err(|e| {
            format!("{missing}\n  • Sunrise also tried to download Minecraft's own Java from Mojang, but: {e}")
        })
    })
}

/// [`java`] with its Java lookup (`find`) and Mojang download (`download` of a runtime
/// component, given what was missing) passed in. A downloaded runtime's home is looked up
/// through [`crate::java::home_in`], so a macOS `jre.bundle` is entered.
fn java_with(
    find: impl Fn(i64, &[PathBuf]) -> Result<crate::jdk::Jdk, String>,
    major: i64,
    games: &[PathBuf],
    manifest: &Path,
    download: &dyn Fn(&str, &str) -> Result<PathBuf, String>,
) -> Result<PathBuf, String> {
    if let Ok(jdk) = find(major, &[]) {
        return Ok(jdk.home());
    }
    let mut runtimes: Vec<PathBuf> = Vec::new();
    for home in games
        .iter()
        .flat_map(|game| crate::java::minecraft_runtimes(&game.join("runtime")))
    {
        if !runtimes.contains(&home) {
            runtimes.push(home);
        }
    }
    let missing = match find(major, &runtimes) {
        Ok(jdk) => return Ok(jdk.home()),
        Err(e) => e,
    };
    let component = read_json(manifest)
        .and_then(|json| json["javaVersion"]["component"].as_str().map(String::from))
        .ok_or_else(|| missing.clone())?;
    let home = download(&component, &missing)?;
    let home = crate::java::home_in(&home).unwrap_or(home);
    find(major, &[home]).map(|jdk| jdk.home())
}

/// The dynamic loader glibc programs on x86-64 Linux start through.
pub(crate) const GLIBC_LOADER: &str = "/lib64/ld-linux-x86-64.so.2";

/// Runs `cmd` with Java at `home`: JAVA_HOME names it and its `bin` leads the PATH, as the
/// versions' own preparers look for Java.
fn with_java(cmd: &mut Command, home: &Path) {
    let mut path = vec![home.join("bin")];
    path.extend(
        std::env::var_os("PATH")
            .iter()
            .flat_map(std::env::split_paths),
    );
    cmd.env("JAVA_HOME", home);
    if let Ok(path) = std::env::join_paths(path) {
        cmd.env("PATH", path);
    }
}

/// The Minecraft folder a preparation whose game files are imported already reads: the sounds,
/// the title panorama, the fonts and the languages come from its asset store. It is the one
/// `resolve` found, linked into the official layout for another launcher's files; without one, the
/// chosen folder, and the preparation goes without them, saying so. Before, such a preparation
/// read the chosen folder as it was, which a Prism Launcher or Modrinth App install does not
/// keep its assets in: the game was silent and its title screen grey.
fn assets_folder(
    resolved: Result<sources::Resolved, String>,
    chosen: &Path,
    version: &str,
    log: &dyn Fn(String),
) -> PathBuf {
    match resolved {
        Ok(sources::Resolved::Folder(dir)) => dir,
        Ok(sources::Resolved::NotFound(_)) => {
            log(format!(
                "Minecraft {version}'s assets were not found in any launcher folder: the game \
                 will be silent and the title screen plain"
            ));
            chosen.to_path_buf()
        }
        Err(e) => {
            log(format!("Minecraft {version}'s assets: {e}"));
            chosen.to_path_buf()
        }
    }
}

/// Where the preparation takes Minecraft's files from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MinecraftFrom {
    /// The player's own install (any launcher's); Mojang only when none has the version and an
    /// account that owns the game is signed in.
    Install,
    /// Mojang, for the signed-in account that owns the game ("Download and prepare").
    Mojang,
}

/// A preparation's steps, in the order they run, as Home names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PrepareStep {
    /// Minecraft's files: found in an install, or downloaded from Mojang.
    Files,
    Java,
    Sounds,
    /// sunrise-prepare's own work on those files.
    Prepare,
}

impl PrepareStep {
    pub const ALL: [PrepareStep; 4] = [Self::Files, Self::Java, Self::Sounds, Self::Prepare];

    pub fn name(self) -> &'static str {
        match self {
            Self::Files => "Game files",
            Self::Java => "Java",
            Self::Sounds => "Sounds",
            Self::Prepare => "Prepare",
        }
    }
}

/// The step the preparation under way is at (`PrepareStep::ALL`'s index).
static PREPARE_STEP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn prepare_step() -> PrepareStep {
    PrepareStep::ALL[PREPARE_STEP.load(std::sync::atomic::Ordering::Relaxed)]
}

pub fn set_prepare_step(step: PrepareStep) {
    PREPARE_STEP.store(step as usize, std::sync::atomic::Ordering::Relaxed);
}

/// Checks what the preparation needs and picks how to run it. PLAY.txt describes the same
/// steps: the player's own launcher install, the server jar from that version's own json, and
/// Java (and Python, for versions before sunrise-prepare). Also returns the Minecraft folder
/// to prepare from: `minecraft`, or the files another launcher keeps, linked into the official
/// launcher's layout (sources.rs).
pub fn preparer(
    tag: &str,
    minecraft: &Path,
    from: MinecraftFrom,
    log: &dyn Fn(String),
) -> Result<(Command, PathBuf), String> {
    set_prepare_step(PrepareStep::Files);
    // A native build reads the resources of its base, so it is the base that gets prepared.
    let owner = resources_owner(tag);
    if owner != tag && !version_dir(&owner).join(COMPLETE).is_file() {
        return Err(crate::i18n::trf(
            "This native build needs its base version {base}; reinstall it.",
            &[("base", &owner)],
        ));
    }
    let tag = owner.as_str();
    let version = game_version(tag);
    let dir = version_dir(tag);
    let binary = crate::preparer::binary(&dir).ok_or_else(|| crate::preparer::too_old(tag))?;
    // The version's own answer: a sibling's verified import is linked in when found, and whether its exporters ship.
    let needs =
        crate::preparer::needs(&binary, &dir, log)?.ok_or_else(|| crate::preparer::too_old(tag))?;
    let reused = !needs.server_jar;
    let chosen = minecraft.to_path_buf();
    // Resolved even when the game files are reused: the assets still come from the install.
    let minecraft = {
        let pins = read_json(&dir.join("research-lock.json"))
            .and_then(|lock| sources::Pins::read(&lock))
            .ok_or("The version's research-lock.json doesn't pin the Minecraft files")?;
        let own = data_dir().join("minecraft");
        let mut known = vec![("Sunrise's own download", own.clone())];
        known.extend(
            sources::Folders::current()
                .map(|f| sources::known_folders(sources::Os::current(), &f))
                .unwrap_or_default(),
        );
        let fetch = |url: &str| -> Result<Vec<u8>, String> {
            let mut body = Vec::new();
            get(url)?
                .into_reader()
                .read_to_end(&mut body)
                .map_err(|e| e.to_string())?;
            Ok(body)
        };
        let linked = data_dir().join("linked-minecraft");
        if from == MinecraftFrom::Mojang {
            // Straight from Mojang (what is already there and whole is kept), for an owner only.
            download_for_owner(&pins, &own, &fetch, log)
                .map_err(|e| e.trim_start_matches("\n  • ").to_string())?;
            own
        } else {
            let resolved = sources::resolve(minecraft, &pins, &known, &linked, &fetch, log);
            if reused {
                assets_folder(resolved, &chosen, &version, log)
            } else {
                match resolved? {
                    sources::Resolved::Folder(dir) => dir,
                    sources::Resolved::NotFound(checked) => {
                        let not_found = sources::not_found(&version, &checked, minecraft);
                        download_for_owner(&pins, &own, &fetch, log).map_err(|e| not_found + &e)?;
                        own
                    }
                }
            }
        }
    };
    let manifest = minecraft
        .join("versions")
        .join(&version)
        .join(format!("{version}.json"));
    // The Java the version pins, found as sunrise-prepare finds it, so a missing one is
    // reported (or downloaded) before the server jar is downloaded. Versions from before
    // java_major ran Java 25.
    let major = read_json(&dir.join("research-lock.json"))
        .and_then(|lock| lock["minecraft"]["java_major"].as_i64())
        .unwrap_or(25);
    let mut games = vec![chosen, minecraft.clone(), data_dir().join("minecraft")];
    games.extend(
        sources::Folders::current()
            .map(|f| sources::known_folders(sources::Os::current(), &f))
            .unwrap_or_default()
            .into_iter()
            .map(|(_, dir)| dir),
    );
    games.extend(
        sources::Folders::current()
            .map(|f| sources::java_only_folders(sources::Os::current(), &f))
            .unwrap_or_default(),
    );
    set_prepare_step(PrepareStep::Java);
    let java_home = java(major, needs.jdk, &games, &manifest, log)?;
    let jar = data_dir()
        .join("server-jars")
        .join(format!("{version}.jar"));
    if !reused {
        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&manifest).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        let fetch = |url: &str| -> Result<Vec<u8>, String> {
            let mut body = Vec::new();
            get(url)?
                .into_reader()
                .read_to_end(&mut body)
                .map_err(|e| e.to_string())?;
            Ok(body)
        };
        ensure_server_jar(&jar, &json, &fetch, log)?;
    }
    let mut cmd = Command::new(binary);
    with_java(&mut cmd, &java_home);
    cmd.current_dir(&dir).arg("--root").arg(&dir);
    if !reused {
        cmd.args(["--prepare", "--server-jar"]).arg(&jar);
    }
    cmd.arg("--minecraft-dir").arg(&minecraft);
    Ok((cmd, minecraft))
}

/// Makes sure `jar` holds the server jar `manifest` names: one already cached whose sha1 is not
/// the manifest's (a cut-off or captive-portal download) is fetched again, and a download is
/// kept only when it matches. A manifest without a sha1 can't check either.
fn ensure_server_jar(
    jar: &Path,
    manifest: &serde_json::Value,
    fetch: sources::Fetch,
    log: &dyn Fn(String),
) -> Result<(), String> {
    use crate::i18n::{tr, trf};
    let server = &manifest["downloads"]["server"];
    let sha1 = server["sha1"].as_str();
    let whole =
        |body: &[u8]| sha1.is_none_or(|want| sources::sha1_hex(body).eq_ignore_ascii_case(want));
    if fs::read(jar).is_ok_and(|body| whole(&body)) {
        return Ok(());
    }
    let url = server["url"]
        .as_str()
        .ok_or_else(|| tr("The version file has no server download").to_string())?;
    log(trf(
        "Downloading the {version} server jar from Mojang",
        &[(
            "version",
            &jar.file_stem().unwrap_or_default().to_string_lossy(),
        )],
    ));
    let body = fetch(url)?;
    if !whole(&body) {
        return Err(trf(
            "The server jar from Mojang is not the one its version file names (sha1 {sha1})",
            &[("sha1", &sources::sha1_hex(&body))],
        ));
    }
    sources::write_file(jar, &body)
}

/// Copies the pinned version's sounds from the player's own launcher install into the version's resources with its own `sunrise-prepare sounds`; sound is optional, so a failure is a line in `log`.
pub fn sounds(tag: &str, minecraft: &Path, log: &dyn Fn(String)) {
    use crate::i18n::{tr, trf};
    let tag = resources_owner(tag);
    let tag = tag.as_str();
    let dir = version_dir(tag);
    set_prepare_step(PrepareStep::Sounds);
    log(tr("Copying the sounds from your Minecraft install").into());
    let result = match crate::preparer::binary(&dir) {
        Some(binary) => crate::preparer::sounds(&binary, &dir, minecraft, log)
            .and_then(|done| done.ok_or_else(|| crate::preparer::too_old(tag))),
        None => Err(crate::preparer::too_old(tag)),
    };
    if let Err(why) = result {
        log(trf(
            "No sound (the game will be silent): {why}",
            &[("why", &why)],
        ));
    }
}

/// Where the game keeps saves and options.txt: `SUNRISE_HOME`, else `sunrise` in the platform's
/// data folder (~/.local/share/sunrise on Linux, as the release folders' sunrise.sh uses). It is
/// never inside a version folder, so installing, updating or removing a version keeps the saves.
pub fn game_home() -> PathBuf {
    if let Some(home) = std::env::var_os("SUNRISE_HOME").filter(|h| !h.is_empty()) {
        return PathBuf::from(home);
    }
    if cfg!(test) {
        return crate::data_location::test_home().join("game");
    }
    // A location that does not open stands in as the default folders, as
    // `data_location::startup` makes it for the window: never a panic here.
    if let Ok(Some(roots)) = crate::data_location::configured_roots() {
        return roots.game;
    }
    crate::identity::game_folder_in(&dirs::data_dir().unwrap_or_else(std::env::temp_dir))
}

/// Whether No AI is on: every AI feature hidden and off (`no_ai`, kept in the
/// game's `sunrise-extras.txt` so the game reads the same switch).
pub fn no_ai() -> bool {
    crate::no_ai::is_on(&game_home())
}

/// Turns No AI on or off for the launcher and the game.
pub fn set_no_ai(on: bool) -> Result<(), String> {
    crate::no_ai::set(&game_home(), on).map_err(|e| format!("Could not save No AI: {e}"))
}

// ----- Logs and crash reports ---------------------------------------------------------------------

/// The game's logs (`latest.log` and the day archives), as it writes them in its home.
pub fn logs_dir() -> PathBuf {
    game_home().join("logs")
}

/// Where the game writes `crash-<date>_<time>-client.txt` when it crashes.
pub fn crash_reports_dir() -> PathBuf {
    game_home().join("crash-reports")
}

/// The crash reports in `dir`, oldest first (their names sort by time).
fn crash_reports_in(dir: &Path) -> Vec<PathBuf> {
    let mut reports: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt"))
        })
        .collect();
    reports.sort();
    reports
}

/// The newest crash report, if any.
pub fn newest_crash_report() -> Option<PathBuf> {
    crash_reports_in(&crash_reports_dir()).pop()
}

/// The newest crash report the game playing in `home` wrote at or after `since` (one from the
/// run that started then).
pub fn crash_report_since(home: &Path, since: std::time::SystemTime) -> Option<PathBuf> {
    crash_report_in_since(&home.join("crash-reports"), since)
}

fn crash_report_in_since(dir: &Path, since: std::time::SystemTime) -> Option<PathBuf> {
    crash_reports_in(dir).into_iter().rev().find(|p| {
        fs::metadata(p)
            .and_then(|m| m.modified())
            .is_ok_and(|t| t >= since)
    })
}

/// The error a crash report describes, on one line: the lines between its description and the
/// stack (`panicked at src/game.rs:10:5: boom`).
pub fn crash_summary(report: &str) -> Option<String> {
    let mut lines = report
        .lines()
        .skip_while(|l| !l.starts_with("Description:"));
    lines.next()?;
    let error: Vec<&str> = lines
        .skip_while(|l| l.trim().is_empty())
        .take_while(|l| !l.trim().is_empty() && !l.starts_with('\t'))
        .map(str::trim)
        .collect();
    (!error.is_empty()).then(|| error.join(" "))
}

/// The line the game prints when it has drawn its first frame.
pub const GAME_FIRST_FRAME: &str = "Sunrise: first frame";

/// Whether a game's output ends in a crash while its Vulkan renderer started: an adapter line
/// (`Sunrise renderer: <GPU> (Vulkan, <type>)`) and no first frame. A native crash in a graphics
/// driver leaves no crash report, only those lines (#746).
pub fn vulkan_start_crashed<S: AsRef<str>>(lines: impl IntoIterator<Item = S>) -> bool {
    let mut vulkan = false;
    for line in lines {
        let line = line.as_ref();
        if line.starts_with(GAME_FIRST_FRAME) {
            return false;
        }
        if let Some(adapter) = line.strip_prefix("Sunrise renderer: ") {
            // The adapter line, not one of the start's steps: "<GPU> (<backend>, <type>)".
            if adapter.ends_with(')') && adapter.contains(" (") {
                vulkan = adapter.contains(" (Vulkan, ");
            }
        }
    }
    vulkan
}

/// The game's graphics settings file in its home (the game's `graphics.rs`).
const GRAPHICS_FILE: &str = "sunrise-graphics.txt";

/// The graphics API (the game's setting name, and its label) Auto switches to after a native
/// crash at the Vulkan renderer's start: DirectX 12 on Windows (#746), OpenGL on Linux, where
/// Auto also runs Mesa's hasvk on Ivy Bridge and Haswell, which Mesa itself calls incomplete
/// there (#4293). `None` where the game has no other API (macOS: Metal only).
pub fn vulkan_crash_fallback(os: &str) -> Option<(&'static str, &'static str)> {
    match os {
        "windows" => Some(("dx12", "DirectX 12")),
        "macos" => None,
        _ => Some(("opengl", "OpenGL")),
    }
}

/// Sets `graphicsApi:<api>` in `home`'s graphics settings when it is unset or `auto`, keeping
/// every other line. True when it changed; a player's own choice stays.
pub fn switch_from_auto(home: &Path, api: &str) -> Result<bool, String> {
    let path = home.join(GRAPHICS_FILE);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut lines: Vec<String> = Vec::new();
    let mut found = false;
    for line in text.lines() {
        match line.split_once(':') {
            Some((key, value)) if key.trim() == "graphicsApi" => {
                if value.trim() != "auto" {
                    return Ok(false);
                }
                found = true;
                lines.push(format!("graphicsApi:{api}"));
            }
            _ => lines.push(line.to_owned()),
        }
    }
    if !found {
        lines.push(format!("graphicsApi:{api}"));
    }
    fs::create_dir_all(home).map_err(|e| format!("{}: {e}", home.display()))?;
    fs::write(&path, lines.join("\n") + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(true)
}

/// The last `lines` lines of `file`, or why it cannot be read.
fn tail_of(file: &Path, lines: usize) -> Result<String, String> {
    let bytes = fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let all: Vec<&str> = text.lines().collect();
    Ok(all[all.len().saturating_sub(lines)..].join("\n") + "\n")
}

/// `logs`: the end of the game's latest.log, or with `crash` its newest crash report. The path
/// read comes first.
pub fn read_logs(lines: usize, crash: bool) -> Result<(PathBuf, String), String> {
    if crash {
        let path = newest_crash_report()
            .ok_or_else(|| format!("No crash report in {}.", crash_reports_dir().display()))?;
        let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        return Ok((path, text));
    }
    let path = logs_dir().join("latest.log");
    let text = tail_of(&path, lines.max(1))?;
    Ok((path, text))
}

/// Splits the player's extra game arguments (Settings' one line) into words, the way a shell
/// would: spaces separate them, single or double quotes group a word with spaces in it, and a
/// backslash escapes the next character (outside quotes, or inside double quotes).
pub fn split_args(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else if c == '\\' && q == '"' {
                    // Inside double quotes only `"` and `\` are escaped, as a shell does.
                    match chars.next() {
                        Some(n @ ('"' | '\\')) => word.push(n),
                        Some(n) => {
                            word.push('\\');
                            word.push(n);
                        }
                        None => word.push('\\'),
                    }
                } else {
                    word.push(c);
                }
            }
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    started = true;
                }
                '\\' => {
                    if let Some(n) = chars.next() {
                        word.push(n);
                        started = true;
                    }
                }
                c if c.is_whitespace() => {
                    if started {
                        words.push(std::mem::take(&mut word));
                        started = false;
                    }
                }
                _ => {
                    word.push(c);
                    started = true;
                }
            },
        }
    }
    if started {
        words.push(word);
    }
    words
}

/// The extra game arguments the player set in Settings, split into words.
pub fn game_args() -> Vec<String> {
    split_args(&Settings::load().game_args)
}

/// Appends the player's extra game arguments, so they add to the launcher's own or override them.
pub fn append_game_args(cmd: &mut Command) {
    cmd.args(game_args());
}

/// Renderer workarounds a player may have exported to start the launcher itself (a transparent
/// window on old Mesa drivers): they belong to the launcher's own webview and must not reach the
/// game, which picks its own backend. Inherited, they left the client on CPU rendering at 2-3 FPS
/// (#7832).
pub const LAUNCHER_RENDER_ENV: [&str; 4] = [
    "LIBGL_ALWAYS_SOFTWARE",
    "GALLIUM_DRIVER",
    "WEBKIT_DISABLE_DMABUF_RENDERER",
    "WEBKIT_DISABLE_COMPOSITING_MODE",
];

fn strip_launcher_render_env(cmd: &mut Command) {
    for key in LAUNCHER_RENDER_ENV {
        cmd.env_remove(key);
    }
}

pub fn client_command(tag: &str) -> Command {
    client_command_at(tag, &resources(tag))
}

pub fn managed_client_command(
    tag: &str,
    owner: &crate::browser::sunrise::txn::Lease,
) -> Result<Command, String> {
    let active = owner.active_build()?.ok_or_else(|| {
        crate::i18n::tr("This profile has no committed build to start.").to_string()
    })?;
    if active.executable() != tag {
        return Err(crate::i18n::tr(
            "The launch command does not match this profile's saved build and folders.",
        )
        .into());
    }
    Ok(client_command_at(tag, &active.resources()))
}

fn client_command_at(tag: &str, resources: &Path) -> Command {
    let home = game_home();
    let _ = fs::create_dir_all(&home);
    let mut cmd = Command::new(client_program(tag));
    if crate::developer_install::uses_developer(tag) {
        cmd.env("SUNRISE_TELEMETRY_URL", "off");
    }
    crate::run_leases::mark(&mut cmd, &version_dir(tag));
    strip_launcher_render_env(&mut cmd);
    crate::telemetry::ping_only_if_allowed(&mut cmd, !Settings::load().ping_off);
    cmd.current_dir(&home)
        .env("SUNRISE_HOME", &home)
        .arg("--resources")
        .arg(resources);
    // How the game asks for a renewed session (`account session`) while it runs.
    if let Ok(exe) = std::env::current_exe() {
        cmd.env("SUNRISE_LAUNCHER_EXE", exe);
    }
    // PipeWire remembers volume and mute per application name. The game's own name is
    // "PipeWire ALSA [sunrise-client]", shared with development and test runs, where
    // muting it once left every later game silent: played from here it is "Sunrise".
    if std::env::var_os("PIPEWIRE_PROPS").is_none() {
        cmd.env(
            "PIPEWIRE_PROPS",
            "{ application.name = \"Sunrise\" media.role = \"Game\" }",
        );
    }
    cmd
}

/// The game's program: on macOS the one inside the version's Sunrise.app, so the Dock and Cmd+Tab
/// show Sunrise's name and icon (game_app.rs); the bare binary elsewhere, or if that fails.
pub(crate) fn client_program(tag: &str) -> PathBuf {
    let dir = version_dir(tag);
    #[cfg(target_os = "macos")]
    match crate::game_app::ensure(&dir) {
        Ok(program) => return program,
        Err(e) => eprintln!("Starting the game outside {}: {e}", crate::game_app::APP),
    }
    dir.join(CLIENT)
}

mod dedicated;

pub(crate) use dedicated::root as server_root;

pub fn server_command(tag: &str, folder: &Path) -> Result<Command, String> {
    dedicated::command(tag, folder)
}

pub fn verify_server_command(tag: &str, folder: &Path, command: &Command) -> Result<(), String> {
    dedicated::verify(tag, folder, command)
}

/// Whether `eula.txt` says `eula` is true, read as a Java properties file as the server reads it:
/// `#` and `!` lines are comments, the key ends at the first `=`, `:` or blank, spaces around the
/// separator are free, the value runs to the end of the line (a trailing blank is part of it), the
/// last `eula` line wins, and the value counts as true in any case. A line ending in a backslash
/// continues on the next line (its leading blanks dropped), and `\n`, `\r\n` and a lone `\r` all end
/// a line. Left out: backslash escapes inside the text (`\u0074rue`), which no real file uses.
pub fn eula_agreed(folder: &Path) -> bool {
    let Ok(text) = fs::read_to_string(folder.join("eula.txt")) else {
        return false;
    };
    let mut agreed = false;
    // The logical lines: a physical line ending in an odd number of backslashes joins the next.
    let mut logical: Vec<String> = Vec::new();
    let mut joining = false;
    for physical in text.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        let physical = physical.trim_start_matches([' ', '\t', '\x0c']);
        let comment = !joining && physical.starts_with(['#', '!']);
        if joining {
            logical.last_mut().unwrap().push_str(physical);
        } else {
            logical.push(physical.to_string());
        }
        let slashes = physical.len() - physical.trim_end_matches('\\').len();
        joining = !comment && slashes % 2 == 1;
        if joining {
            let last = logical.last_mut().unwrap();
            last.pop();
        }
    }
    for line in &logical {
        if line.starts_with(['#', '!']) {
            continue;
        }
        let Some(rest) = line.strip_prefix("eula") else {
            continue;
        };
        // `eulas=...` is another key.
        if !rest.is_empty() && !rest.starts_with([' ', '\t', '\x0c', '=', ':']) {
            continue;
        }
        let rest = rest.trim_start_matches([' ', '\t', '\x0c']);
        let value = rest
            .strip_prefix(['=', ':'])
            .unwrap_or(rest)
            .trim_start_matches([' ', '\t', '\x0c']);
        agreed = value.eq_ignore_ascii_case("true");
    }
    agreed
}

/// Writes eula.txt as vanilla's server expects it, once the player agreed in the launcher.
pub fn agree_eula(folder: &Path) -> Result<(), String> {
    fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let text = "#By changing the setting below to TRUE you are indicating your agreement to our EULA (https://aka.ms/MinecraftEULA).\neula=true\n";
    fs::write(folder.join("eula.txt"), text).map_err(|e| e.to_string())
}

// ----- Shared steps for the front ends -----------------------------------------------------------

/// The installed version to use: the one asked for, else the selected one, else the newest.
pub fn resolve_version(requested: Option<&str>) -> Result<String, String> {
    let installed = installed();
    if let Some(tag) = requested {
        return installed
            .iter()
            .find(|t| *t == tag)
            .cloned()
            .ok_or_else(|| format!("{tag} is not installed."));
    }
    let selected = Settings::load().selected.filter(|s| installed.contains(s));
    selected
        .or_else(|| installed.first().cloned())
        .ok_or_else(no_version_installed)
}

/// What a command that needs an installed version says when there is none: the way to get one.
pub fn no_version_installed() -> String {
    use crate::i18n::tr;
    format!(
        "{} {}",
        tr("No version is installed yet."),
        tr("Run `sunrise-launcher install latest` to get one.")
    )
}

/// Whether a running program was started from the version's folder: the game and a server take
/// their program and `--resources` from it. Windows refuses to delete a running program's files,
/// so there the delete itself fails. With no answer from `ps` the version counts as running.
#[cfg(unix)]
pub(crate) fn in_use(tag: &str) -> bool {
    let dir = format!("{}/", version_dir(tag).display());
    Command::new("ps")
        .args(["-A", "-ww", "-o", "args="])
        .output()
        .ok()
        .is_none_or(|out| {
            !out.status.success() || String::from_utf8_lossy(&out.stdout).contains(&dir)
        })
}

#[cfg(windows)]
pub(crate) fn in_use(tag: &str) -> bool {
    crate::windows_runtime::in_use(&version_dir(tag))
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn in_use(_tag: &str) -> bool {
    false
}

/// The Minecraft folder to prepare from: the one given, the one chosen in the launcher, or the
/// official launcher's default. It may be absent when imported files can be reused.
pub fn minecraft_dir(requested: Option<PathBuf>) -> Result<PathBuf, String> {
    Ok(requested
        .or(Settings::load().minecraft_dir)
        .or_else(default_minecraft_dir)
        .unwrap_or_else(|| crate::preparer::minecraft_dir(None)))
}

/// The server folder: the one given, the one chosen in the launcher, or `server/` in the data folder.
pub fn server_dir(requested: Option<PathBuf>) -> PathBuf {
    requested
        .or(Settings::load().server_dir)
        .unwrap_or_else(|| data_dir().join("server"))
}

/// Whether `tag` is prepared, with the run-time reports it needs completed when another copy has them.
pub fn need_prepared(tag: &str) -> Result<(), String> {
    let about = crate::native_mods::about(tag);
    if let Some(about) = &about {
        if !version_dir(&about.base).join(COMPLETE).is_file() {
            return Err(crate::i18n::trf(
                "This native build needs its base version {base}; reinstall it.",
                &[("base", &about.base)],
            ));
        }
    }
    let tag = about.as_ref().map_or(tag, |about| about.base.as_str());
    if check_prepared(tag) {
        complete_reports(&versions_dir(), tag);
        Ok(())
    } else {
        Err(format!(
            "{tag} is not prepared yet. Prepare it first: `sunrise-launcher prepare --version {tag}` \
             uses your own Minecraft {v}, or downloads it from Mojang for the signed-in account \
             that owns it.",
            v = game_version(tag)
        ))
    }
}

/// A step Play waits on before the game starts that the window says by name (`Msg::PlayStep`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayStep {
    Assets,
    SignIn,
}

/// Says on stderr which step Play is at, one line each: a terminal run shows where it stops when
/// it never gets to the game (#12179).
pub fn play_step(step: &str) {
    eprintln!("Sunrise: Play: {step}…");
}

/// The client's command, once the owner gate lets it start and its resources are prepared. Past
/// the gate, a sign-in that can't be renewed (no network, expired) still lets the game start,
/// offline.
pub fn client(tag: &str) -> Result<Command, String> {
    client_as(tag, None)
}

/// What the command line adds to a refusal to play for want of an account.
pub const SIGN_IN_HINT: &str = "Sign in with `sunrise-launcher account sign-in`.";

/// Start the game straight into a friend's world: `--server friend:CODE`.
pub fn join_friend(cmd: &mut Command, code: &str) -> Result<(), String> {
    sunrise_launcher_friends::invite::parse_detailed(code).map_err(|e| e.to_string())?;
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    cmd.arg("--server").arg(format!(
        "{}{code}",
        sunrise_launcher_friends::ADDRESS_PREFIX
    ));
    Ok(())
}

/// The dedicated server's command in `folder`, once prepared and the EULA is agreed to.
pub fn server(tag: &str, folder: &Path, accept_eula: bool) -> Result<Command, String> {
    let command = server_command(tag, folder)?;
    let folder = command
        .get_current_dir()
        .expect("dedicated command owns its directory");
    if crate::native_mods::is_native(tag) {
        return Err(crate::i18n::tr(
            "A dedicated server cannot use a profile’s native client build.",
        )
        .into());
    }
    if let Some(error) = game_architecture_launch_error(tag) {
        return Err(error);
    }
    need_prepared(tag)?;
    if accept_eula {
        agree_eula(folder)?;
    }
    if !eula_agreed(folder) {
        return Err("Running a server needs your agreement to the Minecraft EULA (https://aka.ms/MinecraftEULA). Accept it to continue.".into());
    }
    fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    Ok(command)
}

mod release_selection;
pub(crate) use release_selection::WorldBuildOrder;

/// One release's notes: the tag given, or the newest.
pub fn release_notes(tag: Option<&str>) -> Result<Release, String> {
    let releases = releases()?;
    match tag {
        Some(tag) => release_selection::by_tag(&releases, tag)
            .cloned()
            .ok_or_else(|| format!("No Sunrise release is tagged {tag}.")),
        None => releases
            .into_iter()
            .next()
            .ok_or_else(|| "No release is published yet.".into()),
    }
}

/// A process whose output is kept: the last `LOG_KEEP` lines, and its result once it ends.
pub struct Logged {
    pub running: Running,
    pub log: Arc<Mutex<std::collections::VecDeque<String>>>,
    pub exit: Arc<Mutex<Option<Result<(), String>>>>,
}

pub const LOG_KEEP: usize = 2000;

impl Logged {
    pub fn start(
        job: Job,
        cmd: Command,
        profile: Option<&crate::browser::sunrise::txn::Lease>,
    ) -> Result<Self, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        let running = spawn(job, cmd, tx, profile)?;
        let log = Arc::new(Mutex::new(std::collections::VecDeque::new()));
        let exit = Arc::new(Mutex::new(None));
        let (l, e) = (log.clone(), exit.clone());
        std::thread::spawn(move || {
            for msg in rx {
                match msg {
                    Msg::Line(_, line) => {
                        let mut log = l.lock().unwrap();
                        log.push_back(line);
                        if log.len() > LOG_KEEP {
                            log.pop_front();
                        }
                    }
                    Msg::Exit(_, result) => *e.lock().unwrap() = Some(result),
                    _ => {}
                }
            }
        });
        Ok(Self { running, log, exit })
    }

    pub fn finished(&self) -> Option<Result<(), String>> {
        self.exit.lock().unwrap().clone()
    }

    pub fn tail(&self, lines: usize) -> Vec<String> {
        let log = self.log.lock().unwrap();
        log.iter()
            .skip(log.len().saturating_sub(lines))
            .cloned()
            .collect()
    }

    pub fn wait(&self) -> Result<(), String> {
        loop {
            if let Some(result) = self.finished() {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    /// Asks a server to stop, and kills it if it has not after `grace`.
    pub fn stop(&mut self, grace: std::time::Duration) {
        self.running.send("stop");
        let start = std::time::Instant::now();
        while self.finished().is_none() && start.elapsed() < grace {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if self.finished().is_none() {
            self.running.kill();
        }
    }
}

// ----- Updates -----------------------------------------------------------------------------------

/// What a check found: a newer Sunrise build and a newer launcher, each feed checked on its own, with why one couldn't be read.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Updates {
    pub sunrise: Option<Release>,
    pub launcher: Option<Release>,
    pub sunrise_error: Option<String>,
    pub launcher_error: Option<String>,
}

/// The newest Sunrise build with a build here (the current Nightly), if it isn't installed and
/// no installed version is newer. Two Nightlies of one day compare equal: the one on GitHub is
/// the update.
pub fn sunrise_update(releases: &[Release], installed: &[String]) -> Option<Release> {
    let newest = latest(releases)?;
    let newer = !installed.contains(&newest.tag)
        && installed
            .iter()
            .all(|t| compare_versions(&newest.tag, t).is_ge());
    newer.then(|| newest.clone())
}

/// The newest launcher release with an archive for this platform, if newer than this launcher ([`own_tag`]).
pub fn launcher_update(releases: &[Release]) -> Option<Release> {
    launcher_update_from(releases, &own_tag())
}

fn launcher_update_from(releases: &[Release], own: &str) -> Option<Release> {
    releases
        .iter()
        .find(|r| r.asset.is_some())
        .filter(|r| compare_versions(&r.tag, own).is_gt())
        .cloned()
}

/// Both release lists and both update kinds: the game's releases published first, and neither feed's failure stopping the other's check.
pub fn releases_and_updates(on_releases: impl FnOnce(Result<Vec<Release>, String>)) -> Updates {
    releases_and_updates_with(
        release_list,
        installed,
        || launcher_releases(crate::github::CachePolicy::PreferCache),
        on_releases,
    )
}

fn releases_and_updates_with(
    load: impl FnOnce() -> Result<serde_json::Value, String>,
    installed: impl FnOnce() -> Vec<String>,
    launchers: impl FnOnce() -> Result<Vec<Release>, String>,
    on_releases: impl FnOnce(Result<Vec<Release>, String>),
) -> Updates {
    let mut updates = Updates::default();
    match load() {
        Ok(list) => {
            let releases = game_releases(&list);
            let installed_versions = installed();
            updates.sunrise = sunrise_update(&installable_updates(&releases), &installed_versions)
                .or_else(|| architecture_update(&releases, &installed_versions));
            on_releases(Ok(releases));
        }
        Err(e) => {
            on_releases(Err(e.clone()));
            updates.sunrise_error = Some(e);
        }
    }
    match launchers() {
        Ok(launchers) => updates.launcher = launcher_update(&launchers),
        Err(e) => updates.launcher_error = Some(e),
    }
    updates
}

/// If the selected installed game has the wrong architecture, offer the same release again so
/// the preference also applies when there is no newer game version.
fn architecture_update(releases: &[Release], installed: &[String]) -> Option<Release> {
    if !can_choose_game_architecture() {
        return None;
    }
    let settings = Settings::load();
    let tag = settings
        .selected
        .filter(|tag| installed.contains(tag))
        .or_else(|| installed.iter().find(|tag| is_game_tag(tag)).cloned())?;
    if !is_game_tag(&tag) {
        return None;
    }
    let release = releases.iter().find(|release| release.tag == tag)?;
    needs_game_architecture_install(&tag, release).then(|| release.clone())
}

/// A fresh release-list request for both kinds of update (plus older-launcher fallback if needed).
pub fn check_updates() -> Updates {
    releases_and_updates(|_| {})
}

mod update_selection;
pub use update_selection::select_after_update;

/// Installs `tag` (or with "latest" the newest release, pre-releases left out), then updates the selection as
/// `select_after_update` says when `as_update`, or always selects it otherwise.
pub fn install_tag(tag: &str, as_update: bool, progress: impl Fn(f32)) -> Result<String, String> {
    let releases = [releases()?, release_selection::snapshot(tag)?].concat();
    let release = if tag == "latest" {
        releases
            .iter()
            .find(|r| r.asset.is_some() && !r.prerelease)
            .ok_or("No release has a build for this platform yet.")?
    } else {
        release_selection::by_tag(&releases, tag)
            .ok_or_else(|| format!("No Sunrise release is tagged {tag}."))?
    };
    if release.asset.is_none() {
        return Err(format!("{} has no build for this platform.", release.tag));
    }
    let before = installed();
    let downgrade = nightly_downgrade(release, &before);
    if let Some(warning) = &downgrade {
        eprintln!("{warning}");
    }
    install(release, progress)?;
    let mut settings = Settings::load();
    // An older Nightly is a deliberate rollback: installed, but an update never moves the selection back.
    let select = !as_update
        || (downgrade.is_none() && select_after_update(settings.selected.as_deref(), &before));
    if select {
        settings.selected = Some(release.tag.clone());
        settings.save()?;
    }
    for (tag, error) in version_storage::prune_old_versions(&releases, &release.tag, false) {
        eprintln!(
            "{}",
            crate::i18n::trf(
                "Could not remove {build}: {error}. Try again in Settings → Storage.",
                &[("build", &tag), ("error", &error)],
            )
        );
    }
    Ok(release.tag.clone())
}

/// A warning when `release` is a Nightly older than a Nightly already installed: a rollback, which is
/// allowed, but never what an update should do silently (an older build served as the current one
/// shows its own date, since that date is signed). None otherwise.
pub fn nightly_downgrade(release: &Release, installed: &[String]) -> Option<String> {
    release.nightly.as_ref()?;
    let newest = installed
        .iter()
        .filter(|v| crate::nightly::date_of(v).is_some())
        .max_by(|a, b| compare_versions(a, b))?;
    compare_versions(&release.tag, newest).is_lt().then(|| {
        crate::i18n::trf(
            "{tag} is older than {newest}, which is installed: installing it as a rollback. The selection \
             only moves to it when you pick it.",
            &[("tag", &release.tag), ("newest", newest)],
        )
    })
}

/// The launcher's own path, read once at start: after a self-update the running file is gone.
pub fn exe_path() -> &'static Path {
    static EXE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    EXE.get_or_init(|| std::env::current_exe().unwrap_or_else(|_| PathBuf::from(LAUNCHER_BINARY)))
}

/// The AppImage this launcher runs from, when it does: the runtime sets `$APPIMAGE` to the file it
/// mounted and `$APPDIR` to the mount, while [`exe_path`] then names a copy inside that read-only
/// mount, gone with it. Updating and restarting work on this file, and the menu entry, shortcuts and
/// agent configurations start it.
pub fn appimage_path() -> Option<PathBuf> {
    appimage_path_in(
        std::env::var_os("APPIMAGE").as_deref(),
        std::env::var_os("APPDIR").as_deref(),
        exe_path(),
    )
}

/// `$APPIMAGE`, if it names a file still there and this binary runs from inside `$APPDIR`. Both
/// variables reach every child of an AppImage: a launcher a different AppImage's program started
/// must never take that program's file for its own (an update would overwrite it).
fn appimage_path_in(
    appimage: Option<&std::ffi::OsStr>,
    appdir: Option<&std::ffi::OsStr>,
    exe: &Path,
) -> Option<PathBuf> {
    let appdir = Path::new(appdir?);
    if !appdir.is_absolute() || !exe.starts_with(appdir) {
        return None;
    }
    appimage
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_file())
}

/// The file a self-update replaces and a restart starts: the AppImage when the launcher runs from
/// one, the binary this process started from otherwise.
pub fn update_target() -> PathBuf {
    appimage_path().unwrap_or_else(|| exe_path().to_path_buf())
}

/// The launcher archive this build installs from: the AppImage's when it runs from one (a `.tar.gz`
/// binary would be written inside the read-only mount), this platform's otherwise.
pub fn launcher_asset() -> Option<&'static str> {
    launcher_asset_running_from(appimage_path().is_some())
}

fn launcher_asset_running_from(appimage: bool) -> Option<&'static str> {
    if appimage {
        LAUNCHER_APPIMAGE_ASSET
    } else {
        LAUNCHER_ASSET
    }
}

/// Development builds and `SUNRISE_LAUNCHER_NO_SELF_UPDATE` never replace themselves.
pub fn self_update_allowed() -> bool {
    !cfg!(debug_assertions) && std::env::var_os("SUNRISE_LAUNCHER_NO_SELF_UPDATE").is_none()
}

/// The version in `sunrise-launcher --version`'s output (`sunrise-launcher 0.1.5`).
fn parse_version_output(out: &str) -> Option<&str> {
    out.trim()
        .strip_prefix(crate::identity::VERSION_PREFIX)
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

/// The version of the launcher binary now at `path`, which is not this process's once another
/// update (the command line, another window) has replaced the file.
pub(crate) fn installed_launcher_version(path: &Path) -> Option<String> {
    let mut cmd = crate::process::command(path);
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    let out = cmd.output().ok()?;
    parse_version_output(&String::from_utf8_lossy(&out.stdout)).map(String::from)
}

/// Puts `new` at `exe`, the path this launcher started from. The running process keeps its
/// file (unlinked) and works on. Unix only: `self_replace` asks the system where the running
/// binary is, and Linux answers "<path> (deleted)" once the file has been replaced, so a second
/// update in the same run failed with "No such file or directory".
#[cfg(unix)]
fn replace_binary(new: &Path, exe: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let dir = exe.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "the launcher has no folder")
    })?;
    let name = exe.file_name().unwrap_or_default().to_string_lossy();
    let temp = dir.join(format!(".{name}.update-{}", std::process::id()));
    let mode = fs::metadata(exe).map_or(0o755, |m| m.permissions().mode() | 0o111);
    let result = fs::copy(new, &temp)
        .and_then(|_| fs::set_permissions(&temp, fs::Permissions::from_mode(mode)))
        .and_then(|_| fs::rename(&temp, exe));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Windows keeps a running binary from being written or deleted but lets it be renamed: the
/// running one steps aside to `<name>.old-<pid>-<n>` in its folder, the new one takes its path,
/// and a later start removes the old file (`remove_replaced_binaries`). The `self_replace` crate
/// did this by copying the launcher under a random hidden name and starting that copy to delete
/// the original through a hidden `cmd.exe`, which Microsoft Defender quarantines as
/// Trojan:Script/Wacatac (#3175).
#[cfg(not(unix))]
fn replace_binary(new: &Path, exe: &Path) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU32, Ordering};
    // A second update in the same run moves the first one's binary aside too.
    static MOVED: AtomicU32 = AtomicU32::new(0);
    let dir = exe.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "the launcher has no folder")
    })?;
    let name = exe.file_name().unwrap_or_default().to_string_lossy();
    let pid = std::process::id();
    let temp = dir.join(format!("{name}.update-{pid}"));
    let old = dir.join(format!(
        "{name}.old-{pid}-{}",
        MOVED.fetch_add(1, Ordering::Relaxed)
    ));
    if let Err(e) = fs::copy(new, &temp) {
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    match fs::rename(exe, &old) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            let _ = fs::remove_file(&temp);
            return Err(e);
        }
        _ => {}
    }
    if let Err(e) = fs::rename(&temp, exe) {
        let _ = fs::rename(&old, exe);
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    Ok(())
}

/// Deletes the binaries earlier self-updates moved aside (`replace_binary`), and the copies of an
/// update that stopped before its new binary took its place (`<name>.update-<pid>`). The one the
/// launcher just restarted from is still exiting, so this keeps trying for a few seconds in the
/// background; one that stays in use (another launcher window) goes on a later start.
#[cfg(windows)]
pub fn remove_replaced_binaries() {
    let exe = exe_path();
    let (Some(dir), Some(name)) = (exe.parent(), exe.file_name()) else {
        return;
    };
    let dir = dir.to_path_buf();
    let name = name.to_string_lossy();
    let old = format!("{name}.old-");
    let update = format!("{name}.update-");
    let own = format!("{update}{}", std::process::id());
    std::thread::spawn(move || {
        for _ in 0..10 {
            let Ok(entries) = fs::read_dir(&dir) else {
                return;
            };
            let mut left = false;
            for entry in entries.flatten() {
                let file = entry.file_name();
                let file = file.to_string_lossy();
                if file.starts_with(&old) || (file.starts_with(&update) && file != own) {
                    left |= fs::remove_file(entry.path()).is_err();
                }
            }
            if !left {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    });
}

/// The app bundle the launcher runs from (`…/Sunrise.app` for `…/Sunrise.app/Contents/MacOS/
/// sunrise-launcher`), or None for a bare binary, as launchers before the .app shipped.
pub fn app_bundle(exe: &Path) -> Option<&Path> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && app.extension()? == "app")
        .then_some(app)
}

/// Finds the first `*.app` folder under `dir`.
fn find_app(dir: &Path) -> Option<PathBuf> {
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.extension().is_some_and(|e| e == "app") {
                return Some(path);
            }
            if let Some(found) = find_app(&path) {
                return Some(found);
            }
        }
    }
    None
}

/// Puts a verified app bundle in place, retaining old bundles until no process uses them.
#[cfg(target_os = "macos")]
fn replace_bundle(new: &Path, app: &Path) -> Result<(), String> {
    crate::mac_update::replace_bundle(new, app)
}

/// Retired macOS launchers are removed only once their executable is no longer in use.
/// Run for every entry point (GUI, CLI and MCP), before a command can return or exec.
pub fn cleanup_launcher_updates() {
    #[cfg(target_os = "macos")]
    if let Some(app) = app_bundle(exe_path()) {
        crate::mac_update::cleanup(app);
    }
}

#[cfg(not(target_os = "macos"))]
fn replace_bundle(_new: &Path, _app: &Path) -> Result<(), String> {
    Err("App bundles are a macOS thing.".into())
}

/// A launcher release downloaded and checked against its signed SHA256SUMS, waiting to take this
/// launcher's place: the new AppImage, `.app` bundle or binary, in a stage folder of its own inside
/// [`update_work_dir`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Staged {
    pub tag: String,
    pub payload: PathBuf,
    /// The signed SHA-256 of the archive beside it, checked again just before it goes in
    /// ([`place_checked`]).
    pub sha256: String,
}

/// Where launcher updates are downloaded and unpacked before one takes this launcher's place. Each
/// has its own folder in here, `<pid>-<nonce>`, named after the process that made it, because a
/// stage waits there until its window closes and other launchers stage updates meanwhile.
pub fn update_work_dir() -> PathBuf {
    data_dir().join(".launcher-update")
}

/// How long a stage whose process has gone is kept: the installer may still be working from it.
/// The window marks it when it hands it over ([`mark_handed_over`]), so this counts from there.
const STAGE_GRACE: std::time::Duration = std::time::Duration::from_secs(60 * 60);
const HANDED_OVER: &str = "handed-over";
const UNPACKED: &str = "unpacked";
/// The archive's name in a stage says how to open it, in the installer too.
static ARCHIVE_ENDINGS: [&str; 3] = [".tar.gz", ".AppImage", ".zip"];

fn archive_name(url: &str) -> String {
    let ending = ARCHIVE_ENDINGS
        .iter()
        .find(|ending| url.ends_with(**ending))
        .unwrap_or(&".zip");
    format!("download{ending}")
}

/// The process that made the stage folder called `name`; None for any other name.
fn stage_owner(name: &str) -> Option<u32> {
    let (pid, nonce) = name.split_once('-')?;
    if nonce.len() != 32 || !nonce.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    pid.parse().ok()
}

pub(crate) fn new_stage_folder(work: &Path) -> Result<PathBuf, String> {
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|e| e.to_string())?;
    let nonce: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let stage = work.join(format!("{}-{nonce}", std::process::id()));
    fs::create_dir_all(work).map_err(|e| e.to_string())?;
    fs::create_dir(&stage).map_err(|e| e.to_string())?;
    Ok(stage)
}

/// The stage folder directly under `work` that holds `payload`. None for a path anywhere else,
/// which is not an update a launcher staged, and which nothing may delete or install.
pub fn stage_folder(work: &Path, payload: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let mut inside = payload.strip_prefix(work).ok()?.components();
    let Component::Normal(name) = inside.next()? else {
        return None;
    };
    stage_owner(name.to_str()?)?;
    let rest: Vec<_> = inside.collect();
    (!rest.is_empty() && rest.iter().all(|c| matches!(c, Component::Normal(_))))
        .then(|| work.join(name))
}

/// Removes the stage holding `payload`, and only that one.
pub fn remove_stage(work: &Path, payload: &Path) {
    if let Some(stage) = stage_folder(work, payload) {
        let _ = fs::remove_dir_all(stage);
    }
}

/// The window is about to start the installer for `staged`: from here its folder is in use by a
/// process no other launcher can tell from its name.
pub fn mark_handed_over(staged: &Staged) {
    if let Some(stage) = stage_folder(&update_work_dir(), &staged.payload) {
        let _ = fs::write(stage.join(HANDED_OVER), b"");
    }
}

/// Removes what a crash or a killed window left behind: stages whose process has gone and that
/// nothing touched for [`STAGE_GRACE`], and the files launchers before stage folders kept directly
/// in `work`. A stage whose process is alive stays however old it is; when its number has been
/// reused, it only stays longer.
fn remove_abandoned_stages(work: &Path, now: std::time::SystemTime, alive: impl Fn(u32) -> bool) {
    let Ok(entries) = fs::read_dir(work) else {
        return;
    };
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_str()
            .and_then(stage_owner)
            .is_some_and(&alive)
        {
            continue;
        }
        let path = entry.path();
        let touched = [path.clone(), path.join(HANDED_OVER)]
            .iter()
            .filter_map(|p| fs::metadata(p).and_then(|m| m.modified()).ok())
            .max();
        let abandoned =
            touched.is_some_and(|t| now.duration_since(t).is_ok_and(|age| age >= STAGE_GRACE));
        if abandoned {
            let _ = match entry.file_type() {
                Ok(kind) if kind.is_dir() => fs::remove_dir_all(&path),
                _ => fs::remove_file(&path),
            };
        }
    }
}

/// Whether process `pid` is running. One this user may not look at counts as running.
#[cfg(unix)]
pub(crate) fn process_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 sends nothing, it only asks whether the process is there.
    pid > 0
        && (unsafe { libc::kill(pid, 0) } == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM))
}

/// Whether process `pid` is running. One this user may not look at counts as running.
#[cfg(windows)]
pub(crate) fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, STILL_ACTIVE,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: the handle is checked before use and closed once; `code` outlives the call.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return GetLastError() == ERROR_ACCESS_DENIED;
        }
        let mut code = 0u32;
        // An exited process stays openable while something holds a handle to it.
        let running = GetExitCodeProcess(handle, &mut code) != 0 && code as i32 == STILL_ACTIVE;
        CloseHandle(handle);
        running
    }
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = sunrise_launcher_release_verify::Sha256::default();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(hash.hex());
        }
        hash.update(&buf[..n]);
    }
}

/// Checks the stage's archive against the signed SHA-256 again, then hands `place` the launcher
/// from those same bytes: an AppImage is the archive, anything else is unpacked afresh, whatever
/// has happened to the stage since it was downloaded. Nothing is placed when the bytes differ.
pub fn place_checked(
    work: &Path,
    staged: &Staged,
    place: impl FnOnce(&Staged) -> Result<(), String>,
) -> Result<(), String> {
    let not_staged = || {
        format!(
            "{} is not a launcher update this launcher downloaded.",
            staged.payload.display()
        )
    };
    let stage = stage_folder(work, &staged.payload).ok_or_else(not_staged)?;
    let archive = ARCHIVE_ENDINGS
        .iter()
        .map(|ending| stage.join(format!("download{ending}")))
        .find(|path| path.is_file())
        .ok_or_else(not_staged)?;
    sunrise_launcher_release_verify::check(&staged.tag, &staged.sha256, &file_sha256(&archive)?)?;
    let payload = if staged.payload == archive {
        archive
    } else {
        let inside = staged
            .payload
            .strip_prefix(stage.join(UNPACKED))
            .map_err(|_| not_staged())?;
        let fresh = stage.join("checked");
        let _ = fs::remove_dir_all(&fresh);
        unpack(&archive, &archive.to_string_lossy(), &fresh)?;
        let payload = fresh.join(inside);
        if !payload.exists() {
            return Err("The download has no launcher binary.".into());
        }
        payload
    };
    place(&Staged {
        payload,
        ..staged.clone()
    })
}

/// Whether `installed`, the launcher in place, is already the release `tag` or a newer one.
pub(crate) fn already_in_place(installed: Option<&str>, tag: &str) -> bool {
    match (installed, launcher_version(tag)) {
        (Some(installed), Some(target)) => compare_versions(installed, target).is_ge(),
        _ => false,
    }
}

/// Downloads a launcher release and puts it in place of this one. The running process keeps
/// working; `restart` starts the new one. When the file there already is that release or
/// a newer one (updated from the command line or another window), there is nothing to do.
pub fn self_update(release: &Release, progress: impl Fn(f32)) -> Result<(), String> {
    if let Some(staged) = stage_update(release, &progress)? {
        let work = update_work_dir();
        let placed = place_checked(&work, &staged, place_staged);
        remove_stage(&work, &staged.payload);
        placed?;
    }
    progress(1.0);
    Ok(())
}

/// Downloads and unpacks a launcher release next to this one without putting it in place yet
/// ([`place_checked`] does, in this process or in the installer `update_handoff` starts), in a new
/// stage folder that no other launcher writes to. None when the file there already is that release
/// or a newer one.
pub fn stage_update(release: &Release, progress: impl Fn(f32)) -> Result<Option<Staged>, String> {
    let (url, size) = release
        .asset
        .clone()
        .ok_or("This launcher release has no build for this platform.")?;
    let target = launcher_version(&release.tag).unwrap_or("0");
    if installed_launcher_version(&update_target())
        .is_some_and(|v| v != VERSION && compare_versions(&v, target).is_ge())
    {
        return Ok(None);
    }
    let sha256 = crate::signed::launcher_digest(
        &release.tag,
        launcher_asset().unwrap_or_default(),
        &sunrise_launcher_release_verify::trusted_launcher_keys(),
        fetch_small,
    )?;
    let work = update_work_dir();
    remove_abandoned_stages(&work, std::time::SystemTime::now(), process_alive);
    let stage = new_stage_folder(&work)?;
    let staged = fill_stage(&stage, &release.tag, &url, size, &sha256, &progress);
    if staged.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    staged.map(Some)
}

fn fill_stage(
    stage: &Path,
    tag: &str,
    url: &str,
    size: u64,
    sha256: &str,
    progress: &impl Fn(f32),
) -> Result<Staged, String> {
    let archive = stage.join(archive_name(url));
    download(url, size, sha256, &archive, progress)?;
    let staged = |payload| {
        Ok(Staged {
            tag: tag.to_owned(),
            payload,
            sha256: sha256.to_owned(),
        })
    };
    // An AppImage is the whole program: the new one takes the place of the file `$APPIMAGE` names
    // (the running mount keeps its copy until `restart` starts it). Its archive is not a `.tar.gz`
    // to unpack, so this comes before the unpacking below.
    if appimage_path().is_some() {
        if !url.ends_with(".AppImage") {
            return Err(format!(
                "{tag} is not an AppImage, and this launcher runs from one. Install it from its release page instead."
            ));
        }
        return staged(archive);
    }
    let unpacked = stage.join(UNPACKED);
    unpack(&archive, url, &unpacked)?;
    // A launcher inside Sunrise.app replaces the whole bundle (its Info.plist, icon and
    // signature belong with the binary); a bare binary, as older launchers are, only the binary,
    // which the macOS archive also holds (Sunrise.app/Contents/MacOS/sunrise-launcher).
    if app_bundle(exe_path()).is_some() {
        if let Some(new) = find_app(&unpacked) {
            return staged(new);
        }
    }
    let binary =
        find_file(&unpacked, LAUNCHER_BINARY).ok_or("The download has no launcher binary.")?;
    staged(binary)
}

/// Puts a [`stage_update`]d launcher where this one runs from: the AppImage, the `.app` bundle
/// or the binary. [`place_checked`] calls this with the bytes it has just checked.
pub fn place_staged(staged: &Staged) -> Result<(), String> {
    let new = &staged.payload;
    if let Some(appimage) = appimage_path() {
        replace_binary(new, &appimage).map_err(|e| {
            format!(
                "Could not replace the AppImage at {}: {e}",
                appimage.display()
            )
        })?;
        return Ok(());
    }
    match app_bundle(exe_path()) {
        Some(app) if new.extension().is_some_and(|e| e == "app") => replace_bundle(new, app)?,
        _ => replace_binary(new, exe_path()).map_err(|e| {
            format!(
                "Could not replace the launcher at {}: {e}",
                exe_path().display()
            )
        })?,
    }
    // Only a registered Setup install owns its folder. Its new executable refreshes the
    // notices, including when the CLI applied this update without restarting the window.
    #[cfg(windows)]
    crate::installer::after_update(launcher_version(&staged.tag).unwrap_or(VERSION));
    Ok(())
}

/// The website's install page, where a launcher that can't update itself sends the player.
pub fn download_page(website: &str) -> String {
    format!("{}/docs/install", website.trim_end_matches('/'))
}

/// Whether this copy of the launcher can't put a new one in its place: its folder is read-only (a
/// disk image, or the read-only copy macOS runs a downloaded app from until it is moved, its "App
/// Translocation"), or on Windows it runs from the temporary folder, where Explorer and archive
/// tools run a program opened inside a `.zip` (and delete it later). Such a copy is told to download
/// the launcher again (#9287). Elsewhere a launcher unpacked in /tmp replaces itself fine.
pub fn cannot_replace_itself() -> bool {
    let target = update_target();
    let place = app_bundle(&target).unwrap_or(&target);
    place.parent().is_none_or(|dir| {
        !writable(dir) || (cfg!(windows) && in_temp_folder(dir, &std::env::temp_dir()))
    })
}

/// Whether a package manager owns this launcher: the update target is classified, rather than
/// the executable inside an AppImage's mount. Asked while the sidebar is drawn, and the launcher
/// doesn't move while it runs, so the path is resolved once.
pub fn package_managed_launcher() -> bool {
    static MANAGED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *MANAGED.get_or_init(|| {
        let target = update_target();
        let target = target.canonicalize().unwrap_or(target);
        crate::update_recovery::package_managed(&target)
    })
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".sunrise-write-check-{}", std::process::id()));
    let made = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .is_ok();
    let _ = fs::remove_file(&probe);
    made
}

/// `dir` is the temporary folder or inside it, however either is spelled: %TEMP% can use an
/// account's short name (`C:\Users\USERNA~1\...`) while the launcher's own path has the long one.
fn in_temp_folder(dir: &Path, temp: &Path) -> bool {
    let resolved = |path: &Path| plain(path.canonicalize().unwrap_or_else(|_| path.to_path_buf()));
    resolved(dir).starts_with(resolved(temp))
}

fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if entry.file_name() == name {
            return Some(path);
        }
    }
    None
}

/// Starts the launcher again (the updated binary, or the replaced AppImage) with the same
/// arguments in place of this one: on Unix the process becomes the new binary (exec), on Windows
/// the new one starts and this one exits. Returns only when the new one could not start.
pub fn restart() -> String {
    let target = update_target();
    let mut cmd = Command::new(&target);
    cmd.args(std::env::args_os().skip(1));
    #[cfg(unix)]
    let error = {
        use std::os::unix::process::CommandExt;
        cmd.exec()
    };
    #[cfg(not(unix))]
    let error = match cmd.spawn() {
        Ok(_) => std::process::exit(0),
        Err(e) => e,
    };
    format!(
        "Could not start {}: {error}. Close the launcher and open it again.",
        target.display()
    )
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Job {
    Prepare,
    Client,
    Server,
}

/// A running process; its output arrives as `Msg::Line`, its end as `Msg::Exit`.
pub struct Running {
    pub child: Arc<Mutex<Child>>,
    pub stdin: Option<Box<dyn std::io::Write + Send>>,
}

/// Launcher-owned tree-lock primitives and optional Windows runtime state.
pub struct RuntimeGuard {
    _leases: Vec<crate::run_leases::Claim>,
    #[cfg(windows)]
    _windows: Option<crate::windows_runtime::Guard>,
}

pub use crate::run_leases::control::startup_cleanup;

pub fn prepare_command(cmd: Command) -> Result<(Command, RuntimeGuard), String> {
    crate::developer_offline::validate(&cmd)?;
    let leases = crate::run_leases::shared(&cmd).map_err(crate::native_mods::cache_error)?;
    #[cfg(windows)]
    let cmd = crate::windows_runtime::redirect(cmd);
    #[cfg(windows)]
    let guard = crate::windows_runtime::prepare(&cmd).map_err(|e| {
        crate::i18n::trf(
            "Could not run {program}: {error}",
            &[
                ("program", &cmd.get_program().to_string_lossy()),
                ("error", &e.to_string()),
            ],
        )
    })?;
    Ok((
        cmd,
        RuntimeGuard {
            _leases: leases,
            #[cfg(windows)]
            _windows: guard,
        },
    ))
}

/// Spawn a terminal-attached game through the same lifetime protocol as GUI launches.
pub fn spawn_attached(
    cmd: &mut Command,
    profile: Option<&crate::browser::sunrise::txn::Lease>,
    runtime: &RuntimeGuard,
) -> std::io::Result<Child> {
    let _permission =
        native_client_handoff(cmd, profile, runtime).map_err(std::io::Error::other)?;
    crate::run_leases::spawn(cmd, false, profile)
}

fn native_client_handoff(
    cmd: &mut Command,
    profile: Option<&crate::browser::sunrise::txn::Lease>,
    _runtime: &RuntimeGuard,
) -> Result<Option<crate::local_json::LockedJson>, String> {
    let Some(owner) = profile else {
        return Ok(None);
    };
    let active = owner.verify_launch()?;
    #[cfg(windows)]
    {
        let copy = _runtime
            ._windows
            .as_ref()
            .ok_or_else(|| "The profile's runtime copy was not prepared.".to_string())?;
        active.verify_runtime_copy(copy.game())?;
    }
    if !active.allow_accelerator() {
        // The game can generate a cache on a later run, but an unbound or changed cache
        // must never supply bytes in place of the primary prepared files.
        cmd.env("SUNRISE_NO_DATA_FILE", "1");
    }
    crate::native_status::publication_guard(active.executable(), None)
}

pub fn spawn(
    job: Job,
    cmd: Command,
    tx: Sender<Msg>,
    profile: Option<&crate::browser::sunrise::txn::Lease>,
) -> Result<Running, String> {
    spawn_holding(job, cmd, tx, None, profile)
}

/// Holds the preparation lock of the version that owns `version`'s resources (its base, for a
/// native build), so a second preparation of the same folder can't start while one runs; it is
/// released when the guard drops.
pub fn prepare_lock(version: &str) -> Result<sunrise_launcher_smod::locks::Locked, String> {
    let owner = resources_owner(version);
    prepare_lock_at(&version_dir(&owner), &owner)
}

fn prepare_lock_at(
    dir: &Path,
    version: &str,
) -> Result<sunrise_launcher_smod::locks::Locked, String> {
    let path = dir.join(".preparing.lock");
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    sunrise_launcher_smod::locks::locked(file, sunrise_launcher_smod::locks::LockMode::Exclusive)
        .map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => crate::i18n::trf(
                "Sunrise {version} is already being prepared.",
                &[("version", version)],
            ),
            std::fs::TryLockError::Error(e) => format!("{}: {e}", path.display()),
        })
}

/// [`spawn`], keeping `hold` (a lock) until the program ends; a preparation's whole process group is stopped if the launcher quits first.
pub fn spawn_holding(
    job: Job,
    cmd: Command,
    tx: Sender<Msg>,
    hold: Option<sunrise_launcher_smod::locks::Locked>,
    profile: Option<&crate::browser::sunrise::txn::Lease>,
) -> Result<Running, String> {
    let (mut cmd, runtime) = prepare_command(cmd)?;
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    crate::process::hide_console(&mut cmd);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = {
        let _permission = if job == Job::Client {
            native_client_handoff(&mut cmd, profile, &runtime)?
        } else {
            None
        };
        play_log::spawn(&mut cmd, profile)?
    };
    let pid = child.id();
    let group = matches!(job, Job::Prepare);
    if group {
        crate::process::owned::CHILDREN.track_group(pid);
    }
    let stdin = child.stdin.take();
    let mut readers = Vec::new();
    let out = child.stdout.take();
    let err = child.stderr.take();
    for stream in [out, err].into_iter().flatten() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                let _ = tx.send(Msg::Line(job, line));
            }
        }));
    }
    let child = Arc::new(Mutex::new(child));
    let waited = child.clone();
    std::thread::spawn(move || {
        for r in readers {
            let _ = r.join();
        }
        let status = loop {
            if let Some(s) = waited.lock().unwrap().try_wait().transpose() {
                break s;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        };
        let result = match status {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => Err(format!("Exited with {s}")),
            Err(e) => Err(e.to_string()),
        };
        drop(runtime);
        if group {
            crate::process::owned::CHILDREN.release(pid);
        }
        drop(hold);
        let _ = tx.send(Msg::Exit(job, result));
    });
    Ok(Running { child, stdin })
}

/// The launcher's appearance, as older launchers saved it: a theme is one look now, and a saved
/// `light` opens on the Taiga theme (`Settings::parse`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    Dark,
    Light,
    /// Follow the system's light or dark.
    System,
}

/// What the launcher's window does while the game runs, as the player picks it in Settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WhilePlaying {
    /// The window stays where it is.
    #[default]
    Stay,
    /// The window goes to the Dock or the taskbar, and comes back when the game closes.
    Minimize,
    /// The launcher hides (macOS only: elsewhere it minimises), and comes back when the game
    /// closes.
    Hide,
}

/// How Home lists the worlds under Continue playing, as the player picks it in Settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorldLayout {
    /// One world a row: its picture, its name and its buttons.
    #[default]
    Rows,
    /// A grid of the worlds' pictures.
    Tiles,
}

impl WorldLayout {
    pub const ALL: [WorldLayout; 2] = [Self::Rows, Self::Tiles];

    /// Its name in `settings.json` and on the command line.
    pub fn name(self) -> &'static str {
        match self {
            Self::Rows => "rows",
            Self::Tiles => "tiles",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|l| l.name() == name)
    }
}

impl WhilePlaying {
    pub const ALL: [WhilePlaying; 3] = [Self::Stay, Self::Minimize, Self::Hide];

    /// Its name in `settings.json` and on the command line.
    pub fn name(self) -> &'static str {
        match self {
            Self::Stay => "stay",
            Self::Minimize => "minimize",
            Self::Hide => "hide",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        let name = match name.as_str() {
            "minimise" => "minimize",
            "keep" | "open" | "off" => "stay",
            n => n,
        };
        Self::ALL.into_iter().find(|w| w.name() == name)
    }

    /// What this computer does for it: only macOS can hide a whole app.
    pub fn here(self) -> Self {
        match self {
            Self::Hide if !cfg!(target_os = "macos") => Self::Minimize,
            w => w,
        }
    }
}

/// The roundest the look's corners go, in px (Settings > Appearance > Corners), and how round
/// they are until the player picks.
pub const MAX_CORNERS: u8 = 24;
pub const DEFAULT_CORNERS: u8 = 4;
/// A button's corner radius as the look draws it, and the roundest the player can set, in px.
pub const DEFAULT_CORNER_RADIUS: u8 = 9;
pub const MAX_CORNER_RADIUS: u8 = 30;

/// Custom panel opacity, in whole percentages. Unset keeps the theme's existing opacity.
pub const PANEL_OPACITY_MIN: u8 = 82;
pub const PANEL_OPACITY_MAX: u8 = 100;

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub selected: Option<String>,
    /// Game asset architecture. Auto preserves the launcher's process-architecture behavior.
    #[serde(default)]
    pub game_architecture: GameArchitecture,
    /// The instance Play starts (`instances`), by id. Unset means Sunrise, the game's home.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    pub minecraft_dir: Option<PathBuf>,
    pub server_dir: Option<PathBuf>,
    /// Unset means on: after an install the launcher deletes old automatic downloads nothing needs
    /// (`version_storage::prune_old_versions`). Only a player who turned it off has `false` here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prune_old_versions: Option<bool>,
    /// How many of the newest release versions pruning keeps. Unset means [`version_storage::KEEP_NEWEST`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_versions: Option<usize>,
    /// Off by default, so the opening animation shows unless the player turns it off.
    #[serde(default)]
    pub skip_opening: bool,
    /// Whether the corner cat shows (`cat`). Unset means yes; only a player who switched it off
    /// has `false` here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cat: Option<bool>,
    /// Linux: whether the launcher keeps its entry in the applications menu (`desktop`). Unset
    /// means yes; removing the entry stores `false`, so it doesn't come back on the next start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_entry: Option<bool>,
    /// Extra arguments for the game's command line, as one line (see [`split_args`]). Empty by
    /// default. Appended after the launcher's own arguments, so they can add to or override them.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub game_args: String,
    /// The proxy every download goes through (`proxy`), such as `http://127.0.0.1:7890` or
    /// `socks5://127.0.0.1:1080`.
    /// Unset means none: the environment's `HTTPS_PROXY` and `HTTP_PROXY` apply instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    /// Only read to move an old light appearance to the Taiga theme (`parse`): always dark now,
    /// a theme being one look.
    #[serde(default)]
    pub appearance: Appearance,
    /// The player has read what the "I'm playing" ping holds (`telemetry::PING`): on the
    /// first-run question, or on the Play page once if they answered that before it existed.
    #[serde(default)]
    pub ping_notice_seen: bool,
    /// The player turned the anonymous "I'm playing" ping off (`telemetry::PING`): the game is
    /// started with `SUNRISE_NO_HEARTBEAT`. On by default, as upstream sends it.
    #[serde(default)]
    pub ping_off: bool,
    /// The player has been through the first-run welcome (theme, accent).
    #[serde(default)]
    pub welcomed: bool,
    /// The player without an account that owns the game has read, once, why the launcher needs
    /// one now (the owner gate's screen).
    #[serde(default)]
    pub owner_notice_seen: bool,
    /// What the window does while the game runs. Stays by default.
    #[serde(default)]
    pub while_playing: WhilePlaying,
    /// How Home lists the worlds under Continue playing (`worlds`). Rows by default, and for
    /// every settings file written before the choice existed.
    #[serde(default)]
    pub world_layout: WorldLayout,
    /// The accent colour on top of the appearance (`themes::Accent`). When absent from saved
    /// settings, parsing uses the theme's own accent.
    #[serde(default)]
    pub accent: crate::themes::Accent,
    /// How much white the glass look's plates take, in percent (the mockup's Glass, 0 to 16).
    /// Unset is the mockup's 6.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glass: Option<f32>,
    /// How much the glass look's plates blur the scene behind them, in px (0 to 48). Unset is
    /// the mockup's 24.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blur: Option<f32>,
    /// Explicit glass panel opacity; unset preserves the theme's existing material.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel_opacity: Option<u8>,
    /// The launcher theme (`themes::theme`): its sky behind the light appearance. Unset is Dawn;
    /// the indigo one is `themes::TWILIGHT`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// Keep launcher effects still and use opaque glass. Unset is off. On disk this is
    /// still `performance`, the setting saved by earlier launchers.
    #[serde(
        rename = "performance",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub reduced_graphics: Option<bool>,
    /// Home's background: a picture the player picked, kept in the data folder under this name
    /// (`themes::keep_background`). Unset shows Sunrise's landscape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// The window's language. Unset follows the system's (`i18n::Language::system`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<crate::i18n::Language>,
    /// First-use native code permission for this launcher install, Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_mods_allowed_at: Option<u64>,
    /// Managed cache limit in GiB (unset: 25; zero: no automatic cleanup). Player data isn't cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_limit_gib: Option<u64>,
    /// The accent picked under each launcher theme, by its id (`themes::TWILIGHT` for Twilight's
    /// own), so going back to a theme brings back its colour. A launcher older than this map
    /// drops it when it saves: the themes then only start from their own accents again.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub theme_accents: std::collections::BTreeMap<String, crate::themes::Accent>,
    /// Reduced motion: unset follows the system's, true keeps the launcher still, false lets it
    /// move even when the system asks for less.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reduce_motion: Option<bool>,
    /// How round the look's corners are: their radius in px, 0 (square) to [`MAX_CORNERS`];
    /// unset is [`DEFAULT_CORNERS`] (read it with [`Settings::corners`]).
    #[serde(default, rename = "corners", skip_serializing_if = "Option::is_none")]
    pub corners_px: Option<u8>,
    /// How round every corner in the launcher is, as a button's corner radius in px: 0 square,
    /// [`DEFAULT_CORNER_RADIUS`] the look as drawn, up to [`MAX_CORNER_RADIUS`]; cards and panels
    /// keep their proportion to it. Unset is the default (read it with [`Settings::corner_radius`]).
    #[serde(
        default,
        rename = "corner_radius",
        skip_serializing_if = "Option::is_none"
    )]
    pub corner_radius_px: Option<u8>,
    /// Home's scenery between the sidebars: the theme's animated pixel scene (or the player's
    /// picture). Unset is on; off leaves the plain page (read it with [`Settings::scenery`]).
    #[serde(default, rename = "scenery", skip_serializing_if = "Option::is_none")]
    pub scenery_on: Option<bool>,
    /// Show names alongside game profiles in the sidebar. Starts collapsed.
    #[serde(default)]
    pub sidebar_expanded: bool,
    #[doc(hidden)]
    #[serde(skip)]
    pub settings_baseline: SettingsBaseline,
}

impl Settings {
    /// The look's corner radius in px: the player's, else [`DEFAULT_CORNERS`].
    pub fn corners(&self) -> u8 {
        self.corners_px.unwrap_or(DEFAULT_CORNERS).min(MAX_CORNERS)
    }

    /// The saved panel opacity override. `None` keeps the theme's default paint.
    pub fn panel_opacity(&self) -> Option<u8> {
        self.panel_opacity
    }

    /// Choose a custom panel opacity, within the safe range, or reset to the theme's default.
    pub fn set_panel_opacity(&mut self, opacity: Option<u8>) {
        self.panel_opacity =
            opacity.map(|percent| percent.clamp(PANEL_OPACITY_MIN, PANEL_OPACITY_MAX));
    }

    /// A button's corner radius in px, as the player set it.
    pub fn corner_radius(&self) -> u8 {
        self.corner_radius_px
            .unwrap_or(DEFAULT_CORNER_RADIUS)
            .min(MAX_CORNER_RADIUS)
    }

    /// How round the launcher's corners are, as a factor of the look's radii (1 as drawn).
    pub fn roundness(&self) -> f32 {
        f32::from(self.corner_radius()) / f32::from(DEFAULT_CORNER_RADIUS)
    }

    /// Whether Home shows its scenery: on unless the player turned it off.
    pub fn scenery(&self) -> bool {
        self.scenery_on.unwrap_or(true)
    }
}

impl Settings {
    pub fn load() -> Self {
        Self::load_from(&data_dir())
    }
    pub(crate) fn load_from(data: &Path) -> Self {
        Self::parse(&fs::read_to_string(data.join("settings.json")).unwrap_or_default())
    }
    pub fn parse(text: &str) -> Self {
        let mut value = serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::json!({}));
        // No accent saved: the theme's own (Dawn's for a new install).
        let accent_saved = value.get("accent").is_some();
        if let Some(fields) = value.as_object_mut() {
            // An appearance from a newer launcher is Dark, not a reason to lose every setting.
            if fields
                .get("appearance")
                .is_some_and(|a| serde_json::from_value::<Appearance>(a.clone()).is_err())
            {
                fields.remove("appearance");
            }
            if fields
                .get("while_playing")
                .is_some_and(|w| serde_json::from_value::<WhilePlaying>(w.clone()).is_err())
            {
                fields.remove("while_playing");
            }
            // A layout from a newer launcher is rows.
            if fields
                .get("world_layout")
                .is_some_and(|l| serde_json::from_value::<WorldLayout>(l.clone()).is_err())
            {
                fields.remove("world_layout");
            }
            if fields
                .get("game_architecture")
                .is_some_and(|a| serde_json::from_value::<GameArchitecture>(a.clone()).is_err())
            {
                fields.remove("game_architecture");
            }
            if fields.get("accent").is_some_and(|a| {
                serde_json::from_value::<crate::themes::Accent>(a.clone()).is_err()
            }) {
                fields.remove("accent");
            }
            // A glass or blur that isn't a number is the default, not a reason to lose the rest.
            for key in ["glass", "blur"] {
                if fields.get(key).is_some_and(|v| !v.is_number()) {
                    fields.remove(key);
                }
            }
            // A sidebar choice that isn't yes or no starts collapsed, not a reason to lose the rest.
            if fields
                .get("sidebar_expanded")
                .is_some_and(|v| !v.is_boolean())
            {
                fields.remove("sidebar_expanded");
            }
            look_settings::tidy(fields);
            // A language this launcher doesn't have yet follows the system's.
            if fields.get("language").is_some_and(|l| {
                serde_json::from_value::<Option<crate::i18n::Language>>(l.clone()).is_err()
            }) {
                fields.remove("language");
            }
        }
        let mut settings: Self = serde_json::from_value(value).unwrap_or_default();
        if !accent_saved {
            settings.accent =
                crate::themes::starting_accent(settings.theme.as_deref(), &settings.theme_accents);
        }
        look_settings::settle(&mut settings);
        settings.record_baseline();
        settings
    }
    /// Whether the corner cat shows.
    pub fn show_cat(&self) -> bool {
        self.cat.unwrap_or(true)
    }
    /// Whether the launcher keeps its entry in the applications menu (Linux).
    pub fn desktop_entry(&self) -> bool {
        self.desktop_entry.unwrap_or(true)
    }
    /// The window's language: the one picked, else the system's.
    pub fn language(&self) -> crate::i18n::Language {
        self.language.unwrap_or_else(crate::i18n::Language::system)
    }
    /// Whether old versions are deleted after an install.
    pub fn auto_prune(&self) -> bool {
        self.prune_old_versions.unwrap_or(true)
    }
    /// The plates' white as a fraction (0 to 0.16): [`glass`](Self::glass) in percent.
    pub fn glass_tint(&self) -> f32 {
        self.glass
            .filter(|v| v.is_finite())
            .map_or(sunrise_launcher_theme::glass::TINT, |v| {
                v.clamp(0., 16.) / 100.
            })
    }
    /// The plates' effective blur in px (0 to 48), never applied under Reduced graphics.
    pub fn glass_blur(&self) -> f32 {
        if self.reduced_graphics() {
            return 0.;
        }
        self.blur
            .filter(|v| v.is_finite())
            .map_or(sunrise_launcher_theme::glass::BLUR, |v| v.clamp(0., 48.))
    }
    /// Whether the launcher reduces graphics effects.
    pub fn reduced_graphics(&self) -> bool {
        self.reduced_graphics.unwrap_or(false)
    }
    /// How many of the newest release versions pruning keeps: at least the newest.
    pub fn keep_newest(&self) -> usize {
        self.keep_versions
            .unwrap_or(version_storage::KEEP_NEWEST)
            .max(1)
    }
}

#[cfg(test)]
pub(crate) use installation::install_as;

#[cfg(test)]
#[path = "backend/eula_tests.rs"]
mod eula_tests;

#[cfg(test)]
#[path = "backend/release_loading_tests.rs"]
mod release_loading_tests;

#[cfg(test)]
#[path = "backend/java_tests.rs"]
mod java_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering::*;
    use std::ffi::OsStr;

    #[test]
    fn with_every_name_taken_the_fallback_is_never_a_claimed_folder() {
        let temp = Path::new("/scratch/tmp");
        let short = Some(PathBuf::from("C:/PROGRA~3/ok"));
        assert_eq!(
            build_fallback(short, 7, temp),
            PathBuf::from("C:/PROGRA~3/ok")
        );
        // No short name, or one that is no better than the long one.
        let expected = temp.join("sunrise-launcher-7");
        assert_eq!(build_fallback(None, 7, temp), expected);
        assert_eq!(
            build_fallback(Some(PathBuf::from("C:/Users/Ж/x")), 7, temp),
            expected
        );
    }

    #[test]
    fn a_build_folder_made_by_another_account_is_passed_over() {
        let base = Path::new("C:/ProgramData/sunrise-launcher-1a2b3c4d");
        // The first two are taken by another account; the third is free.
        let claimed = first_claimed_dir(base, |dir| {
            let name = dir.file_name().unwrap().to_str().unwrap();
            name.ends_with("-3")
        });
        assert_eq!(
            claimed,
            Some(PathBuf::from("C:/ProgramData/sunrise-launcher-1a2b3c4d-3"))
        );
        assert_eq!(first_claimed_dir(base, |_| true), Some(base.to_path_buf()));
        assert_eq!(first_claimed_dir(base, |_| false), None);
    }

    #[test]
    fn a_cached_server_jar_that_fails_its_sha1_is_fetched_again() {
        let dir = std::env::temp_dir().join(format!("sunrise-jar-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let jar = dir.join("server-jars/26.3.jar");
        let good = b"the real server jar".to_vec();
        // The sha1 of `good`, from `printf 'the real server jar' | sha1sum`.
        let manifest = serde_json::json!({"downloads": {"server": {
            "url": "https://piston-data.mojang.com/server.jar",
            "sha1": "0527c3af27483cb5b7a62b046066baf42588a449"}}});
        let served = |body: Vec<u8>| move |_: &str| Ok(body.clone());
        let nothing = |_: String| {};
        // A captive portal's page was cached: it is replaced.
        fs::create_dir_all(jar.parent().unwrap()).unwrap();
        fs::write(&jar, b"<html>sign in</html>").unwrap();
        ensure_server_jar(&jar, &manifest, &served(good.clone()), &nothing).unwrap();
        assert_eq!(fs::read(&jar).unwrap(), good);
        // A whole cached jar is not fetched again.
        let refuse = |_: &str| -> Result<Vec<u8>, String> { Err("fetched".into()) };
        ensure_server_jar(&jar, &manifest, &refuse, &nothing).unwrap();
        // A bad download is refused and never cached.
        fs::remove_file(&jar).unwrap();
        let error = ensure_server_jar(&jar, &manifest, &served(b"cut".to_vec()), &nothing);
        assert!(error.is_err());
        assert!(!jar.exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_server_jar_whose_version_file_has_no_sha1_is_kept_and_not_checked() {
        let dir = std::env::temp_dir().join(format!("sunrise-jar-nosha-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let jar = dir.join("server-jars/26.3.jar");
        let manifest = serde_json::json!({"downloads": {"server": {
            "url": "https://piston-data.mojang.com/server.jar"}}});
        let nothing = |_: String| {};
        let served = |_: &str| Ok(b"whatever came down".to_vec());
        // With nothing to check against, a download is kept as it came.
        ensure_server_jar(&jar, &manifest, &served, &nothing).unwrap();
        assert_eq!(fs::read(&jar).unwrap(), b"whatever came down");
        // And a cached jar is trusted, not fetched again.
        let refuse = |_: &str| -> Result<Vec<u8>, String> { Err("fetched".into()) };
        ensure_server_jar(&jar, &manifest, &refuse, &nothing).unwrap();
        assert_eq!(fs::read(&jar).unwrap(), b"whatever came down");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_launchers_build_is_its_selected_one_when_installed_else_the_newest() {
        let installed = ["v0.4.0".to_owned(), "v0.3.0".to_owned()];
        assert_eq!(launcher_build(Some("v0.3.0"), &installed), Some("v0.3.0"));
        assert_eq!(launcher_build(Some("v0.2.0"), &installed), Some("v0.4.0"));
        assert_eq!(launcher_build(None, &installed), Some("v0.4.0"));
        assert_eq!(launcher_build(Some("v0.3.0"), &[]), None);
    }

    #[test]
    fn an_installed_build_keeps_its_release_file_only_when_it_is_the_signed_one() {
        let dir = tempfile::tempdir().unwrap();
        let file = br#"{"version_id":"26.4-pre1","vanilla":"26.4 Pre-Release 1"}"#;
        let signed = sunrise_launcher_release_verify::sha256_hex(file);
        assert!(keep_release_file(dir.path(), b"{}", &signed).is_err());
        assert!(!dir.path().join(RELEASE_FILE).exists());
        keep_release_file(dir.path(), file, &signed).unwrap();
        let kept = read_json(&dir.path().join(RELEASE_FILE)).unwrap();
        assert_eq!(kept["version_id"], "26.4-pre1");
    }

    #[test]
    fn the_played_game_comes_from_the_release_file_else_the_lock_else_the_pin() {
        let versions = tempfile::tempdir().unwrap();
        let v = versions.path();
        let played = |selected, field| played_in(v, selected, field);
        assert_eq!(played(None, "version_id"), PINNED_GAME);
        for (tag, files) in [
            ("v0.3.1", vec![]),
            (
                "v0.3.2",
                vec![("research-lock.json", r#"{"minecraft":{"version":"26.2"}}"#)],
            ),
            (
                "v0.3.3",
                vec![
                    ("research-lock.json", r#"{"minecraft":{"version":"26.2"}}"#),
                    (
                        "sunrise-release.json",
                        r#"{"version_id":"26.4-pre1","vanilla":"26.4 Pre-Release 1"}"#,
                    ),
                ],
            ),
            (
                "v0.3.4",
                vec![("research-lock.json", r#"{"minecraft":{"version":"26.9"}}"#)],
            ),
        ] {
            fs::create_dir_all(v.join(tag)).unwrap();
            for (name, text) in files {
                fs::write(v.join(tag).join(name), text).unwrap();
            }
        }
        assert_eq!(
            played(None, "version_id"),
            PINNED_GAME,
            "only installed builds count"
        );
        for tag in ["v0.3.1", "v0.3.2", "v0.3.3"] {
            fs::write(v.join(tag).join(COMPLETE), "").unwrap();
        }
        assert_eq!(played(None, "version_id"), "26.4-pre1");
        assert_eq!(played(None, "vanilla"), "26.4 Pre-Release 1");
        assert_eq!(played(Some("v0.3.2"), "vanilla"), "26.2");
        assert_eq!(played(Some("v0.3.1"), "version_id"), PINNED_GAME);
        assert_eq!(played(Some("v9.9.9"), "version_id"), "26.4-pre1");
        // A build's own game, and nothing for one that isn't installed: no other build's.
        assert_eq!(game_in(v, "v0.3.3").as_deref(), Some("26.4-pre1"));
        assert_eq!(game_in(v, "v0.3.2").as_deref(), Some("26.2"));
        assert_eq!(game_in(v, "v9.9.9"), None);
        assert_eq!(game_in(v, "v0.3.4"), None, "not complete: not installed");
    }

    fn pe_architecture(bytes: &[u8]) -> Option<GameArchitecture> {
        if bytes.get(..2)? != b"MZ" {
            return None;
        }
        let pe = u32::from_le_bytes(bytes.get(0x3c..0x40)?.try_into().ok()?) as usize;
        if bytes.get(pe..pe.checked_add(4)?)? != b"PE\x00\x00" {
            return None;
        }
        architecture_for_pe_machine(u16::from_le_bytes(
            bytes
                .get(pe.checked_add(4)?..pe.checked_add(6)?)?
                .try_into()
                .ok()?,
        ))
    }

    #[test]
    #[cfg(windows)]
    fn failed_removal_is_incomplete_and_can_be_retried() {
        use std::os::windows::fs::OpenOptionsExt;
        let Some(root) = std::env::var_os("REMOVE_TEST_ROOT") else {
            let root = tempfile::tempdir().unwrap();
            assert!(Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "backend::tests::failed_removal_is_incomplete_and_can_be_retried"
                ])
                .env("REMOVE_TEST_ROOT", root.path())
                .env("SUNRISE_LAUNCHER_HOME", root.path())
                .env("SUNRISE_HOME", root.path().join("home"))
                .env("XDG_DATA_HOME", root.path().join("xdg-data"))
                .env("XDG_CONFIG_HOME", root.path().join("xdg-config"))
                .env("SUNRISE_LAUNCHER_TRASH", root.path().join("trash"))
                .env("SUNRISE_LAUNCHER_CLIPBOARD", root.path().join("clipboard"))
                .env("SUNRISE_LAUNCHER_DESKTOP", root.path().join("desktop"))
                .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
                .env("SUNRISE_NO_AUDIO", "1")
                .status()
                .unwrap()
                .success());
            return;
        };
        let versions = PathBuf::from(root).join("versions");
        let dir = versions.join("v0.2.0");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(COMPLETE), b"").unwrap();
        fs::write(dir.join(CLIENT), sunrise_launcher_smod::holder::CAPABILITY).unwrap();
        crate::run_leases::publish(&dir).unwrap();
        let running = fs::File::options()
            .read(true)
            .share_mode(1)
            .open(dir.join(CLIENT))
            .unwrap();
        assert!(crate::cache_cleanup::delete_game_unit(&dir).is_err());
        assert!(installed_in(&versions).is_empty());
        assert!(!dir.join(COMPLETE).exists());
        drop(running);
        crate::cache_cleanup::delete_game_unit_observed(&dir, || {
            assert!(matches!(
                crate::run_leases::installation(),
                Err(crate::native_cache::Error::Busy)
            ));
        })
        .unwrap();
        assert!(!dir.exists());
    }

    /// A build from before runtime leases has no lease marker in its client; with nothing
    /// running from it, removing it must still work.
    #[test]
    fn a_version_without_the_lease_marker_can_be_removed_when_idle() {
        const ACTOR: &str = "SUNRISE_TEST_PRELEASE_REMOVAL";
        if std::env::var_os(ACTOR).is_none() {
            // Deletion scans every managed profile before proceeding. Do not let unrelated
            // tests' deliberately invalid or changing profiles share this fixture's roots.
            let temp = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "backend::tests::a_version_without_the_lease_marker_can_be_removed_when_idle",
                    "--nocapture",
                ])
                .env(ACTOR, "1")
                .env("SUNRISE_LAUNCHER_HOME", temp.path().join("data"))
                .env("SUNRISE_HOME", temp.path().join("game"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let dir = version_dir("v0.0.1-preleases");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(COMPLETE), b"").unwrap();
        fs::write(dir.join(CLIENT), b"a client from before leases").unwrap();
        assert!(installed().contains(&"v0.0.1-preleases".to_string()));
        remove_version("v0.0.1-preleases").unwrap();
        assert!(!dir.exists());
        assert!(!installed().contains(&"v0.0.1-preleases".to_string()));
    }

    /// A canonical path loses Windows' verbatim prefix on a drive (horizon-mixins' output
    /// check stats `\\?\C:`, the volume itself, which Windows refuses, os error 1), and keeps
    /// it where there is no drive form.
    #[test]
    fn a_canonical_windows_path_loses_only_a_drives_verbatim_prefix() {
        let plain = |p: &str| plain(PathBuf::from(p));
        assert_eq!(
            plain(r"\\?\C:\Users\me\AppData\Roaming\sunrise-launcher\native"),
            PathBuf::from(r"C:\Users\me\AppData\Roaming\sunrise-launcher\native")
        );
        // A share has no drive form to fall back to; other paths have no prefix.
        assert_eq!(
            plain(r"\\?\UNC\server\share\native"),
            PathBuf::from(r"\\?\UNC\server\share\native")
        );
        assert_eq!(
            plain("/home/me/.local/share/sunrise-launcher/native"),
            PathBuf::from("/home/me/.local/share/sunrise-launcher/native")
        );
    }

    /// [#5189](https://github.com/horizon-rust/horizon/issues/5189): MinGW-w64's tools can't
    /// read a non-ASCII path, so the build root is the data folder's resolved ASCII spelling —
    /// which a link's target or a short (8.3) name already is.
    #[test]
    fn the_build_dir_is_a_path_the_tools_can_read() {
        assert!(ascii(Path::new(
            r"C:\Users\me\AppData\Roaming\sunrise-launcher"
        )));
        assert!(!ascii(Path::new(
            r"C:\Users\Кириченко Павло\AppData\Roaming\sunrise-launcher"
        )));
        assert!(!ascii(Path::new("/høme/me/sunrise-launcher")));
        // A space splits windres' include folders, which matters on Windows only.
        let spaced = Path::new(r"C:\Users\Jane Doe\AppData\Roaming\sunrise-launcher");
        assert!(ascii(spaced));
        assert_eq!(tool_safe(spaced), !cfg!(windows));
        let built = build_dir();
        assert!(ascii(&built) || cfg!(windows), "{}", built.display());
        // Resolved: the build never sees a link in the path (where the answer is the data
        // folder itself, so already resolved — Windows' other spellings aren't its own).
        if ascii(&built) {
            assert_eq!(
                built,
                built
                    .canonicalize()
                    .map(plain)
                    .unwrap_or_else(|_| built.clone())
            );
        }
    }

    /// [#6009](https://github.com/horizon-rust/horizon/issues/6009): with a Cyrillic user name the
    /// build folder is a real ASCII folder of ProgramData's (not a short name of the data folder,
    /// which rustc resolves back to the Cyrillic one), its own for each data folder.
    #[test]
    fn a_cyrillic_data_folder_builds_in_an_ascii_folder_of_its_own() {
        let pavlo = Path::new(r"C:\Users\Кириченко Павло\AppData\Roaming\sunrise-launcher");
        let other = Path::new(r"C:\Users\Олена\AppData\Roaming\sunrise-launcher");
        let public = PathBuf::from(if cfg!(windows) { r"D:\Data" } else { "/data" });
        let dir = ascii_build_dir(pavlo, Some(public.clone()));
        assert!(ascii(&dir), "{}", dir.display());
        assert_eq!(dir.parent(), Some(public.as_path()));
        let name = dir.file_name().unwrap().to_str().unwrap();
        assert!(
            name.starts_with("sunrise-launcher-") && name.len() == 25,
            "{name}"
        );
        assert_eq!(dir, ascii_build_dir(pavlo, Some(public.clone())), "stable");
        assert_ne!(dir, ascii_build_dir(other, Some(public)), "one per user");
        // %ProgramData% unset, relative or itself not ASCII: C:\ProgramData.
        let default = PathBuf::from(r"C:\ProgramData");
        for pd in [None, Some("ProgramData".into()), Some(r"C:\Дані".into())] {
            let dir = ascii_build_dir(pavlo, pd);
            assert_eq!(dir.parent(), Some(default.as_path()));
            assert!(ascii(&dir));
        }
    }

    /// The launcher downloads Mojang's game files (client jar, libraries, assets) only for an
    /// account the services said owns the game: never without one, and never past the gate by
    /// the developer option (`Settings::dev_offline`), whose pass has no owner.
    #[test]
    fn mojang_downloads_only_for_an_owner() {
        let dir = std::env::temp_dir().join(format!("sunrise-owner-only-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let pins = sources::Pins::read(&serde_json::json!({ "minecraft": {
            "version": "26.3",
            "client_sha1": "00",
            "manifest_sha1": "00",
        }}))
        .unwrap();
        let asked = std::sync::Mutex::new(Vec::<String>::new());
        let fetch = |url: &str| -> Result<Vec<u8>, String> {
            asked.lock().unwrap().push(url.to_string());
            Err("no network in tests".into())
        };
        let log = |_: String| {};
        let no_owner = || Ok(account::Launch::default());
        for (pass, signed_in) in [
            (Err(account::NoOwner::default()), false),
            (Err(account::NoOwner::default()), true),
            (no_owner(), false),
            (no_owner(), true),
        ] {
            assert!(download_for(&pins, &dir, pass, signed_in, &fetch, &log).is_err());
        }
        assert!(asked.lock().unwrap().is_empty());
        assert!(!dir.exists());
        // With no account, the message stands alone once its bullet is taken off.
        let said = download_for(
            &pins,
            &dir,
            Err(account::NoOwner::default()),
            false,
            &fetch,
            &log,
        )
        .unwrap_err();
        assert_eq!(
            said.trim_start_matches("\n  • "),
            "Sign in with the Microsoft account that owns Minecraft: Sunrise then downloads 26.3 from Mojang itself."
        );
        // An owner: Mojang is asked (and the download fails here, without network).
        let owner = account::Account {
            uuid: "069a79f444e94726a5befca90e38aaf5".into(),
            name: "Notch".into(),
            skin: None,
            cape: None,
        };
        let secrets = serde_json::from_str(
            r#"{"refresh_token":"r","minecraft_token":"t","minecraft_expires_at":1,"owns":true}"#,
        )
        .unwrap();
        let pass = Ok(account::Launch {
            owner: Some((owner, secrets)),
            ..Default::default()
        });
        assert!(download_for(&pins, &dir, pass, true, &fetch, &log).is_err());
        assert!(!asked.lock().unwrap().is_empty());
        let _ = fs::remove_dir_all(&dir);

        // A store that lacks assets is not filled for a non-owner either: nothing is asked.
        fs::create_dir_all(dir.join("assets/indexes")).unwrap();
        fs::write(
            dir.join("assets/indexes/34.json"),
            r#"{"objects":{"a":{"hash":"00","size":1}}}"#,
        )
        .unwrap();
        asked.lock().unwrap().clear();
        for (pass, signed_in) in [(Err(account::NoOwner::default()), true), (no_owner(), true)] {
            assert!(download_for(&pins, &dir, pass, signed_in, &fetch, &log).is_err());
        }
        assert!(asked.lock().unwrap().is_empty());
        let _ = fs::remove_dir_all(&dir);

        // In the sources: one call downloads the game, in `download_for`, and the only caller
        // of that asks the strict gate. Game-file hosts appear nowhere else.
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = vec![src.clone()];
        let mut sources = Vec::new();
        while let Some(path) = files.pop() {
            if path.is_dir() {
                files.extend(fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
            } else if path.extension().is_some_and(|e| e == "rs") {
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                let in_tests = name.contains("tests") || path.parent().unwrap() != src;
                // The code alone: test modules (this one included) come after it.
                // A Windows checkout has CRLF line ends: the searches below look for "\n".
                let mut text = fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
                text.truncate(text.find("#[cfg(test)]").unwrap_or(text.len()));
                sources.push((name, in_tests, text));
            }
        }
        let calls: Vec<_> = sources
            .iter()
            .filter(|(_, tests, _)| !tests)
            .flat_map(|(name, _, text)| {
                text.matches("mojang::download(")
                    .map(move |_| name.as_str())
            })
            .collect();
        assert_eq!(calls, ["backend.rs"]);
        let backend = &sources.iter().find(|(n, ..)| n == "backend.rs").unwrap().2;
        let body = |f: &str| {
            let start = backend.find(&format!("\nfn {f}(")).unwrap();
            &backend[start..start + backend[start + 1..].find("\n}\n").unwrap()]
        };
        assert!(body("download_for").contains("mojang::download("));
        assert!(body("download_for_owner").contains("account::launch("));
        assert!(!body("download_for_owner").contains("launch_for_game("));
        // `download_for` is defined once and called once, by `download_for_owner`.
        assert_eq!(backend.matches("download_for(").count(), 2);
        assert!(body("download_for_owner").contains("download_for("));
        // Refilling an incomplete store asks the same gate and names no host or download itself.
        assert!(body("refill_own_store").contains("download_for_owner("));
        assert!(!body("refill_own_store").contains("mojang::"));
        // It runs at start-up, never on Play, which also runs `complete_assets`.
        let public = |f: &str| {
            let start = backend
                .find(&format!(
                    "
pub fn {f}("
                ))
                .unwrap();
            &backend[start
                ..start
                    + backend[start + 1..]
                        .find(
                            "
}
",
                        )
                        .unwrap()]
        };
        assert!(public("reuse_prepared_everywhere").contains("refill_own_store("));
        assert!(!public("complete_assets").contains("refill_own_store("));
        for host in [
            "piston-data",
            "libraries.minecraft.net",
            "resources.download.minecraft.net",
        ] {
            for (name, tests, text) in &sources {
                assert!(
                    *tests || name == "mojang.rs" || !text.contains(host),
                    "{name} names {host}"
                );
            }
        }
    }

    /// A preparation that reuses the imported game files still reads the assets from the
    /// launcher that has them: a Prism Launcher instance's are in the launcher root, linked into
    /// the official layout (jan.sonki on Discord: silent, grey title screen since 0.2.48).
    #[test]
    fn reused_imports_take_the_assets_from_the_launcher_that_has_them() {
        let dir =
            std::env::temp_dir().join(format!("sunrise-reused-assets-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let client = b"fake client jar";
        let manifest = br#"{"id": "26.3"}"#;
        let write = |path: &str, data: &[u8]| {
            let path = dir.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, data).unwrap();
        };
        let prism = "PrismLauncher";
        write(
            &format!("{prism}/libraries/com/mojang/minecraft/26.3/minecraft-26.3-client.jar"),
            client,
        );
        write(&format!("{prism}/assets/indexes/34.json"), b"{}");
        let instance = dir.join(prism).join("instances/26.3/minecraft");
        fs::create_dir_all(&instance).unwrap();
        let pins = sources::Pins::read(&serde_json::json!({ "minecraft": {
            "version": "26.3",
            "client_sha1": sources::sha1_hex(client),
            "manifest_sha1": sources::sha1_hex(manifest),
        }}))
        .unwrap();
        let linked = dir.join("linked");
        let fetch = |_: &str| Ok(manifest.to_vec());
        let lines = std::cell::RefCell::new(Vec::new());
        let log = |line: String| lines.borrow_mut().push(line);
        let resolved = sources::resolve(&instance, &pins, &[], &linked, &fetch, &log);
        let folder = assets_folder(resolved, &instance, "26.3", &log);
        assert_eq!(folder, linked);
        assert!(folder.join("assets/indexes/34.json").is_file());

        // Nothing found: the chosen folder, and a line saying what the game will lack.
        let empty = dir.join("empty");
        fs::create_dir_all(&empty).unwrap();
        let resolved = sources::resolve(&empty, &pins, &[], &linked, &fetch, &log);
        assert_eq!(assets_folder(resolved, &empty, "26.3", &log), empty);
        assert!(lines.borrow().last().unwrap().contains("will be silent"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn native_resources_resolve_to_the_base_and_prevent_its_removal() {
        const ACTOR: &str = "SUNRISE_RESOURCE_TEST_ACTOR";
        if std::env::var_os(ACTOR).is_none() {
            let home = tempfile::tempdir().unwrap();
            assert!(Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "backend::tests::native_resources_resolve_to_the_base_and_prevent_its_removal",
                    "--nocapture"
                ])
                .env(ACTOR, "1")
                .env("SUNRISE_LAUNCHER_HOME", home.path())
                .status()
                .unwrap()
                .success());
            return;
        }
        let base = format!("resource-test-{}", std::process::id());
        let native = format!("native-resource-test-{}", std::process::id());
        let base_dir = version_dir(&base);
        let native_dir = version_dir(&native);
        fs::create_dir_all(&base_dir).unwrap();
        fs::create_dir_all(&native_dir).unwrap();
        fs::write(base_dir.join(COMPLETE), "").unwrap();
        fs::write(native_dir.join(COMPLETE), "").unwrap();
        fs::write(native_dir.join("native-mods.json"), serde_json::json!({
            "base": base, "sunrise": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "target": "client"
        }).to_string()).unwrap();
        assert_eq!(resources(&native), resources(&base));
        // Preparing a native build prepares the base whose resources it reads.
        assert_eq!(resources_owner(&native), base);
        assert_eq!(resources_owner(&base), base);
        // Two native builds on one base, and the base itself, share one preparation lock.
        let sibling = format!("{native}-sibling");
        fs::create_dir_all(version_dir(&sibling)).unwrap();
        fs::copy(
            native_dir.join("native-mods.json"),
            version_dir(&sibling).join("native-mods.json"),
        )
        .unwrap();
        {
            let _held = prepare_lock(&native).unwrap();
            assert!(prepare_lock(&base).is_err());
            assert!(prepare_lock(&sibling).is_err());
        }
        assert!(prepare_lock(&base).is_ok());
        // Preparing a native build runs the base's preparer, and says so when it can't.
        let try_prepare = || {
            preparer(&native, Path::new("."), MinecraftFrom::Install, &|_| {})
                .map(|_| ())
                .unwrap_err()
        };
        assert_eq!(try_prepare(), crate::preparer::too_old(&base));
        fs::remove_file(base_dir.join(COMPLETE)).unwrap();
        let error = try_prepare();
        assert!(
            error.contains(&base) && !error.contains("too old"),
            "{error}"
        );
        fs::write(base_dir.join(COMPLETE), "").unwrap();
        fs::remove_dir_all(version_dir(&sibling)).unwrap();
        assert!(crate::native_mods::resource_dependents(&base));
        assert!(remove_version(&base).is_err());
        assert!(base_dir.join(COMPLETE).exists());
        fs::remove_dir_all(&base_dir).unwrap();
        let error = need_prepared(&native).unwrap_err();
        assert!(error.contains(&base), "{error}");
        fs::remove_dir_all(&native_dir).unwrap();
        assert!(!crate::native_mods::resource_dependents(&base));
    }

    #[test]
    fn resources_without_sounds_are_reported() {
        let dir = std::env::temp_dir().join(format!("sunrise-no-sounds-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("assets/minecraft/textures")).unwrap();
        fs::write(dir.join(RESOURCES_MANIFEST), "{}").unwrap();
        assert!(!sounds_in(&dir));
        fs::write(dir.join("assets/minecraft/sounds.json"), "{}").unwrap();
        assert!(sounds_in(&dir));
        fs::remove_dir_all(&dir).unwrap();

        assert_eq!(
            resources_summary(true, true, "26.3"),
            "Prepared for Minecraft 26.3"
        );
        let silent = resources_summary(true, false, "26.3");
        assert!(silent.contains("without its sounds") && silent.contains("Prepare again"));
        // Not prepared says only that, sounds or not.
        assert_eq!(
            resources_summary(false, false, "26.3"),
            resources_summary(false, true, "26.3")
        );
    }

    #[test]
    fn each_mac_picks_the_build_for_its_processor() {
        assert_eq!(
            platform_assets("macos", "aarch64"),
            Some((
                "sunrise-macos-arm64.zip",
                "sunrise-launcher-macos-arm64.zip"
            ))
        );
        assert_eq!(
            platform_assets("macos", "x86_64"),
            Some(("sunrise-macos-x64.zip", "sunrise-launcher-macos-x64.zip"))
        );
        assert_eq!(
            platform_assets("windows", "x86_64"),
            Some((
                "sunrise-windows-x64.zip",
                "sunrise-launcher-windows-x64.zip"
            ))
        );
        assert_eq!(
            platform_assets("windows", "aarch64"),
            Some((
                "sunrise-windows-arm64.zip",
                "sunrise-launcher-windows-arm64.zip"
            ))
        );
        assert_eq!(platform_assets("linux", "aarch64"), None);
        assert_eq!(platform_assets("macos", "x86"), None);
        assert_eq!(
            ASSET,
            platform_assets(std::env::consts::OS, std::env::consts::ARCH).map(|(game, _)| game)
        );
        assert_eq!(
            game_fallback_for("windows", "aarch64"),
            Some("sunrise-windows-x64.zip")
        );
        assert_eq!(game_fallback_for("windows", "x86_64"), None);
        assert_eq!(game_fallback_for("macos", "aarch64"), None);
    }

    /// A snapshot is never listed, so no update or version list offers it, even once one is
    /// installed (`release_selection::snapshot` finds one by its tag alone).
    #[test]
    fn a_snapshot_is_never_offered_or_picked_as_an_update() {
        const SNAPSHOT: &str = "snapshot-2026-11-13-bbbbbbb";
        let zip = "sunrise-windows-x64.zip";
        let asset = |tag: &str| serde_json::json!([{"name": zip, "browser_download_url": format!("https://example.test/{tag}/{zip}"), "size": 1}]);
        let list = serde_json::json!([
            {"tag_name": SNAPSHOT, "name": "Snapshot 2026-11-13 · bbbbbbb", "prerelease": true,
             "published_at": "2026-11-13T10:00:00Z", "assets": asset(SNAPSHOT)},
            {"tag_name": "nightly", "name": "Nightly 2026-11-12 · aaaaaaa", "prerelease": true,
             "published_at": "2026-11-12T10:00:00Z", "assets": asset("nightly")},
            {"tag_name": "v0.2.81", "name": "v0.2.81", "published_at": "2026-10-01T10:00:00Z",
             "assets": asset("v0.2.81")},
        ]);
        let releases = game_releases_with(&list, Some(zip), None);
        assert_eq!(
            releases.iter().map(|r| r.tag.as_str()).collect::<Vec<_>>(),
            ["nightly-2026-11-12-aaaaaaa", "v0.2.81"]
        );
        for installed in [vec![], vec![SNAPSHOT.to_owned()]] {
            let update = sunrise_update(&releases, &installed).unwrap();
            assert_eq!(update.tag, "nightly-2026-11-12-aaaaaaa", "{installed:?}");
        }
        // A snapshot the developer selected stays selected when that Nightly installs as an
        // update, whether it is the only version installed or beside a release.
        for installed in [
            vec![SNAPSHOT.to_owned()],
            vec![SNAPSHOT.to_owned(), "v0.2.81".into()],
        ] {
            assert!(
                !select_after_update(Some(SNAPSHOT), &installed),
                "{installed:?}"
            );
        }
    }

    /// The ARM64 Windows launcher installs the ARM64 build where a release has one, and the x64
    /// build (under emulation) from the releases that have only that, checked against that file's
    /// signed checksum.
    #[test]
    fn an_arm64_windows_launcher_falls_back_to_the_x64_game() {
        let (arm, x64) = ("sunrise-windows-arm64.zip", "sunrise-windows-x64.zip");
        let asset = |name: &str, url: &str| serde_json::json!({"name": name, "browser_download_url": url, "size": 1});
        let list = serde_json::json!([
            {"tag_name": "v2026.11.05", "assets": [
                asset(x64, "https://example.test/v2026.11.05/sunrise-windows-x64.zip"),
                asset(arm, "https://example.test/v2026.11.05/sunrise-windows-arm64.zip")]},
            {"tag_name": "v2026.11.04", "assets": [
                asset(x64, "https://example.test/v2026.11.04/sunrise-windows-x64.zip")]},
            {"tag_name": "v2026.11.03", "assets": []},
        ]);
        let releases = game_releases_with(&list, Some(arm), Some(x64));
        let urls: Vec<_> = releases
            .iter()
            .map(|r| {
                r.asset
                    .as_ref()
                    .map(|(url, _)| url.rsplit('/').next().unwrap())
            })
            .collect();
        assert_eq!(urls, [Some(arm), Some(x64), None]);
        let installs: Vec<_> = releases
            .iter()
            .map(|r| asset_installed(r, Some(arm), Some(x64)))
            .collect();
        assert_eq!(installs, [Some(arm), Some(x64), None]);

        // An x64 launcher, on any Windows, never looks at the ARM64 build.
        let x64_only = game_releases_with(&list, Some(x64), None);
        assert!(x64_only[..2]
            .iter()
            .all(|r| r.asset.as_ref().unwrap().0.ends_with(x64)));
    }

    #[test]
    fn a_game_architecture_choice_is_independent_of_the_launcher_process() {
        let (arm, x64) = ("sunrise-windows-arm64.zip", "sunrise-windows-x64.zip");
        // GetNativeSystemInfo reports ARM64 even when std::env::consts::ARCH is x86_64.
        assert!(supports_game_architecture_choice(
            "windows",
            Some("aarch64")
        ));
        assert!(!supports_game_architecture_choice(
            "windows",
            Some("x86_64")
        ));
        assert!(!supports_game_architecture_choice("linux", Some("aarch64")));

        // An x64 launcher keeps the legacy x64 default, but can opt into ARM64.
        assert_eq!(
            game_assets_for("windows", Some("aarch64"), "x86_64", GameArchitecture::Auto),
            Some((x64, None))
        );
        assert_eq!(
            game_assets_for(
                "windows",
                Some("aarch64"),
                "x86_64",
                GameArchitecture::Arm64
            ),
            Some((arm, Some(x64)))
        );

        // The ARM64 launcher defaults to ARM64 and can choose x64 for compatibility.
        assert_eq!(
            game_assets_for(
                "windows",
                Some("aarch64"),
                "aarch64",
                GameArchitecture::Auto
            ),
            Some((arm, Some(x64)))
        );
        assert_eq!(
            game_assets_for("windows", Some("aarch64"), "aarch64", GameArchitecture::X64),
            Some((x64, None))
        );

        // Native x64 Windows and the other platforms keep their old process-based selection.
        assert_eq!(
            game_assets_for("windows", Some("x86_64"), "x86_64", GameArchitecture::Arm64),
            Some((x64, None))
        );
        assert_eq!(
            game_assets_for("linux", None, "x86_64", GameArchitecture::Arm64),
            Some(("sunrise-linux-x86_64.tar.gz", None))
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_native_architecture_detection_sees_the_host_under_emulation() {
        let native = native_windows_architecture().expect("supported Windows machine");
        assert!(matches!(native, "x86_64" | "aarch64"));
        if std::env::var_os("SUNRISE_TEST_EXPECT_NATIVE_ARM64").is_some() {
            assert_eq!(std::env::consts::ARCH, "x86_64");
            assert_eq!(native, "aarch64");
        }
        if native == "aarch64" {
            assert!(supports_game_architecture_choice("windows", Some(native)));
            let (asset, fallback) = game_assets_for(
                "windows",
                Some(native),
                std::env::consts::ARCH,
                GameArchitecture::Auto,
            )
            .unwrap();
            if std::env::consts::ARCH == "x86_64" {
                assert_eq!((asset, fallback), ("sunrise-windows-x64.zip", None));
            } else {
                assert_eq!(
                    (asset, fallback),
                    ("sunrise-windows-arm64.zip", Some("sunrise-windows-x64.zip"))
                );
            }
        }
    }

    #[test]
    fn selected_architecture_uses_available_assets_and_only_the_safe_fallback() {
        let arm = "sunrise-windows-arm64.zip";
        let x64 = "sunrise-windows-x64.zip";
        let asset = |name: &str| serde_json::json!({"name": name, "browser_download_url": format!("https://example.test/{name}"), "size": 1});
        let list = serde_json::json!([
            {"tag_name": "v2026.11.05", "assets": [asset(x64), asset(arm)]},
            {"tag_name": "v2026.11.04", "assets": [asset(x64)]},
            {"tag_name": "v2026.11.03", "assets": []}
        ]);

        let arm_choice = game_releases_with(&list, Some(arm), Some(x64));
        assert_eq!(
            arm_choice
                .iter()
                .map(|release| release
                    .asset
                    .as_ref()
                    .and_then(|(url, _)| url.rsplit('/').next()))
                .collect::<Vec<_>>(),
            [Some(arm), Some(x64), None]
        );
        assert_eq!(
            asset_installed(&arm_choice[0], Some(arm), Some(x64)),
            Some(arm)
        );
        assert_eq!(
            asset_installed(&arm_choice[1], Some(arm), Some(x64)),
            Some(x64)
        );
        assert_eq!(asset_installed(&arm_choice[2], Some(arm), Some(x64)), None);

        let x64_choice = game_releases_with(&list, Some(x64), None);
        assert!(x64_choice.iter().take(2).all(|release| release
            .asset
            .as_ref()
            .unwrap()
            .0
            .ends_with(x64)));
        assert_eq!(
            asset_installed(&arm_choice[0], Some(x64), None),
            None,
            "a stale ARM64 release selection must not install under an x64 choice"
        );
    }

    #[test]
    fn sidebar_starts_collapsed_and_remembers_both_positions() {
        let mut settings = Settings::parse("{}");
        assert!(!settings.sidebar_expanded);
        for expanded in [true, false] {
            settings.sidebar_expanded = expanded;
            let saved = serde_json::to_string(&settings).unwrap();
            assert_eq!(Settings::parse(&saved).sidebar_expanded, expanded);
        }
    }

    #[test]
    fn selected_game_build_and_safe_fallback_satisfy_the_saved_choice() {
        let fallback = InstalledGameArchitecture {
            requested: GameArchitecture::Arm64,
            binary: GameArchitecture::X64,
        };
        assert!(installed_architecture_matches(
            GameArchitecture::Arm64,
            fallback
        ));
        assert!(installed_architecture_matches(
            GameArchitecture::X64,
            fallback
        ));
        assert!(!installed_architecture_matches(
            GameArchitecture::Arm64,
            InstalledGameArchitecture {
                requested: GameArchitecture::X64,
                binary: GameArchitecture::X64,
            }
        ));
    }

    #[test]
    fn updates_reinstall_only_when_the_selected_game_architecture_needs_a_new_build() {
        let arm_fallback = InstalledGameArchitecture {
            requested: GameArchitecture::Arm64,
            binary: GameArchitecture::X64,
        };
        let old_x64 = InstalledGameArchitecture {
            requested: GameArchitecture::X64,
            binary: GameArchitecture::X64,
        };
        let arm = InstalledGameArchitecture {
            requested: GameArchitecture::Arm64,
            binary: GameArchitecture::Arm64,
        };

        assert!(!game_architecture_install_needed(
            GameArchitecture::Arm64,
            GameArchitecture::X64,
            Some(arm_fallback),
        ));
        assert!(game_architecture_install_needed(
            GameArchitecture::Arm64,
            GameArchitecture::X64,
            Some(old_x64),
        ));
        assert!(game_architecture_install_needed(
            GameArchitecture::Arm64,
            GameArchitecture::Arm64,
            Some(arm_fallback),
        ));
        assert!(!game_architecture_install_needed(
            GameArchitecture::Arm64,
            GameArchitecture::Arm64,
            Some(arm),
        ));
    }

    #[test]
    fn native_mod_builds_report_when_the_selected_game_has_no_matching_compiler() {
        assert_eq!(
            native_mod_architecture_message(GameArchitecture::X64, GameArchitecture::Arm64),
            Some(
                "Native mods cannot build for the selected ARM64 game yet. The available compiler builds x64 only."
            )
        );
        assert_eq!(
            native_mod_architecture_message(GameArchitecture::Arm64, GameArchitecture::X64),
            Some(
                "Native mods cannot build for the selected x64 game yet. This ARM64 launcher has no matching compiler."
            )
        );
        assert_eq!(
            native_mod_architecture_message(GameArchitecture::Arm64, GameArchitecture::Arm64),
            None
        );
    }

    #[test]
    fn windows_pe_machine_types_identify_the_installed_game_architecture() {
        let pe = |machine: u16| {
            let mut bytes = vec![0; 0x86];
            bytes[..2].copy_from_slice(b"MZ");
            bytes[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
            bytes[0x80..0x84].copy_from_slice(b"PE\x00\x00");
            bytes[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
            bytes
        };
        assert_eq!(pe_architecture(&pe(0x8664)), Some(GameArchitecture::X64));
        assert_eq!(pe_architecture(&pe(0xaa64)), Some(GameArchitecture::Arm64));
        assert_eq!(pe_architecture(b"not a PE executable"), None);
    }

    /// launcher.yml publishes a release even when its ARM64 build failed. An ARM64 launcher then
    /// stays on its version until a release has its own zip: it never takes the x64 one.
    #[test]
    fn an_arm64_launcher_skips_a_release_without_its_zip() {
        let (arm, x64) = (
            "sunrise-launcher-windows-arm64.zip",
            "sunrise-launcher-windows-x64.zip",
        );
        let asset = |name: &str| serde_json::json!({"name": name, "browser_download_url": format!("https://example.test/{name}"), "size": 1});
        let list = serde_json::json!([
            {"tag_name": "v1.3.0", "assets": [asset(x64)]},
            {"tag_name": "v1.2.0", "assets": [asset(x64), asset(arm)]},
        ]);
        let releases = parse_launcher_releases(&list, Some(arm));
        assert!(launcher_update_from(&releases, "v1.2.0").is_none());
        let update = launcher_update_from(&releases, "v1.1.0").unwrap();
        assert_eq!(update.tag, "v1.2.0");
        assert!(update.asset.unwrap().0.ends_with(arm));
    }

    #[test]
    fn the_game_plays_under_its_own_audio_stream_name() {
        if std::env::var_os("PIPEWIRE_PROPS").is_some() {
            return;
        }
        let cmd = client_command("v2026.10.13");
        let props = cmd
            .get_envs()
            .find(|(k, _)| *k == "PIPEWIRE_PROPS")
            .and_then(|(_, v)| v);
        assert!(props.is_some_and(|v| {
            v.to_string_lossy()
                .contains("application.name = \"Sunrise\"")
        }));
    }

    #[test]
    fn a_crash_after_the_vulkan_adapter_line_and_before_the_first_frame_is_a_start_crash() {
        let start = [
            "Sunrise: resources loaded in 11.3s; 35723 block states, 1658 items, 67 biomes",
            "Sunrise renderer: Intel(R) HD Graphics 530 (Vulkan, IntegratedGpu)",
        ];
        assert!(vulkan_start_crashed(start));
        // A step line after the adapter keeps it a start crash.
        let step = [start[1], "Sunrise renderer: creating the pipelines"];
        assert!(vulkan_start_crashed(step));
        // A crash after the first frame is the game's, not the renderer start's.
        let later = [start[1], "Sunrise: first frame after 812 ms"];
        assert!(!vulkan_start_crashed(later));
        // Already on DirectX 12, or no renderer yet: nothing to switch.
        let dx12 = ["Sunrise renderer: Intel(R) HD Graphics 530 (Dx12, IntegratedGpu)"];
        assert!(!vulkan_start_crashed(dx12));
        assert!(!vulkan_start_crashed(&start[..1]));
        // Vulkan failed to open and DirectX 12 took over: its line is the last adapter.
        assert!(!vulkan_start_crashed([start[1], dx12[0]]));
    }

    #[test]
    fn a_renderer_start_crash_switches_auto_to_dx12_once() {
        let home = std::env::temp_dir().join(format!("sunrise-dx12-switch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        // No settings file yet: one with the API alone.
        assert_eq!(switch_from_auto(&home, "dx12"), Ok(true));
        assert_eq!(
            fs::read_to_string(home.join(GRAPHICS_FILE)).unwrap(),
            "graphicsApi:dx12\n"
        );
        // Already DirectX 12, or the player's own Vulkan: left alone.
        assert_eq!(switch_from_auto(&home, "dx12"), Ok(false));
        fs::write(home.join(GRAPHICS_FILE), "sky:false\ngraphicsApi:vulkan\n").unwrap();
        assert_eq!(switch_from_auto(&home, "dx12"), Ok(false));
        // Auto becomes DirectX 12 and the other settings stay.
        fs::write(
            home.join(GRAPHICS_FILE),
            "sky:false\ngraphicsApi:auto\nfog:true\n",
        )
        .unwrap();
        assert_eq!(switch_from_auto(&home, "dx12"), Ok(true));
        assert_eq!(
            fs::read_to_string(home.join(GRAPHICS_FILE)).unwrap(),
            "sky:false\ngraphicsApi:dx12\nfog:true\n"
        );
        let _ = fs::remove_dir_all(&home);
    }

    /// #4293: an Ivy Bridge on Linux whose Vulkan driver (Mesa's hasvk) crashes at the renderer's
    /// start plays on OpenGL next time; Windows keeps DirectX 12 (#746), macOS has nothing else.
    #[test]
    fn a_vulkan_start_crash_on_linux_switches_auto_to_opengl() {
        assert_eq!(vulkan_crash_fallback("windows").unwrap().0, "dx12");
        assert_eq!(vulkan_crash_fallback("macos"), None);
        let (api, _) = vulkan_crash_fallback("linux").unwrap();
        let home = std::env::temp_dir().join(format!("sunrise-gl-switch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join(GRAPHICS_FILE), "graphicsApi:auto\nvsync:true\n").unwrap();
        assert_eq!(switch_from_auto(&home, api), Ok(true));
        assert_eq!(
            fs::read_to_string(home.join(GRAPHICS_FILE)).unwrap(),
            "graphicsApi:opengl\nvsync:true\n"
        );
        // The next crash (OpenGL had no GPU either and Vulkan ran again) leaves it: no loop.
        assert_eq!(switch_from_auto(&home, api), Ok(false));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn extra_game_arguments_split_like_a_shell() {
        assert!(split_args("").is_empty());
        assert!(split_args("   ").is_empty());
        assert_eq!(split_args("--server-thread"), ["--server-thread"]);
        assert_eq!(
            split_args("--render-distance 32 --server-thread"),
            ["--render-distance", "32", "--server-thread"]
        );
        // Quotes keep a word with spaces together; either kind works.
        assert_eq!(
            split_args("--name 'Two Words' --world \"My World\""),
            ["--name", "Two Words", "--world", "My World"]
        );
        // An empty quoted word is still a word.
        assert_eq!(split_args("--name \"\""), ["--name", ""]);
        // A backslash escapes a space outside quotes, and " and \ inside double quotes.
        assert_eq!(split_args("--name My\\ World"), ["--name", "My World"]);
        assert_eq!(split_args("a\\tb c"), ["atb", "c"]);
        assert_eq!(split_args("\"a\\\"b\" 'c\\d'"), ["a\"b", "c\\d"]);
    }

    #[test]
    fn the_game_arguments_setting_round_trips_and_reaches_the_command() {
        let settings = Settings::parse(r#"{"game_args":"--server-thread"}"#);
        assert_eq!(settings.game_args, "--server-thread");
        // Unset in an older settings.json: no arguments, and nothing is written for them.
        let old = Settings::parse(r#"{"installed":true}"#);
        assert_eq!(old.game_args, "");
        assert!(!serde_json::to_string(&old).unwrap().contains("game_args"));

        let mut cmd = Command::new("true");
        cmd.args(split_args("--server-thread --render-distance 40"));
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, ["--server-thread", "--render-distance", "40"]);
    }

    #[test]
    fn tags_never_mix() {
        assert!(is_game_tag("v2026.10.13"));
        assert!(is_game_tag("v0.2.0"));
        assert!(!is_game_tag("launcher@1.0.0"));
        assert!(!is_game_tag("vnext"));
        assert_eq!(launcher_version("v1.2.0"), Some("1.2.0"));
        assert_eq!(launcher_version("vnext"), None);
        assert_eq!(launcher_version("launcher@1.0.0"), None);
        // Each list comes from its own repository: the game's still carries `launcher@1.0.0`, never a game release.
        let file = |name: &str| serde_json::json!([{"name": name, "browser_download_url": "u", "size": 1}]);
        let game_list = serde_json::json!([
            {"tag_name": "v2026.10.12", "assets": file("sunrise-linux-x86_64.tar.gz")},
            {"tag_name": "launcher@1.0.0", "assets": file("sunrise-launcher-linux-x86_64.tar.gz")},
            {"tag_name": "v2026.10.13", "assets": []},
        ]);
        let game: Vec<_> = parse_releases(&game_list, ASSET, is_game_tag)
            .into_iter()
            .map(|r| r.tag)
            .collect();
        assert_eq!(game, ["v2026.10.13", "v2026.10.12"]);
        let launcher_list = serde_json::json!([
            {"tag_name": "v1.1.0", "assets": file("sunrise-launcher-linux-x86_64.tar.gz")},
            {"tag_name": "v1.0.9", "draft": true, "assets": []},
        ]);
        let launcher: Vec<_> = parse_launcher_releases(&launcher_list, LAUNCHER_ASSET)
            .into_iter()
            .map(|r| r.tag)
            .collect();
        assert_eq!(launcher, ["v1.1.0"]);
    }

    /// Rolling a release back (`gh release edit <tag> --draft`, docs/release-flow.md) makes the
    /// previous one the newest again: it is what `install latest` and the update check offer.
    #[test]
    fn withdrawn_release_is_never_offered() {
        let asset = ASSET.unwrap_or("sunrise-linux-x86_64.tar.gz");
        let list = serde_json::json!([
            {"tag_name": "v2026.10.15.1", "draft": true, "assets": [{"name": asset, "browser_download_url": "bad", "size": 1}]},
            {"tag_name": "v2026.10.15", "assets": [{"name": asset, "browser_download_url": "good", "size": 1}]},
            {"tag_name": "v2026.10.14", "assets": [{"name": asset, "browser_download_url": "old", "size": 1}]},
        ]);
        let releases = parse_releases(&list, Some(asset), is_game_tag);
        assert!(releases.iter().all(|r| r.tag != "v2026.10.15.1"));
        assert_eq!(
            latest(&releases).map(|r| r.tag.as_str()),
            Some("v2026.10.15")
        );
        let offered = |installed: &[&str]| {
            let installed: Vec<String> = installed.iter().map(|t| t.to_string()).collect();
            sunrise_update(&releases, &installed).map(|r| r.tag)
        };
        assert_eq!(offered(&["v2026.10.14"]), Some("v2026.10.15".into()));
        assert_eq!(offered(&[]), Some("v2026.10.15".into()));
        // Who installed the withdrawn one is not told to "update" to an older version.
        assert_eq!(offered(&["v2026.10.15.1", "v2026.10.14"]), None);
    }

    /// Every release is published as the release, and launchers update to it. The pre-releases
    /// of the old Alpha channel (v0.2.1 on) are only installed by hand: a launcher that
    /// had one installed waits for the next release instead of going back.
    #[test]
    fn updates_follow_every_release_and_leave_the_old_alpha_ones_out() {
        let asset = ASSET.unwrap_or("sunrise-linux-x86_64.tar.gz");
        let list = serde_json::json!([
            {"tag_name": "v0.2.13", "assets": [{"name": asset, "browser_download_url": "n", "size": 1}]},
            {"tag_name": "v0.2.12", "prerelease": true, "assets": [{"name": asset, "browser_download_url": "a", "size": 1}]},
            {"tag_name": "v0.2.0", "assets": [{"name": asset, "browser_download_url": "s", "size": 1}]},
        ]);
        let releases = parse_releases(&list, Some(asset), is_game_tag);
        assert_eq!(
            releases.iter().map(|r| r.prerelease).collect::<Vec<_>>(),
            [false, true, false]
        );
        let offered = |releases: &[Release], installed: &[&str]| {
            let installed: Vec<String> = installed.iter().map(|t| t.to_string()).collect();
            sunrise_update(&installable_updates(releases), &installed).map(|r| r.tag)
        };
        assert_eq!(offered(&releases, &[]), Some("v0.2.13".into()));
        assert_eq!(offered(&releases, &["v0.2.0"]), Some("v0.2.13".into()));
        assert_eq!(offered(&releases, &["v0.2.12"]), Some("v0.2.13".into()));
        assert_eq!(offered(&releases, &["v0.2.13"]), None);
        // Before the first release of the single channel: the old Alpha one stays where it is.
        let before = &releases[1..];
        assert_eq!(offered(before, &["v0.2.0"]), None);
        assert_eq!(offered(before, &["v0.2.12", "v0.2.0"]), None);
        assert_eq!(CHECK_EVERY.as_secs(), 30 * 60);
    }

    #[test]
    fn worlds_are_rows_unless_tiles_are_picked() {
        assert_eq!(Settings::default().world_layout, WorldLayout::Rows);
        assert_eq!(Settings::parse("{}").world_layout, WorldLayout::Rows);
        // A file from before the choice existed keeps everything else it says.
        let old = Settings::parse(r#"{"while_playing": "hide", "game_args": "--x"}"#);
        assert_eq!(
            (old.world_layout, old.while_playing, old.game_args.as_str()),
            (WorldLayout::Rows, WhilePlaying::Hide, "--x")
        );
        let tiles = Settings::parse(r#"{"world_layout": "tiles", "game_args": "--x"}"#);
        assert_eq!(tiles.world_layout, WorldLayout::Tiles);
        let saved = serde_json::to_string(&tiles).unwrap();
        assert!(saved.contains(r#""world_layout":"tiles""#), "{saved}");
        assert_eq!(Settings::parse(&saved).world_layout, WorldLayout::Tiles);
        // A layout this launcher doesn't know, or one that isn't a name, loses only itself.
        for text in [
            r#"{"world_layout": "carousel", "game_args": "--x"}"#,
            r#"{"world_layout": 3, "game_args": "--x"}"#,
        ] {
            let future = Settings::parse(text);
            assert_eq!(
                (future.world_layout, future.game_args.as_str()),
                (WorldLayout::Rows, "--x"),
                "{text}"
            );
        }
        assert_eq!(WorldLayout::parse(" Tiles "), Some(WorldLayout::Tiles));
        assert_eq!(WorldLayout::parse("grid"), None);
    }

    #[test]
    fn the_window_stays_while_playing_unless_picked() {
        assert_eq!(Settings::parse("{}").while_playing, WhilePlaying::Stay);
        let hide = Settings::parse(r#"{"while_playing": "hide", "game_args": "--x"}"#);
        assert_eq!(hide.while_playing, WhilePlaying::Hide);
        let future = Settings::parse(r#"{"while_playing": "fade", "game_args": "--x"}"#);
        assert_eq!(
            (future.while_playing, future.game_args.as_str()),
            (WhilePlaying::Stay, "--x")
        );
        assert_eq!(
            WhilePlaying::parse("Minimise"),
            Some(WhilePlaying::Minimize)
        );
        assert_eq!(WhilePlaying::parse("off"), Some(WhilePlaying::Stay));
        assert_eq!(WhilePlaying::parse("close"), None);
        for w in WhilePlaying::ALL {
            let text = serde_json::to_string(&w).unwrap();
            assert_eq!(text, format!("\"{}\"", w.name()));
        }
        let here = WhilePlaying::Hide.here();
        assert_eq!(here == WhilePlaying::Hide, cfg!(target_os = "macos"));
    }

    #[test]
    fn the_game_architecture_defaults_to_auto_and_persists_manual_choices() {
        assert_eq!(
            Settings::parse("{}").game_architecture,
            GameArchitecture::Auto
        );
        let arm = Settings::parse(r#"{"game_architecture":"arm64","game_args":"--x"}"#);
        assert_eq!(
            (arm.game_architecture, arm.game_args.as_str()),
            (GameArchitecture::Arm64, "--x")
        );
        let saved = serde_json::to_value(&arm).unwrap();
        assert_eq!(saved["game_architecture"], "arm64");

        let unknown = Settings::parse(r#"{"game_architecture":"future","game_args":"--x"}"#);
        assert_eq!(
            (unknown.game_architecture, unknown.game_args.as_str()),
            (GameArchitecture::Auto, "--x")
        );
        assert_eq!(GameArchitecture::parse("X64"), Some(GameArchitecture::X64));
        assert_eq!(GameArchitecture::parse("other"), None);
    }

    #[test]
    fn the_language_follows_the_system_unless_picked() {
        use crate::i18n::Language;
        assert_eq!(Settings::parse("{}").language, None);
        let fr = Settings::parse(r#"{"language": "fr"}"#);
        assert_eq!(fr.language(), Language::French);
        let future = Settings::parse(r#"{"language": "tlh", "game_args": "--x"}"#);
        assert_eq!((future.language, future.game_args.as_str()), (None, "--x"));
        let saved = serde_json::to_value(Settings::default()).unwrap();
        assert!(saved.get("language").is_none(), "unset is left out");
    }

    #[test]
    fn glass_and_blur_default_to_the_mockups_and_stay_in_range() {
        let unset = Settings::parse("{}");
        assert_eq!((unset.glass_tint(), unset.glass_blur()), (0.06, 24.));
        let set = Settings::parse(r#"{"glass": 12, "blur": 8.5}"#);
        assert_eq!((set.glass_tint(), set.glass_blur()), (0.12, 8.5));
        let wild = Settings::parse(r#"{"glass": 90, "blur": -3}"#);
        assert_eq!((wild.glass_tint(), wild.glass_blur()), (0.16, 0.));
        // Not a number: the default, and the other settings survive.
        let wrong = Settings::parse(r#"{"glass": "frosty", "blur": null, "game_args": "--x"}"#);
        assert_eq!(
            (
                wrong.glass_tint(),
                wrong.glass_blur(),
                wrong.game_args.as_str()
            ),
            (0.06, 24., "--x")
        );
    }

    #[test]
    fn reduced_graphics_turns_off_blur_without_forgetting_its_value() {
        let mut s = Settings::parse(r#"{"blur":8.5,"performance":true}"#);
        assert_eq!(s.glass_blur(), 0.);
        let saved = serde_json::to_string(&s).unwrap();
        s = Settings::parse(&saved);
        assert_eq!(s.glass_blur(), 0.);
        s.reduced_graphics = Some(false);
        assert_eq!(s.glass_blur(), 8.5);
    }

    #[test]
    fn a_malformed_sidebar_choice_keeps_every_other_setting_through_a_save() {
        for bad in ["null", "\"yes\"", "1", "[true]"] {
            let saved = format!(
                r#"{{"sidebar_expanded": {bad}, "instance": "skyblock", "ping_off": true,
                    "selected": "nightly-2026-11-13-aaaaaaa", "game_args": "--x"}}"#
            );
            // What the sidebar's toggle does: load, flip, save, and what the next start reads.
            let mut loaded = Settings::parse(&saved);
            assert!(!loaded.sidebar_expanded, "{bad} starts collapsed");
            loaded.sidebar_expanded = !loaded.sidebar_expanded;
            let again = Settings::parse(&serde_json::to_string(&loaded).unwrap());
            assert!(again.sidebar_expanded);
            assert_eq!(again.instance.as_deref(), Some("skyblock"), "{bad}");
            assert!(again.ping_off, "{bad} must not turn the ping back on");
            assert_eq!(
                again.selected.as_deref(),
                Some("nightly-2026-11-13-aaaaaaa")
            );
            assert_eq!(again.game_args, "--x");
        }
        assert!(Settings::parse(r#"{"sidebar_expanded": true}"#).sidebar_expanded);
    }

    #[test]
    fn a_new_install_is_dawn_and_an_unknown_accent_keeps_the_rest() {
        use crate::themes::Accent;
        let fresh = Settings::parse("{}");
        assert_eq!(crate::themes::accent_key(fresh.theme.as_deref()), "dawn");
        assert_eq!(
            fresh.accent,
            crate::themes::starting_accent(Some("dawn"), &Default::default())
        );
        assert_ne!(fresh.accent, Accent::VIOLET);
        assert_eq!(
            Settings::parse(r#"{"theme": "sunrise"}"#).accent,
            Accent::VIOLET
        );
        let teal =
            Settings::parse(r#"{"accent": "teal", "background": "background-0123456789ab.png"}"#);
        assert_eq!(
            (teal.accent, teal.background.as_deref()),
            (Accent::TEAL, Some("background-0123456789ab.png"))
        );
        let future = Settings::parse(r#"{"accent": "plaid", "game_args": "--x"}"#);
        assert_eq!(
            (future.accent, future.game_args.as_str()),
            (Accent::VIOLET, "--x")
        );
    }

    #[test]
    fn an_old_appearance_moves_to_a_theme_and_an_unknown_one_keeps_the_rest() {
        assert_eq!(Settings::parse("{}").appearance, Appearance::Dark);
        assert_eq!(Settings::parse("{}").theme, None);
        // Light, alone or under a theme, opens on Taiga and keeps the rest.
        for text in [
            r#"{"appearance": "light", "game_args": "--x"}"#,
            r#"{"appearance": "light", "theme": "deep_ocean", "game_args": "--x"}"#,
        ] {
            let light = Settings::parse(text);
            assert_eq!(
                (
                    light.appearance,
                    light.theme.as_deref(),
                    light.game_args.as_str()
                ),
                (Appearance::Dark, Some("taiga"), "--x"),
                "{text}"
            );
        }
        // Dark and the system's keep the saved theme's look.
        for appearance in ["dark", "system"] {
            let text = format!(r#"{{"appearance": "{appearance}", "theme": "nord"}}"#);
            let kept = Settings::parse(&text);
            assert_eq!(
                (kept.appearance, kept.theme.as_deref()),
                (Appearance::Dark, Some("nord"))
            );
        }
        let future =
            Settings::parse(r#"{"appearance": "sepia", "theme": "nord", "game_args": "--x"}"#);
        assert_eq!(
            (
                future.appearance,
                future.theme.as_deref(),
                future.game_args.as_str()
            ),
            (Appearance::Dark, Some("nord"), "--x")
        );
        // Saved back as dark, so a migrated light stays on Taiga only until the player picks.
        let text = serde_json::to_string(&Settings::parse(r#"{"appearance": "light"}"#)).unwrap();
        assert!(text.contains(r#""appearance":"dark""#), "{text}");
    }

    #[test]
    fn the_old_update_channel_settings_are_dropped_and_the_rest_kept() {
        for old in [
            r#"{"update_channel": "alpha", "game_args": "--x", "selected": "v0.2.0"}"#,
            r#"{"update_channel": "stable", "game_args": "--x", "selected": "v0.2.0"}"#,
            r#"{"beta_builds": true, "game_args": "--x", "selected": "v0.2.0"}"#,
            r#"{"beta_builds": false, "update_channel": "nightly", "game_args": "--x", "selected": "v0.2.0"}"#,
        ] {
            let settings = Settings::parse(old);
            assert_eq!(settings.game_args, "--x", "{old}");
            assert_eq!(settings.selected.as_deref(), Some("v0.2.0"), "{old}");
            let saved = serde_json::to_value(&settings).unwrap();
            assert!(saved.get("update_channel").is_none(), "{old}");
            assert!(saved.get("beta_builds").is_none(), "{old}");
        }
    }

    #[test]
    fn the_version_is_the_one_the_release_workflow_injects() {
        assert_eq!(version_or(Some("0.4.2")), "0.4.2");
        assert_eq!(version_or(Some("")), env!("CARGO_PKG_VERSION"));
        assert_eq!(version_or(None), env!("CARGO_PKG_VERSION"));
        assert_eq!(
            VERSION,
            option_env!("SUNRISE_LAUNCHER_VERSION")
                .filter(|v| !v.is_empty())
                .unwrap_or(env!("CARGO_PKG_VERSION"))
        );
        // Self-update compares releases with that version: this launcher's own release is no update.
        let r = |tag: String| Release {
            tag,
            name: String::new(),
            body: String::new(),
            date: String::new(),
            time: String::new(),
            asset: Some(("u".into(), 1)),
            prerelease: false,
            nightly: None,
        };
        assert!(launcher_update(&[r(own_tag())]).is_none());
        assert_eq!(launcher_version(&own_tag()), Some(VERSION));
        assert_eq!(
            parse_version_output(&format!("{}{VERSION}\n", crate::identity::VERSION_PREFIX)),
            Some(VERSION)
        );
    }

    #[test]
    fn an_older_nightly_installs_as_a_rollback_with_a_warning() {
        let nightly = |tag: &str| Release {
            tag: tag.into(),
            name: String::new(),
            body: String::new(),
            date: String::new(),
            time: String::new(),
            asset: Some(("u".into(), 1)),
            prerelease: false,
            nightly: Some("nightly".into()),
        };
        let installed = [
            "v0.2.81".to_owned(),
            "nightly-2026-10-28-0c02067".to_owned(),
        ];
        let warning =
            nightly_downgrade(&nightly("nightly-2026-10-21-aaaaaaa"), &installed).unwrap();
        assert!(
            warning.contains("older than nightly-2026-10-28-0c02067"),
            "{warning}"
        );
        assert!(nightly_downgrade(&nightly("nightly-2026-10-29-bbbbbbb"), &installed).is_none());
        assert!(nightly_downgrade(&nightly("nightly-2026-10-28-0c02067"), &installed).is_none());
        // No Nightly installed, or a release: nothing to warn about.
        assert!(
            nightly_downgrade(&nightly("nightly-2026-10-21-aaaaaaa"), &installed[..1]).is_none()
        );
        let mut release = nightly("v0.2.80");
        release.nightly = None;
        assert!(nightly_downgrade(&release, &installed).is_none());
    }

    #[test]
    fn a_launcher_knows_which_line_it_was_released_on() {
        // release.yml gives the tag; a build without one is its version as a tag of this repository.
        assert_eq!(own_tag_of(Some("v1.1.0"), Some("1.1.0")), "v1.1.0");
        assert_eq!(own_tag_of(None, Some("1.1.0")), "v1.1.0");
        assert_eq!(own_tag_of(Some(""), Some("1.1.0")), "v1.1.0");
        assert_eq!(
            own_tag_of(None, None),
            format!("v{}", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn a_launcher_from_the_game_repository_updates_to_this_repositorys_releases() {
        let r = |tag: &str| Release {
            tag: tag.into(),
            name: String::new(),
            body: String::new(),
            date: String::new(),
            time: String::new(),
            asset: Some(("u".into(), 1)),
            prerelease: false,
            nightly: None,
        };
        let mut list = vec![r("v1.1.0"), r("v1.0.1"), r("v1.2.0")];
        list.sort_by(|a, b| compare_versions(&b.tag, &a.tag));
        let tags: Vec<_> = list.iter().map(|r| r.tag.as_str()).collect();
        assert_eq!(tags, ["v1.2.0", "v1.1.0", "v1.0.1"]);
        let offered = |own: &str| launcher_update_from(&list, own).map(|r| r.tag);
        // 1.0.0 was released from the game repository as launcher@1.0.0.
        assert_eq!(offered("launcher@1.0.0"), Some("v1.2.0".into()));
        assert_eq!(offered("v1.1.0"), Some("v1.2.0".into()));
        assert_eq!(offered("v1.2.0"), None);
        assert_eq!(offered("v1.3.0-rc.1"), None);
        // Only this repository's own tags are launcher releases here.
        assert_eq!(launcher_version("v1.2.0"), Some("1.2.0"));
        assert_eq!(launcher_version("launcher@1.0.0"), None);
        assert_eq!(launcher_version("nightly"), None);
    }

    /// A launcher in an AppImage updates with the AppImage's own file: the `.tar.gz` binary would
    /// be written inside the AppImage's read-only mount.
    #[test]
    fn an_appimage_updates_with_the_appimage() {
        assert_eq!(launcher_asset_running_from(false), LAUNCHER_ASSET);
        assert_eq!(launcher_asset_running_from(true), LAUNCHER_APPIMAGE_ASSET);
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        assert_eq!(
            launcher_asset_running_from(true),
            Some("sunrise-launcher-linux-x86_64.AppImage")
        );
    }

    /// A program opened inside a `.zip` runs from the temporary folder, under a name that depends on
    /// the tool, and is deleted later: an update written there is lost, so such a copy is told to
    /// download the launcher again (#9287). A folder of the same look elsewhere is a real install.
    #[test]
    fn a_launcher_run_from_the_temp_folder_is_a_temporary_copy() {
        let temp = Path::new("/tmp/t");
        let inside = |dir: &str| in_temp_folder(Path::new(dir), temp);
        assert!(inside("/tmp/t"));
        assert!(inside(
            "/tmp/t/Temp12_sunrise-launcher-windows-x64.zip/inner"
        ));
        assert!(inside("/tmp/t/7zO8A1B2C3D"));
        assert!(!inside("/tmp/tools"));
        assert!(!inside("/home/me/Temp1_sunrise.zip"));
    }

    /// The same folder spelled two ways, as %TEMP% and the launcher's own path can be on an account
    /// with a short name: `temp_dir()` as the system gives it against its resolved form, and a
    /// spelling through `..`, which a literal comparison misses on every system.
    #[test]
    fn the_temp_folder_is_recognized_however_it_is_spelled() {
        let root = scratch("temp-spelling");
        let temp = root.join("temp");
        let app = temp.join("Temp1_sunrise.zip");
        let outside = root.join("installed");
        fs::create_dir_all(&app).unwrap();
        fs::create_dir_all(&outside).unwrap();

        assert!(in_temp_folder(
            &app,
            &root.join("installed").join("..").join("temp")
        ));
        assert!(!in_temp_folder(&outside, &temp));

        let system = std::env::temp_dir();
        let long = system.canonicalize().unwrap();
        let copy = long.join(format!("sunrise-temp-copy-{}", std::process::id()));
        fs::create_dir_all(&copy).unwrap();
        let found = in_temp_folder(&copy, &system);
        let _ = fs::remove_dir_all(&copy);
        assert!(found);
        let _ = fs::remove_dir_all(root);
    }

    /// A fresh folder for one test, under the temporary folder.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sunrise-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn stage_named(work: &Path, pid: u32) -> PathBuf {
        let stage = work.join(format!("{pid}-0123456789abcdef0123456789abcdef"));
        fs::create_dir_all(&stage).unwrap();
        fs::write(stage.join("download.zip"), b"archive").unwrap();
        stage
    }

    /// A crashed launcher's stage goes, and nothing another process may still use does: a live
    /// process's however old, a recent one whose process has gone, and one handed to an installer
    /// after sitting in a long-lived window, though its folder is old.
    #[test]
    fn only_abandoned_stages_are_cleaned_up() {
        let work = scratch("stage-cleanup");
        let live = stage_named(&work, 100);
        let crashed = stage_named(&work, 200);
        let handed_over = stage_named(&work, 300);
        let old_launcher_file = work.join("download");
        fs::write(&old_launcher_file, b"partial").unwrap();
        let is_alive = |pid: u32| pid == 100;
        let later = std::time::SystemTime::now() + 2 * STAGE_GRACE;
        let marker = fs::File::create(handed_over.join(HANDED_OVER)).unwrap();
        marker.set_modified(later).unwrap();
        drop(marker);

        remove_abandoned_stages(&work, std::time::SystemTime::now(), is_alive);
        assert!(
            crashed.exists(),
            "gone only just now: its installer may be working"
        );

        remove_abandoned_stages(&work, later, is_alive);
        assert_eq!(fs::read(live.join("download.zip")).unwrap(), b"archive");
        assert!(handed_over.join("download.zip").exists());
        assert!(!crashed.exists());
        assert!(!old_launcher_file.exists());
        let _ = fs::remove_dir_all(work);
    }

    /// Each stage is its own folder, and removing one leaves another process's as it was. Only a
    /// path inside a stage folder removes anything.
    #[test]
    fn removing_a_stage_leaves_the_others_alone() {
        let work = scratch("stage-remove");
        let mine = new_stage_folder(&work).unwrap();
        let theirs = new_stage_folder(&work).unwrap();
        assert_ne!(mine, theirs);
        fs::write(theirs.join("download.zip"), b"their download").unwrap();
        fs::write(mine.join("download.zip"), b"mine").unwrap();

        remove_stage(&work, &work.join("unpacked").join("sunrise-launcher"));
        remove_stage(&work, &mine.join("..").join("sunrise-launcher"));
        remove_stage(&work, &work);
        assert!(mine.exists() && theirs.exists());

        remove_stage(&work, &mine.join("download.zip"));
        assert!(!mine.exists());
        assert_eq!(
            fs::read(theirs.join("download.zip")).unwrap(),
            b"their download"
        );
        let _ = fs::remove_dir_all(work);
    }

    /// A download that doesn't match keeps nothing, not even a temporary file, and doesn't touch a
    /// finished file already there. One that matches appears under its name only.
    #[test]
    fn a_download_takes_its_name_only_after_the_hash_check() {
        let dir = scratch("save-checked");
        let to = dir.join("download.zip");
        let bytes: &[u8] = b"the launcher";
        let right = sunrise_launcher_release_verify::sha256_hex(bytes);
        let wrong = sunrise_launcher_release_verify::sha256_hex(b"something else");
        fs::write(&to, b"finished earlier").unwrap();

        assert!(save_checked(bytes, 0, "asset", &wrong, &to, &|_: f32| {}).is_err());
        assert_eq!(fs::read(&to).unwrap(), b"finished earlier");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);

        save_checked(bytes, 0, "asset", &right, &to, &|_: f32| {}).unwrap();
        assert_eq!(fs::read(&to).unwrap(), bytes);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_process_that_has_exited_is_not_alive() {
        assert!(process_alive(std::process::id()));
        // The test binary itself, which exits at once on an option it doesn't know.
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        child.wait().unwrap();
        drop(child);
        assert!(!process_alive(pid));
    }

    /// A launcher whose folder takes no new file (a disk image, macOS's translocated copy of an
    /// app it was not moved from) can't put the new one in place.
    #[cfg(unix)]
    #[test]
    fn a_read_only_folder_takes_no_update() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("sunrise-read-only-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        assert!(writable(&dir));
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
        // root writes anywhere.
        if unsafe { libc::geteuid() } != 0 {
            assert!(!writable(&dir));
        }
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            0,
            "the probe is removed"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    /// `$APPIMAGE` names the file the runtime mounted; a path that is not there (moved or deleted
    /// since the launcher started) is no AppImage, and the launcher updates its own binary again.
    /// Nor is one inherited from another AppImage's program: the binary has to run from `$APPDIR`.
    // AppImages are Linux's, and the paths below are absolute only on Unix.
    #[cfg(unix)]
    #[test]
    fn only_an_appimage_file_counts() {
        use std::ffi::OsStr;
        let mount = OsStr::new("/tmp/.mount_Sunrise123");
        let exe = Path::new("/tmp/.mount_Sunrise123/usr/bin/sunrise-launcher");
        assert_eq!(appimage_path_in(None, Some(mount), exe), None);
        assert_eq!(
            appimage_path_in(Some(OsStr::new("Sunrise.AppImage")), Some(mount), exe),
            None,
            "a relative path is no $APPIMAGE"
        );
        assert_eq!(
            appimage_path_in(
                Some(OsStr::new("/nonexistent/Sunrise.AppImage")),
                Some(mount),
                exe
            ),
            None
        );
        let file = std::env::temp_dir().join(format!("sunrise-appimage-{}", std::process::id()));
        std::fs::write(&file, b"#!AppImage").unwrap();
        assert_eq!(
            appimage_path_in(Some(file.as_os_str()), Some(mount), exe),
            Some(file.clone())
        );
        // Inherited from another AppImage: this launcher is installed elsewhere.
        let elsewhere = Path::new("/home/a/.local/bin/sunrise-launcher");
        assert_eq!(
            appimage_path_in(Some(file.as_os_str()), Some(mount), elsewhere),
            None
        );
        assert_eq!(appimage_path_in(Some(file.as_os_str()), None, exe), None);
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn game_0_2_follows_every_date_release() {
        assert_eq!(compare_versions("v0.2.0", "v2026.10.19.15"), Greater);
        assert_eq!(compare_versions("v0.2.0", "v2026.10.21"), Greater);
        assert_eq!(compare_versions("v2099.12.31.9", "v0.2.0"), Less);
        assert_eq!(compare_versions("v0.2.1", "v0.2.0"), Greater);
        assert_eq!(compare_versions("v0.2.10", "v0.2.9"), Greater);
        assert_eq!(compare_versions("v0.3.0", "v0.2.57"), Greater);
        assert_eq!(compare_versions("v0.2.0", "v0.2.0"), Equal);
        // Pre-releases come before their release and after the one before.
        assert_eq!(compare_versions("v0.3.0-rc.1", "v0.3.0"), Less);
        assert_eq!(compare_versions("v0.3.0-rc.1", "v0.2.9"), Greater);
        assert_eq!(compare_versions("v0.3.0-beta.11", "v0.3.0-beta.2"), Greater);
        assert_eq!(compare_versions("v0.3.0-rc.1", "v0.3.0-beta.9"), Greater);
        assert_eq!(compare_versions("v0.3.0-beta.1", "v0.3.0-beta"), Greater);
        assert_eq!(compare_versions("v0.3.0-alpha", "v0.3.0-1"), Greater);
        assert_eq!(compare_versions("v0.2.1-beta.1", "v2026.10.21"), Greater);
        // Launcher and game tags of 0.2 compare alike; old launcher tags stay below.
        assert_eq!(
            compare_versions("launcher@0.2.0", "launcher-v0.16.0"),
            Greater
        );
        assert_eq!(compare_versions("launcher@0.2.0", "v0.2.0"), Equal);

        let asset = ASSET.unwrap_or("sunrise-linux-x86_64.tar.gz");
        let list = serde_json::json!([
            {"tag_name": "v0.2.0", "prerelease": true, "assets": [{"name": asset, "browser_download_url": "b", "size": 1}]},
            {"tag_name": "v2026.10.21", "prerelease": true, "assets": [{"name": asset, "browser_download_url": "d", "size": 1}]},
            {"tag_name": "v2026.10.19.15", "assets": [{"name": asset, "browser_download_url": "s", "size": 1}]},
        ]);
        let releases = parse_releases(&list, Some(asset), is_game_tag);
        let installed = ["v2026.10.19.15".to_string(), "v2026.10.12".to_string()];
        // A pre-release 0.2.0 is not an update; once published as the release, it is one.
        let offered = |releases: &[Release]| {
            sunrise_update(&installable_updates(releases), &installed).map(|r| r.tag)
        };
        assert_eq!(offered(&releases), None);
        let mut released = releases.clone();
        released[0].prerelease = false;
        assert_eq!(offered(&released), Some("v0.2.0".into()));
        let mut sorted = vec![
            "v2026.10.19.15".to_string(),
            "v0.2.1".into(),
            "v0.2.0".into(),
            "v2026.10.21".into(),
        ];
        sorted.sort_by(|a, b| compare_versions(b, a));
        assert_eq!(
            sorted,
            ["v0.2.1", "v0.2.0", "v2026.10.21", "v2026.10.19.15"]
        );
    }

    #[test]
    fn only_a_release_list_counts_as_a_checked_launcher_feed() {
        assert!(launcher_releases_from(Ok(Some(serde_json::json!([]))))
            .unwrap()
            .is_empty());
        assert!(launcher_releases_from(Ok(None)).is_err());
        assert!(launcher_releases_from(Ok(Some(serde_json::json!({"message": "x"})))).is_err());
        assert!(launcher_releases_from(Err("offline".into())).is_err());
    }

    #[test]
    fn before_the_first_launcher_release_there_is_no_update() {
        assert!(parse_launcher_releases(&serde_json::json!([]), launcher_asset()).is_empty());
        assert!(launcher_update_from(&[], "v1.0.0").is_none());
    }

    #[test]
    fn launcher_discovery_keeps_only_exact_vxyz_tags() {
        let asset = launcher_asset().unwrap_or("unsupported");
        let release = |tag: &str| {
            serde_json::json!({"tag_name": tag, "assets": [
                {"name": asset, "browser_download_url": "u", "size": 1}
            ]})
        };
        let list = serde_json::json!([
            "v1.1.0",
            "v1.2.0-rc1",
            "v1.2",
            "v1.3.0/../x",
            "launcher@1.4.0"
        ]
        .map(release));
        let tags: Vec<_> = parse_launcher_releases(&list, launcher_asset())
            .into_iter()
            .map(|r| r.tag)
            .collect();
        assert_eq!(tags, ["v1.1.0"]);
    }

    #[test]
    fn versions_compare_by_number() {
        assert_eq!(compare_versions("v2026.10.13", "v2026.10.12"), Greater);
        assert_eq!(compare_versions("v2026.9.30", "v2026.10.1"), Less);
        assert_eq!(compare_versions("v2026.10.13.1", "v2026.10.13"), Greater);
        assert_eq!(compare_versions("0.10.0", "0.9.9"), Greater);
        assert_eq!(compare_versions("launcher-v1.2", "1.2.0"), Equal);
        assert_eq!(compare_versions("1.0.0-rc1", "1.0.0"), Equal);
    }

    #[test]
    fn updates() {
        let r = |tag: &str| Release {
            tag: tag.into(),
            name: String::new(),
            body: String::new(),
            date: String::new(),
            time: String::new(),
            asset: Some(("u".into(), 1)),
            prerelease: false,
            nightly: None,
        };
        let releases = [r("v2026.10.13"), r("v2026.10.12")];
        assert_eq!(
            sunrise_update(&releases, &["v2026.10.12".into()]).map(|r| r.tag),
            Some("v2026.10.13".into())
        );
        assert!(sunrise_update(&releases, &["v2026.10.13".into()]).is_none());
        assert!(launcher_update(&[r("launcher-v0.0.1")]).is_none());
        assert!(launcher_update(&[r("launcher@99.0.0")]).is_some());
        assert!(launcher_update_from(&[r("launcher-v99.0.0")], "launcher-v0.16.0").is_some());
        let installed = ["v2026.10.12".to_string(), "v2026.10.1".to_string()];
        assert!(select_after_update(Some("v2026.10.12"), &installed));
        assert!(!select_after_update(Some("v2026.10.1"), &installed));
        assert!(select_after_update(None, &installed));
        // A Nightly follows updates only from the newest one: an older pick is a rollback.
        let nightlies = [
            "nightly-2026-11-11-bbbbbbb".to_string(),
            "nightly-2026-11-10-aaaaaaa".to_string(),
        ];
        assert!(select_after_update(
            Some("nightly-2026-11-11-bbbbbbb"),
            &nightlies
        ));
        assert!(!select_after_update(
            Some("nightly-2026-11-10-aaaaaaa"),
            &nightlies
        ));
        assert!(!select_after_update(Some("developer-abc"), &[]));
        assert!(!select_after_update(Some("developer-abc"), &installed));
        assert!(!select_after_update(
            Some("imported-2026-11-01-abcdef0"),
            &["imported-2026-11-01-abcdef0".to_string()]
        ));
    }

    /// The Nightly is the only build Sunrise ships (docs/release-flow.md): every launcher takes the
    /// current one as its update, over every release; the last week's are listed, never offered.
    #[test]
    fn every_launcher_updates_to_the_current_nightly() {
        let asset = ASSET.unwrap_or("sunrise-linux-x86_64.tar.gz");
        let build = |url: &str| serde_json::json!([{"name": asset, "browser_download_url": url, "size": 1}]);
        let list = serde_json::json!([
            {"tag_name": "nightly", "name": "Nightly 2026-10-28 · 9b772b2", "prerelease": true,
             "published_at": "2026-10-28T09:58:01Z", "assets": build("n")},
            {"tag_name": "launcher@0.2.21", "assets": []},
            {"tag_name": "v0.2.95", "assets": build("r")},
            {"tag_name": "nightly-2026-10-28", "name": "Nightly 2026-10-28 · 83f15fa", "prerelease": true,
             "published_at": "2026-10-28T07:54:57Z", "assets": build("d")},
            {"tag_name": "nightly-2026-10-27", "name": "Nightly 2026-10-27 · 7954902", "prerelease": true,
             "published_at": "2026-10-28T00:15:40Z", "assets": build("e")},
        ]);
        let releases = game_releases(&list);
        let listed: Vec<_> = releases
            .iter()
            .map(|r| (r.tag.as_str(), r.nightly.as_deref(), r.prerelease))
            .collect();
        assert_eq!(
            listed,
            [
                ("nightly-2026-10-28-9b772b2", Some("nightly"), false),
                (
                    "nightly-2026-10-28-83f15fa",
                    Some("nightly-2026-10-28"),
                    true
                ),
                (
                    "nightly-2026-10-27-7954902",
                    Some("nightly-2026-10-27"),
                    true
                ),
                ("v0.2.95", None, false),
            ]
        );
        let offered = |installed: &[&str]| {
            let installed: Vec<String> = installed.iter().map(|t| t.to_string()).collect();
            sunrise_update(&installable_updates(&releases), &installed).map(|r| r.tag)
        };
        let current = Some("nightly-2026-10-28-9b772b2".to_string());
        // A player of the release channel, a new player, one on an older Nightly of the same day.
        assert_eq!(offered(&["v0.2.95", "v0.2.94"]), current);
        assert_eq!(offered(&[]), current);
        assert_eq!(offered(&["nightly-2026-10-28-83f15fa", "v0.2.95"]), current);
        assert_eq!(offered(&["nightly-2026-10-28-9b772b2"]), None);
        // Nightlies come after every release in every list.
        assert!(compare_versions("nightly-2026-10-27-7954902", "v0.2.95").is_gt());
        assert!(
            compare_versions("nightly-2026-10-28-9b772b2", "nightly-2026-10-27-7954902").is_gt()
        );
    }

    /// Players move to the Nightly at their next check: whoever was on the newest release, on a
    /// Nightly, or on nothing. A release picked by hand stays picked.
    #[test]
    fn players_of_the_newest_release_move_to_the_nightly() {
        let installed = [
            "nightly-2026-10-27-7954902".to_string(),
            "v0.2.95".into(),
            "v0.2.90".into(),
        ];
        assert!(select_after_update(Some("v0.2.95"), &installed));
        assert!(select_after_update(
            Some("nightly-2026-10-27-7954902"),
            &installed
        ));
        assert!(select_after_update(None, &installed));
        assert!(!select_after_update(Some("v0.2.90"), &installed));
        assert!(select_after_update(
            Some("v0.2.95"),
            &["v0.2.95".to_string()]
        ));
    }

    #[test]
    fn old_automatic_download_settings_do_not_discard_other_preferences() {
        let settings = Settings::parse(
            r#"{
            "selected": "v2026.10.13", "auto_install": false,
            "install_new_versions": true, "prune_old_versions": false
        }"#,
        );
        assert_eq!(settings.selected.as_deref(), Some("v2026.10.13"));
        assert!(!settings.auto_prune());
        let saved = serde_json::to_value(&settings).unwrap();
        assert!(saved.get("install_new_versions").is_none());
        assert!(saved.get("auto_install").is_none());
    }

    #[test]
    fn links_fall_back_to_the_built_in_ones() {
        let defaults = Links::default();
        assert_eq!(defaults.discord, "https://discord.gg/eswQcHfRKC");
        assert_eq!(defaults.website, "https://playsunrise.net");
        assert_eq!(defaults.roadmap, "");
        assert_eq!(defaults.docs, "");
        assert_eq!(
            Links::parse(r#"{"roadmap":"https://example.org/tasks","docs":" "}"#),
            Links {
                roadmap: "https://example.org/tasks".into(),
                docs: String::new(),
                ..defaults.clone()
            }
        );
        // The file on main says the same thing as the defaults.
        assert_eq!(Links::parse(include_str!("../links.json")), defaults);
        // Unreadable or missing, offline included: the defaults.
        assert_eq!(Links::parse(""), defaults);
        assert_eq!(Links::parse("<html>404</html>"), defaults);
        assert_eq!(Links::parse("[1]"), defaults);
        assert_eq!(Links::parse(r#"{"discord": 5}"#), defaults);
        // A key that is missing keeps its default; a new address replaces it; empty hides it.
        assert_eq!(
            Links::parse(r#"{"discord": "https://x.y/z"}"#),
            Links {
                discord: "https://x.y/z".into(),
                ..defaults.clone()
            }
        );
        assert_eq!(
            Links::parse(r#"{"discord": "https://x.y/z", "website": " "}"#),
            Links {
                discord: "https://x.y/z".into(),
                website: String::new(),
                ..defaults.clone()
            }
        );
        // A key this launcher does not know is ignored.
        assert_eq!(Links::parse(r#"{"mastodon": "https://x.y/z"}"#), defaults);
    }

    #[test]
    fn a_drawn_page_asks_the_disk_once_and_the_launch_check_asks_again() {
        use std::time::{Duration, Instant};
        let dir =
            std::env::temp_dir().join(format!("sunrise-launcher-cached-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(RESOURCES_MANIFEST), "{}").unwrap();
        fs::write(dir.join("items.json"), "{}").unwrap();
        let version = dir.join("version-folder");
        let cache = Mutex::new(PreparedCache::default());
        let t0 = Instant::now();
        // The page's question: without the cache each of these would run the check.
        let disk_reads = std::cell::Cell::new(0);
        let draw = |now| {
            cached_prepared(&cache, &version, now, || {
                disk_reads.set(disk_reads.get() + 1);
                prepared_at(&dir)
            })
        };
        assert!(draw(t0), "a complete folder is prepared");

        // A table is emptied behind the cache (a killed export): a frame still answers from
        // memory, without touching the disk, for hundreds of frames.
        fs::write(dir.join("items.json"), "").unwrap();
        for frame in 1..=500 {
            assert!(draw(t0 + Duration::from_millis(frame)), "frame {frame}");
        }
        assert_eq!(disk_reads.get(), 1, "500 frames read the disk once");

        // Before a launch the folder is checked on the disk, and finds the empty table.
        assert!(
            !checked_prepared(&cache, &version, &dir, t0),
            "an empty table is caught"
        );
        // ... and the pages then show it without checking again.
        assert!(!draw(t0 + Duration::from_secs(1)));
        assert_eq!(disk_reads.get(), 1);

        // Repaired, the folder is believed prepared again once the answer is dropped or old.
        fs::write(dir.join("items.json"), "{}").unwrap();
        assert!(!draw(t0 + Duration::from_secs(2)), "still the old answer");
        cache.lock().unwrap().0.clear();
        assert!(draw(t0 + Duration::from_secs(2)));
        assert_eq!(disk_reads.get(), 2);
        fs::write(dir.join("items.json"), "").unwrap();
        assert!(draw(t0 + Duration::from_secs(3)), "answer is a second old");
        assert!(
            !draw(t0 + Duration::from_secs(2) + PREPARED_TTL),
            "past the time limit a change made by another process shows"
        );
        assert_eq!(disk_reads.get(), 3);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn each_version_keeps_its_own_prepared_answer() {
        let now = std::time::Instant::now();
        let cache = Mutex::new(PreparedCache::default());
        let (a, b) = (Path::new("versions/a"), Path::new("versions/b"));
        assert!(cached_prepared(&cache, a, now, || true));
        assert!(!cached_prepared(&cache, b, now, || false));
        assert!(cached_prepared(&cache, a, now, || unreachable!(
            "a is known"
        )));
        assert!(!cached_prepared(&cache, b, now, || unreachable!(
            "b is known"
        )));
    }

    #[test]
    fn a_prepared_folder_with_an_empty_table_is_not_prepared() {
        let dir =
            std::env::temp_dir().join(format!("sunrise-launcher-empty-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert!(!prepared_at(&dir), "no manifest is not prepared");

        fs::write(dir.join(RESOURCES_MANIFEST), "{}").unwrap();
        assert!(prepared_at(&dir));

        // A killed export left the biggest table empty: Play must prepare
        // again instead of starting a game that crashes on it.
        fs::write(dir.join("gameplay-facts.json"), "").unwrap();
        assert!(!prepared_at(&dir));

        fs::write(dir.join("gameplay-facts.json"), "{}").unwrap();
        assert!(prepared_at(&dir));

        fs::write(dir.join(RESOURCES_MANIFEST), "").unwrap();
        assert!(!prepared_at(&dir), "an empty manifest is not prepared");
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_version_is_prepared_once_at_a_time() {
        let dir = std::env::temp_dir().join(format!(
            "sunrise-launcher-prepare-lock-{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let first = prepare_lock_at(&dir, "v1").unwrap();
        let second = prepare_lock_at(&dir, "v1").err().unwrap();
        assert!(second.contains("already being prepared"), "{second}");
        drop(first);
        assert!(
            prepare_lock_at(&dir, "v1").is_ok(),
            "free once the first ends"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "daily"]
    fn a_prepared_version_asks_its_preparer_for_a_data_file_once() {
        use crate::preparer::DATA_FILE;
        use std::os::unix::fs::PermissionsExt;
        let root =
            std::env::temp_dir().join(format!("sunrise-launcher-data-file-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let resources = root.join("resources");
        fs::create_dir_all(&resources).unwrap();
        let binary = root.join("sunrise-prepare");
        fs::write(
            &binary,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$(dirname \"$0\")/asked\"\necho '{\"format\": 1, \"written\": true}'\n",
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let asked = || fs::read_to_string(root.join("asked")).unwrap_or_default();
        // Not prepared yet: nothing to hold, nothing asked.
        write_data_file(&root, &resources).unwrap();
        assert_eq!(asked(), "");
        fs::write(resources.join(RESOURCES_MANIFEST), "{}").unwrap();
        write_data_file(&root, &resources).unwrap();
        let args: Vec<String> = asked().lines().map(str::to_owned).collect();
        assert_eq!(args[0], "data-file");
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "--output" && pair[1] == resources.to_string_lossy()),
            "{args:?}"
        );
        // One already there stays: the game judges whether it can use it.
        fs::write(resources.join(DATA_FILE), "kept").unwrap();
        fs::remove_file(root.join("asked")).unwrap();
        write_data_file(&root, &resources).unwrap();
        assert_eq!(asked(), "");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_new_version_reuses_resources_prepared_by_a_compatible_one() {
        let versions =
            std::env::temp_dir().join(format!("sunrise-launcher-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&versions);
        let oracle = "mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaHeightmapOracle.java";
        let camera = "mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCameraOracle.java";
        let collision =
            "mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCollisionOracle.java";
        let write = |tag: &str, path: &str, text: &str| {
            let path = versions.join(tag).join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        let release = |tag: &str, minecraft: &str| {
            write(tag, COMPLETE, "");
            let lock = serde_json::json!({"minecraft": {"version": minecraft, "client_sha1": "c", "server_sha1": "s"}});
            write(tag, "research-lock.json", &lock.to_string());
            write(tag, "tools/prepare_native_client.py", "schema 7");
            write(tag, oracle, "heightmap");
            write(tag, collision, "collision");
            write(tag, camera, tag); // Not an exporter: it may change freely.
        };
        let prepared = |tag: &str, minecraft: &str| {
            let manifest =
                serde_json::json!({"schema": 7, "collision_exporters_sha256": {collision: "x"}});
            write(
                tag,
                &format!(".local/sunrise-client/{minecraft}/{RESOURCES_MANIFEST}"),
                &manifest.to_string(),
            );
            write(
                tag,
                &format!(".local/sunrise-client/{minecraft}/worldgen/biomes.json"),
                "{}",
            );
        };
        let is_prepared = |tag: &str| prepared_at(&resources_at(&versions.join(tag)));

        release("v2026.10.13", "26.3");
        prepared("v2026.10.13", "26.3");
        release("v2026.10.12", "26.2");
        prepared("v2026.10.12", "26.2");
        // A data file names its own root: the new version writes its own.
        let data_file = crate::preparer::DATA_FILE;
        write(
            "v2026.10.13",
            &format!(".local/sunrise-client/26.3/{data_file}"),
            "written for v2026.10.13",
        );
        release("v2026.10.13.1", "26.3");
        assert_eq!(
            reuse_prepared(&versions, "v2026.10.13.1"),
            Some("v2026.10.13".into())
        );
        assert!(is_prepared("v2026.10.13.1"));
        assert!(resources_at(&versions.join("v2026.10.13.1"))
            .join("worldgen/biomes.json")
            .exists());
        assert!(!resources_at(&versions.join("v2026.10.13.1"))
            .join(data_file)
            .exists());
        assert!(resources_at(&versions.join("v2026.10.13"))
            .join(data_file)
            .exists());
        let left: Vec<_> = fs::read_dir(versions.join("v2026.10.13.1/.local/sunrise-client"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left, ["26.3"]);
        // Already prepared: nothing to do.
        assert_eq!(reuse_prepared(&versions, "v2026.10.13.1"), None);

        // Another Minecraft version, a changed exporter or a changed prepare script: prepare again.
        release("v2026.10.14", "26.4");
        assert_eq!(reuse_prepared(&versions, "v2026.10.14"), None);
        release("v2026.10.15", "26.3");
        write("v2026.10.15", collision, "collision, fixed");
        assert_eq!(reuse_prepared(&versions, "v2026.10.15"), None);
        release("v2026.10.16", "26.3");
        write("v2026.10.16", "tools/prepare_native_client.py", "schema 8");
        assert_eq!(reuse_prepared(&versions, "v2026.10.16"), None);
        assert!(!is_prepared("v2026.10.15") && !is_prepared("v2026.10.16"));

        // A version an older launcher installed without preparing it is caught up by the sweep
        // that starting the window, `versions` and `update` run; the incompatible ones stay as
        // they are, and a second sweep has nothing to do.
        release("v2026.10.17", "26.3");
        assert_eq!(
            reuse_prepared_in(&versions),
            [("v2026.10.17".to_string(), "v2026.10.13.1".to_string())]
        );
        assert!(is_prepared("v2026.10.17"));
        assert!(!is_prepared("v2026.10.15") && !is_prepared("v2026.10.16"));
        assert!(reuse_prepared_in(&versions).is_empty());

        // Versions with sunrise-prepare: its sources are the preparer, so the same sources
        // reuse each other's resources, and a change or a version without them prepares again.
        let crate_file = "crates/sunrise-prepare/src/lib.rs";
        release("v2026.10.18", "26.3");
        write("v2026.10.18", crate_file, "rust preparer");
        assert_eq!(reuse_prepared(&versions, "v2026.10.18"), None);
        prepared("v2026.10.18", "26.3");
        release("v2026.10.19", "26.3");
        write("v2026.10.19", crate_file, "rust preparer");
        assert_eq!(
            reuse_prepared(&versions, "v2026.10.19"),
            Some("v2026.10.18".into())
        );
        release("v2026.10.20", "26.3");
        write("v2026.10.20", crate_file, "rust preparer, changed");
        assert_eq!(reuse_prepared(&versions, "v2026.10.20"), None);
        let _ = fs::remove_dir_all(&versions);
    }

    #[test]
    fn prepared_resources_follow_the_preparers_output_version() {
        let versions = std::env::temp_dir().join(format!(
            "sunrise-launcher-output-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&versions);
        let write = |tag: &str, path: &str, text: &str| {
            let path = versions.join(tag).join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        let lib = "crates/sunrise-prepare/src/lib.rs";
        // A preparer with its sources (as releases ship them) and the exporters it runs.
        let release = |tag: &str, code: &str| {
            write(tag, COMPLETE, "");
            let lock = serde_json::json!({"minecraft": {"version": "26.3", "client_sha1": "c", "server_sha1": "s"}});
            write(tag, "research-lock.json", &lock.to_string());
            write(tag, "crates/sunrise-prepare/Cargo.toml", "[package]");
            write(tag, "crates/sunrise-prepare/build.rs", "fn main() {}");
            write(tag, lib, code);
            for name in LEGACY_EXPORTERS {
                write(
                    tag,
                    &format!("mods/sunrise/src/test/java/dev/sunriselab/parity/{name}"),
                    name,
                );
            }
        };
        let prepared = |tag: &str| {
            write(
                tag,
                &format!(".local/sunrise-client/26.3/{RESOURCES_MANIFEST}"),
                r#"{"schema": 7}"#,
            );
            write(tag, ".local/sunrise-client/26.3/facts.json", tag);
        };
        let facts = |tag: &str| {
            fs::read_to_string(resources_at(&versions.join(tag)).join("facts.json")).ok()
        };

        // The preparer's code changed, not what it makes: the resources are kept.
        release(
            "v0.3.0",
            "pub const OUTPUT_VERSION: u32 = 1;\nfn prepare() {}",
        );
        prepared("v0.3.0");
        release(
            "v0.3.1",
            "pub const OUTPUT_VERSION: u32 = 1;\nfn prepare() { faster() }",
        );
        assert_eq!(reuse_prepared(&versions, "v0.3.1"), Some("v0.3.0".into()));
        assert_eq!(facts("v0.3.1").as_deref(), Some("v0.3.0"));
        // Its output changed, so the number did: prepare again.
        release(
            "v0.3.2",
            "pub const OUTPUT_VERSION: u32 = 2;\nfn prepare() {}",
        );
        assert_eq!(reuse_prepared(&versions, "v0.3.2"), None);
        assert_eq!(facts("v0.3.2"), None);
        // Another Minecraft jar, same number: prepare again.
        release(
            "v0.3.3",
            "pub const OUTPUT_VERSION: u32 = 1;\nfn prepare() {}",
        );
        let lock = serde_json::json!({"minecraft": {"version": "26.3", "client_sha1": "other", "server_sha1": "s"}});
        write("v0.3.3", "research-lock.json", &lock.to_string());
        assert_eq!(reuse_prepared(&versions, "v0.3.3"), None);

        // A preparer from before the number is known by its fingerprint.
        release("v0.2.64", "fn prepare() {}");
        let fingerprint = legacy_fingerprint(&versions.join("v0.2.64")).unwrap();
        let legacy = [(fingerprint.as_str(), 1)];
        let at = |tag: &str| output_version_in(&versions.join(tag), &legacy);
        assert_eq!(at("v0.2.64"), Some(1));
        assert_eq!(at("v0.3.2"), Some(2));
        // Line endings as checked out don't count; an exporter's or the sources' change does.
        write(
            "v0.2.64",
            "mods/sunrise/src/test/java/dev/sunriselab/parity/ParityData.java",
            "ParityData.java\r",
        );
        assert_eq!(at("v0.2.64"), Some(1));
        write(
            "v0.2.64",
            "mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaCollisionOracle.java",
            "changed",
        );
        assert_eq!(at("v0.2.64"), None);
        release("v0.2.65", "fn prepare() { changed() }");
        assert_eq!(at("v0.2.65"), None);
        // A folder without the preparer's sources has no number.
        assert_eq!(output_version_in(&versions.join("missing"), &legacy), None);
        let _ = fs::remove_dir_all(&versions);
    }

    #[test]
    fn any_missing_extra_sends_the_store_refill_once_per_run() {
        use std::sync::atomic::AtomicBool;
        let lacks = |kinds: &[&str]| -> crate::preparer::Extras {
            kinds.iter().map(|k| (k.to_string(), "x".into())).collect()
        };
        for kinds in [
            &["fonts"][..],
            &["packs"],
            &["panorama"],
            &["sounds", "fonts"],
            &["languages"],
        ] {
            let asked = AtomicBool::new(false);
            assert!(
                refill_wanted(true, Some(&lacks(kinds)), &asked),
                "{kinds:?}"
            );
            // Asked once: the second call in the run says no.
            assert!(
                !refill_wanted(true, Some(&lacks(kinds)), &asked),
                "{kinds:?}"
            );
        }
        // Nothing missing, no answer from the preparer, or no asset index: not asked, and a
        // refusal does not use up the run's one ask.
        let asked = AtomicBool::new(false);
        assert!(!refill_wanted(true, Some(&lacks(&[])), &asked));
        assert!(!refill_wanted(true, None, &asked));
        assert!(!refill_wanted(false, Some(&lacks(&["fonts"])), &asked));
        assert!(refill_wanted(true, Some(&lacks(&["fonts"])), &asked));
    }

    #[test]
    fn prepared_versions_get_the_run_time_reports_without_preparing_again() {
        let versions = std::env::temp_dir().join(format!(
            "sunrise-launcher-reports-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&versions);
        let write = |tag: &str, path: &str, text: &str| {
            let path = versions.join(tag).join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        let release = |tag: &str, server: &str| {
            write(tag, COMPLETE, "");
            let lock = serde_json::json!({"minecraft": {"version": "26.3", "client_sha1": "c", "server_sha1": server}});
            write(tag, "research-lock.json", &lock.to_string());
            write(
                tag,
                &format!(".local/sunrise-client/26.3/{RESOURCES_MANIFEST}"),
                "{}",
            );
        };
        let resources = |tag: &str| resources_at(&versions.join(tag));
        let datagen = |tag: &str, name: &str| {
            write(tag, &format!(".local/datagen/out/reports/{name}"), tag);
        };

        // Prepared before the reports shipped with the resources: its own scratch output.
        release("v2026.10.13", "s");
        datagen("v2026.10.13", "registries.json");
        datagen("v2026.10.13", "commands.json");
        // Resources copied from it: the reports come from the version that ran the generator.
        release("v2026.10.14", "s");
        // Another server jar: the reports of the others would be wrong, and it has none.
        release("v2026.10.15", "other");
        assert!(reuse_prepared_in(&versions).is_empty());
        for tag in ["v2026.10.13", "v2026.10.14"] {
            for name in RUNTIME_REPORTS {
                assert_eq!(
                    fs::read_to_string(resources(tag).join(name)).unwrap(),
                    "v2026.10.13"
                );
            }
        }
        assert!(!resources("v2026.10.15").join("registries.json").exists());
        assert!(!complete_reports(&versions, "v2026.10.15"));
        // A copy that already holds the old shared temp name (another call mid-copy) does not
        // stop this call: each call copies to a name of its own.
        let squatted =
            resources("v2026.10.14").join(format!(".commands.json.partial-{}", std::process::id()));
        fs::remove_file(resources("v2026.10.14").join("commands.json")).unwrap();
        fs::create_dir(&squatted).unwrap();
        assert!(complete_reports(&versions, "v2026.10.14"));
        assert_eq!(
            fs::read_to_string(resources("v2026.10.14").join("commands.json")).unwrap(),
            "v2026.10.13"
        );
        fs::remove_dir(&squatted).unwrap();
        // Once there they stay as they are, even when the scratch output is gone.
        fs::remove_dir_all(versions.join("v2026.10.13/.local/datagen")).unwrap();
        assert!(complete_reports(&versions, "v2026.10.13"));
        let left: Vec<_> = fs::read_dir(resources("v2026.10.14"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().into_string().unwrap())
            .filter(|n| n.contains("partial"))
            .collect();
        assert!(left.is_empty());
        let _ = fs::remove_dir_all(&versions);
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "daily"]
    fn prepared_versions_get_the_extras_their_own_preparer_adds_once() {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("sunrise-launcher-extras-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let versions = dir.join("versions");
        // Each version's own preparer: what its resources lack, and what `extras` did with the folders it got.
        let version = |tag: &str, prepared: bool, preparer: &str| {
            let root = versions.join(tag);
            fs::create_dir_all(&root).unwrap();
            if prepared {
                let resources = resources_at(&root);
                fs::create_dir_all(&resources).unwrap();
                fs::write(resources.join(RESOURCES_MANIFEST), b"{}").unwrap();
            }
            let binary = root.join("sunrise-prepare");
            fs::write(&binary, format!("#!/bin/sh\n{preparer}\n")).unwrap();
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
            root
        };
        let lacking = r#"{"format": 1, "prepared": true, "missing": [{"kind": "packs", "what": "The built-in packs"}, {"kind": "sounds", "what": "The sounds"}]}"#;
        let added = r#"{"format": 1, "added": ["packs"], "failed": [{"kind": "sounds", "error": "no sound index"}], "missing": [{"kind": "sounds", "what": "The sounds"}]}"#;
        let preparer = format!(
            r#"case "$1" in
missing-extras) printf '%s\n' '{lacking}' ;;
extras) printf '%s\n' "$@" >> "$(dirname "$0")/extras-calls"; printf '%s\n' '{{"message": "Built-in packs ready"}}' '{added}'; exit 1 ;;
*) exit 2 ;;
esac"#
        );
        let lines = std::cell::RefCell::new(Vec::new());
        let log = |line: String| lines.borrow_mut().push(line);
        let asked = std::cell::Cell::new(0);
        let folders = || {
            asked.set(asked.get() + 1);
            vec![dir.join("official"), dir.join("prism")]
        };

        let root = version("v1", true, &preparer);
        complete_assets_in(&versions, "v1", folders, &log);
        complete_assets_in(&versions, "v1", folders, &log);
        assert_eq!(
            *lines.borrow(),
            [
                "Built-in packs ready",
                "v1: The sounds not added: no sound index"
            ]
        );
        assert_eq!(asked.get(), 1, "one try per run");
        let calls = fs::read_to_string(root.join("extras-calls")).unwrap();
        let args: Vec<&str> = calls.lines().collect();
        for wanted in [
            &["--only", "packs,sounds"][..],
            &["--minecraft-dir", dir.join("official").to_str().unwrap()],
            &["--minecraft-dir", dir.join("prism").to_str().unwrap()],
        ] {
            assert!(
                args.windows(2).any(|pair| pair == wanted),
                "{wanted:?} in {args:?}"
            );
        }

        // A preparer from before `missing-extras`, a version not prepared, and one that lacks nothing: no folder is asked for.
        lines.borrow_mut().clear();
        version("v0", true, "exit 2");
        version("v2", false, &preparer);
        version(
            "v3",
            true,
            r#"printf '%s\n' '{"format": 1, "prepared": true, "missing": []}'"#,
        );
        for tag in ["v0", "v2", "v3"] {
            complete_assets_in(&versions, tag, || unreachable!("no folder is needed"), &log);
        }
        assert!(lines.borrow().is_empty(), "{:?}", lines.borrow());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_the_app_bundle_around_the_binary() {
        let exe = Path::new("/Applications/Sunrise.app/Contents/MacOS/sunrise-launcher");
        assert_eq!(
            app_bundle(exe),
            Some(Path::new("/Applications/Sunrise.app"))
        );
        assert_eq!(app_bundle(Path::new("/Users/me/sunrise-launcher")), None);
        assert_eq!(
            app_bundle(Path::new("/x/Contents/MacOS/sunrise-launcher")),
            None
        );
        let work = std::env::temp_dir().join(format!("sunrise-find-app-{}", std::process::id()));
        let _ = fs::remove_dir_all(&work);
        fs::create_dir_all(work.join("dist/Sunrise.app/Contents/MacOS")).unwrap();
        assert_eq!(find_app(&work), Some(work.join("dist/Sunrise.app")));
        let _ = fs::remove_dir_all(&work);
    }

    /// The whole update of an app bundle, as the release ships it: signed, zipped with `ditto`,
    /// unpacked by the launcher, swapped in place of the installed one, still valid.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_app_bundle_updates_in_place() {
        use std::os::unix::fs::PermissionsExt;
        let work = std::env::temp_dir().join(format!("sunrise-bundle-{}", std::process::id()));
        let _ = fs::remove_dir_all(&work);
        let make = |dir: &Path, version: &str| {
            let app = dir.join("Sunrise.app");
            fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
            fs::create_dir_all(app.join("Contents/Resources")).unwrap();
            let bin = app.join("Contents/MacOS").join(crate::identity::EXECUTABLE);
            fs::copy("/usr/bin/true", &bin).unwrap();
            fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
            fs::write(app.join("Contents/Resources/version"), version).unwrap();
            fs::write(
                app.join("Contents/Info.plist"),
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
                 <key>CFBundleExecutable</key><string>sunrise-launcher</string>\
                 <key>CFBundleIdentifier</key><string>dev.sunrise.test</string>\
                 <key>CFBundlePackageType</key><string>APPL</string></dict></plist>\n"
                    .replace("sunrise-launcher", crate::identity::EXECUTABLE),
            )
            .unwrap();
            let signed = Command::new("codesign")
                .args(["--force", "--sign", "-"])
                .arg(&app)
                .status()
                .unwrap();
            assert!(signed.success());
            app
        };
        let installed = make(&work.join("Applications"), "1.0.0");
        let release = make(&work.join("release"), "2.0.0");
        let zip = work.join("release.zip");
        let zipped = Command::new("ditto")
            .args(["-c", "-k", "--norsrc", "--noextattr", "--keepParent"])
            .arg(&release)
            .arg(&zip)
            .status()
            .unwrap();
        assert!(zipped.success());
        unpack(&zip, "release.zip", &work.join("unpacked")).unwrap();
        let new = find_app(&work.join("unpacked")).unwrap();
        replace_bundle(&new, &installed).unwrap();
        let version = fs::read_to_string(installed.join("Contents/Resources/version")).unwrap();
        assert_eq!(version, "2.0.0");
        let bin = installed
            .join("Contents/MacOS")
            .join(crate::identity::EXECUTABLE);
        assert!(
            Command::new(&bin).status().unwrap().success(),
            "still executable"
        );
        let valid = Command::new("codesign")
            .args(["--verify", "--strict"])
            .arg(&installed)
            .status()
            .unwrap();
        assert!(valid.success(), "the updated bundle's signature holds");
        let left: Vec<_> = fs::read_dir(work.join("Applications"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(
            left,
            ["Sunrise.app"],
            "nothing staged or moved aside is left"
        );

        // A bundle changed after it was signed is refused, and the installed one stays.
        let tampered = make(&work.join("tampered"), "3.0.0");
        fs::write(tampered.join("Contents/Resources/version"), "3.0.1").unwrap();
        assert!(replace_bundle(&tampered, &installed).is_err());
        let version = fs::read_to_string(installed.join("Contents/Resources/version")).unwrap();
        assert_eq!(version, "2.0.0");
        let left = fs::read_dir(work.join("Applications")).unwrap().count();
        assert_eq!(left, 1, "nothing staged is left");
        let _ = fs::remove_dir_all(&work);
    }

    #[test]
    fn launcher_versions_read_from_the_binary() {
        assert_eq!(
            parse_version_output("sunrise-launcher 0.1.5\n"),
            Some("0.1.5")
        );
        assert_eq!(parse_version_output("something else"), None);
        assert_eq!(parse_version_output(""), None);
        // This test binary is not a launcher: no version, so an update goes ahead.
        assert_eq!(
            installed_launcher_version(&std::env::current_exe().unwrap()),
            None
        );
        assert_eq!(
            installed_launcher_version(Path::new("/nonexistent/sunrise-launcher")),
            None
        );
    }

    /// The launcher's file was replaced while it ran (an update from the command line, or an
    /// earlier one): the next replacement still lands at the path it started from.
    #[cfg(unix)]
    #[test]
    fn a_replaced_binary_is_replaced_again_at_its_path() {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("sunrise-launcher-replace-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("sunrise-launcher");
        fs::write(&exe, "old").unwrap();
        // Another update swaps the file under the running launcher.
        fs::write(dir.join("other"), "other update").unwrap();
        fs::rename(dir.join("other"), &exe).unwrap();
        fs::write(dir.join("new"), "new").unwrap();
        replace_binary(&dir.join("new"), &exe).unwrap();
        assert_eq!(fs::read_to_string(&exe).unwrap(), "new");
        // The file there keeps its mode, made executable (the swapped-in one was 0644).
        assert_eq!(
            fs::metadata(&exe).unwrap().permissions().mode() & 0o777,
            0o755
        );
        // Gone altogether: it is put back, executable.
        fs::remove_file(&exe).unwrap();
        replace_binary(&dir.join("new"), &exe).unwrap();
        assert_eq!(fs::read_to_string(&exe).unwrap(), "new");
        assert_eq!(
            fs::metadata(&exe).unwrap().permissions().mode() & 0o777,
            0o755
        );
        // No temporary file is left beside it.
        let mut names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["new", "sunrise-launcher"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn crash_reports_are_found_and_summarised() {
        let dir =
            std::env::temp_dir().join(format!("sunrise-launcher-crash-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let start = std::time::SystemTime::now() - std::time::Duration::from_secs(2);
        assert!(crash_report_in_since(&dir, start).is_none());
        fs::create_dir_all(&dir).unwrap();
        let report = "---- Sunrise Crash Report ----\n// Details below.\n\nTime: 2026-10-14 09:05:07\nDescription: Unexpected error\n\npanicked at src/game.rs:10:5:\nboom\n\t0: sunrise_client::game::frame\n";
        fs::write(dir.join("crash-2026-10-14_09.05.07-client.txt"), report).unwrap();
        fs::write(dir.join("notes.txt"), "x").unwrap();
        assert_eq!(
            crash_report_in_since(&dir, start).unwrap(),
            dir.join("crash-2026-10-14_09.05.07-client.txt")
        );
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
        assert!(crash_report_in_since(&dir, later).is_none());
        assert_eq!(
            crash_summary(report).as_deref(),
            Some("panicked at src/game.rs:10:5: boom")
        );
        assert_eq!(crash_summary("no report"), None);
        let log = dir.join("latest.log");
        fs::write(&log, "a\nb\nc\n").unwrap();
        assert_eq!(tail_of(&log, 2).unwrap(), "b\nc\n");
        assert_eq!(tail_of(&log, 100).unwrap(), "a\nb\nc\n");
        assert!(tail_of(&dir.join("missing.log"), 5).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn plain_notes() {
        let text = markdown_to_text(
            "# Title\n\nSome *words* and [a link](https://x.y).\n\n- one\n- two\n",
        );
        assert_eq!(
            text,
            "Title\n\nSome words and a link (https://x.y).\n\n- one\n- two\n"
        );
    }

    #[test]
    fn the_game_does_not_inherit_the_launchers_renderer_workarounds() {
        let mut cmd = Command::new("true");
        cmd.env("GALLIUM_DRIVER", "llvmpipe")
            .env("LIBGL_ALWAYS_SOFTWARE", "1")
            .env("WEBKIT_DISABLE_DMABUF_RENDERER", "1")
            .env("WEBKIT_DISABLE_COMPOSITING_MODE", "1")
            .env("PIPEWIRE_PROPS", "kept");
        strip_launcher_render_env(&mut cmd);
        let envs = cmd.get_envs().collect::<std::collections::HashMap<_, _>>();
        for key in LAUNCHER_RENDER_ENV {
            assert_eq!(envs.get(OsStr::new(key)), Some(&None), "{key} is removed");
        }
        // Only the launcher's own workarounds go: the rest of the environment still reaches the game.
        assert_eq!(
            envs.get(OsStr::new("PIPEWIRE_PROPS")),
            Some(&Some(OsStr::new("kept")))
        );
    }
}

mod messages;
pub use messages::Msg;
