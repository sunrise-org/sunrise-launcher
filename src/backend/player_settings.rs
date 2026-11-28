//! settings.json's model: appearance, what happens while playing, the world layout, the
//! settings themselves and their short-lived cache.
use super::{data_dir, look_settings, version_storage, GameArchitecture};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The launcher's appearance: dark, light, or the system's. Sunrise has a light look; the other
/// themes stay dark in both. Files from before the choice existed are settled in `Settings::parse`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    Dark,
    Light,
    /// Follow the system's light or dark.
    System,
}

impl Appearance {
    pub const ALL: [Appearance; 3] = [Self::Dark, Self::Light, Self::System];

    /// Its name in `settings.json` and on the command line.
    pub fn name(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::System => "system",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|a| a.name() == name)
    }
}

/// Where Home's profile header (mark, name, Play) stands in its banner, as the player picks it
/// in Settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HeaderAlign {
    /// At the banner's left padding.
    Left,
    /// At the banner's right padding.
    Right,
    /// On the banner's own middle (the default), with the top bar's search over it.
    #[default]
    CenterPanel,
    /// On the whole window's middle, the sidebar included.
    CenterWindow,
}

impl HeaderAlign {
    pub const ALL: [HeaderAlign; 4] = [
        Self::Left,
        Self::Right,
        Self::CenterPanel,
        Self::CenterWindow,
    ];

    /// Its name in `settings.json` and on the command line.
    pub fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::CenterPanel => "center-panel",
            Self::CenterWindow => "center-window",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|a| a.name() == name)
    }

    /// What Settings calls it (a catalog key).
    pub fn label(self) -> &'static str {
        match self {
            Self::Left => crate::i18n::tr_key("Left"),
            Self::Right => crate::i18n::tr_key("Right"),
            Self::CenterPanel => crate::i18n::tr_key("Centre of the page"),
            Self::CenterWindow => crate::i18n::tr_key("Centre of the window"),
        }
    }
}

/// What the launcher's window does while the game runs, as the player picks it in Settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WhilePlaying {
    /// The window stays where it is.
    #[default]
    Stay,
    /// The window goes to the Dock or the taskbar, and comes back when the game closes.
    Minimize,
    /// The launcher hides (macOS only: elsewhere it minimises), and comes back when the game
    /// closes.
    Hide,
}

/// How Home lists the worlds under Continue playing, as the player picks it in Settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorldLayout {
    /// One world a row: its picture, its name and its buttons.
    #[default]
    Rows,
    /// A grid of the worlds' pictures.
    Tiles,
}

impl WorldLayout {
    pub const ALL: [WorldLayout; 2] = [Self::Rows, Self::Tiles];

    /// Its name in `settings.json` and on the command line.
    pub fn name(self) -> &'static str {
        match self {
            Self::Rows => "rows",
            Self::Tiles => "tiles",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|l| l.name() == name)
    }
}

impl WhilePlaying {
    pub const ALL: [WhilePlaying; 3] = [Self::Stay, Self::Minimize, Self::Hide];

    /// Its name in `settings.json` and on the command line.
    pub fn name(self) -> &'static str {
        match self {
            Self::Stay => "stay",
            Self::Minimize => "minimize",
            Self::Hide => "hide",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        let name = match name.as_str() {
            "minimise" => "minimize",
            "keep" | "open" | "off" => "stay",
            n => n,
        };
        Self::ALL.into_iter().find(|w| w.name() == name)
    }

    /// What this computer does for it: only macOS can hide a whole app.
    pub fn here(self) -> Self {
        match self {
            Self::Hide if !cfg!(target_os = "macos") => Self::Minimize,
            w => w,
        }
    }
}

/// A button's corner radius as the look draws it, and the roundest the player can set, in px.
pub const DEFAULT_CORNER_RADIUS: u8 = 9;
pub const MAX_CORNER_RADIUS: u8 = 30;

