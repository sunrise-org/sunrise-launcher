//! Recovery for legacy `mod-browser/replaced.json` backups written by earlier launchers'
//! automatic updater.
use crate::browser::{installed, SourceId};
use crate::i18n::tr;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::Path;
use sunrise_launcher_smod::index;

/// Where earlier launchers kept a replaced mod file until the next start, relative to the game folder.
const KEPT_DIR: &str = "mod-browser/replaced";
const KEPT: &str = "mod-browser/replaced.json";

/// A replaced file kept by an earlier launcher's automatic updater, with the entry it had.
#[derive(Clone, Debug, Deserialize)]
struct Kept {
    entry: installed::Entry,
    /// Its copy, relative to the game folder.
    copy: String,
}

fn kept_list(home: &Path) -> Result<Vec<Kept>, String> {
    let path = home.join(KEPT);
    let fail = |error: &dyn std::fmt::Display| format!("{}: {error}", path.display());
    let root = path.parent().expect("browser folder");
    match fs::symlink_metadata(root) {
        Ok(meta) if meta.is_dir() && regular(&meta) => (),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(fail(&e)),
        Ok(_) => return Err(fail(&io::ErrorKind::InvalidData)),
    }
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_file() && regular(&meta) => (),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(fail(&e)),
        Ok(_) => return Err(fail(&io::ErrorKind::InvalidData)),
    }
    let bytes = fs::read(&path).map_err(|e| fail(&e))?;
    let list: Vec<Kept> = serde_json::from_slice(&bytes).map_err(|e| fail(&e))?;
    let mut seen = BTreeSet::new();
    if list.iter().any(|row| {
        let first = format!("{KEPT_DIR}/{}", row.entry.project);
        let fresh = row
            .copy
            .strip_prefix(&first)
            .and_then(|rest| rest.strip_prefix('.'))
            .is_some_and(|random| {
                random.len() == 24 && random.bytes().all(|b| b.is_ascii_alphanumeric())
            });
        row.entry.source != SourceId::Sunrise
            || !index::valid_id(&row.entry.project)
            || (row.copy != first && !fresh)
            || !seen.insert(row.entry.project.as_str())
    }) {
        return Err(fail(&io::ErrorKind::InvalidData));
    }
    Ok(list)
}

fn regular(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    true
}

fn exact_package(home: &Path, path: &Path, checksum: &str) -> bool {
    let Some(hash) = checksum.strip_prefix("SHA-256:") else {
        return false;
    };
    let mut parent = path.parent();
    while let Some(dir) = parent.filter(|dir| *dir != home) {
        if !fs::symlink_metadata(dir).is_ok_and(|meta| meta.is_dir() && regular(&meta)) {
            return false;
        }
        parent = dir.parent();
    }
    parent == Some(home)
        && fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file() && regular(&meta))
        && index::sha256_file(path).is_ok_and(|actual| actual.eq_ignore_ascii_case(hash))
}

fn remove_kept(home: &Path) -> Result<(), String> {
    let path = home.join(KEPT);
    match fs::remove_file(&path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(format!("{}: {e}", path.display())),
        _ => Ok(()),
    }
}

fn lock_for_updates(home: &Path) -> Result<crate::browser::HomeLockGuard, String> {
    crate::browser::lock_home(home).map_err(|error| {
        crate::elog!(
            "Sunrise launcher: could not lock registry updates for {}: {error}",
            home.display()
        );
        tr("The launcher couldn't check for unfinished mod updates. You can still play.").into()
    })
}

/// Retire legacy `mod-browser/replaced.json` entries only when every recorded entry and live
/// package is proved intact. Unreadable or ambiguous evidence stays in place so Play can proceed
/// without losing a backup.
pub fn recover(home: &Path) -> Result<(), String> {
    let _lock = lock_for_updates(home)?;
    installed::recover_game_install(home).map_err(|problem| problem.summary().to_owned())?;
    let notice = || {
        tr("A backup from an older mod update was kept because it couldn't be checked. You can still play.").to_owned()
    };
    let kept = kept_list(home).map_err(|_| notice())?;
    if kept.is_empty() {
        return Ok(());
    }
    // The usual display reader drops unknown or malformed claims. No dropped claim may
    // be treated as proof that another row alone owns these bytes.
    #[derive(Deserialize)]
    struct Claims {
        entries: Vec<installed::Entry>,
    }
    let manifest = home.join(installed::MANIFEST);
    if !fs::symlink_metadata(&manifest).is_ok_and(|meta| meta.is_file() && regular(&meta)) {
        return Err(notice());
    }
    let bytes = fs::read(&manifest).map_err(|_| notice())?;
    let claims: Claims = serde_json::from_slice(&bytes).map_err(|_| notice())?;
    if claims
        .entries
        .iter()
        .any(|claim| claim.path(home).is_none())
    {
        return Err(notice());
    }
    for old in &kept {
        let mut rows = claims
            .entries
            .iter()
            .filter(|row| row.source == SourceId::Sunrise && row.project == old.entry.project);
        let current = rows.next().filter(|_| rows.next().is_none());
        if !current.is_some_and(|row| {
            row.path(home)
                .is_some_and(|path| exact_package(home, &path, &row.checksum))
        }) || !exact_package(home, &home.join(&old.copy), &old.entry.checksum)
        {
            return Err(notice());
        }
    }
    remove_kept(home).map_err(|_| notice())?;
    // No longer referenced. Retain even proved copies in this standalone upgrade step;
    // the old format doesn't bind directory identity or provide a retirement barrier.
    Ok(())
}

#[cfg(test)]
mod tests;
