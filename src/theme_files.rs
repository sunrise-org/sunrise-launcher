//! Themes a player makes: `<data>/themes/<slug>/theme.toml`, with its one picture beside it.
//!
//! A file names up to six colours and nothing else: the page behind the scenery, the tint of
//! the panels and of the sidebar and top bar, the two inks and the accent. What it leaves out
//! is Dawn's. It can't set opacity, blur, type, sizes or the label on Play, which is white in
//! every theme.
//!
//! The folder is read once when the launcher starts and again on Reload ([`loaded`],
//! [`reload`]), never while a frame is drawn. Everything a file says is checked here, where it
//! is read: what comes out is a [`Local`] made of the file's colours, or a [`Reason`]
//! the Appearance page shows beside the folder's name. A file that fails is never rewritten,
//! and never stops another theme from loading.
use crate::i18n::{tr, trf};
use crate::themes::{DAWN_DARK, DAWN_LIGHT};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::Dir;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use sunrise_launcher_theme::contrast;
use sunrise_launcher_theme::night::{over, Night};

/// What a player theme's id starts with in `Settings::theme`: `local:<slug>`.
pub const PREFIX: &str = "local:";
pub const FILE: &str = "theme.toml";
const MAX_TOML: u64 = 16 << 10;
pub const MAX_PICTURE: u64 = 40 << 20;
const MIN_SIDE: u32 = 64;
const MAX_SIDE: u32 = 8192;
const MAX_PIXELS: u64 = 16_000_000;
/// The picture is drawn behind a window: one this wide on its longer side fills any screen.
const SHOWN_SIDE: u32 = 2560;
/// Names Windows keeps for devices, whatever follows the first dot.
const DEVICES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];
const ROLES: [&str; 9] = [
    "background",
    "card",
    "chrome",
    "text",
    "muted",
    "accent",
    "line",
    "success",
    "danger",
];

/// The nine colours of a theme file, each `0xRRGGBB`: the file's, else Dawn's (the lines, the
/// text's own colour), and the look they sit on. Everything else the launcher paints is
/// worked out from these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Roles {
    pub background: u32,
    pub card: u32,
    pub chrome: u32,
    pub text: u32,
    pub muted: u32,
    pub accent: u32,
    /// Dividers and outlines.
    pub line: u32,
    pub success: u32,
    pub danger: u32,
    /// Whether the scene and the glass around the colours are Sunrise's day, not its night.
    pub light: bool,
}

impl Roles {
    pub const DAWN: Roles = Roles {
        background: DAWN_DARK.page,
        card: DAWN_DARK.card,
        chrome: DAWN_DARK.side,
        text: DAWN_DARK.ink,
        muted: DAWN_DARK.muted,
        accent: DAWN_DARK.accent,
        line: DAWN_DARK.ink,
        success: DAWN_DARK.success,
        danger: DAWN_DARK.danger,
        light: false,
    };

    /// Sunrise's day: what the light look starts from.
    pub const DAWN_LIGHT: Roles = Roles {
        background: DAWN_LIGHT.page,
        card: DAWN_LIGHT.card,
        chrome: DAWN_LIGHT.side,
        text: DAWN_LIGHT.ink,
        muted: DAWN_LIGHT.muted,
        accent: DAWN_LIGHT.accent,
        line: DAWN_LIGHT.stroke,
        success: DAWN_LIGHT.success,
        danger: DAWN_LIGHT.danger,
        light: true,
    };

    /// White or black: the end the inks are moved toward when they don't read as given.
    fn ink_end(&self) -> u32 {
        if contrast(0xffffff, self.card) >= contrast(0x000000, self.card) {
            0xffffff
        } else {
            0x000000
        }
    }

    /// The colours as the launcher's palettes are made from them: Dawn's glass, scenery and
    /// warning colour (its day's when `light`) around the file's nine.
    pub fn night(&self) -> Night {
        let end = self.ink_end();
        let base = if self.light { DAWN_LIGHT } else { DAWN_DARK };
        Night {
            page: self.background,
            side: self.chrome,
            card: self.card,
            card_hi: over(end, 0.08, self.card),
            stroke: self.line,
            ink: self.text,
            ink_soft: self.text,
            muted: self.muted,
            accent: self.accent,
            accent_hi: self.accent,
            on_accent: 0xffffff,
            success: self.success,
            danger: self.danger,
            dark: end == 0xffffff,
            ..base
        }
    }
}

