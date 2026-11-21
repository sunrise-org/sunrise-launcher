//! The CLI and Storage page share one fixed-path inventory.
#[cfg(test)]
#[path = "inventory_regressions.rs"]
mod regressions;
use super::*;
use crate::{backend, instances, native_mods};
use serde::Serialize;
use std::sync::Arc;

const BUILD_CACHES: [BuildCache; 9] = [
    BuildCache::Source,
    BuildCache::Tools,
    BuildCache::PrebuiltOutput,
    BuildCache::InternalsSync,
    BuildCache::Prebuilt,
    BuildCache::ServerBuilds,
    BuildCache::Rust,
    BuildCache::Cargo,
    BuildCache::Scratch,
];

#[derive(Serialize)]
pub struct Row {
    pub category: Category,
    pub bytes: u64,
    pub freed_bytes: u64,
    pub remaining_bytes: u64,
    #[serde(skip)]
    private_compiler_bytes: u64,
    /// Refused or busy units are retained, with their allocated bytes and a visible reason.
    pub kept: Vec<Kept>,
}

impl Row {
    pub fn clearable_bytes(&self) -> u64 {
        self.remaining_bytes
            .saturating_sub(self.private_compiler_bytes)
    }
}

#[derive(Serialize)]
pub struct Kept {
    pub bytes: u64,
    pub reason: String,
}

#[derive(Serialize)]
pub struct Report {
    pub rows: Vec<Row>,
    pub total_bytes: u64,
}

impl Report {
    fn include_retained_compiler(&mut self, bytes: Option<u64>) {
        if let Some(bytes) = bytes {
            let row = self
                .rows
                .iter_mut()
                .find(|r| r.category == Category::BuildFiles)
                .expect("report contains Build files");
            row.private_compiler_bytes = bytes;
            row.bytes = row.bytes.saturating_add(bytes);
            row.remaining_bytes = row.remaining_bytes.saturating_add(bytes);
            self.total_bytes = self.total_bytes.saturating_add(bytes);
            row.kept.push(Kept { bytes, reason: crate::i18n::tr("Private compiler work was kept because the launcher couldn't prove all workers had stopped.").into() });
        }
    }
}

impl Category {
    pub const ALL: [Self; 5] = [
        Self::BuildFiles,
        Self::PrebuiltGames,
        Self::SpareGameBuilds,
        Self::RustCompiler,
        Self::OtherCaches,
    ];
}

