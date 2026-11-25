//! The mod browser's sources: Sunrise's own registry (mods, modpacks, resource and shader packs)
//! and Modrinth, searched only for resource packs, data packs and Iris-format shader packs. Each
//! result says how well it runs, and a file is downloaded only after its checksum matches
//! (docs/mod-browser.md).
//!
//! Java mods and modpacks are not offered from Modrinth: Sunrise doesn't run Java code
//! (docs/modding/src/how-to/content-mods.md). Packs go to `resourcepacks/` and `shaderpacks/`,
//! data packs into a world ([`installed::install_datapack`]).
pub mod details;
pub mod download;
pub mod images;
pub mod installed;
pub mod mod_list;
pub mod modrinth;
pub mod sunrise;

pub use sunrise_launcher_smod::install_lock::{lock_home, try_lock_home, HomeLockGuard};

/// The Minecraft version Sunrise plays: the installed build's, read once per search or install.
pub fn game() -> String {
    crate::backend::played_version_id()
}

pub use sunrise_launcher_smod::index::browser_state::{Kind, SourceId};

/// How well a project runs in Sunrise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Support {
    /// Made for the version Sunrise plays, and Sunrise reads everything it uses.
    Works,
    /// It loads, with the limit said.
    Partial(String),
    /// It can't run in Sunrise: never offered for install.
    No(String),
}

impl Support {
    pub fn installable(&self) -> bool {
        !matches!(self, Support::No(_))
    }

    pub fn label(&self) -> &'static str {
        match self {
            Support::Works => "Works",
            Support::Partial(_) => "Partly",
            Support::No(_) => "Doesn't run",
        }
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Support::Works => None,
            Support::Partial(r) | Support::No(r) => Some(r),
        }
    }
}

/// A search result or a looked-up project.
#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub source: SourceId,
    pub id: String,
    pub slug: String,
    pub title: String,
    pub summary: String,
    pub author: String,
    pub downloads: u64,
    /// The project's page on its source, for the player to read before installing.
    pub page: String,
    pub icon: Option<String>,
    pub license: String,
    pub kind: Kind,
    /// The Minecraft versions its files list, and the loaders (`fabric`, `neoforge`, `minecraft`...).
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    /// Why no version of it can be installed here, when its source says so (a registry project
    /// whose versions for this game were taken down or made for the earlier mod system).
    pub blocked: Option<String>,
}

impl Project {
    pub fn support(&self, game: &str) -> Support {
        match &self.blocked {
            Some(why) => Support::No(why.clone()),
            None => support(self.kind, &self.game_versions, &self.loaders, game),
        }
    }

    /// The project as a data pack, for a search of data packs or an install into a world;
    /// its page is Modrinth's data pack page.
    pub fn as_datapack(self) -> Project {
        Project {
            page: match self.source {
                SourceId::Modrinth => format!("https://modrinth.com/datapack/{}", self.slug),
                _ => self.page,
            },
            kind: Kind::DataPack,
            ..self
        }
    }
}

/// One downloadable file of a version, with the checksums its source publishes.
#[derive(Clone, Debug, PartialEq)]
pub struct FileRef {
    pub url: String,
    pub name: String,
    pub size: u64,
    pub sha512: Option<String>,
    /// What Sunrise's registry lists.
    pub sha256: Option<String>,
    pub sha1: Option<String>,
    pub primary: bool,
}

impl FileRef {
    /// The strongest checksum the source gives: what the download is checked against.
    pub fn checksum(&self) -> Option<(&'static str, &str)> {
        [
            ("SHA-512", &self.sha512),
            ("SHA-256", &self.sha256),
            ("SHA-1", &self.sha1),
        ]
        .into_iter()
        .find_map(|(algo, h)| Some((algo, h.as_deref()?)))
    }
}

/// A version of a project: its files and what it needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Version {
    pub id: String,
    pub number: String,
    pub published: String,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub files: Vec<FileRef>,
    /// Required dependencies, as project ids on the same source.
    pub required: Vec<String>,
    /// Why this version can't be downloaded from here (CurseForge authors can forbid it).
    pub blocked: Option<String>,
}

