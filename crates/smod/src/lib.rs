//! Sunrise's mod files as the launcher reads them: `.smod` packages and their manifest, modpacks,
//! the mod registry's index, and the files a game run shares with the launcher.
pub mod dependencies;
pub mod files;
pub mod game_identity;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
pub mod holder;
pub mod icon;
pub mod index;
pub mod index_v2;
pub mod install_lock;
pub mod layouts;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
pub mod local_control;
pub mod locks;
pub mod manifest;
pub mod mod_list;
pub mod modpack;
pub mod output;
pub mod package;
pub mod packages;
pub mod runtime_leases;
pub mod selection;

pub use manifest::{Capability, Manifest, Package, find};
