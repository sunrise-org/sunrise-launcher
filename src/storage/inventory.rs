//! The CLI and Storage page share one fixed-path inventory.
mod count;
#[cfg(test)]
#[path = "inventory_regressions.rs"]
mod regressions;
use super::*;
use crate::{backend, instances, native_mods};
use serde::Serialize;
use std::sync::Arc;

// Only files the game can regenerate from authoritative inputs may be offered here.
// Both behaviours cache versions are named explicitly: v8 is written by live 26.3+
// builds and v7 by older installed builds. Never replace with a pattern.
const PROFILE_GAME_CACHES: [&str; 6] = [
    ".sunrise-cache/behaviours-v8.bin",
    ".sunrise-cache/behaviours-v7.bin",
    ".sunrise-cache/registry-v1.bin",
    ".sunrise-cache/mod-content",
    ".sunrise-cache/resourcepacks",
    "cache/dx12-shaders.bin",
];

const BUILD_CACHES: [BuildCache; 11] = [
    BuildCache::Source,
    BuildCache::Tools,
    BuildCache::PrebuiltOutput,
    BuildCache::InternalsSync,
    BuildCache::Prebuilt,
    BuildCache::ServerBuilds,
    BuildCache::KeptMods,
    BuildCache::ModBuildCache,
    BuildCache::Rust,
    BuildCache::Cargo,
    BuildCache::Scratch,
];

/// Whether a measured row has candidates for an explicit Clear. A candidate is not a promise
/// that the live exclusion checks at deletion time will pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Eligibility {
    NotMeasured,
    Empty,
    Candidate,
    WhollyRefused,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeptKind {
    Busy,
    OlderBuild,
    Incomplete,
    Unpublished,
    ReferencedProfile,
    UnprovenReferences,
    ResourceDependent,
    UnsupportedFilesystem,
    Unsafe,
    Unreadable,
}

#[derive(Clone, Serialize)]
pub struct Row {
    pub category: Category,
    pub bytes: u64,
    pub freed_bytes: u64,
    pub remaining_bytes: u64,
    #[serde(skip)]
    measured: bool,
    #[serde(skip)]
    has_candidates: bool,
    #[serde(skip)]
    pub after_clear: bool,
    #[serde(skip)]
    pub older_build_kept: bool,
    /// Refused or busy units are retained, with their allocated bytes and a visible reason.
    pub kept: Vec<Kept>,
}

impl Row {
    fn new(category: Category) -> Self {
        Self {
            category,
            bytes: 0,
            freed_bytes: 0,
            remaining_bytes: 0,
            measured: false,
            has_candidates: false,
            after_clear: false,
            older_build_kept: false,
            kept: Vec::new(),
        }
    }

    /// The measured remainder, if this row was included in the operation.
    pub fn measured_bytes(&self) -> Option<u64> {
        self.measured.then_some(self.remaining_bytes)
    }

    pub fn eligibility(&self) -> Eligibility {
        if !self.measured {
            Eligibility::NotMeasured
        } else if self.has_candidates {
            Eligibility::Candidate
        } else if self.kept.is_empty() {
            Eligibility::Empty
        } else {
            Eligibility::WhollyRefused
        }
    }

    /// Byte upper bound; not a replacement for `eligibility()` (a link still occupies a slot).
    pub fn clearable_bytes(&self) -> u64 {
        if self.has_candidates {
            self.remaining_bytes
        } else {
            0
        }
    }

    pub fn only_kept_kind(&self, kind: KeptKind) -> bool {
        !self.kept.is_empty() && self.kept.iter().all(|unit| unit.kind == kind)
    }

