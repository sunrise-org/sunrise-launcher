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
#[derive(Clone, Debug)]
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
            Ok(_) => return Err(at(&root, unsettled!())),
        };
        for entry in entries {
            let path = entry.map_err(|e| at(&root, e))?.path();
            let meta = fs::symlink_metadata(&path).map_err(|e| at(&path, e))?;
            if !plain_file(&meta) {
                return Err(at(&path, unsettled!()));
            }
            if path.extension().is_none_or(|ext| ext != "smod") {
                if !meta.is_dir() {
                    continue;
                }
                let path = path.join(manifest::MANIFEST);
                match fs::symlink_metadata(&path) {
                    Ok(meta) if meta.is_file() && plain_file(&meta) => (),
                    Ok(_) => return Err(at(&path, unsettled!())),
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
pub(crate) fn inputs(home: &Path) -> Result<[u8; 32], String> {
    require_dir(home)?;
    let mut hash = Sha256::new();
    let browser = home.join("mod-browser");
    match fs::symlink_metadata(&browser) {
        Ok(meta) if meta.is_dir() && plain_file(&meta) => (),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (),
        Err(e) => return Err(at(&browser, e)),
        Ok(_) => return Err(at(&browser, unsettled!())),
    }
    for rel in [
        mod_list::DIR,
        mod_list::NATIVE_DIR,
        mod_list::FILE,
        sunrise_launcher_smod::index::browser_state::MANIFEST,
    ] {
        hash.update(rel.as_bytes());
        hash.update([0]);
        if matches!(rel, mod_list::DIR | mod_list::NATIVE_DIR) {
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

use sunrise_launcher_smod::index::browser_state::records::Entry as Row;

/// The in-game mod ID a mod row's `file` names, under the mods or native mods folder.
fn row_mod_id(file: &str) -> Option<&str> {
    if let Some(rest) = file.strip_prefix(mod_list::NATIVE_DIR) {
        return rest.strip_prefix('/')?.strip_suffix(".smod");
    }
    let rest = file.strip_prefix(mod_list::DIR)?.strip_prefix('/')?;
    Some(rest.strip_suffix(".smod").unwrap_or(rest))
}

/// Each registry row that names a mod file must claim exactly that package, once. These rows are
/// game-facing metadata, never destructive authority for the launcher.
pub(super) fn check_browser_rows(home: &Path, packages: &[ScannedPackage]) -> Result<(), String> {
    use serde_json::Value;
    let path = home.join(index::browser_state::MANIFEST);
    let Some(bytes) = super::optional_bytes(&path)? else {
        return Ok(());
    };
    let mut document: serde_json::Map<String, Value> =
        serde_json::from_slice(&bytes).map_err(|e| at(&path, e))?;
    let entries = match document.remove("entries") {
        Some(Value::Array(entries)) => entries,
        _ => return Err(unsettled!()),
    };
    let claims = index::browser_state::Installed { entries };
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
        let row: Row = serde_json::from_value(raw.clone()).map_err(|e| at(&path, e))?;
        let package = by_file.get(relative).ok_or_else(|| unsettled!())?;
        let id = &package.manifest.id;
        let id_ok = if row.mod_id.is_some() {
            row.mod_id.as_deref() == Some(id.as_str()) && row.is_v2_registry()
        } else {
            &row.project == id
        };
        if package.directory
            || !index::browser_state::in_folders(file)
            || row.kind != index::browser_state::Kind::Mod
            || row.source != index::browser_state::SourceId::Sunrise
            || !id_ok
            || row_mod_id(&row.file) != Some(id.as_str())
            || claims.single(id).map_err(|e| e.render(crate::i18n::trf))? != Some(position)
        {
            return Err(unsettled!());
        }
        claims
            .check_other_claims(home, id, file)
            .map_err(|e| e.render(crate::i18n::trf))?;
    }
    Ok(())
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
        let before = inputs(home).unwrap();
        fs::write(unpacked.join("local-addition"), b"different bytes").unwrap();
        assert_ne!(inputs(home).unwrap(), before);
        let archive = home.join(mod_list::NATIVE_DIR).join("different-name.smod");
        fs::create_dir_all(archive.parent().unwrap()).unwrap();
        fs::write(archive, manifest::Package::write_files(&body, &[]).unwrap()).unwrap();
        assert!(scan_packages(home).is_err());
    }
}
