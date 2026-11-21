//! Conservative proof for preserving saved mod definitions across a staged edit.
use crate::managed_play::{self, Active, ScannedPackage};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Read},
    path::Path,
};
use sunrise_launcher_smod::{dependencies, files, manifest, mod_list};

/// Previously available mod IDs and their namespaces whose effects are not proved unchanged.
pub type SuspectMods = BTreeMap<String, BTreeSet<String>>;

/// The namespaces in which a flat content replacement drops a registered ID.
/// An opaque or unreadable side says nothing about which IDs survive.
pub(crate) fn update_drops(old: &ScannedPackage, new: &ScannedPackage) -> Option<BTreeSet<String>> {
    let definitions = |package: &ScannedPackage| {
        if package.directory
            || package.manifest.native.is_some()
            || !package.manifest.jar.is_empty()
        {
            return None;
        }
        let owned: BTreeSet<_> = package.manifest.owned_namespaces().collect();
        Some(
            content_bytes(&package.path)?
                .into_keys()
                .filter(|path| {
                    owned.contains(path.split('/').nth(1).expect("definition namespace"))
                })
                .collect::<BTreeSet<_>>(),
        )
    };
    let old = definitions(old)?;
    let new = definitions(new)?;
    Some(
        old.difference(&new)
            .map(|path| {
                path.split('/')
                    .nth(1)
                    .expect("definition namespace")
                    .to_owned()
            })
            .collect(),
    )
}

/// Compare admitted bases, the selected package environment, and the old packages explicitly
/// replaced or removed (which may already be off). A missing base is not evidence of equality.
/// Failure to prove equivalence leaves the old IDs suspect rather than clearing their warnings.
pub fn unproved_mod_effects<'a>(
    before: &Path,
    before_base: Option<&Active>,
    after: &Path,
    after_base: Option<&Active>,
    changed: impl IntoIterator<Item = &'a str>,
) -> Result<SuspectMods, String> {
    let previous = managed_play::scan_packages(before)?;
    let changed: BTreeSet<_> = changed.into_iter().collect();
    let old_on = enabled(before)?;
    let mut watched: SuspectMods = previous
        .iter()
        .filter(|p| old_on.contains(&p.manifest.id) || changed.contains(p.manifest.id.as_str()))
        .map(|p| {
            (
                p.manifest.id.clone(),
                p.manifest.owned_namespaces().map(str::to_owned).collect(),
            )
        })
        .collect();
    if watched.is_empty()
        || !before_base
            .zip(after_base)
            .is_some_and(|(a, b)| a.same_base(b))
    {
        return Ok(watched);
    }
    let next = managed_play::scan_packages(after)?;
    let new_on = enabled(after)?;
    let previous = previous
        .iter()
        .filter(|p| old_on.contains(&p.manifest.id))
        .collect::<Vec<_>>();
    let next = next
        .iter()
        .filter(|p| new_on.contains(&p.manifest.id))
        .collect::<Vec<_>>();
    if let Some(preserved) = preserved_mods(before, after, &previous, &next) {
        watched.retain(|id, _| !preserved.contains(id.as_str()));
    }
    Ok(watched)
}