/// A theme file that passed every check.
#[derive(Clone, Debug, PartialEq)]
pub struct Local {
    /// Its folder's name.
    pub slug: String,
    /// Its own name, as the player reads it: plain text, one line.
    pub name: String,
    pub roles: Roles,
    /// Its picture for Home, in its folder, checked when the folder was read.
    pub picture: Option<PathBuf>,
}

impl Local {
    /// Its name in `Settings::theme`.
    pub fn id(&self) -> String {
        format!("{PREFIX}{}", self.slug)
    }
}

/// Why a theme folder is listed but can't be chosen.
#[derive(Clone, Debug, PartialEq)]
pub enum Reason {
    Slug,
    /// The folder, the file or the picture is a link.
    Linked(&'static str),
    NotRegular(&'static str),
    NoFile,
    TooLong,
    NotText,
    /// What the TOML reader said.
    Toml(String),
    Unknown(String),
    Missing(&'static str),
    Version,
    Name,
    NotString(String),
    Colour(String),
    NoColours,
    PictureName,
    PictureMissing(String),
    PictureKind(String),
    AnimatedWebP(String),
    PictureTooBig(String),
    PictureSize(String, u32, u32),
    Io(String),
}

impl Reason {
    /// The reason in the launcher's language. Names from the file are filled in as data.
    pub fn text(&self) -> String {
        match self {
            Reason::Slug => tr(
                "The folder's name must be 1 to 48 lowercase letters, digits or hyphens, starting with a letter or digit.",
            )
            .into(),
            Reason::Linked(what) => trf(
                "The {what} is a link. A theme's files must be in its own folder.",
                &[("what", tr(what))],
            ),
            Reason::NotRegular(what) => trf("The {what} must be a regular file.", &[("what", tr(what))]),
            Reason::NoFile => tr("There is no theme.toml in this folder.").into(),
            Reason::TooLong => tr("theme.toml is over 16 KiB.").into(),
            Reason::NotText => tr("theme.toml isn't UTF-8 text.").into(),
            Reason::Toml(said) => trf("theme.toml can't be read: {error}", &[("error", said)]),
            Reason::Unknown(key) => trf(
                "theme.toml has a key this launcher doesn't know: {key}",
                &[("key", key)],
            ),
            Reason::Missing(key) => trf("theme.toml is missing {key}.", &[("key", key)]),
            Reason::Version => tr("theme.toml must say version = 1.").into(),
            Reason::Name => tr(
                "The name must be 1 to 64 characters of plain text on one line.",
            )
            .into(),
            Reason::NotString(key) => trf("{key} must be text in quotes.", &[("key", key)]),
            Reason::Colour(key) => trf(
                "{key} must be a colour written as \"#RRGGBB\".",
                &[("key", key)],
            ),
            Reason::NoColours => tr(
                "[palette] must set at least one of background, card, chrome, text, muted and accent.",
            )
            .into(),
            Reason::PictureName => tr(
                "background must be the name of one picture in the theme's own folder.",
            )
            .into(),
            Reason::PictureMissing(name) => trf(
                "The picture {name} isn't in the theme's folder.",
                &[("name", name)],
            ),
            Reason::PictureKind(name) => trf(
                "{name} isn't a PNG, JPEG or WebP picture.",
                &[("name", name)],
            ),
            Reason::AnimatedWebP(name) => trf(
                "{name} is an animated WebP. Choose a still picture for Home.",
                &[("name", name)],
            ),
            Reason::PictureTooBig(name) => trf("{name} is over 40 MB.", &[("name", name)]),
            Reason::PictureSize(name, w, h) => trf(
                "{name} is {width}×{height}: a picture must be 64 to 8192 pixels a side and 16 megapixels at most.",
                &[
                    ("name", name),
                    ("width", &w.to_string()),
                    ("height", &h.to_string()),
                ],
            ),
            Reason::Io(said) => said.clone(),
        }
    }
}

/// `text` as a terminal may be given it. A folder's name and what a file says come from
/// outside the launcher and are kept as they are, so that a wrong one can be named: before a
/// terminal sees them, every control character but a line break, and every character that
/// turns text around, is written out as its escape (`\u{1b}`) instead of being obeyed.
pub fn for_terminal(text: &str) -> String {
    let mut shown = String::with_capacity(text.len());
    for c in text.chars() {
        if c != '\n' && (c.is_control() || turns_text(c)) {
            shown.extend(c.escape_unicode());
        } else {
            shown.push(c);
        }
    }
    shown
}

/// A folder under `themes/`: a theme, or why it isn't one.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Good(Local),
    Bad { folder: String, reason: Reason },
}

/// Whether `name` is a theme folder's name: 1 to 48 of `a-z`, `0-9` and `-`, not starting with
/// a hyphen, and not a device's name.
pub fn slug_ok(name: &str) -> bool {
    (1..=48).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !DEVICES.contains(&name)
}

/// A link, or on Windows any reparse point (a junction, a cloud placeholder).
fn linked(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & REPARSE_POINT != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

/// The themes folder at `root`, opened as itself: a link or junction standing in its place
/// is refused, never followed. `None` when there is no themes folder.
fn open_root(root: &Path) -> Result<Option<Dir>, Reason> {
    let meta = match fs::symlink_metadata(root) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Reason::Io(e.to_string())),
    };
    if linked(&meta) || !meta.is_dir() {
        return Err(Reason::Linked(crate::i18n::tr_key("folder")));
    }
    let (Some(parent), Some(name)) = (root.parent(), root.file_name()) else {
        return Err(Reason::Linked(crate::i18n::tr_key("folder")));
    };
    // Opened through its parent without following: a link put there since the check above is
    // refused by the open itself.
    Dir::open_ambient_dir(parent, cap_std::ambient_authority())
        .and_then(|parent| parent.open_dir_nofollow(name))
        .map(Some)
        .map_err(|_| Reason::Linked(crate::i18n::tr_key("folder")))
}

/// Whether a link stands where the themes folder should be: nothing is opened through it.
pub fn root_linked(root: &Path) -> bool {
    matches!(open_root(root), Err(Reason::Linked(_)))
}

/// The folder `name` directly in `dir`, opened relative to it and never through a link.
fn open_folder(dir: &Dir, name: &str) -> Result<Dir, Reason> {
    dir.open_dir_nofollow(name)
        .map_err(|e| match dir.symlink_metadata(name) {
            Ok(meta) if !meta.is_dir() => Reason::Linked(crate::i18n::tr_key("folder")),
            _ => Reason::Io(e.to_string()),
        })
}

/// At most `max` bytes of the regular file `name` directly in `dir`: opened relative to the
/// open folder without following a link, and checked on the opened file, so what is checked is
/// what is read. `over` when it holds more.
fn bounded(
    dir: &Dir,
    name: &str,
    max: u64,
    what: &'static str,
    missing: Reason,
    over: Reason,
) -> Result<Vec<u8>, Reason> {
    use std::io::Read as _;
    let kind = dir
        .symlink_metadata(name)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                missing
            } else {
                Reason::Io(e.to_string())
            }
        })?
        .file_type();
    if kind.is_symlink() {
        return Err(Reason::Linked(what));
    }
    if !kind.is_file() {
        return Err(Reason::NotRegular(what));
    }
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    // Even if a pipe replaces the regular file between the check and the open, do not wait
    // for a writer. Check the opened handle below before reading anything from it.
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY);
    }
    let file = dir
        .open_with(name, &options)
        .map_err(|e| match dir.symlink_metadata(name) {
            Ok(meta) if meta.file_type().is_symlink() => Reason::Linked(what),
            Ok(meta) if !meta.is_file() => Reason::NotRegular(what),
            _ => Reason::Io(e.to_string()),
        })?;
    if !file
        .metadata()
        .map_err(|e| Reason::Io(e.to_string()))?
        .is_file()
    {
        return Err(Reason::NotRegular(what));
    }
    let mut bytes = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Reason::Io(e.to_string()))?;
    if bytes.len() as u64 > max {
        return Err(over);
    }
    Ok(bytes)
}

