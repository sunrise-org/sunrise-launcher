//! Creating an instance, empty or as a copy of another, and publishing its new folder.
use super::*;

/// An instance's folder name: its name in lowercase words joined by dashes.
pub(super) fn slug(name: &str) -> String {
    let words: Vec<String> = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    if words.is_empty() {
        "pack".into()
    } else {
        words.join("-")
    }
}

/// What a copy of a game folder leaves out: other instances and packs, logs and crash reports,
/// caches, data collection's state, and sign-in sessions (a session never leaves its folder).
pub(super) const NOT_COPIED: &[&str] = &[
    OWN,
    STAGING,
    "ports",
    "sunrise-packs",
    "logs",
    "crash-reports",
    "sessions",
    "mod-browser",
    ".mod-browser.lock",
    ".install-txn.lock",
    GATES,
    INITIAL,
    RETIRED,
    ".sunrise-cache",
    // Far terrain's copies of what servers sent, rebuilt as the instance plays.
    "server-terrain",
];

/// Files of the player, not of one instance, that a new instance starts with: the keys and video
/// settings, and the offline name (so the player stays the same player in its worlds).
pub(super) const PLAYER_FILES: &[&str] = &[crate::paths::OPTIONS, "sunrise-player.txt"];

/// Creates an instance named `name`: empty but for the player's options and offline name, or a
/// copy of `from`'s game folder (its worlds, options and packs). Nothing of `from` changes.
pub fn create(name: &str, from: Option<&str>) -> Result<Instance, String> {
    let from = from.map(find).transpose()?;
    create_whole_in(
        &backend::game_home(),
        &backend::data_dir(),
        name,
        from.as_ref(),
        None,
        |_| {},
    )
}

/// The directory and build the creation picker told the player it would duplicate.
#[derive(Clone)]
pub struct CopyPreview {
    instance: Instance,
    base: Option<String>,
}

impl CopyPreview {
    /// Read only; the worker rechecks both the directory and this build under admission.
    pub fn read(instance: &Instance) -> Result<Self, String> {
        let refs = crate::profile_txn::references(&instance.home)?;
        let base = refs
            .pointer
            .as_deref()
            .map(crate::managed_play::Active::parse)
            .transpose()?
            .map(|active| active.base().to_owned());
        if let Some(expected) = instance.incarnation {
            expected.check(instance, Identity::of(&instance.home)?)?;
        }
        Ok(Self {
            instance: instance.clone(),
            base,
        })
    }

    pub fn base(&self) -> Option<&str> {
        self.base.as_deref()
    }

    /// The profile it would copy.
    pub fn source(&self) -> &Instance {
        &self.instance
    }
}

#[derive(Clone, Copy)]
pub(super) enum CopyFrom<'a> {
    Selected(&'a Instance),
    Preview(&'a CopyPreview),
}

impl<'a> CopyFrom<'a> {
    fn instance(self) -> &'a Instance {
        match self {
            Self::Selected(selected) => selected,
            Self::Preview(preview) => &preview.instance,
        }
    }
}

/// [`create`], with everything else the New Profile dialog chose, as one step: `picture` (None
/// keeps what a copy brought, `Some(None)` leaves it without one) and `choose`, which sets the
/// rest of its choices. An instance appears only once its folder, picture and choices are all in
/// place. Once visible, its initial choices can be recovered even if the launcher exits.
pub fn create_with(
    name: &str,
    from: Option<&CopyPreview>,
    picture: Option<Option<&[u8]>>,
    choose: impl FnOnce(&mut Meta),
) -> Result<Instance, String> {
    create_with_publish(
        &backend::game_home(),
        &backend::data_dir(),
        name,
        from.map(CopyFrom::Preview),
        picture,
        choose,
        |_| {},
    )
}

/// What creating or renaming an instance says when another already has `name`.
pub fn name_taken(name: &str) -> String {
    crate::i18n::trf(
        "There is already a profile called {name}. Choose another name.",
        &[("name", name)],
    )
}

pub(super) fn create_whole_in(
    game_home: &Path,
    data: &Path,
    name: &str,
    from: Option<&Instance>,
    picture: Option<Option<&[u8]>>,
    choose: impl FnOnce(&mut Meta),
) -> Result<Instance, String> {
    create_with_publish(
        game_home,
        data,
        name,
        from.map(CopyFrom::Selected),
        picture,
        choose,
        |_| {},
    )
}

