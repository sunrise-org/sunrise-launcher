//! What the inventory finds on disk: the fixed roots, their entries, and the logs and jars it may clear.
use super::*;

pub(super) struct Root {
    path: PathBuf,
    pub(super) directory: Option<Arc<Dir>>,
}
impl Root {
    pub(super) fn open(root: &Path) -> Result<Self, Error> {
        let mut directory = match Dir::open_ambient_dir(root, cap_std::ambient_authority()) {
            Ok(directory) => Some(Arc::new(directory)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        checked(root, true)?;
        let mut path = root.to_owned();
        if directory.is_some() {
            #[cfg(windows)]
            for ancestor in root.ancestors().filter(|path| path.parent().is_some()) {
                checked(ancestor, true)?;
            }
            #[cfg(all(test, not(windows)))]
            CANONICALIZING.with(|hook| {
                if let Some(hook) = &*hook.borrow() {
                    hook(root);
                }
            });
            match present(root.canonicalize())? {
                Some(canonical) => {
                    let canonical = backend::plain(canonical);
                    #[cfg(not(windows))]
                    if canonical != root {
                        return Err(Error::Unsafe(root.to_owned()));
                    }
                    path = canonical;
                }
                None => directory = None,
            }
        }
        Ok(Self { path, directory })
    }
}

fn compiler_attempt(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|name| {
        name.starts_with(native_mods::ATTEMPT_PREFIX)
            || name.starts_with(native_mods::OLD_ATTEMPT_PREFIX)
    })
}

fn is_compiler_attempt(relative: &Path) -> bool {
    relative.parent() == Some(Path::new("native"))
        && relative.file_name().is_some_and(compiler_attempt)
}

pub(super) struct ProfileContext {
    pub(super) home: PathBuf,
    pub(super) work: PathBuf,
    pub(super) version: String,
}

pub(super) struct Entry {
    pub(super) category: Category,
    pub(super) directory: Option<Arc<Dir>>,
    pub(super) relative: PathBuf,
    pub(super) path: PathBuf,
    pub(super) root_id: Option<(u64, u64)>,
    pub(super) profile: Option<Arc<ProfileContext>>,
}

fn held_entry_identity(
    root: &Dir,
    relative: &Path,
    owns_links: bool,
) -> Result<Option<(u64, u64)>, Error> {
    let Some(name) = relative.file_name() else {
        return Ok(None);
    };
    let mut parent = root.try_clone()?;
    for part in relative.parent().unwrap_or(Path::new("")).components() {
        match parent.open_dir_nofollow(part.as_os_str()) {
            Ok(next) => parent = next,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                ) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        }
    }
    let name = Path::new(name);
    let meta = match parent.symlink_metadata(name) {
        Ok(meta) => meta,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error.into()),
    };
    #[cfg(windows)]
    let is_link = {
        use cap_std::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    };
    #[cfg(not(windows))]
    let is_link = meta.file_type().is_symlink();
    if is_link {
        return if owns_links {
            Ok(None)
        } else {
            Err(Error::Unsafe(relative.to_owned()))
        };
    }
    if !meta.is_dir() && !meta.is_file() {
        return Ok(None);
    }
    match entry_identity(&parent, name, meta.is_dir(), relative) {
        Ok(id) => Ok(Some(id)),
        Err(Error::Io(error))
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
            ) || (cfg!(windows) && error.raw_os_error() == Some(32)) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

