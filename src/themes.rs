//! The player's own look: a theme (one of them light), an accent colour and a picture behind
//! Home.
//!
//! An accent is a hue, and the player may pick any of them; seven hues have names, as shortcuts.
//! Sunrise's violet is the default. Another hue turns the violet tokens (`sunrise_launcher_theme::Blocks`'
//! accent and the main action's face, bevel and lip) to it, keeping their saturation and
//! lightness, then darkens or lightens each just enough to read as well as the violet does: white
//! on the face at 4.5:1, the accent at 3:1 on every background.
use std::path::{Path, PathBuf};
use sunrise_launcher_theme::night::{Land, Mood, Night, SkyArt};
use sunrise_launcher_theme::{contrast, Blocks};

mod builtin;
pub mod library;
pub mod share;
pub use builtin::*;

/// An accent: a hue in degrees, or none for Sunrise's violet, whose tokens are the palette's own
/// and stay untouched; and for a colour typed as hex, that colour (the glass look wears it as
/// is, the light look takes its hue).
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Accent {
    hue: Option<f32>,
    rgb: Option<u32>,
}

/// The accent a launcher theme brings: a hue in degrees (First Light) or one exact colour (Nord's
/// frost).
#[derive(Clone, Copy, Debug)]
pub enum ThemeAccent {
    Hue(f32),
    Colour(u32),
}

impl ThemeAccent {
    pub fn accent(self) -> Accent {
        match self {
            Self::Hue(hue) => Accent::custom(hue),
            Self::Colour(rgb) => Accent::from_rgb(rgb),
        }
    }
}

/// A launcher theme: free for everyone in Settings › Appearance, never sold or claimed. Each
/// has a sky (six stops, top to bottom) and the accent it brings. A theme that brings a whole
/// palette carries its one [`Night`] (there is no light and dark of a theme: Sunrise Light is a
/// theme of its own) and paints Sunrise's own pages with it; the other themes only change the
/// sky.
pub struct Theme {
    /// Its name in `Settings::theme`.
    pub id: &'static str,
    /// Its name in Settings, in English (through `tr`).
    pub name: &'static str,
    /// A short line under its name in Settings, in English (through `tr`; kept short, as it is
    /// translated into every language).
    pub description: &'static str,
    pub sky: [u32; 6],
    pub accent: ThemeAccent,
    /// The theme's own palette, for a theme that is more than a sky: every theme is one look.
    pub palette: Option<&'static Night>,
    /// Its palette in the light look, for the themes that have one (Sunrise, and its light card).
    /// A player gets the light look by choosing the light theme; the others are dark.
    pub light_palette: Option<&'static Night>,
}

impl Theme {
    /// Whether choosing this theme makes the launcher light.
    pub fn is_light(&self) -> bool {
        self.palette.is_some_and(|p| !p.dark)
    }

    /// Whether this is Sunrise, in its night or its day: they share one scene and banner.
    pub fn is_sunrise(&self) -> bool {
        self.id == DAWN_THEME.id || self.id == DAWN_LIGHT_THEME.id
    }

    /// The palette this theme paints Sunrise's pages with in the dark or the light look.
    pub fn palette_in(&self, dark: bool) -> Option<&'static Night> {
        if dark {
            self.palette
        } else {
            self.light_palette.or(self.palette)
        }
    }
}

/// Sunrise's own theme's line in Settings.
pub const TWILIGHT_NAME: &str = crate::i18n::tr_key("Twilight");
pub const TWILIGHT_DESCRIPTION: &str = crate::i18n::tr_key("Indigo night and violet");

/// The launcher themes past Sunrise's own sky: the featured families first, then the older ones.
pub const THEMES: [Theme; 11] = [
    DAWN_THEME,
    DAWN_LIGHT_THEME,
    HYPR_NEON,
    DEEP_OCEAN,
    AUTUMN_FOREST,
    CHERRY_BLOSSOM,
    MIDNIGHT_AURORA,
    OVERWORLD,
    TAIGA,
    FIRST_LIGHT,
    NORD,
];

/// Sunrise's own theme's key in `Settings::theme_accents`.
pub const TWILIGHT: &str = "twilight";

