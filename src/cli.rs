//! `sunrise-launcher <subcommand>`: the same steps as the window, from a terminal. Every
//! command goes through `backend`; this file only parses arguments and prints.
mod progress;
use progress::{native_progress, progress};

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
#[derive(Subcommand)]
enum JavaCmd {
    /// The Java runtimes Sunrise downloaded from Mojang and the ones installed on this computer
    List,
    /// Download Mojang's Java runtime: a major (8, 16, 17, 21, 25) or a component name from a
    /// version's JSON (java-runtime-delta)
    Install { java: String },
}
#[derive(Subcommand)]
enum RustCmd {
    /// Whether it is installed, its version and whether this computer can link with it
    Status,
    /// Download the pinned toolchain from static.rust-lang.org (once), checked by SHA-256
    Install,
    /// Delete the toolchain and the native builds' Cargo home
    Remove,
}
#[derive(Subcommand)]
enum ServersCmd {
    /// The servers, their folder and whether they run
    List,
    /// Create a server; with no options it is a new world, survival, normal, 10 players, friends only
    Create {
        #[arg(long, default_value = "My server")]
        name: String,
        /// The new world's seed (default: random)
        #[arg(long, conflicts_with = "copy_world")]
        seed: Option<String>,
        /// Start from a copy of this world folder (one of your saves)
        #[arg(long)]
        copy_world: Option<PathBuf>,
        #[arg(long, default_value = "survival", value_parser = ["survival", "creative", "adventure", "spectator"])]
        mode: String,
        #[arg(long, default_value = "normal", value_parser = ["peaceful", "easy", "normal", "hard"])]
        difficulty: String,
        #[arg(long, default_value_t = 10)]
        max_players: u32,
        /// Let anyone in (no whitelist)
        #[arg(long)]
        open: bool,
        /// Do not check players' Minecraft accounts (offline mode: anyone can use any name)
        #[arg(long)]
        offline: bool,
        /// Kept for old scripts: account checking is now the default
        #[arg(long, hide = true, conflicts_with = "offline")]
        check_accounts: bool,
        /// Who may join (repeatable); the first becomes an operator
        #[arg(long = "player")]
        players: Vec<String>,
        /// The installed version whose programs and game files servers use, if none are set up yet
        #[arg(long)]
        version: Option<String>,
        /// Agree to the Minecraft EULA (https://aka.ms/MinecraftEULA)
        #[arg(long)]
        accept_eula: bool,
    },
    /// Copy an installed, prepared version's programs and game files for the servers (again)
    SetUp {
        #[arg(long)]
        version: Option<String>,
    },
    /// Run a sunrise-server command on a server, for example: servers run "My server" -- status
    Run {
        /// The server's name or folder
        server: String,
        #[arg(last = true, required = true)]
        args: Vec<String>,
    },
    /// Take a server off the list (its folder stays)
    Forget { server: String },
}
#[derive(Subcommand)]
enum AccountCmd {
    /// The signed-in accounts and which one plays
    List,
    /// Sign in with a Microsoft account in your browser (device code). Play needs one that owns
    /// Minecraft: Java Edition
    SignIn {
        /// Sign this account (name or UUID) in again: another Microsoft account is refused and
        /// who plays stays the same
        #[arg(long = "for")]
        target: Option<String>,
    },
    /// Renew an account's session now, even when its access token is still fresh
    Refresh { who: String },
    /// Renew an account's session for a game already running: prints the read-once session file
    /// it hands over (`--json`: {"session_file"} or {"authentication"} when it can't)
    Session { who: String },
    /// Sign out: the account's tokens leave the keychain (and the secrets file, where it kept them)
    SignOut {
        /// Its name or UUID
        who: String,
    },
    /// Play as this account
    Select { who: String },
}

