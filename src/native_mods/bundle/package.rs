//! Validated flat packages: copied from the game's `sunrise-native-build` (tests/test_launcher_copies.py).
use super::Bundle;
use crate::i18n::{tr, trf};
use std::{fs, path::Path};
use sunrise_launcher_smod::manifest::USES_FILE;
use sunrise_launcher_smod::{manifest, Package};

/// The validated native project, if this directory has a package manifest. A present but
/// invalid manifest is reported by the package reader.
pub fn read(dir: &Path) -> Result<Option<Bundle>, String> {
    let path = dir.join(manifest::MANIFEST);
    match fs::symlink_metadata(&path) {
        Ok(_) => {}
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(None);
        }
        Err(e) => return Err(format!("{}: {e}", path.display())),
    }
    let package = Package::read(dir)?;
    if !package.is_native() {
        return Ok(None);
    }
    if let Some(state) = package
        .manifest
        .native
        .as_ref()
        .and_then(|native| native.state.first())
    {
        return Err(trf(
            "E_MIXIN_STATE_LAYOUT: {mod}:{state}: native state changes the prebuilt game's layout; use an attachment instead",
            &[("mod", &package.manifest.id), ("state", &state.id)],
        ));
    }
    package.directory_files(dir)?;
    Ok(Some(Bundle {
        manifest: package.manifest,
        dir: dir.to_owned(),
    }))
}

fn checked_files(bundle: &Bundle) -> Result<std::collections::BTreeMap<String, Vec<u8>>, String> {
    Package::read(&bundle.dir)?.directory_files(&bundle.dir)
}

pub fn files(bundle: &Bundle) -> Result<Vec<(String, std::path::PathBuf)>, String> {
    Ok(checked_files(bundle)?
        .into_keys()
        .map(|name| {
            let path = bundle.dir.join(&name);
            (name, path)
        })
        .collect())
}

/// The package's files and its uses table, which is judged against another build as the package was.
pub fn copy(bundle: &Bundle, to: &Path) -> Result<(), String> {
    let mut files = checked_files(bundle)?;
    if let Ok(table) = fs::read(bundle.dir.join(USES_FILE)) {
        files.insert(USES_FILE.to_owned(), table);
    }
    copy_files(to, files)
}

fn copy_files(to: &Path, files: impl IntoIterator<Item = (String, Vec<u8>)>) -> Result<(), String> {
    for (name, bytes) in files {
        let path = to.join(name);
        fs::create_dir_all(path.parent().ok_or(tr("invalid destination"))?)
            .map_err(|e| e.to_string())?;
        fs::write(path, bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
