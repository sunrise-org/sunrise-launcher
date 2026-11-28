//! `sunrise-launcher <subcommand>`: the same steps as the window, from a terminal. Every
//! command goes through `backend`; this file only parses arguments and prints.
mod progress;
mod switch;
use progress::{native_progress, progress};
mod toolchains;
use toolchains::{java, rust, JavaCmd, RustCmd};
mod update;
use update::update;
mod accounts;
use accounts::{account_command, AccountCmd};
mod data;
use data::{data, DataCmd};
mod settings;
#[cfg(test)]
use settings::set_setting;
use settings::{on_off, SETTING_KEYS};
mod servers;
use servers::{multiplayer, servers, MultiplayerCmd, ServersCmd};
mod mods;
use mods::{approve_mod_change, mods, ModsCmd};
mod instance;
use instance::{after_exit, instance, modpack, InstanceCmd, ModpackCmd};
mod content;
use content::{game_packs, screenshots, worlds, PacksCmd, ScreenshotsCmd, WorldsCmd};

mod about;
mod builds;
mod logs;
mod native;
mod play;
mod storage;
use clap::{Parser, Subcommand};
use sunrise_launcher::account::{self, Accounts};
use sunrise_launcher::backend::{self, Release};
use sunrise_launcher::github_mods;
use sunrise_launcher::i18n::{tr, trf};
use sunrise_launcher::instances;
use sunrise_launcher::native_mods;
use sunrise_launcher::server_list;
use sunrise_launcher::storage::{Category, Storage};
#[derive(Clone, Debug)]
enum ClearStorage {
    All,
    Row(Category),
}
fn parse_storage_row(value: &str) -> Result<ClearStorage, String> {
    use clap::ValueEnum;
    if value == "all" {
        Ok(ClearStorage::All)
    } else {
        Category::ALL
            .into_iter()
            .find(|category| category.to_possible_value().unwrap().get_name() == value)
            .map(ClearStorage::Row)
            .ok_or_else(|| tr("Unknown").into())
    }
}
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;
#[derive(Parser)]
#[command(
    name = crate::identity::EXECUTABLE,
    version = backend::VERSION,
    about = "Installs, prepares and starts Sunrise. Without a subcommand, opens the window."
)]
struct Cli {
    /// Print JSON instead of text, where a command has something to report
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Option<Cmd>,
}

/// The `--profile` value. [`native_mods::Profile`] lives in the launcher's pure type crate, which
/// stays free of clap, so the command line parses its own one-variant name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum ProfileArg {
    Optimized,
}

