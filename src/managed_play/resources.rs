//! Managed Play reads the exact prepared tree saved with its generation. The ordinary
//! preparation cache remains available for a later Prepare, without changing an active game.
use super::{artifact, digest_file, optional_bytes, plain_file, require_dir, unsettled};
use crate::{backend, preparer, profile_txn::durable};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
};

const ROOT: &str = "launcher-prepared";
const EVIDENCE: &str = "data-file.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Resources {
    primary: [u8; 32],
    accelerator: Option<[u8; 32]>,
}

impl Resources {
    pub(super) fn capture(
        base: &str,
        stage: &crate::profile_txn::Staging<'_>,
    ) -> Result<Self, String> {
        let preparation = backend::prepare_lock(base)?;
        backend::need_prepared_locked(base, &preparation)?;
        let handoff = preparation.handoff(Some(stage.handoff()));
        backend::complete_assets_staged(base, &handoff);
        let root = backend::version_dir(base);
        Self::seal(&root, &backend::resources(base), |resources| {
            let binary = preparer::binary(&root)?;
            match preparer::data_file(&binary, &root, resources, Some(&handoff)) {
                Ok(Some(true)) => digest_file(&resources.join(preparer::DATA_FILE))
                    .ok()
                    .flatten(),
                Ok(_) => None,
                Err(error) => {
                    eprintln!("{error}");
                    None
                }
            }
        })
    }

    pub(super) fn path(&self, base: &str) -> PathBuf {
        parent(&backend::version_dir(base))
            .join(key(self.primary))
            .join("resources")
    }

    pub(super) fn verify(&self, base: &str) -> Result<(), String> {
        let root = backend::version_dir(base);
        let path = self.path(base);
        for dir in [
            &root,
            &root.join(".local"),
            &parent(&root),
            path.parent().expect("snapshot parent"),
        ] {
            require_dir(dir)?;
        }
        let actual = artifact::tree(&path, true)?;
        if actual != Some(self.primary) {
            return Err(setup_error(&path));
        }
        Ok(())
    }

    pub(super) fn allow_accelerator(&self, base: &str) -> bool {
        self.accelerator.is_some_and(|digest| {
            digest_file(&self.path(base).join(preparer::DATA_FILE))
                .is_ok_and(|found| found == Some(digest))
        })
    }

    // When the source is prepared, flush a private primary tree and publish it without
    // replacing anything. Build the optional cache against the final path: the game caches
    // the canonical resource root inside it. A kill after exposure but before its evidence
    // write leaves a usable primary tree and an untrusted accelerator, never the reverse.
    fn seal(
        root: &Path,
        source: &Path,
        accelerator: impl FnOnce(&Path) -> Option<[u8; 32]>,
    ) -> Result<Self, String> {
        require_dir(root)?;
        require_dir(&root.join(".local"))?;
        if !source.starts_with(root) || !backend::prepared_at(source) {
            return Err(setup_error(source));
        }
        for ancestor in source.ancestors().take_while(|path| *path != root) {
            require_dir(ancestor)?;
        }
        let Some(primary) = artifact::tree(source, true)? else {
            return Err(setup_error(source));
        };
        let parent = parent(root);
        match fs::create_dir(&parent) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(at(&parent, error)),
        }
        require_dir(&parent)?;
        let snapshot = parent.join(key(primary));
        let resources = snapshot.join("resources");
        let evidence = snapshot.join(EVIDENCE);
        match fs::symlink_metadata(&snapshot) {
            Ok(_) => {
                require_dir(&snapshot)?;
                if artifact::tree(&resources, true)? != Some(primary) {
                    return Err(setup_error(&resources));
                }
                let accelerator = optional_bytes(&evidence)?
                    .map(|bytes| serde_json::from_slice(&bytes).map_err(|e| e.to_string()))
                    .transpose()?;
                return Ok(Self {
                    primary,
                    accelerator,
                });
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(at(&snapshot, error)),
        }
        let stage = tempfile::Builder::new()
            .prefix(".prepared-")
            .tempdir_in(&parent)
            .map_err(|error| at(&parent, error))?;
        let candidate = stage.path().join("resources");
        copy_primary(source, &candidate, true, 0)?;
        if artifact::tree(&candidate, true)? != Some(primary)
            || artifact::tree(source, true)? != Some(primary)
        {
            return Err(setup_error(source));
        }
        durable::flush_tree(stage.path()).map_err(|e| at(stage.path(), e))?;
        durable::move_new_directory(stage.path(), &snapshot).map_err(|e| at(&snapshot, e))?;
        let accelerator = match accelerator(&resources) {
            Some(digest) => {
                let bytes = serde_json::to_vec(&digest).map_err(|error| error.to_string())?;
                durable::replace(&snapshot, &evidence, &bytes).map_err(|e| at(&evidence, e))?;
                Some(digest)
            }
            None => None,
        };
        if artifact::tree(&resources, true)? != Some(primary) {
            return Err(setup_error(&resources));
        }
        Ok(Self {
            primary,
            accelerator,
        })
    }
}

