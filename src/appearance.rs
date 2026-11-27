//! The player's choices of look, as operations on the settings: the theme, the accent under
//! it, and Home's picture. Settings › Appearance and `settings KEY VALUE` on the command line
//! both call these, so a choice means the same from either.
//!
//! Nothing here saves, and nothing here deletes: a picture the choice replaced stays in the
//! data folder, so a save that fails, or another writer's choice, loses nothing.
//! What is painted when a choice can't be honoured (Dawn for a theme file that is gone, a
//! darker Play under a pale accent) is never written back.
use crate::backend::Settings;
use crate::i18n::{tr, trf};
use crate::theme_files::{self, Entry};
use crate::themes::{self, Accent, Resolved};
use std::path::{Path, PathBuf};

/// The id of the theme being painted: the saved one, Dawn for none saved or a player's theme
/// that is missing, Sunrise for an id this launcher doesn't know.
pub fn effective_id(settings: &Settings) -> String {
    match themes::resolve(settings.theme.as_deref()) {
        Resolved::Twilight => themes::TWILIGHT.into(),
        Resolved::Builtin(theme) => theme.id.into(),
        Resolved::Local(local) => local.id(),
        Resolved::Missing => themes::DAWN_THEME.id.into(),
    }
}

/// Whether the saved theme is a player's theme that can't be read right now.
pub fn missing(settings: &Settings) -> bool {
    matches!(
        themes::resolve(settings.theme.as_deref()),
        Resolved::Missing
    )
}

/// The accent being painted. A player's theme without an accent picked under it follows its
/// file, edits included; a missing one wears Dawn's.
pub fn accent(settings: &Settings) -> Accent {
    let key = themes::accent_key(settings.theme.as_deref());
    match themes::resolve(settings.theme.as_deref()) {
        Resolved::Missing => themes::DAWN_THEME.accent.accent(),
        Resolved::Local(local) if !settings.theme_accents.contains_key(key) => {
            Accent::from_rgb(local.roles.accent)
        }
        _ => settings.accent,
    }
}

/// Every theme that can be chosen, by id, then each folder that can't with its reason: what a
/// refused choice lists.
fn choices() -> String {
    let mut ids = vec![themes::TWILIGHT.to_string()];
    ids.extend(themes::THEMES.iter().map(|t| t.id.to_string()));
    let mut bad = Vec::new();
    for entry in theme_files::loaded().iter() {
        match entry {
            Entry::Good(local) => ids.push(local.id()),
            Entry::Bad { folder, reason } => bad.push(format!("{folder}: {}", reason.text())),
        }
    }
    let mut said = trf("Say default or one of {ids}.", &[("ids", &ids.join(", "))]);
    if !bad.is_empty() {
        said.push(' ');
        said.push_str(&trf(
            "Not usable: {reasons}",
            &[("reasons", &bad.join("; "))],
        ));
    }
    said
}

/// Chooses a theme: `default` (no saved id, which is Dawn), a built-in's id, or a player's
/// `local:<slug>`. The accent becomes the one last picked under that theme, else its own. A
/// choice that isn't there changes nothing and says what there is.
pub fn select_theme(settings: &mut Settings, choice: &str) -> Result<(), String> {
    let choice = choice.trim();
    let id = (choice != "default").then_some(choice);
    let known = match id {
        None => true,
        Some(id) if id == themes::TWILIGHT => true,
        Some(id) if id.starts_with(theme_files::PREFIX) => theme_files::find(id).is_some(),
        Some(id) => themes::THEMES.iter().any(|t| t.id == id),
    };
    if !known {
        return Err(format!("{choice}: {}", choices()));
    }
    // A colour set under the outgoing theme before accents were kept per theme is kept now.
    // A player's theme has no such past: its colour is its file's until one is picked.
    let old = settings.theme.as_deref();
    let old_key = themes::accent_key(old);
    if !old_key.starts_with(theme_files::PREFIX) && settings.accent != themes::own_accent(old) {
        settings
            .theme_accents
            .insert(old_key.to_string(), settings.accent);
    }
    settings.theme = id.map(str::to_string);
    // The light theme is what makes the launcher light, and every other one is dark.
    settings.appearance = if themes::is_light(id) {
        crate::backend::Appearance::Light
    } else {
        crate::backend::Appearance::Dark
    };
    settings.accent = themes::starting_accent(id, &settings.theme_accents);
    Ok(())
}

