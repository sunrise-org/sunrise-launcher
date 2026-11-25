//! `mods …`: browser-installed content and registry mods in a profile.
use super::*;

#[derive(Subcommand)]
pub(super) enum ModsCmd {
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
    /// (checksums checked); a data pack goes into the world `--world` names. Installing and
    /// updating Sunrise modpacks is temporarily unavailable
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
        /// Allow an update known to drop registered definitions that saved worlds may use
        #[arg(long)]
        confirm_world_loss: bool,
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
        /// Run this unreviewed native code (asked again on every run)
        #[arg(long)]
        trust_native_code: bool,
        /// Allow a replacement that could remove saved-world content when adding an already installed mod
        #[arg(long)]
        confirm_world_loss: bool,
    },
    /// Check native mods from GitHub, and update those installed from a release with a matching record
    #[command(
        after_help = "Before replacement, Sunrise verifies the installed archive against its recorded GitHub asset and shows its path and SHA-256. A commit install can still be checked with --check; to replace it, remove it explicitly and add the new version."
    )]
    UpdateGithub {
        /// One mod (default: all of them)
        id: Option<String>,
        /// Only say what is newer
        #[arg(long)]
        check: bool,
        /// The profile (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
        /// Run this unreviewed native code (asked again on every run)
        #[arg(long)]
        trust_native_code: bool,
        /// Allow proved loss of registered definitions (not needed for native mods)
        #[arg(long)]
        confirm_world_loss: bool,
    },
    /// Remove an installed mod archive or a pack the browser installed
    #[command(
        after_help = "For a mod archive, the review names its exact file and SHA-256 before removal, and any mods that need it are turned off rather than deleted."
    )]
    Remove {
        /// An installed mod ID or a project the browser installed
        project: String,
        /// The profile (default: the selected one)
        #[arg(long)]
        instance: Option<String>,
        /// Allow removal even if it could remove content from a saved world
        #[arg(long)]
        confirm_world_loss: bool,
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
pub(super) enum KindArg {
    Mod,
    Resource,
    Modpack,
    /// Sunrise's own shader packs (Sunrise's registry only)
    Shader,
    /// Data packs (Modrinth): install one into a world with `mods install <pack> --world <name>`
    Datapack,
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub(super) enum SourceArg {
    Modrinth,
    /// Sunrise's own registry of reviewed mods
    Sunrise,
}

pub(super) fn approve_mod_change(
    impact: &sunrise_launcher::browser::sunrise::ChangeImpact,
    confirmed: bool,
) -> bool {
    eprintln!("{}", impact.detail());
    let allowed = confirmed || !impact.risks_worlds();
    if !allowed {
        eprintln!(
            "{}",
            tr("Nothing was changed. Use --confirm-world-loss only if you accept this risk.")
        );
    }
    allowed
}

/// Commands for browser-installed content and registry mods in a selected profile.
pub(super) fn mods(action: ModsCmd, as_json: bool) -> Result<(), Error> {
    use sunrise_launcher::browser::SourceId;
    use sunrise_launcher::browser::{installed, modrinth::Modrinth, sunrise, Kind, Query, Source};
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
    let mutation_progress = |step| {
        if let sunrise::InstallProgress::Downloading(f) = step {
            line(tr("Downloading"), f);
        }
    };
    let mutation_hooks = || sunrise::InstallHooks {
        progress: &mutation_progress,
        continue_after_failure: &|_| false,
        cancellation: None,
    };
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
            if let SourceArg::Sunrise = source {
                if kind != Kind::Mod {
                    return Err(tr("Sunrise's registry lists only mods: use --kind mod.").into());
                }
                let catalog = sunrise::v2::index().map_err(|error| error.to_string())?;
                let text = text.join(" ").to_lowercase();
                let found: Vec<_> = catalog
                    .index
                    .projects
                    .iter()
                    .filter(|p| {
                        [&p.name, &p.slug, &p.summary]
                            .iter()
                            .any(|field| field.to_lowercase().contains(&text))
                    })
                    .collect();
                if as_json {
                    json(found.iter().map(|p| serde_json::json!({
                        "id": p.id, "slug": p.slug, "title": p.name, "summary": p.summary,
                        "authors": p.authors,
                    })).collect());
                } else {
                    for p in found {
                        println!(
                            "{}  ({})  {}\n  {}",
                            p.name,
                            p.slug,
                            p.authors.join(", "),
                            p.summary
                        );
                    }
                }
                return Ok(());
            }
            let query = Query {
                everything,
                game: game.clone(),
                ..Query::new(&text.join(" "), kind)
            };
            let found = Modrinth.search(&query)?;
            let datapack = kind == Kind::DataPack;
            if as_json {
                json(found.iter().map(|p| {
                    let support = p.support(&query.game);
                    serde_json::json!({
                        "id": p.id, "slug": p.slug, "title": p.title, "summary": p.summary,
                        "author": p.author, "downloads": p.downloads, "page": p.page,
                        "license": p.license, "support": support.label(), "reason": support.reason(),
                    })
                }).collect());
            } else {
                if datapack && !found.is_empty() {
                    println!("{}\n", sunrise_launcher::browser::datapack_support());
                }
                for p in &found {
                    let support = p.support(&query.game);
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
            if let SourceArg::Sunrise = source {
                if world.is_some() {
                    return Err(tr("Sunrise's registry lists only mods.").into());
                }
                if everything {
                    return Err(tr("Registry mods must fit the profile's game build.").into());
                }
                let catalog = sunrise::v2::index().map_err(|error| error.to_string())?;
                let p = catalog
                    .index
                    .project(&project)
                    .or_else(|| catalog.index.project_by_slug(&project))
                    .ok_or_else(|| {
                        trf("{mod} isn't in Sunrise's registry.", &[("mod", &project)])
                    })?;
                let progress_line = progress(p.name.clone());
                let target = sunrise::Target::of(&instance)?;
                let installed = sunrise::install_registry_v2(
                    &target,
                    target.identity(),
                    vec![p.id.clone()],
                    sunrise::V2InstallOptions::default(),
                    || sunrise_launcher::managed_play::headless_first(&instance),
                    sunrise::V2Hooks {
                        install: sunrise::InstallHooks {
                            progress: &|step| {
                                if let sunrise::InstallProgress::Downloading(f) = step {
                                    progress_line(f);
                                }
                            },
                            continue_after_failure: &|_| false,
                            cancellation: None,
                        },
                        choose_provider: None,
                        confirm_publisher_change: None,
                    },
                )
                .map_err(|error| error.to_string())?;
                let done = installed
                    .added
                    .iter()
                    .chain(installed.updated.iter().map(|(_, entry)| entry));
                if as_json {
                    json(done.map(entry_json).collect());
                } else {
                    for e in done {
                        println!("{} {}: {} ({})", e.title, e.number, e.file, e.checksum);
                    }
                }
                return Ok(());
            }
            let source = Modrinth;
            let p = source.project(&project)?;
            if let Some(world) = world {
                let profile = sunrise_launcher::managed_play::Profile::open_selected(&instance)?;
                let world = sunrise_launcher::worlds::find(profile.home(), &world)?;
                let done = installed::install_datapack(
                    &source,
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
                return Err(
                    format!("{} is a data pack: say which world with --world.", p.title).into(),
                );
            }
            if p.kind == Kind::Mod {
                return Err(
                    tr("Only Sunrise registry mods can be installed by the browser.").into(),
                );
            }
            let done = installed::install(&source, &p, everything, &home, &game, &line)?;
            if as_json {
                json(done.iter().map(entry_json).collect());
            }
            for e in done.iter().filter(|_| !as_json) {
                println!("{} {}: {} ({})", e.title, e.number, e.file, e.checksum);
            }
        }
        ModsCmd::Update {
            project,
            check,
            confirm_world_loss,
            ..
        } => {
            let identity = sunrise_launcher::profile_txn::Identity::of(&home);
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
                }
                .into());
            }
            let registry = |e: &installed::Entry| {
                instance.kind != instances::Kind::Pack
                    && e.source == SourceId::Sunrise
                    && e.kind == Kind::Mod
            };
            if !check {
                if project.is_some()
                    && chosen
                        .iter()
                        .all(|entry| entry.kind == Kind::Mod && !registry(entry))
                {
                    return Err(
                        tr("Only Sunrise registry mods can be updated by this command.").into(),
                    );
                }
                let ids = chosen
                    .iter()
                    .copied()
                    .filter(|e| registry(e))
                    .map(|e| e.project.clone())
                    .collect::<Vec<_>>();
                if !ids.is_empty() {
                    let requested = if project.is_some() {
                        sunrise::UpdateRequest::Strict(ids)
                    } else {
                        sunrise::UpdateRequest::Bulk(ids)
                    };
                    let target = sunrise::Target::of(&instance)?;
                    let result = sunrise::update_registry_v2(
                        &target,
                        identity?,
                        requested,
                        sunrise::V2InstallOptions::default(),
                        || sunrise_launcher::managed_play::headless_first(&instance),
                        sunrise::V2Hooks {
                            install: mutation_hooks(),
                            choose_provider: None,
                            confirm_publisher_change: None,
                        },
                        &|impact| approve_mod_change(impact, confirm_world_loss),
                    )
                    .map_err(|e| match e {
                        sunrise::InstallError::Cancelled => tr("Nothing was updated.").into(),
                        e => e.to_string(),
                    })?;
                    if as_json {
                        json(result.updated.iter().map(|(_, e)| entry_json(e)).collect());
                    } else if result.updated.is_empty() && result.added.is_empty() {
                        println!("{}", tr("No registry updates available."));
                    } else {
                        for e in result.updated.iter().map(|(_, e)| e).chain(&result.added) {
                            println!("{} {}: {}", e.title, e.number, e.file);
                        }
                    }
                }
            }
            // What `--check` compares a mod against: the v2 index on the profile's build.
            let checked = if check && chosen.iter().any(|e| e.kind == Kind::Mod) {
                let catalog = sunrise::v2::index().map_err(|error| error.to_string())?;
                let build = sunrise_launcher::managed_play::display_active(&home)?
                    .map(|active| active.base().to_owned());
                Some((catalog, build))
            } else {
                None
            };
            for e in chosen {
                use installed::PackUpdate;
                let status = if e.kind != Kind::Mod {
                    installed::update_pack(&home, e, &game, check, &line)?
                } else if let Some((catalog, build)) = &checked {
                    let Some(project) = sunrise::v2::installed_project(&catalog.index, e) else {
                        if e.mod_id.is_none() {
                            println!("{}", installed::old_registry_pack(&e.title));
                        } else {
                            println!(
                                "{}",
                                trf("{mod} isn't in Sunrise's registry.", &[("mod", &e.title)])
                            );
                        }
                        continue;
                    };
                    let unfit = build
                        .as_deref()
                        .map(sunrise::known_unfit_updates)
                        .unwrap_or_default();
                    build
                        .as_deref()
                        .and_then(|build| {
                            sunrise::v2::newer_release(
                                &catalog.index,
                                project,
                                &e.version,
                                build,
                                &unfit,
                            )
                        })
                        .map_or(PackUpdate::Current, |r| {
                            PackUpdate::Available(r.version.clone())
                        })
                } else {
                    continue;
                };
                match status {
                    PackUpdate::Current => println!("{} {}: up to date.", e.title, e.number),
                    PackUpdate::Available(number) => {
                        println!("{} {}: {number} is available.", e.title, e.number)
                    }
                    PackUpdate::Updated(entries) => {
                        for done in entries {
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
            confirm_world_loss,
            ..
        } => {
            let link = github_mods::parse(&url)?;
            let found = github_mods::resolve(&link, &github_mods::api)?;
            if trust_native_code {
                trust_unreviewed_builds();
            }
            github_native(GithubInstall {
                instance: &instance,
                found: &found,
                only: mod_id.as_deref(),
                all,
                update: false,
                trust: trust_native_code,
                confirm_world_loss,
            })?;
        }
        ModsCmd::UpdateGithub {
            id,
            check,
            trust_native_code,
            confirm_world_loss,
            ..
        } => {
            let mut origins = github_mods::origins(&home);
            origins.retain(|o| id.as_deref().is_none_or(|i| o.id == i));
            if trust_native_code {
                trust_unreviewed_builds();
            }
            if origins.is_empty() {
                return Err(match id {
                    Some(i) => format!("{i} wasn't installed from GitHub."),
                    None => "No native mod was installed from GitHub: sunrise-launcher mods add-github <url>".into(),
                }.into());
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
                    github_native(GithubInstall {
                        instance: &instance,
                        found: &latest,
                        only: Some(&o.id),
                        all: false,
                        update: true,
                        trust: trust_native_code,
                        confirm_world_loss,
                    })?;
                }
            }
        }
        ModsCmd::Remove {
            project,
            confirm_world_loss,
            ..
        } => {
            let identity = sunrise_launcher::profile_txn::Identity::of(&home);
            let list = installed::Installed::read(&home)?;
            let installed = list.lookup(&project);
            if let Some(row) = installed.filter(|row| row.kind != Kind::Mod) {
                let entry = installed::remove_pack(&home, row)?;
                println!("Removed {} ({}).", entry.title, entry.file);
            } else {
                if instance.kind == instances::Kind::Pack {
                    return Err(tr("This pack controls which mods it installs.").into());
                }
                let id = installed.map_or(project, |row| row.project.clone());
                let target = sunrise::Target::of(&instance)?;
                let removed = sunrise::remove_mods(
                    &target,
                    identity?,
                    &[id],
                    || sunrise_launcher::managed_play::headless_first(&instance),
                    mutation_hooks(),
                    &|impact| approve_mod_change(impact, confirm_world_loss),
                )
                .map_err(|e| match e {
                    sunrise::InstallError::Cancelled => tr("Nothing was removed.").into(),
                    e => e.to_string(),
                })?;
                if as_json {
                    json(removed.into());
                } else {
                    for name in removed {
                        println!("{}", trf("Removed {mod}.", &[("mod", &name)]));
                    }
                }
            }
        }
        ModsCmd::Enable { project, .. } | ModsCmd::Disable { project, .. } => {
            let profile = if on {
                let identity = sunrise_launcher::profile_txn::Identity::of(&home)?;
                Some(
                    sunrise_launcher::managed_play::Profile::open_for(&home, None, Some(identity))
                        .map_err(String::from)?,
                )
            } else {
                None
            };
            let e = installed::Installed::load(&home)
                .lookup(&project)
                .cloned()
                .ok_or_else(|| format!("{project} wasn't installed by the browser."))?;
            let id = sunrise::mod_id(&e)
                .filter(|_| e.kind == Kind::Mod)
                .ok_or_else(|| format!("{} isn't a mod: only mods turn on and off.", e.title))?;
            if on {
                let hash = e.checksum.trim_start_matches("SHA-256:");
                if let Some(r) = sunrise::v2::index()
                    .ok()
                    .filter(|_| e.source == SourceId::Sunrise)
                    .and_then(|catalog| {
                        // A whole-project takedown reaches a v2 row's project, delisted or not,
                        // and an old-registry row only when the index proves its project; any row
                        // is taken down by its archive's hash.
                        let project = if e.mod_id.is_some() {
                            Some(e.project.as_str())
                        } else {
                            sunrise::v2::installed_project(&catalog.index, &e)
                                .map(|p| p.id.as_str())
                        };
                        project
                            .and_then(|p| catalog.index.revoked(p, Some(hash)))
                            .or_else(|| catalog.index.revoked_by_sha256(hash))
                            .cloned()
                    })
                {
                    return Err(format!("{} was taken down: {}", e.title, r.reason).into());
                }
            }
            if let Some(profile) = profile {
                if let Some((_, reason)) = profile
                    .enable_unheld(&[id], sunrise_launcher::browser::sunrise::activate)?
                    .into_iter()
                    .next()
                {
                    return Err(format!("{}: {}.", e.title, reason.label()).into());
                }
            } else {
                sunrise_launcher::browser::mod_list::set_enabled(&home, id, false)?;
            }
            println!("{} is {}.", e.title, if on { "on" } else { "off" });
        }
    }
    Ok(())
}
