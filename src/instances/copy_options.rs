//! What a copy of a profile brings along. The default is what a copy always brought (everything
//! in the game folder, with the play time starting again at zero); the New profile dialog and
//! `instance create --copy` can leave parts out.
use sunrise_launcher_smod::{index::browser_state, mod_list};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CopyOptions {
    /// The time played and when it was last played.
    pub play_time: bool,
    /// `saves/`.
    pub worlds: bool,
    /// The game's `options.txt`.
    pub options: bool,
    pub resource_packs: bool,
    pub screenshots: bool,
    pub shader_packs: bool,
    /// The Multiplayer list, `servers.dat`.
    pub servers: bool,
    /// The mods, their list and the mod browser's record of them.
    pub mods: bool,
}

impl Default for CopyOptions {
    fn default() -> Self {
        Self {
            play_time: false,
            worlds: true,
            options: true,
            resource_packs: true,
            screenshots: true,
            shader_packs: true,
            servers: true,
            mods: true,
        }
    }
}

impl CopyOptions {
    /// Everything, play time too.
    pub const fn everything() -> Self {
        Self {
            play_time: true,
            worlds: true,
            options: true,
            resource_packs: true,
            screenshots: true,
            shader_packs: true,
            servers: true,
            mods: true,
        }
    }

    /// Nothing at all.
    pub const fn nothing() -> Self {
        Self {
            play_time: false,
            worlds: false,
            options: false,
            resource_packs: false,
            screenshots: false,
            shader_packs: false,
            servers: false,
            mods: false,
        }
    }

    /// The top-level names of the game folder these options leave out.
    pub(super) fn skipped(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        for (on, names) in [
            (self.worlds, SAVES),
            (self.options, &[crate::paths::OPTIONS][..]),
            (self.resource_packs, RESOURCE_PACKS),
            (self.screenshots, SCREENSHOTS),
            (self.shader_packs, SHADER_PACKS),
            (self.servers, SERVERS),
            (self.mods, &MODS[..]),
        ] {
            if !on {
                out.extend_from_slice(names);
            }
        }
        out
    }
}

const SAVES: &[&str] = &["saves"];
const RESOURCE_PACKS: &[&str] = &["resourcepacks"];
const SCREENSHOTS: &[&str] = &["screenshots"];
const SHADER_PACKS: &[&str] = &["shaderpacks"];
const SERVERS: &[&str] = &["servers.dat", "servers.dat_old", "servers.dat_new"];
const MODS: [&str; 4] = [
    mod_list::DIR,
    mod_list::NATIVE_DIR,
    mod_list::FILE,
    browser_state::MANIFEST,
];

#[cfg(test)]
mod tests;
