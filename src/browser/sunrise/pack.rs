//! Sunrise modpacks from the registry (`.spack`, docs/modding/src/reference/registry-index.md), each played as its own
//! profile: `sunrise-packs/<id>/` in the game's home holds the pack's file, `pack.json` (what
//! was installed) and `home/`, the profile's game folder: its saves, its options, and the pack's
//! mods in `home/sunrise-mods/`. Read old pack records without opening a live writer.
use crate::i18n::tr;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

const RECORD: &str = "pack.json";

/// Where registry modpacks live.
pub fn packs_dir() -> PathBuf {
    crate::backend::game_home().join("sunrise-packs")
}

/// What `pack.json` keeps of an installed pack.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Installed {
    pub id: String,
    pub name: String,
    pub version: String,
    pub sha256: String,
    /// The pack's mods: `id version`.
    pub mods: Vec<String>,
    #[serde(skip)]
    pub work: PathBuf,
}

/// The installed registry packs, by name.
pub fn list() -> Vec<Installed> {
    list_in(&packs_dir())
}

pub(crate) fn list_in(dir: &Path) -> Vec<Installed> {
    list_in_until(dir, &|| false).unwrap_or_default()
}

pub(crate) fn list_in_until(dir: &Path, cancelled: &dyn Fn() -> bool) -> Option<Vec<Installed>> {
    if cancelled() {
        return None;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return Some(Vec::new());
    };
    let mut packs = Vec::new();
    for entry in entries {
        if cancelled() {
            return None;
        }
        let Ok(entry) = entry else {
            continue;
        };
        if let Some(pack) = read(&entry.path()) {
            packs.push(pack);
        }
    }
    packs.sort_by_key(|p| p.name.to_lowercase());
    Some(packs)
}

fn read(work: &Path) -> Option<Installed> {
    let text = fs::read_to_string(work.join(RECORD)).ok()?;
    let mut pack: Installed = serde_json::from_str(&text).ok()?;
    pack.work = work.to_path_buf();
    Some(pack)
}

/// The profile's game folder.
pub fn home(work: &Path) -> PathBuf {
    work.join("home")
}

pub fn play_unavailable() -> String {
    tr("Playing Sunrise modpacks is paused. Choose another profile to play.").into()
}

pub fn remove_unavailable() -> String {
    tr("Removing Sunrise modpacks is paused. Leave the pack in place; its files are unchanged.")
        .into()
}