/// `"#RRGGBB"`, in either case.
fn colour(value: &toml::Value, key: &str) -> Result<u32, Reason> {
    let bad = || Reason::Colour(key.to_string());
    let text = value.as_str().ok_or_else(bad)?;
    let hex = text.strip_prefix('#').ok_or_else(bad)?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad());
    }
    u32::from_str_radix(hex, 16).map_err(|_| bad())
}

/// One picture's name and nothing that leads anywhere else: no folder, drive, stream, address
/// or device.
fn picture_name_ok(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default();
    !name.is_empty()
        && name.len() <= 255
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':'])
        && !name.chars().any(char::is_control)
        && !name.ends_with(['.', ' '])
        && !DEVICES.contains(&stem.to_ascii_lowercase().as_str())
}

/// Characters that turn the text around them, which a name must not hide behind.
pub(crate) fn turns_text(c: char) -> bool {
    matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}' | '\u{061c}')
}

/// What a theme file says, before its picture is looked at.
#[derive(Debug, PartialEq)]
pub struct Parsed {
    pub name: String,
    pub roles: Roles,
    pub picture: Option<String>,
}

/// Reads a theme file's text. Every key is known, of its type and within its limits, or the
/// file is refused with the first thing wrong.
pub fn parse(text: &str) -> Result<Parsed, Reason> {
    // A key written twice is the reader's error.
    let table: toml::Table = text
        .parse()
        .map_err(|e: toml::de::Error| Reason::Toml(e.message().to_string()))?;
    if let Some(key) = table
        .keys()
        .find(|k| !["version", "name", "look", "background", "palette"].contains(&k.as_str()))
    {
        return Err(Reason::Unknown(key.clone()));
    }
    match table.get("version") {
        None => return Err(Reason::Missing("version")),
        Some(toml::Value::Integer(1)) => {}
        Some(_) => return Err(Reason::Version),
    }
    let name = table
        .get("name")
        .ok_or(Reason::Missing("name"))?
        .as_str()
        .ok_or(Reason::Name)?
        .trim();
    if !(1..=64).contains(&name.chars().count())
        || name.chars().any(|c| c.is_control() || turns_text(c))
    {
        return Err(Reason::Name);
    }
    let light = match table.get("look").map(|v| v.as_str()) {
        None | Some(Some("dark")) => false,
        Some(Some("light")) => true,
        Some(_) => return Err(Reason::NotString("look".into())),
    };
    let picture = match table.get("background") {
        None => None,
        Some(value) => {
            let name = value
                .as_str()
                .ok_or_else(|| Reason::NotString("background".into()))?;
            if !picture_name_ok(name) {
                return Err(Reason::PictureName);
            }
            Some(name.to_string())
        }
    };
    let palette = table
        .get("palette")
        .ok_or(Reason::Missing("[palette]"))?
        .as_table()
        .ok_or(Reason::Missing("[palette]"))?;
    if let Some(key) = palette.keys().find(|k| !ROLES.contains(&k.as_str())) {
        return Err(Reason::Unknown(format!("palette.{key}")));
    }
    if palette.is_empty() {
        return Err(Reason::NoColours);
    }
    let role = |key: &str, dawn: u32| match palette.get(key) {
        Some(value) => colour(value, &format!("palette.{key}")),
        None => Ok(dawn),
    };
    let ink = role("text", Roles::DAWN.text)?;
    let roles = Roles {
        background: role("background", Roles::DAWN.background)?,
        card: role("card", Roles::DAWN.card)?,
        chrome: role("chrome", Roles::DAWN.chrome)?,
        text: ink,
        muted: role("muted", Roles::DAWN.muted)?,
        accent: role("accent", Roles::DAWN.accent)?,
        // A file that says nothing of its lines gets the text's colour, as it always did.
        line: role("line", ink)?,
        success: role("success", Roles::DAWN.success)?,
        danger: role("danger", Roles::DAWN.danger)?,
        light,
    };
    Ok(Parsed {
        name: name.to_string(),
        roles,
        picture,
    })
}