struct Root {
    path: PathBuf,
    directory: Option<Arc<Dir>>,
}
impl Root {
    fn open(root: &Path) -> Result<Self, Error> {
        let directory = match Dir::open_ambient_dir(root, cap_std::ambient_authority()) {
            Ok(directory) => Some(Arc::new(directory)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        checked(root, true)?;
        if root.exists() {
            // Windows canonical paths gain a `\\?\` prefix and long names, so compare ancestors' reparse points instead.
            #[cfg(windows)]
            for ancestor in root.ancestors().filter(|path| path.parent().is_some()) {
                checked(ancestor, true)?;
            }
            #[cfg(not(windows))]
            if root.canonicalize()? != root {
                return Err(Error::Unsafe(root.to_owned()));
            }
        }
        Ok(Self {
            path: root.to_owned(),
            directory,
        })
    }
}

struct Entry {
    category: Category,
    directory: Option<Arc<Dir>>,
    relative: PathBuf,
    path: PathBuf,
}

impl Entry {
    fn fixed(root: &Root, relative: &Path, category: Category) -> Result<Self, Error> {
        let mut path = root.path.clone();
        for component in relative.components() {
            let std::path::Component::Normal(component) = component else {
                return Err(Error::Unsafe(relative.to_owned()));
            };
            path.push(component);
            match fs::symlink_metadata(&path) {
                Ok(meta) if linked(&meta) => return Err(Error::Unsafe(path)),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(Self {
            category,
            directory: root.directory.clone(),
            relative: relative.to_owned(),
            path,
        })
    }

    /// Parse the exact revision-bearing names written by the cache constructors.
    fn revision(&self) -> Option<&str> {
        let name = self.relative.file_name()?.to_str()?;
        let parent = self.relative.parent()?;
        let revision = if parent == Path::new("native/tools") {
            (name.len() == 12).then_some(name)?
        } else if parent == Path::new("native/prebuilt") {
            let (revision, rest) = name.split_once('-')?;
            // A build's SDK slices (`<build id>-<target>[-server]`, a developer install's with
            // `.developer`), and the older single-archive caches (`<commit>-sunrise-prebuilt-v<n>-
            // <target>`), which native builds no longer read but eviction must still recognize.
            let rest = rest.strip_suffix(".developer").unwrap_or(rest);
            let current = sunrise_launcher_smod::index::native::prebuilt_prefix();
            let target = [
                "sunrise-prebuilt-v3-",
                "sunrise-prebuilt-v4-",
                "sunrise-prebuilt-v5-",
                current.as_str(),
            ]
            .into_iter()
            .find_map(|prefix| rest.strip_prefix(prefix))
            .unwrap_or(rest);
            let target = target.strip_suffix("-server").unwrap_or(target);
            crate::rust_toolchain::hosts()
                .iter()
                .any(|host| host.target == target)
                .then_some(revision)?
        } else if parent == Path::new("native/internals-sync") {
            if let Some(catalog) = name.strip_prefix("catalog-") {
                let (revision, host) = catalog.split_once('-')?;
                crate::rust_toolchain::hosts()
                    .iter()
                    .any(|candidate| candidate.target == host)
                    .then_some(revision)?
            } else {
                name
            }
        } else if parent == Path::new("native/source") {
            name
        } else {
            return None;
        };
        let expected = if parent == Path::new("native/tools") {
            12
        } else {
            40
        };
        (revision.len() == expected
            && revision
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
        .then_some(revision)
    }

    /// What an interrupted unpack or download left beside the units of `native/prebuilt`: a
    /// `.partial` folder, a `.lock` file, a `.download` archive. Nothing uses one while the build
    /// lease is held, and eviction holds it, so a lock file is removed only under that lease.
    fn leftover(&self) -> bool {
        self.relative.parent() == Some(Path::new("native/prebuilt"))
            && self
                .relative
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    [".partial", ".lock", ".download"]
                        .iter()
                        .any(|suffix| name.ends_with(suffix))
                })
    }

    fn unpublished_game(&self) -> bool {
        self.relative.parent() == Some(Path::new("versions"))
            && self
                .relative
                .file_name()
                .is_some_and(crate::backend::installation::private_stage)
    }

    fn last_used(&self) -> Result<Option<std::time::SystemTime>, Error> {
        if self.unpublished_game() {
            return Ok(None); // Inventory never stamps a private publisher's staged directory.
        }
        let Some(root) = &self.directory else {
            return Ok(None);
        };
        let meta = match fs::symlink_metadata(&self.path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if linked(&meta) {
            return Err(Error::Unsafe(self.path.clone()));
        }
        let (relative, stamp) = if meta.is_dir() {
            (
                self.relative.as_path(),
                std::ffi::OsString::from(".last-used"),
            )
        } else {
            let Some(name) = self.relative.file_name() else {
                return Ok(None);
            };
            let mut stamp = std::ffi::OsString::from(".");
            stamp.push(name);
            stamp.push(".last-used");
            (self.relative.parent().unwrap_or(Path::new("")), stamp)
        };
        let mut directory = root.try_clone()?;
        for part in relative.components() {
            directory = directory.open_dir_nofollow(part.as_os_str())?;
        }
        use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
        let mut options = cap_std::fs::OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        match directory.open_with(&stamp, &options) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let meta = directory.symlink_metadata(&stamp)?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(Error::Unsafe(self.path.join(stamp)));
        }
        Ok(Some(meta.modified()?.into_std()))
    }

    /// The lease integration changes this classification, not policy or deletion code.
    fn access(&self) -> Result<Access, Error> {
        match self.category {
            Category::RustCompiler => Ok(Access::BuildTools),
            _ => Access::for_unit(&self.path),
        }
    }

    fn clear(&self) -> Result<(), Error> {
        if !self.path.try_exists()? {
            return Ok(());
        }
        let Some(root) = &self.directory else {
            return Ok(());
        };
        let access = self.access()?;
        if matches!(access, Access::LeasedGame) {
            return crate::cache_cleanup::delete_game_unit(&self.path);
        }
        access.check()?;
        let mut parent = root.try_clone()?;
        for part in self.relative.parent().unwrap_or(Path::new("")).components() {
            parent = parent.open_dir_nofollow(part.as_os_str())?;
        }
        Storage::clear_entry(root, &parent, Path::new(self.relative.file_name().unwrap()))
    }
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
    fn entries(&self) -> Result<Vec<Entry>, Error> {
        let mut entries = Vec::new();
        let build_root = Root::open(&self.root)?;
        for cache in BUILD_CACHES {
            if matches!(
                cache,
                BuildCache::Rust
                    | BuildCache::Source
                    | BuildCache::Tools
                    | BuildCache::PrebuiltOutput
                    | BuildCache::Prebuilt
                    | BuildCache::InternalsSync
            ) {
                let parent = self.root.join(cache.relative());
                if checked(&parent, true)?.is_some() {
                    for child in fs::read_dir(parent)? {
                        let child = child?;
                        if usage_stamp(Path::new(&child.file_name())) {
                            continue;
                        }
                        entries.push(Entry::fixed(
                            &build_root,
                            &Path::new(cache.relative()).join(child.file_name()),
                            cache.category(),
                        )?);
                    }
                }
            } else {
                add(
                    &mut entries,
                    &build_root,
                    cache.relative(),
                    cache.category(),
                )?;
            }
        }
        let build_files = Category::BuildFiles;
        for name in [
            "built.json",
            "internals.json",
            "build.log",
            "catalog.partial",
        ] {
            add(
                &mut entries,
                &build_root,
                &format!("native/{name}"),
                build_files,
            )?;
        }
        // The target lock files must remain in place. Their contents are inventoried here but
        // deletion goes through Cache, which also acquires Cargo's locks.
        let target = self.root.join("native/target");
        if checked(&target, true)?.is_some() {
            for child in fs::read_dir(&target)? {
                let child = child?;
                if crate::cache_cleanup::reserved(Path::new(&child.file_name()))
                    || usage_stamp(Path::new(&child.file_name()))
                {
                    continue;
                }
                let relative = Path::new("native/target").join(child.file_name());
                checked(&child.path(), child.file_type()?.is_dir())?;
                if child.file_type()?.is_dir() && child.file_name().to_string_lossy().contains('-')
                {
                    // Cross-compilation target triples own independent profile units too.
                    for profile in fs::read_dir(child.path())? {
                        let profile = profile?;
                        if crate::cache_cleanup::reserved(Path::new(&profile.file_name()))
                            || usage_stamp(Path::new(&profile.file_name()))
                        {
                            continue;
                        }
                        entries.push(Entry::fixed(
                            &build_root,
                            &relative.join(profile.file_name()),
                            build_files,
                        )?);
                    }
                } else {
                    entries.push(Entry::fixed(&build_root, &relative, build_files)?);
                }
            }
        }
        let data = Root::open(&backend::data_dir())?;
        let played: HashSet<_> = instances::list()
            .into_iter()
            .filter_map(|instance| instance.resolve_version(None).ok())
            .collect();
        for version in backend::installed() {
            if native_mods::is_native(&version)
                && !played.contains(&version)
                && native_mods::resource_dependent(&version).is_none()
                && !backend::in_use(&version)
            {
                add(
                    &mut entries,
                    &data,
                    &format!("versions/{version}"),
                    Category::SpareGameBuilds,
                )?;
            }
        }
        // Interrupted removal deliberately loses completion. Keep the remaining unit
        // visible for residual reporting and a safe retry, even though installed() omits it.
        match fs::read_dir(data.path.join("versions")) {
            Ok(children) => {
                for child in children {
                    let child = child?;
                    if child.file_type()?.is_dir()
                        && !child.path().join(".installed").is_file()
                        && !child.file_name().to_string_lossy().starts_with(".trash-")
                    {
                        entries.push(Entry::fixed(
                            &data,
                            &Path::new("versions").join(child.file_name()),
                            Category::SpareGameBuilds,
                        )?);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        for relative in ["cache/github", "mod-browser", "profile-heads"] {
            add(&mut entries, &data, relative, Category::OtherCaches)?;
        }
        for instance in instances::list() {
            // A missing version cannot prove that the instance is idle. Keep its caches.
            let Ok(version) = instance.resolve_version(None) else {
                continue;
            };
            if backend::in_use(&version) {
                continue;
            }
            let root = Root::open(&instance.home)?;
            old_logs(&mut entries, &root)?;
            unused_jars(&mut entries, &root)?;
            for relative in [".sunrise-cache", "cache"] {
                add(&mut entries, &root, relative, Category::OtherCaches)?;
            }
        }
        // Detached trees can remain beside any managed cache after an interrupted removal.
        let parents: HashSet<_> = entries
            .iter()
            .filter_map(|entry| entry.path.parent().map(Path::to_owned))
            .chain([
                self.root.clone(),
                data.path.clone(),
                data.path.join("versions"),
            ])
            .collect();
        for parent in parents {
            let root = Root::open(&parent)?;
            if root.directory.is_none() {
                continue;
            }
            for entry in fs::read_dir(&parent)? {
                let entry = entry?;
                if entry.file_name().to_string_lossy().starts_with(".trash-") {
                    entries.push(Entry::fixed(
                        &root,
                        Path::new(&entry.file_name()),
                        Category::OtherCaches,
                    )?);
                }
            }
        }
        Ok(entries)
    }

    pub(super) fn stamp_build(&self) -> Result<(), Error> {
        for entry in self.entries()? {
            if !entry.path.starts_with(&self.root) || !entry.path.is_dir() {
                continue;
            }
            let relative = entry.path.strip_prefix(&self.root).unwrap();
            if !(relative == Path::new("cargo")
                || relative == Path::new("tmp")
                || (relative.starts_with("rust")
                    && entry.path.file_name().is_some_and(|name| {
                        name.to_string_lossy()
                            .starts_with(crate::rust_toolchain::VERSION)
                    })))
            {
                continue;
            }
            let Some(root) = &entry.directory else {
                continue;
            };
            let mut directory = root.try_clone()?;
            for part in entry.relative.components() {
                directory = directory.open_dir_nofollow(part.as_os_str())?;
            }
            use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
            let mut options = cap_std::fs::OpenOptions::new();
            options
                .write(true)
                .create(true)
                .truncate(true)
                .follow(FollowSymlinks::No);
            directory.open_with(".last-used", &options)?;
        }
        Ok(())
    }

    /// Not deletion candidates: an unproved worker can still be writing to them.
    fn retained_compiler_bytes(
        &self,
        counted: &mut HashSet<(u64, u64)>,
    ) -> Result<Option<u64>, Error> {
        let native = self.root.join("native");
        let mut bytes = None;
        if checked(&native, true)?.is_some() {
            for entry in fs::read_dir(&native)? {
                let entry = entry?;
                if entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with(native_mods::ATTEMPT_PREFIX))
                {
                    // A changing or unreadable tree is still protected; zero means its size is unknown.
                    let size = Tree::build(&entry.path()).map_or(0, |tree| tree.size(counted));
                    bytes = Some(bytes.unwrap_or(0u64).saturating_add(size));
                }
            }
        }
        Ok(bytes)
    }

    pub(super) fn evict_inventory(
        &self,
        now: std::time::SystemTime,
        max_bytes: Option<u64>,
        native_allowed: bool,
    ) -> Result<Report, Error> {
        self.evict_inventory_observed(now, max_bytes, native_allowed, |_| {})
    }

    pub(crate) fn evict_inventory_observed(
        &self,
        now: std::time::SystemTime,
        max_bytes: Option<u64>,
        native_allowed: bool,
        mut before_clear: impl FnMut(&Path),
    ) -> Result<Report, Error> {
        use super::policy::{self, Tier, Unit};
        let entries = self.entries()?;
        for entry in &entries {
            entry.last_used()?;
        }
        let mut revisions = HashSet::new();
        let mut unknown_revision = false;
        let mut native_enabled = false;
        for instance in instances::list() {
            for bundle in crate::native_status::bundles_on(&instance.home) {
                if let Some(native) = bundle.manifest.native {
                    native_enabled = true;
                    revisions.insert(native.sunrise);
                }
            }
            match instance.resolve_version(None) {
                Ok(version) => {
                    if let Some(about) = native_mods::about(&version) {
                        revisions.insert(about.sunrise);
                    } else if let Some(nightly) = version.strip_prefix("nightly-") {
                        revisions.insert(nightly.rsplit('-').next().unwrap_or(nightly).to_owned());
                    } else {
                        unknown_revision = true;
                    }
                }
                Err(_) => unknown_revision = true,
            }
        }
        // One walk of each folder serves its unit size, its row's bytes and its kept size.
        let mut trees = entries
            .iter()
            .map(|entry| Tree::build(&entry.path))
            .collect::<Result<Vec<_>, Error>>()?;
        let mut seen = HashSet::new();
        let retained = self.retained_compiler_bytes(&mut seen)?;
        let mut units: Vec<_> = entries
            .iter()
            .zip(&trees)
            .map(|(entry, tree)| {
                let last_used = entry.last_used()?;
                let tier = if entry.relative.starts_with("rust")
                    && entry.path.file_name().is_some_and(|name| {
                        name.to_string_lossy()
                            .starts_with(crate::rust_toolchain::VERSION)
                    })
                    && native_allowed
                    && native_enabled
                {
                    // Conservatively retain the whole toolchain set while consent is enabled.
                    Tier::Protected
                } else if entry.leftover() {
                    Tier::Expiring
                } else if [
                    "native/source",
                    "native/tools",
                    "native/internals-sync",
                    "native/prebuilt",
                ]
                .iter()
                .any(|root| entry.relative.starts_with(root))
                {
                    let Some(unit_revision) = entry.revision() else {
                        return Ok(Unit {
                            eligible: false,
                            bytes: tree.size(&mut seen),
                            last_used,
                            tier: Tier::Protected,
                        });
                    };
                    let tool = entry.relative.starts_with("native/tools");
                    let used = unknown_revision
                        || (tool && native_mods::MIXINS_REV.get(..12) == Some(unit_revision))
                        || revisions.iter().any(|revision| {
                            let length = revision.len().min(unit_revision.len());
                            revision.get(..length) == unit_revision.get(..length)
                        });
                    if used && entry.category == Category::PrebuiltGames {
                        Tier::Lru
                    } else if used {
                        Tier::Protected
                    } else {
                        Tier::Expiring
                    }
                } else if entry.relative.starts_with("rust")
                    || entry
                        .path
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with(".trash-"))
                {
                    Tier::Expiring
                } else {
                    Tier::Lru
                };
                Ok(Unit {
                    eligible: entry.access()?.check().is_ok(),
                    bytes: tree.size(&mut seen),
                    last_used,
                    tier,
                })
            })
            .collect::<Result<_, Error>>()?;
        if let Some(bytes) = retained {
            units.push(Unit {
                bytes,
                tier: Tier::Protected,
                eligible: false,
                last_used: None,
            });
        }
        let selected = policy::select(&units, max_bytes, now);
        let mut report = self.apply_retaining(Vec::new(), None, false, true)?;
        for row in &mut report.rows {
            row.bytes = allocated(&entries, &trees, row.category);
        }
        for (index, entry) in entries.iter().enumerate() {
            let row = report
                .rows
                .iter_mut()
                .find(|row| row.category == entry.category)
                .unwrap();
            if selected.contains(&index) {
                before_clear(&entry.path);
                let result = if entry.path == self.root.join("native/target") {
                    self.lease.clear().map(|_| ())
                } else {
                    entry.clear()
                };
                match result {
                    // Only a cleared folder is walked again, to see what is left of it.
                    Ok(()) => trees[index] = Tree::build(&entry.path)?,
                    // A refusal comes before any deletion: the folder is as it was walked.
                    Err(
                        error @ (Error::Busy
                        | Error::UnprovenExclusion
                        | Error::IncompleteUnit
                        | Error::UnpublishedGame
                        | Error::ResourceDependent(_)
                        | Error::ReferencedProfile
                        | Error::UnprovenReferences),
                    ) => row.kept.push(Kept {
                        bytes: trees[index].size(&mut HashSet::new()),
                        reason: native_mods::cache_error(error),
                    }),
                    Err(error) => return Err(error),
                }
            }
            let remaining = trees[index].size(&mut HashSet::new());
            if !selected.contains(&index) && remaining > 0 {
                row.kept.push(Kept {
                    bytes: remaining,
                    reason: if !units[index].eligible {
                        native_mods::cache_error(
                            entry
                                .access()?
                                .check()
                                .err()
                                .unwrap_or(Error::UnprovenExclusion),
                        )
                    } else {
                        crate::i18n::tr("Protected, recently used, or last use unknown.").into()
                    },
                });
            }
        }
        for row in &mut report.rows {
            row.remaining_bytes = allocated(&entries, &trees, row.category);
            row.freed_bytes = row.bytes.saturating_sub(row.remaining_bytes);
        }
        report.total_bytes = report.rows.iter().map(|row| row.bytes).sum();
        report.include_retained_compiler(retained);
        Ok(report)
    }

    pub fn report(&self, clear: Option<Category>, all: bool) -> Result<Report, Error> {
        let registry_entries = match fs::read_dir(backend::data_dir().join("run-leases")) {
            Ok(mut entries) => entries.try_fold(0usize, |count, entry| entry.map(|_| count + 1))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error.into()),
        };
        eprintln!(
            "{}",
            crate::i18n::trf(
                "Runtime lease registry entries: {count}",
                &[("count", &registry_entries.to_string())]
            )
        );
        // Also refuse an external Cargo holding a target lock before starting a cleanup.
        self.lease.bytes()?;
        if clear.is_some() || all {
            use sunrise_launcher_smod::runtime_leases::{cleanup::Cleanup, Error as LeaseError};
            match Cleanup::existing(&backend::data_dir()) {
                Ok(cleanup) => {
                    cleanup.collect_records()?;
                }
                Err(LeaseError::Busy) => {} // Other cache classes can still be reported/retained.
                Err(LeaseError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        let entries = self.entries()?;
        let mut report = self.apply_retaining(entries, clear, all, true)?;
        report.include_retained_compiler(self.retained_compiler_bytes(&mut HashSet::new())?);
        if clear.is_none() && !all {
            for entry in self.entries()? {
                if let Err(error) = entry.access().and_then(|access| access.check()) {
                    // A refused unit already has its line.
                    let bytes = match allocated_bytes(&entry.path) {
                        Err(Error::Unsafe(_)) => 0,
                        bytes => bytes?,
                    };
                    if bytes > 0 {
                        report
                            .rows
                            .iter_mut()
                            .find(|row| row.category == entry.category)
                            .unwrap()
                            .kept
                            .push(Kept {
                                bytes,
                                reason: native_mods::cache_error(error),
                            });
                    }
                }
            }
        }
        Ok(report)
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

    fn apply_retaining(
        &self,
        entries: Vec<Entry>,
        clear: Option<Category>,
        all: bool,
        retain: bool,
    ) -> Result<Report, Error> {
        // Validate every descendant before any deletion and deduplicate allocated blocks per row.
        let mut rows: Vec<_> = Category::ALL
            .into_iter()
            .map(|category| Row {
                category,
                bytes: 0,
                freed_bytes: 0,
                remaining_bytes: 0,
                private_compiler_bytes: 0,
                kept: Vec::new(),
            })
            .collect();
        // Each folder is walked once, here: the walk validates every descendant and its sizes
        // serve the rows and the kept lines.
        let (entries, mut trees) = if retain {
            refused(entries, &mut rows)?
        } else {
            let trees = entries
                .iter()
                .map(|entry| Tree::build(&entry.path))
                .collect::<Result<Vec<_>, Error>>()?;
            (entries, trees)
        };
        for row in &mut rows {
            row.bytes = allocated(&entries, &trees, row.category);
            row.remaining_bytes = row.bytes;
        }
        for (index, entry) in entries.iter().enumerate() {
            if all || clear == Some(entry.category) {
                let result = if entry.path == self.root.join("native/target") {
                    self.lease.clear().map(|_| ())
                } else {
                    entry.clear()
                };
                match result {
                    // Only a cleared folder is walked again, to see what is left of it.
                    Ok(()) => trees[index] = Tree::build(&entry.path)?,
                    Err(
                        error @ (Error::Busy
                        | Error::UnprovenExclusion
                        | Error::IncompleteUnit
                        | Error::UnpublishedGame
                        | Error::ResourceDependent(_)
                        | Error::ReferencedProfile
                        | Error::UnprovenReferences),
                    ) if retain => {
                        let row = rows
                            .iter_mut()
                            .find(|row| row.category == entry.category)
                            .unwrap();
                        row.kept.push(Kept {
                            bytes: trees[index].size(&mut HashSet::new()),
                            reason: native_mods::cache_error(error),
                        });
                        // A busy build may be using subsequent build-tool units too. Stop
                        // this clear attempt rather than deleting more after contention.
                        if matches!(entry.access()?, Access::BuildTools) {
                            break;
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        for row in &mut rows {
            if all || clear == Some(row.category) {
                let remaining = allocated(&entries, &trees, row.category);
                row.freed_bytes = row.bytes.saturating_sub(remaining);
                row.remaining_bytes = remaining;
            }
        }
        Ok(Report {
            total_bytes: rows
                .iter()
                .fold(0u64, |total, row| total.saturating_add(row.bytes)),
            rows,
        })
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
fn refused(entries: Vec<Entry>, rows: &mut [Row]) -> Result<(Vec<Entry>, Vec<Tree>), Error> {
    let mut sound = Vec::new();
    let mut trees = Vec::new();
    for entry in entries {
        match Tree::build(&entry.path) {
            Ok(tree) => {
                sound.push(entry);
                trees.push(tree);
            }
            Err(error @ Error::Unsafe(_)) => rows
                .iter_mut()
                .find(|row| row.category == entry.category)
                .unwrap()
                .kept
                .push(Kept {
                    bytes: 0,
                    reason: native_mods::cache_error(error),
                }),
            Err(error) => return Err(error),
        }
    }
    Ok((sound, trees))
}

fn old_logs(entries: &mut Vec<Entry>, home: &Root) -> Result<(), Error> {
    for directory in ["logs", "crash-reports"] {
        let folder = Entry::fixed(home, Path::new(directory), Category::OtherCaches)?.path;
        if checked(&folder, true)?.is_none() {
            continue;
        }
        let mut files = Vec::new();
        for entry in fs::read_dir(&folder)? {
            let entry = entry?;
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

fn unused_jars(entries: &mut Vec<Entry>, home: &Root) -> Result<(), Error> {
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
        let path = Entry::fixed(home, Path::new(directory), Category::OtherCaches)?.path;
        for package in sunrise_launcher_smod::find(&path) {
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
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
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
mod eviction_tests {
    use super::*;
    #[test]
    fn first_seen_stamp_ages_without_being_refreshed_by_inventory() {
        let home = crate::storage::tests::Home::new();
        home.seed("rust/old/payload");
        let root = Root::open(&home.0).unwrap();
        let entry = Entry::fixed(&root, Path::new("rust/old"), Category::RustCompiler).unwrap();
        let used = entry.last_used().unwrap().unwrap();
        assert_eq!(entry.last_used().unwrap(), Some(used));
        let units = [policy::Unit {
            bytes: 100,
            eligible: true,
            last_used: Some(used),
            tier: policy::Tier::Expiring,
        }];
        assert!(policy::select(&units, None, used).is_empty());
        assert_eq!(
            policy::select(
                &units,
                None,
                used + std::time::Duration::from_secs(7 * 86400)
            ),
            [0]
        );
    }

    #[test]
    fn revision_names_are_parsed_not_searched() {
        let home = crate::storage::tests::Home::new();
        let root = Root::open(&home.0).unwrap();
        let sha = "0123456789abcdef0123456789abcdef01234567";
        for (path, expected) in [
            (format!("native/source/{sha}"), Some(sha)),
            (format!("native/source/not-{sha}"), None),
            (format!("native/tools/{}", &sha[..12]), Some(&sha[..12])),
            (
                format!("native/prebuilt/{sha}-sunrise-prebuilt-v3-x86_64-unknown-linux-gnu"),
                Some(sha),
            ),
            (
                format!("native/prebuilt/{sha}-sunrise-prebuilt-v3-x86_64-unknown-linux-gnu-server"),
                Some(sha),
            ),
            (
                format!("native/prebuilt/{sha}-sunrise-prebuilt-v4-x86_64-unknown-linux-gnu"),
                Some(sha),
            ),
            (
                format!("native/prebuilt/{sha}-sunrise-prebuilt-v4-x86_64-unknown-linux-gnu-server"),
                Some(sha),
            ),
            (format!("native/prebuilt/{sha}-sunrise-prebuilt-v3-unknown-target"), None),
            (format!("native/prebuilt/{sha}-sunrise-prebuilt-v4-unknown-target"), None),
            (format!("native/prebuilt/{sha}-sunrise-prebuilt-v30-x86_64-unknown-linux-gnu"), None),
            (format!("native/prebuilt/{sha}-sunrise-prebuilt-v4-x86_64-unknown-linux-gnu-server-server"), None),
            (format!("native/prebuilt/{sha}-sunrise-prebuilt-v4-x86_64-unknown-linux-gnu.tar.gz"), None),
            (format!("native/prebuilt/not-{sha}-sunrise-prebuilt-v4-x86_64-unknown-linux-gnu"), None),
            // A build's SDK slices, and a developer install's laid-out game.
            (format!("native/prebuilt/{sha}-x86_64-unknown-linux-gnu"), Some(sha)),
            (format!("native/prebuilt/{sha}-x86_64-unknown-linux-gnu-server"), Some(sha)),
            (format!("native/prebuilt/{sha}-x86_64-unknown-linux-gnu.developer"), Some(sha)),
            (format!("native/prebuilt/{sha}-unknown-target"), None),
            (format!("native/prebuilt/{sha}-x86_64-unknown-linux-gnu.partial"), None),
            (
                format!("native/internals-sync/catalog-{sha}-x86_64-unknown-linux-gnu"),
                Some(sha),
            ),
            (format!("native/internals-sync/garbage-{sha}"), None),
        ] {
            assert_eq!(
                Entry::fixed(&root, Path::new(&path), Category::BuildFiles)
                    .unwrap()
                    .revision(),
                expected
            );
        }
        // What an interrupted unpack or download left is known as such, and nothing else is.
        for (path, leftover) in [
            (format!("native/prebuilt/.{sha}-sunrise-prebuilt-v6-x86_64-unknown-linux-gnu.tar.gz.partial"), true),
            (format!("native/prebuilt/{sha}-x86_64-unknown-linux-gnu.lock"), true),
            ("native/prebuilt/runtime-x86_64-unknown-linux-gnu.tar.xz.download".to_owned(), true),
            (format!("native/prebuilt/{sha}-x86_64-unknown-linux-gnu"), false),
            (format!("native/source/{sha}.partial"), false),
        ] {
            let entry = Entry::fixed(&root, Path::new(&path), Category::BuildFiles).unwrap();
            assert_eq!(entry.leftover(), leftover, "{path}");
        }
    }
    #[test]
    fn refused_units_are_residual_with_bytes_and_reason() {
        let home = crate::storage::tests::Home::new();
        let file = home.seed("game/output");
        let storage = Storage::at(&home.0).unwrap();
        let root = Root::open(&home.0).unwrap();
        let entries = vec![Entry::fixed(&root, Path::new("game"), Category::BuildFiles).unwrap()];
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
            .find(|row| row.category == Category::BuildFiles)
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

    /// The walk the eviction pass made before one walk served every figure: a fresh recursive
    /// walk with its own set of counted blocks, as `allocated` and `allocated_bytes` made it.
    fn legacy_walk(path: &Path, seen: &mut HashSet<(u64, u64)>) -> u64 {
        let meta = match fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(_) => return 0,
        };
        let (id, bytes) = super::super::allocation(path, &meta).unwrap();
        if !seen.insert(id) {
            return 0;
        }
        let mut total = bytes;
        if meta.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                total += legacy_walk(&entry.unwrap().path(), seen);
            }
        }
        total
    }

    fn legacy_allocated(entries: &[Entry], category: Category) -> u64 {
        let mut seen = HashSet::new();
        entries
            .iter()
            .filter(|entry| entry.category == category)
            .map(|entry| legacy_walk(&entry.path, &mut seen))
            .sum()
    }

    #[test]
    fn retained_compiler_bytes_use_budget_but_cannot_be_cleared() {
        let home = crate::storage::tests::Home::new();
        let cache = home.seed("cargo/registry/download");
        let storage = Storage::at(&home.0).unwrap();
        let now = std::time::SystemTime::now();
        let ordinary = storage.evict_inventory(now, None, false).unwrap();
        assert!(ordinary.total_bytes > 0 && cache.exists());
        let old_worker = home.seed("native/.native-attempt-old/compiler/work");
        let report = storage
            .evict_inventory(now, Some(ordinary.total_bytes), false)
            .unwrap();
        assert!(
            !cache.exists(),
            "retained worker must count toward the cache limit"
        );
        assert!(
            old_worker.is_file(),
            "the old worker is never a deletion candidate"
        );
        let protected = &report.rows[0];
        assert_eq!(protected.category, Category::BuildFiles);
        assert!(protected.remaining_bytes > 0);
        assert_eq!(protected.bytes, protected.remaining_bytes);
        assert_eq!(protected.clearable_bytes(), 0);
        assert_eq!(protected.kept[0].bytes, protected.remaining_bytes);
        let manual = storage.report(Some(Category::BuildFiles), false).unwrap();
        assert_eq!(manual.rows[0].clearable_bytes(), 0);
        assert!(old_worker.is_file());
    }

    #[test]
    fn one_walk_per_folder_reports_what_the_repeated_walks_did() {
        let home = crate::storage::tests::Home::new();
        let shared = home.seed("rust/old/payload");
        home.seed("rust/old/deep/more");
        // A file shared by a cleared compiler folder and a kept build folder, and one shared by
        // two folders of the same row: each row counts a block once, whatever the other rows hold.
        fs::create_dir_all(home.0.join("native/source/not-a-revision")).unwrap();
        fs::hard_link(&shared, home.0.join("native/source/not-a-revision/shared")).unwrap();
        let twin = home.seed("rust/old2/own");
        fs::hard_link(&twin, home.0.join("rust/old2/twin")).unwrap();
        // Kept: recently used (cargo) and protected (a source folder with no revision in its name).
        home.seed("cargo/registry/crate");
        home.seed("native/source/not-a-revision/file");
        let storage = Storage::at(&home.0).unwrap();
        let before_entries = storage.entries().unwrap();
        let count = before_entries.len();
        let sizes: Vec<u64> = before_entries
            .iter()
            .map(|entry| legacy_walk(&entry.path, &mut HashSet::new()))
            .collect();
        let before: Vec<u64> = Category::ALL
            .iter()
            .map(|category| legacy_allocated(&before_entries, *category))
            .collect();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(8 * 86400);
        let walks_before = crate::storage::walks();
        let report = storage
            .evict_inventory(later, Some(25 << 30), false)
            .unwrap();
        let walked = crate::storage::walks() - walks_before;
        let entries = storage.entries().unwrap();
        let cleared = before_entries
            .iter()
            .zip(&sizes)
            .filter(|(entry, size)| {
                **size > 0 && legacy_walk(&entry.path, &mut HashSet::new()) != **size
            })
            .count();
        // Each folder once, and once more only the folders that were cleared.
        assert_eq!(walked, count + cleared);
        let mut total = 0;
        let mut kept_any = false;
        for (row, bytes) in report.rows.iter().zip(&before) {
            let remaining = legacy_allocated(&entries, row.category);
            assert_eq!(row.bytes, *bytes, "{:?}", row.category);
            assert_eq!(row.remaining_bytes, remaining, "{:?}", row.category);
            assert_eq!(row.freed_bytes, bytes.saturating_sub(remaining));
            // A folder left as it was (not cleared) and holding something is kept.
            let kept: Vec<u64> = before_entries
                .iter()
                .zip(&sizes)
                .filter(|(entry, _)| entry.category == row.category)
                .map(|(entry, size)| (legacy_walk(&entry.path, &mut HashSet::new()), *size))
                .filter(|(now, size)| now == size && *now > 0)
                .map(|(now, _)| now)
                .collect();
            assert_eq!(
                row.kept.iter().map(|kept| kept.bytes).collect::<Vec<_>>(),
                kept,
                "{:?}",
                row.category
            );
            kept_any |= !kept.is_empty();
            total += bytes;
        }
        assert_eq!(report.total_bytes, total);
        assert!(kept_any);
        assert!(home.0.join("cargo/registry/crate").exists());
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
        assert!(report.rows[3].freed_bytes > 0);
        #[cfg(unix)]
        assert_eq!(report.rows[4].kept.len(), 1);
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "manual allocated 40 GiB proof; requires SUNRISE_EVICTION_PROOF_ROOT in checkout scratch"]
    fn allocated_forty_gib_cleanup_proof() {
        let path = PathBuf::from(
            std::env::var_os("SUNRISE_EVICTION_PROOF_ROOT").expect("isolated proof root"),
        );
        fs::create_dir_all(&path).unwrap();
        let home = crate::storage::tests::Home(path.canonicalize().unwrap());
        for (folder, gib) in [
            ("rust/old", 12),
            ("cargo", 12),
            ("tmp", 4),
            ("native/patched/proof", 4),
            ("versions/native-leased", 4),
            ("versions/native-legacy", 4),
        ] {
            let file = home.seed(&format!("{folder}/payload"));
            assert!(std::process::Command::new("fallocate")
                .args(["-l", &format!("{gib}G")])
                .arg(file)
                .status()
                .unwrap()
                .success());
        }
        assert_eq!(backend::data_dir().canonicalize().unwrap(), home.0);
        let leased = home.0.join("versions/native-leased");
        let legacy = home.0.join("versions/native-legacy");
        for directory in [&leased, &legacy] {
            fs::write(directory.join(".installed"), b"").unwrap();
        }
        fs::write(
            leased.join(backend::CLIENT),
            sunrise_launcher_smod::holder::CAPABILITY,
        )
        .unwrap();
        fs::write(legacy.join(backend::CLIENT), b"pre-holder game").unwrap();
        crate::run_leases::publish(&leased).unwrap();
        let registry = sunrise_launcher_smod::runtime_leases::Registry::existing(&home.0).unwrap();
        let _game = registry
            .lock(
                &sunrise_launcher_smod::runtime_leases::open_directory(&leased).unwrap(),
                sunrise_launcher_smod::locks::LockMode::Shared,
            )
            .unwrap();
        let storage = Storage::at(&home.0).unwrap();
        assert!(matches!(Cache::acquire(&home.0), Err(Error::Busy)));
        let before = allocated_bytes(&home.0).unwrap();
        let now = std::time::SystemTime::now();
        let first = storage.evict_inventory(now, Some(25 << 30), false).unwrap();
        let later = now + std::time::Duration::from_secs(8 * 86400);
        let second = storage
            .evict_inventory(later, Some(25 << 30), false)
            .unwrap();
        let freed: u64 = first
            .rows
            .iter()
            .chain(&second.rows)
            .map(|row| row.freed_bytes)
            .sum();
        let residual = allocated_bytes(&home.0).unwrap();
        println!("allocated_before={before} inventoried_before={} freed={freed} allocated_residual={residual}", first.total_bytes);
        assert!(before >= 40 * (1 << 30));
        assert!(freed >= 28 * (1 << 30));
        assert!(residual >= 12 * (1 << 30));
        assert!(residual < 25 * (1 << 30));
        assert!(leased.join("payload").exists());
        assert!(legacy.join("payload").exists());
        assert!(home.0.join("native/patched/proof/payload").exists());
        assert!(second.rows.iter().any(|row| row
            .kept
            .iter()
            .any(|kept| kept.bytes >= 4 * (1 << 30) && !kept.reason.is_empty())));
        assert!(matches!(Cache::acquire(&home.0), Err(Error::Busy)));
    }
}