/// What Twilight was saved as while it carried the name Sunrise. Settings that say so are read
/// as Twilight (`backend::look_settings`), so the indigo look a player chose stays theirs now
/// that Sunrise names the orange one.
pub const TWILIGHT_BEFORE: &str = "sunrise";

/// A theme's key in `Settings::theme_accents`: its id (a player's theme keeps its own
/// `local:<slug>`, there or not), or [`TWILIGHT`]. No saved theme and `dawn` share Dawn's.
pub fn accent_key(id: Option<&str>) -> &str {
    match id {
        Some(local) if local.starts_with(crate::theme_files::PREFIX) => local,
        _ => theme(id).map_or(TWILIGHT, |t| t.id),
    }
}

/// Whether the theme saved as `id` is the light one: what the launcher paints light is decided
/// by this choice alone.
pub fn is_light(id: Option<&str>) -> bool {
    id == Some(DAWN_LIGHT_THEME.id)
}

/// The accent a theme brings, before any the player picked under it.
pub fn own_accent(id: Option<&str>) -> Accent {
    match resolve(id) {
        Resolved::Twilight => Accent::VIOLET,
        Resolved::Builtin(t) => t.accent.accent(),
        Resolved::Local(local) => Accent::from_rgb(local.roles.accent),
        Resolved::Missing => DAWN_THEME.accent.accent(),
    }
}

/// The accent a theme starts with: the one last picked under it, else its own (violet for
/// Sunrise's).
pub fn starting_accent(
    id: Option<&str>,
    kept: &std::collections::BTreeMap<String, Accent>,
) -> Accent {
    kept.get(accent_key(id))
        .copied()
        .unwrap_or_else(|| own_accent(id))
}

/// What a saved theme id means right now.
#[derive(Clone)]
pub enum Resolved {
    /// The indigo and violet look: `twilight`, or an id this launcher doesn't know.
    Twilight,
    Builtin(&'static Theme),
    /// A theme file of the player's (`theme_files`).
    Local(crate::theme_files::Local),
    /// A `local:` id whose file is gone or broken: Dawn is painted until it is back, and the
    /// saved choice stays as it is.
    Missing,
}

/// Resolves `id` (`Settings::theme`) against the built-in themes and the player's, as last
/// read from disk.
pub fn resolve(id: Option<&str>) -> Resolved {
    match id {
        Some(local) if local.starts_with(crate::theme_files::PREFIX) => {
            crate::theme_files::find(local).map_or(Resolved::Missing, Resolved::Local)
        }
        _ => theme(id).map_or(Resolved::Twilight, Resolved::Builtin),
    }
}

/// The theme named `id` (`Settings::theme`): Dawn while none is saved, None for Sunrise's own sky
/// ([`TWILIGHT`]) or a name this launcher doesn't know.
pub fn theme(id: Option<&str>) -> Option<&'static Theme> {
    let id = id.unwrap_or(DAWN_THEME.id);
    THEMES.iter().find(|t| t.id == id)
}

/// The glass look's colour for each of [`Accent::ALL`]: the mockup's swatches.
pub const SWATCHES: [u32; 7] = [
    0x4f60e6, 0x2563eb, 0x0f766e, 0x15803d, 0xc2410c, 0xdc2626, 0xbe185d,
];

/// [`Accent::ALL`]'s names, in its order.
const NAMES: [&str; 7] = ["violet", "blue", "teal", "green", "amber", "red", "pink"];

impl Accent {
    pub const VIOLET: Accent = Accent {
        hue: None,
        rgb: None,
    };
    pub const BLUE: Accent = Accent {
        hue: Some(212.),
        rgb: None,
    };
    pub const TEAL: Accent = Accent {
        hue: Some(176.),
        rgb: None,
    };
    pub const GREEN: Accent = Accent {
        hue: Some(140.),
        rgb: None,
    };
    pub const AMBER: Accent = Accent {
        hue: Some(36.),
        rgb: None,
    };
    pub const RED: Accent = Accent {
        hue: Some(356.),
        rgb: None,
    };
    pub const PINK: Accent = Accent {
        hue: Some(322.),
        rgb: None,
    };