#[derive(Subcommand)]
enum DataCmd {
    /// What is allowed, the install ID and how many items wait (the default)
    Status,
    /// Turn kinds on: performance, crashes, bugs, benchmarks, or all
    Allow {
        #[arg(required = true)]
        kinds: Vec<String>,
    },
    /// Turn kinds off: performance, crashes, bugs, benchmarks, or all
    Deny {
        #[arg(required = true)]
        kinds: Vec<String>,
    },
    /// Print exactly what would be sent
    Show,
    /// Delete everything sent from here, and what waits, and take a new install ID
    Delete,
    /// Send what is waiting now
    Send,
    /// Send a bug report you write; it is printed first, as it would leave
    Bug {
        text: String,
        /// Add the end of the game's log (scrubbed, without chat)
        #[arg(long)]
        log: bool,
        /// Send without asking
        #[arg(long)]
        yes: bool,
    },
    /// Upload a benchmark report (docs/benchmark-report.md); it is printed first
    Benchmark {
        file: PathBuf,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum ModpackCmd {
    /// The installed Sunrise modpacks (the default)
    List,
    /// Play a Sunrise modpack as its own profile
    Play {
        pack: String,
        /// Allow missing original Java asset/data jars to be downloaded from Modrinth
        #[arg(long)]
        allow_jar_downloads: bool,
        #[arg(long)]
        version: Option<String>,
    },
    /// Remove a Sunrise modpack and the profile's saves
    Remove { pack: String },
}

#[derive(Subcommand)]
enum InstanceCmd {
    /// The profiles, each with its game folder (* = the one Play starts)
    List,
    /// Make a profile: empty but for your options and name, or a copy of another one's
    /// worlds, options and packs
    Create {
        name: String,
        /// The profile to copy (its name or id)
        #[arg(long)]
        copy: Option<String>,
    },
    /// Rename a profile (its folder stays)
    Rename { instance: String, name: String },
    /// Delete a profile: its folder (worlds, options, packs) goes to the Trash, where you can
    /// get it back. Sunrise's own profile stays
    Delete { instance: String },
    /// Set a profile's icon: a block or item name from your prepared files (grass_block),
    /// a colour (#3fa7d6), or none
    Icon { instance: String, icon: String },
    /// Put a profile in a sidebar group, or take it out with no group
    Group {
        instance: String,
        group: Option<String>,
    },
    /// Make Play start this profile
    Select { instance: String },
    /// Keep a profile on one installed Sunrise version, or "latest" to follow updates again
    Version { instance: String, version: String },
    /// Export a profile to share as a zip: its settings, packs, Sunrise mods and the worlds
    /// named. Never logs, screenshots or account data; worlds go without their per-player files
    Export {
        /// The profile (default: the selected one)
        instance: Option<String>,
        /// A world to put in it (its name or folder); repeat for more
        #[arg(long)]
        world: Vec<String>,
        /// Where to write it (default: "<name>.zip" in the current folder)
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Show or set what runs around a profile's game: a shell command before it (the game
    /// starts only if it succeeds), a wrapper it runs through (`nice -n 5`), environment
    /// variables ("NAME=value NAME2=value") and a shell command after it exits. An empty value
    /// clears one. The game and the commands also get INST_NAME, INST_ID, INST_DIR,
    /// INST_MC_DIR. On Linux, also GameMode, MangoHud and
    /// the dedicated graphics card, when the computer has them
    Launch {
        instance: String,
        #[arg(long)]
        pre_launch: Option<String>,
        #[arg(long)]
        wrapper: Option<String>,
        #[arg(long)]
        post_exit: Option<String>,
        #[arg(long)]
        env: Option<String>,
        /// Linux: run it through GameMode (gamemoderun), on or off
        #[arg(long)]
        gamemode: Option<String>,
        /// Linux: MangoHud's overlay, on or off
        #[arg(long)]
        mangohud: Option<String>,
        /// Linux: the dedicated graphics card (DRI_PRIME, or NVIDIA's offload), on or off
        #[arg(long)]
        dedicated_gpu: Option<String>,
    },
    /// Put a shortcut on the desktop that plays a profile (`play --instance`): a .desktop
    /// file (also in the applications menu) on Linux, an app on macOS, a .lnk on Windows
    Shortcut {
        /// The profile (default: the selected one)
        instance: Option<String>,
        /// Move the profile's shortcuts to the trash instead
        #[arg(long)]
        remove: bool,
    },
    /// Import what a file is: a shared profile (a game folder zip, such as `instance export`
    /// writes) becomes a new profile; a resource or shader pack goes into the profile
    /// (default: the selected one)
    Import {
        file: PathBuf,
        /// The profile a pack goes into
        #[arg(long)]
        instance: Option<String>,
    },
}

#[derive(Subcommand)]
enum WorldsCmd {
    /// The worlds: name, mode, version, last played and size (the default)
    List {
        /// The profile (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
    },
    /// Zip a world into the profile's backups/ folder, as the game's Make Backup does
    Backup {
        /// Its folder or name
        world: String,
        #[arg(long)]
        instance: Option<String>,
    },
    /// Copy a world beside it, as "<name> (1)"
    Copy {
        world: String,
        #[arg(long)]
        instance: Option<String>,
    },
    /// Move a world to the system's trash
    Delete {
        world: String,
        #[arg(long)]
        instance: Option<String>,
    },
    /// A world's data packs, and which are on
    DatapackList {
        world: String,
        #[arg(long)]
        instance: Option<String>,
    },
    /// Copy a data pack zip into a world (the game turns it on when the world opens)
    DatapackAdd {
        world: String,
        file: PathBuf,
        #[arg(long)]
        instance: Option<String>,
    },
    /// Move a world's data pack to the system's trash
    DatapackRemove {
        world: String,
        /// Its file name, with or without .zip
        pack: String,
        #[arg(long)]
        instance: Option<String>,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum PackKindArg {
    Resource,
    Shader,
}

impl From<PackKindArg> for sunrise_launcher::packs::Kind {
    fn from(k: PackKindArg) -> Self {
        match k {
            PackKindArg::Resource => Self::Resource,
            PackKindArg::Shader => Self::Shader,
        }
    }
}

#[derive(Subcommand)]
enum PacksCmd {
    /// The resource and shader packs, and which are on (the default)
    List {
        /// Only this kind
        #[arg(long, value_enum)]
        kind: Option<PackKindArg>,
        /// The profile (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
    },
    /// Copy a pack file in: a resource pack .zip, a shader pack .zip or a .sshader
    Add {
        file: PathBuf,
        #[arg(long)]
        instance: Option<String>,
    },
    /// Turn a pack on (a resource pack goes on top; a shader pack replaces the one that was on)
    On {
        /// Its file name or title
        pack: String,
        #[arg(long, value_enum, default_value_t = PackKindArg::Resource)]
        kind: PackKindArg,
        #[arg(long)]
        instance: Option<String>,
    },
    /// Turn a pack off
    Off {
        pack: String,
        #[arg(long, value_enum, default_value_t = PackKindArg::Resource)]
        kind: PackKindArg,
        #[arg(long)]
        instance: Option<String>,
    },
    /// Turn a pack off and move it to the system's trash
    Remove {
        pack: String,
        #[arg(long, value_enum, default_value_t = PackKindArg::Resource)]
        kind: PackKindArg,
        #[arg(long)]
        instance: Option<String>,
    },
}

#[derive(Subcommand)]
enum ScreenshotsCmd {
    /// The screenshots, the newest first, with their files (the default)
    List {
        /// The profile (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
    },
    /// Move a screenshot to the system's trash
    Delete {
        /// Its file name, with or without .png
        screenshot: String,
        #[arg(long)]
        instance: Option<String>,
    },
}

#[derive(Subcommand)]
enum MultiplayerCmd {
    /// The servers of the game's Multiplayer screen (the default)
    List {
        /// The profile (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
    },
    /// Add a server to the list
    Add {
        /// Its address, as in the game's Add Server (host or host:port)
        address: String,
        /// Its name in the list (default: Minecraft Server)
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        instance: Option<String>,
    },
    /// Remove a server from the list: its position from 1, name or address
    Remove {
        server: String,
        #[arg(long)]
        instance: Option<String>,
    },
}

#[derive(Subcommand)]
enum ModsCmd {
    /// What the browser installed in a profile (the default)
    List {
        /// The profile (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
    },
    /// Search for packs Sunrise loads, or the registry's mods and modpacks
    Search {
        text: Vec<String>,
        #[arg(long, value_enum, default_value_t = KindArg::Resource)]
        kind: KindArg,
        /// Also what was made for other versions or runs only partly, each marked
        #[arg(long)]
        everything: bool,
        #[arg(long, value_enum, default_value_t = SourceArg::Modrinth)]
        source: SourceArg,
        /// The profile (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
    },
    /// Install a pack or a Sunrise mod and what it needs into a profile, by its id or slug
    /// (checksums checked); a Sunrise modpack is installed as a profile of its own; a data pack
    /// goes into the world `--world` names
    Install {
        project: String,
        /// Install the project's data pack into this world of the profile (its name or folder)
        #[arg(long)]
        world: Option<String>,
        /// The profile (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
        /// Allow a version made for another Minecraft version
        #[arg(long)]
        everything: bool,
        #[arg(long, value_enum, default_value_t = SourceArg::Modrinth)]
        source: SourceArg,
    },
    /// Update what the browser installed (or one pack)
    Update {
        project: Option<String>,
        /// Only say what is newer
        #[arg(long)]
        check: bool,
        /// The profile (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
    },
    /// Install a native mod from a GitHub link (docs/modding/src/how-to/native-mods.md, "Install from GitHub"):
    /// https://github.com/OWNER/REPO (its latest release, else its default branch),
    /// .../releases/tag/TAG, .../tree/REF or .../commit/SHA. Resolved to an exact release file or
    /// commit, checked like a registry mod. NOT reviewed by the Sunrise registry
    AddGithub {
        url: String,
        /// The mod to install when the link holds several
        #[arg(long = "mod")]
        mod_id: Option<String>,
        /// Install every mod the link holds
        #[arg(long)]
        all: bool,
        /// The profile (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
        /// Allow native code for this install (remembered; revoke in Settings, Storage)
        #[arg(long)]
        trust_native_code: bool,
    },
    /// Look for newer versions of the native mods installed from GitHub (the latest release, or the
    /// default branch's head for a mod installed from a commit), and install them
    UpdateGithub {
        /// One mod (default: all of them)
        id: Option<String>,
        /// Only say what is newer
        #[arg(long)]
        check: bool,
        /// The profile (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
        /// Allow native code for this install (remembered; revoke in Settings, Storage)
        #[arg(long)]
        trust_native_code: bool,
    },
    /// Remove a pack the browser installed
    Remove {
        project: String,
        /// The profile (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
    },
    /// Turn on a mod the browser installed (sunrise-mods.txt; the game's Mods screen has no
    /// switch)
    Enable {
        project: String,
        /// The profile (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
    },
    /// Turn off a mod the browser installed; it stays installed
    Disable {
        project: String,
        /// The profile (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum KindArg {
    Mod,
    Resource,
    Modpack,
    /// Sunrise's own shader packs (Sunrise's registry only)
    Shader,
    /// Data packs (Modrinth): install one into a world with `mods install <pack> --world <name>`
    Datapack,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum SourceArg {
    Modrinth,
    /// Sunrise's own registry: mods, modpacks and resource packs made for Sunrise, reviewed
    Sunrise,
}

impl SourceArg {
    fn get(self) -> Box<dyn sunrise_launcher::browser::Source> {
        use sunrise_launcher::browser::{self, SourceId};
        browser::source(match self {
            SourceArg::Modrinth => SourceId::Modrinth,
            SourceArg::Sunrise => SourceId::Sunrise,
        })
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
    /// Download and install a release (a tag such as v2026.10.13, or "latest")
    Install { tag: String },
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
    Play {
        /// Allow missing original Java asset/data jars to be downloaded from Modrinth
        #[arg(long)]
        allow_jar_downloads: bool,
        #[arg(long, help = "Installed version to play (default: profile version)")]
        version: Option<String>,
        /// The profile to play (its name or id; default: the selected one)
        #[arg(long)]
        instance: Option<String>,
        /// Open or create this world (a folder under saves) at once
        #[arg(long)]
        world: Option<String>,
        /// Creative mode for a new world
        #[arg(long)]
        creative: bool,
        /// Join a friend's world with its invite code
        #[arg(long)]
        join: Option<String>,
        /// Join this server at once: an address, or a server of the list by name or position
        #[arg(long, conflicts_with_all = ["world", "join"])]
        server: Option<String>,
        /// An extra argument for the game, passed through as-is (repeatable), for example
        /// --server-thread
        #[arg(long = "arg", value_name = "ARG", allow_hyphen_values = true)]
        arg: Vec<String>,
        /// Play this launch as this signed-in account (its name or UUID), without changing
        /// the one `account select` picked: several games can play as several accounts at once
        #[arg(long, value_name = "WHO")]
        account: Option<String>,
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
    /// Show disposable storage by category as JSON. Builds must be idle
    ///
    /// Rust/Cargo/temp build-tool caches remain clearable. Game-accessible caches, installed-version
    /// removal and automatic runtime eviction retain files while any launch path lacks enforceable
    /// lifetime exclusion; an empty process scan is not authorization. Read-only reports stay available.
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
    /// server export still fails, as no engine is made
    NativeBuild {
        /// Discover internal uses through compiler diagnostics. Prints resolved declarations
        /// as JSON for cargo sunrise-mod internals sync; never installs or selects a game
        #[arg(long, conflicts_with_all = ["server_output", "instance", "version", "update", "profile"], requires = "folder")]
        sync_internals: bool,
        #[arg(long, requires = "folder", conflicts_with_all = ["sync_internals", "server_output", "instance", "version", "update", "profile"])]
        catalog: bool,
        /// Copy discovery catalogs into this caller-owned empty directory
        #[arg(long, requires = "catalog")]
        catalog_output: Option<PathBuf>,
        /// A mod folder (sunrise-mod.toml with [native], Cargo.toml and src/), or a folder of them
        /// (default: the native mods installed and turned on in the profile)
        folder: Option<PathBuf>,
        /// Export mode: build a standalone dedicated-server engine into this folder instead of
        /// a profile's build. Needs a mod FOLDER; no profile is read or changed, and no
        /// launcher config is written
        #[arg(
            long = "server-output",
            value_name = "DIR",
            conflicts_with = "instance",
            requires = "folder"
        )]
        server_output: Option<PathBuf>,
        /// Allow exact-hash jar downloads into the local cache for the server export;
        /// jar bytes are never included in the exported folder
        #[arg(long, requires = "server_output")]
        allow_jar_downloads: bool,
        /// The profile whose installed native mods to build, and which then plays the build
        /// (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
        /// The installed version whose game files the build reuses, and whose commit it builds on
        /// when no mod pins one (default: the one the profile plays, so a rebuild stays on its
        /// Sunrise)
        #[arg(long)]
        version: Option<String>,
        /// Move to the newest installed release or Nightly instead of the Sunrise the profile
        /// plays (rebuilds the mods for that pinned SDK)
        #[arg(long, conflicts_with = "version")]
        update: bool,
        /// Allow native code for this install (remembered; revoke in Settings, Storage)
        #[arg(long)]
        trust_native_code: bool,
        /// Native mods use the pinned SDK's optimized compilation settings.
        /// The game is never rebuilt by a mod build.
        #[arg(long, value_enum)]
        profile: Option<native_mods::Profile>,
        /// Build the horizon-mixins tool from its pinned source instead of downloading the
        /// prebuilt one: for CI, which keeps that path proven
        #[arg(long, hide = true)]
        mixins_from_source: bool,
        /// When a mod this build can't use was saved into the profile's worlds, build without
        /// it anyway: what the mod added to those worlds is lost when they are next opened
        /// (back them up in the game first). Without this the build stops and names the
        /// worlds, leaving the profile as it is
        #[arg(long, conflicts_with = "server_output")]
        leave_out_mods: bool,
    },
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
    /// prune-old-versions on|off (delete old versions after an install, keeping the
    /// selected one, the ones profiles use and the newest few), keep-versions N (how many of the
    /// newest to keep; "default" is 2), opening-animation on|off, cat on|off (the corner cat), proxy http://host:port|socks5://host:port
    /// ("default" removes it), while-playing stay|minimize|hide (what the window does while the game runs), language
    /// system|CODE (the window's language: en, fr, de, ja, pt-br... any language the launcher has;
    /// system follows the computer's; a wrong code lists them all), theme default|ID|local:SLUG
    /// (a built-in theme, or one of yours from the themes folder; "default" is Dawn; a wrong id
    /// lists them all), accent
    /// violet|blue|teal|green|amber|red|pink|#rrggbb|DEGREES (a named accent, any colour, or any
    /// hue; "theme" goes back to the theme's own), background PATH (your picture behind Home;
    /// "default" goes back to the theme's), scenery on|off (whether anything is behind the
    /// pages), panel-opacity PERCENT|default (glass panels over Home's background),
    /// reduced-graphics on|off (no glass blur or decorative motion), cache-limit-gib N (managed cache limit
    /// in GiB: 10, 25 (default), 50, 100; 0 disables automatic cleanup)
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
    /// Sunrise modpacks from the registry: list them, play one as its own profile, or remove it
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

/// The keys `settings` sets.
const SETTING_KEYS: [&str; 19] = [
    "selected-version",
    "minecraft-dir",
    "server-dir",
    "prune-old-versions",
    "keep-versions",
    "opening-animation",
    "cat",
    "game-architecture",
    "proxy",
    "while-playing",
    "worlds",
    "language",
    "theme",
    "accent",
    "background",
    "scenery",
    "panel-opacity",
    "reduced-graphics",
    "cache-limit-gib",
];

fn on_off(value: &str) -> Result<bool, String> {
    match value {
        "on" | "true" => Ok(true),
        "off" | "false" => Ok(false),
        _ => Err(format!("{value}: say on or off")),
    }
}

fn set_setting(s: &mut backend::Settings, key: &str, value: &str) -> Result<(), String> {
    let folder = |value: &str| (value != "default").then(|| PathBuf::from(value));
    match key {
        "selected-version" => s.selected = Some(backend::resolve_version(Some(value))?),
        "minecraft-dir" => s.minecraft_dir = folder(value),
        "server-dir" => s.server_dir = folder(value),
        "prune-old-versions" => s.prune_old_versions = Some(on_off(value)?),
        "keep-versions" => {
            s.keep_versions = match value {
                "default" => None,
                n => Some(
                    n.parse::<usize>()
                        .ok()
                        .filter(|n| *n > 0)
                        .ok_or_else(|| format!("{value}: say a number from 1, or default"))?,
                ),
            }
        }
        "opening-animation" => s.skip_opening = !on_off(value)?,
        "cat" => s.cat = Some(on_off(value)?),
        "game-architecture" => {
            let choice = backend::GameArchitecture::parse(value).ok_or_else(|| {
                tr("Say auto, x64 or arm64 for the game architecture.").to_string()
            })?;
            if choice != backend::GameArchitecture::Auto && !backend::can_choose_game_architecture()
            {
                return Err(
                    tr("Manual game architectures are available on Windows ARM only.").into(),
                );
            }
            s.game_architecture = choice;
        }
        "while-playing" => {
            s.while_playing = backend::WhilePlaying::parse(value)
                .ok_or_else(|| format!("{value}: say stay, minimize or hide"))?
        }
        "worlds" => {
            s.world_layout = backend::WorldLayout::parse(value).ok_or_else(|| {
                trf(
                    "{value}: say {one} or {other}",
                    &[("value", value), ("one", "rows"), ("other", "tiles")],
                )
            })?
        }
        "theme" => sunrise_launcher::appearance::select_theme(s, value)?,
        "accent" => sunrise_launcher::appearance::type_accent(s, value)?,
        "background" => match value.trim() {
            "default" | "" => s.background = None,
            file => sunrise_launcher::appearance::set_background(
                s,
                &backend::data_dir(),
                std::path::Path::new(file),
            )?,
        },
        "scenery" => s.scenery_on = Some(on_off(value)?),
        "panel-opacity" => {
            let percent =
                match value.trim() {
                    "default" => None,
                    value => Some(value.parse::<u8>().ok().filter(|n| *n <= 100).ok_or_else(
                        || {
                            tr("Enter a whole panel opacity percentage from 0 to 100, or default.")
                                .to_string()
                        },
                    )?),
                };
            s.set_panel_opacity(percent);
        }
        "reduced-graphics" => s.reduced_graphics = Some(on_off(value)?),
        "language" => {
            use sunrise_launcher::i18n::Language;
            s.language = match value.trim() {
                "system" | "default" => None,
                code => Some(Language::parse(code).ok_or_else(|| {
                    let codes: Vec<&str> = Language::ALL.iter().map(|l| l.code()).collect();
                    format!("{value}: say system or one of {}", codes.join(", "))
                })?),
            }
        }
        "cache-limit-gib" => {
            s.cache_limit_gib = match value {
                "default" => None,
                n => Some(
                    n.parse::<u64>()
                        .ok()
                        .filter(|n| matches!(n, 0 | 10 | 25 | 50 | 100))
                        .ok_or_else(|| {
                            tr("Enter a cache limit in GiB; 0 disables automatic cleanup.")
                                .to_string()
                        })?,
                ),
            };
        }
        "proxy" => {
            if value == "default" || value.is_empty() {
                s.proxy = None;
            } else if let Some(why) = sunrise_launcher::proxy::refuse(value) {
                return Err(why);
            } else {
                s.proxy = Some(value.trim().to_string());
            }
        }
        _ => return Err(format!("No setting {key}.")),
    }
    Ok(())
}

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
        Cmd::Mcp => crate::mcp::serve().map(|()| 0),
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
            eprintln!("error: {}", sunrise_launcher::theme_files::for_terminal(&e));
            1
        }
    })
}

fn json(value: serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
}

/// Runs a game process in this terminal and reports how it ended.
fn attached(cmd: Command) -> Result<i32, String> {
    let (mut cmd, _runtime) = backend::prepare_command(cmd)?;
    let status = backend::spawn_attached(&mut cmd)
        .and_then(|mut child| child.wait())
        .map_err(|e| format!("Could not start {:?}: {e}", cmd.get_program()))?;
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

fn execute(command: Cmd, as_json: bool) -> Result<i32, String> {
    if matches!(command, Cmd::Versions { .. } | Cmd::Update { .. }) {
        for (tag, from) in backend::reuse_prepared_everywhere() {
            if !as_json {
                eprintln!("Prepared {tag} with the resources of {from}.");
            }
        }
    }
    match command {
        Cmd::Completions { shell } => {
            clap_complete::generate(
                shell,
                &mut self::command(),
                "sunrise-launcher",
                &mut std::io::stdout(),
            );
        }
        Cmd::Versions { installed: true } => {
            let installed = backend::installed();
            let selected = backend::resolve_version(None).ok();
            if as_json {
                json(serde_json::json!({ "installed": installed, "selected": selected }));
            } else if installed.is_empty() {
                println!("No version is installed.");
            }
            for tag in installed.iter().filter(|_| !as_json) {
                let mark = if selected.as_ref() == Some(tag) {
                    "*"
                } else {
                    " "
                };
                let state = if !backend::check_prepared(tag) {
                    "not prepared"
                } else if backend::has_sounds(tag) {
                    "prepared"
                } else {
                    "prepared, no sounds (prepare again)"
                };
                let label =
                    sunrise_launcher::developer_install::label(tag).unwrap_or_else(|| tag.clone());
                println!("{mark} {label}  {state}");
            }
        }
        Cmd::Versions { installed: false } => {
            let releases = backend::releases()?;
            if as_json {
                json(releases.iter().map(release_json).collect());
                return Ok(0);
            }
            let installed = backend::installed();
            for r in &releases {
                let state = match (installed.contains(&r.tag), r.asset.is_some()) {
                    (true, _) => "installed",
                    (false, true) => "",
                    (false, false) => "no build for this platform",
                };
                println!("{:<15} {}  {}", r.tag, r.date, state);
            }
        }
        Cmd::Install { tag } => {
            let label = format!("Installing {tag}");
            let tag = backend::install_tag(&tag, false, progress(label))?;
            if as_json {
                json(
                    serde_json::json!({ "installed": tag, "prepared": backend::check_prepared(&tag) }),
                );
            } else if backend::check_prepared(&tag) {
                println!(
                    "Installed {tag}. It is prepared already, from another installed version."
                );
            } else {
                println!(
                    "Installed {tag}. Prepare it next: sunrise-launcher prepare --version {tag}"
                );
            }
        }
        Cmd::Storage { clear } => {
            let storage = Storage::open().map_err(native_mods::cache_error)?;
            let (row, all) = match clear {
                Some(ClearStorage::Row(row)) => (Some(row), false),
                Some(ClearStorage::All) => (None, true),
                None => (None, false),
            };
            let report = storage.report(row, all).map_err(native_mods::cache_error)?;
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
            if row.is_some() || all {
                if let Some(kept) = report.rows.iter().flat_map(|row| &row.kept).next() {
                    return Err(kept.reason.clone());
                }
            }
        }
        Cmd::NativeLock {
            folder,
            update_lock,
        } => native_mods::registry_lock(&folder, update_lock)?,
        Cmd::NativeBuild {
            sync_internals,
            catalog,
            catalog_output,
            folder,
            server_output,
            allow_jar_downloads,
            instance,
            version,
            update,
            trust_native_code,
            profile,
            mixins_from_source,
            leave_out_mods,
        } => {
            native_permission(trust_native_code)?;
            native_mods::mixins_from_source(mixins_from_source);
            if catalog {
                let destination = catalog_output.ok_or("--catalog requires --catalog-output")?;
                let destination =
                    native_mods::catalog(&folder.expect("required by clap"), &destination)?;
                println!("{}", destination.display());
                return Ok(0);
            }
            if sync_internals {
                let entries = native_mods::sync_internals(&folder.expect("required by clap"))?;
                println!(
                    "{}",
                    serde_json::to_string(&entries).map_err(|error| error.to_string())?
                );
                return Ok(0);
            }
            let progress = native_progress(std::io::stderr().is_terminal());
            let steps = |step| {
                if let Some(label) = progress(step) {
                    eprintln!("{label}");
                }
            };
            if let Some(output) = server_output {
                // Export mode: a standalone server engine in `output`. No instance is read
                // or changed, and no launcher config is written; `folder` is required (clap).
                let folder = folder.unwrap();
                let base = match &version {
                    Some(v) => backend::resolve_version(Some(v))?,
                    // The instance's version is never looked at here: the newest installed
                    // release or Nightly, or that one with `--update` (never a native build).
                    None => native_mods::base_for(None, &backend::installed(), update)
                        .ok_or_else(backend::no_version_installed)?,
                };
                let built = native_mods::build_server(
                    &base,
                    &folder,
                    &output,
                    profile,
                    allow_jar_downloads,
                    steps,
                    registry_consent,
                )?;
                let built = std::path::absolute(&built).unwrap_or(built);
                let path = built.display().to_string();
                println!(
                    "{}",
                    trf("Built the server engine: {path}", &[("path", &path)])
                );
                println!(
                    "{}",
                    tr(
                        "Point the server at it: set engine.path to this file in the server's \
                        server.toml (docs/hosting.md); the launcher writes nothing itself."
                    )
                );
                return Ok(0);
            }
            let target = match &instance {
                Some(name) => instances::find(name)?,
                None => instances::selection()?,
            };
            let target = match profile {
                Some(profile) => {
                    instances::edit(&target.id, |meta| meta.native_profile = Some(profile))?
                }
                None => target,
            };
            let base = match version {
                Some(v) => backend::resolve_version(Some(&v))?,
                None => {
                    let plays = target.resolve_version(None).ok();
                    native_mods::base_for(plays.as_deref(), &backend::installed(), update)
                        .ok_or_else(backend::no_version_installed)?
                }
            };
            let explicit = folder.is_some() || profile.is_some();
            let folder = match folder {
                Some(folder) => folder,
                None => native_mods::installed_mods(&target.home)?,
            };
            let profile = explicit.then_some(target.native_profile);
            let built =
                match native_mods::build(&base, &folder, &target, profile, leave_out_mods, &native_mods::Cancellation::default(), steps, registry_consent, || backend::releases().ok()) {
                    // A content mod needs no build: the instance stays as it is.
                    Err(nothing @ native_mods::Failed::Nothing) => {
                        println!("{nothing}");
                        return Ok(0);
                    }
                    result => result,
                }
                .map_err(|error| {
                    if let native_mods::Failed::Worlds(stake) = &error {
                        return trf(
                            "{error}\n{stay}: nothing changed. Or run again with --leave-out-mods to build without the mods named.",
                            &[
                                ("error", &stake.text),
                                ("stay", &native_mods::fit::stay(&stake.stay)),
                            ],
                        );
                    }
                    let error = String::from(error);
                    if error.starts_with(native_mods::MISMATCH) {
                        trf("{error} Run `cargo sunrise-mod update --sunrise {commit}` in each mod's folder to pin it to this game, then build again.", &[("error", &error), ("commit", &base)])
                    } else { error }
                })?;
            println!("{built}");
        }
        Cmd::DeveloperInstall(args) => developer_builds::install(args, as_json)?,
        Cmd::Remove { tag } => {
            backend::remove_version(&tag)?;
            println!("Removed {tag}.");
        }
        Cmd::ImportBuild {
            path,
            version,
            trust_unsigned,
        } => developer_builds::import(&path, version.as_deref(), trust_unsigned, as_json)?,
        Cmd::Prepare {
            minecraft,
            download,
            version,
        } => {
            let tag = backend::resolve_version(version.as_deref())?;
            let minecraft = backend::minecraft_dir(minecraft)?;
            let from = if download {
                backend::MinecraftFrom::Mojang
            } else {
                backend::MinecraftFrom::Install
            };
            let _preparing = backend::prepare_lock(&tag)?;
            let (preparer, minecraft) =
                backend::preparer(&tag, &minecraft, from, &|line| eprintln!("{line}"))?;
            backend::sounds(&tag, &minecraft, &|line| println!("{line}"));
            let code = attached(preparer)?;
            if code != 0 {
                return Ok(code);
            }
            println!("{tag} is prepared.");
        }
        Cmd::Play {
            allow_jar_downloads,
            version,
            instance,
            world,
            creative,
            join,
            server,
            arg,
            account,
        } => {
            let instance = match instance {
                Some(name) => instances::find(&name)?,
                None => instances::selection()?,
            };
            // Who plays this launch only.
            let who = match account {
                Some(who) => Some(
                    Accounts::load()
                        .find(&who)
                        .ok_or_else(|| {
                            format!("No signed-in account {who}. See `sunrise-launcher account`.")
                        })?
                        .uuid
                        .clone(),
                ),
                None => None,
            };
            let tag = instance.resolve_version(version.as_deref())?;
            let mut cmd = backend::client_as(&tag, who.as_deref())?;
            instances::apply(&mut cmd, &instance)?;
            if let Some(code) = join {
                backend::join_friend(&mut cmd, &code)?;
            }
            if let Some(world) = world {
                cmd.arg("--world").arg(world);
            }
            if let Some(server) = server {
                let list = server_list::List::load(&instance.home);
                let ip = server_list::find(&list, &server).map_or(server, |(_, s)| s.ip);
                cmd.arg("--server").arg(ip);
            }
            if creative {
                cmd.arg("--creative");
            }
            cmd.args(arg);
            let started = std::time::Instant::now();
            let vars = instance.vars();
            cmd.envs(vars.iter().map(|(k, v)| (k, v)));
            for notice in
                sunrise_launcher::browser::sunrise::before_play(&mut cmd, &tag, &instance.launch)?
            {
                eprintln!("{notice}");
            }
            sunrise_launcher::jars::prepare_launch(
                &mut cmd,
                &instance.launch,
                sunrise_launcher::jars::Permission::headless(allow_jar_downloads),
            )
            .map_err(|error| error.to_string())?;
            let played = attached(sunrise_launcher::launch::wrap(cmd, &instance.launch)?);
            if let Err(e) = instances::record_play(&instance.id, started.elapsed()) {
                eprintln!("Play time not saved: {e}");
            }
            after_exit(&instance, &vars);
            return played;
        }
        Cmd::Server {
            dir,
            version,
            accept_eula,
            allow_jar_downloads,
        } => {
            let tag = backend::resolve_version(version.as_deref())?;
            let dir = backend::server_dir(dir);
            // The server inherits this terminal, so typed lines reach its console.
            let mut cmd = backend::server(&tag, &dir, accept_eula)?;
            sunrise_launcher::jars::prepare_launch(
                &mut cmd,
                &Default::default(),
                sunrise_launcher::jars::Permission::headless(allow_jar_downloads),
            )
            .map_err(|error| error.to_string())?;
            return attached(cmd);
        }
        Cmd::Notes { tag } => {
            let r = backend::release_notes(tag.as_deref())?;
            if as_json {
                json(
                    serde_json::json!({ "tag": r.tag, "name": r.name, "date": r.date, "notes": r.body }),
                );
            } else {
                let title = if r.name.is_empty() { &r.tag } else { &r.name };
                println!(
                    "{title}\n{}  {}\n\n{}",
                    r.tag,
                    r.date,
                    backend::markdown_to_text(&r.body)
                );
            }
        }
        Cmd::Account { action } => account_command(action.unwrap_or(AccountCmd::List), as_json)?,
        Cmd::Data { action } => data(action.unwrap_or(DataCmd::Status), as_json)?,
        Cmd::Benchmark { repeats } => {
            let path = crate::perf::benchmark(repeats.max(1))?;
            if as_json {
                json(serde_json::json!({ "report": path }));
            } else {
                println!("{}", path.display());
            }
        }
        Cmd::Links => {
            let links = backend::links();
            if as_json {
                json(serde_json::to_value(&links).unwrap());
            } else {
                for (name, url) in [
                    ("Website", &links.website),
                    ("Roadmap", &links.roadmap),
                    ("Documentation", &links.docs),
                    ("Discord", &links.discord),
                ] {
                    println!("{name:<8} {}", if url.is_empty() { "(none)" } else { url });
                }
            }
        }
        Cmd::NoAi { state } => {
            if let Some(state) = state {
                backend::set_no_ai(state == "on")?;
            }
            let on = backend::no_ai();
            if as_json {
                json(serde_json::json!({ "no_ai": on }));
            } else if on {
                println!("No AI is on: every AI feature is hidden and off.");
            } else {
                println!("No AI is off: AI features are available.");
            }
        }
        Cmd::Settings { key, value } => {
            let mut settings = backend::Settings::load();
            if let Some(key) = key {
                let value = value.ok_or("settings KEY VALUE: the value is missing")?;
                set_setting(&mut settings, &key, &value)?;
                settings.save()?;
                sunrise_launcher::proxy::announce(settings.proxy.clone());
            }
            let all = settings_json(&settings);
            if as_json {
                json(all);
            } else {
                for (key, value) in all.as_object().into_iter().flatten() {
                    println!("{key}: {value}");
                }
            }
        }
        Cmd::GameArgs { args } => {
            let mut settings = backend::Settings::load();
            if let Some(line) = args {
                settings.game_args = line;
                settings.save()?;
            }
            let line = settings.game_args;
            if as_json {
                json(serde_json::json!({
                    "game_args": line,
                    "parsed": backend::split_args(&line),
                }));
            } else if line.is_empty() {
                println!("No extra game arguments.");
            } else {
                println!("Game arguments: {line}");
            }
        }
        Cmd::ShareLog { instance } => {
            use sunrise_launcher::game_log;
            let i = match instance {
                Some(name) => instances::find(&name)?,
                None => instances::selection()?,
            };
            let path = i.home.join("logs/latest.log");
            let text =
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let names = game_log::names(&[&i.home, &backend::game_home()]);
            let text = game_log::private(&text, dirs::home_dir().as_deref(), &names);
            let shared = game_log::share(&text)?;
            if as_json {
                json(serde_json::json!({
                    "url": shared.url, "delete": format!("{}:{}", shared.id, shared.token),
                }));
            } else {
                println!("{}", shared.url);
                println!(
                    "Delete it: sunrise-launcher unshare-log {}:{}",
                    shared.id, shared.token
                );
            }
        }
        Cmd::UnshareLog { shared } => {
            let (id, token) = shared
                .split_once(':')
                .ok_or("Give the \"<id>:<token>\" share-log printed.")?;
            sunrise_launcher::game_log::unshare(&sunrise_launcher::game_log::Shared {
                url: String::new(),
                id: id.into(),
                token: token.into(),
            })?;
            println!("Deleted the shared log {id}.");
        }
        Cmd::Logs { lines, crash } => {
            let (path, text) = backend::read_logs(lines, crash)?;
            if as_json {
                json(serde_json::json!({ "path": path, "text": text }));
            } else {
                print!("{text}");
            }
        }
        Cmd::Update { check } => update(check, as_json)?,
        Cmd::BugReport { text, log } => {
            let path = sunrise_launcher::bug_report::write(&text, log)?;
            if as_json {
                json(serde_json::json!({ "path": path }));
            } else {
                println!(
                    "Wrote {}: read it, then attach it to your issue.",
                    path.display()
                );
            }
        }
        Cmd::DesktopEntry { remove } => {
            let mut settings = backend::Settings::load();
            settings.desktop_entry = Some(!remove);
            settings.save()?;
            if remove {
                sunrise_launcher::desktop::remove()?;
                println!("Removed Sunrise from the applications menu.");
            } else {
                sunrise_launcher::desktop::install(backend::exe_path())?;
                println!("Sunrise is in the applications menu.");
            }
        }
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
fn java(action: JavaCmd, as_json: bool) -> Result<(), String> {
    use sunrise_launcher::java;
    match action {
        JavaCmd::List => {
            let mut all = java::mojang_runtimes(&java::runtimes_dir());
            all.extend(java::installed());
            if as_json {
                json(serde_json::json!(all));
            } else if all.is_empty() {
                println!("No Java yet. Sunrise downloads Mojang's when a profile needs it.");
            } else {
                for r in &all {
                    let from = match r.source {
                        java::Source::Mojang => "Mojang's, downloaded by Sunrise",
                        java::Source::Installed => "installed",
                    };
                    println!("Java {}  ({from})\n  {}", r.version, r.home.display());
                }
            }
        }
        JavaCmd::Install { java: which } => {
            let need = java::Need::named(&which)?;
            let runtime = java::install(&need, &|line| eprintln!("{line}"))?;
            if as_json {
                json(serde_json::json!(runtime));
            } else {
                println!(
                    "Java {} is ready: {}",
                    runtime.version,
                    runtime.home.display()
                );
            }
        }
    }
    Ok(())
}
fn rust(action: RustCmd, as_json: bool) -> Result<(), String> {
    use sunrise_launcher::rust_toolchain as rust;
    match action {
        RustCmd::Status => {
            let installed = rust::installed();
            let version = installed.as_ref().map(|t| t.version()).transpose()?;
            let linker = rust::linker().err();
            if as_json {
                json(serde_json::json!({
                    "pinned": rust::VERSION,
                    "host": rust::host().map(|h| h.target),
                    "installed": installed.as_ref().map(|t| t.dir.display().to_string()),
                    "version": version,
                    "linker": linker,
                }));
            } else {
                match (&installed, &version) {
                    (Some(t), Some(v)) => println!("{v} in {}", t.dir.display()),
                    _ => println!(
                        "Rust {} is not installed (`sunrise-launcher rust install`).",
                        rust::VERSION
                    ),
                }
                println!(
                    "{}",
                    linker.unwrap_or_else(|| "This computer can link the game.".into())
                );
            }
        }
        RustCmd::Install => {
            let shown = std::cell::Cell::new(0);
            let toolchain = rust::install(|f| {
                let percent = (f * 100.0) as u32;
                if percent >= shown.get() + 10 {
                    shown.set(percent);
                    eprintln!("{percent}%");
                }
            })?;
            println!("{} in {}", toolchain.version()?, toolchain.dir.display());
        }
        RustCmd::Remove => {
            rust::remove()?;
            println!("Removed the launcher's Rust toolchain and Cargo home.");
        }
    }
    Ok(())
}
fn update(check: bool, as_json: bool) -> Result<(), String> {
    let updates = backend::check_updates();
    // A feed that couldn't be read is never "up to date": its error is printed, and the command fails after the other feed's result.
    let failed = || match (&updates.sunrise_error, &updates.launcher_error) {
        (None, None) => Ok(()),
        (Some(e), None) => Err(format!("Sunrise's update check failed: {e}")),
        (None, Some(e)) => Err(format!("The launcher's update check failed: {e}")),
        (Some(s), Some(l)) => Err(format!(
            "Both update checks failed: Sunrise's ({s}) and the launcher's ({l})"
        )),
    };
    if as_json && check {
        json(serde_json::json!({
            "sunrise": updates.sunrise.as_ref().map(|r| &r.tag),
            "launcher": updates.launcher.as_ref().and_then(|r| backend::launcher_version(&r.tag)),
            "launcher_current": backend::VERSION,
            "sunrise_error": updates.sunrise_error,
            "launcher_error": updates.launcher_error,
        }));
        return failed();
    }
    match &updates.sunrise {
        None if updates.sunrise_error.is_some() => println!("Sunrise: not checked."),
        None => println!("Sunrise: up to date."),
        Some(r) if check => println!("Sunrise: {} is available.", r.tag),
        Some(r) => {
            let tag =
                backend::install_tag(&r.tag, true, progress(format!("Installing {}", r.tag)))?;
            println!("Sunrise: installed {tag}.");
        }
    }
    match &updates.launcher {
        None if updates.launcher_error.is_some() => println!("Launcher: not checked."),
        None => println!("Launcher: {} is up to date.", backend::VERSION),
        Some(r) => {
            let version = backend::launcher_version(&r.tag).unwrap_or_default();
            if check {
                println!(
                    "Launcher: {version} is available (this is {}).",
                    backend::VERSION
                );
            } else if !backend::self_update_allowed() {
                println!("Launcher: {version} is available; this build does not replace itself.");
            } else {
                backend::self_update(r, progress(format!("Updating the launcher to {version}")))?;
                println!("Launcher: updated to {version}. Restart the launcher to use it.");
            }
        }
    }
    failed()
}
/// `account …`: names and UUIDs only; tokens are never printed.
fn account_command(action: AccountCmd, as_json: bool) -> Result<(), String> {
    let mut accounts = Accounts::load();
    let find = |accounts: &Accounts, who: &str| {
        accounts
            .find(who)
            .cloned()
            .ok_or_else(|| format!("No signed-in account {who}. See `sunrise-launcher account`."))
    };
    match action {
        AccountCmd::List => {
            if as_json {
                json(serde_json::json!({
                    "accounts": accounts.accounts.iter().map(|a| serde_json::json!({
                        "name": a.name, "uuid": a.uuid,
                        "selected": accounts.selected.as_ref() == Some(&a.uuid),
                        "authentication": account::session::status(account::store(), &a.uuid),
                    })).collect::<Vec<_>>(),
                    "sign_in_configured": account::client_id().is_some(),
                    "owner": accounts.has_owner(account::store()),
                }));
            } else if accounts.accounts.is_empty() {
                println!(
                    "No Microsoft account signed in. Play needs one that owns Minecraft: Java \
                     Edition: sunrise-launcher account sign-in"
                );
            } else {
                for a in &accounts.accounts {
                    let mark = if accounts.selected.as_ref() == Some(&a.uuid) {
                        "*"
                    } else {
                        " "
                    };
                    let status = account::session::status(account::store(), &a.uuid);
                    // Why it couldn't be checked (a keyring that didn't answer, #12179).
                    let why = match status.state {
                        account::session::State::Retry => format!(": {}", status.message),
                        _ => String::new(),
                    };
                    println!("{mark} {}  {}  {}{why}", a.name, a.uuid, status.label());
                }
                if !accounts.has_owner(account::store()) {
                    println!(
                        "{} {}",
                        account::NoOwner::default(),
                        tr(backend::SIGN_IN_HINT)
                    );
                }
            }
        }
        AccountCmd::Refresh { who } => {
            let status = account::session::refresh(&who)?;
            if as_json {
                json(serde_json::to_value(&status).map_err(|e| e.to_string())?);
            } else if status.state != account::session::State::Ready {
                return Err(format!("{}: {}", status.label(), status.message));
            } else {
                println!("{}: {}", status.label(), status.message);
            }
        }
        AccountCmd::Session { who } => {
            let dir = backend::game_home().join("sessions");
            match (account::session::renew_for_game(&dir, &who), as_json) {
                (Ok(file), true) => json(serde_json::json!({ "session_file": file })),
                (Ok(file), false) => println!("{}", file.display()),
                (Err(status), true) => json(serde_json::json!({ "authentication": status })),
                (Err(status), false) => return Err(status.message),
            }
        }
        AccountCmd::SignIn { target: Some(who) } => {
            let a = find(&accounts, &who)?;
            let sign_in = account::SignIn::start_for(a.clone());
            let mut shown = false;
            let signed_in = loop {
                match sign_in.state() {
                    account::SignInState::Code(code) if !shown => {
                        shown = true;
                        if as_json {
                            json(serde_json::json!({
                                "link": code.verification_uri,
                                "code": code.user_code,
                            }));
                        } else {
                            eprintln!(
                                "To sign {} in again, open {} in a browser and enter the code {}",
                                a.name, code.verification_uri, code.user_code
                            );
                        }
                    }
                    account::SignInState::Done(result) => break result?,
                    _ => {}
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            };
            if as_json {
                json(serde_json::json!({ "signed_in": signed_in.uuid }));
            } else {
                println!("Signed in as {} again.", signed_in.name);
            }
        }
        AccountCmd::SignIn { target: None } => {
            let client_id = account::client_id().ok_or(account::NOT_CONFIGURED)?;
            let ep = account::Endpoints::live();
            let code = account::device_code(&ep, &client_id)?;
            eprintln!(
                "To sign in, open {} in a browser and enter the code {}",
                code.verification_uri, code.user_code
            );
            let ms = account::wait_for_sign_in(&ep, &client_id, &code, &|| false)?;
            let (signed_in, secrets) = account::finish_sign_in(&ep, ms)?;
            accounts.add(account::store(), signed_in.clone(), &secrets)?;
            accounts.save()?;
            println!("Signed in as {}.", signed_in.name);
            if let Some(said) = account::no_keychain() {
                eprintln!("{said}");
            }
        }
        AccountCmd::SignOut { who } => {
            let a = find(&accounts, &who)?;
            accounts.remove(account::store(), &a.uuid)?;
            accounts.save()?;
            println!("Signed out {}.", a.name);
        }
        AccountCmd::Select { who } => {
            let a = find(&accounts, &who)?;
            accounts.play_as(&a.uuid)?;
            accounts.save()?;
            println!("The game plays as {}.", a.name);
        }
    }
    Ok(())
}
/// `data`: the same consent as Settings → Data collection.
fn data(action: DataCmd, as_json: bool) -> Result<(), String> {
    use sunrise_launcher::telemetry;
    let review = |item: &sunrise_launcher_telemetry::Item, yes: bool| -> Result<bool, String> {
        println!(
            "{}",
            serde_json::to_string_pretty(item).map_err(|e| e.to_string())?
        );
        if yes {
            return Ok(true);
        }
        if !std::io::stdin().is_terminal() {
            return Err("Add --yes to send it without asking.".into());
        }
        eprint!("Send this? [y/N] ");
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        Ok(line.trim().eq_ignore_ascii_case("y"))
    };
    match action {
        DataCmd::Status => {
            let status = telemetry::status_json();
            if as_json {
                json(status);
                return Ok(());
            }
            let consent = telemetry::consent();
            if consent.needs_asking() {
                println!("Not answered yet: nothing is collected.");
            }
            for (kind, name, _, _) in telemetry::SWITCHES {
                let on = consent.allows(kind);
                println!("{name}: {}", if on { "on" } else { "off" });
            }
            println!("Waiting to be sent: {}", status["waiting"]);
            match sunrise_launcher_telemetry::endpoint() {
                Some(url) => println!("Server: {url}"),
                None => println!("Server: turned off (SUNRISE_TELEMETRY_URL=off), nothing is sent"),
            }
        }
        DataCmd::Allow { kinds } => {
            telemetry::set(&telemetry::parse_kinds(&kinds)?, true)?;
            return data(DataCmd::Status, as_json);
        }
        DataCmd::Deny { kinds } => {
            telemetry::set(&telemetry::parse_kinds(&kinds)?, false)?;
            return data(DataCmd::Status, as_json);
        }
        DataCmd::Show => {
            let (what, text) = telemetry::preview();
            eprintln!("{what}");
            println!("{text}");
        }
        DataCmd::Delete => println!("{}", telemetry::delete_my_data()?),
        DataCmd::Send => println!("{:?}", telemetry::send()),
        DataCmd::Bug { text, log, yes } => {
            telemetry::ensure_allowed(sunrise_launcher_telemetry::Kind::Bug)?;
            let item = telemetry::bug_report(&text, log)?;
            if review(&item, yes)? {
                println!("{}", telemetry::submit(&item)?);
            }
        }
        DataCmd::Benchmark { file, yes } => {
            telemetry::ensure_allowed(sunrise_launcher_telemetry::Kind::Benchmark)?;
            let item = telemetry::benchmark_report(&file)?;
            if review(&item, yes)? {
                println!("{}", telemetry::submit(&item)?);
            }
        }
    }
    Ok(())
}
fn servers(action: ServersCmd, as_json: bool) -> Result<(), String> {
    use sunrise_launcher::servers::{self, NewServer, World};
    let find = |which: &str| {
        servers::list()
            .into_iter()
            .find(|e| e.name == which || e.dir == std::path::Path::new(which))
            .ok_or_else(|| {
                format!("No server called {which}: `sunrise-launcher servers` lists them")
            })
    };
    match action {
        ServersCmd::List => {
            let list = servers::list();
            if as_json {
                json(serde_json::Value::Array(
                    list.iter()
                        .map(|e| {
                            serde_json::json!({
                                "name": e.name, "dir": e.dir,
                                "phase": servers::status(&e.dir).phase,
                            })
                        })
                        .collect(),
                ));
            } else if list.is_empty() {
                println!("No server yet: sunrise-launcher servers create --accept-eula");
            }
            for e in list.iter().filter(|_| !as_json) {
                println!(
                    "{}  ({}): {}",
                    e.name,
                    e.dir.display(),
                    servers::status(&e.dir).phase
                );
            }
        }
        ServersCmd::Create {
            name,
            seed,
            copy_world,
            mode,
            difficulty,
            max_players,
            open,
            offline,
            check_accounts: _,
            players,
            version,
            accept_eula,
        } => {
            let spec = NewServer {
                name,
                world: match copy_world {
                    Some(dir) => World::Copy(dir),
                    None => World::New {
                        seed: seed.unwrap_or_default(),
                    },
                },
                game_mode: mode,
                difficulty,
                max_players,
                friends_only: !open,
                check_accounts: !offline,
                players,
                eula: accept_eula,
            };
            let tag = backend::resolve_version(version.as_deref()).ok();
            let entry = servers::create(&spec, tag.as_deref(), &|l| eprintln!("{l}"))
                .map_err(|p| p.message)?;
            if as_json {
                json(serde_json::json!({ "name": entry.name, "dir": entry.dir }));
            } else {
                println!("Created {} in {}", entry.name, entry.dir.display());
            }
        }
        ServersCmd::SetUp { version } => {
            let tag = backend::resolve_version(version.as_deref())?;
            servers::set_up(&tag)?;
            println!(
                "Servers run Sunrise {}",
                servers::runtime_release().unwrap_or(tag)
            );
        }
        ServersCmd::Run { server, args } => {
            let entry = find(&server)?;
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let status = servers::command(&entry.dir, &args)?
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err(format!("sunrise-server exited with {status}"));
            }
        }
        ServersCmd::Forget { server } => {
            let entry = find(&server)?;
            servers::forget(&entry.dir)?;
            println!(
                "{} is off the list; its folder stays in {}",
                entry.name,
                entry.dir.display()
            );
        }
    }
    Ok(())
}
/// Installs the native mods of `found` (a GitHub link, resolved) into the game folder `home`: the
/// warning and the exact source link first, and nothing downloaded without `trust`.
fn mods(action: ModsCmd, as_json: bool) -> Result<(), String> {
    use sunrise_launcher::browser::{installed, sunrise, Kind, Query, SourceId};
    // What the browser installs goes into an instance's game folder.
    let instance = match &action {
        ModsCmd::List { instance }
        | ModsCmd::Search { instance, .. }
        | ModsCmd::Install { instance, .. }
        | ModsCmd::Update { instance, .. }
        | ModsCmd::Remove { instance, .. }
        | ModsCmd::AddGithub { instance, .. }
        | ModsCmd::UpdateGithub { instance, .. }
        | ModsCmd::Enable { instance, .. }
        | ModsCmd::Disable { instance, .. } => match instance {
            Some(name) => instances::find(name)?,
            None => instances::selection()?,
        },
    };
    let home = match &action {
        ModsCmd::Search { .. } => backend::game_home(),
        _ => instance.home.clone(),
    };
    // The Minecraft version the instance's own build plays (its pinned build), else the launcher's.
    let game = backend::played_version_id_by(instance.version.as_deref());
    let entry_json = |e: &installed::Entry| serde_json::to_value(e).unwrap_or_default();
    let line = |label: &str, f: f32| progress(label.to_string())(f);
    let from_registry = installed::Installed::load(&home)
        .entries
        .iter()
        .any(|e| e.source == SourceId::Sunrise);
    if from_registry && sunrise::index().is_ok() {
        if let Some(index) = sunrise::fetched_index() {
            for said in sunrise::turn_off_revoked(&index, &home)? {
                eprintln!("{said}");
            }
        }
    }
    let on = matches!(action, ModsCmd::Enable { .. });
    match action {
        ModsCmd::List { .. } => {
            let list = installed::Installed::load(&home).entries;
            if as_json {
                json(list.iter().map(entry_json).collect());
            } else if list.is_empty() && github_mods::origins(&home).is_empty() {
                println!(
                    "Nothing installed from the browser yet: sunrise-launcher mods search <text>"
                );
            }
            for o in sunrise_launcher::github_mods::origins(&home)
                .iter()
                .filter(|_| !as_json)
            {
                println!(
                    "{} {}  (native mod, from GitHub: {})\n  {}",
                    o.id,
                    o.version,
                    o.label(),
                    o.source_url()
                );
            }
            for e in list.iter().filter(|_| !as_json) {
                println!(
                    "{}  {}  ({}, {})
  {}  {}",
                    e.title,
                    e.number,
                    e.source.label(),
                    e.slug,
                    e.file,
                    e.checksum
                );
            }
        }
        ModsCmd::Search {
            text,
            kind,
            everything,
            source,
            ..
        } => {
            let kind = match kind {
                KindArg::Resource => Kind::ResourcePack,
                KindArg::Modpack => Kind::Modpack,
                KindArg::Mod => Kind::Mod,
                KindArg::Shader => Kind::Shader,
                KindArg::Datapack => Kind::DataPack,
            };
            let source = source.get();
            // A registry modpack becomes a profile of its own, on the launcher's build, as its import does.
            let game = match (kind, source.id()) {
                (Kind::Modpack, SourceId::Sunrise) => sunrise_launcher::browser::game(),
                _ => game.clone(),
            };
            let query = Query {
                everything,
                game,
                ..Query::new(&text.join(" "), kind)
            };
            let found = source.search(&query)?;
            let datapack = kind == Kind::DataPack;
            if as_json {
                json(found.iter().map(|p| {
                    let support = p.support(&query.game);
                    serde_json::json!({
                        "id": p.id, "slug": p.slug, "title": p.title, "summary": p.summary,
                        "author": p.author, "downloads": p.downloads, "page": p.page,
                        "license": p.license, "support": support.label(), "reason": support.reason(),
                        "about": if p.source == SourceId::Sunrise { sunrise::trust_and_capabilities(&p.id, Some(&instance), &query.game) } else { Vec::new() },
                    })
                }).collect());
            } else {
                if datapack && !found.is_empty() {
                    println!("{}\n", sunrise_launcher::browser::datapack_support());
                }
                for p in &found {
                    let support = p.support(&query.game);
                    if p.source == SourceId::Sunrise {
                        println!("{}  ({})  {}\n  {}", p.title, p.slug, p.author, p.summary);
                        for line in
                            sunrise::trust_and_capabilities(&p.id, Some(&instance), &query.game)
                        {
                            println!("  {line}");
                        }
                        continue;
                    }
                    println!(
                        "{}  ({})  {} · {} downloads
  {}",
                        p.title,
                        p.slug,
                        support.label(),
                        p.downloads,
                        p.summary
                    );
                    if datapack {
                        println!("  sunrise-launcher mods install {} --world <world>", p.slug);
                    } else if let Some(why) = support.reason() {
                        println!("  {why}");
                    }
                }
            }
        }
        ModsCmd::Install {
            project,
            everything,
            source,
            world,
            ..
        } => {
            let source = source.get();
            let p = source.project(&project)?;
            if let Some(world) = world {
                let world = sunrise_launcher::worlds::find(&home, &world)?;
                let done = installed::install_datapack(
                    &*source,
                    &p,
                    everything,
                    &world.path,
                    &game,
                    &progress(p.title.clone()),
                )?;
                if as_json {
                    json(serde_json::json!({
                        "world": world.folder, "file": done.pack.file, "version": done.number,
                        "runs": done.coverage.read, "not_read": done.coverage.unread,
                    }));
                } else {
                    println!(
                        "{} {} is in {}'s data packs as {}: the game turns it on when the world \
                         opens. {}",
                        p.title,
                        done.number,
                        world.name,
                        done.pack.file,
                        done.coverage.summary()
                    );
                }
                return Ok(());
            }
            if p.kind == Kind::DataPack {
                return Err(format!(
                    "{} is a data pack: say which world with --world.",
                    p.title
                ));
            }
            if p.kind == Kind::Modpack && p.source == SourceId::Sunrise {
                let pack = sunrise::pack::install(&p.id, &line)?;
                if as_json {
                    json(
                        serde_json::json!({"pack": pack.id, "version": pack.version, "mods": pack.mods, "home": sunrise::pack::home(&pack.work)}),
                    );
                } else {
                    println!(
                        "{} {} is installed as its own profile, with {}.",
                        pack.name,
                        pack.version,
                        pack.mods.join(", ")
                    );
                    println!("Play it: sunrise-launcher modpack play {}", pack.id);
                }
                return Ok(());
            }
            let done = installed::install(&*source, &p, everything, &home, &game, &line)?;
            if as_json {
                json(done.iter().map(entry_json).collect());
            }
            for e in done.iter().filter(|_| !as_json) {
                println!("{} {}: {} ({})", e.title, e.number, e.file, e.checksum);
            }
        }
        ModsCmd::Update { project, check, .. } => {
            let list = installed::Installed::load(&home).entries;
            let chosen: Vec<_> = list
                .iter()
                .filter(|e| {
                    project
                        .as_deref()
                        .is_none_or(|n| e.project == n || e.slug == n)
                })
                .collect();
            if chosen.is_empty() {
                return Err(match project {
                    Some(n) => format!("{n} wasn't installed by the browser."),
                    None => "Nothing installed from the browser yet.".into(),
                });
            }
            for e in chosen {
                let source = sunrise_launcher::browser::source(e.source);
                let newer = installed::update_for(&*source, e, &game)?;
                match newer {
                    None => println!("{} {}: up to date.", e.title, e.number),
                    Some(v) if check => {
                        println!("{} {}: {} is available.", e.title, e.number, v.number)
                    }
                    Some(_) => {
                        let p = source.project(&e.project)?;
                        let done = installed::install(&*source, &p, true, &home, &game, &line)?;
                        for done in done {
                            println!("{} {}: {}", done.title, done.number, done.file);
                        }
                    }
                }
            }
        }
        ModsCmd::AddGithub {
            url,
            mod_id,
            all,
            trust_native_code,
            ..
        } => {
            let link = github_mods::parse(&url)?;
            let found = github_mods::resolve(&link, &github_mods::api)?;
            github_native(&home, &found, mod_id.as_deref(), all, trust_native_code)?;
        }
        ModsCmd::UpdateGithub {
            id,
            check,
            trust_native_code,
            ..
        } => {
            let mut origins = github_mods::origins(&home);
            origins.retain(|o| id.as_deref().is_none_or(|i| o.id == i));
            if origins.is_empty() {
                return Err(match id {
                    Some(i) => format!("{i} wasn't installed from GitHub."),
                    None => "No native mod was installed from GitHub: sunrise-launcher mods add-github <url>".into(),
                });
            }
            for o in origins {
                let latest = github_mods::resolve(&o.follows(), &github_mods::api)?;
                if !github_mods::is_update(&o, &latest) {
                    println!("{} {}: up to date ({}).", o.id, o.version, o.label());
                    continue;
                }
                println!(
                    "{}: {} is available (now {}).",
                    o.id,
                    latest.label(),
                    o.label()
                );
                if !check {
                    github_native(&home, &latest, Some(&o.id), false, trust_native_code)?;
                }
            }
        }
        ModsCmd::Remove { project, .. } if github_mods::origin(&home, &project).is_some() => {
            github_mods::remove(&home, &project)?;
            println!("Removed {project} (installed from GitHub).");
        }
        ModsCmd::Remove { project, .. } => {
            let e = installed::remove(&home, &project)?;
            println!("Removed {} ({}).", e.title, e.file);
        }
        ModsCmd::Enable { project, .. } | ModsCmd::Disable { project, .. } => {
            let e = installed::Installed::load(&home)
                .lookup(&project)
                .cloned()
                .ok_or_else(|| format!("{project} wasn't installed by the browser."))?;
            let id = sunrise::mod_id(&e)
                .filter(|_| e.kind == Kind::Mod)
                .ok_or_else(|| format!("{} isn't a mod: only mods turn on and off.", e.title))?;
            if on {
                if let Some(r) = sunrise::index()
                    .ok()
                    .filter(|_| e.source == SourceId::Sunrise)
                    .and_then(|i| {
                        i.revoked(&e.project, e.checksum.trim_start_matches("SHA-256:"))
                            .cloned()
                    })
                {
                    return Err(format!("{} was taken down: {}", e.title, r.reason));
                }
            }
            sunrise_launcher::browser::mod_list::set_enabled(&home, id, on)?;
            println!("{} is {}.", e.title, if on { "on" } else { "off" });
        }
    }
    Ok(())
}
/// Runs the profile's post-exit command in this terminal, if it has one, and waits for it.
fn after_exit(instance: &instances::Instance, vars: &[(&'static str, std::ffi::OsString)]) {
    let Some(mut cmd) =
        sunrise_launcher::launch::after_exit(&instance.launch, &instance.home, vars)
    else {
        return;
    };
    match cmd.status() {
        Ok(s) if s.success() => {}
        Ok(s) => eprintln!("The post-exit command failed ({s})."),
        Err(e) => eprintln!("The post-exit command didn't run: {e}"),
    }
}

fn instance(action: InstanceCmd, as_json: bool) -> Result<(), String> {
    use instances::Icon;
    let value = |i: &instances::Instance, selected: bool| {
        serde_json::json!({
            "id": i.id, "name": i.name, "kind": format!("{:?}", i.kind).to_lowercase(),
            "home": i.home, "icon": i.icon, "group": i.group, "selected": selected,
            "version": i.version, "version_label": i.version.as_deref().and_then(sunrise_launcher::developer_install::label), "played_secs": i.played.as_secs(),
            "last_played_ms": i.last_played.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64),
        })
    };
    let line = |i: &instances::Instance, selected: bool| {
        let mark = if selected { "*" } else { " " };
        let group = i
            .group
            .as_ref()
            .map(|g| format!("  [{g}]"))
            .unwrap_or_default();
        let pinned = match &i.version {
            Some(v) => format!(
                "  {}",
                sunrise_launcher::developer_install::label(v).unwrap_or_else(|| v.clone())
            ),
            None => String::new(),
        };
        let played = match i.last_played {
            Some(_) => format!("  {} played", instances::play_time(i.played)),
            None => String::new(),
        };
        println!(
            "{mark} {} ({}){pinned}{group}{played}\n    {}",
            i.name,
            i.id,
            i.home.display()
        );
    };
    let changed = |i: instances::Instance| {
        let selected = i.id == instances::selected().id;
        match as_json {
            true => json(value(&i, selected)),
            false => line(&i, selected),
        }
    };
    match action {
        InstanceCmd::List => {
            let selected = instances::selected().id;
            let all = instances::list();
            if as_json {
                json(all.iter().map(|i| value(i, i.id == selected)).collect());
            } else {
                all.iter().for_each(|i| line(i, i.id == selected));
            }
        }
        InstanceCmd::Create { name, copy } => changed(instances::create(&name, copy.as_deref())?),
        InstanceCmd::Delete { instance } => {
            let gone = instances::delete(&instance)?;
            match as_json {
                true => json(serde_json::json!({"deleted": gone.id, "trashed": gone.work})),
                false => println!("{} is in the Trash.", gone.name),
            }
        }
        InstanceCmd::Rename { instance, name } => {
            changed(instances::edit(&instance, |m| m.name = Some(name))?)
        }
        InstanceCmd::Icon { instance, icon } => {
            let icon = (icon != "none").then(|| Icon::parse(&icon)).transpose()?;
            changed(instances::edit(&instance, |m| m.icon = icon)?)
        }
        InstanceCmd::Group { instance, group } => {
            changed(instances::edit(&instance, |m| m.group = group)?)
        }
        InstanceCmd::Select { instance } => changed(instances::select(&instance)?),
        InstanceCmd::Version { instance, version } => {
            let pin = match version.as_str() {
                "latest" => None,
                tag => Some(backend::resolve_version(Some(tag))?),
            };
            let profile = instances::find(&instance)?;
            let _lock = sunrise_launcher::browser::lock_home(&profile.home)
                .map_err(|error| error.to_string())?;
            sunrise_launcher::browser::installed::recover_game_install(&profile.home)?;
            changed(instances::edit(&profile.id, |m| m.version = pin)?)
        }
        InstanceCmd::Launch {
            instance,
            pre_launch,
            wrapper,
            env,
            gamemode,
            mangohud,
            dedicated_gpu,
            post_exit,
        } => {
            use sunrise_launcher::launch;
            let env = env.as_deref().map(launch::parse_env).transpose()?;
            let [gamemode, mangohud, dedicated_gpu] =
                [gamemode, mangohud, dedicated_gpu].map(|v| v.as_deref().map(on_off));
            let [gamemode, mangohud, dedicated_gpu] = [
                gamemode.transpose()?,
                mangohud.transpose()?,
                dedicated_gpu.transpose()?,
            ];
            let found = launch::Found::here();
            for (on, has, name) in [
                (gamemode, found.gamemode, "GameMode (gamemoderun)"),
                (mangohud, found.mangohud, "MangoHud"),
                (dedicated_gpu, found.two_gpus, "A second graphics card"),
            ] {
                if on == Some(true) && !has {
                    eprintln!(
                        "{name} isn't on this computer: the switch does nothing until it is."
                    );
                }
            }
            let changing = pre_launch.is_some()
                || wrapper.is_some()
                || post_exit.is_some()
                || env.is_some()
                || gamemode.is_some()
                || mangohud.is_some()
                || dedicated_gpu.is_some();
            let i = match changing {
                false => instances::find(&instance)?,
                true => instances::edit(&instance, |m| {
                    if let Some(p) = pre_launch {
                        m.launch.pre_launch = p.trim().to_string();
                    }
                    if let Some(w) = wrapper {
                        m.launch.wrapper = w.trim().to_string();
                    }
                    if let Some(p) = post_exit {
                        m.launch.post_exit = p.trim().to_string();
                    }
                    if let Some(e) = env {
                        m.launch.env = e;
                    }
                    if let Some(on) = gamemode {
                        m.launch.gamemode = on;
                    }
                    if let Some(on) = mangohud {
                        m.launch.mangohud = on;
                    }
                    if let Some(on) = dedicated_gpu {
                        m.launch.dedicated_gpu = on;
                    }
                })?,
            };
            let l = &i.launch;
            if as_json {
                json(serde_json::json!({
                    "instance": i.id, "pre_launch": l.pre_launch, "wrapper": l.wrapper, "env": l.env,
                    "post_exit": l.post_exit,
                    "gamemode": l.gamemode, "mangohud": l.mangohud, "dedicated_gpu": l.dedicated_gpu,
                    "found": {
                        "gamemode": found.gamemode, "mangohud": found.mangohud,
                        "two_gpus": found.two_gpus, "nvidia": found.nvidia,
                    },
                }));
            } else {
                println!(
                    "{}: {}",
                    i.name,
                    l.summary().unwrap_or("nothing around the game".into())
                );
                println!("  pre-launch: {}", l.pre_launch);
                println!("  wrapper:    {}", l.wrapper);
                println!("  env:        {}", launch::env_line(&l.env));
                println!("  post-exit:  {}", l.post_exit);
                let yes = |b: bool| if b { "on" } else { "off" };
                for (name, on, has) in [
                    ("gamemode:", l.gamemode, found.gamemode),
                    ("mangohud:", l.mangohud, found.mangohud),
                    ("gpu:", l.dedicated_gpu, found.two_gpus),
                ] {
                    let note = if has { "" } else { " (not on this computer)" };
                    println!("  {name:<11} {}{note}", yes(on));
                }
            }
        }
        InstanceCmd::Shortcut { instance, remove } => {
            use sunrise_launcher::shortcut;
            let i = match instance {
                Some(name) => instances::find(&name)?,
                None => instances::selection()?,
            };
            let paths = match remove {
                false => shortcut::make(&backend::update_target(), &i.id, &i.name)?,
                true => shortcut::remove(&i.id)?,
            };
            if as_json {
                json(serde_json::json!({ "instance": i.id, "paths": paths }));
            } else if remove && paths.is_empty() {
                println!("{} has no shortcut.", i.name);
            } else {
                let did = if remove { "Moved to the trash" } else { "Made" };
                for path in paths {
                    println!("{did}: {}", path.display());
                }
            }
        }
        InstanceCmd::Import { file, instance } => {
            use sunrise_launcher::import::{self, Dropped};
            match import::what(&file)? {
                Dropped::Instance => {
                    let i = import::import_instance(&file, &|line, _| eprintln!("{line}"))?;
                    if as_json {
                        json(value(&i, false));
                    } else {
                        println!(
                            "Made the profile {} in {}: sunrise-launcher instance select \"{}\"",
                            i.name,
                            i.home.display(),
                            i.name
                        );
                    }
                }
                // A game build is its own command: it installs a version
                // rather than touching an instance, and takes a `--version` to
                // say which release's game files it plays with.
                Dropped::Build => {
                    let tag = sunrise_launcher::build_import::import(&file, None)?;
                    if as_json {
                        json(serde_json::json!({ "version": tag }));
                    } else {
                        println!("Imported the build as {tag}, and selected it.");
                    }
                }
                Dropped::Pack(_) => {
                    let i = match instance {
                        Some(name) => instances::find(&name)?,
                        None => instances::selection()?,
                    };
                    let p = sunrise_launcher::packs::add(&i.home, &file)?;
                    println!(
                        "Added {} to {}: turn it on with sunrise-launcher packs on",
                        p.title, i.name
                    );
                }
                Dropped::DataPack => {
                    return Err(format!(
                        "{} is a data pack: put it into a world with sunrise-launcher worlds \
                         datapack-add <world> <file>",
                        file.display()
                    ));
                }
            }
        }
        InstanceCmd::Export {
            instance,
            world,
            out,
        } => {
            use sunrise_launcher::export;
            let i = match instance {
                Some(name) => instances::find(&name)?,
                None => instances::selection()?,
            };
            let worlds = world
                .iter()
                .map(|w| sunrise_launcher::worlds::find(&i.home, w).map(|w| w.folder))
                .collect::<Result<Vec<_>, _>>()?;
            let out = match out {
                Some(out) => out,
                None => export::free_path(
                    &std::env::current_dir().map_err(|e| e.to_string())?,
                    &i.name,
                ),
            };
            let done = export::export(&i.home, &i.name, &worlds, &out)?;
            if as_json {
                json(serde_json::json!({
                    "path": done.path, "copied": done.copied,
                    "worlds": done.worlds,
                }));
            } else {
                println!(
                    "Exported {} to {}: {} files{}.",
                    i.name,
                    done.path.display(),
                    done.copied,
                    match done.worlds.is_empty() {
                        true => String::new(),
                        false => format!(", worlds: {}", done.worlds.join(", ")),
                    }
                );
            }
        }
    }
    Ok(())
}

fn game_packs(action: PacksCmd, as_json: bool) -> Result<(), String> {
    use sunrise_launcher::packs::{self, Kind};
    let home = |instance: Option<String>| -> Result<PathBuf, String> {
        Ok(match instance {
            Some(name) => instances::find(&name)?.home,
            None => instances::selection()?.home,
        })
    };
    let value = |p: &packs::Pack| {
        serde_json::json!({
            "kind": format!("{:?}", p.kind).to_lowercase(), "file": p.file, "title": p.title,
            "description": p.description, "enabled": p.enabled, "path": p.path,
        })
    };
    match action {
        PacksCmd::List { kind, instance } => {
            let home = home(instance)?;
            let kinds = match kind {
                Some(k) => vec![Kind::from(k)],
                None => vec![Kind::Resource, Kind::Shader],
            };
            let all: Vec<packs::Pack> = kinds
                .iter()
                .flat_map(|k| packs::list_with(&home, *k, packs::Pictures::None))
                .collect();
            if as_json {
                json(all.iter().map(value).collect());
            }
            for k in kinds.iter().filter(|_| !as_json) {
                println!("{}:", k.folder());
                let these: Vec<_> = all.iter().filter(|p| p.kind == *k).collect();
                if these.is_empty() {
                    println!("    none");
                }
                for p in these {
                    let on = if p.enabled { "on " } else { "off" };
                    match p.title == p.file {
                        true => println!("  {on} {}", p.file),
                        false => println!("  {on} {} ({})", p.title, p.file),
                    }
                }
            }
        }
        PacksCmd::Add { file, instance } => {
            let pack = packs::add(&home(instance)?, &file)?;
            match as_json {
                true => json(value(&pack)),
                false => println!("Added {} to {} (off).", pack.file, pack.kind.folder()),
            }
        }
        PacksCmd::On {
            pack,
            kind,
            instance,
        } => set_pack(&home(instance)?, kind.into(), &pack, true)?,
        PacksCmd::Off {
            pack,
            kind,
            instance,
        } => set_pack(&home(instance)?, kind.into(), &pack, false)?,
        PacksCmd::Remove {
            pack,
            kind,
            instance,
        } => {
            let home = home(instance)?;
            let p = packs::find(&home, kind.into(), &pack)?;
            packs::remove(&home, &p)?;
            println!("{} is in the trash.", p.file);
        }
    }
    Ok(())
}

fn set_pack(
    home: &std::path::Path,
    kind: sunrise_launcher::packs::Kind,
    query: &str,
    on: bool,
) -> Result<(), String> {
    use sunrise_launcher::packs;
    let pack = packs::find(home, kind, query)?;
    packs::set_enabled(home, &pack, on)?;
    println!("{} is {}.", pack.title, if on { "on" } else { "off" });
    Ok(())
}

fn screenshots(action: ScreenshotsCmd, as_json: bool) -> Result<(), String> {
    use sunrise_launcher::screenshots;
    let home = |instance: Option<String>| -> Result<PathBuf, String> {
        Ok(match instance {
            Some(name) => instances::find(&name)?.home,
            None => instances::selection()?.home,
        })
    };
    match action {
        ScreenshotsCmd::List { instance } => {
            let home = home(instance)?;
            let shots = screenshots::list(&home);
            if as_json {
                json(
                    shots
                        .iter()
                        .map(|s| {
                            let millis = s
                                .taken
                                .duration_since(std::time::UNIX_EPOCH)
                                .map_or(0, |d| d.as_millis() as u64);
                            serde_json::json!({ "name": s.name, "path": s.path, "taken_ms": millis, "size": s.size })
                        })
                        .collect(),
                );
            } else if shots.is_empty() {
                println!("No screenshots in {}", screenshots::folder(&home).display());
            }
            for s in shots.iter().filter(|_| !as_json) {
                println!("{}", s.path.display());
            }
        }
        ScreenshotsCmd::Delete {
            screenshot,
            instance,
        } => {
            let shot = screenshots::find(&home(instance)?, &screenshot)?;
            screenshots::delete(&shot)?;
            println!("{} is in the trash.", shot.name);
        }
    }
    Ok(())
}

fn multiplayer(action: MultiplayerCmd, as_json: bool) -> Result<(), String> {
    let home = |instance: Option<String>| -> Result<PathBuf, String> {
        Ok(match instance {
            Some(name) => instances::find(&name)?.home,
            None => instances::selection()?.home,
        })
    };
    match action {
        MultiplayerCmd::List { instance } => {
            let home = home(instance)?;
            let servers = server_list::List::load(&home).servers();
            if as_json {
                json(
                    servers
                        .iter()
                        .map(|s| serde_json::json!({ "name": s.name, "address": s.ip, "icon": s.icon.is_some() }))
                        .collect(),
                );
            } else if servers.is_empty() {
                println!("No servers in {}", home.join("servers.dat").display());
            }
            for (i, s) in servers.iter().enumerate().filter(|_| !as_json) {
                println!("{}. {} ({})", i + 1, s.name, s.ip);
            }
        }
        MultiplayerCmd::Add {
            address,
            name,
            instance,
        } => {
            let home = home(instance)?;
            let mut list = server_list::List::load(&home);
            list.add(name.as_deref().unwrap_or(""), &address)?;
            list.save(&home)?;
            println!("Added {}.", address.trim());
        }
        MultiplayerCmd::Remove { server, instance } => {
            let home = home(instance)?;
            let mut list = server_list::List::load(&home);
            let (i, _) = server_list::find(&list, &server)?;
            let gone = list.remove(i)?;
            list.save(&home)?;
            println!("Removed {} ({}).", gone.name, gone.ip);
        }
    }
    Ok(())
}

fn worlds(action: WorldsCmd, as_json: bool) -> Result<(), String> {
    use sunrise_launcher::datapacks;
    use sunrise_launcher::worlds::{self, World};
    let home = |instance: Option<String>| -> Result<PathBuf, String> {
        Ok(match instance {
            Some(name) => instances::find(&name)?.home,
            None => instances::selection()?.home,
        })
    };
    let value = |w: &World| {
        let millis = w
            .last_played
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        serde_json::json!({
            "folder": w.folder, "name": w.name, "mode": w.mode, "version": w.version,
            "last_played_ms": millis, "size": w.size, "path": w.path, "icon": w.icon,
        })
    };
    match action {
        WorldsCmd::List { instance } => {
            let home = home(instance)?;
            let all = worlds::list(&home);
            if as_json {
                json(all.iter().map(value).collect());
            } else if all.is_empty() {
                println!("No worlds in {}", home.join("saves").display());
            }
            for w in all.iter().filter(|_| !as_json) {
                let version = w.version.as_deref().unwrap_or("unknown version");
                let mb = w.size as f64 / 1_000_000.0;
                println!(
                    "{} ({})\n    {}, {version}, {mb:.1} MB",
                    w.name, w.folder, w.mode
                );
            }
        }
        WorldsCmd::Backup { world, instance } => {
            let home = home(instance)?;
            let zip = worlds::backup(&home, &worlds::find(&home, &world)?)?;
            match as_json {
                true => json(serde_json::json!({ "backup": zip })),
                false => println!("{}", zip.display()),
            }
        }
        WorldsCmd::Copy { world, instance } => {
            let home = home(instance)?;
            let copy = worlds::copy(&worlds::find(&home, &world)?)?;
            match as_json {
                true => json(value(&copy)),
                false => println!("Copied to {} ({})", copy.name, copy.folder),
            }
        }
        WorldsCmd::Delete { world, instance } => {
            let home = home(instance)?;
            let w = worlds::find(&home, &world)?;
            worlds::delete(&w)?;
            println!("{} is in the trash.", w.name);
        }
        WorldsCmd::DatapackList { world, instance } => {
            let w = worlds::find(&home(instance)?, &world)?;
            let packs = datapacks::list(&w.path);
            if as_json {
                json(
                    packs
                        .iter()
                        .map(|p| serde_json::json!({ "file": p.file, "description": p.description, "enabled": p.enabled, "path": p.path }))
                        .collect(),
                );
            } else if packs.is_empty() {
                println!("No data packs in {}", w.name);
            }
            for p in packs.iter().filter(|_| !as_json) {
                let on = if p.enabled { "on " } else { "off" };
                println!("  {on} {}  {}", p.file, p.description);
            }
        }
        WorldsCmd::DatapackAdd {
            world,
            file,
            instance,
        } => {
            let w = worlds::find(&home(instance)?, &world)?;
            let p = datapacks::add(&w.path, &file)?;
            println!(
                "Added {} to {}: the game turns it on when the world opens.",
                p.file, w.name
            );
        }
        WorldsCmd::DatapackRemove {
            world,
            pack,
            instance,
        } => {
            let w = worlds::find(&home(instance)?, &world)?;
            let p = datapacks::find(&w.path, &pack)?;
            datapacks::remove(&w.path, &p)?;
            println!("{} is in the trash.", p.file);
        }
    }
    Ok(())
}

fn modpack(action: ModpackCmd, as_json: bool) -> Result<i32, String> {
    use sunrise_launcher::browser::sunrise;
    let work = |pack: &str| {
        let work = sunrise::pack::packs_dir().join(pack);
        if sunrise::pack::is_pack(&work) {
            Ok(work)
        } else {
            Err(format!("{pack} isn't an installed modpack."))
        }
    };
    match action {
        ModpackCmd::List => {
            let registry = sunrise::pack::list();
            if registry.is_empty() && !as_json {
                println!("No modpack installed yet: sunrise-launcher mods install <modpack> --source sunrise");
            }
            for p in &registry {
                if as_json {
                    json(
                        serde_json::json!({"pack": p.id, "name": p.name, "version": p.version, "source": "sunrise", "mods": p.mods}),
                    );
                } else {
                    println!(
                        "{} {}  ({})\n  {}",
                        p.name,
                        p.version,
                        p.id,
                        p.mods.join(", ")
                    );
                }
            }
        }
        ModpackCmd::Play {
            pack,
            version,
            allow_jar_downloads,
        } => {
            let tag = backend::resolve_version(version.as_deref())?;
            let mut cmd = backend::client(&tag)?;
            sunrise::pack::apply(&mut cmd, &work(&pack)?)?;
            let launch = Default::default();
            for notice in sunrise::before_play(&mut cmd, &tag, &launch)? {
                eprintln!("{notice}");
            }
            sunrise_launcher::jars::prepare_launch(
                &mut cmd,
                &launch,
                sunrise_launcher::jars::Permission::headless(allow_jar_downloads),
            )
            .map_err(|error| error.to_string())?;
            return attached(cmd);
        }
        ModpackCmd::Remove { pack } => sunrise::pack::remove(&work(&pack)?)?,
    }
    Ok(0)
}

#[cfg(test)]
mod storage_tests;
#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn attached_preserves_real_process_codes_and_unix_signals() {
        let mut codes = vec![0, 37];
        #[cfg(unix)]
        codes.push(143);
        #[cfg(windows)]
        codes.push(0xFEDC_BA98u32 as i32);
        for code in codes {
            let root = tempfile::tempdir().unwrap();
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "cli::tests::attached_status_actor",
                    "--nocapture",
                ])
                .env("SUNRISE_CLI_STATUS_ROLE", "wrapper")
                .env("SUNRISE_CLI_STATUS_CODE", code.to_string())
                .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
                .env("SUNRISE_NO_AUDIO", "1")
                .current_dir(root.path())
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::inherit());
            for (key, folder) in [
                ("SUNRISE_HOME", "logs"),
                ("SUNRISE_LAUNCHER_HOME", "launcher"),
                ("XDG_DATA_HOME", "xdg-data"),
                ("XDG_CONFIG_HOME", "xdg-config"),
                ("SUNRISE_LAUNCHER_TRASH", "trash"),
                ("SUNRISE_LAUNCHER_CLIPBOARD", "clipboard"),
                ("SUNRISE_LAUNCHER_DESKTOP", "desktop"),
            ] {
                command.env(key, root.path().join(folder));
            }
            assert_eq!(command.status().unwrap().code(), Some(code));
        }
    }

    #[test]
    fn attached_status_actor() {
        let Ok(role) = std::env::var("SUNRISE_CLI_STATUS_ROLE") else {
            return;
        };
        let code: i32 = std::env::var("SUNRISE_CLI_STATUS_CODE")
            .unwrap()
            .parse()
            .unwrap();
        if role == "child" {
            assert!(std::env::var_os(sunrise_launcher_smod::holder::ENV).is_none());
            assert!(std::env::var_os(sunrise_launcher_smod::holder::ENTRY).is_none());
            use std::io::IsTerminal;
            let shape = format!(
                "{}:{}:{}",
                std::io::stdin().is_terminal(),
                std::io::stdout().is_terminal(),
                std::io::stderr().is_terminal()
            );
            assert_eq!(shape, std::env::var("SUNRISE_CLI_STDIO_SHAPE").unwrap());
            #[cfg(unix)]
            if code == 143 {
                unsafe {
                    libc::signal(libc::SIGTERM, libc::SIG_DFL);
                    libc::raise(libc::SIGTERM);
                }
                panic!("SIGTERM did not terminate the actor");
            }
            std::process::exit(code);
        }
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "cli::tests::attached_status_actor",
                "--nocapture",
            ])
            .env("SUNRISE_CLI_STATUS_ROLE", "child");
        use std::io::IsTerminal;
        command.env(
            "SUNRISE_CLI_STDIO_SHAPE",
            format!(
                "{}:{}:{}",
                std::io::stdin().is_terminal(),
                std::io::stdout().is_terminal(),
                std::io::stderr().is_terminal()
            ),
        );
        assert_eq!(attached(command).unwrap(), code);
        std::process::exit(code);
    }

    #[test]
    fn completions_cover_each_shell_and_nested_commands() {
        for shell in [
            clap_complete::Shell::Bash,
            clap_complete::Shell::Zsh,
            clap_complete::Shell::Fish,
            clap_complete::Shell::Elvish,
            clap_complete::Shell::PowerShell,
        ] {
            let shell_name = shell.to_string();
            let args = ["sunrise-launcher", "completions", shell_name.as_str()].map(str::to_owned);
            let cli = Cli::try_parse_from(args).unwrap();
            assert!(matches!(cli.command, Some(Cmd::Completions { .. })));
            let mut output = Vec::new();
            clap_complete::generate(shell, &mut command(), "sunrise-launcher", &mut output);
            let script = String::from_utf8(output).unwrap();
            assert!(script.contains("sunrise-launcher"));
            assert!(script.contains("versions"));
            assert!(script.contains("multiplayer"));
        }
        assert!(Cli::try_parse_from(["sunrise-launcher", "completions"]).is_err());
        assert!(Cli::try_parse_from(["sunrise-launcher", "completions", "unknown"]).is_err());
    }

    /// The command line makes the same choices of look as Settings › Appearance does.
    #[test]
    fn settings_choose_the_look_as_appearance_does() {
        use sunrise_launcher::themes::{self, Accent};
        let mut settings = backend::Settings::parse("{}");
        set_setting(&mut settings, "theme", "nord").unwrap();
        assert_eq!(settings.theme.as_deref(), Some("nord"));
        assert_eq!(settings.accent, themes::NORD.accent.accent());
        set_setting(&mut settings, "accent", "#C2366F").unwrap();
        assert_eq!(settings.theme_accents["nord"].rgb(), 0xc2366f);
        set_setting(&mut settings, "theme", "default").unwrap();
        assert_eq!(settings.theme, None);
        assert_eq!(settings.accent.rgb(), 0xb4552f, "no saved theme is Dawn");
        set_setting(&mut settings, "theme", "nord").unwrap();
        assert_eq!(settings.accent.rgb(), 0xc2366f, "its colour is back");
        set_setting(&mut settings, "accent", "theme").unwrap();
        assert_eq!(settings.accent, themes::NORD.accent.accent());
        assert!(settings.theme_accents.is_empty());
        // "default" is still the violet, as it was before themes kept their accents.
        set_setting(&mut settings, "accent", "default").unwrap();
        assert_eq!(settings.accent, Accent::VIOLET);
        set_setting(&mut settings, "scenery", "off").unwrap();
        assert!(!settings.scenery());
        assert!(set_setting(&mut settings, "scenery", "maybe").is_err());
        set_setting(&mut settings, "scenery", "on").unwrap();
        assert!(settings.scenery());
        // What is shown says the theme painted and the one saved.
        let shown = settings_json(&settings);
        assert_eq!(
            (&shown["theme"], &shown["theme_choice"]),
            (&"nord".into(), &"nord".into())
        );
        assert_eq!(shown["scenery"], true);
        assert!(shown["themes_dir"].as_str().unwrap().ends_with("themes"));
    }

    #[test]
    fn panel_opacity_sets_a_clamped_percent_and_can_reset() {
        let mut settings = backend::Settings::default();
        assert!(settings_json(&settings)["panel_opacity"].is_null());
        let cli = Cli::try_parse_from(["sunrise-launcher", "settings", "panel-opacity", "0"]);
        assert!(cli.is_ok());
        set_setting(&mut settings, "panel-opacity", "0").unwrap();
        assert_eq!(
            settings_json(&settings)["panel_opacity"],
            backend::PANEL_OPACITY_MIN
        );
        set_setting(&mut settings, "panel-opacity", "100").unwrap();
        assert_eq!(settings.panel_opacity, Some(backend::PANEL_OPACITY_MAX));
        for bad in ["-1", "101", "80.5", "opaque"] {
            assert!(set_setting(&mut settings, "panel-opacity", bad).is_err());
            assert_eq!(settings.panel_opacity, Some(backend::PANEL_OPACITY_MAX));
        }
        set_setting(&mut settings, "panel-opacity", "default").unwrap();
        assert!(settings_json(&settings)["panel_opacity"].is_null());
    }

    #[test]
    fn reduced_graphics_cli_sets_and_reads_the_shared_choice() {
        let mut settings = backend::Settings::parse(
            r#"{"blur":8.5,"reduce_motion":false,"cat":true,
                "scenery":false,"background":"background-0123456789ab.png"}"#,
        );
        assert_eq!(settings_json(&settings)["reduced_graphics"], false);
        for (value, on) in [("on", true), ("off", false)] {
            let cli =
                Cli::try_parse_from(["sunrise-launcher", "settings", "reduced-graphics", value])
                    .unwrap();
            let Some(Cmd::Settings {
                key: Some(key),
                value: Some(value),
            }) = cli.command
            else {
                panic!("expected settings key and value");
            };
            set_setting(&mut settings, &key, &value).unwrap();
            let before = serde_json::to_value(&settings).unwrap();
            assert!(set_setting(&mut settings, &key, "maybe").is_err());
            assert_eq!(serde_json::to_value(&settings).unwrap(), before);
            settings = backend::Settings::parse(&before.to_string());
            assert_eq!(settings_json(&settings)["reduced_graphics"], on);
            assert_eq!(settings.glass_blur(), if on { 0. } else { 8.5 });
            assert_eq!(settings.reduce_motion, Some(false));
            assert!(settings.show_cat());
            assert!(!settings.scenery());
            assert_eq!(
                settings.background.as_deref(),
                Some("background-0123456789ab.png")
            );
        }
    }

