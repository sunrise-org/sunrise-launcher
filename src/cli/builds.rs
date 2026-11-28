//! `versions`, `install`, `remove`, `prepare` and `notes`: the game builds on this computer.
use super::*;

pub(super) fn installed(as_json: bool) -> Result<(), Error> {
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
        let label = sunrise_launcher::developer_install::label(tag).unwrap_or_else(|| tag.clone());
        println!("{mark} {label}  {state}");
    }
    Ok(())
}

pub(super) fn releases(as_json: bool) -> Result<i32, Error> {
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
    Ok(0)
}

pub(super) fn install(tag: String, as_json: bool) -> Result<(), Error> {
    let label = format!("Installing {tag}");
    let tag = backend::install_tag(&tag, false, progress(label))?;
    if as_json {
        json(serde_json::json!({ "installed": tag, "prepared": backend::check_prepared(&tag) }));
    } else if backend::check_prepared(&tag) {
        println!("Installed {tag}. It is prepared already, from another installed version.");
    } else {
        println!("Installed {tag}. Prepare it next: sunrise-launcher prepare --version {tag}");
    }
    Ok(())
}

pub(super) fn remove(tag: String) -> Result<(), Error> {
    backend::remove_version(&tag)?;
    println!("Removed {tag}.");
    Ok(())
}

pub(super) fn prepare(
    minecraft: Option<PathBuf>,
    download: bool,
    version: Option<String>,
) -> Result<i32, Error> {
    // `--download` is kept for CLI compatibility; preparing always assembles the pinned version
    // into Sunrise's own folder, copying hash-matching files and downloading the rest for an owner.
    let _ = download;
    let tag = backend::resolve_version(version.as_deref())?;
    let explicit = minecraft
        .map(|dir| backend::minecraft_dir(Some(dir)))
        .transpose()?;
    let preparation = backend::prepare_lock(&tag)?;
    let (preparer, minecraft) = backend::preparer(
        &tag,
        explicit.as_deref(),
        &|line| eprintln!("{line}"),
        &preparation,
    )?;
    backend::sounds(&tag, &minecraft, &|line| println!("{line}"), &preparation);
    let (preparer, _runtime) = backend::prepare_command(preparer)?;
    let status = preparation
        .spawn_attached(preparer)
        .and_then(|mut child| child.wait())
        .map_err(|e| format!("Could not run the game's preparer: {e}"))?;
    let code = attached_status(status)?;
    if code != 0 {
        return Ok(code);
    }
    backend::finish_preparation(&tag, &preparation)?;
    println!("{tag} is prepared.");
    Ok(0)
}

pub(super) fn notes(tag: Option<String>, as_json: bool) -> Result<(), Error> {
    let r = backend::release_notes(tag.as_deref())?;
    if as_json {
        json(serde_json::json!({ "tag": r.tag, "name": r.name, "date": r.date, "notes": r.body }));
    } else {
        let title = if r.name.is_empty() { &r.tag } else { &r.name };
        println!(
            "{title}\n{}  {}\n\n{}",
            r.tag,
            r.date,
            backend::markdown_to_text(&r.body)
        );
    }
    Ok(())
}

/// Before `versions` and `update`: builds that can share another build's prepared resources do.
pub(super) fn reuse_prepared(command: &Cmd, as_json: bool) {
    if matches!(command, Cmd::Versions { .. } | Cmd::Update { .. }) {
        for (tag, from) in backend::reuse_prepared_everywhere() {
            if !as_json {
                eprintln!("Prepared {tag} with the resources of {from}.");
            }
        }
    }
}