/// The picture's bytes if it is one the launcher shows: PNG, JPEG or WebP by what it holds,
/// 40 MB at most, 64 to 8192 pixels a side and 16 megapixels at most.
fn picture_bytes(folder: &Dir, name: &str) -> Result<Vec<u8>, Reason> {
    let name = name.to_string();
    let bytes = bounded(
        folder,
        &name,
        MAX_PICTURE,
        crate::i18n::tr_key("picture"),
        Reason::PictureMissing(name.clone()),
        Reason::PictureTooBig(name.clone()),
    )?;
    kind(&bytes, &name)?;
    Ok(bytes)
}

/// What kind of picture `bytes` are, by what they hold and never by a file's name: PNG, JPEG
/// or WebP, 64 to 8192 pixels a side and 16 megapixels at most. `name` is for the reason.
pub fn kind(bytes: &[u8], name: &str) -> Result<image::ImageFormat, Reason> {
    let kind = image::guess_format(bytes).ok().filter(|f| {
        matches!(
            f,
            image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP
        )
    });
    let Some(kind) = kind else {
        return Err(Reason::PictureKind(name.to_string()));
    };
    let source = std::io::Cursor::new(bytes);
    let unreadable = |_| Reason::PictureKind(name.to_string());
    let (w, h) = if kind == image::ImageFormat::WebP {
        use image::ImageDecoder as _;
        let decoder = image::codecs::webp::WebPDecoder::new(source).map_err(unreadable)?;
        // Home never needs WebP frame composition. Reject animations before any frame is read.
        if decoder.has_animation() {
            return Err(Reason::AnimatedWebP(name.to_string()));
        }
        decoder.dimensions()
    } else {
        image::ImageReader::with_format(source, kind)
            .into_dimensions()
            .map_err(unreadable)?
    };
    let sides = MIN_SIDE..=MAX_SIDE;
    if !sides.contains(&w) || !sides.contains(&h) || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(Reason::PictureSize(name.to_string(), w, h));
    }
    Ok(kind)
}

