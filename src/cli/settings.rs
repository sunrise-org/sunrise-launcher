//! `settings`: the keys it sets and how their values are read.
use super::*;

/// The keys `settings` sets.
pub(super) const SETTING_KEYS: [&str; 18] = [
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
    "appearance",
    "theme",
    "accent",
    "background",
    "scenery",
    "reduced-graphics",
];

pub(super) fn on_off(value: &str) -> Result<bool, String> {
    match value {
        "on" | "true" => Ok(true),
        "off" | "false" => Ok(false),
        _ => Err(format!("{value}: say on or off")),
    }
}

pub(super) fn set_setting(s: &mut backend::Settings, key: &str, value: &str) -> Result<(), String> {
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
        "appearance" => {
            s.appearance = backend::Appearance::parse(value)
                .ok_or_else(|| trf("{value}: say dark, light or system", &[("value", value)]))?
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

pub(super) fn show(key: Option<String>, value: Option<String>, as_json: bool) -> Result<(), Error> {
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
    Ok(())
}

pub(super) fn game_args(args: Option<String>, as_json: bool) -> Result<(), Error> {
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
    Ok(())
}

pub(super) fn no_ai(state: Option<String>, as_json: bool) -> Result<(), Error> {
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
    Ok(())
}

pub(super) fn desktop_entry(remove: bool) -> Result<(), Error> {
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
    Ok(())
}