impl Entry {
    pub(super) fn fixed(root: &Root, relative: &Path, category: Category) -> Result<Self, Error> {
        let owns_links = category == Category::ModBuildCache || is_compiler_attempt(relative);
        let mut path = root.path.clone();
        let mut components = relative.components().peekable();
        while let Some(component) = components.next() {
            let std::path::Component::Normal(component) = component else {
                return Err(Error::Unsafe(relative.to_owned()));
            };
            path.push(component);
            match fs::symlink_metadata(&path) {
                Ok(meta) if linked(&meta) && !(components.peek().is_none() && owns_links) => {
                    return Err(Error::Unsafe(path));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        let root_id = match &root.directory {
            Some(directory) => held_entry_identity(directory, relative, owns_links)?,
            None => None,
        };
        Ok(Self {
            category,
            directory: root.directory.clone(),
            relative: relative.to_owned(),
            path,
            root_id,
            profile: None,
        })
    }

    pub(super) fn incomplete_target(&self) -> bool {
        self.category == Category::BuildFiles && self.relative.starts_with("native/target")
    }

    pub(super) fn is_candidate(&self, tree: &Tree) -> bool {
        if self.incomplete_target() || !tree.present {
            return false;
        }
        // Plan intentionally retains the directory and its operational lock skeleton.
        // Other entries are detached whole or own their directory/link entry outright.
        if self.build_tool() {
            self.category == Category::ModBuildCache
                || is_compiler_attempt(&self.relative)
                || tree.has_payload
        } else {
            true
        }
    }

    pub(super) fn tree(&self, cancel: &Cancel) -> Result<Tree, Error> {
        if self.category == Category::ModBuildCache || is_compiler_attempt(&self.relative) {
            Tree::mod_work(&self.path, cancel)
        } else {
            Tree::build_until(&self.path, cancel)
        }
    }

    /// The lease integration changes this classification, not deletion code.
    pub(super) fn access(&self) -> Result<Access, Error> {
        #[cfg(test)]
        ACCESS.with(|calls| calls.set(calls.get() + 1));
        if self.build_tool() {
            Ok(Access::BuildTools)
        } else {
            Access::for_unit(&self.path)
        }
    }

    pub(super) fn build_tool(&self) -> bool {
        matches!(
            self.category,
            Category::RustCompiler | Category::ModBuildCache
        ) || is_compiler_attempt(&self.relative)
            || self.relative.starts_with(BuildCache::KeptMods.relative())
    }

    #[cfg(all(test, unix))]
    pub(super) fn clear(&self, build_root: &Path) -> Result<(), Error> {
        self.remove(build_root, false, None)
    }

    fn within_authorized_root(&self, build_root: &Path) -> bool {
        if let Some(profile) = &self.profile {
            strict_descendant(&self.path, &profile.home)
        } else if self.build_tool() {
            strict_descendant(&self.path, build_root)
        } else {
            strict_descendant(&self.path, &backend::data_dir())
                || strict_descendant(&self.path, build_root)
        }
    }

    pub(super) fn remove(
        &self,
        build_root: &Path,
        explicit: bool,
        expected: Option<(u64, u64)>,
    ) -> Result<(), Error> {
        if !self.within_authorized_root(build_root) {
            return Err(Error::Unsafe(self.path.clone()));
        }
        let owns_links =
            self.category == Category::ModBuildCache || is_compiler_attempt(&self.relative);
        if !owns_links && !self.path.try_exists()? {
            return Ok(());
        }
        let Some(root) = &self.directory else {
            return Ok(());
        };
        let _profile = if let Some(profile) = &self.profile {
            let owner = instances::lifecycle::owner_file(&backend::game_home(), &profile.home)
                .map_err(|_| Error::Busy)?;
            let claim = crate::profile_txn::Admission::claim(&profile.home, owner.as_deref())
                .map_err(|_| Error::Busy)?;
            if backend::in_use(&profile.version) {
                return Err(Error::Busy);
            }
            Some(claim)
        } else {
            None
        };
        let context_roots: Vec<&Path> = match &self.profile {
            Some(profile) if profile.work != profile.home => {
                vec![profile.home.as_path(), profile.work.as_path()]
            }
            Some(profile) => vec![profile.home.as_path()],
            None => Vec::new(),
        };
        if !explicit {
            let access = self.access()?;
            if matches!(access, Access::LeasedGame) {
                return crate::cache_cleanup::delete_game_unit_with_contexts(
                    &self.path,
                    &context_roots,
                    expected,
                );
            }
            access.check()?;
        } else if self.incomplete_target() {
            return Err(Error::IncompleteUnit);
        } else if !self.build_tool() {
            return crate::cache_cleanup::clear_claimed_unit_with_contexts(
                &self.path,
                &context_roots,
                expected,
            );
        }
        let mut parent = root.try_clone()?;
        for part in self.relative.parent().unwrap_or(Path::new("")).components() {
            parent = parent.open_dir_nofollow(part.as_os_str())?;
        }
        let name = Path::new(self.relative.file_name().expect("entry has a name"));
        if owns_links {
            let Some(meta) = present(parent.symlink_metadata(name))? else {
                return Ok(());
            };
            // On Windows, a junction is a directory entry; unlink it without opening it.
            #[cfg(windows)]
            let is_link = {
                use cap_std::fs::MetadataExt;
                use windows_sys::Win32::Storage::FileSystem::{
                    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
                };
                let attributes = meta.file_attributes();
                let is_link = attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;
                if is_link && expected.is_some() {
                    return Err(Error::Unsafe(self.path.clone()));
                }
                if is_link && attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                    return present(parent.remove_dir(name)).map(|_| ());
                }
                is_link
            };
            #[cfg(not(windows))]
            let is_link = meta.file_type().is_symlink();
            if is_link && expected.is_some() {
                return Err(Error::Unsafe(self.path.clone()));
            }
            if let Some(expected) = expected {
                check_entry_identity(&parent, name, meta.is_dir(), expected, &self.path)?;
            }
            if is_link || meta.is_file() {
                return present(parent.remove_file(name)).map(|_| ());
            }
            // Keep the no-follow parent as the boundary throughout recursive removal.
            return present(parent.remove_dir_all(name)).map(|_| ());
        }
        Storage::clear_entry_bound(root, &parent, name, expected)
    }
}

/// A folder's entries for a report that holds no build lease, so a folder or entry can vanish
/// while it is read: that one is skipped, as the size walk skips one. Every other failure, and
/// every link check on what is found, still stands.
#[cfg(test)]
pub(super) fn children(folder: &Path) -> Result<Vec<fs::DirEntry>, Error> {
    children_until(folder, &Cancel::default())
}

fn children_until(folder: &Path, cancel: &Cancel) -> Result<Vec<fs::DirEntry>, Error> {
    if cancel.is_cancelled() {
        return Ok(Vec::new());
    }
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut found = Vec::new();
    for entry in entries {
        if cancel.is_cancelled() {
            break;
        }
        match entry {
            Ok(entry) => found.push(entry),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    #[cfg(test)]
    LISTED.with(|hook| {
        if let Some(hook) = &*hook.borrow() {
            hook(folder);
        }
    });
    Ok(found)
}

// Lets a test change the disk between a folder's listing and what is done with its entries.
#[cfg(test)]
pub(super) type ListedHook = Box<dyn Fn(&Path)>;
#[cfg(test)]
thread_local! {
    #[cfg(not(windows))]
    pub(super) static CANONICALIZING: std::cell::RefCell<Option<ListedHook>> =
        const { std::cell::RefCell::new(None) };
    pub(super) static LISTED: std::cell::RefCell<Option<ListedHook>> =
        const { std::cell::RefCell::new(None) };
}

/// `Ok(None)` for a result that failed only because its entry vanished.
fn present<T>(result: std::io::Result<T>) -> Result<Option<T>, Error> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

// Registry::remove_unit names its detached entries PID-counter; Plan names them
// PID-timestamp-counter. A bare prefix is not ownership of an arbitrary sibling.
fn detached_name(name: &std::ffi::OsStr) -> bool {
    let Some(suffix) = name.to_str().and_then(|name| name.strip_prefix(".trash-")) else {
        return false;
    };
    let mut numbers = suffix.split('-');
    let decimal = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    numbers.next().is_some_and(decimal)
        && numbers.next().is_some_and(decimal)
        && numbers.next().is_none_or(decimal)
        && numbers.next().is_none()
}

pub(super) fn usage_stamp(name: &Path) -> bool {
    let name = name.as_os_str().to_string_lossy();
    name == ".last-used" || (name.starts_with('.') && name.ends_with(".last-used"))
}

fn add(
    entries: &mut Vec<Entry>,
    root: &Root,
    relative: &str,
    category: Category,
) -> Result<(), Error> {
    entries.push(Entry::fixed(root, Path::new(relative), category)?);
    Ok(())
}

impl Storage {
    /// What a report or Clear works on, found once: the fixed build-home units, the spare game
    /// builds and each profile's caches, and detached trash beside any of them.
    #[cfg(test)]
    pub(super) fn entries(&self) -> Result<Vec<Entry>, Error> {
        self.entries_until(&Cancel::default(), None)
    }

    pub(super) fn entries_until(
        &self,
        cancel: &Cancel,
        selected: Option<Category>,
    ) -> Result<Vec<Entry>, Error> {
        #[cfg(test)]
        ENTRIES.with(|calls| calls.set(calls.get() + 1));
        let includes = |category| selected.is_none_or(|selected| selected == category);
        let mut entries = Vec::new();
        if selected != Some(Category::OtherCaches) {
            self.build_entries(&mut entries, &Root::open(&self.root)?, cancel, selected)?;
        }
        if cancel.is_cancelled()
            || !(includes(Category::SpareGameBuilds) || includes(Category::OtherCaches))
        {
            return Ok(entries);
        }
        let data = Root::open(&self.data)?;
        // One read-only profile snapshot proves both which versions are played and where
        // their caches are. An unreadable snapshot proves neither.
        let profiles = match instances::list_for_storage_until(cancel) {
            Ok(Some(profiles)) => Some(profiles),
            Ok(None) => return Ok(entries),
            Err(_) => None,
        };
        if cancel.is_cancelled() {
            return Ok(entries);
        }
        if includes(Category::SpareGameBuilds) {
            let versions =
                Entry::fixed(&data, Path::new("versions"), Category::SpareGameBuilds)?.path;
            if let Some(profiles) = &profiles {
                let settings = backend::Settings::load();
                let selected = settings.selected.as_deref();
                spare_builds(&mut entries, &data, profiles, selected, cancel)?;
                // Interrupted removal loses completion; keep unpinned incomplete units visible for
                // retry, never overriding ordinary profile pins or an unreadable profile snapshot.
                for child in children_until(&versions, cancel)? {
                    if cancel.is_cancelled() {
                        break;
                    }
                    let name = child.file_name();
                    let pinned = name.to_str().is_some_and(|tag| {
                        selected.is_some_and(|v| v.eq_ignore_ascii_case(tag))
                            || profiles.iter().any(|profile| {
                                profile
                                    .version
                                    .as_deref()
                                    .is_some_and(|v| v.eq_ignore_ascii_case(tag))
                            })
                    });
                    if !pinned
                        && present(child.file_type())?.is_some_and(|kind| kind.is_dir())
                        && !child.path().join(".installed").is_file()
                        && !name.to_string_lossy().starts_with(".trash-")
                        && !crate::backend::installation::private_stage(&name)
                    {
                        entries.push(Entry::fixed(
                            &data,
                            &Path::new("versions").join(name),
                            Category::SpareGameBuilds,
                        )?);
                    }
                }
            }
        }
        if includes(Category::OtherCaches) && !cancel.is_cancelled() {
            let profiles = profiles.as_deref().unwrap_or(&[]);
            for relative in ["cache/github", "mod-browser", "profile-heads"] {
                if !overlaps_profile(&data.path.join(relative), profiles) {
                    add(&mut entries, &data, relative, Category::OtherCaches)?;
                }
            }
            profile_caches(&mut entries, profiles, cancel)?;
            self.detached_trash(&mut entries, &data, profiles, cancel)?;
        }
        Ok(entries)
    }

    fn build_entries(
        &self,
        entries: &mut Vec<Entry>,
        build_root: &Root,
        cancel: &Cancel,
        selected: Option<Category>,
    ) -> Result<(), Error> {
        for cache in BUILD_CACHES {
            if cancel.is_cancelled() {
                return Ok(());
            }
            if selected.is_some_and(|selected| selected != cache.category()) {
                continue;
            }
            if matches!(
                cache,
                BuildCache::Rust
                    | BuildCache::Source
                    | BuildCache::Tools
                    | BuildCache::PrebuiltOutput
                    | BuildCache::Prebuilt
                    | BuildCache::InternalsSync
                    | BuildCache::ModBuildCache
            ) {
                let parent =
                    Entry::fixed(build_root, Path::new(cache.relative()), cache.category())?.path;
                if checked(&parent, true)?.is_some() {
                    for child in children_until(&parent, cancel)? {
                        if cancel.is_cancelled() {
                            break;
                        }
                        let name = child.file_name();
                        if usage_stamp(Path::new(&name))
                            || (!matches!(cache, BuildCache::Rust | BuildCache::ModBuildCache)
                                && detached_name(&name))
                            || (matches!(cache, BuildCache::ModBuildCache)
                                && crate::cache_cleanup::reserved(Path::new(&name)))
                        {
                            continue;
                        }
                        entries.push(Entry::fixed(
                            build_root,
                            &Path::new(cache.relative()).join(name),
                            cache.category(),
                        )?);
                    }
                }
            } else {
                add(entries, build_root, cache.relative(), cache.category())?;
            }
        }
        let build_files = Category::BuildFiles;
        if selected.is_some_and(|selected| selected != build_files) {
            return Ok(());
        }
        let native = Entry::fixed(build_root, Path::new("native"), build_files)?;
        for child in children_until(&native.path, cancel)? {
            if cancel.is_cancelled() {
                break;
            }
            if compiler_attempt(&child.file_name())
                && present(child.file_type())?
                    .is_some_and(|kind| kind.is_dir() || kind.is_symlink())
            {
                entries.push(Entry::fixed(
                    build_root,
                    &Path::new("native").join(child.file_name()),
                    build_files,
                )?);
            }
        }
        for name in [
            "built.json",
            "internals.json",
            "build.log",
            "catalog.partial",
        ] {
            add(entries, build_root, &format!("native/{name}"), build_files)?;
        }
        // Report incomplete target outputs without offering them for manual Clear. Cargo's
        // lock files are not disposal candidates.
        let target = self.root.join("native/target");
        if checked(&target, true)?.is_some() {
            for child in children_until(&target, cancel)? {
                if cancel.is_cancelled() {
                    break;
                }
                if crate::cache_cleanup::reserved(Path::new(&child.file_name()))
                    || usage_stamp(Path::new(&child.file_name()))
                {
                    continue;
                }
                let relative = Path::new("native/target").join(child.file_name());
                let Some(kind) = present(child.file_type())? else {
                    continue;
                };
                checked(&child.path(), kind.is_dir())?;
                if kind.is_dir() && child.file_name().to_string_lossy().contains('-') {
                    // Cross-compilation target triples own independent profile units too.
                    for profile in children_until(&child.path(), cancel)? {
                        if cancel.is_cancelled() {
                            break;
                        }
                        if crate::cache_cleanup::reserved(Path::new(&profile.file_name()))
                            || usage_stamp(Path::new(&profile.file_name()))
                        {
                            continue;
                        }
                        entries.push(Entry::fixed(
                            build_root,
                            &relative.join(profile.file_name()),
                            build_files,
                        )?);
                    }
                } else {
                    entries.push(Entry::fixed(build_root, &relative, build_files)?);
                }
            }
        }
        Ok(())
    }

    /// Detached trees can remain beside any managed cache after an interrupted removal.
    pub(super) fn detached_trash(
        &self,
        entries: &mut Vec<Entry>,
        data: &Root,
        profiles: &[instances::Instance],
        cancel: &Cancel,
    ) -> Result<(), Error> {
        // Plan detaches into its held build root; runtime-claimed units detach beside the
        // original unit. Do not infer trash parents from profile or arbitrary row entries.
        let parents: HashSet<_> = [
            self.root.clone(),
            data.path.clone(),
            data.path.join("versions"),
            data.path.join("cache"),
            self.root.join("native"),
            self.root.join("native/source"),
            self.root.join("native/tools"),
            self.root.join("native/prebuilt-build"),
            self.root.join("native/prebuilt"),
            self.root.join("native/internals-sync"),
        ]
        .into_iter()
        .collect();
        let game = &self.game;
        let profile_home =
            |path: &Path| path == game || profiles.iter().any(|profile| profile.home == path);
        for parent in parents {
            if cancel.is_cancelled() {
                break;
            }
            if profile_home(&parent) || parent.parent().is_some_and(profile_home) {
                continue;
            }
            let root = match Root::open(&parent) {
                Ok(root) => root,
                Err(Error::Unsafe(_)) => continue,
                Err(Error::Io(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotADirectory | std::io::ErrorKind::PermissionDenied
                    ) =>
                {
                    continue
                }
                Err(error) => return Err(error),
            };
            if root.directory.is_none() {
                continue;
            }
            for entry in children_until(&parent, cancel)? {
                if cancel.is_cancelled() {
                    break;
                }
                let name = entry.file_name();
                if detached_name(&name) && !overlaps_profile(&entry.path(), profiles) {
                    entries.push(Entry::fixed(
                        &root,
                        Path::new(&name),
                        Category::OtherCaches,
                    )?);
                }
            }
        }
        Ok(())
    }
}

fn strict_descendant(path: &Path, root: &Path) -> bool {
    path.strip_prefix(root)
        .is_ok_and(|relative| !relative.as_os_str().is_empty())
}

fn overlaps_profile(path: &Path, profiles: &[instances::Instance]) -> bool {
    let overlaps = |home: &Path| path.starts_with(home) || home.starts_with(path);
    overlaps(&backend::game_home()) || profiles.iter().any(|profile| overlaps(&profile.home))
}

/// Native builds no profile plays and nothing is running or building from.
fn spare_builds(
    entries: &mut Vec<Entry>,
    data: &Root,
    profiles: &[instances::Instance],
    selected: Option<&str>,
    cancel: &Cancel,
) -> Result<(), Error> {
    let played: HashSet<_> = profiles
        .iter()
        .take_while(|_| !cancel.is_cancelled())
        .filter_map(|instance| instance.resolve_version(None).ok())
        .map(|tag| tag.to_ascii_lowercase())
        .collect();
    if cancel.is_cancelled() {
        return Ok(());
    }
    let Some(installed) = backend::installed_until(cancel) else {
        return Ok(());
    };
    let pinned = |version: &str| {
        selected.is_some_and(|v| v.eq_ignore_ascii_case(version))
            || played.contains(&version.to_ascii_lowercase())
            || profiles.iter().any(|instance| {
                instance
                    .version
                    .as_deref()
                    .is_some_and(|v| v.eq_ignore_ascii_case(version))
            })
    };
    for version in installed {
        if cancel.is_cancelled() {
            break;
        }
        if native_mods::is_native(&version)
            && !pinned(&version)
            && native_mods::resource_dependent(&version).is_none()
            && !backend::in_use(&version)
        {
            add(
                entries,
                data,
                &format!("versions/{version}"),
                Category::SpareGameBuilds,
            )?;
        }
    }
    Ok(())
}

/// The caches, old logs and unused jars of every profile whose game is not running.
fn profile_caches(
    entries: &mut Vec<Entry>,
    profiles: &[instances::Instance],
    cancel: &Cancel,
) -> Result<(), Error> {
    for instance in profiles {
        if cancel.is_cancelled() {
            break;
        }
        // A missing version cannot prove that the instance is idle. Keep its caches.
        let Ok(version) = instance.resolve_version(None) else {
            continue;
        };
        if backend::in_use(&version) {
            continue;
        }
        let root = Root::open(&instance.home)?;
        let start = entries.len();
        old_logs(entries, &root, cancel)?;
        unused_jars_until(entries, &root, cancel)?;
        profile_game_caches(entries, &root)?;
        let profile = Arc::new(ProfileContext {
            home: instance.home.clone(),
            work: instance.work.clone(),
            version,
        });
        for entry in &mut entries[start..] {
            entry.profile = Some(profile.clone());
        }
    }
    Ok(())
}

pub(super) fn profile_game_caches(entries: &mut Vec<Entry>, home: &Root) -> Result<(), Error> {
    for relative in PROFILE_GAME_CACHES {
        add(entries, home, relative, Category::OtherCaches)?;
    }
    Ok(())
}

fn old_logs(entries: &mut Vec<Entry>, home: &Root, cancel: &Cancel) -> Result<(), Error> {
    for directory in ["logs", "crash-reports"] {
        if cancel.is_cancelled() {
            return Ok(());
        }
        let folder = Entry::fixed(home, Path::new(directory), Category::OtherCaches)?.path;
        if checked(&folder, true)?.is_none() {
            continue;
        }
        let mut files = Vec::new();
        for entry in children_until(&folder, cancel)? {
            if cancel.is_cancelled() {
                return Ok(());
            }
            if crate::cache_cleanup::reserved(Path::new(&entry.file_name()))
                || usage_stamp(Path::new(&entry.file_name()))
            {
                continue;
            }
            let Some(meta) = checked(&entry.path(), false)? else {
                continue;
            };
            files.push((meta.modified()?, entry.file_name()));
        }
        files.sort();
        files.pop(); // Always retain the latest report, even when it is older than fourteen days.
        for (modified, name) in files {
            if cancel.is_cancelled() {
                return Ok(());
            }
            if modified
                .elapsed()
                .is_ok_and(|age| age.as_secs() > 14 * 86400)
            {
                entries.push(Entry::fixed(
                    home,
                    &Path::new(directory).join(name),
                    Category::OtherCaches,
                )?);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn unused_jars(entries: &mut Vec<Entry>, home: &Root) -> Result<(), Error> {
    unused_jars_until(entries, home, &Cancel::default())
}

fn unused_jars_until(entries: &mut Vec<Entry>, home: &Root, cancel: &Cancel) -> Result<(), Error> {
    use sunrise_launcher_smod::{
        mod_list::{DIR, NATIVE_DIR},
        Package,
    };
    let folder = Entry::fixed(home, Path::new("jars"), Category::OtherCaches)?.path;
    if checked(&folder, true)?.is_none() {
        return Ok(());
    }
    let mut referenced = HashSet::new();
    for directory in [DIR, NATIVE_DIR] {
        if cancel.is_cancelled() {
            return Ok(());
        }
        let path = Entry::fixed(home, Path::new(directory), Category::OtherCaches)?.path;
        // An unreadable mod directory or subfolder cannot prove a jar is unused. Retain all jars.
        let Ok(packages) = sunrise_launcher_smod::try_find_until(&path, || cancel.is_cancelled())
        else {
            return Ok(());
        };
        let Some(packages) = packages else {
            return Ok(());
        };
        for package in packages {
            if cancel.is_cancelled() {
                return Ok(());
            }
            // A malformed package cannot prove a jar is unused. Retain all jars in that case.
            let Ok(package) = Package::read(&package) else {
                return Ok(());
            };
            referenced.extend(
                package
                    .manifest
                    .jar
                    .iter()
                    .map(|jar| jar.cache_path(&folder)),
            );
        }
    }
    for entry in children_until(&folder, cancel)? {
        if cancel.is_cancelled() {
            return Ok(());
        }
        if !crate::cache_cleanup::reserved(Path::new(&entry.file_name()))
            && !usage_stamp(Path::new(&entry.file_name()))
            && !referenced.contains(&entry.path())
        {
            entries.push(Entry::fixed(
                home,
                &Path::new("jars").join(entry.file_name()),
                Category::OtherCaches,
            )?);
        }
    }
    Ok(())
}