fn enabled(home: &Path) -> Result<BTreeSet<String>, String> {
    let path = home.join(mod_list::FILE);
    match fs::read_to_string(&path) {
        Ok(text) => Ok(mod_list::ModList::parse(&text).enabled),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(BTreeSet::new()),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

fn preserved_mods<'a>(
    before: &Path,
    after: &Path,
    previous: &[&'a ScannedPackage],
    next: &[&'a ScannedPackage],
) -> Option<BTreeSet<&'a str>> {
    let before_mods = before.join(mod_list::DIR);
    let after_mods = after.join(mod_list::DIR);
    if overrides_bytes(&before_mods)? != overrides_bytes(&after_mods)? {
        return None;
    }
    let overrides = dependencies::Overrides::read(&before_mods).ok()?;
    let old: Vec<_> = previous.iter().map(|p| &p.manifest).collect();
    let new: Vec<_> = next.iter().map(|p| &p.manifest).collect();
    let old_packages: BTreeMap<_, _> = previous
        .iter()
        .map(|p| (p.manifest.id.as_str(), *p))
        .collect();
    let new_packages: BTreeMap<_, _> = next.iter().map(|p| (p.manifest.id.as_str(), *p)).collect();
    let mut reordered = BTreeSet::new();
    for side in [manifest::Side::Client, manifest::Side::Server] {
        let a = dependencies::resolve_manifests(&old, side, &overrides).ok()?;
        let b = dependencies::resolve_manifests(&new, side, &overrides).ok()?;
        let ranks: BTreeMap<_, _> = b
            .order
            .into_iter()
            .map(|i| new[i].id.as_str())
            .filter(|id| old_packages.contains_key(id))
            .enumerate()
            .map(|(i, id)| (id, i))
            .collect();
        let mut prefix_max: Option<usize> = None;
        for (index, id) in a
            .order
            .into_iter()
            .map(|i| old[i].id.as_str())
            .filter(|id| ranks.contains_key(id))
            .enumerate()
        {
            let rank = ranks[id];
            // Ignore the positions of added/removed mods, but require the same set of
            // surviving predecessors; an equal numeric rank alone doesn't prove that.
            if rank != index || prefix_max.is_some_and(|previous| previous > rank) {
                reordered.insert(id);
            }
            prefix_max = Some(prefix_max.map_or(rank, |previous| previous.max(rank)));
        }
    }
    let mut preserved = BTreeSet::new();
    let mut changed = BTreeSet::new();
    let mut shadows: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    let contents = |p: Option<&ScannedPackage>| match p {
        None => Some(BTreeMap::new()),
        Some(p) if !p.directory && p.manifest.native.is_none() && p.manifest.jar.is_empty() => {
            content_bytes(&p.path)
        }
        Some(_) => None,
    };
    for &id in old_packages.keys().chain(
        new_packages
            .keys()
            .filter(|id| !old_packages.contains_key(*id)),
    ) {
        let a = old_packages.get(id).copied();
        let b = new_packages.get(id).copied();
        let pair = a.zip(b);
        let same_position = !reordered.contains(id)
            && pair.is_some_and(|(a, b)| {
                a.path
                    .strip_prefix(before)
                    .ok()
                    .zip(b.path.strip_prefix(after).ok())
                    .is_some_and(|(a, b)| a == b)
            });
        let (Some(old_files), Some(new_files)) = (contents(a), contents(b)) else {
            // An opaque layer can supply definitions in any namespace. Exact equality
            // limits this uncertainty to the layer itself; it does not clear that layer.
            let digest = |p: &Path| opaque_digest(p, 0, &mut (files::MAX_FILES, files::MAX_TOTAL));
            if !same_position
                || !pair.is_some_and(|(a, b)| {
                    digest(&a.path)
                        .zip(digest(&b.path))
                        .is_some_and(|(a, b)| a == b)
                })
            {
                return None;
            }
            continue;
        };
        let comparable = same_position
            && pair.is_some_and(|(a, b)| equivalent_manifest(&a.manifest, &b.manifest));
        let mut same_definitions = comparable;
        // If placement or declarations changed, even unchanged paths may become visible.
        for (name, _) in old_files
            .iter()
            .filter(|(name, digest)| !comparable || new_files.get(*name) != Some(*digest))
            .chain(
                new_files
                    .iter()
                    .filter(|(name, _)| !old_files.contains_key(*name)),
            )
        {
            // `content_bytes` keeps only registered paths; never use a visual layer as
            // evidence that a package can change another namespace's saved definitions.
            let namespace = name.split('/').nth(1).expect("content namespace");
            same_definitions = false;
            if matches!(namespace, "minecraft" | "c") {
                // A shared registration can change any mod's saved definitions.
                return None;
            }
            shadows.entry(namespace.into()).or_default().insert(id);
        }
        if same_definitions {
            preserved.insert(id);
        }
        // A leaf can have a new release with the same definitions. Its consumers still
        // see a changed version when they resolve a dependency or compatibility rule.
        if !same_definitions || pair.is_some_and(|(a, b)| a.manifest.version != b.manifest.version)
        {
            changed.insert(id);
        }
    }
    let mut consumers: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for m in old.into_iter().chain(new) {
        if m.owned_namespaces().any(|ns| {
            shadows
                .get(ns)
                .is_some_and(|writers| writers.iter().any(|&writer| writer != m.id.as_str()))
        }) {
            preserved.remove(m.id.as_str());
            changed.insert(m.id.as_str());
        }
        for target in m
            .dependencies
            .keys()
            .chain(m.incompatible.keys())
            .chain(m.discouraged.keys())
        {
            consumers.entry(target).or_default().push(&m.id);
        }
    }
    let mut pending: Vec<_> = changed.iter().copied().collect();
    while let Some(id) = pending.pop() {
        for consumer in consumers.get(id).into_iter().flatten().copied() {
            preserved.remove(consumer);
            if changed.insert(consumer) {
                pending.push(consumer);
            }
        }
    }
    Some(preserved)
}

fn equivalent_manifest(a: &manifest::Manifest, b: &manifest::Manifest) -> bool {
    let mut a = a.clone();
    // These fields describe a content archive to people. The dependency comparison
    // separately checks version uses. Comparing the rest keeps new semantic fields in scope.
    a.name.clone_from(&b.name);
    a.version.clone_from(&b.version);
    a.description.clone_from(&b.description);
    a.authors.clone_from(&b.authors);
    a.license.clone_from(&b.license);
    a.license_url.clone_from(&b.license_url);
    a.credits.clone_from(&b.credits);
    a.homepage.clone_from(&b.homepage);
    a.issues.clone_from(&b.issues);
    a == *b
}

fn overrides_bytes(mods: &Path) -> Option<Option<Vec<u8>>> {
    let path = mods.join(dependencies::OVERRIDES);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() && metadata.len() <= manifest::MAX_MANIFEST => {
            let mut bytes = Vec::new();
            fs::File::open(path)
                .ok()?
                .take(manifest::MAX_MANIFEST + 1)
                .read_to_end(&mut bytes)
                .ok()?;
            (bytes.len() as u64 <= manifest::MAX_MANIFEST).then_some(Some(bytes))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Some(None),
        _ => None,
    }
}

fn content_bytes(path: &Path) -> Option<BTreeMap<String, [u8; 32]>> {
    let mut zip = zip::ZipArchive::new(fs::File::open(path).ok()?).ok()?;
    let mut contents = BTreeMap::new();
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).ok()?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().ok()?.into_owned();
        if matches!(
            name.as_str(),
            manifest::MANIFEST | "icon.png" | sunrise_launcher_smod::layouts::FILE
        ) {
            continue;
        }
        // Unknown package payloads may affect the game; the remainder must be bounded
        // flat content. Compare only entries that the game saves by registry ID.
        files::check_path(&name).ok()?;
        if entry.size() > files::MAX_FILE || i >= files::MAX_FILES {
            return None;
        }
        total = total.checked_add(entry.size())?;
        if total > files::MAX_TOTAL {
            return None;
        }
        // SMOD declares registry entries as data/<namespace>/sunrise/<kind>/<id>.json
        // (see the kind list in browser::sunrise::auto). Menus and creative tabs are not
        // saved. Models, blockstate maps, textures, tags and feature placement do not
        // change the serialized definition (e.g. the chunk block state palette).
        let mut parts = name.splitn(5, '/');
        if !matches!(
            (parts.next(), parts.nth(1), parts.next(), parts.next()),
            (
                Some("data"),
                Some("sunrise"),
                Some("block" | "item" | "entity_type" | "block_entity_type" | "mob_effect"),
                Some(file),
            ) if file.ends_with(".json")
        ) {
            continue;
        }
        let mut digest = Sha256::new();
        let n = io::copy(&mut entry.by_ref().take(files::MAX_FILE + 1), &mut digest).ok()?;
        if n != entry.size() || contents.insert(name, digest.finalize().into()).is_some() {
            return None;
        }
    }
    Some(contents)
}

