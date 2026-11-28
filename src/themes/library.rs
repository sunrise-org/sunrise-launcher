//! The player's own colours: the nine a theme is made of, the theme library (saved themes are
//! ordinary theme folders, the same `theme.toml` a player can write by hand) and the one working
//! copy the colour editor paints.
//!
//! Nothing here is a second format: a theme in the library is read back by `theme_files` like any
//! other, and a file is only written once `theme_files::parse` accepts what is about to be in
//! it. Every function takes the themes folder it works in, so a test never touches the
//! player's.
use crate::backend::Settings;
use crate::i18n::tr_key;
use crate::theme_files::{self, Entry, Local, Reason, Roles};
use crate::themes::{self, Resolved};
use std::path::Path;
use sunrise_launcher_theme::night::{Night, NIGHT};

/// The folder of the working copy: what the colour editor paints. It is a theme like any other,
/// but the library doesn't list it.
pub const WORKING: &str = "my-colours";
/// The working copy's name in `Settings::theme`.
pub const WORKING_ID: &str = "local:my-colours";
/// The most characters a name may have (`theme_files::parse`'s limit).
const NAME_MAX: usize = 64;

/// A theme as the library and a share code hold it: a name and its colours.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Custom {
    pub name: String,
    pub roles: Roles,
}

/// One colour a theme is made of: its key in `theme.toml`, its name in Settings (an English
/// key for `tr`) and how it is read and set. Every row of the colour editor, every line of a
/// theme file and every colour in a share code is one of these, in this order.
pub struct Slot {
    pub key: &'static str,
    pub label: &'static str,
    pub get: fn(&Roles) -> u32,
    pub set: fn(&mut Roles, u32),
}

pub const SLOTS: [Slot; 9] = [
    Slot {
        key: "background",
        label: tr_key("Background"),
        get: |r| r.background,
        set: |r, v| r.background = v,
    },
    Slot {
        key: "chrome",
        label: tr_key("Sidebar and top bar"),
        get: |r| r.chrome,
        set: |r, v| r.chrome = v,
    },
    Slot {
        key: "card",
        label: tr_key("Cards"),
        get: |r| r.card,
        set: |r, v| r.card = v,
    },
    Slot {
        key: "text",
        label: tr_key("Text"),
        get: |r| r.text,
        set: |r, v| r.text = v,
    },
    Slot {
        key: "muted",
        label: tr_key("Muted text"),
        get: |r| r.muted,
        set: |r, v| r.muted = v,
    },
    Slot {
        key: "accent",
        label: tr_key("Accent"),
        get: |r| r.accent,
        set: |r, v| r.accent = v,
    },
    Slot {
        key: "line",
        label: tr_key("Lines"),
        get: |r| r.line,
        set: |r, v| r.line = v,
    },
    Slot {
        key: "success",
        label: tr_key("Success"),
        get: |r| r.success,
        set: |r, v| r.success = v,
    },
    Slot {
        key: "danger",
        label: tr_key("Danger"),
        get: |r| r.danger,
        set: |r, v| r.danger = v,
    },
];

/// The index of the accent in [`SLOTS`].
pub const ACCENT: usize = 5;

/// The colours of a palette: how the launcher's own looks are shown in the editor.
pub fn roles_of(night: &Night) -> Roles {
    Roles {
        background: night.page,
        card: night.card,
        chrome: night.side,
        text: night.ink,
        muted: night.muted,
        accent: night.accent,
        line: night.stroke,
        success: night.success,
        danger: night.danger,
        light: !night.dark,
    }
}

/// The colours being painted now, whichever theme they come from: the editor starts from them.
pub fn current(settings: &Settings) -> Roles {
    let id = settings.theme.as_deref();
    let mut roles = match themes::resolve(id) {
        Resolved::Local(local) => local.roles,
        Resolved::Builtin(theme) => {
            roles_of(theme.palette_in(!themes::is_light(id)).unwrap_or(&NIGHT))
        }
        Resolved::Twilight => roles_of(&NIGHT),
        Resolved::Missing => roles_of(&themes::DAWN_DARK),
    };
    // An accent picked on its own (`settings accent`) is the one painted.
    roles.accent = crate::appearance::accent(settings).rgb();
    roles
}

