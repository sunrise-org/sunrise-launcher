//! Listing the instances, finding one by name or id, and the Play selection.
use super::*;

/// The instances, by the default game home and the launcher's data folder: by group and the
/// player's order, then name. Sunrise is first until it is moved.
pub fn list() -> Vec<Instance> {
    list_in(&backend::game_home(), &backend::data_dir())
}

/// Read the atomic metadata snapshot without housekeeping or taking a profile gate.
pub(crate) fn list_for_storage() -> Result<Vec<Instance>, String> {
    let data = backend::data_dir();
    let store = Store::read(&data)?;
    Ok(list_snapshot(
        &backend::game_home(),
        &data,
        store,
        true,
        false,
    ))
}

/// Read the profile directories without the migrations or housekeeping of `list`.
pub(crate) fn profile_homes_strict(game: &Path) -> Result<Vec<PathBuf>, String> {
    let mut homes = Vec::new();
    for (directory, pack) in [(OWN, false), ("sunrise-packs", true)] {
        let root = game.join(directory);
        let error = |e: std::io::Error| format!("{}: {e}", root.display());
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => match fs::symlink_metadata(&root)
            {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                _ => return Err(error(e)),
            },
            Err(e) => return Err(error(e)),
        };
        let directory = |path: &Path, meta: fs::Metadata| -> Result<bool, String> {
            if sunrise_launcher_smod::runtime_leases::linked(&meta) {
                // Skip links only when their target is known not to be a directory.
                return Ok(fs::metadata(path).map_err(error)?.is_dir());
            }
            Ok(meta.is_dir())
        };
        for entry in entries {
            let path = entry.map_err(error)?.path();
            if !directory(&path, fs::symlink_metadata(&path).map_err(error)?)? {
                continue;
            }
            if pack {
                let home = registry_pack::home(&path);
                match fs::symlink_metadata(&home) {
                    Ok(meta) => {
                        if directory(&home, meta)? {
                            homes.push(home);
                        }
                    }
                    // The work folder below already covers a not-yet-created home.
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                    Err(e) => return Err(error(e)),
                }
            }
            homes.push(path);
        }
    }
    Ok(homes)
}

pub(crate) fn list_in(game_home: &Path, data: &Path) -> Vec<Instance> {
    list_with_snapshot(game_home, data, true, || {})
}

pub(super) fn list_with_snapshot(
    game_home: &Path,
    data: &Path,
    housekeeping: bool,
    after_read: impl FnOnce(),
) -> Vec<Instance> {
    match Store::under_lock(data, || {
        let stored = Store::read(data);
        after_read();
        match stored {
            Ok(store) => list_snapshot(game_home, data, store, true, housekeeping),
            Err(_) => list_snapshot(game_home, data, Store::default(), false, false),
        }
    }) {
        Ok(instances) => instances,
        // Keep the listing usable, but don't authorize mutation with unpaired choices.
        Err(_) => list_snapshot(game_home, data, Store::load(data), false, false),
    }
}

fn list_snapshot(
    game_home: &Path,
    data: &Path,
    mut store: Store,
    admitted: bool,
    housekeeping: bool,
) -> Vec<Instance> {
    let mut found = vec![(
        DEFAULT.to_string(),
        "Sunrise".to_string(),
        Kind::Default,
        game_home.to_path_buf(),
        game_home.to_path_buf(),
    )];
    let mut own: Vec<_> = fs::read_dir(game_home.join(OWN))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    own.sort();
    // Directory renames and metadata snapshots share the lock. Play/Delete need the initial
    // choices even after an interrupted create, but a refused request must not save the store.
    if admitted {
        let parent = game_home.join(OWN);
        let before = store.instances.len();
        if housekeeping && parent.is_dir() {
            store.instances.retain(|id, _| {
                id == DEFAULT
                    || crate::browser::safe_name(id) != Some(id)
                    || !fs::symlink_metadata(parent.join(id))
                        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
            });
        }
        let retained = store.instances.len();
        for id in &own {
            if !store.instances.contains_key(id) {
                if let Some(meta) = initial_choices(&parent.join(id)) {
                    store.instances.insert(id.clone(), meta);
                }
            }
        }
        if housekeeping && (retained != before || store.instances.len() != retained) {
            if let Err(error) = store.save(data) {
                eprintln!("Sunrise: instances.json is not saved: {error}");
            }
        }
    }
    // An instance an earlier launcher made for Minecraft: Java Edition is not listed: its folder
    // and its record in `instances.json` stay as they are.
    own.retain(|slug| store.instances.get(slug).is_none_or(|m| m.java.is_none()));
    for slug in own {
        let dir = game_home.join(OWN).join(&slug);
        found.push((slug.clone(), slug, Kind::Own, dir.clone(), dir));
    }
    for p in registry_pack::list_in(&game_home.join("sunrise-packs")) {
        let home = registry_pack::home(&p.work);
        found.push((format!("pack:{}", p.id), p.name, Kind::Pack, p.work, home));
    }
    let mut recovered_choices = false;
    for (id, _, kind, _, home) in &found {
        if *kind == Kind::Pack {
            continue;
        }
        let Some(choice) = pending_build_choice(data, home) else {
            continue;
        };
        if store
            .instances
            .get(id)
            .is_some_and(|meta| meta.reflects_build_choice(choice.id))
        {
            continue;
        }
        let meta = store
            .instances
            .entry(id.clone())
            .or_insert_with(|| initial_choices(home).unwrap_or_default());
        if apply_committed_choice(meta, &choice) {
            recovered_choices = true;
        }
    }
    if admitted && housekeeping && recovered_choices {
        if let Err(error) = store.save(data) {
            eprintln!("Sunrise: instances.json is not saved: {error}");
        }
    }
    let mut out: Vec<(Option<u32>, Instance)> = found
        .into_iter()
        .map(|(id, name, kind, work, home)| {
            let meta = store.instances.get(&id).cloned().unwrap_or_default();
            let order = meta.order;
            let instance = Instance {
                name: meta.name.unwrap_or(name),
                icon: meta.icon,
                group: meta.group,
                version: meta.version,
                channel: meta.channel.unwrap_or_default(),
                native_profile: meta
                    .native_profile
                    .unwrap_or(crate::native_mods::Profile::Optimized),
                played: Duration::from_secs(meta.played_secs),
                last_played: meta
                    .last_played
                    .and_then(|s| calendar_time(Duration::from_secs(s))),
                launch: meta.launch,
                id,
                kind,
                incarnation: admitted
                    .then(|| Incarnation::of(&work, &home).ok())
                    .flatten(),
                work,
                home,
            };
            (order, instance)
        })
        .collect();
    // By group, then the player's own order (the ones never moved come after, by name). The
    // groups sit in the order the player gave them, ungrouped profiles first and a group never
    // moved after those, by name. The default profile is placed like the rest, but until it is
    // first moved it stays first, as it always was.
    let group_order = store.group_order.clone();
    out.sort_by_key(|(order, i)| {
        (
            !(i.kind == Kind::Default && order.is_none()),
            groups::group_rank(&group_order, i.group.as_deref()),
            i.group.clone(),
            order.is_none(),
            order.unwrap_or(0),
            i.name.to_lowercase(),
        )
    });
    out.into_iter().map(|(_, i)| i).collect()
}