impl Version {
    /// The file to install: the primary one, else the first.
    pub fn file(&self) -> Option<&FileRef> {
        self.files
            .iter()
            .find(|f| f.primary)
            .or_else(|| self.files.first())
    }
}

/// A search.
#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub text: String,
    pub kind: Kind,
    /// Off: only what runs (made for `game`). On: everything of the kind, each result marked
    /// with how well it runs.
    pub everything: bool,
    /// The Minecraft version Sunrise plays ([`game`]).
    pub game: String,
    pub offset: usize,
    pub limit: usize,
    pub filters: details::Filters,
}

impl Query {
    pub fn new(text: &str, kind: Kind) -> Self {
        Self {
            text: text.trim().to_owned(),
            kind,
            everything: false,
            game: game(),
            offset: 0,
            limit: 20,
            filters: details::Filters::default(),
        }
    }
}

/// A place projects come from.
pub trait Source: Send + Sync {
    fn id(&self) -> SourceId;
    fn search(&self, query: &Query) -> Result<Vec<Project>, String>;
    fn project(&self, id: &str) -> Result<Project, String>;
    /// Versions, newest first.
    fn versions(&self, project: &Project) -> Result<Vec<Version>, String>;
    /// The project's page: description, gallery, links, authors.
    fn details(&self, project: &Project) -> Result<details::Details, String> {
        Ok(details::Details {
            body: project.summary.clone(),
            ..details::Details::default()
        })
    }
    /// Downloads one of its files into `folder`, checked ([`download::fetch`]).
    fn fetch(
        &self,
        file: &FileRef,
        folder: &std::path::Path,
        progress: &dyn Fn(f32),
    ) -> Result<std::path::PathBuf, String> {
        download::fetch(file, folder, progress)
    }
}

/// The Modrinth loader that marks a shader pack in the Iris format.
pub const IRIS: &str = "iris";
/// What an Iris-format shader pack is labelled with.
pub const IRIS_LABEL: &str = "Iris-format pack: most work, some effects may not";

/// How well a project of `kind` made for `game_versions` and `loaders` runs in Sunrise on `game`.
pub fn support(kind: Kind, game_versions: &[String], loaders: &[String], game: &str) -> Support {
    match kind {
        Kind::ResourcePack => {
            if game_versions.iter().any(|v| v == game) {
                Support::Works
            } else if let Some(newest) = newest_release(game_versions) {
                Support::Partial(format!(
                    "Made for {newest}: it loads with vanilla's compatibility warning, and \
                     what changed since may look wrong."
                ))
            } else {
                Support::Partial("Lists no Minecraft version.".into())
            }
        }
        // Iris-format packs from Modrinth: a shaders/-folder zip, which most of Sunrise's
        // shader pipeline reads; the registry's own `.sshader` packs took the first branch.
        Kind::Shader if loaders.iter().any(|l| l == IRIS) => {
            Support::Partial(crate::i18n::tr(IRIS_LABEL).into())
        }
        Kind::Shader => Support::No("Not one of Sunrise's own shader packs.".into()),
        Kind::DataPack => {
            let made_for = match newest_release(game_versions) {
                _ if game_versions.iter().any(|v| v == game) => String::new(),
                Some(newest) => format!("Made for {newest}: the game may refuse it. "),
                None => "Lists no Minecraft version. ".into(),
            };
            Support::Partial(format!("{made_for}{}", datapack_support()))
        }
        // Sunrise's own mods and modpacks took the first branch; nothing else is a Sunrise mod.
        Kind::Mod | Kind::Modpack => {
            Support::No("Sunrise doesn't run Java mods or modpacks.".into())
        }
    }
}

/// What a data pack does in Sunrise today, said before install; the install says it for the
/// pack's own files (`crate::datapacks::coverage`).
pub fn datapack_support() -> &'static str {
    crate::i18n::tr(
        "Sunrise runs a data pack's functions, function tags, predicates, item modifiers, recipes, loot tables, advancements, villager trades, damage types, most tags and most worldgen (biomes, features, structures, dimensions). Other files, such as world presets, are not read yet.",
    )
}