pub(super) fn create_with_publish(
    game_home: &Path,
    data: &Path,
    name: &str,
    copy: Option<CopyFrom<'_>>,
    picture: Option<Option<&[u8]>>,
    choose: impl FnOnce(&mut Meta),
    after_visible: impl FnOnce(&Path),
) -> Result<Instance, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(crate::i18n::tr("A profile needs a name.").into());
    }
    let from = copy.map(CopyFrom::instance);
    if let Some(from) = from.filter(|f| f.kind == Kind::Pack) {
        return Err(crate::i18n::trf(
            "{name} plays its pack's mods: import the pack again for a second copy.",
            &[("name", &from.name)],
        ));
    }
    let create_error = |e: std::io::Error| {
        crate::i18n::trf(
            "Could not make profile {name}: {error}",
            &[("name", name), ("error", &e.to_string())],
        )
    };
    fs::create_dir_all(game_home).map_err(create_error)?;
    let root = game_home.canonicalize().map_err(create_error)?;
    let own = creation_parent(&root, OWN).map_err(create_error)?;
    let staging = creation_parent(&root, STAGING).map_err(create_error)?;
    if list_in(game_home, data)
        .iter()
        .any(|i| i.name.eq_ignore_ascii_case(name))
    {
        return Err(name_taken(name));
    }
    let base = slug(name);
    let taken = |s: &str| s == DEFAULT || own.join(s).symlink_metadata().is_ok();
    let slug = (1..)
        .map(|n| {
            if n == 1 {
                base.clone()
            } else {
                format!("{base}-{n}")
            }
        })
        .find(|s| !taken(s))
        .unwrap();
    let dir = own.join(&slug);
    // Each attempt owns one private directory. Move its child, not the temp directory's
    // own path, so dropping the guard can never clean up a reused path after publication.
    Store::read(data)?;
    let stage = tempfile::tempdir_in(&staging).map_err(create_error)?;
    let contents = stage.path().join("home");
    let source = from
        .map(|from| ManagedProfile::open_instance(game_home, data, from).map_err(String::from))
        .transpose()?;
    let active = if let Some(source) = &source {
        let _inputs = source.lock_inputs()?;
        let active = source.active()?;
        if let Some(CopyFrom::Preview(preview)) = copy {
            if active.as_ref().map(crate::managed_play::Active::base) != preview.base() {
                return Err(lifecycle::changed());
            }
        }
        let worlds = crate::worlds::SavedWorlds::claim(&source.home().join("saves"))?;
        copy_home(source.home(), &contents, &worlds).map_err(create_error)?;
        active
            .map(|active| source.stage_copy(active, &contents))
            .transpose()?
    } else {
        fs::create_dir(&contents).map_err(create_error)?;
        for f in PLAYER_FILES {
            let source = game_home.join(f);
            if source.is_file() {
                fs::copy(source, contents.join(f)).map_err(create_error)?;
            }
        }
        None
    };
    let icon = contents.join(PICTURE);
    match picture {
        Some(Some(png)) => fs::write(icon, png).map_err(create_error)?,
        Some(None) if icon.is_file() => fs::remove_file(icon).map_err(create_error)?,
        _ => {}
    }
    let mut choices = Meta::default();
    choices.name = (name != slug).then(|| name.to_string());
    choices.icon = from.and_then(|f| f.icon.clone());
    choices.group = from.and_then(|f| f.group.clone());
    choices.channel = Some(Channel::Stable);
    choose(&mut choices);
    if let Some(active) = &active {
        choices.version = Some(active.base().to_owned());
    }
    let bytes = serde_json::to_vec(&choices).map_err(|e| e.to_string())?;
    fs::write(contents.join(INITIAL), bytes).map_err(create_error)?;
    durable::flush_tree(&contents).map_err(create_error)?;
    let _name = NameGate::open(&lifecycle::gate_path(&root, OWN, slug.as_ref())?)?;
    if list_in(game_home, data)
        .iter()
        .any(|i| i.name.eq_ignore_ascii_case(name))
    {
        return Err(name_taken(name));
    }
    match fs::symlink_metadata(&dir) {
        Ok(_) => return Err(name_taken(name)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(create_error(e)),
    }
    {
        let _references = ReferencePublication::begin_at(data, game_home, &root)?;
        if let Some(active) = &active {
            active.verify_artifacts()?;
        }
        crate::run_leases::publish_at(&own, data).map_err(crate::native_mods::cache_error)?;
        crate::run_leases::publish_at(&contents, data).map_err(crate::native_mods::cache_error)?;
        creation_parent(&root, OWN).map_err(create_error)?;
        Store::locked(data, |store| {
            // Clear the old incarnation's row before the new folder can be observed, and
            // don't let another metadata writer fill that slot before publication.
            if store.instances.remove(&slug).is_some() {
                store.save(data)?;
            }
            durable::move_new_directory(&contents, &dir).map_err(create_error)?;
            after_visible(&dir);
            store.instances.insert(slug.clone(), choices);
            match store.save(data) {
                Ok(()) => {
                    let _ = fs::remove_file(dir.join(INITIAL));
                }
                Err(error) => {
                    // The sidecar is already durable. Do not unpublish or discard it.
                    eprintln!("Sunrise: instances.json is not saved: {error}");
                }
            }
            Ok(())
        })?;
    }
    find_in(&list_in(game_home, data), &slug)
}