    #[test]
    fn settings_accepts_german_and_chinese() {
        use crate::i18n::Language;
        let mut settings = backend::Settings::default();
        for (code, language) in [("de", Language::German), ("zh", Language::Chinese)] {
            set_setting(&mut settings, "language", code).unwrap();
            assert_eq!(settings.language, Some(language));
        }
        // A wrong code lists every language the launcher has, not only the first few.
        let wrong = set_setting(&mut settings, "language", "xx").unwrap_err();
        assert!(
            wrong.starts_with("xx: say system or one of en, "),
            "{wrong}"
        );
        for code in ["ja", "pt-br", "th", "pl"] {
            assert!(
                wrong.contains(&format!(" {code},")) || wrong.ends_with(&format!(" {code}")),
                "{code}: {wrong}"
            );
        }
        set_setting(&mut settings, "language", "system").unwrap();
        assert_eq!(settings.language, None);
    }

    #[test]
    fn play_takes_an_account_for_one_launch() {
        let cli = Cli::try_parse_from(["sunrise-launcher", "play", "--account", "Alex"]).unwrap();
        match cli.command {
            Some(Cmd::Play { account, .. }) => assert_eq!(account.as_deref(), Some("Alex")),
            _ => panic!("not play"),
        }
    }

    #[test]
    fn play_takes_extra_arguments_that_start_with_hyphens() {
        let cli = Cli::try_parse_from([
            "sunrise-launcher",
            "play",
            "--arg",
            "--server-thread",
            "--arg",
            "40",
        ])
        .unwrap();
        match cli.command {
            Some(Cmd::Play { arg, .. }) => assert_eq!(arg, ["--server-thread", "40"]),
            _ => panic!("expected play"),
        }
    }