/// Paints `roles`: they are written to the working copy, which becomes the chosen theme. The
/// accent picked under it before is forgotten, since the working copy's own is the one meant.
pub fn paint(settings: &mut Settings, roles: &Roles) -> Result<(), String> {
    write(&theme_files::dir(), WORKING, "Custom", roles, None)?;
    theme_files::reload();
    settings.theme_accents.remove(WORKING_ID);
    crate::appearance::select_theme(settings, WORKING_ID)
}

/// Goes back to Sunrise's own colours, in its night or its day.
pub fn reset(settings: &mut Settings, light: bool) -> Result<(), String> {
    let id = if light {
        themes::DAWN_LIGHT_THEME.id
    } else {
        "default"
    };
    crate::appearance::select_theme(settings, id)
}

/// `#rrggbb`, `rrggbb` or `#rgb`, as the player types it.
pub fn parse_hex(text: &str) -> Option<u32> {
    let digits = text.trim().trim_start_matches('#');
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    match digits.len() {
        6 => u32::from_str_radix(digits, 16).ok(),
        3 => {
            let doubled: String = digits.chars().flat_map(|c| [c, c]).collect();
            u32::from_str_radix(&doubled, 16).ok()
        }
        _ => None,
    }
}

/// The text of a theme file for these colours. What this writes, `theme_files::parse` reads.
pub fn render(name: &str, roles: &Roles, picture: Option<&str>) -> String {
    // A JSON string is a TOML basic string too.
    let quote = |text: &str| serde_json::to_string(text).expect("a string is always JSON");
    let look = if roles.light { "light" } else { "dark" };
    let mut text = format!("version = 1\nname = {}\nlook = \"{look}\"\n", quote(name));
    if let Some(picture) = picture {
        text.push_str(&format!("background = {}\n", quote(picture)));
    }
    text.push_str("\n[palette]\n");
    for slot in &SLOTS {
        text.push_str(&format!("{} = \"#{:06x}\"\n", slot.key, (slot.get)(roles)));
    }
    text
}

fn said(path: &Path, e: impl std::fmt::Display) -> String {
    format!("{}: {e}", path.display())
}

fn linked() -> String {
    Reason::Linked(tr_key("folder")).text()
}

/// Writes the theme `slug` under `root`: its folder is made, and `theme.toml` replaces the old
/// one in a single step. Nothing is written that the launcher would refuse to read back, and
/// nothing is written through a link.
pub fn write(
    root: &Path,
    slug: &str,
    name: &str,
    roles: &Roles,
    picture: Option<&str>,
) -> Result<(), String> {
    let text = render(name, roles, picture);
    theme_files::parse(&text).map_err(|reason| reason.text())?;
    if !theme_files::slug_ok(slug) || theme_files::root_linked(root) {
        return Err(linked());
    }
    let folder = root.join(slug);
    let is_link = |path: &Path| std::fs::symlink_metadata(path).is_ok_and(|m| m.is_symlink());
    let file = folder.join(theme_files::FILE);
    if is_link(&folder) || is_link(&file) {
        return Err(linked());
    }
    std::fs::create_dir_all(&folder).map_err(|e| said(&folder, e))?;
    let temp = folder.join(".theme.toml.tmp");
    std::fs::write(&temp, text)
        .and_then(|()| std::fs::rename(&temp, &file))
        .map_err(|e| said(&file, e))
}

/// The folder's name for a theme called `name`: its letters and digits in lower case, the
/// first one no other folder under `root` has (`-2`, `-3`… are added).
pub fn slug_for(root: &Path, name: &str) -> String {
    let words: String = name
        .chars()
        .map(|c| c.to_ascii_lowercase())
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let words: Vec<&str> = words.split('-').filter(|w| !w.is_empty()).collect();
    let mut base: String = words.join("-").chars().take(40).collect();
    base = base.trim_end_matches('-').to_string();
    if !theme_files::slug_ok(&base) {
        base = "theme".to_string();
    }
    let taken = |slug: &str| slug == WORKING || std::fs::symlink_metadata(root.join(slug)).is_ok();
    let mut slug = base.clone();
    let mut n = 2;
    while taken(&slug) {
        slug = format!("{base}-{n}");
        n += 1;
    }
    slug
}