/// One still picture from `bytes`, for Home: the first frame only, whatever else the file
/// holds, never more memory than the checked size needs, scaled down to what a window shows.
/// Slow: call it off the window's thread.
pub fn still(bytes: &[u8], name: &str) -> Result<image::RgbaImage, Reason> {
    let unreadable = || Reason::PictureKind(name.to_string());
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), kind(bytes, name)?);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_PIXELS * 4 * 2);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|_| unreadable())?;
    let long = decoded.width().max(decoded.height());
    let shown = if long > SHOWN_SIDE {
        decoded.thumbnail(SHOWN_SIDE, SHOWN_SIDE)
    } else {
        decoded
    };
    Ok(shown.to_rgba8())
}

/// `picture` no longer than `side` on its longer side: the window is never given a picture
/// larger than the GPU holds ([`crate::pictures::gpu_limit`]).
pub fn fitted(picture: image::RgbaImage, side: u32) -> image::RgbaImage {
    if picture.width().max(picture.height()) <= side {
        return picture;
    }
    image::DynamicImage::ImageRgba8(picture)
        .thumbnail(side.max(1), side.max(1))
        .to_rgba8()
}

/// The theme's picture, decoded for Home: its first frame, never more memory than its checked
/// size needs, scaled down to what a window shows. Slow: call it off the window's thread.
pub fn picture(path: &Path) -> Result<image::RgbaImage, Reason> {
    picture_in(&dir(), path)
}

/// [`picture`] for the themes folder at `root`. The path is walked again from the folder
/// itself, each step opened relative to the last and never through a link: a folder swapped
/// for a link since the themes were read leads nowhere.
pub fn picture_in(root: &Path, path: &Path) -> Result<image::RgbaImage, Reason> {
    let shown = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let missing = || Reason::PictureMissing(shown.clone());
    let inside = path.strip_prefix(root).map_err(|_| missing())?;
    let mut parts = inside.iter().map(|part| part.to_str());
    let (Some(Some(slug)), Some(Some(file)), None) = (parts.next(), parts.next(), parts.next())
    else {
        return Err(missing());
    };
    let folder = open_folder(&open_root(root)?.ok_or_else(missing)?, slug)?;
    let bytes = picture_bytes(&folder, file)?;
    still(&bytes, &shown)
}

