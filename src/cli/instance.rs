//! `instance …` and `modpack …`: making, changing, sharing and importing profiles, and Sunrise modpacks.
use super::*;
use sunrise_launcher::launch;

/// Which way `instance move` sends a profile.
#[derive(Clone, Copy, clap::ValueEnum)]
pub(super) enum Way {
    Up,
    Down,
}

/// What `create --copy` brings along or leaves out; with none of these it copies everything but
/// the play time.
#[derive(clap::Args)]
pub(super) struct CopyFlags {
    /// Keep the copied profile's play time and when it last played
    #[arg(long, requires = "copy")]
    keep_play_time: bool,
    /// Leave out the worlds
    #[arg(long, requires = "copy")]
    no_worlds: bool,
    /// Leave out the game options (options.txt)
    #[arg(long, requires = "copy")]
    no_options: bool,
    /// Leave out the resource packs
    #[arg(long, requires = "copy")]
    no_resource_packs: bool,
    /// Leave out the screenshots
    #[arg(long, requires = "copy")]
    no_screenshots: bool,
    /// Leave out the shader packs
    #[arg(long, requires = "copy")]
    no_shader_packs: bool,
    /// Leave out the server list
    #[arg(long, requires = "copy")]
    no_servers: bool,
    /// Leave out the mods
    #[arg(long, requires = "copy")]
    no_mods: bool,
}

impl CopyFlags {
    fn options(&self) -> instances::CopyOptions {
        instances::CopyOptions {
            play_time: self.keep_play_time,
            worlds: !self.no_worlds,
            options: !self.no_options,
            resource_packs: !self.no_resource_packs,
            screenshots: !self.no_screenshots,
            shader_packs: !self.no_shader_packs,
            servers: !self.no_servers,
            mods: !self.no_mods,
        }
    }
}

