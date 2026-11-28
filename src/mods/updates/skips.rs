//! The versions the player skipped, so the next check doesn't offer them: one file of
//! `id@version` lines in the launcher's data folder, for every profile.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Skips {
    skipped: BTreeSet<String>,
}

fn key(id: &str, version: &str) -> String {
    format!("{id}@{version}")
}

impl Skips {
    fn file(dir: &Path) -> PathBuf {
        dir.join("skipped-updates.json")
    }

    /// What `dir` keeps: nothing, when there is no file or it can't be read (a skip is a
    /// convenience, never worth an error).
    pub(crate) fn load(dir: &Path) -> Self {
        let skipped = std::fs::read(Self::file(dir))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<BTreeSet<String>>(&bytes).ok())
            .unwrap_or_default();
        Self { skipped }
    }

    /// Writes the file whole, through a sibling so a crash never leaves half of it.
    pub(crate) fn save(&self, dir: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&self.skipped).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let file = Self::file(dir);
        let temp = file.with_extension("json.tmp");
        std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&temp, &file).map_err(|e| e.to_string())
    }

    pub(crate) fn skip(&mut self, id: &str, version: &str) {
        self.skipped.insert(key(id, version));
    }

    pub(crate) fn is_skipped(&self, id: &str, version: &str) -> bool {
        self.skipped.contains(&key(id, version))
    }

    pub(crate) fn clear(&mut self) {
        self.skipped.clear();
    }
}
