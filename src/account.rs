//! Microsoft account sign-in (sunrise-org/sunrise's docs/microsoft-account.md): the OAuth device-code flow, then Xbox
//! Live, XSTS and the Minecraft services, the game ownership check and the profile. Secrets (the
//! Microsoft refresh token, the Minecraft access token, the last ownership answer) live in the OS
//! keychain, or in a file only the player can read where there is none; accounts.json keeps only
//! names and UUIDs. The game gets the session in a file it
//! reads once (`--session-file`), never on its command line. Tokens never reach logs, errors or
//! output.
//!
//! The owner gate ([`launch`]): the launcher starts the game only while an account in it owns
//! Minecraft: Java Edition, as other launchers do, and the game plays as the selected account.
//! The game binary itself, its harnesses and the servers don't go through the launcher's gate.
//!
//! The Azure app (client ID) is configuration read at build time: `SUNRISE_MSA_CLIENT_ID`
//! (release builds take it from the repository variable), which the same variable at run time
//! overrides. Without one, sign-in says it is not configured.
mod background_sign_in;
pub mod capes;
mod game_launch;
pub mod look;
mod microsoft_sign_in;
pub mod player_key;
mod saved_accounts;
mod secret_store;
mod secrets_file;
pub mod session;
pub mod skins;
mod system_keychain;
pub use background_sign_in::*;
pub use game_launch::*;
pub use microsoft_sign_in::*;
pub use saved_accounts::*;
pub use secret_store::*;
pub use secrets_file::*;
pub use system_keychain::*;

use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The keychain service name every secret is stored under.
pub const SERVICE: &str = crate::identity::KEYCHAIN_SERVICE;
const SCOPE: &str = "XboxLive.signin offline_access";
pub const NOT_CONFIGURED: &str = crate::i18n::tr_key("This build of the launcher has no Microsoft sign-in (SUNRISE_MSA_CLIENT_ID), so it can't start the game.");
/// What every Play says while no account in the launcher owns the game.
pub const NEEDS_OWNER: &str = crate::i18n::tr_key(
    "Sign in with a Microsoft account that owns Minecraft: Java Edition to play",
);
const DOES_NOT_OWN: &str = crate::i18n::tr_key("This account doesn't own Minecraft: Java Edition.");
const NO_PROFILE: &str = crate::i18n::tr_key(
    "This account has no Minecraft profile yet. Create one at minecraft.net, then sign in again.",
);
const OTHER_PROFILE: &str =
    crate::i18n::tr_key("The Microsoft account now has another Minecraft profile. Sign in again.");
const SIGN_IN_AGAIN: &str = crate::i18n::tr_key("Your Microsoft sign-in expired. Sign in again.");
/// The Minecraft token is renewed when it has less than this left.
const RENEW_BEFORE: u64 = 3600;

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
pub(crate) mod tests;
