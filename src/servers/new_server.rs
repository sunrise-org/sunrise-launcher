//! Creating a server: its world, its settings and a free port.

use super::*;

/// The world a new server starts with.
#[derive(Clone, Debug, PartialEq)]
pub enum World {
    /// A new world; an empty seed is a random one.
    New { seed: String },
    /// A copy of one of the player's worlds (its folder).
    Copy(PathBuf),
    /// A world from a file: a .zip of a world, or a world folder anywhere. It is extracted into
    /// the server, with the same checks as adding a world to a profile.
    Import(PathBuf),
}

/// What the Create dialog asks; [`NewServer::default`] alone makes a working server.
#[derive(Clone, Debug, PartialEq)]
pub struct NewServer {
    pub name: String,
    pub world: World,
    pub game_mode: String,
    pub difficulty: String,
    pub max_players: u32,
    /// Only listed players (and operators) may join.
    pub friends_only: bool,
    /// Check players' Minecraft accounts: they must sign in with Microsoft. On by default; an offline
    /// server is a security risk, so it is turned off only per server (`--offline`, or the server's
    /// settings page).
    pub check_accounts: bool,
    /// Who may join first (whitelisted), and the first of them becomes an operator.
    pub players: Vec<String>,
    pub eula: bool,
}

impl Default for NewServer {
    fn default() -> Self {
        Self {
            name: "My server".into(),
            world: World::New {
                seed: String::new(),
            },
            game_mode: "survival".into(),
            difficulty: "normal".into(),
            max_players: 10,
            friends_only: true,
            check_accounts: true,
            players: Vec::new(),
            eula: false,
        }
    }
}

/// A port nobody listens on, from `from` up, and not one of `taken`.
pub fn free_port(from: u16, taken: &[u16]) -> u16 {
    let bind = if cfg!(test) || std::env::var_os("SUNRISE_TEST_LOOPBACK").is_some_and(|v| v == "1")
    {
        "127.0.0.1"
    } else {
        "0.0.0.0"
    };
    (from..u16::MAX)
        .find(|p| !taken.contains(p) && std::net::TcpListener::bind((bind, *p)).is_ok())
        .unwrap_or(from)
}

/// The ports the launcher's other servers use: their game port and their web panel.
fn ports_taken() -> Vec<u16> {
    let mut out = Vec::new();
    for e in list() {
        if let Some(c) = config(&e.dir) {
            out.extend(c["server"]["port"].as_u64().map(|p| p as u16));
            out.extend(
                c["panel"]["bind"]
                    .as_str()
                    .and_then(|b| b.rsplit_once(':'))
                    .and_then(|(_, p)| p.parse::<u16>().ok()),
            );
        }
    }
    out
}

/// The player's worlds (`saves/` in the game folder `home`), the last played first: their name
/// as the game's list shows it and their folder.
pub fn player_worlds(home: &Path) -> Vec<(String, PathBuf)> {
    player_world_list(home)
        .into_iter()
        .map(|w| (w.name, w.path))
        .collect()
}

/// [`player_worlds`] with what the worlds list shows of each: its mode and when it was played.
pub fn player_world_list(home: &Path) -> Vec<worlds::World> {
    worlds::list(home)
        .into_iter()
        // Only a readable level.dat is a world to copy; the Worlds page also keeps folders with
        // just a level.dat_old or a damaged level.dat, so they can be backed up.
        .filter(|w| worlds::has_readable_level_dat(&w.path))
        .collect()
}

/// The `sunrise-server init` arguments for a new server.
pub fn init_args(spec: &NewServer, world: &str, port: u16) -> Vec<String> {
    let mut args: Vec<String> = vec!["init".into(), "--yes".into()];
    if spec.eula {
        args.push("--eula".into());
    }
    args.extend(["--world".into(), world.to_string()]);
    if let World::New { seed } = &spec.world {
        if !seed.trim().is_empty() {
            args.extend(["--seed".into(), seed.trim().to_string()]);
        }
    }
    args.extend(["--port".into(), port.to_string()]);
    args.push(if spec.friends_only {
        "--whitelist".into()
    } else {
        "--no-whitelist".into()
    });
    if !spec.check_accounts {
        args.push("--offline".into());
    }
    for p in &spec.players {
        args.extend(["--player".into(), p.clone()]);
    }
    if let Some(first) = spec.players.first() {
        args.extend(["--op".into(), first.clone()]);
    }
    args
}

