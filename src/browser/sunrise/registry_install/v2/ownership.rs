//! Who owns an installed package (a v2 project, a v1 row or the player), revoked releases, and the v2 inventory of a profile.

use super::*;
use crate::managed_play::{
    classify_v2_ownership, has_source_origin, revoked_for_installed_package,
};

/// Turn off any enabled mods in `home` revoked by the signed v2 index, leaving their files on disk.
pub fn turn_off_revoked_v2(index: &Index, home: &Path) -> Result<Vec<String>, String> {
    let _lock = crate::browser::lock_home(home).map_err(|e| e.to_string())?;
    installed::recover_game_install(home)?;
    let packages = managed_play::scan_packages(home)?;
    let rows = installed::Installed::read(home)?;
    let mut notices = Vec::new();
    for package in &packages {
        if package.directory {
            continue;
        }
        let digest = index::sha256_file(&package.path)?;
        let Some(revoked) = revoked_for_installed_package(index, &rows, home, package, &digest)
        else {
            continue;
        };
        if mod_list::is_enabled(home, &package.manifest.id) {
            mod_list::set_enabled(home, &package.manifest.id, false)?;
            notices.push(trf(
                "{mod} was turned off because it was taken down: {reason}",
                &[("mod", &package.manifest.name), ("reason", &revoked.reason)],
            ));
        }
    }
    for entry in &rows.entries {
        if entry.source != SourceId::Sunrise
            || !matches!(entry.kind, Kind::ResourcePack | Kind::Shader)
        {
            continue;
        }
        let revoked = if index_v2::valid_project_id(&entry.project) {
            index.revoked(&entry.project, entry.sha256())
        } else {
            entry
                .sha256()
                .and_then(|hash| index.revoked_by_sha256(hash))
        };
        if let Some(revoked) = revoked {
            notices.push(trf(
                "{mod} was taken down: {reason} Remove it.",
                &[("mod", &entry.title), ("reason", &revoked.reason)],
            ));
        }
    }
    Ok(notices)
}

/// Scan every installed package in `home` and classify its verified v2 ownership against `index`.
pub fn inventory_v2(
    home: &Path,
    index: &Index,
) -> Result<BTreeMap<String, InstalledV2Mod>, String> {
    let packages = managed_play::scan_packages(home)?;
    let rows = installed::Installed::read(home)?;
    let mut by_slot: BTreeMap<String, &installed::Entry> = BTreeMap::new();
    for row in &rows.entries {
        if let Some(slot) = row.in_game_mod_id() {
            if by_slot.insert(slot.to_owned(), row).is_some() {
                return Err(trf(
                    "Several entries in {manifest} name {id}.",
                    &[("manifest", installed::MANIFEST), ("id", slot)],
                ));
            }
        }
    }
    let mut out = BTreeMap::new();
    for package in packages {
        let id = package.manifest.id.clone();
        let enabled = mod_list::is_enabled(home, &id);
        let (digest, size) = if package.directory {
            (String::new(), 0)
        } else {
            (
                index::sha256_file(&package.path)?,
                fs::metadata(&package.path)
                    .map_err(|e| at(&package.path, e))?
                    .len(),
            )
        };
        let has_github = has_source_origin(home, &package);
        let ownership = classify_v2_ownership(
            index,
            home,
            by_slot.get(&id).copied(),
            &package,
            &digest,
            size,
            has_github,
        );
        out.insert(
            id,
            InstalledV2Mod {
                package,
                digest,
                size,
                enabled,
                ownership,
            },
        );
    }
    Ok(out)
}