    pub const ALL: [Accent; 7] = [
        Self::VIOLET,
        Self::BLUE,
        Self::TEAL,
        Self::GREEN,
        Self::AMBER,
        Self::RED,
        Self::PINK,
    ];

    /// Any hue the player picks.
    pub fn custom(hue: f32) -> Self {
        Self {
            hue: Some(hue.rem_euclid(360.)),
            rgb: None,
        }
    }

    /// A colour (`0xrrggbb`): one of the swatches is its named accent.
    pub fn from_rgb(rgb: u32) -> Self {
        let rgb = rgb & 0xff_ffff;
        match SWATCHES.iter().position(|&c| c == rgb) {
            Some(i) => Self::ALL[i],
            None => Self {
                hue: Some(hsl(rgb).0),
                rgb: Some(rgb),
            },
        }
    }

    /// The colour the glass look wears: the one typed, a named accent's swatch, else the hue at
    /// the violet's saturation and lightness.
    pub fn rgb(self) -> u32 {
        if let Some(rgb) = self.rgb {
            return rgb;
        }
        match (Self::ALL.iter().position(|a| *a == self), self.hue) {
            (Some(i), _) => SWATCHES[i],
            (None, Some(hue)) => {
                let (_, s, l) = hsl(SWATCHES[0]);
                from_hsl(hue, s, l)
            }
            (None, None) => SWATCHES[0],
        }
    }

    /// [`rgb`](Self::rgb) as `#rrggbb`.
    pub fn hex(self) -> String {
        format!("#{:06x}", self.rgb())
    }

    /// Its name in `settings.json` and on the command line: a named hue's, a typed colour's
    /// `#rrggbb`, else its degrees.
    pub fn name(self) -> String {
        if self.rgb.is_some() {
            return self.hex();
        }
        let named = Self::ALL
            .iter()
            .position(|a| *a == self)
            .map(|i| NAMES[i].to_owned());
        named.unwrap_or_else(|| match self.hue {
            None => "violet".to_owned(),
            Some(hue) => format!("{hue}"),
        })
    }

    /// A name, `#rrggbb`, or a hue in degrees.
    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        if let Some(hex) = name.strip_prefix('#') {
            return (hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| u32::from_str_radix(hex, 16).ok())
                .flatten()
                .map(Self::from_rgb);
        }
        let name = if name == "purple" || name == "default" {
            "violet"
        } else {
            &name
        };
        if let Some(i) = NAMES.iter().position(|n| *n == name) {
            return Some(Self::ALL[i]);
        }
        name.parse::<f32>()
            .ok()
            .filter(|hue| hue.is_finite())
            .map(Self::custom)
    }

    /// Its hue in degrees; None for the violet, whose tokens stay as they are.
    pub fn hue(self) -> Option<f32> {
        self.hue
    }

    /// `palette` with this accent.
    pub fn apply(self, mut p: Blocks) -> Blocks {
        let Some(hue) = self.hue else {
            return p;
        };
        let turn = |c: u32| {
            let (_, s, l) = hsl(c);
            (hue, s, l)
        };
        // The face holds its label: white on a dark face, Nord's ink on a light one. Move the
        // face away from the label until the two read.
        let step = if contrast(p.on_face, 0x000000) < contrast(p.on_face, 0xffffff) {
            1.
        } else {
            -1.
        };
        let face = fit(turn(p.face), |c| contrast(p.on_face, c) >= 4.5, step);
        let (_, _, face_l) = hsl(face);
        let (h, s, l) = turn(p.face_bevel);
        p.face_bevel = from_hsl(h, s, l.max(face_l + 0.08).min(0.92));
        let (h, s, l) = turn(p.face_lip);
        p.face_lip = from_hsl(h, s, l.min(face_l - 0.12).max(0.05));
        p.face = face;
        // The accent marks things on every background: lighter on dark, darker on light.
        let grounds = [p.page, p.raised, p.button, p.panel, p.well];
        let reads = |c: u32| grounds.iter().all(|&g| contrast(c, g) >= 3.2);
        p.accent = fit(turn(p.accent), reads, if p.dark { 1. } else { -1. });
        p
    }
}