pub(super) fn generated_cache(name: &OsStr) -> bool {
    name == preparer::DATA_FILE
        || name.to_str().is_some_and(|name| {
            name.strip_prefix("sunrise-data.bin.")
                .and_then(|name| name.strip_suffix(".tmp"))
                .is_some_and(|pid| !pid.is_empty() && pid.bytes().all(|c| c.is_ascii_digit()))
        })
}

fn parent(root: &Path) -> PathBuf {
    root.join(".local").join(ROOT)
}
fn key(digest: [u8; 32]) -> String {
    format!("{:x}", sha2::digest::Output::<Sha256>::from(digest))
}

fn copy_primary(source: &Path, dest: &Path, top: bool, depth: usize) -> Result<(), String> {
    if depth > 64 {
        return Err(setup_error(source));
    }
    require_dir(source)?;
    fs::create_dir(dest).map_err(|e| at(dest, e))?;
    for entry in fs::read_dir(source).map_err(|e| at(source, e))? {
        let entry = entry.map_err(|e| at(source, e))?;
        let name = entry.file_name();
        if top && generated_cache(&name) {
            continue;
        }
        let src = entry.path();
        let dst = dest.join(name);
        let meta = fs::symlink_metadata(&src).map_err(|e| at(&src, e))?;
        if !plain_file(&meta) {
            return Err(setup_error(&src));
        }
        if meta.is_dir() {
            copy_primary(&src, &dst, false, depth + 1)?;
        } else if meta.is_file() {
            preparer::clone_or_copy(&src, &dst).map_err(|e| at(&dst, e))?;
        } else {
            return Err(setup_error(&src));
        }
    }
    Ok(())
}

fn setup_error(path: &Path) -> String {
    format!("{}: {}", path.display(), unsettled())
}
fn at(path: &Path, error: io::Error) -> String {
    format!("{}: {error}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_snapshot_does_not_read_a_cache_from_a_different_tree() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path();
        let source = root.join(".local/prepared");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("sunrise-resources.json"), "{}").unwrap();
        fs::write(source.join("blocks.json"), "{\"old\": true}").unwrap();
        fs::write(source.join(preparer::DATA_FILE), b"untrusted").unwrap();
        let prepared = Resources::seal(root, &source, |dir| {
            let data = dir.join(preparer::DATA_FILE);
            assert!(!data.exists());
            fs::write(&data, b"generated from the snapshot").unwrap();
            digest_file(&data).unwrap()
        })
        .unwrap();
        let snapshot = parent(root).join(key(prepared.primary)).join("resources");
        assert_eq!(
            fs::read_to_string(snapshot.join("blocks.json")).unwrap(),
            "{\"old\": true}"
        );
        assert_eq!(
            Resources::seal(root, &source, |_| panic!("snapshot reused")).unwrap(),
            prepared
        );
        fs::write(source.join("blocks.json"), "{\"new\": true}").unwrap();
        let changed = Resources::seal(root, &source, |_| None).unwrap();
        assert_ne!(prepared.primary, changed.primary);
        fs::write(snapshot.join(preparer::DATA_FILE), b"different cache").unwrap();
        fs::write(snapshot.join("sunrise-data.bin.123.tmp"), b"pending cache").unwrap();
        assert_eq!(
            artifact::tree(&snapshot, true).unwrap(),
            Some(prepared.primary)
        );
        fs::write(snapshot.join("blocks.json"), b"different primary bytes").unwrap();
        assert_ne!(
            artifact::tree(&snapshot, true).unwrap(),
            Some(prepared.primary)
        );
    }
}
