//! `play` and `server`: starting the game or a dedicated server in this terminal.
use super::*;

pub(super) fn server(
    dir: Option<PathBuf>,
    version: Option<String>,
    accept_eula: bool,
    allow_jar_downloads: bool,
) -> Result<i32, Error> {
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
    let (cmd, _runtime) = backend::prepare_server_command(&tag, &dir, cmd)?;
    attached_prepared(cmd).map_err(Error::from)
}

// `play`'s options, read by clap as they were from the variant's own fields.
#[derive(clap::Args)]
pub(super) struct PlayArgs {
    /// Allow missing original Java asset/data jars to be downloaded from Modrinth
    #[arg(long)]
    pub(super) allow_jar_downloads: bool,
    /// The profile to play (its name or id; default: the selected one)
    #[arg(long)]
    pub(super) instance: Option<String>,
    /// Open or create this world (a folder under saves) at once
    #[arg(long)]
    pub(super) world: Option<String>,
    /// Creative mode for a new world
    #[arg(long)]
    pub(super) creative: bool,
    /// Join a friend's world with its invite code
    #[arg(long)]
    pub(super) join: Option<String>,
    /// Join this server at once: an address, or a server of the list by name or position
    #[arg(long, conflicts_with_all = ["world", "join"])]
    pub(super) server: Option<String>,
    /// An extra argument for the game, passed through as-is (repeatable), for example
    /// --server-thread
    #[arg(long = "arg", value_name = "ARG", allow_hyphen_values = true)]
    pub(super) arg: Vec<String>,
    /// Play this launch as this signed-in account (its name or UUID), without changing
    /// the one `account select` picked: several games can play as several accounts at once
    #[arg(long, value_name = "WHO")]
    pub(super) account: Option<String>,
}

pub(super) fn client(args: PlayArgs) -> Result<i32, Error> {
    let PlayArgs {
        allow_jar_downloads,
        instance,
        world,
        creative,
        join,
        server,
        arg,
        account,
    } = args;
    let instance = instances::for_play(instance.as_deref())?;
    use sunrise_launcher::{jars, managed_play};
    let accounts = Accounts::load();
    let account = account
        .as_deref()
        .map(|who| {
            accounts.find(who).ok_or_else(|| {
                format!("No signed-in account {who}. See `sunrise-launcher account`.")
            })
        })
        .transpose()?;
    let selected = account.or_else(|| accounts.selected());
    let add_arguments = game_arguments(join, world, server, creative, arg, &instance.home);
    let jar_permission = jars::Permission::headless(allow_jar_downloads);
    let vars = instance.vars();
    let (started, played, finished) = {
        let start = managed_play::Client {
            instance: &instance,
            accepted_account: selected.map(|a| a.uuid.as_str()),
            explicit_account: account.is_some(),
            piped: false,
            registry: sunrise_launcher::browser::sunrise::v2::current_index,
        }
        .start(
            || managed_play::headless_first(&instance),
            |cmd| {
                backend::append_game_args(cmd);
                add_arguments(cmd)
            },
            |cmd| jars::prepare_for_command(cmd, jar_permission).map_err(|e| e.to_string()),
            &native_mods::Cancellation::default(),
            |progress| match progress {
                managed_play::Progress::Prepare => backend::play_step("preparing the game's files"),
                managed_play::Progress::SignIn => backend::play_step("reading the sign-in"),
                managed_play::Progress::Line(line) => eprintln!("{line}"),
                _ => {}
            },
        );
        let mut game = match start {
            Ok(game) => game,
            Err(managed_play::StartError::Hook(
                error @ sunrise_launcher::launch::PreLaunchError::Exit(status),
            )) => {
                eprintln!("{error}");
                return attached_status(status).map_err(Error::from);
            }
            Err(error) => return Err(error.into()),
        };
        for notice in game.registry_notices.iter().chain(&game.notice) {
            eprintln!("{notice}");
        }
        let started = std::time::Instant::now();
        let status = game
            .child
            .wait()
            .map_err(sunrise_launcher::process::child::unconfirmed_exit)?;
        game.runtime.game_exited();
        (started, attached_status(status), game.instance)
    };
    if let Err(error) = instances::finish_play(&finished, started.elapsed(), || {
        after_exit(&finished, &vars)
    }) {
        eprintln!(
            "{}",
            crate::trf("Play time not saved: {error}", &[("error", &error)])
        );
    }
    played.map_err(Error::from)
}

/// What a run adds to the game's command line: a friend's world, a world, a server of the list
/// (looked up in `home`), creative mode and the player's own arguments.
fn game_arguments(
    join: Option<String>,
    world: Option<String>,
    server: Option<String>,
    creative: bool,
    arg: Vec<String>,
    home: &Path,
) -> impl FnOnce(&mut Command) -> Result<(), String> + '_ {
    move |cmd: &mut Command| {
        if let Some(code) = join {
            backend::join_friend(cmd, &code)?;
        }
        if let Some(world) = world {
            cmd.arg("--world").arg(world);
        }
        if let Some(server) = server {
            let list = server_list::List::load(home);
            let ip = server_list::find(&list, &server).map_or(server, |(_, s)| s.ip);
            cmd.arg("--server").arg(ip);
        }
        if creative {
            cmd.arg("--creative");
        }
        cmd.args(arg);
        Ok(())
    }
}
