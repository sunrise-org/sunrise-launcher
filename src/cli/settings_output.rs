use super::*;

pub(super) fn settings_json(s: &backend::Settings) -> serde_json::Value {
    let background =
        sunrise_launcher::themes::background(&backend::data_dir(), s.background.as_deref());
    settings_json_with_paths(
        s,
        &sunrise_launcher::theme_files::dir(),
        background.as_deref(),
    )
}

/// The settings as the command prints them, with the themes folder and resolved background.
/// A folder goes out as the text a player would read: a path that isn't Unicode (any path is
/// allowed on Unix) is shown with its odd bytes replaced, never a reason to fail after a
/// setting was already saved.
fn settings_json_with_paths(
    s: &backend::Settings,
    themes: &Path,
    background: Option<&Path>,
) -> serde_json::Value {
    serde_json::json!({
        "selected_version": backend::resolve_version(None).ok(),
        "game_architecture": s.game_architecture.name(),
        "minecraft_dir": s.minecraft_dir,
        "server_dir": backend::server_dir(None),
        "prune_old_versions": s.auto_prune(),
        "keep_versions": s.keep_newest(),
        "opening_animation": !s.skip_opening,
        "cat": s.show_cat(),
        "game_args": s.game_args,
        "proxy": s.proxy,
        "desktop_entry": s.desktop_entry.unwrap_or(true),
        "while_playing": s.while_playing.name(),
        "world_layout": s.world_layout.name(),
        "language": s.language.map_or("system", |l| l.code()),
        "theme": sunrise_launcher::appearance::effective_id(s),
        "theme_choice": s.theme,
        "accent": sunrise_launcher::appearance::accent(s).name(),
        "scenery": s.scenery(),
        "panel_opacity": s.panel_opacity(),
        "reduced_graphics": s.reduced_graphics(),
        "themes_dir": themes.display().to_string(),
        "background": background.map(|kept| kept.display().to_string()),
        "cache_limit_gib": s.cache_limit_gib.unwrap_or(sunrise_launcher::storage::policy::DEFAULT_LIMIT_GIB),
        "no_ai": backend::no_ai(),
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt as _;

    /// The data folder may be any path the system allows: one that isn't Unicode is reported,
    /// with a kept picture in it, and nothing panics.
    #[test]
    fn folders_that_are_not_unicode_are_reported_not_a_panic() {
        // Format paths in memory: some Unix file systems cannot store non-UTF-8 names.
        let data = Path::new(std::ffi::OsStr::from_bytes(b"launcher-\xff\xfe"));
        let themes = data.join("themes");
        let background = data.join("background-0123456789ab.png");
        let s = backend::Settings::parse("{}");
        let shown = settings_json_with_paths(&s, &themes, Some(&background));
        assert_eq!(shown["themes_dir"], "launcher-��/themes");
        assert_eq!(
            shown["background"],
            "launcher-��/background-0123456789ab.png"
        );
        let none = settings_json_with_paths(&s, &themes, None);
        assert!(none["background"].is_null());
    }
}
