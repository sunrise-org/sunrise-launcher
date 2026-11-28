//! Everything that is not UI: GitHub, installs, resource preparation, the game's processes and
//! updates. The GUI, the CLI and the MCP server all go through this module. Functions here
//! block; the GUI runs them on threads and reads their messages.
use crate::backend::lock::recover;
use crate::process::child::Child;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

/// Where the game's releases, Nightlies, Dev builds and mod SDKs are published, and its bugs reported.
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

mod architecture;
mod game_dirs;
mod http;
mod install;
mod location;
pub mod lock;
mod look_settings;
mod play_commands;
mod play_log;
mod player_settings;
mod prepare;
mod release_assets;
mod release_notes;
mod releases;
mod runtime;
mod self_update;
mod settings_file;
pub use architecture::*;
pub use game_dirs::*;
pub use http::*;
pub use install::*;
pub(crate) use location::{admit_build_dir, plain, relocation_build_dir};
pub use location::{build_dir, data_dir};
pub use play_commands::*;
pub use player_settings::*;
pub use prepare::*;
pub use release_assets::PLATFORMS;
pub use release_notes::{markdown_to_text, notes_summary};
pub use releases::*;
pub use runtime::*;
pub use self_update::*;
pub use settings_file::{
    allow_native_files, allow_native_files_in, allowed_native_files, allowed_native_files_in,
};

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

pub use sunrise_launcher_domain::{
    compare_game_versions, dev_date, is_game_tag, stable_version, Release,
};

pub use crate::signed::LauncherCheckError;

/// The community and website links, from `links.json` on main, so they can change
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
    crate::proxy::agent("links", &url, |builder| {
        builder
            .user_agent(&format!("sunrise-launcher/{VERSION}"))
            .timeout_connect(std::time::Duration::from_secs(15))
            .timeout_read(std::time::Duration::from_secs(60))
            .redirects(0)
    })
    .ok()
    .and_then(|agent| agent.get(&url).call().ok())
    .filter(|r| r.status() == 200)
    .and_then(|r| r.into_string().ok())
    .map_or_else(Links::default, |text| Links::parse(&text))
}

pub use installation::{InstallOutcome, InstallPhase, NightlyReceiptWarning};

mod developer_play;
pub use developer_play::client_as;

/// Downloads `release`'s archive for this platform, checks it against `sha256` (its signed
/// checksum), and unpacks it as the installed version `tag`.
pub(crate) mod installation;
pub use installation::recover_game_installations;

/// The Minecraft version a Sunrise build targets, from its research-lock.json.
pub fn game_version(tag: &str) -> String {
    game_version_at(&version_dir(tag))
}

/// Hosting uses the locked version as a destination. Older builds with no lock keep the default.
pub(crate) fn hosting_game_version(folder: &Path) -> Result<String, String> {
    let path = folder.join("research-lock.json");
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .ok()
            .and_then(|lock| crate::sources::Pins::version_in(&lock).map(str::to_owned))
            .ok_or_else(invalid_hosting_game_version),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(PINNED_GAME.into()),
        Err(_) => Err(invalid_hosting_game_version()),
    }
}