impl TryFrom<String> for Accent {
    type Error = String;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        Self::parse(&name).ok_or_else(|| {
            crate::i18n::trf(
                "{name}: a colour name or a hue in degrees",
                &[("name", &name)],
            )
        })
    }
}

impl From<Accent> for String {
    fn from(accent: Accent) -> Self {
        accent.name()
    }
}

/// The colour the hue picker shows for `hue`: a strip of them, at one saturation and lightness.
pub fn hue_colour(hue: f32) -> u32 {
    from_hsl(hue.rem_euclid(360.), 0.62, 0.55)
}

/// The colour's hue (degrees), saturation and lightness (0 to 1).
fn hsl(c: u32) -> (f32, f32, f32) {
    let [r, g, b] = [16, 8, 0].map(|s| ((c >> s) & 255) as f32 / 255.);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.;
    if max == min {
        return (0., 0., l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2. - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6. } else { 0. }
    } else if max == g {
        (b - r) / d + 2.
    } else {
        (r - g) / d + 4.
    };
    (h * 60., s, l)
}

fn from_hsl(h: f32, s: f32, l: f32) -> u32 {
    let c = (1. - (2. * l - 1.).abs()) * s;
    let x = c * (1. - ((h / 60.) % 2. - 1.).abs());
    let m = l - c / 2.;
    let (r, g, b) = match (h.rem_euclid(360.) / 60.) as u32 {
        0 => (c, x, 0.),
        1 => (x, c, 0.),
        2 => (0., c, x),
        3 => (0., x, c),
        4 => (x, 0., c),
        _ => (c, 0., x),
    };
    let byte = |v: f32| ((v + m).clamp(0., 1.) * 255.).round() as u32;
    byte(r) << 16 | byte(g) << 8 | byte(b)
}

/// The colour, made lighter (`step` 1) or darker (-1) a little at a time until `good` holds.
fn fit((h, s, l): (f32, f32, f32), good: impl Fn(u32) -> bool, step: f32) -> u32 {
    let mut l = l;
    for _ in 0..100 {
        let c = from_hsl(h, s, l);
        if good(c) || !(0.0..=1.0).contains(&l) {
            return c;
        }
        l += step * 0.01;
    }
    from_hsl(h, s, l.clamp(0., 1.))
}

use crate::theme_files;
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::Dir;

/// Keeps `file` as Home's background in `dir` (`background-<hash>.<ext>`: a new picture gets a
/// new name, so the window doesn't show the one it cached), and returns the kept file's name.
/// Nothing is kept unless it is a picture Home can show: at most 40 MB are read, what kind it
/// is comes from what it holds and never from its name, its size is within the limits, and it
/// decodes to one still picture. Slow: call it off the window's thread. An older background
/// stays: nothing in the data folder is deleted because a setting changed, since a name alone
/// doesn't say whose a file is.
pub fn keep_background(dir: &Path, file: &Path) -> Result<String, String> {
    use std::io::Read as _;
    let shown = file.display().to_string();
    let mut bytes = Vec::new();
    std::fs::File::open(file)
        .and_then(|f| f.take(theme_files::MAX_PICTURE + 1).read_to_end(&mut bytes))
        .map_err(|e| format!("{shown}: {e}"))?;
    if bytes.len() as u64 > theme_files::MAX_PICTURE {
        return Err(theme_files::Reason::PictureTooBig(shown).text());
    }
    let ext = match theme_files::kind(&bytes, &shown).map_err(|r| r.text())? {
        image::ImageFormat::Png => "png",
        image::ImageFormat::Jpeg => "jpg",
        _ => "webp",
    };
    theme_files::still(&bytes, &shown).map_err(|r| r.text())?;
    let hash = crate::sources::sha1_hex(&bytes);
    let name = format!("background-{}.{ext}", &hash[..12]);
    publish(dir, &name, &bytes).map_err(|e| format!("{name}: {e}"))?;
    Ok(name)
}

