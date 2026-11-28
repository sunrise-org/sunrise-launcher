//! A package's runtime data files: the `runtime/` folder and the `[runtime]` table that lists
//! each file with its SHA-512.
//!
//! Runtime files are the bytes a mod's code opens beside itself (a compiled script bundle, a baked
//! lookup table). They are not pack data: no datapack or resource pack can replace them and no
//! reload touches them. Native mods may carry them.
use super::{sha512_hex, sha512_of};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

pub use crate::package::RUNTIME_DIR;

/// The most one runtime file may weigh.
pub const MAX_RUNTIME_FILE: u64 = 64 << 20;
/// The most all of a package's runtime files may weigh together.
pub const MAX_RUNTIME_TOTAL: u64 = 256 << 20;
/// The most files a package's `[runtime]` may list.
pub const MAX_RUNTIME_FILES: usize = 256;

/// A `[runtime]` table: each file's name under `runtime/` and its SHA-512.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(try_from = "BTreeMap<String, String>")]
pub struct RuntimeTable(BTreeMap<String, String>);

impl TryFrom<BTreeMap<String, String>> for RuntimeTable {
    type Error = String;

    fn try_from(files: BTreeMap<String, String>) -> Result<Self, Self::Error> {
        if files.is_empty() {
            return Err("[runtime] lists no file".into());
        }
        if files.len() > MAX_RUNTIME_FILES {
            return Err(format!(
                "[runtime] lists {} files, over the {MAX_RUNTIME_FILES}-file runtime cap",
                files.len()
            ));
        }
        for (name, hash) in &files {
            if !super::plain_path(name) {
                return Err(format!("[runtime] {name}: not a plain relative path"));
            }
            sha512_hex(hash).map_err(|e| format!("[runtime] {name} {e}"))?;
        }
        Ok(Self(files))
    }
}

impl RuntimeTable {
    /// Each listed file's name under `runtime/` and its SHA-512, in name order.
    pub fn files(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .map(|(name, hash)| (name.as_str(), hash.as_str()))
    }

    /// Whether the table lists `name`.
    pub fn lists(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    /// What two installations compare: a SHA-256 over every listed name and SHA-512, as 64 hex
    /// digits. Equal digests mean equal files, since each hash was checked against its bytes
    /// when the package opened.
    pub fn digest(&self) -> String {
        let mut sha = Sha256::new();
        for (name, hash) in &self.0 {
            sha.update(name.as_bytes());
            sha.update([0]);
            sha.update(hash.as_bytes());
            sha.update(b"\n");
        }
        sha.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Refuse a package whose `runtime/` holds a file this table does not list, lacks one it
    /// lists, or goes over a cap. `found` is each file under `runtime/` as `(name below it, bytes)`.
    pub fn check_layout(table: Option<&Self>, found: &[(String, u64)]) -> Result<(), String> {
        if found.len() > MAX_RUNTIME_FILES {
            return Err(format!(
                "{RUNTIME_DIR}/ holds {} files, over the {MAX_RUNTIME_FILES}-file runtime cap",
                found.len()
            ));
        }
        let mut total = 0u64;
        for (name, size) in found {
            if !table.is_some_and(|table| table.lists(name)) {
                return Err(format!("{RUNTIME_DIR}/{name}: not listed in [runtime]"));
            }
            if *size > MAX_RUNTIME_FILE {
                return Err(format!(
                    "{RUNTIME_DIR}/{name} is over {} MiB, the per-file runtime cap",
                    MAX_RUNTIME_FILE >> 20
                ));
            }
            total = total.saturating_add(*size);
        }
        if total > MAX_RUNTIME_TOTAL {
            return Err(format!(
                "{RUNTIME_DIR}/ is over {} MiB, the package's runtime total cap",
                MAX_RUNTIME_TOTAL >> 20
            ));
        }
        if let Some(table) = table
            && let Some(missing) = table
                .0
                .keys()
                .find(|name| !found.iter().any(|(have, _)| have == *name))
        {
            return Err(format!(
                "[runtime] lists {missing}, but the package has no {RUNTIME_DIR}/{missing}"
            ));
        }
        Ok(())
    }

    /// Refuse `bytes` that are not what the table lists for `name`.
    pub fn check(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let Some(hash) = self.0.get(name) else {
            return Err(format!("{RUNTIME_DIR}/{name}: not listed in [runtime]"));
        };
        if bytes.len() as u64 > MAX_RUNTIME_FILE {
            return Err(format!(
                "{RUNTIME_DIR}/{name} is over {} MiB, the per-file runtime cap",
                MAX_RUNTIME_FILE >> 20
            ));
        }
        if sha512_of(bytes) != *hash {
            return Err(format!(
                "{RUNTIME_DIR}/{name} doesn't match its [runtime] sha512"
            ));
        }
        Ok(())
    }
}

/// A package's runtime files, each checked against its hash when the package was opened. Reads
/// serve these bytes and never hash again.
#[derive(Clone, Debug)]
pub struct RuntimeData {
    files: BTreeMap<String, Arc<[u8]>>,
    digest: String,
    /// The package folder, when the package is one: the files sit in `<dir>/runtime/`.
    dir: Option<PathBuf>,
}

impl RuntimeData {
    pub(super) fn new(
        files: BTreeMap<String, Arc<[u8]>>,
        digest: String,
        dir: Option<PathBuf>,
    ) -> Self {
        Self { files, digest, dir }
    }

    /// The file `name` below `runtime/`, `None` for a name the manifest does not list.
    pub fn file(&self, name: &str) -> Option<Arc<[u8]>> {
        self.files.get(name).cloned()
    }

    /// Where the file sits on disk, for a package that is a folder. `None` for a name the
    /// manifest does not list, and for a `.smod` archive, which has no file to point at.
    pub fn path(&self, name: &str) -> Option<PathBuf> {
        self.files.contains_key(name).then_some(())?;
        Some(self.dir.as_ref()?.join(RUNTIME_DIR).join(name))
    }

    /// The names of the files, in order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// [`RuntimeTable::digest`] of the table these files were checked against.
    pub fn digest(&self) -> &str {
        &self.digest
    }
}
