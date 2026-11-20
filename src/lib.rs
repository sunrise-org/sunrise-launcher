//! The launcher's core, shared by the GUI, the CLI and the MCP server.
pub mod account;
pub mod appearance;
pub mod backend;
pub mod browser;
pub mod build_import;
pub mod carry_over;
pub mod data_location;
pub mod datapacks;
pub mod desktop;
pub mod desktop_seams;
pub mod developer_install;
pub mod developer_offline;
pub(crate) mod download_retry;
pub mod downloads;
pub mod export;
#[cfg(unix)]
pub mod game_app;
pub mod game_log;
pub mod github;
pub mod github_mods;
pub mod heads;
pub mod i18n;
pub mod identity;
pub mod import;
#[cfg(windows)]
pub mod installer;
pub mod instances;
pub mod java;
pub mod jdk;
pub mod launch;
mod local_json;
#[cfg(target_os = "macos")]
mod mac_update;
pub mod mojang;
mod native_cache;
pub mod native_fit;
pub mod native_mods;
pub mod native_status;
pub mod net;
pub mod nightly;
pub mod no_ai;
pub mod notices;
pub mod packs;
pub mod pictures;
pub mod preparer;
pub mod private_window;
pub mod process;
pub mod proxy;
pub mod relocation;
mod run_leases;
pub mod rust_toolchain;
pub mod screenshots;
pub mod server_list;
pub mod servers;
pub mod setup_args;
pub mod shortcut;
pub mod signed;
pub mod sources;
pub mod tailscale;
pub mod telemetry;
pub mod theme_files;
pub mod themes;
pub mod uninstall_aside;
pub mod update_handoff;
pub mod update_recovery;
#[cfg(windows)]
mod windows_runtime;
pub mod worlds;
pub mod zip_intake;

pub mod jars;

pub mod storage;

pub mod cache_cleanup;

pub mod bug_report;

#[cfg(test)]
mod test_support;