/// A real child directory of the canonical game root, not a symlink or a junction.
pub(super) fn creation_parent(root: &Path, name: &str) -> std::io::Result<PathBuf> {
    let path = root.join(name);
    match fs::create_dir(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let meta = path.symlink_metadata()?;
    if !meta.is_dir() || crate::native_cache::linked(&meta) || path.canonicalize()? != path {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            crate::i18n::trf(
                "{path} is not a directory inside the game folder",
                &[("path", &path.display().to_string())],
            ),
        ));
    }
    Ok(path)
}

/// Publish atomically without replacing another attempt's folder, even if it is empty.
#[cfg(not(windows))]
pub(crate) fn publish_new_folder(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt as _;
        let from = CString::new(from.as_os_str().as_bytes())?;
        let to = CString::new(to.as_os_str().as_bytes())?;
        // SAFETY: both pointers name owned, null-terminated path strings for the call.
        let result = unsafe {
            #[cfg(target_os = "linux")]
            let result = libc::renameat2(
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            );
            #[cfg(target_os = "macos")]
            let result = libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL);
            result
        };
        if result == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (from, to);
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

pub(super) fn copy_home(
    from: &Path,
    to: &Path,
    worlds: &crate::worlds::SavedWorlds,
) -> std::io::Result<()> {
    let saves = from.join("saves");
    let mut pending = vec![(from.to_owned(), to.to_owned())];
    while let Some((source, target)) = pending.pop() {
        if source.parent() == Some(&saves) {
            worlds.verify(&source)?;
        }
        fs::create_dir(&target)?;
        for entry in fs::read_dir(&source)? {
            let entry = entry?;
            let name = entry.file_name();
            if (source == from && NOT_COPIED.iter().any(|n| name == *n))
                || (source.parent() == Some(&saves) && name == crate::worlds::LOCK)
            {
                continue;
            }
            let path = entry.path();
            let meta = path.symlink_metadata()?;
            if crate::native_cache::linked(&meta) {
                return Err(std::io::ErrorKind::InvalidData.into());
            }
            if meta.is_dir() {
                pending.push((path, target.join(name)));
            } else if meta.is_file() {
                fs::copy(path, target.join(name))?;
            } else {
                return Err(std::io::ErrorKind::InvalidData.into());
            }
        }
    }
    // Preserve chosen-versions.json pins while keeping caches inside mod-browser/ out of copies.
    let chosen_dir = from.join("mod-browser");
    if let Ok(dir_meta) = chosen_dir.symlink_metadata() {
        if dir_meta.is_dir() && !crate::native_cache::linked(&dir_meta) {
            let chosen = chosen_dir.join("chosen-versions.json");
            match chosen.symlink_metadata() {
                Ok(meta) if meta.is_file() && !crate::native_cache::linked(&meta) => {
                    let target_dir = to.join("mod-browser");
                    fs::create_dir_all(&target_dir)?;
                    fs::copy(&chosen, target_dir.join("chosen-versions.json"))?;
                }
                Ok(_) => return Err(std::io::ErrorKind::InvalidData.into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
    }
    worlds.verify_all()
}