/// The default profile of `all`, which is no longer always the first of them.
fn default_of(all: &[Instance]) -> Instance {
    all.iter()
        .find(|i| i.kind == Kind::Default)
        .unwrap_or(&all[0])
        .clone()
}

/// The instance an id, a name (any case) or a pack's folder name names.
pub fn find(query: &str) -> Result<Instance, String> {
    find_in(&list(), query)
}

pub(super) fn find_in(all: &[Instance], query: &str) -> Result<Instance, String> {
    let q = query.trim();
    let bare = |id: &str| id.split_once(':').map_or(id, |(_, rest)| rest).to_string();
    all.iter()
        .find(|i| i.id == q)
        .or_else(|| all.iter().find(|i| i.name.eq_ignore_ascii_case(q)))
        .or_else(|| all.iter().find(|i| bare(&i.id) == q))
        .cloned()
        .ok_or_else(|| {
            crate::i18n::trf(
                "No profile {query}: sunrise-launcher instance list shows them",
                &[("query", q)],
            )
        })
}

/// Resolve headless Play without repairing unrelated profile metadata on a refused request.
pub fn for_play(query: Option<&str>) -> Result<Instance, String> {
    let all = list_with_snapshot(&backend::game_home(), &backend::data_dir(), false, || {});
    let found = match query {
        Some(name) => find_in(&all, name),
        None => selection_in(&all, backend::Settings::load().instance.as_deref()),
    }?;
    if found.kind == Kind::Pack {
        return Err(registry_pack::play_unavailable());
    }
    Ok(found)
}

/// The instance Play starts: the one picked in the launcher, else Sunrise.
pub fn selected() -> Instance {
    let all = list();
    backend::Settings::load()
        .instance
        .and_then(|id| all.iter().find(|i| i.id == id).cloned())
        .unwrap_or_else(|| default_of(&all))
}

/// The saved selection when it names an instance that isn't listed any more: a pack an earlier
/// launcher ported, or a setup it made for another game. Its folder is still there, untouched,
/// but nothing here can use it.
pub fn retired_selection(all: &[Instance], saved: Option<&str>) -> Option<String> {
    let id = saved?;
    (!all.iter().any(|i| i.id == id)).then(|| id.to_string())
}

fn retired_message(id: &str) -> String {
    let name = id.split_once(':').map_or(id, |(_, rest)| rest);
    crate::i18n::trf(
        "The selected profile {name} was made by an earlier launcher and isn't supported any more. Its files are untouched. Choose another: sunrise-launcher instance select <name>",
        &[("name", name)],
    )
}

/// The message for a retired selection ([`retired_selection`]).
pub fn retired_selection_message(id: &str) -> String {
    retired_message(id)
}

/// The instance a command without `--instance` acts on: [`selected`], except that a retired
/// selection is an error naming it, so nothing is written to Sunrise's setup in its place.
pub fn read_profile(query: Option<&str>) -> Result<Instance, String> {
    let all = list_with_snapshot(&backend::game_home(), &backend::data_dir(), false, || {});
    match query {
        Some(name) => find_in(&all, name),
        None => selection_in(&all, backend::Settings::load().instance.as_deref()),
    }
}

pub fn selection() -> Result<Instance, String> {
    selection_in(&list(), backend::Settings::load().instance.as_deref())
}

pub(super) fn selection_in(all: &[Instance], saved: Option<&str>) -> Result<Instance, String> {
    if let Some(id) = retired_selection(all, saved) {
        return Err(retired_message(&id));
    }
    Ok(saved
        .and_then(|id| all.iter().find(|i| i.id == id).cloned())
        .unwrap_or_else(|| default_of(all)))
}

pub fn select(query: &str) -> Result<Instance, String> {
    let instance = find(query)?;
    let mut settings = backend::Settings::load();
    settings.instance = (instance.kind != Kind::Default).then(|| instance.id.clone());
    settings.save()?;
    Ok(instance)
}
