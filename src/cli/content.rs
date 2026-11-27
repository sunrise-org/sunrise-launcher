//! `worlds …`, `packs …` and `screenshots …`: a profile's worlds and data packs, resource and shader packs, and screenshots.
use super::*;

#[derive(Subcommand)]
pub(super) enum WorldsCmd {
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
    /// Add a world to the profile: a world folder, or a .zip of one (a taken name gets " (2)")
    Import {
        /// A folder with a level.dat, or a zip holding one
        path: PathBuf,
        #[arg(long)]
        instance: Option<String>,
    },
    /// Change a world's name in the game's list (its folder keeps its name)
    Rename {
        world: String,
        /// The new name
        name: String,
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

pub(super) fn worlds(action: WorldsCmd, as_json: bool) -> Result<(), Error> {
    use sunrise_launcher::datapacks;
    use sunrise_launcher::worlds::{self, World};
    let profile = |instance: Option<String>| match instance {
        Some(name) => instances::find(&name),
        None => instances::selection(),
    };
    let home = |instance| profile(instance).map(|profile| profile.home);
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
            let profile =
                sunrise_launcher::managed_play::Profile::open_selected(&profile(instance)?)?;
            let home = profile.home();
            let zip = worlds::backup(home, &worlds::find(home, &world)?)?;
            match as_json {
                true => json(serde_json::json!({ "backup": zip })),
                false => println!("{}", zip.display()),
            }
        }
        WorldsCmd::Copy { world, instance } => {
            let copy = worlds::copy(&profile(instance)?, &world)?;
            match as_json {
                true => json(value(&copy)),
                false => println!("Copied to {} ({})", copy.name, copy.folder),
            }
        }
        WorldsCmd::Import { path, instance } => {
            let added = worlds::import(&profile(instance)?, &path)?;
            match as_json {
                true => json(value(&added)),
                false => println!("Added {} ({})", added.name, added.folder),
            }
        }
        WorldsCmd::Rename {
            world,
            name,
            instance,
        } => {
            let renamed = worlds::rename(&profile(instance)?, &world, &name)?;
            match as_json {
                true => json(value(&renamed)),
                false => println!("Renamed to {} ({})", renamed.name, renamed.folder),
            }
        }
        WorldsCmd::Delete { world, instance } => {
            let w = worlds::delete(&profile(instance)?, &world)?;
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
            let profile =
                sunrise_launcher::managed_play::Profile::open_selected(&profile(instance)?)?;
            let w = worlds::find(profile.home(), &world)?;
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
            let profile =
                sunrise_launcher::managed_play::Profile::open_selected(&profile(instance)?)?;
            let w = worlds::find(profile.home(), &world)?;
            let p = datapacks::find(&w.path, &pack)?;
            datapacks::remove(&w.path, &p, None)?;
            println!("{} is in the trash.", p.file);
        }
    }
    Ok(())
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub(super) enum PackKindArg {
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
pub(super) enum PacksCmd {
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

pub(super) fn game_packs(action: PacksCmd, as_json: bool) -> Result<(), Error> {
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

pub(super) fn set_pack(
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

#[derive(Subcommand)]
pub(super) enum ScreenshotsCmd {
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

pub(super) fn screenshots(action: ScreenshotsCmd, as_json: bool) -> Result<(), Error> {
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