/// Explicit permission to build one GitHub source mod. A rename within the same owner
/// retains consent; a change of owner, repository ID or mod ID never does.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SourceModGrant {
    pub repo_id: u64,
    pub owner: String,
    pub id: String,
}

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub selected: Option<String>,
    /// Game asset architecture. Auto preserves the launcher's process-architecture behavior.
    #[serde(default)]
    pub game_architecture: GameArchitecture,
    /// The instance Play starts (`instances`), by id. Unset means Sunrise, the game's home.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    pub minecraft_dir: Option<PathBuf>,
    pub server_dir: Option<PathBuf>,
    /// Unset means on: after an install the launcher deletes unused old automatic downloads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prune_old_versions: Option<bool>,
    /// How many of the newest release versions pruning keeps. Unset means [`version_storage::KEEP_NEWEST`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_versions: Option<usize>,
    /// Off by default, so the opening animation shows unless the player turns it off.
    #[serde(default)]
    pub skip_opening: bool,
    /// Whether the corner cat shows (`cat`). Unset means yes; only a player who switched it off
    /// has `false` here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cat: Option<bool>,
    /// Linux: whether the launcher keeps its entry in the applications menu (`desktop`). Unset
    /// means yes; removing the entry stores `false`, so it doesn't come back on the next start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_entry: Option<bool>,
    /// Extra arguments for the game's command line, as one line (see [`split_args`]). Empty by
    /// default. Appended after the launcher's own arguments, so they can add to or override them.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub game_args: String,
    /// The proxy every download goes through (`proxy`), such as `http://127.0.0.1:7890` or
    /// `socks5://127.0.0.1:1080`.
    /// Unset means none: the environment's `HTTPS_PROXY` and `HTTP_PROXY` apply instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    /// Dark, light or the system's (Settings › Appearance). Dark for every file saved before
    /// the choice existed.
    #[serde(default)]
    pub appearance: Appearance,
    /// Which settling the file has had (`look_settings::LOOK_VERSION`). Absent in files written
    /// before the appearance choice, which settle to dark and move a saved light to Taiga.
    #[serde(default)]
    pub look_version: u8,
    /// The player has answered the data switches in Settings, where the "I'm playing" ping's
    /// text (`telemetry::PING`) stands beside them.
    #[serde(default)]
    pub ping_notice_seen: bool,
    /// The player turned the anonymous "I'm playing" ping off (`telemetry::PING`): the game is
    /// started with `SUNRISE_NO_HEARTBEAT`. On by default, as upstream sends it.
    #[serde(default)]
    pub ping_off: bool,
    /// The player without an account that owns the game has read, once, why the launcher needs
    /// one now (the owner gate's screen).
    #[serde(default)]
    pub owner_notice_seen: bool,
    /// What the window does while the game runs. Stays by default.
    #[serde(default)]
    pub while_playing: WhilePlaying,
    /// How Home lists the worlds under Continue playing (`worlds`). Rows by default, and for
    /// every settings file written before the choice existed.
    #[serde(default)]
    pub world_layout: WorldLayout,
    /// The accent colour on top of the appearance (`themes::Accent`). When absent from saved
    /// settings, parsing uses the theme's own accent.
    #[serde(default)]
    pub accent: crate::themes::Accent,
    /// The launcher theme (`themes::theme`): its sky behind the light appearance. Unset is Dawn;
    /// the indigo one is `themes::TWILIGHT`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// Keep the scenery and the cat still and cut decorative motion. Unset is off. On disk this is
    /// still `performance`, the setting saved by earlier launchers.
    #[serde(
        rename = "performance",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub reduced_graphics: Option<bool>,
    /// Home's background: a picture the player picked, kept in the data folder under this name
    /// (`themes::keep_background`). Unset shows Sunrise's landscape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// The launcher version the player reduced to a dot in the top bar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dismissed_launcher_update: Option<String>,
    /// The window's language. Unset follows the system's (`i18n::Language::system`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<crate::i18n::Language>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_mod_grants: Vec<SourceModGrant>,
    /// The accent picked under each launcher theme, by its id (`themes::TWILIGHT` for Twilight's
    /// own), so going back to a theme brings back its colour. A launcher older than this map
    /// drops it when it saves: the themes then only start from their own accents again.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub theme_accents: std::collections::BTreeMap<String, crate::themes::Accent>,
    /// Reduced motion: unset follows the system's, true keeps the launcher still, false lets it
    /// move even when the system asks for less.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reduce_motion: Option<bool>,
    /// How round every corner in the launcher is, as a button's corner radius in px: 0 square,
    /// [`DEFAULT_CORNER_RADIUS`] the look as drawn, up to [`MAX_CORNER_RADIUS`]; cards and panels
    /// keep their proportion to it. Unset is the default (read it with [`Settings::corner_radius`]).
    #[serde(
        default,
        rename = "corner_radius",
        skip_serializing_if = "Option::is_none"
    )]
    pub corner_radius_px: Option<u8>,
    /// Home's scenery between the sidebars: the theme's animated pixel scene (or the player's
    /// picture). Unset is on; off leaves the plain page (read it with [`Settings::scenery`]).
    #[serde(default, rename = "scenery", skip_serializing_if = "Option::is_none")]
    pub scenery_on: Option<bool>,
    /// The sidebar shows its labels (Ctrl+B folds it to icons). Starts open, as the design
    /// draws it; a saved choice is kept.
    #[serde(default = "open_sidebar")]
    pub sidebar_expanded: bool,
    /// Where Home's profile header stands in its banner.
    #[serde(default)]
    pub header_align: HeaderAlign,
}

