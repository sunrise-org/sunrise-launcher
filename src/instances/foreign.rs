//! Profiles other launchers keep, found on this computer so a new profile can start from their
//! worlds, options and packs. Nothing here changes what it reads: `import_sources` only looks,
//! and a copy leaves the other launcher's folder as it was. Mods are never brought along, as
//! another launcher's mods don't run in Sunrise.
use super::*;
use crate::native_mods::Cancellation;
use crate::sources::{self, Folders, Os};
use serde::Serialize;

mod copy;
mod launchers;
#[cfg(test)]
mod tests;

pub use copy::Progress;

/// What a game folder holds that an import can bring, with how many of each.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Contents {
    pub worlds: usize,
    /// `options.txt`, where the keys are too.
    pub options: bool,
    pub resource_packs: usize,
    pub shader_packs: usize,
    pub screenshots: usize,
    /// The Multiplayer list, `servers.dat`.
    pub servers: bool,
    /// Counted to say they stay behind: they are never imported.
    pub mods: usize,
}

impl Contents {
    /// Reads `game`, a game folder. Whatever can't be read counts as absent.
    pub fn read(game: &Path) -> Self {
        let file = |name: &str| {
            game.join(name)
                .symlink_metadata()
                .is_ok_and(|m| m.is_file())
        };
        Self {
            worlds: launchers::count_children(&game.join("saves"), |p| {
                p.join("level.dat").is_file()
            }),
            options: file(crate::paths::OPTIONS),
            resource_packs: launchers::count_children(&game.join("resourcepacks"), |_| true),
            shader_packs: launchers::count_children(&game.join("shaderpacks"), |_| true),
            screenshots: launchers::count_children(&game.join("screenshots"), |p| {
                p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                    ["png", "jpg", "jpeg"]
                        .iter()
                        .any(|x| e.eq_ignore_ascii_case(x))
                })
            }),
            servers: file("servers.dat"),
            mods: launchers::count_children(&game.join("mods"), |_| true),
        }
    }

    /// Whether there is anything an import could bring.
    pub fn any(&self) -> bool {
        self.worlds > 0
            || self.options
            || self.resource_packs > 0
            || self.shader_packs > 0
            || self.screenshots > 0
            || self.servers
    }
}

/// One profile of another launcher.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Foreign {
    /// The launcher's name: "Prism Launcher".
    pub launcher: String,
    pub name: String,
    /// The game folder: where its saves, options and packs are.
    pub folder: PathBuf,
    pub version: Option<String>,
    /// When it was last played, in seconds since 1970, where the launcher records that.
    pub last_played: Option<u64>,
    /// The seconds played, where the launcher records that.
    pub played_secs: Option<u64>,
    pub contents: Contents,
}

impl Foreign {
    /// What `instance import-from` takes: the launcher, a slash and the name.
    pub fn key(&self) -> String {
        format!("{}/{}", slug_of(&self.launcher), self.name)
    }

    /// What an import of it offers: each part that exists, all on, and the play time when the
    /// launcher kept one. Mods are never on.
    pub fn offered(&self) -> CopyOptions {
        let c = &self.contents;
        CopyOptions {
            play_time: self.played_secs.is_some() || self.last_played.is_some(),
            worlds: c.worlds > 0,
            options: c.options,
            resource_packs: c.resource_packs > 0,
            screenshots: c.screenshots > 0,
            shader_packs: c.shader_packs > 0,
            servers: c.servers,
            mods: false,
        }
    }
}

/// A launcher's name as one lowercase word, for keys: "Prism Launcher" is `prism-launcher`.
fn slug_of(launcher: &str) -> String {
    super::create::slug(launcher)
}

/// Every profile of the launchers found on this computer, by launcher in the order of
/// `sources::known_folders`, each launcher's by name. Sunrise's own game folder is left out.
pub fn import_sources() -> Vec<Foreign> {
    let Some(folders) = Folders::current() else {
        return Vec::new();
    };
    import_sources_in(Os::current(), &folders, &[backend::game_home()])
}