impl From<ProfileArg> for native_mods::Profile {
    fn from(_: ProfileArg) -> Self {
        Self::Optimized
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Write a shell completion script to stdout
    Completions { shell: clap_complete::Shell },
    /// List Sunrise releases, or the installed versions
    Versions {
        /// Only the versions installed here (no network)
        #[arg(long)]
        installed: bool,
    },
    /// Download and install a Sunrise build
    Install {
        /// A build tag (e.g. 26.3-0.1.0, nightly, or dev-2026-11-21-0c02067), or "latest" for the profile's channel
        tag: String,
    },
    /// Install a local unsigned developer build from a trusted checkout
    #[command(about = tr("Install a local unsigned developer build from a trusted checkout"), after_help = tr("Example: sunrise-launcher developer-install CHECKOUT --program CHECKOUT/target/debug/sunrise-client --trust-unsigned\nThis executes the checkout's build code and an unsigned game with your rights. Trust both before continuing."), long_about = tr("Install a local unsigned developer build. This runs the checkout's build code through cargo run -p xtask -- package. Commit tracked changes and rebuild first. Build sunrise-ctl, sunrise-prepare and sunrise-server beside the game, and compile exporters into the checkout's exporters folder. Native builds use the saved local source commit, including unpublished commits. Select this base explicitly; automatic updates never replace it."))]
    DeveloperInstall(developer_builds::DeveloperInstall),
    /// Remove an installed version
    Remove { tag: String },
    /// Install a game program you have, such as a sunrise-client built on your fork's Actions,
    /// as its own version on an installed base's game files, and select it. A local unsigned
    /// developer base is allowed only when explicitly selected or named with --version.
    /// Nothing signed it: it runs with your rights, and updates never replace it
    #[command(
        after_help = "The program must match this computer's executable format and carry the exact `sunrise-game-executable;`\nidentity signature embedded by ordinary and launcher-generated games. It carries no mod API level or build id,\nand is not a signature of authenticity. Unmarked programs are refused."
    )]
    ImportBuild {
        /// The program (sunrise-client, sunrise-client.exe on Windows), or a zip holding it
        path: PathBuf,
        /// The installed base whose game files it plays with (default: the selected usable
        /// base, else the newest release or Nightly; developer bases need explicit selection)
        #[arg(long)]
        version: Option<String>,
        /// You picked this build yourself and trust it with full access to this computer
        #[arg(long)]
        trust_unsigned: bool,
    },
    /// Prepare a version's resources from your own Minecraft install, or from Mojang
    Prepare {
        /// The game folder of the official launcher, Prism Launcher, MultiMC, the Modrinth App,
        /// ATLauncher or GDLauncher (default: the launcher's, then every launcher's default)
        #[arg(long, conflicts_with = "download")]
        minecraft: Option<PathBuf>,
        /// Download Minecraft from Mojang for the signed-in account that owns it, instead of
        /// using an install
        #[arg(long)]
        download: bool,
        /// The installed version (default: the selected one)
        #[arg(long)]
        version: Option<String>,
    },
    /// Start the client (needs a signed-in Microsoft account that owns Minecraft: Java Edition)
    Play(play::PlayArgs),
    /// Switch a profile to an exact installed and prepared game build
    Switch {
        /// The target build tag
        tag: String,
        /// The profile to switch (name or id; defaults to the selected profile)
        #[arg(long)]
        profile: Option<String>,
    },
    /// Run a dedicated server here; type commands into its console
    Server {
        /// Allow missing original Java asset/data jars to be downloaded from Modrinth
        #[arg(long)]
        allow_jar_downloads: bool,
        /// The server's folder (default: the one the launcher uses)
        #[arg(long)]
        dir: Option<PathBuf>,
        #[arg(long)]
        version: Option<String>,
        /// Agree to the Minecraft EULA (https://aka.ms/MinecraftEULA)
        #[arg(long)]
        accept_eula: bool,
    },
    /// Show disposable storage by category as JSON, or clear a category by hand with `--clear`.
    /// Read-only reports never block builds; `--clear` requires builds to be idle.
    ///
    /// Rust/Cargo/temp build-tool caches remain clearable. Game-accessible caches and installed-version
    /// removal retain files while any launch path lacks enforceable lifetime exclusion; an empty
    /// process scan is not authorization. Read-only reports stay available.
    /// Private launch records and raw captures are collected together only after all participating owners
    /// exit, under the exclusive permanent namespace gate. Published unit keys persist until exclusive
    /// unit removal. Registry entry counts are logged; lock entries are not age-collected.
    Storage {
        /// Request cleanup of build-files, prebuilt-games, spare-game-builds, rust-compiler, other-caches or all rows; game-accessible caches refuse cleanup without enforceable lifetime exclusion
        #[arg(long, value_parser = parse_storage_row)]
        clear: Option<ClearStorage>,
    },
    /// Generate the registry-only Cargo.lock for one native mod, without executing mod code
    NativeLock {
        folder: PathBuf,
        /// Explicitly replace an existing lock with newly resolved registry pins
        #[arg(long)]
        update_lock: bool,
    },
    /// Native mods (docs/modding/src/how-to/native-mods.md): compile flat Rust mod projects against the profile's exact
    /// Sunrise source revision with the launcher's own Rust, as the version the profile then
    /// plays. Not sandboxed: the mods' code runs with your rights. If there are no native mods to build, it exits successfully; a
    /// server export still fails, as no engine is made. Reclaim disk by hand with `storage --clear all`
    /// or Settings → Storage
    NativeBuild(native::NativeBuildArgs),
    /// Show a release's patch notes (default: the newest)
    Notes { tag: Option<String> },
    /// Show the website, roadmap and documentation links
    Links,
    /// Microsoft accounts: list them (the default), sign in, sign out, or pick the one that plays
    Account {
        #[command(subcommand)]
        action: Option<AccountCmd>,
    },
    /// Show the launcher's settings, or set one (the Settings page): selected-version TAG,
    /// game-architecture auto|x64|arm64,
    /// minecraft-dir PATH, server-dir PATH ("default" resets a folder),
    /// prune-old-versions on|off (delete unused old automatic downloads after an install, keeping
    /// the newest few), keep-versions N (how many of the
    /// newest to keep; "default" is 2), opening-animation on|off, cat on|off (the corner cat), proxy http://host:port|socks5://host:port
    /// ("default" removes it), while-playing stay|minimize|hide (what the window does while the game runs), language
    /// system|CODE (the window's language: en, fr, de, ja, pt-br... any language the launcher has;
    /// system follows the computer's; a wrong code lists them all), theme default|ID|local:SLUG
    /// (a built-in theme, or one of yours from the themes folder; "default" is Dawn; a wrong id
    /// lists them all), accent
    /// violet|blue|teal|green|amber|red|pink|#rrggbb|DEGREES (a named accent, any colour, or any
    /// hue; "theme" goes back to the theme's own), background PATH (your picture in the Home banner;
    /// "default" goes back to the theme's), scenery on|off (whether the Home banner shows its
    /// scene or picture), reduced-graphics on|off (a still scene and no decorative motion),
    /// header-align left|right|center-panel|center-window (where the Home header stands in its banner)
    #[command(after_help = tr("worlds rows|tiles: how Home lists the worlds under Continue playing."))]
    Settings {
        #[arg(value_parser = SETTING_KEYS)]
        key: Option<String>,
        /// The new value (with KEY)
        #[arg(requires = "key")]
        value: Option<String>,
    },
    /// Show or set the extra arguments the game starts with (for example --server-thread);
    /// quotes group a word with spaces in it
    GameArgs {
        /// Omit to show the current arguments
        #[arg(allow_hyphen_values = true)]
        args: Option<String>,
    },
    /// Upload a profile's latest.log to mclo.gs (a paste service for Minecraft logs) and print
    /// its link, for a bug report: only when the player asks. The home folder, the user name and
    /// the account names are taken out first; the link expires after 90 days unread
    ShareLog {
        /// The profile (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
    },
    /// Delete a log shared with share-log: the "<id>:<token>" it printed
    UnshareLog { shared: String },
    /// Print the end of the game's log (logs/latest.log in the game folder), or its newest crash
    /// report
    Logs {
        /// How many lines of latest.log
        #[arg(long, default_value_t = 100)]
        lines: usize,
        /// The newest crash report instead
        #[arg(long)]
        crash: bool,
    },
    /// Look for a newer Sunrise release and launcher, and install them
    Update {
        /// Only report what is available
        #[arg(long)]
        check: bool,
    },
    /// Linux: put Sunrise in the applications menu (an entry and the icon, under
    /// $XDG_DATA_HOME), or take it out with --remove
    DesktopEntry {
        /// Remove the entry and the icon, and stop adding them back
        #[arg(long)]
        remove: bool,
    },
    /// Write a bug report to a file you attach to the issue by hand; nothing is sent
    /// (docs/telemetry.md)
    BugReport {
        text: String,
        /// Add the end of the game's log (names and worlds hidden, without chat)
        #[arg(long)]
        log: bool,
    },
    /// Opt-in data collection (docs/telemetry.md): what is allowed, what would be sent, and
    /// deleting it; all off until you allow a kind
    Data {
        #[command(subcommand)]
        action: Option<DataCmd>,
    },
    /// Profiles: separate game folders, each with its own worlds, options and packs
    #[command(visible_alias = "profile")]
    Instance {
        #[command(subcommand)]
        action: Option<InstanceCmd>,
    },
    /// A profile's single-player worlds: list them, back one up, copy or delete it
    Worlds {
        #[command(subcommand)]
        action: Option<WorldsCmd>,
    },
    /// A profile's resource and shader packs: list, add, turn on or off, remove
    Packs {
        #[command(subcommand)]
        action: Option<PacksCmd>,
    },
    /// Java for Minecraft: Java Edition profiles: list the runtimes, download Mojang's
    Java {
        #[command(subcommand)]
        action: Option<JavaCmd>,
    },
    /// The screenshots F2 took in a profile: list, delete
    Screenshots {
        #[command(subcommand)]
        action: Option<ScreenshotsCmd>,
    },
    /// The servers in a profile's Multiplayer list (servers.dat): list, add, remove
    Multiplayer {
        #[command(subcommand)]
        action: Option<MultiplayerCmd>,
    },
    /// Sunrise modpacks from the registry: list installed packs
    Modpack {
        #[command(subcommand)]
        action: Option<ModpackCmd>,
    },
    /// The mod browser: Java mods, resource packs and modpacks from Modrinth
    /// (CurseForge support is coming soon), each file checked against its checksum
    /// (docs/mod-browser.md)
    Mods {
        #[command(subcommand)]
        action: Option<ModsCmd>,
    },
    /// The portable Rust toolchain native mods compile with (docs/modding/src/how-to/native-mods.md): the launcher's
    /// own, in its data folder, never installed on the system
    Rust {
        #[command(subcommand)]
        action: Option<RustCmd>,
    },
    /// Servers hosted with sunrise-server (the Servers page): list them, create one, or run any
    /// sunrise-server command on one
    Servers {
        #[command(subcommand)]
        action: Option<ServersCmd>,
    },
    /// Serve the Model Context Protocol on stdin and stdout, for AI agents (refused while No AI
    /// is on)
    Mcp,
    /// Show or set No AI: on hides and turns off every AI feature (the companion, agent control,
    /// sunrise-ctl and both MCP servers); off, the default, makes them available
    NoAi {
        /// Omit to show the current state
        #[arg(value_parser = ["on", "off"])]
        state: Option<String>,
    },
    /// Measure the launcher: starts (the first is the cold one), page switching, idle CPU and
    /// memory, the release list fetch; writes a benchmark report to benchmarks/ in its data folder
    Benchmark {
        /// How many times to start it
        #[arg(long, default_value_t = 5)]
        repeats: usize,
    },
}