impl Settings {
    /// A button's corner radius in px, as the player set it.
    pub fn corner_radius(&self) -> u8 {
        self.corner_radius_px
            .unwrap_or(DEFAULT_CORNER_RADIUS)
            .min(MAX_CORNER_RADIUS)
    }

    /// How round the launcher's corners are, as a factor of the look's radii (1 as drawn).
    pub fn roundness(&self) -> f32 {
        f32::from(self.corner_radius()) / f32::from(DEFAULT_CORNER_RADIUS)
    }

    /// Whether Home shows its scenery: on unless the player turned it off.
    pub fn scenery(&self) -> bool {
        self.scenery_on.unwrap_or(true)
    }
}

struct CachedSettings {
    pub(super) modified: Option<std::time::SystemTime>,
    pub(super) len: u64,
    pub(super) settings: Settings,
}

static SETTINGS_CACHE: Mutex<Option<std::collections::HashMap<PathBuf, CachedSettings>>> =
    Mutex::new(None);
#[cfg(test)]
pub(super) static SETTINGS_FILE_READS: std::sync::LazyLock<
    Mutex<std::collections::HashMap<PathBuf, u64>>,
> = std::sync::LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));

fn settings_stamp(path: &Path) -> (Option<std::time::SystemTime>, u64) {
    fs::metadata(path)
        .map(|m| (m.modified().ok(), m.len()))
        .unwrap_or((None, 0))
}

/// Drops the process's parsed Settings after a write to `data`, so a save is visible even when the
/// filesystem timestamp did not move and the file kept its length.
pub(crate) fn forget_settings_cache(data: &Path) {
    let mut cache = SETTINGS_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(folders) = cache.as_mut() {
        folders.remove(data);
    }
}