/// [`import_sources`] for `os` and `folders`, never listing a game folder inside `own`.
pub fn import_sources_in(os: Os, folders: &Folders, own: &[PathBuf]) -> Vec<Foreign> {
    let own: Vec<PathBuf> = own.iter().filter_map(|p| p.canonicalize().ok()).collect();
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut found = Vec::new();
    let known = sources::known_folders(os, folders)
        .into_iter()
        .chain(sources::import_only_folders(os, folders));
    for (launcher, root) in known {
        if !root.is_dir() {
            continue;
        }
        let mut batch = launchers::read_launcher(launcher, &root);
        batch.sort_by_cached_key(|f| f.name.to_lowercase());
        for entry in batch {
            let Ok(real) = entry.folder.canonicalize() else {
                continue;
            };
            if own
                .iter()
                .any(|o| real.starts_with(o) || o.starts_with(&real))
                || seen.contains(&real)
            {
                continue;
            }
            seen.push(real);
            found.push(entry);
        }
    }
    found
}

/// The profile `spec` names among `all`: `launcher/name` (the launcher as [`Foreign::key`]
/// writes it or as its name is spelled, in any case), or a name or launcher alone when only one
/// profile matches.
pub fn find_import<'a>(all: &'a [Foreign], spec: &str) -> Result<&'a Foreign, String> {
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    let launcher_is = |f: &Foreign, l: &str| same(&slug_of(&f.launcher), &slug_of(l));
    let matches: Vec<&Foreign> = match spec.split_once('/') {
        Some((launcher, name)) => all
            .iter()
            .filter(|f| launcher_is(f, launcher) && same(&f.name, name))
            .collect(),
        None => all
            .iter()
            .filter(|f| same(&f.name, spec) || launcher_is(f, spec))
            .collect(),
    };
    match matches[..] {
        [one] => Ok(one),
        [] => Err(crate::i18n::trf(
            "No profile of another launcher is called {name}.",
            &[("name", spec)],
        )),
        _ => Err(crate::i18n::trf(
            "More than one profile matches {name}: write it as launcher/name.",
            &[("name", spec)],
        )),
    }
}

/// A game folder the player picked, read like the launchers' own: a profile folder of Prism
/// Launcher, the Modrinth App, ATLauncher, GDLauncher or CurseForge, or a plain game folder
/// (with `.minecraft` or `minecraft` in it, or the saves and options right there).
pub fn read_import_folder(folder: &Path) -> Result<Foreign, String> {
    launchers::read_folder(folder)
        .ok_or_else(|| crate::i18n::tr("There is nothing in that folder to import.").to_owned())
}

/// Creates the profile `name` from `from`'s game folder, bringing what `options` keep (never
/// mods), and reports to `progress` as it copies. `cancel` stops it; a stop or any error leaves
/// no profile and no half-copied folder. `picture` and `choose` are [`create_with`]'s.
pub fn create_importing(
    name: &str,
    from: &Foreign,
    options: CopyOptions,
    cancel: &Cancellation,
    progress: &Progress,
    picture: Option<Option<&[u8]>>,
    choose: impl FnOnce(&mut Meta),
) -> Result<Instance, String> {
    create_importing_in(
        &backend::game_home(),
        &backend::data_dir(),
        name,
        from,
        options,
        cancel,
        progress,
        picture,
        choose,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn create_importing_in(
    game_home: &Path,
    data: &Path,
    name: &str,
    from: &Foreign,
    options: CopyOptions,
    cancel: &Cancellation,
    progress: &Progress,
    picture: Option<Option<&[u8]>>,
    choose: impl FnOnce(&mut Meta),
) -> Result<Instance, String> {
    let options = CopyOptions {
        mods: false,
        ..options
    };
    super::create::create_staged(
        game_home,
        data,
        name,
        None,
        picture,
        |meta| {
            choose(meta);
            if options.play_time {
                if let Some(secs) = from.played_secs {
                    meta.played_secs = secs;
                }
                if let Some(when) = from.last_played {
                    meta.last_played = Some(when);
                }
            }
        },
        |_| {},
        |contents| copy::copy_into(from, &options, contents, cancel, progress),
    )
}