/// The CLI's definition, for the MCP tools generated from it (cli_tools.rs).
pub fn command() -> clap::Command {
    <Cli as clap::CommandFactory>::command()
}

/// The commands that exist for AI agents: hidden and refused while No AI is on. Every other
/// command stays: the launcher, packagers and scripts need them, and none drives the game.
pub const AI_COMMANDS: [&str; 1] = ["mcp"];

/// Runs a subcommand and returns the exit code, or None when the window should open.
pub fn run() -> Option<i32> {
    use clap::FromArgMatches;
    let no_ai = backend::no_ai();
    let mut definition = command();
    if no_ai {
        // Hidden from the help too; a call still gets the refusal below.
        for name in AI_COMMANDS {
            definition = definition.mut_subcommand(name, |c| c.hide(true));
        }
    }
    let cli = Cli::from_arg_matches(&definition.get_matches()).unwrap_or_else(|e| e.exit());
    let command = cli.command?;
    let result = match command {
        Cmd::Mcp if no_ai => Err(sunrise_launcher::no_ai::REFUSAL.into()),
        Cmd::Mcp => crate::mcp::serve().map(|()| 0).map_err(Error::from),
        command => {
            let result = execute(command, cli.json);
            // A game it started has ended (attached) or none did (an error): a session file it
            // didn't read goes now.
            account::discard_unread_sessions();
            result
        }
    };
    Some(match result {
        Ok(code) => code,
        Err(e) => {
            // An error may name a file or folder as it was found: shown as text, never obeyed.
            eprintln!(
                "error: {}",
                sunrise_launcher::theme_files::for_terminal(&e.to_string())
            );
            1
        }
    })
}