/// Picks an accent under the theme in use, and remembers it for that theme.
pub fn set_accent(settings: &mut Settings, picked: Accent) -> Result<(), String> {
    if missing(settings) {
        return Err(tr(
            "The chosen theme can't be read, so its accent can't be changed. Repair it and reload, or choose another theme.",
        )
        .into());
    }
    settings.accent = picked;
    settings.theme_accents.insert(
        themes::accent_key(settings.theme.as_deref()).to_string(),
        picked,
    );
    Ok(())
}

/// Goes back to the theme's own accent, forgetting the one picked under it.
pub fn reset_accent(settings: &mut Settings) -> Result<(), String> {
    if missing(settings) {
        return set_accent(settings, settings.accent);
    }
    let id = settings.theme.clone();
    settings
        .theme_accents
        .remove(themes::accent_key(id.as_deref()));
    settings.accent = themes::own_accent(id.as_deref());
    Ok(())
}

/// Reads an accent as typed: `theme` goes back to the theme's own, else a name, a hue in
/// degrees or `#rrggbb`.
pub fn type_accent(settings: &mut Settings, typed: &str) -> Result<(), String> {
    if typed.trim().eq_ignore_ascii_case("theme") {
        return reset_accent(settings);
    }
    let picked = Accent::parse(typed).ok_or_else(|| {
        let names: Vec<_> = Accent::ALL.iter().map(|a| a.name()).collect();
        trf(
            "{value}: say theme, {names}, a colour like #2563eb or a hue in degrees (0 to 360)",
            &[("value", typed), ("names", &names.join(", "))],
        )
    })?;
    set_accent(settings, picked)
}

/// Makes `file` Home's picture: a copy is kept in the data folder and named in the settings.
/// The copy it replaces stays until the caller has saved.
pub fn set_background(settings: &mut Settings, data: &Path, file: &Path) -> Result<(), String> {
    settings.background = Some(themes::keep_background(data, file)?);
    Ok(())
}

/// What is behind Home.
#[derive(Clone, Debug, PartialEq)]
pub enum Behind {
    /// Scenery is off: the theme's plain background colour.
    Plain,
    /// The picture the player chose, kept in the data folder.
    Personal(PathBuf),
    /// The picture of the player's theme in use.
    Theme(PathBuf),
    /// The theme's scene.
    Scene,
}

/// What is behind Home: the player's own picture first, then the theme's, then the scene. A
/// picture that is named but gone falls through to the next.
pub fn behind_home(settings: &Settings, data: &Path) -> Behind {
    if !settings.scenery() {
        return Behind::Plain;
    }
    if let Some(path) = themes::background(data, settings.background.as_deref()) {
        return Behind::Personal(path);
    }
    behind_without_own(settings)
}

/// What is behind Home when the player's own picture can't be shown (gone, or a kept copy
/// that isn't a picture Home can decode): the theme's picture, then the scene.
pub fn behind_without_own(settings: &Settings) -> Behind {
    if !settings.scenery() {
        return Behind::Plain;
    }
    match themes::resolve(settings.theme.as_deref()) {
        Resolved::Local(local) => local.picture.map_or(Behind::Scene, Behind::Theme),
        _ => Behind::Scene,
    }
}

/// A picture is named in the settings but its kept copy is gone: Appearance says so, and the
/// name stays.
pub fn background_lost(settings: &Settings, data: &Path) -> bool {
    settings.background.is_some()
        && themes::background(data, settings.background.as_deref()).is_none()
}

#[cfg(test)]
mod tests;