    #[test]
    fn game_args_takes_one_line_that_starts_with_hyphens() {
        let cli = Cli::try_parse_from([
            "sunrise-launcher",
            "game-args",
            "--server-thread --render-distance 40",
        ])
        .unwrap();
        match cli.command {
            Some(Cmd::GameArgs { args }) => {
                assert_eq!(
                    args.as_deref(),
                    Some("--server-thread --render-distance 40")
                );
            }
            _ => panic!("expected game-args"),
        }
    }

    #[test]
    fn mods_commands_name_their_instance() {
        let parse = |args: &[&str]| {
            let cli = Cli::try_parse_from([&["sunrise-launcher", "mods"], args].concat()).unwrap();
            match cli.command {
                Some(Cmd::Mods { action }) => action,
                _ => panic!("expected mods"),
            }
        };
        match parse(&["install", "fresh-animations", "--instance", "Skyblock"]) {
            Some(ModsCmd::Install {
                project, instance, ..
            }) => assert_eq!(
                (project.as_str(), instance.as_deref()),
                ("fresh-animations", Some("Skyblock"))
            ),
            _ => panic!("expected mods install"),
        }
        assert!(matches!(
            parse(&["list"]),
            Some(ModsCmd::List { instance: None })
        ));
        assert!(parse(&[]).is_none());
        match parse(&[
            "add-github",
            "https://github.com/a/b",
            "--mod",
            "zoom",
            "--all",
        ]) {
            Some(ModsCmd::AddGithub {
                url,
                mod_id,
                all,
                trust_native_code,
                ..
            }) => assert_eq!(
                (url.as_str(), mod_id.as_deref(), all, trust_native_code),
                ("https://github.com/a/b", Some("zoom"), true, false)
            ),
            _ => panic!("expected mods add-github"),
        }
        assert!(matches!(
            parse(&["update-github", "--check"]),
            Some(ModsCmd::UpdateGithub {
                check: true,
                id: None,
                ..
            })
        ));
        assert!(matches!(
            parse(&["search", "x", "--instance", "A"]),
            Some(ModsCmd::Search { instance: Some(i), .. }) if i == "A"
        ));
        // The Java mod check is gone.
        assert!(Cli::try_parse_from(["sunrise-launcher", "mods", "check", "x"]).is_err());
    }

