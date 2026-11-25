//! The mod-browser's typed rows: what it installed into a game folder, and the verified
//! repository identity recorded for signed registry installs.
use super::{Kind, MANIFEST, SourceId, in_folders};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Verified GitHub repository and release workflow identity recorded from a Sigstore attestation
/// for a schema-2 Sunrise registry install.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedRepository {
    pub id: u64,
    pub name: String,
    pub path: String,
    pub workflow: String,
}

impl VerifiedRepository {
    pub fn valid(&self) -> bool {
        self.id > 0
            && !self.name.trim().is_empty()
            && !self.name.chars().any(char::is_control)
            && !self.path.trim().is_empty()
            && !self.path.chars().any(char::is_control)
            && !self.workflow.trim().is_empty()
            && !self.workflow.chars().any(char::is_control)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub source: SourceId,
    pub project: String,
    /// In-game mod ID for a schema-2 Sunrise registry entry where `project` is the permanent
    /// 8-character project ID. `None` on legacy v1 entries and non-mod entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mod_id: Option<String>,
    pub slug: String,
    pub title: String,
    pub kind: Kind,
    pub version: String,
    pub number: String,
    /// Relative to the game folder: `resourcepacks/<name>`.
    pub file: String,
    /// The checksum the download was checked against: `SHA-512:<hex>` or `SHA-256:<hex>`.
    pub checksum: String,
    pub page: String,
    /// Installed because another pack needs it.
    #[serde(default)]
    pub dependency: bool,
    /// What a Sunrise mod's trust was when it was installed (`trust_label`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<String>,
    /// A retained install's verified source stays off until the player can address this failure.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "launcher_install_hold"
    )]
    pub install_hold: Option<InstallHold>,
    /// Verified publisher repository and workflow recorded for a signed schema-2 Sunrise install.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<VerifiedRepository>,
}

/// Why verified bytes are intentionally installed but off after a partial native install.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstallHold {
    Compile,
    Needs { name: String },
}

impl Entry {
    /// Where the entry's file is in the game folder `home`, when it is inside one the browser
    /// installs into ([`in_folders`]).
    pub fn path(&self, home: &Path) -> Option<PathBuf> {
        in_folders(&self.file).then(|| home.join(&self.file))
    }

    /// In-game mod ID when this is a Sunrise mod entry (`mod_id` on v2 rows, `project` on v1 rows).
    pub fn in_game_mod_id(&self) -> Option<&str> {
        if self.source != SourceId::Sunrise || self.kind != Kind::Mod {
            return None;
        }
        Some(self.mod_id.as_deref().unwrap_or(self.project.as_str()))
    }

    /// Lowercase SHA-256 hex digest recorded in `checksum`, if present and well-formed.
    pub fn sha256(&self) -> Option<&str> {
        let hash = self.checksum.strip_prefix("SHA-256:")?;
        (hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then_some(hash)
    }

    /// True when this entry carries a schema-2 Sunrise project ID, in-game mod ID, and verified
    /// repository identity.
    pub fn is_v2_registry(&self) -> bool {
        self.source == SourceId::Sunrise
            && self.kind == Kind::Mod
            && crate::index_v2::valid_project_id(&self.project)
            && self.mod_id.is_some()
            && self
                .repository
                .as_ref()
                .is_some_and(VerifiedRepository::valid)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Installed {
    /// An entry this launcher can't read (a shader pack, which older launchers installed) is
    /// left out, and its file left where it is; so is one whose file is outside the folders the
    /// browser installs into ([`in_folders`]).
    #[serde(deserialize_with = "readable_entries")]
    pub entries: Vec<Entry>,
}

fn readable_entries<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Entry>, D::Error> {
    let entries = Vec::<serde_json::Value>::deserialize(d)?;
    Ok(entries
        .into_iter()
        .filter_map(|e| serde_json::from_value::<Entry>(e).ok())
        .filter(|e| {
            let kept = in_folders(&e.file);
            if !kept {
                eprintln!(
                    "{MANIFEST}: left out {:?}, whose file {:?} isn't in a pack or mod folder",
                    e.title, e.file
                );
            }
            kept
        })
        .collect())
}

impl Installed {
    pub fn load(home: &Path) -> Self {
        Self::read(home).unwrap_or_default()
    }

    pub fn read(home: &Path) -> Result<Self, String> {
        let path = home.join(MANIFEST);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(format!("{MANIFEST}: {error}")),
        };
        serde_json::from_str(&text).map_err(|error| format!("{MANIFEST}: {error}"))
    }

    /// Atomically publishes the list. Mutating callers must hold `lock_home`
    /// from before `load` through this save, not just while writing the file.
    pub fn save(&self, home: &Path) -> Result<(), String> {
        let _lock = crate::install_lock::lock_home(home).map_err(|e| e.to_string())?;
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = home.join(MANIFEST);
        let part = home.join(format!(".{MANIFEST}.{}-{id}.part", std::process::id()));
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::create_dir_all(home).map_err(|e| format!("{}: {e}", home.display()))?;
        let mut file =
            fs::File::create_new(&part).map_err(|e| format!("{}: {e}", part.display()))?;
        let written = std::io::Write::write_all(&mut file, text.as_bytes());
        drop(file);
        let res = written.and_then(|()| fs::rename(&part, &path));
        if let Err(e) = res {
            let _ = fs::remove_file(&part);
            return Err(format!("{}: {e}", path.display()));
        }
        Ok(())
    }

    pub fn find(&self, source: SourceId, project: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.source == source && e.project == project)
    }

    pub fn single(&self, source: SourceId, project: &str) -> Result<Option<&Entry>, String> {
        let mut rows = self
            .entries
            .iter()
            .filter(|e| e.source == source && e.project == project);
        let first = rows.next();
        if rows.next().is_some() {
            return Err(format!("Several browser entries claim {project}."));
        }
        Ok(first)
    }

    /// By project id or slug, as the CLI takes them.
    pub fn lookup(&self, name: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.project == name || e.slug == name)
    }
}