/// Whether `name` in `dir` is already this picture: a regular file, itself and not a link,
/// holding exactly `bytes`. `None` when nothing has that name.
fn same_copy(dir: &Dir, name: &str, bytes: &[u8]) -> std::io::Result<Option<bool>> {
    use std::io::Read as _;
    match dir.symlink_metadata(name) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
        Ok(meta) if !meta.is_file() => return Ok(Some(false)),
        Ok(_) => {}
    }
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = dir.open_with(name, &options)?;
    if !file.metadata()?.is_file() {
        return Ok(Some(false));
    }
    let mut held = Vec::new();
    file.take(bytes.len() as u64 + 1).read_to_end(&mut held)?;
    Ok(Some(held == bytes))
}

/// Puts `bytes` in `dir` as `name` without ever writing through what is already there. The
/// bytes go to a new file of the launcher's own, made exclusively, which then takes the name
/// in one step only if the name is free. A copy already there is kept as it is when it is
/// this picture, and anything else under the name (a link, a folder, other bytes) is refused
/// and left alone.
fn publish(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let taken = || {
        std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "something else already has this name in the launcher's folder; it was left alone",
        )
    };
    std::fs::create_dir_all(dir)?;
    let dir = Dir::open_ambient_dir(dir, cap_std::ambient_authority())?;
    match same_copy(&dir, name, bytes)? {
        Some(true) => return Ok(()),
        Some(false) => return Err(taken()),
        None => {}
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let temp = format!(".background-{}-{nanos}.tmp", std::process::id());
    let mut options = cap_std::fs::OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    let written = dir
        .open_with(&temp, &options)
        .and_then(|mut file| file.write_all(bytes).and_then(|()| file.sync_all()));
    // A hard link takes the name only if it is free, and never follows what is there.
    let published = written.and_then(|()| {
        dir.hard_link(&temp, &dir, name).or_else(|e| {
            // A disk with no hard links (FAT): the name is taken by a move, if it is free.
            let free = matches!(dir.symlink_metadata(name), Err(e) if e.kind() == std::io::ErrorKind::NotFound);
            if e.kind() != std::io::ErrorKind::AlreadyExists && free {
                dir.rename(&temp, &dir, name)
            } else {
                Err(e)
            }
        })
    });
    let _ = dir.remove_file(&temp);
    match published {
        Ok(()) => Ok(()),
        // Another launcher got there first: fine if it put the same picture.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            match same_copy(&dir, name, bytes)? {
                Some(true) => Ok(()),
                _ => Err(taken()),
            }
        }
        Err(e) => Err(e),
    }
}

/// The kept background `name`, decoded for Home: the copy in `dir` itself, never through a
/// link, read with the same bounds a new picture is held to and decoded to one still picture.
/// A copy kept by an older launcher that isn't such a picture is an error, never shown. Slow:
/// call it off the window's thread.
pub fn background_picture(dir: &Path, name: &str) -> Result<image::RgbaImage, String> {
    use std::io::Read as _;
    let said = |e: std::io::Error| format!("{name}: {e}");
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = Dir::open_ambient_dir(dir, cap_std::ambient_authority())
        .and_then(|dir| dir.open_with(name, &options))
        .map_err(said)?;
    if !file.metadata().map_err(said)?.is_file() {
        return Err(theme_files::Reason::Linked(crate::i18n::tr_key("picture")).text());
    }
    let mut bytes = Vec::new();
    file.take(theme_files::MAX_PICTURE + 1)
        .read_to_end(&mut bytes)
        .map_err(said)?;
    if bytes.len() as u64 > theme_files::MAX_PICTURE {
        return Err(theme_files::Reason::PictureTooBig(name.to_string()).text());
    }
    theme_files::still(&bytes, name).map_err(|r| r.text())
}

/// Home's background, if the player keeps one and it is still there.
pub fn background(dir: &Path, name: Option<&str>) -> Option<PathBuf> {
    let name = name.filter(|n| n.starts_with("background-") && !n.contains(['/', '\\']))?;
    Some(dir.join(name)).filter(|p| p.is_file())
}

#[cfg(test)]
mod tests;
