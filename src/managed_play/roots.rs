//! Canonical game homes that have published an artifact reference into this shared cache.
//! The cache publication fence serializes roster changes, pointers/attempts, and deletion.
use super::{at, optional_bytes, plain_file, require_dir, unsettled, Active};
use cap_fs_ext::MetadataExt;
use cap_std::{ambient_authority, fs::Dir};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

const FILE: &str = "profile-artifact-roots.json";

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    volume: u64,
    entry: u64,
}

impl Identity {
    fn of(path: &Path) -> Result<Self, String> {
        require_dir(path)?;
        let metadata = Dir::open_ambient_dir(path, ambient_authority())
            .and_then(|dir| dir.dir_metadata())
            .map_err(|e| at(path, e))?;
        Ok(Self {
            volume: metadata.dev(),
            entry: metadata.ino(),
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Roster {
    version: u32,
    roots: Vec<(PathBuf, Identity)>,
}

fn read(cache: &Path) -> Result<BTreeMap<PathBuf, Identity>, String> {
    let path = cache.join(FILE);
    let Some(bytes) = optional_bytes(&path)? else {
        return Ok(BTreeMap::new());
    };
    let roster: Roster = serde_json::from_slice(&bytes).map_err(|e| at(&path, e))?;
    let mut roots = BTreeMap::new();
    if roster.version != 1 {
        return Err(at(&path, unsettled()));
    }
    for (root, id) in roster.roots {
        if !root.is_absolute() || roots.insert(root, id).is_some() {
            return Err(at(&path, unsettled()));
        }
    }
    Ok(roots)
}

/// Register a true managed home before it has any pending or visible reference. Hold the
/// installation publication fence. The caller already holds the individual profile gate.
pub(super) fn register(profile: &Path) -> Result<(), String> {
    let game_home = crate::backend::game_home();
    let root = game_home.canonicalize().map_err(|e| at(&game_home, e))?;
    let profile = profile.canonicalize().map_err(|e| at(profile, e))?;
    let relative = profile.strip_prefix(&root).map_err(|e| at(&profile, e))?;
    let parts: Vec<_> = relative.iter().collect();
    match parts.as_slice() {
        [] => (),
        [collection, _] if *collection == "instances" => (),
        [collection, _, home] if *collection == "sunrise-packs" && *home == "home" => (),
        _ => return Err(at(&profile, unsettled())),
    }
    register_root(&crate::backend::data_dir(), root)
}

fn register_root(cache: &Path, root: PathBuf) -> Result<(), String> {
    let mut roots = read(cache)?;
    let id = Identity::of(&root)?;
    match roots.get(&root) {
        Some(previous) if *previous == id => return Ok(()),
        Some(_) => return Err(at(&root, unsettled())),
        None => (),
    }
    roots.insert(root, id);
    let path = cache.join(FILE);
    let bytes = serde_json::to_vec(&Roster {
        version: 1,
        roots: roots.into_iter().collect(),
    })
    .map_err(|e| at(&path, e))?;
    crate::profile_txn::durable::replace(cache, &path, &bytes).map_err(|e| at(&path, e))
}

/// Must be read under the cache publication lock. A missing, replaced or unscannable root
/// is not empty. Do not obtain an individual profile lock from this scanner.
pub(super) fn referenced(cache: &Path, game: &Path, tag: &str) -> Result<bool, String> {
    let roots = read(cache)?;
    for (root, id) in &roots {
        if root.canonicalize().map_err(|e| at(root, e))? != *root || Identity::of(root)? != *id {
            return Err(at(root, unsettled()));
        }
        if scan_game(root, tag)? {
            return Ok(true);
        }
    }
    match game.canonicalize() {
        Ok(root) if roots.contains_key(&root) => Ok(false),
        Ok(root) => scan_game(root.as_path(), tag),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(at(game, error)),
    }
}

fn scan_game(root: &Path, tag: &str) -> Result<bool, String> {
    if refers(root, tag)? {
        return Ok(true);
    }
    for (collection, child) in [("instances", None), ("sunrise-packs", Some("home"))] {
        let collection = root.join(collection);
        let entries = match fs::symlink_metadata(&collection) {
            Ok(meta) if meta.is_dir() && plain_file(&meta) => {
                fs::read_dir(&collection).map_err(|e| at(&collection, e))?
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(at(&collection, e)),
            Ok(_) => return Err(at(&collection, unsettled())),
        };
        for entry in entries {
            let path = entry.map_err(|e| at(&collection, e))?.path();
            let meta = fs::symlink_metadata(&path).map_err(|e| at(&path, e))?;
            if !plain_file(&meta) {
                return Err(at(&path, unsettled()));
            }
            if meta.is_dir() {
                let home = child.map_or(path.clone(), |child| path.join(child));
                if refers(&home, tag)? {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

fn refers(home: &Path, tag: &str) -> Result<bool, String> {
    match fs::symlink_metadata(home) {
        Ok(meta) if meta.is_dir() && plain_file(&meta) => (),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(at(home, e)),
        Ok(_) => return Err(at(home, unsettled())),
    }
    let snapshot = crate::profile_txn::references(home)?;
    // ASCII tags can refer to the same entry on case-insensitive volumes. Keep both
    // spellings even when today's filesystem distinguishes them.
    let same = |saved: &str| saved.eq_ignore_ascii_case(tag);
    if snapshot.targets.iter().any(|target| same(target)) {
        return Ok(true);
    }
    let bytes: BTreeSet<_> = [snapshot.pointer, snapshot.before, snapshot.after]
        .into_iter()
        .flatten()
        .collect();
    for bytes in bytes {
        let active = Active::parse(&bytes).map_err(|e| at(home, e))?;
        if same(active.base()) || same(active.executable()) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backend,
        profile_txn::{Admission, Bootstrap, Intent},
        storage::{Category, Storage},
    };
    use std::{
        process::Command,
        time::{Duration, SystemTime},
    };
    use sunrise_launcher_smod::{manifest, mod_list};

    #[test]
    fn an_unavailable_or_replaced_registered_root_cannot_authorize_deletion() {
        let temp = tempfile::tempdir().unwrap();
        let cache = temp.path().join("launcher");
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        for dir in [&cache, &first, &second] {
            fs::create_dir(dir).unwrap();
        }
        register_root(&cache, first.canonicalize().unwrap()).unwrap();
        let mut profile = Admission::open(&first, None).unwrap();
        profile
            .bind_bootstrap(
                || {
                    Ok(Bootstrap::new(
                        "nightly-old".into(),
                        "native-old".into(),
                        [1; 16],
                        [2; 32],
                        serde_json::json!({}),
                    ))
                },
                |_| Ok(()),
            )
            .unwrap();
        assert!(referenced(&cache, &second, "NIGHTLY-OLD").unwrap());
        assert!(referenced(&cache, &second, "native-old").unwrap());
        assert!(!referenced(&cache, &second, "unrelated").unwrap());
        drop(profile);
        fs::rename(&first, temp.path().join("not-the-current-root")).unwrap();
        assert!(referenced(&cache, &second, "unrelated").is_err());
        fs::create_dir(&first).unwrap();
        assert!(referenced(&cache, &second, "unrelated").is_err());
        assert!(register_root(&cache, first.canonicalize().unwrap()).is_err());
    }

    #[test]
    fn real_remove_and_eviction_keep_a_reference_published_after_selection_in_another_home() {
        const NAME: &str = "managed_play::roots::tests::real_remove_and_eviction_keep_a_reference_published_after_selection_in_another_home";
        const BASE: &str = "v0.1.0";
        const DERIVED: &str = "native-profile-race";
        let actor = || {
            let mut actor = Command::new(std::env::current_exe().unwrap());
            actor.args(["--exact", NAME, "--nocapture"]);
            actor
        };
        let Some(root) = std::env::var_os("ARTIFACT_ROOT_RACE") else {
            let temp = tempfile::tempdir().unwrap();
            let root = backend::plain(temp.path().canonicalize().unwrap());
            assert!(actor()
                .env("ARTIFACT_ROOT_RACE", &root)
                .env("SUNRISE_HOME", root.join("unrelated-home"))
                .env("SUNRISE_LAUNCHER_HOME", root.join("launcher"))
                .env("XDG_CONFIG_HOME", root.join("config"))
                .status()
                .unwrap()
                .success());
            return;
        };
        let root = PathBuf::from(root);
        if let Ok(image) = std::env::var("ARTIFACT_ROOT_PUBLISH") {
            let home = backend::game_home();
            fs::create_dir_all(&home).unwrap();
            let mut admission = Admission::open(&home, None).unwrap();
            let epoch = crate::managed_play::inputs(&home, None).unwrap();
            let _fence = crate::managed_play::ReferencePublication::begin(&home).unwrap();
            let stage = admission
                .reserve(
                    Intent::Install,
                    BTreeSet::from([BASE.into(), image.clone()]),
                )
                .unwrap();
            let resources = super::super::Resources::capture(BASE, &stage).unwrap();
            let active = Active::capture(
                BASE,
                &image,
                &home,
                epoch,
                super::super::Generation::First {
                    profile: [1; 16],
                    resources,
                    variant: None,
                },
            )
            .unwrap();
            let pointer = serde_json::to_vec(&active).unwrap();
            stage
                .seal(pointer.clone(), &[])
                .unwrap()
                .publish(|| Ok(()))
                .unwrap();
            assert_eq!(
                crate::profile_txn::references(&home).unwrap().pointer,
                Some(pointer)
            );
            return;
        }
        let install = |tag| {
            let path = backend::version_dir(tag);
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join(".installed"), b"verified image fixture").unwrap();
            fs::write(
                path.join(backend::CLIENT),
                sunrise_launcher_smod::holder::CAPABILITY,
            )
            .unwrap();
            crate::run_leases::publish(&path).unwrap();
            crate::storage::mark_used(&path).unwrap();
            path
        };
        let check_bytes = |path: &Path| {
            assert_eq!(
                fs::read(path.join(".installed")).unwrap(),
                b"verified image fixture"
            );
            assert_eq!(
                fs::read(path.join(backend::CLIENT)).unwrap(),
                sunrise_launcher_smod::holder::CAPABILITY
            );
        };
        let publish = |tag| {
            assert!(actor()
                .env("ARTIFACT_ROOT_PUBLISH", tag)
                .env("SUNRISE_HOME", root.join(format!("using-{tag}")))
                .status()
                .unwrap()
                .success());
        };
        let base = install(BASE);
        let resources = backend::resources(BASE);
        fs::create_dir_all(&resources).unwrap();
        fs::write(resources.join("sunrise-resources.json"), b"{}").unwrap();
        let expected =
            crate::native_mods::cache_error(crate::native_cache::Error::ReferencedProfile);
        assert_eq!(
            backend::remove_observed(BASE, || publish(BASE)).unwrap_err(),
            expected
        );
        check_bytes(&base);

        let derived = install(DERIVED);
        let home = root.join(format!("using-{DERIVED}"));
        fs::create_dir_all(&home).unwrap();
        let body = format!("id='fixture'\nname='Fixture'\nversion='1.0.0'\napi='{}'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{}'\n", manifest::API, "a".repeat(40));
        fs::create_dir(home.join(mod_list::NATIVE_DIR)).unwrap();
        fs::write(
            home.join(mod_list::NATIVE_DIR).join("fixture.smod"),
            manifest::Package::write_files(
                &body,
                &[
                    (
                        "Cargo.toml",
                        b"[package]\nname='fixture'\nversion='1.0.0'\n",
                    ),
                    ("src/lib.rs", b"fn fixture() {}"),
                ],
            )
            .unwrap(),
        )
        .unwrap();
        fs::write(home.join(mod_list::FILE), b"mod.fixture:true\n").unwrap();
        let enabled = crate::native_status::enabled(&home);
        assert_eq!(enabled.len(), 1);
        let (id, version, sha) = &enabled[0];
        fs::write(
            backend::version_dir(DERIVED).join("native-mods.json"),
            serde_json::to_vec(&serde_json::json!({
                "base": BASE, "sunrise": "a".repeat(40), "target": "client",
                "mods": [{"id":id,"version":version,"sha256":sha}],
            }))
            .unwrap(),
        )
        .unwrap();
        let mut selected = false;
        let report = Storage::open()
            .unwrap()
            .evict_inventory_observed(
                SystemTime::now() + Duration::from_secs(14 * 24 * 60 * 60),
                Some(0),
                false,
                |path| {
                    if path == derived {
                        assert!(!std::mem::replace(&mut selected, true));
                        publish(DERIVED);
                    }
                },
            )
            .unwrap();
        assert!(
            selected,
            "eviction must pass through the real selection boundary"
        );
        let row = report
            .rows
            .into_iter()
            .find(|row| row.category == Category::SpareGameBuilds)
            .unwrap();
        assert_eq!(row.freed_bytes, 0);
        assert!(row
            .kept
            .iter()
            .any(|kept| kept.bytes > 0 && kept.reason == expected));
        check_bytes(&derived);
    }
}