/// Why a command failed: the message the CLI prints after `error: `, exactly as it was written.
#[derive(Debug)]
struct Error(String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Anything the commands' own code fails with turns into its message, as it did into a `String`.
impl<E: Into<String>> From<E> for Error {
    fn from(message: E) -> Self {
        Self(message.into())
    }
}

fn json(value: serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
}

/// Runs the fixture process in this terminal and reports how it ended.
#[cfg(test)]
fn attached(cmd: Command) -> Result<i32, String> {
    let (cmd, _runtime) = backend::prepare_command(cmd)?;
    attached_prepared(cmd)
}

fn attached_prepared(mut cmd: Command) -> Result<i32, String> {
    let status = backend::spawn_attached(&mut cmd)
        .and_then(|mut child| child.wait())
        .map_err(|e| format!("Could not start {:?}: {e}", cmd.get_program()))?;
    attached_status(status)
}

fn attached_status(status: std::process::ExitStatus) -> Result<i32, String> {
    #[cfg(unix)]
    {
        sunrise_launcher::process::supervisor::exit_code(status).map_err(|error| error.to_string())
    }
    #[cfg(not(unix))]
    {
        status.code().ok_or_else(|| format!("Exited with {status}"))
    }
}

fn release_json(r: &Release) -> serde_json::Value {
    serde_json::json!({
        "tag": r.tag, "name": r.name, "date": r.date, "prerelease": r.prerelease,
        "available_here": r.asset.is_some(), "installed": backend::installed().contains(&r.tag),
    })
}

fn execute(command: Cmd, as_json: bool) -> Result<i32, Error> {
    builds::reuse_prepared(&command, as_json);
    match command {
        Cmd::Completions { shell } => about::completions(shell),
        Cmd::Versions { installed: true } => builds::installed(as_json)?,
        Cmd::Versions { installed: false } => return builds::releases(as_json),
        Cmd::Install { tag } => builds::install(tag, as_json)?,
        Cmd::Storage { clear } => storage::report(clear)?,
        Cmd::NativeLock {
            folder,
            update_lock,
        } => native_mods::registry_lock(&folder, update_lock)?,
        Cmd::NativeBuild(args) => return native::build(args),
        Cmd::DeveloperInstall(args) => developer_builds::install(args, as_json)?,
        Cmd::Remove { tag } => builds::remove(tag)?,
        Cmd::ImportBuild {
            path,
            version,
            trust_unsigned,
        } => developer_builds::import(&path, version.as_deref(), trust_unsigned, as_json)?,
        Cmd::Prepare {
            minecraft,
            download,
            version,
        } => return builds::prepare(minecraft, download, version),
        Cmd::Play(args) => return play::client(args),
        Cmd::Switch { tag, profile } => switch::profile(tag, profile, as_json)?,
        Cmd::Server {
            dir,
            version,
            accept_eula,
            allow_jar_downloads,
        } => return play::server(dir, version, accept_eula, allow_jar_downloads),
        Cmd::Notes { tag } => builds::notes(tag, as_json)?,
        Cmd::Account { action } => account_command(action.unwrap_or(AccountCmd::List), as_json)?,
        Cmd::Data { action } => data(action.unwrap_or(DataCmd::Status), as_json)?,
        Cmd::Benchmark { repeats } => about::benchmark(repeats, as_json)?,
        Cmd::Links => about::links(as_json),
        Cmd::NoAi { state } => settings::no_ai(state, as_json)?,
        Cmd::Settings { key, value } => settings::show(key, value, as_json)?,
        Cmd::GameArgs { args } => settings::game_args(args, as_json)?,
        Cmd::ShareLog { instance } => logs::share(instance, as_json)?,
        Cmd::UnshareLog { shared } => logs::unshare(shared)?,
        Cmd::Logs { lines, crash } => logs::show(lines, crash, as_json)?,
        Cmd::Update { check } => update(check, as_json)?,
        Cmd::BugReport { text, log } => logs::bug_report(text, log, as_json)?,
        Cmd::DesktopEntry { remove } => settings::desktop_entry(remove)?,
        Cmd::Worlds { action } => worlds(
            action.unwrap_or(WorldsCmd::List { instance: None }),
            as_json,
        )?,
        Cmd::Packs { action } => game_packs(
            action.unwrap_or(PacksCmd::List {
                kind: None,
                instance: None,
            }),
            as_json,
        )?,
        Cmd::Screenshots { action } => screenshots(
            action.unwrap_or(ScreenshotsCmd::List { instance: None }),
            as_json,
        )?,
        Cmd::Multiplayer { action } => multiplayer(
            action.unwrap_or(MultiplayerCmd::List { instance: None }),
            as_json,
        )?,
        Cmd::Instance { action } => instance(action.unwrap_or(InstanceCmd::List), as_json)?,
        Cmd::Modpack { action } => return modpack(action.unwrap_or(ModpackCmd::List), as_json),
        Cmd::Mods { action } => mods(action.unwrap_or(ModsCmd::List { instance: None }), as_json)?,
        Cmd::Servers { action } => servers(action.unwrap_or(ServersCmd::List), as_json)?,
        Cmd::Rust { action } => rust(action.unwrap_or(RustCmd::Status), as_json)?,
        Cmd::Java { action } => java(action.unwrap_or(JavaCmd::List), as_json)?,
        Cmd::Mcp => unreachable!(),
    }
    Ok(0)
}

#[cfg(test)]
mod server_tests;
#[cfg(test)]
mod storage_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod catalog_tests;

mod native_permission_impl;
use native_permission_impl::{
    github_native, native_permission, trust_unreviewed_builds, GithubInstall,
};

mod developer_builds;

mod settings_output;
use settings_output::settings_json;