    fn keep(&mut self, bytes: u64, error: Error, log: bool) {
        let kind = match &error {
            Error::Busy => KeptKind::Busy,
            Error::UnprovenExclusion => KeptKind::OlderBuild,
            Error::IncompleteUnit => KeptKind::Incomplete,
            Error::UnpublishedGame => KeptKind::Unpublished,
            Error::ReferencedProfile => KeptKind::ReferencedProfile,
            Error::UnprovenReferences => KeptKind::UnprovenReferences,
            Error::ResourceDependent(_) => KeptKind::ResourceDependent,
            Error::UnsupportedFilesystem => KeptKind::UnsupportedFilesystem,
            Error::Unsafe(_) => KeptKind::Unsafe,
            Error::Io(_) => KeptKind::Unreadable,
        };
        let detail = error_log_detail(&error);
        let reason = error_message(error);
        if log {
            match detail {
                Some(detail) => crate::launcher_log::line(format!(
                    "Storage Clear kept {:?} unit ({bytes} bytes): {reason} [{detail}]",
                    self.category
                )),
                None => crate::launcher_log::line(format!(
                    "Storage Clear kept {:?} unit ({bytes} bytes): {reason}",
                    self.category
                )),
            }
        }
        self.kept.push(Kept {
            bytes,
            reason,
            kind,
        });
        self.older_build_kept = self.only_kept_kind(KeptKind::OlderBuild);
    }
}

#[derive(Clone, Serialize)]
pub struct Kept {
    pub bytes: u64,
    pub reason: String,
    #[serde(skip)]
    pub kind: KeptKind,
}

/// The rows counted or cleared so far: all of them once it [`is_complete`](Self::is_complete).
#[derive(Clone, Serialize)]
pub struct Report {
    pub rows: Vec<Row>,
    pub total_bytes: u64,
    /// The count or Clear this report came from, so a window can drop one it has cancelled.
    #[serde(skip)]
    origin: Option<Cancel>,
}

impl Report {
    fn new(rows: Vec<Row>) -> Self {
        Self {
            total_bytes: rows
                .iter()
                .fold(0u64, |total, row| total.saturating_add(row.bytes)),
            rows,
            origin: None,
        }
    }

    /// Whether every row is in: a count in progress hands over the rows it has finished.
    pub fn is_complete(&self) -> bool {
        self.rows.len() == Category::ALL.len()
    }

    pub fn tagged(mut self, origin: &Cancel) -> Self {
        self.origin = Some(origin.clone());
        self
    }

    pub fn is_from(&self, origin: &Cancel) -> bool {
        self.origin.as_ref().is_some_and(|own| own.is(origin))
    }

    fn push(&mut self, row: Row) {
        self.total_bytes = self.total_bytes.saturating_add(row.bytes);
        self.rows.push(row);
    }
}

impl Category {
    pub const ALL: [Self; 6] = [
        Self::BuildFiles,
        Self::PrebuiltGames,
        Self::SpareGameBuilds,
        Self::RustCompiler,
        Self::ModBuildCache,
        Self::OtherCaches,
    ];
}