/// Reads the theme in the folder `slug` of the open themes folder, which is at `root`.
fn read(themes: &Dir, root: &Path, slug: &str) -> Result<Local, Reason> {
    if !slug_ok(slug) {
        return Err(Reason::Slug);
    }
    let folder = open_folder(themes, slug)?;
    let bytes = bounded(
        &folder,
        FILE,
        MAX_TOML,
        crate::i18n::tr_key("theme.toml"),
        Reason::NoFile,
        Reason::TooLong,
    )?;
    let text = String::from_utf8(bytes).map_err(|_| Reason::NotText)?;
    let parsed = parse(&text)?;
    let picture = match parsed.picture {
        Some(name) => {
            picture_bytes(&folder, &name)?;
            Some(root.join(slug).join(name))
        }
        None => None,
    };
    Ok(Local {
        slug: slug.to_string(),
        name: parsed.name,
        roles: parsed.roles,
        picture,
    })
}

/// Every folder directly under `root`, by name: each a theme or the reason it isn't one. A
/// themes folder that isn't there has no themes, and one that is a link is listed as that and
/// not looked into.
pub fn scan(root: &Path) -> Vec<Entry> {
    let themes = match open_root(root) {
        Ok(Some(themes)) => themes,
        Ok(None) => return Vec::new(),
        Err(reason) => {
            return vec![Entry::Bad {
                folder: root
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                reason,
            }]
        }
    };
    let mut folders: Vec<String> = themes
        .entries()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir() || t.is_symlink()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    folders.sort();
    folders
        .into_iter()
        .map(|name| match read(&themes, root, &name) {
            Ok(local) => Entry::Good(local),
            Err(reason) => Entry::Bad {
                folder: name,
                reason,
            },
        })
        .collect()
}

/// Where the player's themes are: `themes/` in the launcher's data folder.
pub fn dir() -> PathBuf {
    crate::backend::data_dir().join("themes")
}

static LOADED: RwLock<Option<Arc<Vec<Entry>>>> = RwLock::new(None);

#[cfg(test)]
thread_local! {
    /// A test's own themes: tests never read the data folder's, nor each other's.
    static TEST_LOADED: std::cell::RefCell<Arc<Vec<Entry>>> = Default::default();
}

/// Makes `entries` the loaded themes of the test calling it.
#[cfg(test)]
pub(crate) fn set_loaded(entries: Vec<Entry>) {
    TEST_LOADED.set(Arc::new(entries));
}

/// The player's themes as last read: the first call reads them.
pub fn loaded() -> Arc<Vec<Entry>> {
    #[cfg(test)]
    return TEST_LOADED.with_borrow(Arc::clone);
    #[cfg(not(test))]
    if let Some(entries) = LOADED.read().ok().and_then(|l| l.clone()) {
        return entries;
    }
    #[cfg(not(test))]
    reload()
}

/// Puts `local` among the loaded themes in place of its folder's, without touching the disk: a
/// colour dragged in the editor is painted at once and written when it is let go.
pub fn preview(local: Local) {
    let mut entries: Vec<Entry> = loaded().as_ref().clone();
    entries.retain(|e| !matches!(e, Entry::Good(l) if l.slug == local.slug));
    entries.push(Entry::Good(local));
    #[cfg(test)]
    set_loaded(entries);
    #[cfg(not(test))]
    if let Ok(mut loaded) = LOADED.write() {
        *loaded = Some(Arc::new(entries));
    }
}

/// Reads the themes folder again.
pub fn reload() -> Arc<Vec<Entry>> {
    let entries = Arc::new(scan(&dir()));
    if let Ok(mut loaded) = LOADED.write() {
        *loaded = Some(entries.clone());
    }
    entries
}

/// The loaded theme with this id (`local:<slug>`), if it is there and good.
pub fn find(id: &str) -> Option<Local> {
    let slug = id.strip_prefix(PREFIX)?;
    loaded().iter().find_map(|e| match e {
        Entry::Good(local) if local.slug == slug => Some(local.clone()),
        _ => None,
    })
}

#[cfg(test)]
mod tests;
