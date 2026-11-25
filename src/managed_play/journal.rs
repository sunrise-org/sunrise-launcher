//! The game's own install journal beside `options.txt`, and the records a profile keeps about
//! the packages it runs.
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A game-owned install not safe to finish. The full diagnostic belongs in Details, not in a banner.
#[derive(Debug)]
pub struct GameInstallProblem {
    detail: String,
    may_have_changed: bool,
}

impl GameInstallProblem {
    pub fn from_recovery(
        home: &Path,
        error: sunrise_launcher_smod::index::browser_state::RecoveryProblem,
    ) -> Self {
        use sunrise_launcher_smod::index::browser_state::RecoveryProblem;
        let may_have_changed = matches!(error, RecoveryProblem::Durability { .. });
        let error = error.details(crate::i18n::trf);
        // Relative file names and the cause are useful; the host's local path is not.
        let folder = home.display().to_string();
        let prefix = format!("{folder}{}", std::path::MAIN_SEPARATOR);
        let detail = error.strip_prefix(&prefix).map_or_else(
            || error.strip_prefix(&format!("{folder}: ")).unwrap_or(&error),
            |relative| relative,
        );
        Self {
            detail: detail.to_owned(),
            may_have_changed,
        }
    }

    pub fn summary(&self) -> &'static str {
        if self.may_have_changed {
            crate::i18n::tr(
                "A mod install the game started was interrupted and couldn't be finished.",
            )
        } else {
            crate::i18n::tr("A mod install the game started was interrupted and couldn't be finished. Your mods and files were left as they are.")
        }
    }

    pub fn details(&self) -> String {
        format!("{}\n{}", self.summary(), self.detail)
    }
}

impl From<GameInstallProblem> for String {
    fn from(value: GameInstallProblem) -> Self {
        value.details()
    }
}

/// Settle the game's sidecar using its own home lock. Callers that mutate the profile
/// hold the same, reentrant lock through their changes. No launcher journal is involved.
pub fn recover_game_install(home: &Path) -> Result<(), GameInstallProblem> {
    sunrise_launcher_smod::index::browser_state::reconcile(home)
        .map_err(|error| GameInstallProblem::from_recovery(home, error))
}

pub use sunrise_launcher_smod::index::browser_state::records::VerifiedRepository;

/// Exact project, release, SHA-256 digest, and verified repository recorded in the profile's
/// build lock for a signed schema-2 Sunrise registry mod.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModLock {
    pub project: String,
    pub mod_id: String,
    pub version: String,
    pub sha256: String,
    pub repository: VerifiedRepository,
}

impl ModLock {
    pub(super) fn validate(&self) -> Result<(), String> {
        use sunrise_launcher_smod::{index::valid_id, index_v2::valid_project_id};
        if !valid_project_id(&self.project)
            || !valid_id(&self.mod_id)
            || self.version.is_empty()
            || self.version.chars().any(char::is_control)
            || self.sha256.len() != 64
            || !self.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || !self.repository.valid()
        {
            return Err(unsettled!());
        }
        Ok(())
    }
}
