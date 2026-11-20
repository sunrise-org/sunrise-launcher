//! The exact, bounded live steps of a Switch. No registry, resolver or compiler participates in
//! the publishing phase or in recovery.
use super::{
    build, durable, read, unsettled, write, Active, FirstSwitch, Image, Intent, Lease, Record,
};
use cap_fs_ext::MetadataExt;
use cap_std::{ambient_authority, fs::Dir};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Phase {
    Staging,
    Ready,
    Publishing,
    Undoing,
    Committed,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Identity {
    pub(super) volume: u64,
    id: u64,
}

impl Identity {
    pub(super) fn of(path: &Path) -> Result<Self, String> {
        build::require_dir(path)?;
        let meta = Dir::open_ambient_dir(path, ambient_authority())
            .and_then(|dir| dir.dir_metadata())
            .map_err(|err| format!("{}: {err}", path.display()))?;
        Ok(Self {
            volume: meta.dev(),
            id: meta.ino(),
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    name: PathBuf,
    parent: Identity,
    before: Image,
    after: Image,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Journal {
    phase: Phase,
    home: Identity,
    staging: String,
    staging_id: Identity,
    before: Option<Vec<u8>>,
    target: String,
    after: Option<Vec<u8>>,
    directories: BTreeMap<PathBuf, Identity>,
    files: Vec<File>,
    /// The steps that may have run, or the steps still owed in reverse while undoing.
    /// Prepared directories come first; the pointer is always last.
    frontier: usize,
}

impl Journal {
    pub(super) fn validate(
        &self,
        same_base: bool,
        bootstrap: Option<&build::Bootstrap>,
    ) -> Result<(), String> {
        let before = self.before.as_deref().map(build::parse).transpose()?;
        let (profile, generation) = match (before.as_ref(), bootstrap) {
            (Some(before), None) if (before.base() == self.target) == same_base => {
                (before.profile, before.generation)
            }
            (None, Some(bootstrap)) if bootstrap.target.base() == self.target => {
                (bootstrap.profile, 0)
            }
            _ => return Err(unsettled()),
        };
        build::validate_tag(&self.target)?;
        let mut destinations = BTreeSet::new();
        if !self
            .staging
            .strip_prefix(".install-txn-")
            .is_some_and(|name| !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric()))
            || self.frontier > self.steps() + 1
            || self
                .directories
                .iter()
                .any(|(name, id)| !valid_root(name) || id.volume != self.home.volume)
            || self.files.iter().any(|step| {
                !valid_destination(&step.name)
                    || !destinations.insert(destination_key(&step.name))
                    || self.home.volume != step.parent.volume
            })
            || self.home.volume != self.staging_id.volume
        {
            return Err(unsettled());
        }
        match (self.phase, self.after.as_deref()) {
            (Phase::Staging, None)
                if self.directories.is_empty() && self.files.is_empty() && self.frontier == 0 =>
            {
                Ok(())
            }
            (Phase::Ready | Phase::Publishing | Phase::Undoing | Phase::Committed, Some(after)) => {
                let after = build::parse(after)?;
                if after.profile != profile
                    || after.generation.checked_sub(1) != Some(generation)
                    || after.base() != self.target
                    || (same_base
                        && before.is_some_and(|before| !before.same_base_artifact(&after)))
                    || (self.phase == Phase::Ready && self.frontier != 0)
                    || (self.phase == Phase::Committed && self.frontier != self.steps() + 1)
                {
                    return Err(unsettled());
                }
                Ok(())
            }
            _ => Err(unsettled()),
        }
    }

    pub(super) fn references(&self, tag: &str) -> Result<bool, String> {
        let before = self.before.as_deref().map(build::parse).transpose()?;
        let after = self.after.as_deref().map(build::parse).transpose()?;
        let same = |name: &str| {
            if cfg!(windows) {
                name.eq_ignore_ascii_case(tag)
            } else {
                name == tag
            }
        };
        Ok(same(&self.target)
            || before.is_some_and(|before| same(before.base()) || same(before.executable()))
            || after.is_some_and(|image| same(image.executable())))
    }

    fn staging(&self, home: &Path) -> PathBuf {
        home.join("mod-browser").join(&self.staging)
    }

    fn verify_roots(&self, home: &Path) -> Result<(), String> {
        if Identity::of(home)? != self.home || Identity::of(&self.staging(home))? != self.staging_id
        {
            return Err(unsettled());
        }
        Ok(())
    }

    fn steps(&self) -> usize {
        self.directories.len() + self.files.len()
    }

    fn directory(&self, home: &Path, index: usize) -> Result<(PathBuf, PathBuf, Identity), String> {
        let (name, id) = self.directories.iter().nth(index).ok_or_else(unsettled)?;
        Ok((self.staging(home).join(name), home.join(name), *id))
    }

    fn file_target(&self, home: &Path, index: usize) -> Result<PathBuf, String> {
        let file = &self.files[index];
        let path = home.join(&file.name);
        if Identity::of(path.parent().ok_or_else(unsettled)?)? != file.parent {
            return Err(unsettled());
        }
        Ok(path)
    }
}

fn valid_root(path: &Path) -> bool {
    use sunrise_launcher_smod::mod_list::{DIR, NATIVE_DIR};
    path.as_os_str() == DIR || path.as_os_str() == NATIVE_DIR
}

fn require_absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(fail_at(path, error)),
        Ok(_) => Err(unsettled()),
    }
}

fn valid_destination(path: &Path) -> bool {
    use std::path::Component::Normal;
    let mut parts = path.components();
    let (Some(Normal(first)), second, None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    use sunrise_launcher_smod::mod_list::{DIR, FILE, NATIVE_DIR};
    match second {
        None => first == FILE || first == crate::browser::installed::MANIFEST,
        Some(Normal(file)) => {
            (first == "mod-browser" && matches!(file.to_str(), Some("pack.json" | "pack.spack")))
                || ((first == DIR || first == NATIVE_DIR)
                    && file.to_str().is_some_and(|name| {
                        (name.ends_with(".smod") && crate::browser::safe_name(name).is_some())
                            || (first == NATIVE_DIR
                                && name
                                    .strip_suffix(".origin.json")
                                    .is_some_and(sunrise_launcher_smod::index::valid_id))
                    }))
        }
        _ => false,
    }
}

pub(super) fn valid_package_destination(path: &Path) -> bool {
    use sunrise_launcher_smod::mod_list::{DIR, NATIVE_DIR};
    (path.starts_with(DIR) || path.starts_with(NATIVE_DIR))
        && path.extension().is_some_and(|ext| ext == "smod")
        && valid_destination(path)
}

pub(super) fn destination_key(path: &Path) -> String {
    let path = path.to_string_lossy();
    if cfg!(windows) || cfg!(target_os = "macos") {
        path.to_lowercase()
    } else {
        path.into_owned()
    }
}

fn fail_at(path: &Path, error: io::Error) -> String {
    format!("{}: {error}", path.display())
}

impl Lease {
    /// Reserve this profile before producing any candidate files. The private directory stays
    /// named in the record if the process dies; the next admission cancels it before Play.
    pub(super) fn reserve_switch(&self, prior: &Active, target: &str) -> Result<PathBuf, String> {
        self.reserve_staged(Some(prior), target, None, false)
            .map(|(path, _)| path)
    }

    pub(crate) fn reserve_install_set(&self, prior: &Active) -> Result<PathBuf, String> {
        self.reserve_staged(Some(prior), prior.base(), None, true)
            .map(|(path, _)| path)
    }

    pub(super) fn reserve_first_install(
        &self,
        bound: &super::Target,
    ) -> Result<(PathBuf, build::Bootstrap), String> {
        let (path, bootstrap) = self.reserve_staged(None, bound.base(), Some(bound), true)?;
        Ok((path, bootstrap.ok_or_else(unsettled)?))
    }

    pub(super) fn reserve_first_switch(
        &self,
        target: &super::Target,
    ) -> Result<(PathBuf, build::Bootstrap), String> {
        let (path, binding) = self.reserve_staged(None, target.base(), Some(target), false)?;
        Ok((path, binding.ok_or_else(unsettled)?))
    }

    fn reserve_staged(
        &self,
        prior: Option<&Active>,
        target: &str,
        bound: Option<&super::Target>,
        same_base: bool,
    ) -> Result<(PathBuf, Option<build::Bootstrap>), String> {
        let home = self.home();
        let _lock = super::locked(home)?;
        let _publication =
            crate::run_leases::installation().map_err(crate::native_mods::cache_error)?;
        let before = build::optional_bytes(&home.join(build::FILE))?;
        let record = read(home)?;
        let previous = record.as_ref().and_then(Record::bootstrap).cloned();
        let bootstrap = match (prior, before.as_deref(), bound) {
            (Some(prior), Some(before), None)
                if record.is_none() && build::parse(before)? == *prior =>
            {
                None
            }
            (None, None, Some(target))
                if record.as_ref().is_none_or(|record| {
                    record.staged().is_none()
                        && record.pointer.is_none()
                        && record.attempts.is_empty()
                }) =>
            {
                let binding = if same_base {
                    let binding = previous
                        .clone()
                        .filter(|binding| binding.target == *target)
                        .ok_or_else(unsettled)?;
                    binding.verify(home)?;
                    binding
                } else {
                    if target.download.is_some() {
                        return Err(unsettled());
                    }
                    let mut binding = build::Bootstrap::bind(home, target.clone())?;
                    if let Some(previous) = &previous {
                        binding.profile = previous.profile;
                    }
                    binding
                };
                binding.verify_artifact()?;
                Some(binding)
            }
            _ => return Err(unsettled()),
        };
        let root = home.join("mod-browser");
        match fs::create_dir(&root) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                build::require_dir(&root)?
            }
            Err(error) => return Err(fail_at(&root, error)),
        }
        let dir = tempfile::Builder::new()
            .prefix(".install-txn-")
            .tempdir_in(&root)
            .map_err(|error| fail_at(&root, error))?;
        let path = dir.path();
        let staging = path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .ok_or_else(unsettled)?
            .to_owned();
        let journal = Journal {
            phase: Phase::Staging,
            home: Identity::of(home)?,
            staging,
            staging_id: Identity::of(path)?,
            before,
            target: target.to_owned(),
            after: None,
            directories: BTreeMap::new(),
            files: Vec::new(),
            frontier: 0,
        };
        journal.validate(same_base, bootstrap.as_ref())?;
        let operation = Box::new(journal);
        let intent = if same_base {
            Intent::Install {
                bootstrap: bootstrap.clone().map(Box::new),
                staged: Some(operation),
            }
        } else {
            Intent::Switch {
                operation,
                first: bootstrap
                    .clone()
                    .map(|target| Box::new(FirstSwitch { target, previous })),
            }
        };
        // A reported flush error can still leave the record visible. Keep its directory.
        let path = dir.keep();
        write(
            home,
            &Record {
                intent,
                ..Record::default()
            },
        )?;
        Ok((path, bootstrap))
    }

    pub(crate) fn seal_staged_set(
        &self,
        next: &Active,
        files: &[(PathBuf, Option<PathBuf>)],
    ) -> Result<(), String> {
        let home = self.home();
        let _lock = super::locked(home)?;
        let _publication =
            crate::run_leases::installation().map_err(crate::native_mods::cache_error)?;
        let mut record = read(home)?.ok_or_else(unsettled)?;
        let same_base = matches!(record.intent, Intent::Install { .. });
        let bootstrap = record.staged_binding().cloned();
        let journal = operation(&mut record)?;
        if journal.phase != Phase::Staging {
            return Err(unsettled());
        }
        journal.verify_roots(home)?;
        let staging = journal.staging(home);
        for (i, (name, source)) in files.iter().enumerate() {
            if !valid_destination(name) {
                return Err(unsettled());
            }
            let live = home.join(name);
            let relative_parent = name.parent().ok_or_else(unsettled)?;
            let parent_path = home.join(relative_parent);
            let parent = match fs::symlink_metadata(&parent_path) {
                Err(error)
                    if error.kind() == io::ErrorKind::NotFound && valid_root(relative_parent) =>
                {
                    if let Some(id) = journal.directories.get(relative_parent) {
                        *id
                    } else {
                        let prepared = staging.join(relative_parent);
                        fs::create_dir(&prepared).map_err(|e| fail_at(&prepared, e))?;
                        let id = Identity::of(&prepared)?;
                        journal.directories.insert(relative_parent.to_owned(), id);
                        id
                    }
                }
                Err(error) => return Err(fail_at(&parent_path, error)),
                Ok(_) => Identity::of(&parent_path)?,
            };
            let before = Image::at(&live)?;
            if before != Image::Missing {
                let snapshot = staging.join(format!("old-{i}"));
                fs::copy(&live, &snapshot).map_err(|e| fail_at(&snapshot, e))?;
                if Image::at(&snapshot)? != before {
                    return Err(unsettled());
                }
                durable::sync_restored(&snapshot, &snapshot, false)
                    .map_err(|e| fail_at(&snapshot, e))?;
            }
            let after = match source {
                Some(source) => {
                    let snapshot = staging.join(format!("new-{i}"));
                    let image = Image::at(source)?;
                    if image == Image::Missing {
                        return Err(unsettled());
                    }
                    fs::copy(source, &snapshot).map_err(|e| fail_at(&snapshot, e))?;
                    if Image::at(&snapshot)? != image {
                        return Err(unsettled());
                    }
                    durable::sync_restored(&snapshot, &snapshot, false)
                        .map_err(|e| fail_at(&snapshot, e))?;
                    image
                }
                None => Image::Missing,
            };
            journal.files.push(File {
                name: name.clone(),
                parent,
                before,
                after,
            });
        }
        journal.after = Some(serde_json::to_vec(next).map_err(|e| e.to_string())?);
        journal.phase = Phase::Ready;
        journal.validate(same_base, bootstrap.as_ref())?;
        write(home, &record)
    }

    pub(crate) fn publish_staged_set<G>(
        &self,
        cutoff: impl FnOnce() -> Result<G, String>,
    ) -> Result<(), String> {
        let _lock = super::locked(self.home())?;
        let _publication =
            crate::run_leases::installation().map_err(crate::native_mods::cache_error)?;
        let mut record = read(self.home())?.ok_or_else(unsettled)?;
        let result = publish(self.home(), &mut record, cutoff, |_| {});
        if let Err(error) = result {
            // The on-disk marker is authority even if writing it reported an error.
            let mut disk = read(self.home())?.ok_or_else(unsettled)?;
            recover(self.home(), &mut disk)?;
            return Err(error);
        }
        recover(self.home(), &mut record)
    }
}

fn operation(record: &mut Record) -> Result<&mut Journal, String> {
    match &mut record.intent {
        Intent::Switch { operation, .. }
        | Intent::Install {
            staged: Some(operation),
            ..
        } => Ok(operation),
        Intent::Install { staged: None, .. } => Err(unsettled()),
    }
}

fn publish<G>(
    home: &Path,
    record: &mut Record,
    cutoff: impl FnOnce() -> Result<G, String>,
    mut boundary: impl FnMut(Phase),
) -> Result<(), String> {
    let journal = operation(record)?;
    if journal.phase != Phase::Ready {
        return Err(unsettled());
    }
    journal.verify_roots(home)?;
    let pointer = home.join(build::FILE);
    if build::optional_bytes(&pointer)? != journal.before {
        return Err(unsettled());
    }
    let directories = journal.directories.len();
    let count = journal.steps();
    for i in 0..directories {
        let (stage, target, id) = journal.directory(home, i)?;
        require_absent(&target)?;
        if Identity::of(&stage)? != id {
            return Err(unsettled());
        }
    }
    for (i, step) in journal.files.iter().enumerate() {
        let target = if journal
            .directories
            .contains_key(step.name.parent().ok_or_else(unsettled)?)
        {
            if step.before != Image::Missing {
                return Err(unsettled());
            }
            home.join(&step.name)
        } else {
            journal.file_target(home, i)?
        };
        let staging = journal.staging(home);
        if Image::at(&target)? != step.before
            || Image::at(&staging.join(format!("old-{i}")))? != step.before
            || Image::at(&staging.join(format!("new-{i}")))? != step.after
        {
            return Err(unsettled());
        }
    }
    let mut permission = Some(cutoff()?);
    journal.phase = Phase::Publishing;
    for step in 0..=count {
        operation(record)?.frontier = step + 1;
        write(home, record)?;
        // A revoke that got the guard first refused the cutoff. A later revoke cannot undo
        // a locally committing generation, and doesn't wait for its data-file operations.
        drop(permission.take());
        let journal = operation(record)?;
        if step == count {
            durable::replace(
                home,
                &pointer,
                journal.after.as_deref().ok_or_else(unsettled)?,
            )
            .map_err(|error| fail_at(&pointer, error))?;
        } else if step < directories {
            let (source, target, id) = journal.directory(home, step)?;
            if Identity::of(&source)? != id {
                return Err(unsettled());
            }
            durable::move_new_directory(&source, &target).map_err(|e| fail_at(&target, e))?;
        } else {
            let i = step - directories;
            let target = journal.file_target(home, i)?;
            let staging = journal.staging(home);
            match (&journal.files[i].before, &journal.files[i].after) {
                (Image::Missing, Image::Missing) => {}
                (_, Image::Missing) => {
                    durable::restore_move(&target, &staging.join(format!("removed-{i}")))
                        .map_err(|error| fail_at(&target, error))?
                }
                _ => durable::restore_move(&staging.join(format!("new-{i}")), &target)
                    .map_err(|error| fail_at(&target, error))?,
            }
        }
        boundary(Phase::Publishing);
    }
    operation(record)?.phase = Phase::Committed;
    write(home, record)?;
    boundary(Phase::Committed);
    Ok(())
}

pub(super) fn recover(home: &Path, record: &mut Record) -> Result<(), String> {
    recover_at(home, record, |_| {})
}

fn recover_at(
    home: &Path,
    record: &mut Record,
    mut before_advance: impl FnMut(usize),
) -> Result<(), String> {
    let bootstrap = record.retained_binding().cloned();
    let journal = operation(record)?;
    let committed = journal.phase == Phase::Committed;
    let staging = journal.staging(home);
    journal.verify_roots(home)?;
    let pointer = home.join(build::FILE);
    match journal.phase {
        Phase::Staging | Phase::Ready => {
            if build::optional_bytes(&pointer)? != journal.before {
                return Err(unsettled());
            }
        }
        Phase::Committed => {
            if build::optional_bytes(&pointer)? != journal.after {
                return Err(unsettled());
            }
            for i in 0..journal.directories.len() {
                let (_, target, id) = journal.directory(home, i)?;
                if Identity::of(&target)? != id {
                    return Err(unsettled());
                }
            }
            for (i, step) in journal.files.iter().enumerate() {
                if Image::at(&journal.file_target(home, i)?)? != step.after {
                    return Err(unsettled());
                }
            }
        }
        Phase::Publishing | Phase::Undoing => {
            journal.phase = Phase::Undoing;
            write(home, record)?;
            while operation(record)?.frontier > 0 {
                let journal = operation(record)?;
                let step = journal.frontier - 1;
                if step == journal.steps() {
                    let live = build::optional_bytes(&pointer)?;
                    let discarded = staging.join("discarded-pointer");
                    if live != journal.before {
                        if live != journal.after {
                            return Err(unsettled());
                        }
                        match &journal.before {
                            Some(bytes) => durable::replace(home, &pointer, bytes),
                            None => durable::restore_move(&pointer, &discarded),
                        }
                        .map_err(|e| fail_at(&pointer, e))?;
                    }
                    durable::sync_restored(&pointer, &discarded, journal.before.is_none())
                        .map_err(|e| fail_at(&pointer, e))?;
                } else if step < journal.directories.len() {
                    let (source, target, id) = journal.directory(home, step)?;
                    match fs::symlink_metadata(&target) {
                        Err(e) if e.kind() == io::ErrorKind::NotFound => {
                            if Identity::of(&source)? != id {
                                return Err(unsettled());
                            }
                            durable::sync_restored(&target, &source, true)
                                .map_err(|e| fail_at(&target, e))?;
                        }
                        Err(e) => return Err(fail_at(&target, e)),
                        Ok(_) => {
                            if Identity::of(&target)? != id
                                || fs::read_dir(&target)
                                    .map_err(|e| fail_at(&target, e))?
                                    .next()
                                    .is_some()
                            {
                                return Err(unsettled());
                            }
                            durable::move_new_directory(&target, &source)
                                .map_err(|e| fail_at(&target, e))?;
                        }
                    }
                } else {
                    let i = step - journal.directories.len();
                    let target = journal.file_target(home, i)?;
                    let live = Image::at(&target)?;
                    let step = &journal.files[i];
                    let backup = staging.join(format!("old-{i}"));
                    if live != step.before {
                        if live != step.after || Image::at(&backup)? != step.before {
                            return Err(unsettled());
                        }
                        if step.before == Image::Missing {
                            durable::restore_move(&target, &staging.join(format!("discarded-{i}")))
                        } else {
                            durable::restore_move(&backup, &target)
                        }
                        .map_err(|e| fail_at(&target, e))?;
                    }
                    durable::sync_restored(&target, &backup, step.before == Image::Missing)
                        .map_err(|e| fail_at(&target, e))?;
                }
                before_advance(step);
                operation(record)?.frontier = step;
                write(home, record)?;
            }
            if build::optional_bytes(&pointer)? != operation(record)?.before {
                return Err(unsettled());
            }
        }
    }
    *record = Record::default();
    if !committed {
        record.intent = Intent::Install {
            bootstrap: bootstrap.map(Box::new),
            staged: None,
        };
    }
    write(home, record)?;
    let _ = fs::remove_dir_all(staging);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::sunrise::txn::{
        build::Bootstrap,
        tests::{crash_process, wait_to_be_killed},
        Target,
    };

    #[test]
    fn live_switch_process() {
        let Some(home) = std::env::var_os("SUNRISE_TEST_INSTALL_OWNER") else {
            return;
        };
        let boundary = std::env::var("SUNRISE_TEST_LIVE_PROFILE_WORKER").unwrap();
        let (kind, boundary) = boundary.split_once('-').unwrap();
        let home = PathBuf::from(home);
        let owner = Lease::admit(&home).unwrap();
        let target = Target::legacy("nightly-original").unwrap();
        let (stage, next) = if kind == "first" {
            let bound = owner.bind_bootstrap(|| Ok(target)).unwrap();
            let (stage, binding) = owner.reserve_first_install(&bound).unwrap();
            (stage, binding.test_active())
        } else if matches!(kind, "unbound" | "pending") {
            if kind == "pending" {
                owner.bind_bootstrap(|| Ok(target)).unwrap();
            }
            let new = Target::legacy("nightly-new").unwrap();
            let (stage, binding) = owner.reserve_first_switch(&new).unwrap();
            let record = read(&home).unwrap().unwrap();
            if let Some(previous) = record.retained_binding() {
                assert_eq!(previous.profile, binding.profile);
                assert_eq!(previous.target.base(), "nightly-original");
            } else {
                assert_eq!(kind, "unbound");
            }
            (stage, binding.test_active())
        } else {
            let prior = Bootstrap::bind(&home, target)
                .unwrap()
                .test_active()
                .test_receipt(1);
            build::publish(&home, &prior).unwrap();
            if kind == "install" {
                (
                    owner.reserve_install_set(&prior).unwrap(),
                    prior.test_switch("nightly-original"),
                )
            } else {
                (
                    owner.reserve_switch(&prior, "nightly-new").unwrap(),
                    prior.test_switch("nightly-new"),
                )
            }
        };
        let package = stage.join("candidate.smod");
        fs::write(&package, b"new mod").unwrap();
        let source = stage.join("candidate");
        fs::write(&source, b"mod.waiting:false\n").unwrap();
        let waiting = PathBuf::from("sunrise-native-mods/keep-exact.smod");
        let Image::Sha256(digest) = Image::at(&home.join(&waiting)).unwrap() else {
            panic!("package fixture missing");
        };
        let next = next.test_waiting(
            "waiting",
            build::WaitingMod {
                file: waiting,
                digest,
                reason: "no fit".into(),
                was_on: true,
            },
        );
        owner
            .seal_staged_set(
                &next.test_receipt(2),
                &[
                    (PathBuf::from("sunrise-mods.txt"), Some(source)),
                    (PathBuf::from("sunrise-mods/new.smod"), Some(package)),
                ],
            )
            .unwrap();
        let mut record = read(&home).unwrap().unwrap();
        let mut steps = 0;
        publish(
            &home,
            &mut record,
            || Ok(()),
            |phase| {
                steps += 1;
                if (phase == Phase::Publishing
                    && ((boundary == "root" && steps == 1)
                        || (boundary == "files" && steps == 3)
                        || (boundary == "pointer" && steps == 4)))
                    || (phase == Phase::Committed && boundary == "marker")
                {
                    wait_to_be_killed();
                }
            },
        )
        .unwrap();
    }

    #[test]
    fn github_publication_worker() {
        use crate::github_mods::{origin_path, tests::prepared_update};
        let Some(home) = std::env::var_os("SUNRISE_TEST_INSTALL_OWNER") else {
            return;
        };
        let home = PathBuf::from(home);
        let owner = Lease::admit(&home).unwrap();
        let prior = Bootstrap::bind(&home, Target::legacy("nightly-original").unwrap())
            .unwrap()
            .test_active();
        build::publish(&home, &prior).unwrap();
        let stage = owner.reserve_install_set(&prior).unwrap();
        let next = prior.test_switch("nightly-original");
        let update = prepared_update("v2", "{\"changed\":true}", "");
        let packed = &update.mods()[0];
        let origin = packed.origin(update.source());
        let sidecar = stage.join("github-origin");
        fs::write(&sidecar, serde_json::to_vec(&origin).unwrap()).unwrap();
        owner
            .seal_staged_set(
                &next,
                &[
                    (
                        PathBuf::from("sunrise-native-mods/zoom.smod"),
                        Some(packed.file.clone()),
                    ),
                    (origin_path(Path::new(""), "zoom"), Some(sidecar)),
                ],
            )
            .unwrap();
        drop(update);
        let mut record = read(&home).unwrap().unwrap();
        publish(
            &home,
            &mut record,
            || Ok(()),
            |phase| {
                if phase == Phase::Publishing {
                    wait_to_be_killed();
                }
            },
        )
        .unwrap();
    }

    #[test]
    fn a_kill_between_github_package_and_origin_restores_both_preimages() {
        use crate::github_mods::{origin_path, tests::prepared_update};
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let release = prepared_update("v1", "{}", "");
        let packed = &release.mods()[0];
        let old = fs::read(&packed.file).unwrap();
        let old_origin = serde_json::to_vec(&packed.origin(release.source())).unwrap();
        let file = home.join("sunrise-native-mods/zoom.smod");
        let origin = origin_path(home, "zoom");
        fs::create_dir(file.parent().unwrap()).unwrap();
        fs::write(&file, &old).unwrap();
        fs::write(&origin, &old_origin).unwrap();
        let new = prepared_update("v2", "{\"changed\":true}", "");
        let expected = fs::read(&new.mods()[0].file).unwrap();
        assert_ne!(old, expected);
        crash_process(
            "browser::sunrise::txn::switch_record::tests::github_publication_worker",
            home,
            None,
            || {
                assert_eq!(
                    fs::read(&file).unwrap(),
                    expected,
                    "replacement already reached disk"
                );
                assert_eq!(
                    fs::read(&origin).unwrap(),
                    old_origin,
                    "origin has not been replaced yet"
                );
            },
        );
        let next = Lease::admit(home).unwrap();
        assert_eq!(next.active_build().unwrap().unwrap().generation(), 1);
        assert_eq!(fs::read(file).unwrap(), old);
        assert_eq!(fs::read(origin).unwrap(), old_origin);
        assert!(!next.outstanding());
    }

    #[test]
    fn revocation_before_cutoff_refuses_and_after_cutoff_cannot_undo_publication() {
        use crate::{backend::Settings, local_json::LockedJson};
        for revoke_first in [true, false] {
            let home = tempfile::tempdir().unwrap();
            let home = home.path();
            let data = home.join("launcher-data");
            let set_grant = |allowed: bool| {
                let file = LockedJson::open(&data, "settings.json").unwrap();
                if allowed {
                    file.write(&serde_json::json!({"native_mods_allowed_at":123,"native_consent_revision":1})).unwrap();
                } else {
                    file.write(&serde_json::json!({"native_consent_revision":2}))
                        .unwrap();
                }
            };
            set_grant(true);
            let grant = Settings::native_grant_from(&data).unwrap().unwrap();
            let owner = Lease::admit(home).unwrap();
            let prior = Bootstrap::bind(home, Target::legacy("nightly-original").unwrap())
                .unwrap()
                .test_active();
            build::publish(home, &prior).unwrap();
            let stage = owner.reserve_switch(&prior, "nightly-new").unwrap();
            let source = stage.join("candidate");
            fs::write(&source, "mod.lantern:true\n").unwrap();
            owner
                .seal_staged_set(
                    &prior.test_switch("nightly-new"),
                    &[(PathBuf::from("sunrise-mods.txt"), Some(source))],
                )
                .unwrap();
            let mut record = read(home).unwrap().unwrap();
            if revoke_first {
                set_grant(false);
            }
            let mut late = false;
            let committed = publish(
                home,
                &mut record,
                || grant.hold_in(&data),
                |phase| {
                    if phase == Phase::Publishing && !late {
                        late = true;
                        let (done, reply) = std::sync::mpsc::channel();
                        let data = data.clone();
                        std::thread::spawn(move || {
                            let file = LockedJson::open(&data, "settings.json").unwrap();
                            file.write(&serde_json::json!({"native_consent_revision":2}))
                                .unwrap();
                            done.send(()).unwrap();
                        });
                        reply
                            .recv_timeout(std::time::Duration::from_secs(10))
                            .expect("permission guard should be released after recording cutoff");
                    }
                },
            );
            assert_eq!(committed.is_err(), revoke_first);
            assert_eq!(late, !revoke_first);
            recover(home, &mut record).unwrap();
            assert_eq!(
                owner.active_build().unwrap().unwrap().base(),
                if revoke_first {
                    "nightly-original"
                } else {
                    "nightly-new"
                }
            );
            assert_eq!(home.join("sunrise-mods.txt").exists(), !revoke_first);
            assert!(Settings::native_grant_from(&data).unwrap().is_none());
        }
    }

    #[test]
    fn live_switch_recovery_process() {
        let Some(home) = std::env::var_os("SUNRISE_TEST_INSTALL_OWNER") else {
            return;
        };
        let home = PathBuf::from(home);
        let goal = std::env::var("SUNRISE_TEST_LIVE_PROFILE_WORKER").unwrap();
        let _owner = Lease::acquire(&home).unwrap();
        let mut record = read(&home).unwrap().unwrap();
        let journal = operation(&mut record).unwrap();
        let step = match goal.as_str() {
            "pointer" => journal.steps(),
            "package" => journal.steps() - 1,
            _ => unreachable!(),
        };
        recover_at(&home, &mut record, |at| {
            if at == step {
                wait_to_be_killed();
            }
        })
        .unwrap();
    }

    #[test]
    fn admission_after_staged_kills_restores_the_old_or_committed_exact_set() {
        for (kind, boundary) in ["switch", "install", "first", "unbound", "pending"]
            .into_iter()
            .flat_map(|kind| ["root", "files", "pointer", "marker"].map(|b| (kind, b)))
        {
            let first_generation = matches!(kind, "first" | "unbound" | "pending");
            let retained = matches!(kind, "first" | "pending");
            let home = tempfile::tempdir().unwrap();
            let home = home.path();
            fs::create_dir(home.join("sunrise-native-mods")).unwrap();
            fs::write(
                home.join("sunrise-native-mods/keep-exact.smod"),
                b"retained bytes",
            )
            .unwrap();
            fs::write(home.join("sunrise-mods.txt"), b"mod.waiting:true\n").unwrap();
            crash_process(
                "browser::sunrise::txn::switch_record::tests::live_switch_process",
                home,
                Some(&format!("{kind}-{boundary}")),
                || {
                    assert!(Lease::admit(home).is_err());
                    assert_eq!(
                        fs::read(home.join("sunrise-mods.txt")).unwrap(),
                        if boundary == "root" {
                            b"mod.waiting:true\n".as_slice()
                        } else {
                            b"mod.waiting:false\n".as_slice()
                        }
                    );
                    assert!(home.join("sunrise-mods").is_dir());
                },
            );
            if boundary == "pointer" {
                // Interrupt undo twice, after the disk move and before the frontier write.
                // A second launcher must accept the restored preimage of the frontier, not
                // require the unpublished generation or the already consumed restore file.
                let actor =
                    "browser::sunrise::txn::switch_record::tests::live_switch_recovery_process";
                crash_process(actor, home, Some("pointer"), || {
                    assert!(Lease::admit(home).is_err());
                    assert_eq!(
                        build::read(home).unwrap().map(|active| active.generation()),
                        (!first_generation).then_some(1)
                    );
                    assert!(home.join("sunrise-mods/new.smod").is_file());
                });
                crash_process(actor, home, Some("package"), || {
                    assert!(Lease::admit(home).is_err());
                    assert!(!home.join("sunrise-mods/new.smod").exists());
                    assert_eq!(
                        fs::read(home.join("sunrise-mods.txt")).unwrap(),
                        b"mod.waiting:false\n"
                    );
                });
            }
            let admission = Lease::admit(home).unwrap();
            let committed = admission.active_build().unwrap();
            assert_eq!(
                fs::read(home.join("sunrise-native-mods/keep-exact.smod")).unwrap(),
                b"retained bytes"
            );
            if boundary == "marker" {
                let committed = committed.unwrap();
                assert_eq!(build::waiting_for_update(home)["waiting"].reason, "no fit");
                assert!(crate::browser::mod_list::set_enabled(home, "waiting", true).is_err());
                assert_eq!(
                    committed.base(),
                    if matches!(kind, "switch" | "unbound" | "pending") {
                        "nightly-new"
                    } else {
                        "nightly-original"
                    }
                );
                assert_eq!(committed.generation(), if first_generation { 1 } else { 2 });
                assert_eq!(
                    fs::read(home.join("sunrise-mods/new.smod")).unwrap(),
                    b"new mod"
                );
                assert_eq!(
                    admission.acknowledge_switch([1; 16]).unwrap(),
                    !first_generation
                );
                drop(admission);
                let reopened = Lease::admit(home).unwrap();
                assert!(!reopened.acknowledge_switch([1; 16]).unwrap());
                let pointer = reopened.active_build().unwrap().unwrap();
                assert_eq!(pointer.receipts().len(), 1);
                assert_eq!(pointer.receipts()[0].id(), [2; 16]);
                assert_eq!(
                    pointer.receipts()[0].effects().waiting_for_update,
                    [("waiting".to_owned(), "no fit".to_owned())]
                );
                assert_eq!(
                    pointer.receipts()[0].effects().removed.clone(),
                    [("mod-2".into(), "reason-2".into())]
                );
                assert_eq!(
                    fs::read(home.join("sunrise-mods.txt")).unwrap(),
                    b"mod.waiting:false\n"
                );
            } else {
                if first_generation {
                    assert!(committed.is_none());
                    if retained {
                        let bound = admission
                            .play_target(|| panic!("retained prior binding"))
                            .unwrap();
                        assert_eq!(bound.base(), "nightly-original");
                        assert!(read(home).unwrap().unwrap().staged().is_none());
                    }
                } else {
                    let committed = committed.unwrap();
                    assert_eq!(committed.base(), "nightly-original");
                    assert_eq!(committed.generation(), 1);
                    assert_eq!(
                        committed
                            .receipts()
                            .iter()
                            .map(super::super::Receipt::id)
                            .collect::<Vec<_>>(),
                        [[1; 16]]
                    );
                }
                assert!(build::waiting_for_update(home).is_empty());
                assert!(!home.join("sunrise-mods").exists());
                assert_eq!(
                    fs::read(home.join("sunrise-mods.txt")).unwrap(),
                    b"mod.waiting:true\n"
                );
            }
            assert_eq!(
                read(home).unwrap().is_none(),
                !retained || boundary == "marker"
            );
        }
    }
}