/// The `sunrise-server config set` arguments that finish a new server.
pub fn settings_args(spec: &NewServer, panel_port: u16) -> Vec<String> {
    vec![
        "config".into(),
        "set".into(),
        format!("world.game_mode={}", spec.game_mode),
        format!("world.difficulty={}", spec.difficulty),
        format!("server.max_players={}", spec.max_players.clamp(1, 1000)),
        format!("server.motd={}", spec.name.trim()),
        format!("panel.bind=127.0.0.1:{panel_port}"),
    ]
}

/// Why no server can be created yet, known before the form is filled in: no server programs are
/// set up, and no installed version (`tag`) to set them up from.
pub fn create_blocked(tag: Option<&str>) -> Option<Problem> {
    let set_up = program().is_some() && game_files_ready();
    (!set_up && tag.is_none()).then(|| Problem::plain(tr(NEEDS_VERSION)))
}

/// What the Create dialog says when the programs can't be set up: Prepare only when preparing
/// is what's missing.
pub(super) fn set_up_problem(error: SetUpError) -> Problem {
    match error {
        SetUpError::NotPrepared(message) => Problem {
            message,
            fix: Some(Fix::Prepare),
        },
        SetUpError::Other(message) => Problem::plain(message),
    }
}

const NEEDS_VERSION: &str =
    crate::i18n::tr_key("Install a Sunrise game build first, then prepare it.");

/// Creates a server: its folder, the world (a copy when asked), `init` and the settings, then adds it to
/// the list. `tag` is the installed version whose programs and game files it uses when none are set up.
pub fn create(spec: &NewServer, tag: Option<&str>, log: &dyn Fn(String)) -> Result<Entry, Problem> {
    if !spec.eula {
        return Err(Problem {
            message: tr("Running a server needs your agreement to the Minecraft EULA.").into(),
            fix: Some(Fix::AgreeEula),
        });
    }
    if program().is_none() || !game_files_ready() {
        let tag = tag.ok_or_else(|| Problem::plain(tr(NEEDS_VERSION)))?;
        log(trf(
            "Setting up the server programs from Sunrise {tag}",
            &[("tag", tag)],
        ));
        set_up(tag).map_err(set_up_problem)?;
    }
    let name = if spec.name.trim().is_empty() {
        "My server".to_string()
    } else {
        spec.name.trim().to_string()
    };
    let folder = new_folder(&name);
    let dir = backend::server_root(&folder).map_err(Problem::plain)?;
    fs::create_dir_all(&dir).map_err(|e| Problem::plain(e.to_string()))?;
    let world = match &spec.world {
        World::New { .. } => "world".to_string(),
        World::Copy(from) => {
            let world = from
                .file_name()
                .map(|n| slug(&n.to_string_lossy()))
                .unwrap_or_else(|| "world".into());
            log(trf(
                "Copying the world {folder}",
                &[("folder", &from.display().to_string())],
            ));
            copy_world(from, &dir.join(&world)).map_err(|e| {
                let _ = fs::remove_dir_all(&dir);
                Problem::plain(trf(
                    "Couldn't copy the world: {error}",
                    &[("error", &e.to_string())],
                ))
            })?;
            world
        }
        World::Import(from) => {
            log(trf(
                "Importing the world {folder}",
                &[("folder", &from.display().to_string())],
            ));
            // Extracted beside where it ends up, so a world that fails the checks leaves no
            // half-made folder for the server to find.
            let staged = dir.join(".world-import");
            let result = worlds::extract_world(from, &staged).and_then(|base| {
                let world = slug(&base);
                fs::rename(&staged, dir.join(&world))
                    .map(|()| world)
                    .map_err(|e| e.to_string())
            });
            result.map_err(|e| {
                let _ = fs::remove_dir_all(&dir);
                Problem::plain(e)
            })?
        }
    };
    let taken = ports_taken();
    let port = free_port(25565, &taken);
    let panel = free_port(8130, &taken);
    let undo = |p: Problem| {
        let _ = fs::remove_dir_all(&dir);
        p
    };
    let init = init_args(spec, &world, port);
    log(tr("Writing the server's settings").into());
    run(&dir, &init.iter().map(String::as_str).collect::<Vec<_>>()).map_err(undo)?;
    let settings = settings_args(spec, panel);
    run(
        &dir,
        &settings.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .map_err(undo)?;
    let entry = Entry {
        name,
        dir: folder,
        host: None,
    };
    add(entry.clone()).map_err(Problem::plain)?;
    Ok(entry)
}

/// A copy of a world folder, without the lock a running game holds.
fn copy_world(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == "session.lock" {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type()?.is_dir() {
            copy_world(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