impl Settings {
    pub fn load() -> Self {
        Self::load_cached_from(&data_dir())
    }
    /// The Settings of `data`, parsed once until the file's stamp changes or a save forgets it.
    pub(crate) fn load_cached_from(data: &Path) -> Self {
        let path = data.join(crate::paths::SETTINGS);
        let (modified, len) = settings_stamp(&path);
        {
            let cache = SETTINGS_CACHE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(cached) = cache.as_ref().and_then(|folders| folders.get(data)) {
                if cached.modified == modified && cached.len == len {
                    return cached.settings.clone();
                }
            }
        }
        let settings = Self::load_from(data);
        SETTINGS_CACHE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert_with(std::collections::HashMap::new)
            .insert(
                data.to_owned(),
                CachedSettings {
                    modified,
                    len,
                    settings: settings.clone(),
                },
            );
        settings
    }
    pub(crate) fn load_from(data: &Path) -> Self {
        #[cfg(test)]
        {
            *SETTINGS_FILE_READS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(data.to_owned())
                .or_default() += 1;
        }
        Self::parse(&fs::read_to_string(data.join(crate::paths::SETTINGS)).unwrap_or_default())
    }
    pub fn parse(text: &str) -> Self {
        let mut value = serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::json!({}));
        // No accent saved: the theme's own (Dawn's for a new install).
        let accent_saved = value.get("accent").is_some();
        if let Some(fields) = value.as_object_mut() {
            // An appearance from a newer launcher is Dark, not a reason to lose every setting.
            if fields
                .get("appearance")
                .is_some_and(|a| serde_json::from_value::<Appearance>(a.clone()).is_err())
            {
                fields.remove("appearance");
            }
            if fields
                .get("while_playing")
                .is_some_and(|w| serde_json::from_value::<WhilePlaying>(w.clone()).is_err())
            {
                fields.remove("while_playing");
            }
            // A layout from a newer launcher is rows.
            if fields
                .get("world_layout")
                .is_some_and(|l| serde_json::from_value::<WorldLayout>(l.clone()).is_err())
            {
                fields.remove("world_layout");
            }
            if fields
                .get("game_architecture")
                .is_some_and(|a| serde_json::from_value::<GameArchitecture>(a.clone()).is_err())
            {
                fields.remove("game_architecture");
            }
            if fields.get("accent").is_some_and(|a| {
                serde_json::from_value::<crate::themes::Accent>(a.clone()).is_err()
            }) {
                fields.remove("accent");
            }
            // A sidebar choice that isn't yes or no starts collapsed, not a reason to lose the rest.
            if fields
                .get("sidebar_expanded")
                .is_some_and(|v| !v.is_boolean())
            {
                fields.remove("sidebar_expanded");
            }
            // A bad dismissal should only restore the full update control.
            if fields
                .get("dismissed_launcher_update")
                .is_some_and(|value| !value.is_string())
            {
                fields.remove("dismissed_launcher_update");
            }
            if fields.get("source_mod_grants").is_some_and(|grants| {
                serde_json::from_value::<Vec<SourceModGrant>>(grants.clone()).is_err()
            }) {
                fields.remove("source_mod_grants");
            }
            look_settings::tidy(fields);
            // A language this launcher doesn't have yet follows the system's.
            if fields.get("language").is_some_and(|l| {
                serde_json::from_value::<Option<crate::i18n::Language>>(l.clone()).is_err()
            }) {
                fields.remove("language");
            }
        }
        let mut settings: Self = serde_json::from_value(value).unwrap_or_default();
        if !accent_saved {
            settings.accent =
                crate::themes::starting_accent(settings.theme.as_deref(), &settings.theme_accents);
        }
        look_settings::settle(&mut settings);
        settings
    }
    /// Whether the corner cat shows.
    pub fn show_cat(&self) -> bool {
        self.cat.unwrap_or(true)
    }
    /// Whether the launcher keeps its entry in the applications menu (Linux).
    pub fn desktop_entry(&self) -> bool {
        self.desktop_entry.unwrap_or(true)
    }
    /// The window's language: the one picked, else the system's.
    pub fn language(&self) -> crate::i18n::Language {
        self.language.unwrap_or_else(crate::i18n::Language::system)
    }
    /// Whether old versions are deleted after an install.
    pub fn auto_prune(&self) -> bool {
        self.prune_old_versions.unwrap_or(true)
    }
    /// Whether the launcher reduces graphics effects.
    pub fn reduced_graphics(&self) -> bool {
        self.reduced_graphics.unwrap_or(false)
    }
    /// How many of the newest release versions pruning keeps: at least the newest.
    pub fn keep_newest(&self) -> usize {
        self.keep_versions
            .unwrap_or(version_storage::KEEP_NEWEST)
            .max(1)
    }
}

/// Writes `bytes` to `path` readable by its owner alone (the file may hold proxy credentials);
/// the mode applies where the file is created, and a file already there is emptied first.
pub(super) fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path)?;
    // A file left by a crashed run keeps its old mode when opened: tighten it too.
    #[cfg(unix)]
    file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    file.write_all(bytes)
}

/// A settings file without a sidebar choice opens the sidebar.
fn open_sidebar() -> bool {
    true
}