struct Root {
    path: PathBuf,
    directory: Option<Arc<Dir>>,
}
impl Root {
    fn open(root: &Path) -> Result<Self, Error> {
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

struct ProfileContext {
    home: PathBuf,
    work: PathBuf,
    version: String,
}

struct Entry {
    category: Category,
    directory: Option<Arc<Dir>>,
    relative: PathBuf,
    path: PathBuf,
    root_id: Option<(u64, u64)>,
    profile: Option<Arc<ProfileContext>>,
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
    fn fixed(root: &Root, relative: &Path, category: Category) -> Result<Self, Error> {
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

    fn incomplete_target(&self) -> bool {
        self.category == Category::BuildFiles && self.relative.starts_with("native/target")
    }

    fn is_candidate(&self, tree: &Tree) -> bool {
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

    fn tree(&self, cancel: &Cancel) -> Result<Tree, Error> {
        if self.category == Category::ModBuildCache || is_compiler_attempt(&self.relative) {
            Tree::mod_work(&self.path, cancel)
        } else {
            Tree::build_until(&self.path, cancel)
        }
    }

    /// The lease integration changes this classification, not deletion code.
    fn access(&self) -> Result<Access, Error> {
        #[cfg(test)]
        ACCESS.with(|calls| calls.set(calls.get() + 1));
        if self.build_tool() {
            Ok(Access::BuildTools)
        } else {
            Access::for_unit(&self.path)
        }
    }

    fn build_tool(&self) -> bool {
        matches!(
            self.category,
            Category::RustCompiler | Category::ModBuildCache
        ) || is_compiler_attempt(&self.relative)
            || self.relative.starts_with(BuildCache::KeptMods.relative())
    }

    #[cfg(all(test, unix))]
    fn clear(&self, build_root: &Path) -> Result<(), Error> {
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

    fn remove(
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
fn children(folder: &Path) -> Result<Vec<fs::DirEntry>, Error> {
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
type ListedHook = Box<dyn Fn(&Path)>;
#[cfg(test)]
thread_local! {
    #[cfg(not(windows))]
    static CANONICALIZING: std::cell::RefCell<Option<ListedHook>> = const { std::cell::RefCell::new(None) };
    static LISTED: std::cell::RefCell<Option<ListedHook>> =
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

fn usage_stamp(name: &Path) -> bool {
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
    fn entries(&self) -> Result<Vec<Entry>, Error> {
        self.entries_until(&Cancel::default(), None)
    }

    fn entries_until(
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
        let data = Root::open(&backend::data_dir())?;
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
    fn detached_trash(
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
        let game = backend::game_home();
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

    /// A read-only count, or a manual Clear of one row or all of them under the build lease.
    pub fn report(&self, clear: Option<Category>, all: bool) -> Result<Report, Error> {
        if clear.is_none() && !all {
            return self.report_counting(|_| {});
        }
        // Only manual Clear takes the build lease; its selected units take their own
        // Cargo locks or runtime claim at removal, with no whole-target sizing preflight.
        let selection = if all { None } else { clear };
        let result = (|| {
            let _lease = self.build_lease()?;
            let entries = self.entries_until(&Cancel::default(), selection)?;
            self.apply_retaining(entries, clear, all, true)
        })();
        result.map_err(redact_error)
    }

    #[cfg(test)]
    fn apply(
        &self,
        entries: Vec<Entry>,
        clear: Option<Category>,
        all: bool,
    ) -> Result<Report, Error> {
        self.apply_retaining(entries, clear, all, false)
    }

    #[cfg(test)]
    pub(crate) fn clear_observed(
        &self,
        clear: Option<Category>,
        all: bool,
        before_clear: impl FnMut(&Path),
    ) -> Result<Report, Error> {
        let _lease = self.build_lease()?;
        let entries = self.entries_until(&Cancel::default(), if all { None } else { clear })?;
        self.apply_retaining_observed(entries, clear, all, true, before_clear)
    }

    fn apply_retaining(
        &self,
        entries: Vec<Entry>,
        clear: Option<Category>,
        all: bool,
        retain: bool,
    ) -> Result<Report, Error> {
        self.apply_retaining_observed(entries, clear, all, retain, |_| {})
    }

    fn apply_retaining_observed(
        &self,
        entries: Vec<Entry>,
        clear: Option<Category>,
        all: bool,
        retain: bool,
        mut before_clear: impl FnMut(&Path),
    ) -> Result<Report, Error> {
        // Validate every descendant before any deletion and deduplicate allocated blocks per row.
        let mut rows: Vec<_> = Category::ALL.into_iter().map(Row::new).collect();
        // Each folder is walked once, here: the walk validates every descendant and its sizes
        // serve the rows and the kept lines.
        let (entries, mut trees) = if retain {
            refused(entries, &mut rows, &Cancel::default(), |category| {
                all || clear == Some(category)
            })?
        } else {
            let trees = entries
                .iter()
                .map(|entry| entry.tree(&Cancel::default()))
                .collect::<Result<Vec<_>, Error>>()?;
            (entries, trees)
        };
        for row in &mut rows {
            row.bytes = allocated(&entries, &trees, row.category);
            row.remaining_bytes = row.bytes;
            row.measured = all || clear.is_none() || clear == Some(row.category);
            row.has_candidates = entries
                .iter()
                .zip(&trees)
                .any(|(entry, tree)| entry.category == row.category && entry.is_candidate(tree));
        }
        let mut blocked_build_tools = false;
        let mut refused_indices = HashSet::new();
        for (index, entry) in entries.iter().enumerate() {
            if all || clear == Some(entry.category) {
                if !trees[index].present {
                    continue;
                }
                before_clear(&entry.path);
                let result = if blocked_build_tools && entry.build_tool() {
                    Err(Error::Busy)
                } else {
                    entry.remove(&self.root, retain, entry.root_id)
                };
                match result {
                    // Only a cleared folder is walked again, to see what is left of it.
                    Ok(()) => trees[index] = entry.tree(&Cancel::default())?,
                    Err(
                        error @ (Error::Busy
                        | Error::UnprovenExclusion
                        | Error::IncompleteUnit
                        | Error::UnpublishedGame
                        | Error::ResourceDependent(_)
                        | Error::ReferencedProfile
                        | Error::UnprovenReferences
                        | Error::UnsupportedFilesystem
                        | Error::Unsafe(_)),
                    ) if retain => {
                        let row = rows
                            .iter_mut()
                            .find(|row| row.category == entry.category)
                            .unwrap();
                        blocked_build_tools |= entry.build_tool() && matches!(error, Error::Busy);
                        row.keep(trees[index].size(&mut HashSet::new()), error, true);
                        refused_indices.insert(index);
                    }
                    Err(error @ Error::Io(_)) if retain => {
                        // A failed recursive removal can already have freed some files. If
                        // the rest can't be measured, conservatively keep its original size.
                        if let Ok(tree) = entry.tree(&Cancel::default()) {
                            trees[index] = tree;
                        }
                        let row = rows
                            .iter_mut()
                            .find(|row| row.category == entry.category)
                            .expect("all categories have a row");
                        row.keep(trees[index].size(&mut HashSet::new()), error, true);
                        refused_indices.insert(index);
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        for row in &mut rows {
            if all || clear == Some(row.category) {
                row.after_clear = true;
                let remaining = allocated(&entries, &trees, row.category);
                row.freed_bytes = row.bytes.saturating_sub(remaining);
                row.remaining_bytes = remaining;
                row.has_candidates =
                    entries
                        .iter()
                        .zip(&trees)
                        .enumerate()
                        .any(|(index, (entry, tree))| {
                            entry.category == row.category
                                && entry.is_candidate(tree)
                                && !refused_indices.contains(&index)
                        });
            }
        }
        Ok(Report::new(rows))
    }
}

/// A row's bytes: its folders' walks, each allocated block counted once across the row.
fn allocated(entries: &[Entry], trees: &[Tree], category: Category) -> u64 {
    let mut seen = HashSet::new();
    entries
        .iter()
        .zip(trees)
        .filter(|(entry, _)| entry.category == category)
        .fold(0u64, |bytes, (_, tree)| {
            bytes.saturating_add(tree.size(&mut seen))
        })
}

/// A unit holding a link or file the launcher does not expect is refused whole: it is neither
/// counted nor cleared, and its row says so. The other units are not held up by it.
fn refused(
    entries: Vec<Entry>,
    rows: &mut [Row],
    cancel: &Cancel,
    clearing: impl Fn(Category) -> bool,
) -> Result<(Vec<Entry>, Vec<Tree>), Error> {
    let mut sound = Vec::new();
    let mut trees = Vec::new();
    for entry in entries {
        if cancel.is_cancelled() {
            break;
        }
        let row = rows
            .iter_mut()
            .find(|row| row.category == entry.category)
            .unwrap();
        if let Some(tree) = measure_entry(&entry, row, cancel, clearing(entry.category))? {
            sound.push(entry);
            trees.push(tree);
        }
    }
    Ok((sound, trees))
}

fn measure_entry(
    entry: &Entry,
    row: &mut Row,
    cancel: &Cancel,
    clearing: bool,
) -> Result<Option<Tree>, Error> {
    match entry.tree(cancel) {
        Ok(tree) => {
            if !clearing && entry.incomplete_target() && tree.present {
                row.keep(tree.size(&mut HashSet::new()), Error::IncompleteUnit, false);
            }
            Ok(Some(tree))
        }
        Err(Error::Unsafe(_) | Error::Io(_)) if cancel.is_cancelled() => Ok(None),
        Err(error @ (Error::Unsafe(_) | Error::Io(_))) => {
            row.keep(0, error, clearing);
            Ok(None)
        }
        Err(error) => Err(error),
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

fn profile_game_caches(entries: &mut Vec<Entry>, home: &Root) -> Result<(), Error> {
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
fn unused_jars(entries: &mut Vec<Entry>, home: &Root) -> Result<(), Error> {
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

#[cfg(test)]
mod manual_tests {
    use super::*;

    #[test]
    fn profile_game_cache_inventory_counts_only_the_rebuildable_entries() {
        let home = crate::storage::tests::Home::new();
        let files = [
            home.seed(".sunrise-cache/behaviours-v8.bin"),
            home.seed(".sunrise-cache/behaviours-v7.bin"),
            home.seed(".sunrise-cache/registry-v1.bin"),
            home.seed(".sunrise-cache/mod-content/out"),
            home.seed(".sunrise-cache/resourcepacks/out"),
            home.seed("cache/dx12-shaders.bin"),
        ];
        let canaries = [
            home.seed(".sunrise-cache/behaviours-v9.bin"),
            home.seed(".sunrise-cache/player-data"),
            home.seed("cache/unknown/keep"),
            home.seed("cache/.trash-unrelated/keep"),
        ];
        let root = Root::open(&home.0).unwrap();
        let mut entries = Vec::new();
        profile_game_caches(&mut entries, &root).unwrap();
        // Simulate a profile separate from both the build root and launcher data directory.
        let launcher = crate::storage::tests::Home::new();
        let storage = Storage::at(&launcher.0).unwrap();
        let data = Root::open(&launcher.0).unwrap();
        storage
            .detached_trash(&mut entries, &data, &[], &Cancel::default())
            .unwrap();
        assert_eq!(entries.len(), 6);
        for file in files {
            assert_eq!(
                entries
                    .iter()
                    .filter(|entry| file.starts_with(&entry.path))
                    .count(),
                1
            );
        }
        for file in canaries {
            assert!(!entries.iter().any(|entry| file.starts_with(&entry.path)));
        }
    }

    #[test]
    fn refused_units_are_residual_with_bytes_and_reason() {
        let home = crate::storage::tests::Home::new();
        let file = home.seed("game/output");
        let storage = Storage::at(&home.0).unwrap();
        let root = Root::open(&home.0).unwrap();
        let entries = vec![Entry::fixed(&root, Path::new("game"), Category::RustCompiler).unwrap()];
        let lock = home.seed("game/.cargo-lock");
        let lock = fs::File::options()
            .read(true)
            .write(true)
            .open(lock)
            .unwrap();
        lock.lock().unwrap();
        let directory = file.parent().unwrap();
        // Give inline-directory filesystems enough names to allocate directory blocks too.
        for index in 0..32 {
            fs::write(
                directory.join(format!("padding-{index:02}-{}", "x".repeat(180))),
                [],
            )
            .unwrap();
        }
        // Independent filesystem allocations, not the recursive production sum.
        let own_bytes = |path: &Path| {
            super::super::allocation(path, &fs::symlink_metadata(path).unwrap())
                .unwrap()
                .1
        };
        let bytes = own_bytes(directory)
            + fs::read_dir(directory)
                .unwrap()
                .map(|entry| own_bytes(&entry.unwrap().path()))
                .sum::<u64>();
        let report = storage.apply_retaining(entries, None, true, true).unwrap();
        let row = report
            .rows
            .iter()
            .find(|row| row.category == Category::RustCompiler)
            .unwrap();
        assert_eq!(row.bytes, bytes);
        assert_eq!(row.freed_bytes, 0);
        assert_eq!(row.remaining_bytes, bytes);
        assert_eq!(row.kept.len(), 1);
        assert_eq!(row.kept[0].bytes, bytes);
        assert!(!row.kept[0].reason.is_empty());
        assert!(file.exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_unit_with_an_unexpected_link_is_refused_on_its_row_and_holds_up_no_other() {
        let home = crate::storage::tests::Home::new();
        let good = home.seed("rust/old/payload");
        let bad = home.seed("cache/payload");
        home.seed("worlds/keep");
        std::os::unix::fs::symlink(home.0.join("worlds"), home.0.join("cache/link")).unwrap();
        let storage = Storage::at(&home.0).unwrap();
        let root = Root::open(&home.0).unwrap();
        let entries = vec![
            Entry::fixed(&root, Path::new("cache"), Category::OtherCaches).unwrap(),
            Entry::fixed(&root, Path::new("rust/old"), Category::RustCompiler).unwrap(),
        ];
        let report = storage.apply_retaining(entries, None, true, true).unwrap();
        let row = |category| {
            report
                .rows
                .iter()
                .find(|row| row.category == category)
                .unwrap()
        };
        let refused = row(Category::OtherCaches);
        assert_eq!((refused.bytes, refused.freed_bytes), (0, 0));
        assert_eq!(refused.kept.len(), 1);
        assert!(refused.kept[0].reason.contains("link"));
        assert!(bad.exists());
        assert!(home.0.join("worlds/keep").exists());
        assert!(row(Category::RustCompiler).freed_bytes > 0);
        assert!(row(Category::RustCompiler).kept.is_empty());
        assert!(!good.exists());
    }

    #[cfg(unix)]
    #[test]
    fn retained_compiler_attempt_with_outside_link_is_cleared_without_following_it() {
        let home = crate::storage::tests::Home::new();
        let payload = home.seed("native/.nb-retained/output");
        let outside = home.seed("player-data/keep");
        let link = payload.with_file_name("link");
        std::os::unix::fs::symlink(outside.parent().unwrap(), &link).unwrap();
        let storage = Storage::at(&home.0).unwrap();
        let root = Root::open(&home.0).unwrap();
        let entries = vec![Entry::fixed(
            &root,
            Path::new("native/.nb-retained"),
            Category::BuildFiles,
        )
        .unwrap()];
        let report = storage
            .apply_retaining(entries, Some(Category::BuildFiles), false, true)
            .unwrap();
        let row = report
            .rows
            .iter()
            .find(|row| row.category == Category::BuildFiles)
            .unwrap();
        assert!(row.freed_bytes > 0);
        assert!(row.kept.is_empty());
        assert!(!payload.parent().unwrap().exists());
        assert!(outside.is_file());
    }

    #[test]
    fn a_refused_unit_is_walked_once_and_a_cleared_one_twice() {
        let home = crate::storage::tests::Home::new();
        home.seed("rust/old/payload");
        let storage = Storage::at(&home.0).unwrap();
        let root = Root::open(&home.0).unwrap();
        #[cfg(unix)]
        {
            home.seed("cache/payload");
            std::os::unix::fs::symlink(home.0.join("rust"), home.0.join("cache/link")).unwrap();
        }
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut entries =
            vec![Entry::fixed(&root, Path::new("rust/old"), Category::RustCompiler).unwrap()];
        #[cfg(unix)]
        entries.push(Entry::fixed(&root, Path::new("cache"), Category::OtherCaches).unwrap());
        let count = entries.len();
        let walks = crate::storage::walks();
        let report = storage.apply_retaining(entries, None, true, true).unwrap();
        // The refused folder was walked once (and stopped there); the sound one once, then once
        // more after it was cleared.
        assert_eq!(crate::storage::walks() - walks, count + 1);
        assert!(
            report
                .rows
                .iter()
                .find(|row| row.category == Category::RustCompiler)
                .unwrap()
                .freed_bytes
                > 0
        );
        #[cfg(unix)]
        assert_eq!(
            report
                .rows
                .iter()
                .find(|row| row.category == Category::OtherCaches)
                .unwrap()
                .kept
                .len(),
            1
        );
    }
}