#[derive(Subcommand)]
pub(super) enum InstanceCmd {
    /// The profiles, each with its game folder (* = the one Play starts)
    List,
    /// Show a profile's committed game build (defaults to the selected profile)
    Show { profile: Option<String> },
    /// Make a profile: empty but for your options and name, or a copy of another one's
    /// worlds, options and packs
    Create {
        name: String,
        /// The profile to copy (its name or id)
        #[arg(long)]
        copy: Option<String>,
        #[command(flatten)]
        leave: CopyFlags,
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
    /// Move a profile up or down in the sidebar, among the profiles of its group. Sunrise's
    /// own profile moves like the others
    Move {
        instance: String,
        #[arg(value_enum)]
        direction: Way,
    },
    /// Make Play start this profile
    Select { instance: String },
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
    Launch(LaunchArgs),
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

/// Runs the profile's post-exit command in this terminal, if it has one, and waits for it.
pub(super) fn after_exit(
    instance: &instances::Instance,
    vars: &[(&'static str, std::ffi::OsString)],
) {
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

pub(super) fn instance(action: InstanceCmd, as_json: bool) -> Result<(), Error> {
    use instances::Icon;
    let changed = |i: instances::Instance| {
        let selected = i.id == instances::selected().id;
        match as_json {
            true => json(value(&i, selected)),
            false => line(&i, selected),
        }
    };
    match action {
        InstanceCmd::List => list(as_json)?,
        InstanceCmd::Show { profile } => show(profile, as_json)?,
        InstanceCmd::Create { name, copy, leave } => changed(match copy {
            Some(from) => instances::create_copying(&name, &from, leave.options())?,
            None => instances::create(&name, None)?,
        }),
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
        InstanceCmd::Move {
            instance,
            direction,
        } => {
            instances::move_profile(&instance, matches!(direction, Way::Up))?;
            changed(instances::find(&instance)?)
        }
        InstanceCmd::Select { instance } => changed(instances::select(&instance)?),
        InstanceCmd::Launch(args) => launch(args, as_json)?,
        InstanceCmd::Shortcut { instance, remove } => shortcut(instance, remove, as_json)?,
        InstanceCmd::Import { file, instance } => import_file(file, instance, as_json)?,
        InstanceCmd::Export {
            instance,
            world,
            out,
        } => export_profile(instance, world, out, as_json)?,
    }
    Ok(())
}

#[derive(Subcommand)]
pub(super) enum ModpackCmd {
    /// The installed Sunrise modpacks (the default)
    List,
    /// Playing Sunrise modpacks is temporarily unavailable
    Play {
        pack: String,
        /// Allow missing original Java asset/data jars to be downloaded from Modrinth
        #[arg(long)]
        allow_jar_downloads: bool,
    },
    /// Removing Sunrise modpacks is temporarily unavailable
    Remove { pack: String },
}

pub(super) fn modpack(action: ModpackCmd, as_json: bool) -> Result<i32, Error> {
    use sunrise_launcher::browser::sunrise;
    match action {
        ModpackCmd::List => {
            let registry = sunrise::pack::list();
            if registry.is_empty() && !as_json {
                println!("{}", tr("No Sunrise modpacks are installed."));
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
        ModpackCmd::Play { .. } => return Err(sunrise::pack::play_unavailable().into()),
        ModpackCmd::Remove { .. } => return Err(sunrise::pack::remove_unavailable().into()),
    }
    Ok(0)
}

// `instance launch`'s options, read by clap as they were from the variant's own fields.
#[derive(clap::Args)]
pub(super) struct LaunchArgs {
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
}

fn value(i: &instances::Instance, selected: bool) -> serde_json::Value {
    serde_json::json!({
        "id": i.id, "name": i.name, "kind": format!("{:?}", i.kind).to_lowercase(),
        "home": i.home, "icon": i.icon, "group": i.group, "selected": selected,
        "version": i.version, "version_label": i.version.as_deref().and_then(sunrise_launcher::developer_install::label), "played_secs": i.played.as_secs(),
        "last_played_ms": i.last_played.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64),
    })
}

fn line(i: &instances::Instance, selected: bool) {
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
}

fn list(as_json: bool) -> Result<(), Error> {
    let selected = instances::selected().id;
    let all = instances::list();
    if as_json {
        json(all.iter().map(|i| value(i, i.id == selected)).collect());
    } else {
        all.iter().for_each(|i| line(i, i.id == selected));
    }
    Ok(())
}

fn show(profile: Option<String>, as_json: bool) -> Result<(), Error> {
    use sunrise_launcher::managed_play::Profile;
    let instance = instances::read_profile(profile.as_deref())?;
    let active = match instance.home.try_exists().map_err(|e| e.to_string())? {
        true => Profile::open(&instance.home, None)?.active()?,
        false => None,
    };
    let channel = match instance.channel {
        instances::Channel::Stable => "stable",
        instances::Channel::Nightly => "nightly",
        instances::Channel::Dev => "dev",
    };
    if as_json {
        json(serde_json::json!({
            "profile": instance.id,
            "build": active.as_ref().map(|a| a.base()),
            "image": active.as_ref().map(|a| a.executable()),
            "generation": active.as_ref().map(|a| a.generation()),
            "channel": channel,
            "pinned": instance.version,
        }));
    } else {
        // A profile has no build of its own until it is first played. Say what it follows,
        // as Home's picker does, rather than a bare "Not initialized" beside a window that
        // shows a build for it.
        let line = match (&active, &instance.version) {
            (Some(active), _) => trf("Profile game build: {build}", &[("build", active.base())]),
            (None, Some(pin)) => trf(
                "Profile game build: not played yet; it stays on {build}",
                &[("build", pin)],
            ),
            (None, None) => trf(
                "Profile game build: not played yet; it follows the newest {channel} build",
                &[(
                    "channel",
                    match instance.channel {
                        instances::Channel::Stable => tr("Stable"),
                        instances::Channel::Nightly => tr("Nightly"),
                        instances::Channel::Dev => tr("Dev"),
                    },
                )],
            ),
        };
        println!("{line}");
    }
    Ok(())
}

fn launch(args: LaunchArgs, as_json: bool) -> Result<(), Error> {
    let LaunchArgs {
        instance,
        pre_launch,
        wrapper,
        post_exit,
        env,
        gamemode,
        mangohud,
        dedicated_gpu,
    } = args;
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
            eprintln!("{name} isn't on this computer: the switch does nothing until it is.");
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
    print_launch(&i, &found, as_json);
    Ok(())
}

fn shortcut(instance: Option<String>, remove: bool, as_json: bool) -> Result<(), Error> {
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
    Ok(())
}

fn import_file(file: PathBuf, instance: Option<String>, as_json: bool) -> Result<(), Error> {
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
            )
            .into());
        }
    }
    Ok(())
}

fn export_profile(
    instance: Option<String>,
    world: Vec<String>,
    out: Option<PathBuf>,
    as_json: bool,
) -> Result<(), Error> {
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
    Ok(())
}

fn print_launch(i: &instances::Instance, found: &launch::Found, as_json: bool) {
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
            launch::summary(l).unwrap_or("nothing around the game".into())
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