// Bound raw archive and directory proofs as well as the decompressed flat proof. Tree
// entries commit to names and types as well as bytes, and never follow a linked node.
fn opaque_digest(path: &Path, depth: usize, budget: &mut (usize, u64)) -> Option<[u8; 32]> {
    if depth > 64 {
        return None;
    }
    let meta = fs::symlink_metadata(path).ok()?;
    if crate::native_cache::linked(&meta) {
        return None;
    }
    let (entries_left, bytes_left) = budget;
    let mut hash = Sha256::new();
    if meta.is_file() {
        hash.update(b"file");
        let len = meta.len();
        *bytes_left = bytes_left.checked_sub(len)?;
        if io::copy(&mut fs::File::open(path).ok()?.take(len + 1), &mut hash).ok()? != len {
            return None;
        }
    } else if meta.is_dir() {
        hash.update(b"dir");
        let mut entries = fs::read_dir(path)
            .ok()?
            .take(*entries_left + 1)
            .collect::<io::Result<Vec<_>>>()
            .ok()?;
        *entries_left = entries_left.checked_sub(entries.len())?;
        entries.sort_unstable_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let name = entry.file_name();
            let name = name.as_encoded_bytes();
            hash.update((name.len() as u64).to_le_bytes());
            hash.update(name);
            hash.update(opaque_digest(&entry.path(), depth + 1, budget)?);
        }
    } else {
        return None;
    }
    Some(hash.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(id: &str) -> String {
        format!("id='{id}'\nname='Test'\nversion='1.0.0'\napi='{}'\nside='both'\ncapabilities=['registries']\n", manifest::API)
    }

    fn content(home: &Path, id: &str, namespaces: &str, files: &[(&str, &[u8])]) {
        let bytes = manifest::Package::write_files(
            &format!("{}namespaces=[{namespaces}]\n", header(id)),
            files,
        )
        .unwrap();
        fs::write(home.join(mod_list::DIR).join(format!("{id}.smod")), bytes).unwrap();
    }

    fn compare(before: &Path, after: &Path) -> Option<BTreeSet<String>> {
        let old = managed_play::scan_packages(before).unwrap();
        let new = managed_play::scan_packages(after).unwrap();
        preserved_mods(
            before,
            after,
            &old.iter().collect::<Vec<_>>(),
            &new.iter().collect::<Vec<_>>(),
        )
        .map(|ids| ids.into_iter().map(str::to_owned).collect())
    }

    fn shadow_profiles() -> [tempfile::TempDir; 2] {
        std::array::from_fn(|_| {
            let home = tempfile::tempdir().unwrap();
            fs::create_dir(home.path().join(mod_list::DIR)).unwrap();
            content(
                home.path(),
                "lamp",
                "'lighting'",
                &[("data/lighting/sunrise/block/lamp.json", b"{}")],
            );
            content(home.path(), "unrelated", "", &[]);
            home
        })
    }

    #[test]
    fn a_foreign_definition_taints_its_namespace_owners_only() {
        let [before, after] = shadow_profiles();
        let path = "data/lighting/sunrise/block/lamp.json";
        let preserved = || compare(before.path(), after.path());
        let independent = Some(BTreeSet::from(["unrelated".into()]));
        content(after.path(), "overlay", "'lighting'", &[(path, b"new")]);
        assert_eq!(preserved(), independent, "added overlay");
        content(before.path(), "overlay", "'lighting'", &[(path, b"old")]);
        assert_eq!(preserved(), independent, "changed overlay");
        fs::remove_file(after.path().join(mod_list::DIR).join("overlay.smod")).unwrap();
        assert_eq!(preserved(), independent, "removed overlay");
    }

    #[test]
    fn a_leaf_changing_only_tags_worldgen_or_an_atlas_preserves_every_mod() {
        let [before, after] = shadow_profiles();
        let definition = ("data/lighting/sunrise/block/lamp.json", b"same".as_slice());
        content(before.path(), "overlay", "'lighting'", &[definition]);
        content(
            after.path(),
            "overlay",
            "'lighting'",
            &[
                definition,
                ("data/minecraft/tags/block/mineable/pickaxe.json", b"{}"),
                ("data/c/tags/block/lights.json", b"{}"),
                (
                    "data/lighting/worldgen/configured_feature/lamps.json",
                    b"{}",
                ),
                ("assets/minecraft/atlases/blocks.json", b"{}"),
            ],
        );
        assert_eq!(
            compare(before.path(), after.path()),
            Some(BTreeSet::from([
                "lamp".into(),
                "overlay".into(),
                "unrelated".into()
            ]))
        );
    }

    #[test]
    fn changing_models_textures_and_blockstate_maps_leaves_no_mod_suspect() {
        let [before, after] = shadow_profiles();
        let files: &[(&str, &[u8])] = &[
            ("assets/lighting/models/block/lamp.json", b"{}"),
            ("assets/lighting/textures/block/lamp.png", b"pix"),
            ("assets/lighting/blockstates/lamp.json", b"{}"),
        ];
        content(before.path(), "overlay", "'lighting'", &[]);
        content(after.path(), "overlay", "'lighting'", files);
        assert_eq!(
            compare(before.path(), after.path()),
            Some(BTreeSet::from([
                "lamp".into(),
                "overlay".into(),
                "unrelated".into()
            ]))
        );
    }

    #[test]
    fn shared_definition_paths_have_no_single_mod_owner() {
        let [before, after] = shadow_profiles();
        for path in [
            "data/minecraft/sunrise/block/lantern.json",
            "data/c/sunrise/item/lantern.json",
        ] {
            content(after.path(), "overlay", "", &[(path, b"{}")]);
            assert_eq!(compare(before.path(), after.path()), None, "{path}");
        }
    }

    #[test]
    fn stable_opaque_layers_stay_suspect_without_tainting_peers() {
        for kind in ["native", "jar", "directory"] {
            let [before, after] = shadow_profiles();
            let root = if kind == "native" {
                mod_list::NATIVE_DIR
            } else {
                mod_list::DIR
            };
            let manifest = header("opaque");
            let fixture = |home: &Path, payload: &'static [u8]| {
                let root = home.join(root);
                fs::create_dir_all(&root).unwrap();
                let archive = match kind {
                    "native" => manifest::Package::write_files(
                        &format!(
                            "{manifest}[native]\nsunrise='{}'\ngame='26.3'\n",
                            "a".repeat(40)
                        ),
                        &[
                            ("Cargo.toml", b"[package]\nname='opaque'\nversion='1.0.0'"),
                            ("src/lib.rs", payload),
                        ],
                    ),
                    "jar" => manifest::Package::write_files(
                        &format!(
                            "{manifest}[[jar]]\nid='other_mod'\nversion='1'\nsha512='{}'\n",
                            "a".repeat(128)
                        ),
                        &[("data/opaque/tags/block/test.json", payload)],
                    ),
                    _ => {
                        let folder = root.join("opaque");
                        let data = folder.join("data/opaque/sunrise/block");
                        fs::create_dir_all(&data).unwrap();
                        fs::write(folder.join(manifest::MANIFEST), &manifest).unwrap();
                        fs::write(data.join("lamp.json"), payload).unwrap();
                        return;
                    }
                };
                fs::write(root.join("opaque.smod"), archive.unwrap()).unwrap();
            };
            fixture(before.path(), b"old");
            assert_eq!(compare(before.path(), after.path()), None, "removed {kind}");
            if kind == "directory" {
                fixture(after.path(), b"old");
            } else {
                fs::create_dir_all(after.path().join(root)).unwrap();
                fs::copy(
                    before.path().join(root).join("opaque.smod"),
                    after.path().join(root).join("opaque.smod"),
                )
                .unwrap();
            }
            assert_eq!(
                compare(before.path(), after.path()),
                Some(BTreeSet::from(["lamp".into(), "unrelated".into()])),
                "unchanged {kind} stays suspect but its peers don't"
            );
            fixture(after.path(), b"new");
            assert_eq!(compare(before.path(), after.path()), None, "changed {kind}");
        }
    }

    #[test]
    fn same_paths_and_compatible_ranges_do_not_prove_the_same_definitions() {
        let before = tempfile::tempdir().unwrap();
        let after = tempfile::tempdir().unwrap();
        let root = "root_mod";
        let dep = "required_mod";
        let defs = [(
            "data/root_mod/sunrise/block/lamp.json",
            br#"{"properties":{"lit":[true,false]}}"#.as_slice(),
        )];
        let write = |home: &Path, id: &str, version: &str, data: &[(&str, &[u8])]| {
            let rule = if id == root {
                "[dependencies]\nrequired_mod='^1'\n"
            } else {
                ""
            };
            let bytes = manifest::Package::write_files(
                &format!("id='{id}'\nname='Test'\nversion='{version}'\napi='{}'\nside='both'\ncapabilities=['registries']\n{rule}", manifest::API),
                data,
            ).unwrap();
            fs::write(home.join(mod_list::DIR).join(format!("{id}.smod")), bytes).unwrap();
        };
        for home in [before.path(), after.path()] {
            fs::create_dir(home.join(mod_list::DIR)).unwrap();
            crate::browser::mod_list::set_all_enabled(home, &[root, dep], true).unwrap();
            write(home, root, "1.0.0", &defs);
            write(home, dep, "1.0.0", &[]);
        }
        let ids = BTreeSet::from([dep.to_owned(), root.to_owned()]);
        let watched: SuspectMods = ids
            .iter()
            .map(|id| (id.clone(), BTreeSet::from([id.clone()])))
            .collect();
        let preserved = || compare(before.path(), after.path());
        write(after.path(), root, "1.1.0", &defs);
        assert_eq!(preserved(), Some(ids));
        write(after.path(), dep, "1.1.0", &[]);
        assert_eq!(preserved(), Some(BTreeSet::from([dep.into()])));
        write(after.path(), dep, "1.0.0", &[]);
        let overrides = after
            .path()
            .join(mod_list::DIR)
            .join(dependencies::OVERRIDES);
        fs::write(&overrides, "[root_mod]\nrequired_mod='drop'\n").unwrap();
        assert_eq!(preserved(), None);
        fs::remove_file(overrides).unwrap();
        write(after.path(), root, "1.1.0", &[(defs[0].0, b"{}")]);
        assert_eq!(preserved(), Some(BTreeSet::from([dep.into()])));
        // Byte-equal archives still cannot testify to equivalence on an unproved base.
        assert_eq!(
            unproved_mod_effects(before.path(), None, before.path(), None, []).unwrap(),
            watched
        );
    }
}