    #[test]
    fn native_build_sync_is_explicit_and_never_selects_an_instance() {
        let parse = |args: &[&str]| {
            Cli::try_parse_from(
                [
                    &["sunrise-launcher", "native-build", "--sync-internals"],
                    args,
                ]
                .concat(),
            )
        };
        assert!(matches!(
            parse(&["mod", "--trust-native-code"]).unwrap().command,
            Some(Cmd::NativeBuild {
                sync_internals: true,
                folder: Some(_),
                ..
            })
        ));
        assert!(parse(&[]).is_err());
        for args in [
            ["mod", "--instance", "A"],
            ["mod", "--server-output", "out"],
            ["mod", "--profile", "developer"],
        ] {
            assert!(parse(&args).is_err());
        }
    }

    #[test]
    fn native_build_export_mode_needs_a_folder_and_refuses_an_instance() {
        let parse = |args: &[&str]| {
            Cli::try_parse_from(
                [
                    &["sunrise-launcher", "native-build", "--trust-native-code"],
                    args,
                ]
                .concat(),
            )
        };
        // Export mode: a mod folder and an output folder.
        match parse(&["mods", "--server-output", "engine-out"])
            .unwrap()
            .command
        {
            Some(Cmd::NativeBuild {
                folder,
                server_output,
                instance,
                version,
                update,
                profile,
                ..
            }) => {
                assert_eq!(folder.as_deref(), Some(Path::new("mods")));
                assert_eq!(server_output.as_deref(), Some(Path::new("engine-out")));
                assert_eq!(instance, None);
                assert_eq!(version, None);
                assert!(!update);
                assert_eq!(profile, None);
            }
            _ => panic!("expected native-build"),
        }
        assert!(matches!(
            parse(&[
                "mods",
                "--server-output",
                "engine-out",
                "--allow-jar-downloads"
            ])
            .unwrap()
            .command,
            Some(Cmd::NativeBuild {
                allow_jar_downloads: true,
                ..
            })
        ));
        assert!(parse(&["mods", "--allow-jar-downloads"]).is_err());
        // Leaving mods out is for an instance's build; a server export switches nothing off.
        assert!(matches!(
            parse(&["--leave-out-mods"]).unwrap().command,
            Some(Cmd::NativeBuild {
                leave_out_mods: true,
                ..
            })
        ));
        assert!(parse(&["mods", "--server-output", "engine-out", "--leave-out-mods"]).is_err());
        // The mod folder is explicit in export mode: no default from an instance.
        assert!(parse(&["--server-output", "engine-out"]).is_err());
        // Export mode never touches an instance.
        assert!(parse(&["mods", "--server-output", "engine-out", "--instance", "A"]).is_err());
        // Export mode accepts an explicit version and profile; a version
        // and --update refuse each other, as in the game mode.
        match parse(&[
            "mods",
            "--server-output",
            "engine-out",
            "--version",
            "v2026.10.13",
            "--profile",
            "optimized",
        ])
        .unwrap()
        .command
        {
            Some(Cmd::NativeBuild {
                version,
                update,
                profile,
                ..
            }) => {
                assert_eq!(version.as_deref(), Some("v2026.10.13"));
                assert!(!update);
                assert_eq!(profile, Some(native_mods::Profile::Optimized));
            }
            _ => panic!("expected native-build"),
        }
        // --update on its own: the newest installed release or Nightly, never a native build.
        match parse(&["mods", "--server-output", "engine-out", "--update"])
            .unwrap()
            .command
        {
            Some(Cmd::NativeBuild { update, .. }) => assert!(update),
            _ => panic!("expected native-build"),
        }
        // A version refuses --update, as in the game mode.
        assert!(parse(&[
            "mods",
            "--server-output",
            "engine-out",
            "--version",
            "v2026.10.13",
            "--update"
        ])
        .is_err());
        assert!(parse(&["mods", "--allow-version-mismatch"]).is_err());
        // The game mode is unchanged: an instance build without --server-output.
        assert!(matches!(
            parse(&["--instance", "A"]).unwrap().command,
            Some(Cmd::NativeBuild { instance: Some(i), server_output: None, .. }) if i == "A"
        ));
    }

    #[test]
    fn export_mode_base_is_an_installed_release_not_a_native_build() {
        // The export mode picks the newest installed release or Nightly, never a native build
        // (`native-*`), and `--update` changes nothing when it already is the newest.
        let installed = vec![
            "26.3-0.1.1".to_string(),
            "26.3-0.1.0".to_string(),
            "native-2b03c1".to_string(),
        ];
        for update in [false, true] {
            assert_eq!(
                native_mods::base_for(None, &installed, update).as_deref(),
                Some("26.3-0.1.1")
            );
        }
        assert_eq!(native_mods::base_for(None, &[], false), None);
    }
}

#[cfg(test)]
#[path = "cli_catalog_tests.rs"]
mod catalog_tests;

#[path = "cli_native_permission.rs"]
mod native_permission_impl;
use native_permission_impl::{github_native, native_permission, registry_consent};

mod developer_builds;

mod settings_output;
use settings_output::settings_json;
