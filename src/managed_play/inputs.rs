use super::{at, digest_file, plain_file, require_dir, unsettled};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
};
use sunrise_launcher_smod::{index, manifest, mod_list};

/// One candidate the game discovers by extension or unpacked manifest. A directory is
/// game content, not a managed archive destination implied by its name or manifest ID.
pub struct ScannedPackage {
    pub manifest: manifest::Manifest,
    pub path: PathBuf,
    pub directory: bool,
}

pub fn scan_packages(home: &Path) -> Result<Vec<ScannedPackage>, String> {
    require_dir(home)?;
    let mut packages = Vec::new();
    let mut ids = BTreeSet::new();
    for directory in [mod_list::DIR, mod_list::NATIVE_DIR] {
        let root = home.join(directory);
        let entries = match fs::symlink_metadata(&root) {
            Ok(meta) if meta.is_dir() && plain_file(&meta) => {
                fs::read_dir(&root).map_err(|e| at(&root, e))?
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(at(&root, e)),
            Ok(_) => return Err(at(&root, unsettled())),
        };
        for entry in entries {
            let path = entry.map_err(|e| at(&root, e))?.path();
            let meta = fs::symlink_metadata(&path).map_err(|e| at(&path, e))?;
            if !plain_file(&meta) {
                return Err(at(&path, unsettled()));
            }
            if path.extension().is_none_or(|ext| ext != "smod") {
                if !meta.is_dir() {
                    continue;
                }
                let path = path.join(manifest::MANIFEST);
                match fs::symlink_metadata(&path) {
                    Ok(meta) if meta.is_file() && plain_file(&meta) => (),
                    Ok(_) => return Err(at(&path, unsettled())),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(at(&path, e)),
                }
            }
            let package = manifest::Package::read(&path).map_err(|e| at(&path, e))?;
            // The game reads content from both roots. Only the native root can
            // supply linked packages, and a managed native input must be archived.
            if package.is_native() && (meta.is_dir() || directory != mod_list::NATIVE_DIR) {
                return Err(crate::i18n::trf(
                    "Pack {mod} as an .smod in the native mods folder before using it as an installed native mod.",
                    &[("mod", &package.manifest.name)],
                ));
            }
            if !ids.insert(package.manifest.id.clone()) {
                return Err(crate::i18n::trf("There is more than one installed package for {mod}. Move the duplicate before trying again.", &[("mod", &package.manifest.id)]));
            }
            packages.push(ScannedPackage {
                manifest: package.manifest,
                path,
                directory: meta.is_dir(),
            });
        }
    }
    packages.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(packages)
}

/// Hash actual input nodes (not a display manifest), including opaque entries in the mod roots.
/// `repaired` is a browser-row postimage to be published in the same exact journal.
pub(crate) fn inputs(home: &Path, repaired: Option<&[u8]>) -> Result<[u8; 32], String> {
    require_dir(home)?;
    let mut hash = Sha256::new();
    let browser = home.join("mod-browser");
    match fs::symlink_metadata(&browser) {
        Ok(meta) if meta.is_dir() && plain_file(&meta) => (),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (),
        Err(e) => return Err(at(&browser, e)),
        Ok(_) => return Err(at(&browser, unsettled())),
    }
    for rel in [
        mod_list::DIR,
        mod_list::NATIVE_DIR,
        mod_list::FILE,
        crate::browser::installed::MANIFEST,
    ] {
        hash.update(rel.as_bytes());
        hash.update([0]);
        if let (crate::browser::installed::MANIFEST, Some(bytes)) = (rel, repaired) {
            hash.update([1]);
            hash.update(Sha256::digest(bytes));
        } else if matches!(rel, mod_list::DIR | mod_list::NATIVE_DIR) {
            if let Some(digest) = super::artifact::tree(&home.join(rel), false)? {
                hash.update([2]);
                hash.update(digest);
            } else {
                hash.update([0]);
            }
        } else if let Some(digest) = digest_file(&home.join(rel))? {
            hash.update([1]);
            hash.update(digest);
        } else {
            hash.update([0]);
        }
    }
    Ok(hash.finalize().into())
}

/// A fresh registry response can corroborate a display change at a claimed destination.
/// These rows are game-facing metadata, never destructive authority for the launcher.
pub(super) fn browser_repair(
    home: &Path,
    packages: &[ScannedPackage],
    registry: Option<&index::Index>,
) -> Result<Option<Vec<u8>>, String> {
    use crate::browser::{installed, sunrise, Kind, SourceId};
    use serde_json::Value;
    let path = home.join(installed::MANIFEST);
    let Some(bytes) = super::optional_bytes(&path)? else {
        return Ok(None);
    };
    let mut document: serde_json::Map<String, Value> =
        serde_json::from_slice(&bytes).map_err(|e| at(&path, e))?;
    let rows = document
        .get_mut("entries")
        .and_then(Value::as_array_mut)
        .ok_or_else(unsettled)?;
    let mut claims = index::browser_state::Installed {
        entries: std::mem::take(rows),
    };
    let by_file = packages
        .iter()
        .map(|package| {
            package
                .path
                .strip_prefix(home)
                .map(|relative| (relative, package))
                .map_err(|e| at(&package.path, e))
        })
        .collect::<Result<std::collections::BTreeMap<_, _>, _>>()?;
    let mut changes = Vec::new();
    for (position, raw) in claims.entries.iter().enumerate() {
        let Some(file) = raw.get("file").and_then(Value::as_str) else {
            continue;
        };
        let relative = Path::new(file);
        if ![mod_list::DIR, mod_list::NATIVE_DIR]
            .iter()
            .any(|dir| relative.starts_with(dir))
        {
            continue;
        }
        let row: installed::Entry =
            serde_json::from_value(raw.clone()).map_err(|e| at(&path, e))?;
        let package = by_file.get(relative).ok_or_else(unsettled)?;
        let id = &package.manifest.id;
        if package.directory
            || !installed::in_folders(file)
            || row.kind != Kind::Mod
            || row.source != SourceId::Sunrise
            || &row.project != id
            || sunrise::mod_id(&row) != Some(id.as_str())
            || claims.single(id).map_err(|e| e.render(crate::i18n::trf))? != Some(position)
        {
            return Err(unsettled());
        }
        claims
            .check_other_claims(home, id, file)
            .map_err(|e| e.render(crate::i18n::trf))?;
        let digest = index::sha256_file(&package.path)?;
        if row.version == package.manifest.version
            && row
                .checksum
                .eq_ignore_ascii_case(&format!("SHA-256:{digest}"))
        {
            continue;
        }
        let kind = if package.manifest.native.is_some() {
            index::Kind::NativeMod
        } else {
            index::Kind::Mod
        };
        // Adopt the actual profile input separately; never manufacture ownership for
        // a replacement from just its ID and location. Leave the old row opaque.
        if registry
            .and_then(|registry| registry.project(id))
            .filter(|project| project.kind == kind)
            .is_some_and(|project| {
                project.versions.iter().any(|release| {
                    release.version == package.manifest.version
                        && release.file.sha256.eq_ignore_ascii_case(&digest)
                })
            })
        {
            changes.push((
                position,
                package.manifest.version.clone(),
                format!("SHA-256:{digest}"),
            ));
        }
    }
    if changes.is_empty() {
        return Ok(None);
    }
    for (position, version, checksum) in changes {
        let raw = &mut claims.entries[position];
        raw["version"] = Value::from(version.clone());
        raw["number"] = Value::from(version);
        raw["checksum"] = Value::from(checksum);
    }
    *rows = claims.entries;
    serde_json::to_vec(&document)
        .map(Some)
        .map_err(|e| at(&path, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manifest_ids_detect_a_collision_between_unpacked_and_archived_content() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path();
        let unpacked = home.join(mod_list::DIR).join("local-folder");
        let body = format!("id='lantern'\nname='Lantern'\nversion='1.0.0'\napi='{}'\ncapabilities=['registries']\n", manifest::API);
        fs::create_dir_all(&unpacked).unwrap();
        fs::write(unpacked.join(manifest::MANIFEST), &body).unwrap();
        let inventory = scan_packages(home).unwrap();
        assert_eq!(inventory.len(), 1);
        assert!(inventory[0].directory);
        assert_eq!(inventory[0].manifest.id, "lantern");
        let before = inputs(home, None).unwrap();
        fs::write(unpacked.join("local-addition"), b"different bytes").unwrap();
        assert_ne!(inputs(home, None).unwrap(), before);
        let archive = home.join(mod_list::NATIVE_DIR).join("different-name.smod");
        fs::create_dir_all(archive.parent().unwrap()).unwrap();
        fs::write(archive, manifest::Package::write_files(&body, &[]).unwrap()).unwrap();
        assert!(scan_packages(home).is_err());
    }
}
