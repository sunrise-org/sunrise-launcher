//! Compare the content definitions carried by packages.
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::Path,
};

/// The kinds a content mod declares one file each for (`data/<namespace>/sunrise/<kind>/<id>.json`), as notices name them.
const DECLARED: [(&str, &str); 6] = [
    ("block", "block"),
    ("item", "item"),
    ("entity_type", "entity"),
    ("mob_effect", "effect"),
    ("menu", "menu"),
    ("creative_tab", "creative tab"),
];

/// The blocks, items, entities, effects, menus and creative tabs a content mod declares, read from its files' paths: the game checks their contents when it loads the mod.
fn content_ids(path: &Path) -> Result<BTreeSet<String>, String> {
    let manifest = sunrise_launcher_smod::Package::read(path)?.manifest;
    let names = if path.is_dir() {
        sunrise_launcher_smod::files::content_files(path)?
    } else {
        let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        zip::ZipArchive::new(file)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .file_names()
            .map(|name| {
                name.map(|name| name.into_owned())
                    .map_err(|e| format!("{}: {e}", path.display()))
            })
            .collect::<Result<_, _>>()?
    };
    let owned: BTreeSet<&str> = manifest.owned_namespaces().collect();
    Ok(names
        .iter()
        .filter_map(|name| {
            let rest = name.strip_prefix("data/")?;
            let (namespace, rest) = rest.split_once('/')?;
            let rest = rest.strip_prefix("sunrise/")?;
            let (kind, file) = rest.split_once('/')?;
            let id = file.strip_suffix(".json")?;
            let label = DECLARED.iter().find(|(dir, _)| *dir == kind)?.1;
            owned
                .contains(namespace)
                .then(|| format!("{label} {namespace}:{id}"))
        })
        .collect())
}

/// What the content mod at `old` has that the one at `new` lacks.
pub(super) fn dropped(old: &Path, new: &Path) -> Result<Vec<String>, String> {
    let new = content_ids(new)?;
    Ok(content_ids(old)?.difference(&new).cloned().collect())
}

/// Compare data definitions by path and bytes, rather than ZIP metadata or declaration IDs.
/// Paths have already been validated by the package inventory.
pub(super) fn definitions_changed(old: &Path, new: &Path) -> Result<bool, String> {
    fn definitions(path: &Path) -> Result<BTreeMap<String, [u8; 32]>, String> {
        let read = || -> io::Result<_> {
            let mut zip = zip::ZipArchive::new(fs::File::open(path)?).map_err(io::Error::other)?;
            let mut entries = BTreeMap::new();
            for i in 0..zip.len() {
                let mut entry = zip.by_index(i).map_err(io::Error::other)?;
                let name = entry.name().map_err(io::Error::other)?.into_owned();
                if !entry.is_dir() && name.starts_with("data/") {
                    let mut hash = Sha256::new();
                    io::copy(&mut entry, &mut hash)?;
                    entries.insert(name, hash.finalize().into());
                }
            }
            Ok(entries)
        };
        read().map_err(|error| format!("{}: {error}", path.display()))
    }
    Ok(definitions(old)? != definitions(new)?)
}