/// The newest plain release (`1.21.4`, `26.2`) in a list of Minecraft versions.
fn newest_release(versions: &[String]) -> Option<&str> {
    versions
        .iter()
        .filter(|v| {
            v.split('.')
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        })
        .max_by_key(|v| {
            v.split('.')
                .map(|p| p.parse::<u32>().unwrap_or(0))
                .collect::<Vec<_>>()
        })
        .map(String::as_str)
}

/// A project's licence as the window shows it: an SPDX id as it is (`MIT`, `LGPL-3.0-only`), and
/// a project's own licence, which Modrinth names `LicenseRef-Words-And-Version`, as those words
/// (`Polyform Shield 1.0.0`, `All Rights Reserved`). [`Project::license`] keeps the id itself.
pub fn license_label(id: &str) -> String {
    match id.strip_prefix("LicenseRef-") {
        Some(own) if !own.is_empty() => own.replace('-', " "),
        _ => id.to_owned(),
    }
}

/// The version to install: the newest one that runs, or with `everything` the newest at all.
/// `versions` is newest first, as [`Source::versions`] gives them.
pub fn pick<'a>(
    kind: Kind,
    versions: &'a [Version],
    everything: bool,
    game: &str,
) -> Option<&'a Version> {
    let mut usable = versions
        .iter()
        .filter(|v| v.blocked.is_none() && v.file().is_some());
    if kind == Kind::Shader {
        // An Iris-format zip: never a jar or another loader's file.
        let mut packs = usable.filter(|v| {
            v.loaders.iter().any(|l| l == IRIS)
                && v.file()
                    .is_some_and(|f| f.name.to_ascii_lowercase().ends_with(".zip"))
        });
        let made_for = |v: &&Version| v.game_versions.iter().any(|g| g == game);
        return packs
            .clone()
            .find(made_for)
            .or_else(|| packs.find(|_| everything));
    }
    if kind == Kind::DataPack {
        // Most data packs on Modrinth ship a mod build too: only the data pack files.
        let mut packs = usable.filter(|v| v.loaders.iter().any(|l| l == "datapack"));
        let made_for = |v: &&Version| v.game_versions.iter().any(|g| g == game);
        return packs
            .clone()
            .find(made_for)
            .or_else(|| packs.find(|_| everything));
    }
    let runs = |v: &&Version| match support(kind, &v.game_versions, &v.loaders, game) {
        Support::Works => true,
        // A modpack's version is for its own Minecraft.
        Support::Partial(_) => kind != Kind::ResourcePack,
        Support::No(_) => false,
    };
    if let Some(v) = usable.clone().find(runs) {
        return Some(v);
    }
    usable.find(|v| everything && support(kind, &v.game_versions, &v.loaders, game).installable())
}

/// A source's file name with the characters Windows forbids in a file name (`:` `*` `?` `"` `<` `>`
/// `|`) swapped for `-`, so a pack called "Yet Another Bingo: Ultimate.mrpack" is saved as
/// "Yet Another Bingo- Ultimate.mrpack". Folders, `..` and the rest still go to [`safe_name`].
pub fn plain_name(name: &str) -> String {
    name.replace([':', '*', '?', '"', '<', '>', '|'], "-")
}

pub(crate) const MAX_SAFE_NAME_BYTES: usize = 200;

/// A file name from a source, kept only when it is one plain name: no folders, no `..`, no
/// hidden file, nothing a file system would read as something else.
pub fn safe_name(name: &str) -> Option<&str> {
    let ok = !name.is_empty()
        && name.len() <= MAX_SAFE_NAME_BYTES
        && !name.starts_with('.')
        && !name.ends_with(['.', ' '])
        && name.chars().all(|c| {
            !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        });
    // Windows opens a device for CON, NUL.jar, COM1.zip and so on, whatever follows the first dot.
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|port| {
            stem.strip_prefix(port).is_some_and(|n| {
                let mut digits = n.chars();
                matches!(digits.next(), Some('0'..='9' | '¹' | '²' | '³'))
                    && digits.next().is_none()
            })
        });
    // Bidirectional and invisible marks that make a shown name read as another.
    let disguised = name.chars().any(|c| {
        matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
    });
    (ok && !device && !disguised).then_some(name)
}

#[cfg(test)]
mod tests;