fn invalid_hosting_game_version() -> String {
    crate::i18n::tr("Invalid Minecraft version in research-lock.json.").into()
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn game_version_at(dir: &Path) -> String {
    locked_game(dir).unwrap_or_else(|| PINNED_GAME.into())
}

fn locked_game(dir: &Path) -> Option<String> {
    crate::sources::Pins::version_in(&read_json(&dir.join("research-lock.json"))?).map(String::from)
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

mod dedicated;

pub(crate) use dedicated::root as server_root;

pub mod version_storage;
pub use version_storage::remove_version;

mod release_selection;
pub use release_selection::build_revision;
pub(crate) use release_selection::WorldBuildOrder;

/// One release's notes: the tag given, or the newest.
pub fn release_notes(tag: Option<&str>) -> Result<Release, String> {
    let releases = releases()?;
    match tag {
        Some(tag) => release_selection::by_tag(&releases, tag)
            .cloned()
            .ok_or_else(|| {
                crate::i18n::trf("No Sunrise release is tagged {tag}.", &[("tag", tag)])
            }),
        None => releases
            .into_iter()
            .find(|release| {
                crate::instances::Channel::of_release(release)
                    != Some(crate::instances::Channel::Dev)
            })
            .ok_or_else(|| crate::i18n::tr("No release is published yet.").into()),
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
    pub fn start(job: Job, cmd: Command) -> Result<Self, String> {
        Self::start_with(|tx| spawn(job, cmd, tx))
    }

    pub fn start_server(tag: &str, folder: &Path, cmd: Command) -> Result<Self, String> {
        let (cmd, runtime) = prepare_server_command(tag, folder, cmd)?;
        Self::start_with(|tx| spawn_prepared(Job::Server, cmd, runtime, tx, None, None))
    }

    fn start_with(
        start: impl FnOnce(Sender<Msg>) -> Result<Running, String>,
    ) -> Result<Self, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        let running = start(tx)?;
        Ok(Self::keep(running, rx, Default::default()))
    }

    pub fn monitor_game(
        child: Child,
        runtime: RuntimeGuard,
        prelaunch_log: std::collections::VecDeque<String>,
    ) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        Self::keep(monitor_game(child, tx, runtime), rx, prelaunch_log)
    }

    fn keep(
        running: Running,
        rx: std::sync::mpsc::Receiver<Msg>,
        mut initial_log: std::collections::VecDeque<String>,
    ) -> Self {
        let excess = initial_log.len().saturating_sub(LOG_KEEP);
        initial_log.drain(..excess);
        let log = Arc::new(Mutex::new(initial_log));
        let exit = Arc::new(Mutex::new(None));
        let (l, e) = (log.clone(), exit.clone());
        std::thread::spawn(move || {
            for msg in rx {
                match msg {
                    Msg::Line(_, line) => {
                        let mut log = recover(&l);
                        log.push_back(line);
                        if log.len() > LOG_KEEP {
                            log.pop_front();
                        }
                    }
                    Msg::Exit(_, result) => *recover(&e) = Some(result),
                    Msg::ClientObservationLost(error) => *recover(&e) = Some(Err(error)),
                    _ => {}
                }
            }
        });
        Self { running, log, exit }
    }

    pub fn finished(&self) -> Option<Result<(), String>> {
        recover(&self.exit).clone()
    }

    pub fn tail(&self, lines: usize) -> Vec<String> {
        let log = recover(&self.log);
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

mod update_selection;
pub use update_selection::select_after_update;

/// Installs `tag` (or with "latest", the profile's latest release), then updates the selection as
/// `select_after_update` says when `as_update`, or always selects it otherwise.
pub fn install_tag(tag: &str, as_update: bool, progress: impl Fn(f32)) -> Result<String, String> {
    let download = |release: &Release| install(release, progress);
    if let Some(dev) = release_selection::dev(tag)? {
        // The exact tag is sufficient to install, but not to decide which builds other profiles use.
        let drafts = crate::draft_releases::folder()?;
        return install_selected_release(&dev, None, as_update, |release| {
            let outcome = download(release)?;
            if let (Some(folder), InstallOutcome::Installed(_)) = (&drafts, &outcome) {
                crate::draft_releases::mark(&release.tag, folder)?;
            }
            Ok(outcome)
        });
    }
    if tag == "latest" {
        return install_offered_tag(&releases()?, tag, as_update, download);
    }
    let list = release_list()?;
    install_explicit_tag_from(
        &list,
        tag,
        as_update,
        &Settings::load(),
        &crate::native_mods::sdk_cache::Base::from_env(),
        &sunrise_launcher_release_verify::trusted_keys_for(GAME_REPO),
        download,
    )
}

fn install_explicit_tag_from(
    list: &[serde_json::Value],
    tag: &str,
    as_update: bool,
    settings: &Settings,
    base: &crate::native_mods::sdk_cache::Base,
    keys: &[sunrise_launcher_release_verify::PublicKey],
    install: impl FnOnce(&Release) -> Result<InstallOutcome, String>,
) -> Result<String, String> {
    let (candidates, target) = game_releases_candidates(list, settings);
    let mut release = release_selection::by_tag(&candidates, tag)
        .cloned()
        .ok_or_else(|| crate::i18n::trf("No Sunrise release is tagged {tag}.", &[("tag", tag)]))?;
    if let Some(target) = target {
        check_sdk_offer_strict(&mut release, target, base, keys)?;
    }
    let has_unresolved_sdk = target.is_some()
        && candidates
            .iter()
            .any(|r| r.tag != release.tag && is_sdk_candidate(r));
    let catalog = (!has_unresolved_sdk).then_some(candidates.as_slice());
    install_selected_release(&release, catalog, as_update, install)
}

fn install_offered_tag(
    releases: &[Release],
    tag: &str,
    as_update: bool,
    install: impl FnOnce(&Release) -> Result<InstallOutcome, String>,
) -> Result<String, String> {
    let release = if tag == "latest" {
        crate::instances::selected()
            .channel
            .following(releases)
            .builds(releases)
            .next()
            .ok_or_else(|| {
                crate::i18n::tr("No release has a build for this platform yet.").to_string()
            })?
    } else {
        release_selection::by_tag(releases, tag).ok_or_else(|| {
            crate::i18n::trf("No Sunrise release is tagged {tag}.", &[("tag", tag)])
        })?
    };
    install_selected_release(release, Some(releases), as_update, install)
}

fn install_selected_release(
    release: &Release,
    catalog: Option<&[Release]>,
    as_update: bool,
    install: impl FnOnce(&Release) -> Result<InstallOutcome, String>,
) -> Result<String, String> {
    if release.asset.is_none() {
        return Err(crate::i18n::trf(
            "{tag} has no build for this platform.",
            &[("tag", &release.tag)],
        ));
    }
    let before = installed();
    let outcome = install(release)?;
    if let Some(warning) = outcome.receipt_warning() {
        eprintln!("{warning}");
    }
    if matches!(outcome, InstallOutcome::ReceiptOnly(_)) {
        return Ok(release.tag.clone());
    }
    let mut settings = Settings::load();
    let select =
        !as_update || select_after_update(settings.selected.as_deref(), &before, &release.tag);
    if select {
        settings.selected = Some(release.tag.clone());
        settings.save()?;
    }
    if let Some(releases) = catalog {
        for (tag, error) in version_storage::prune_old_versions(releases, &release.tag) {
            eprintln!("Could not remove old game build {tag}: {error}");
        }
    }
    Ok(release.tag.clone())
}

pub use crate::run_leases::control::startup_cleanup;

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
mod connection_reuse_tests;

#[cfg(test)]
mod tests;

mod messages;
pub use messages::Msg;