/// `name`, or `name (2)`, `name (3)`… when a name in `names` is already it (in any case).
pub fn unique_name(names: &[String], name: &str) -> String {
    let same = |a: &str, b: &str| a.to_lowercase() == b.to_lowercase();
    let base: String = name.trim().chars().take(NAME_MAX - 6).collect();
    let mut tried = base.clone();
    let mut n = 2;
    while names.iter().any(|taken| same(taken, &tried)) {
        tried = format!("{base} ({n})");
        n += 1;
    }
    tried
}

/// The themes among `entries` that can be used, the working copy left out: the library.
pub fn listed(entries: &[Entry]) -> Vec<Local> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Good(local) if local.slug != WORKING => Some(local.clone()),
            _ => None,
        })
        .collect()
}

/// The library as it is on disk in the folder `root`.
pub fn saved(root: &Path) -> Vec<Local> {
    listed(&theme_files::scan(root))
}

fn find(root: &Path, slug: &str) -> Result<Local, String> {
    saved(root)
        .into_iter()
        .find(|local| local.slug == slug)
        .ok_or_else(|| said(&root.join(slug), Reason::NoFile.text()))
}

/// Saves `roles` as a new theme in the library. The name is made unique ("Ember (2)"). Returns
/// the folder it is in.
pub fn save_new(root: &Path, name: &str, roles: &Roles) -> Result<String, String> {
    let names: Vec<String> = saved(root).into_iter().map(|l| l.name).collect();
    let name = unique_name(&names, name);
    let slug = slug_for(root, &name);
    write(root, &slug, &name, roles, None)?;
    Ok(slug)
}

/// Saves each of `themes` as a new theme. Returns how many were saved.
pub fn add_all(root: &Path, themes: &[Custom]) -> Result<usize, String> {
    for theme in themes {
        save_new(root, &theme.name, &theme.roles)?;
    }
    Ok(themes.len())
}

/// Gives the theme `slug` another name, keeping its colours and its picture.
pub fn rename(root: &Path, slug: &str, name: &str) -> Result<(), String> {
    let local = find(root, slug)?;
    let picture = local
        .picture
        .as_deref()
        .and_then(Path::file_name)
        .and_then(|n| n.to_str());
    write(root, slug, name.trim(), &local.roles, picture)
}

/// A copy of the theme `slug` under a new name, without its picture. Returns its folder.
pub fn duplicate(root: &Path, slug: &str) -> Result<String, String> {
    let local = find(root, slug)?;
    save_new(root, &local.name, &local.roles)
}

/// Deletes the theme `slug`: its `theme.toml` and its picture, then its folder if nothing else
/// is in it.
pub fn remove(root: &Path, slug: &str) -> Result<(), String> {
    let local = find(root, slug)?;
    let folder = root.join(slug);
    let gone = |path: &Path| match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(said(path, e)),
        _ => Ok(()),
    };
    gone(&folder.join(theme_files::FILE))?;
    if let Some(picture) = &local.picture {
        gone(picture)?;
    }
    let _ = std::fs::remove_dir(&folder);
    Ok(())
}

/// Moves the theme `slug`'s whole folder (its colours and its picture) to the system's trash,
/// where the player can still get it back.
pub fn trash(root: &Path, slug: &str) -> Result<(), String> {
    find(root, slug)?;
    let folder = root.join(slug);
    crate::desktop_seams::to_trash(&folder).map_err(|e| said(&folder, e))
}

/// The themes `slugs` name, in the order of the library, for a share code.
pub fn take(root: &Path, slugs: &[String]) -> Vec<Custom> {
    saved(root)
        .into_iter()
        .filter(|local| slugs.contains(&local.slug))
        .map(|local| Custom {
            name: local.name,
            roles: local.roles,
        })
        .collect()
}

#[cfg(test)]
mod tests;
