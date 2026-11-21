//! A profile's exact local publication: stage, fence, move data, move the pointer, commit.
//! This layer doesn't select a build, inspect the network or decide what mods a player wants.
//! Existing launcher routes are not cut over to this record until their replacements can all
//! observe the same admission and committed generation.
pub(crate) mod durable;
mod progress;

use cap_fs_ext::MetadataExt;
use cap_std::{ambient_authority, fs::Dir};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Component, Path, PathBuf},
};

const RECORD: &str = "mod-browser/install-txn.json";
const RETIRED: &str = "mod-browser/install-txn.retired.json";
const POINTER: &str = "mod-browser/active-build.json";
const STAGE_PREFIX: &str = ".install-txn-";
const VERSION: u64 = 2;
const BOOTSTRAP_VERSION: u64 = 1;

/// Moving a managed profile can change its path but never the filesystem identity of a
/// directory referenced by an in-flight transaction.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    volume: u64,
    inode: u64,
}

impl Identity {
    fn of(path: &Path) -> Result<Self, String> {
        if !fs::symlink_metadata(path).is_ok_and(|m| regular(&m) && m.is_dir()) {
            return Err(at(path, unsettled()));
        }
        let meta = Dir::open_ambient_dir(path, ambient_authority())
            .and_then(|dir| dir.dir_metadata())
            .map_err(|e| at(path, e))?;
        Ok(Self {
            volume: meta.dev(),
            inode: meta.ino(),
        })
    }

    fn file_at(path: &Path) -> Result<Option<Self>, String> {
        match fs::symlink_metadata(path) {
            Ok(meta) if regular(&meta) && meta.is_file() => (),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            _ => return Err(at(path, unsettled())),
        }
        let file = fs::File::open(path).map_err(|e| at(path, e))?;
        let meta = cap_std::fs::File::from_std(file)
            .metadata()
            .map_err(|e| at(path, e))?;
        Ok(Some(Self {
            volume: meta.dev(),
            inode: meta.ino(),
        }))
    }
}

fn regular(meta: &fs::Metadata) -> bool {
    !crate::native_cache::linked(meta)
}

fn unsettled() -> String {
    crate::i18n::tr(
        "Sunrise couldn't settle the profile's install transaction. The profile can't be used yet. Any recovery data was kept.",
    )
    .into()
}

fn at(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}

/// An admitted profile. The owner file must be stable and never removed, including across
/// detach/recreate; managed names pass a path outside the directory they can move. Its inherited
/// file description outlives a launcher killed while a game child still holds it.
pub struct Admission {
    home: PathBuf,
    identity: Identity,
    file: fs::File,
    #[cfg(unix)]
    handed_off: std::sync::atomic::AtomicBool,
}

#[cfg(unix)]
impl Drop for Admission {
    fn drop(&mut self) {
        if !*self.handed_off.get_mut() {
            // An unrelated concurrent fork may temporarily hold a copy until exec.
            // It was never given admission. Only an intentional handoff needs close-only.
            let _ = self.file.unlock();
        }
    }
}
impl Admission {
    /// Recovery can retire or restore references. Managed callers fence cache deletion
    /// around admission; no cache guard is held by this lease while a worker runs.
    pub fn open(home: &Path, external_owner: Option<&Path>) -> Result<Self, String> {
        Self::open_fenced(home, external_owner, || Ok(()))
    }

    /// Acquire the caller's reference guards after the profile, never the reverse.
    pub(crate) fn open_fenced<G>(
        home: &Path,
        external_owner: Option<&Path>,
        fence: impl FnOnce() -> Result<G, String>,
    ) -> Result<Self, String> {
        let lease = Self::claim(home, external_owner)?;
        let _fence = fence()?;
        lease.recover()?;
        Ok(lease)
    }

    fn claim(home: &Path, external_owner: Option<&Path>) -> Result<Self, String> {
        let gate = external_owner
            .map(Path::to_path_buf)
            .unwrap_or_else(|| home.join(".install-txn.lock"));
        let identity = Identity::of(home)?;
        match fs::symlink_metadata(&gate) {
            Ok(meta) if regular(&meta) && meta.is_file() => (),
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            _ => return Err(at(&gate, unsettled())),
        }
        let busy = || {
            crate::i18n::tr(
                "This profile is in use. Close the game or wait for the other operation to finish.",
            )
            .to_owned()
        };
        let mut open = fs::File::options();
        open.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            open.share_mode(0); // The Windows share exclusion follows inherited handles.
        }
        let file = match open.open(&gate) {
            Ok(file) => file,
            #[cfg(windows)]
            Err(error) if matches!(error.raw_os_error(), Some(32 | 33)) => return Err(busy()),
            Err(error) => return Err(at(&gate, error)),
        };
        #[cfg(unix)]
        match file.try_lock() {
            Ok(()) => (),
            Err(fs::TryLockError::WouldBlock) => return Err(busy()),
            Err(fs::TryLockError::Error(e)) => return Err(at(&gate, e)),
        }
        if Identity::of(home)? != identity {
            return Err(unsettled());
        }
        Ok(Self {
            home: home.into(),
            identity,
            file,
            #[cfg(unix)]
            handed_off: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// Give a game child the gate of this already-admitted profile. The game spawn
    /// must carry its cache claims and this profile gate in the same handoff.
    pub fn handoff(&self) -> crate::process::leases::ProfileHandoff<'_> {
        crate::process::leases::ProfileHandoff {
            file: &self.file,
            #[cfg(unix)]
            handed_off: &self.handed_off,
        }
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Reserve the candidate artifacts before exposing them. Hold the reference guard
    /// while creating the reservation, not while compiling the private candidate.
    pub fn reserve(
        &mut self,
        intent: Intent,
        targets: BTreeSet<String>,
    ) -> Result<Staging<'_>, String> {
        if read(&self.home)?.is_some() {
            return Err(unsettled());
        }
        self.stage(Record::new(self.identity, intent, targets, None))
    }

    /// Resolve first Play once. A pending binding, with its exact inputs and proof, wins
    /// over a subsequent change to the release feed or profile defaults.
    pub fn bind_bootstrap<G>(
        &mut self,
        choose: impl FnOnce() -> Result<Bootstrap, String>,
        authorize: impl FnOnce(&Bootstrap) -> Result<G, String>,
    ) -> Result<Bootstrap, String> {
        if let Some(record) = read(&self.home)? {
            return record.pending_bootstrap().cloned();
        }
        if pointer_bytes(&self.home)?.is_some() {
            return Err(unsettled());
        }
        let binding = choose()?;
        // Acquire cache publication exclusion only after resolving the candidate. The
        // caller rechecks that exact candidate under its guard before it becomes a reference.
        let _reference_guard = authorize(&binding)?;
        let record = Record::new(
            self.identity,
            Intent::Install,
            binding.targets(),
            Some(binding.clone()),
        );
        record.verify_home(&self.home)?;
        write(&self.home, &record)?;
        Ok(binding)
    }

    /// Persist the accepted archive proof before consuming archive bytes, without rebinding.
    /// Existing proofs are returned verbatim; the caller must decode and verify their schema.
    pub fn prove_bootstrap(
        &mut self,
        prove: impl FnOnce(&Bootstrap) -> Result<serde_json::Value, String>,
    ) -> Result<Bootstrap, String> {
        let mut record = read(&self.home)?.ok_or_else(unsettled)?;
        let binding = record.pending_bootstrap()?.clone();
        if binding.proof.is_some() {
            return Ok(binding);
        }
        let proof = prove(&binding)?;
        let binding = Bootstrap {
            proof: Some(proof),
            ..binding
        };
        record.bootstrap = Some(binding.clone());
        write(&self.home, &record)?;
        Ok(binding)
    }

    /// Stage an attempt on the same binding. Reopening after a failed prepare or publish
    /// cannot silently select a different tag, variant, profile identity, inputs or proof.
    pub fn resume_bootstrap(&mut self) -> Result<Staging<'_>, String> {
        let record = read(&self.home)?.ok_or_else(unsettled)?;
        record.pending_bootstrap()?;
        record.clean_binding_stage(&self.home)?;
        self.stage(record)
    }

    fn stage(&mut self, mut record: Record) -> Result<Staging<'_>, String> {
        clean_retired(&self.home)?;
        let root = record_dir(&self.home)?;
        fs::create_dir_all(&root).map_err(|e| at(&root, e))?;
        let mut builder = tempfile::Builder::new();
        builder.prefix(STAGE_PREFIX).rand_bytes(24);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(fs::Permissions::from_mode(0o700));
        }
        let stage = builder.tempdir_in(&root).map_err(|e| at(&root, e))?;
        let pointer = self.home.join(POINTER);
        record.operation = Some(Operation {
            stage: Stage {
                id: Identity::of(stage.path())?,
                name: stage
                    .path()
                    .file_name()
                    .and_then(std::ffi::OsStr::to_str)
                    .ok_or_else(unsettled)?
                    .to_owned(),
            },
            before: Image::at(&pointer)?,
            previous_pointer: pointer_bytes(&self.home)?,
            ready: None,
        });
        record.retirement = None;
        record.verify_home(&self.home)?;
        if let Some(binding) = record.undo_binding()? {
            let saved = stage.path().join("binding-after-undo");
            let bytes = serde_json::to_vec(&binding).map_err(|e| at(&saved, e))?;
            durable::replace(stage.path(), &saved, &bytes).map_err(|e| at(&saved, e))?;
        }
        // A reported sync failure may still leave the record visible. Keep its directory.
        let path = stage.keep();
        write(&self.home, &record)?;
        Ok(Staging {
            lease: self,
            path,
            record,
        })
    }

    fn recover(&self) -> Result<(), String> {
        if let Some(record) = read(&self.home)? {
            record.complete(&self.home, |_| {}, || {})?;
        } else {
            // No mutation is in flight. A retained cleanup record cannot block Play.
            let _ = clean_retired(&self.home);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Intent {
    Install,
    Switch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
enum Image {
    Missing,
    File {
        sha256: [u8; 32],
        access: durable::Access,
    },
}

impl Image {
    fn at(path: &Path) -> Result<Self, String> {
        let meta = match fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::Missing),
            Err(e) => return Err(at(path, e)),
        };
        if !meta.is_file() || !regular(&meta) {
            return Err(at(path, unsettled()));
        }
        let mut file = fs::File::open(path).map_err(|e| at(path, e))?;
        let access = durable::Access::of(path, &file).map_err(|e| at(path, e))?;
        let mut hash = Sha256::new();
        io::copy(&mut file, &mut hash).map_err(|e| at(path, e))?;
        Ok(Self::File {
            sha256: hash.finalize().into(),
            access,
        })
    }

    fn digest(self) -> Option<[u8; 32]> {
        match self {
            Self::Missing => None,
            Self::File { sha256, .. } => Some(sha256),
        }
    }
}

/// First Play's durable binding, stored in the same journal as its pointer publish.
/// Selection and proof are versioned 0c payloads; the journal owns the stable tags and
/// frozen identities, while the artifact boundary validates those concrete payloads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bootstrap {
    version: u64,
    pub base: String,
    pub image: String,
    pub profile: [u8; 16],
    pub inputs: [u8; 32],
    pub selection: serde_json::Value,
    pub proof: Option<serde_json::Value>,
}

impl Bootstrap {
    pub fn new(
        base: String,
        image: String,
        profile: [u8; 16],
        inputs: [u8; 32],
        selection: serde_json::Value,
    ) -> Self {
        Self {
            version: BOOTSTRAP_VERSION,
            base,
            image,
            profile,
            inputs,
            selection,
            proof: None,
        }
    }

    fn targets(&self) -> BTreeSet<String> {
        BTreeSet::from([self.base.clone(), self.image.clone()])
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u64,
    home: Identity,
    intent: Intent,
    targets: BTreeSet<String>,
    bootstrap: Option<Bootstrap>,
    operation: Option<Operation>,
    retirement: Option<Stage>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stage {
    id: Identity,
    name: String,
}

impl Stage {
    fn path(&self, home: &Path) -> PathBuf {
        home.join("mod-browser").join(&self.name)
    }

    fn validate(&self, owner: Identity) -> Result<(), String> {
        if owner.volume == self.id.volume
            && self.name.strip_prefix(STAGE_PREFIX).is_some_and(|tail| {
                !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_alphanumeric())
            })
        {
            Ok(())
        } else {
            Err(unsettled())
        }
    }

    fn verify(&self, home: &Path) -> Result<(), String> {
        if Identity::of(&self.path(home))? == self.id {
            Ok(())
        } else {
            Err(unsettled())
        }
    }

    fn clean(&self, home: &Path) -> Result<(), String> {
        let path = self.path(home);
        if !absent(&path)? {
            self.verify(home)?;
            fs::remove_dir_all(&path).map_err(|e| at(&path, e))?;
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Operation {
    stage: Stage,
    before: Image,
    previous_pointer: Option<Vec<u8>>,
    ready: Option<Ready>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ready {
    directories: BTreeMap<PathBuf, Identity>,
    files: Vec<Step>,
    next_pointer: Vec<u8>,
    pointer_after: Image,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    name: PathBuf,
    parent: Identity,
    before: Image,
    after: Image,
}

impl Ready {
    fn steps(&self) -> usize {
        self.directories.len() + self.files.len() + 1
    }

    fn directory(
        &self,
        home: &Path,
        stage: &Path,
        index: usize,
    ) -> Result<(PathBuf, PathBuf, Identity), String> {
        let (name, id) = self.directories.iter().nth(index).ok_or_else(unsettled)?;
        Ok((stage.join(name), home.join(name), *id))
    }

    fn validate(&self, op: &Operation) -> Result<(), String> {
        let mut names = BTreeSet::new();
        if self
            .directories
            .iter()
            .any(|(name, id)| !mod_root(name) || id.volume != op.stage.id.volume)
            || self.files.iter().any(|step| {
                !valid_destination(&step.name)
                    || !names.insert((step.parent, destination_key(&step.name)))
                    || step.parent.volume != op.stage.id.volume
            })
            || self.next_pointer.is_empty()
            || self.pointer_after.digest() != Some(Sha256::digest(&self.next_pointer).into())
        {
            return Err(unsettled());
        }
        Ok(())
    }
}

fn record_dir(home: &Path) -> Result<PathBuf, String> {
    let root = home.join("mod-browser");
    match fs::symlink_metadata(&root) {
        Ok(meta) if meta.is_dir() && regular(&meta) => Ok(root),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(root),
        _ => Err(at(&root, unsettled())),
    }
}

fn read_at(home: &Path, name: &str) -> Result<Option<Record>, String> {
    record_dir(home)?;
    let path = home.join(name);
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_file() && regular(&meta) => (),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        _ => return Err(at(&path, unsettled())),
    }
    let bytes = fs::read(&path).map_err(|e| at(&path, e))?;
    let record: Record = serde_json::from_slice(&bytes).map_err(|e| at(&path, e))?;
    if record.version != VERSION {
        return Err(at(&path, unsettled()));
    }
    record.verify_home(home)?;
    Ok(Some(record))
}

fn read(home: &Path) -> Result<Option<Record>, String> {
    let record = read_at(home, RECORD)?;
    if let Some(operation) = record.as_ref().and_then(|record| record.operation.as_ref()) {
        operation.stage.verify(home)?;
    }
    Ok(record)
}

fn clean_retired(home: &Path) -> Result<(), String> {
    if let Some(record) = read_at(home, RETIRED)? {
        let marker = home.join(RETIRED);
        durable::establish_retirement(home, &marker).map_err(|e| at(&marker, e))?;
        remove_retired(home, &marker, &record.operation()?.stage)?;
    }
    Ok(())
}

fn remove_retired(home: &Path, marker: &Path, stage: &Stage) -> Result<(), String> {
    stage.clean(home)?;
    durable::retire(marker).map_err(|e| at(marker, e))
}

/// Read reference evidence under the caller's cache-reference guard, without acquiring
/// profile admission. Journal shape, staging identity and pointer file kinds are checked by
/// the same parser used for recovery. 0c parses the exact pointer bytes as its `Active` type.
pub struct References {
    pub targets: BTreeSet<String>,
    pub before: Option<Vec<u8>>,
    pub after: Option<Vec<u8>>,
    pub pointer: Option<Vec<u8>>,
}

pub fn references(home: &Path) -> Result<References, String> {
    let active = read(home)?;
    // A retirement rename can be visible before its durability barrier. Conservatively
    // keep these references until admission can enforce the barrier and remove the record.
    let retired = read_at(home, RETIRED)?;
    if active.is_some() && retired.is_some() {
        return Err(unsettled());
    }
    let mut refs = References {
        targets: BTreeSet::new(),
        before: None,
        after: None,
        pointer: pointer_bytes(home)?,
    };
    if let Some(record) = active.or(retired) {
        refs.targets = record.targets;
        if let Some(operation) = record.operation {
            refs.before = operation.previous_pointer;
            refs.after = operation.ready.map(|ready| ready.next_pointer);
            if refs.pointer != refs.before && (refs.after.is_none() || refs.pointer != refs.after) {
                return Err(unsettled());
            }
        } else if refs.pointer.is_some() {
            return Err(unsettled());
        }
    }
    Ok(refs)
}

fn pointer_bytes(home: &Path) -> Result<Option<Vec<u8>>, String> {
    record_dir(home)?;
    let file = home.join(POINTER);
    match fs::symlink_metadata(&file) {
        Ok(meta) if meta.is_file() && regular(&meta) => {
            fs::read(&file).map(Some).map_err(|e| at(&file, e))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        _ => Err(at(&file, unsettled())),
    }
}

fn write(home: &Path, record: &Record) -> Result<(), String> {
    let path = home.join(RECORD);
    let bytes = serde_json::to_vec(record).map_err(|e| at(&path, e))?;
    durable::replace(home, &path, &bytes).map_err(|e| at(&path, e))
}

fn snapshot(source: &Path, copy: &Path, image: Image) -> Result<(), String> {
    if let Image::File { access, .. } = image {
        let mut src = fs::File::open(source).map_err(|e| at(source, e))?;
        let mut dst = fs::File::options()
            .create_new(true)
            .write(true)
            .open(copy)
            .map_err(|e| at(copy, e))?;
        io::copy(&mut src, &mut dst)
            .and_then(|_| access.apply(&dst))
            .and_then(|_| durable::flush(&dst))
            .map_err(|e| at(copy, e))?;
        drop(dst);
        if Image::at(copy)? != image {
            return Err(at(copy, unsettled()));
        }
        durable::sync_restored(copy, copy, false).map_err(|e| at(copy, e))?;
    }
    Ok(())
}

pub struct Staging<'a> {
    lease: &'a mut Admission,
    path: PathBuf,
    record: Record,
}

impl<'a> Staging<'a> {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Borrow this stage's existing admission for a preparation or native worker. The
    /// worker may outlive the launcher, but only the launcher may seal proved output.
    pub fn handoff(&self) -> crate::process::leases::ProfileHandoff<'_> {
        self.lease.handoff()
    }

    pub(crate) fn seal_pointer_replacing(
        self,
        previous: &[u8],
        next: Vec<u8>,
    ) -> Result<Sealed<'a>, String> {
        if self.record.operation()?.previous_pointer.as_deref() != Some(previous) {
            return Err(unsettled());
        }
        self.seal(next, &[])
    }

    /// Bind exact preimages and candidates under the caller's reference guard, while only
    /// private files exist. A `None` source removes a file. Accept only launcher-managed names.
    pub fn seal(
        mut self,
        pointer_bytes: Vec<u8>,
        files: &[(PathBuf, Option<PathBuf>)],
    ) -> Result<Sealed<'a>, String> {
        let home = self.lease.home();
        let stage = &self.path;
        self.record.verify_home(home)?;
        self.record.operation()?.stage.verify(home)?;
        let pointer = home.join(POINTER);
        let before = self.record.operation()?.before;
        if Image::at(&pointer)? != before {
            return Err(at(&pointer, unsettled()));
        }
        let older = stage.join("pointer-old");
        snapshot(&pointer, &older, before)?;
        let newer = stage.join("pointer-new");
        durable::replace(stage, &newer, &pointer_bytes).map_err(|e| at(&newer, e))?;
        let mut plan = Ready {
            directories: BTreeMap::new(),
            files: Vec::new(),
            next_pointer: pointer_bytes,
            pointer_after: Image::at(&newer)?,
        };
        let mut entries = BTreeSet::new();
        for (i, (name, source)) in files.iter().enumerate() {
            if !valid_destination(name) {
                return Err(unsettled());
            }
            let live = home.join(name);
            let parent_name = name.parent().ok_or_else(unsettled)?;
            let parent_path = home.join(parent_name);
            let parent = match fs::symlink_metadata(&parent_path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound && mod_root(parent_name) => {
                    if let Some(&id) = plan.directories.get(parent_name) {
                        id
                    } else {
                        let dir = stage.join(parent_name);
                        fs::create_dir(&dir).map_err(|e| at(&dir, e))?;
                        let id = Identity::of(&dir)?;
                        plan.directories.insert(parent_name.into(), id);
                        id
                    }
                }
                _ => Identity::of(&parent_path)?,
            };
            unique_entry(&live, &mut entries)?;
            let before = Image::at(&live)?;
            snapshot(&live, &stage.join(format!("old-{i}")), before)?;
            let after = if let Some(source) = source {
                let relative = source
                    .strip_prefix(stage)
                    .map_err(|_| at(source, unsettled()))?;
                let mut inside = stage.clone();
                let mut components = relative.components().peekable();
                while let Some(part) = components.next() {
                    let Component::Normal(name) = part else {
                        return Err(at(source, unsettled()));
                    };
                    inside.push(name);
                    if components.peek().is_some() && Identity::of(&inside)?.volume != parent.volume
                    {
                        return Err(at(source, unsettled()));
                    }
                }
                let image = Image::at(source)?;
                if image == Image::Missing {
                    return Err(at(source, unsettled()));
                }
                snapshot(source, &stage.join(format!("new-{i}")), image)?;
                image
            } else {
                Image::Missing
            };
            plan.files.push(Step {
                name: name.clone(),
                parent,
                before,
                after,
            });
        }
        plan.validate(self.record.operation()?)?;
        self.record.operation.as_mut().ok_or_else(unsettled)?.ready = Some(plan);
        progress::Slots::create(stage, self.record.digest()?).map_err(|e| at(stage, e))?;
        write(home, &self.record)?;
        Ok(Sealed(self))
    }
}

pub struct Sealed<'a>(Staging<'a>);

impl Sealed<'_> {
    /// Final external checks can return a short consent guard, held until the publishing
    /// frontier is recorded. A caller's cache/reference guard must span this whole call.
    /// No network calls or approvals occur after cutoff.
    pub fn publish<G>(self, cutoff: impl FnOnce() -> Result<G, String>) -> Result<(), String> {
        self.publish_observed(cutoff, |_, _| Ok(()))
    }

    pub(crate) fn publish_observed<G>(
        self,
        cutoff: impl FnOnce() -> Result<G, String>,
        boundary: impl FnMut(usize, Boundary) -> Result<(), String>,
    ) -> Result<(), String> {
        let home = self.0.lease.home();
        let record = read(home)?.ok_or_else(unsettled)?;
        if record.digest()? != self.0.record.digest()? {
            return Err(unsettled());
        }
        let result = record.publish_at(home, cutoff, boundary);
        if record.complete(home, |_| {}, || {})? {
            Ok(())
        } else {
            result
        }
    }
}

#[cfg(test)]
pub(crate) fn tear_next_commit_marker() {
    progress::fail_marker(progress::MarkerFault::TornWrite);
}

fn mod_root(path: &Path) -> bool {
    use sunrise_launcher_smod::mod_list::{DIR, NATIVE_DIR};
    path.as_os_str() == DIR || path.as_os_str() == NATIVE_DIR
}

fn valid_destination(path: &Path) -> bool {
    let mut parts = path.components();
    let (Some(Component::Normal(first)), second, None) = (parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    use sunrise_launcher_smod::mod_list::{DIR, FILE, NATIVE_DIR};
    match second {
        None => first == FILE || first == crate::browser::installed::MANIFEST,
        Some(Component::Normal(file)) if first == "mod-browser" => {
            matches!(file.to_str(), Some("pack.json" | "pack.spack"))
        }
        Some(Component::Normal(file)) if first == DIR || first == NATIVE_DIR => {
            use sunrise_launcher_smod::index::{browser_state::portable_ascii_component, valid_id};
            let Some(name) = file.to_str() else {
                return false;
            };
            valid_smod_leaf(name)
                || (first == NATIVE_DIR
                    && portable_ascii_component(name)
                    && name.strip_suffix(".origin.json").is_some_and(valid_id))
        }
        _ => false,
    }
}

fn valid_smod_leaf(name: &str) -> bool {
    // Before either leaf exists, neither native identity nor a Unicode fold
    // can rule out filesystem-specific aliases.
    sunrise_launcher_smod::index::browser_state::portable_ascii_component(name)
        && name.ends_with(".smod")
        && !name.starts_with('.')
        && name.len() <= 200
        && !name.contains('\x7f')
}

/// A receipt claims the same portable file a later transaction could replace.
pub(crate) fn native_package_key(path: &str) -> Option<String> {
    let (root, name) = path.split_once('/')?;
    (root == sunrise_launcher_smod::mod_list::NATIVE_DIR && valid_smod_leaf(name))
        .then(|| destination_key(Path::new(path)))
}

fn destination_key(path: &Path) -> String {
    // All admitted leaves are portable ASCII. Keep the same key even on a
    // case-sensitive volume so moving a profile cannot introduce an alias.
    path.file_name()
        .and_then(std::ffi::OsStr::to_str)
        .expect("validated destination")
        .to_ascii_lowercase()
}

fn unique_entry(path: &Path, entries: &mut BTreeSet<Identity>) -> Result<(), String> {
    // Existing files also need a filesystem identity check. Two distinct spellings can
    // address the same entry through names the portable key doesn't know (or hard links).
    if Identity::file_at(path)?.is_some_and(|identity| !entries.insert(identity)) {
        return Err(at(path, unsettled()));
    }
    Ok(())
}

fn target(home: &Path, file: &Step) -> Result<PathBuf, String> {
    let path = home.join(&file.name);
    if Identity::of(path.parent().ok_or_else(unsettled)?)? != file.parent {
        return Err(at(&path, unsettled()));
    }
    Ok(path)
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) enum Boundary {
    BeforeRename,
    Renamed,
    Durable,
    BeforeMarker,
    WrittenMarker,
    DurableMarker,
}

impl Record {
    fn new(
        home: Identity,
        intent: Intent,
        targets: BTreeSet<String>,
        bootstrap: Option<Bootstrap>,
    ) -> Self {
        Self {
            version: VERSION,
            home,
            intent,
            targets,
            bootstrap,
            operation: None,
            retirement: None,
        }
    }

    fn operation(&self) -> Result<&Operation, String> {
        self.operation.as_ref().ok_or_else(unsettled)
    }

    fn pending_bootstrap(&self) -> Result<&Bootstrap, String> {
        if self.operation.is_some() {
            return Err(unsettled());
        }
        self.bootstrap.as_ref().ok_or_else(unsettled)
    }

    fn undo_binding(&self) -> Result<Option<Self>, String> {
        self.bootstrap
            .as_ref()
            .map(|bootstrap| {
                let mut pending = Self::new(
                    self.home,
                    Intent::Install,
                    self.targets.clone(),
                    Some(bootstrap.clone()),
                );
                pending.retirement = Some(self.operation()?.stage.clone());
                Ok(pending)
            })
            .transpose()
    }

    fn preallocated_binding(stage: &Path, binding: &Self) -> Result<PathBuf, String> {
        let saved = stage.join("binding-after-undo");
        if Image::at(&saved)?.digest() != Some(binding.digest()?) {
            return Err(at(&saved, unsettled()));
        }
        Ok(saved)
    }

    fn clean_binding_stage(&self, home: &Path) -> Result<(), String> {
        if let Some(stage) = &self.retirement {
            let record = home.join(RECORD);
            durable::establish_retirement(home, &record).map_err(|e| at(&record, e))?;
            stage.clean(home)?;
        }
        Ok(())
    }

    fn verify_home(&self, home: &Path) -> Result<(), String> {
        if self.home != Identity::of(home)?
            || (self.bootstrap.is_none() && self.operation.is_none())
            || (self.operation.is_some() && self.retirement.is_some())
            || self.targets.iter().any(|tag| {
                !tag.as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                    || tag.ends_with('.')
                    || !tag
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.+@".contains(&b))
            })
        {
            return Err(unsettled());
        }
        if let Some(binding) = &self.bootstrap {
            if !matches!(self.intent, Intent::Install)
                || binding.version != BOOTSTRAP_VERSION
                || binding.targets() != self.targets
                || self
                    .operation
                    .as_ref()
                    .is_some_and(|op| op.before != Image::Missing)
            {
                return Err(unsettled());
            }
        }
        if let Some(stage) = &self.retirement {
            stage.validate(self.home)?;
        }
        if let Some(op) = &self.operation {
            op.stage.validate(self.home)?;
            let before = op
                .previous_pointer
                .as_ref()
                .map(|bytes| Sha256::digest(bytes).into());
            if before != op.before.digest() {
                return Err(unsettled());
            }
            if let Some(plan) = &op.ready {
                plan.validate(op)?;
            }
        }
        Ok(())
    }

    fn digest(&self) -> Result<[u8; 32], String> {
        Ok(Sha256::digest(serde_json::to_vec(self).map_err(|e| e.to_string())?).into())
    }

    fn publish_at<G>(
        &self,
        home: &Path,
        cutoff: impl FnOnce() -> Result<G, String>,
        mut boundary: impl FnMut(usize, Boundary) -> Result<(), String>,
    ) -> Result<(), String> {
        self.verify_home(home)?;
        let op = self.operation()?;
        op.stage.verify(home)?;
        let stage = op.stage.path(home);
        let plan = op.ready.as_ref().ok_or_else(unsettled)?;
        if let Some(binding) = self.undo_binding()? {
            Self::preallocated_binding(&stage, &binding)?;
        }
        let pointer = home.join(POINTER);
        let next = plan.pointer_after;
        if Image::at(&pointer)? != op.before
            || Image::at(&stage.join("pointer-old"))? != op.before
            || Image::at(&stage.join("pointer-new"))? != next
        {
            return Err(at(&pointer, unsettled()));
        }
        let mut progress = progress::Slots::open(&stage, self.digest()?, plan.steps())?;
        if progress.current().phase != progress::Phase::Ready {
            return Err(unsettled());
        }
        let directories = plan.directories.len();
        for i in 0..directories {
            let (src, dst, id) = plan.directory(home, &stage, i)?;
            if !absent(&dst)? || Identity::of(&src)? != id {
                return Err(at(&dst, unsettled()));
            }
        }
        let mut entries = BTreeSet::new();
        for (i, file) in plan.files.iter().enumerate() {
            let name = if plan
                .directories
                .contains_key(file.name.parent().ok_or_else(unsettled)?)
            {
                if file.before != Image::Missing {
                    return Err(unsettled());
                }
                home.join(&file.name)
            } else {
                target(home, file)?
            };
            unique_entry(&name, &mut entries)?;
            if Image::at(&name)? != file.before
                || Image::at(&stage.join(format!("old-{i}")))? != file.before
                || Image::at(&stage.join(format!("new-{i}")))? != file.after
            {
                return Err(at(&name, unsettled()));
            }
        }
        let mut permission = Some(cutoff()?);
        for i in 0..plan.steps() {
            progress.advance(progress::Phase::Publishing, i + 1)?;
            drop(permission.take());
            boundary(i, Boundary::BeforeRename)?;
            let mut fault = Ok(());
            let after_rename = || {
                fault = boundary(i, Boundary::Renamed);
            };
            if i == plan.steps() - 1 {
                durable::restore_move_at(&stage.join("pointer-new"), &pointer, after_rename)
                    .map_err(|e| at(&pointer, e))?;
            } else if i < directories {
                let (src, dst, id) = plan.directory(home, &stage, i)?;
                if Identity::of(&src)? != id {
                    return Err(at(&src, unsettled()));
                }
                durable::move_new_directory_at(&src, &dst, after_rename)
                    .map_err(|e| at(&dst, e))?;
            } else {
                let slot = i - directories;
                let file = &plan.files[slot];
                let dst = target(home, file)?;
                match (file.before, file.after) {
                    (Image::Missing, Image::Missing) => (),
                    (_, Image::Missing) => durable::restore_move_at(
                        &dst,
                        &stage.join(format!("removed-{slot}")),
                        after_rename,
                    )
                    .map_err(|e| at(&dst, e))?,
                    _ => durable::restore_move_at(
                        &stage.join(format!("new-{slot}")),
                        &dst,
                        after_rename,
                    )
                    .map_err(|e| at(&dst, e))?,
                }
            }
            fault?;
            boundary(i, Boundary::Durable)?;
        }
        boundary(plan.steps(), Boundary::BeforeMarker)?;
        progress.advance_at(progress::Phase::Committed, plan.steps(), || {
            boundary(plan.steps(), Boundary::WrittenMarker)
        })?;
        boundary(plan.steps(), Boundary::DurableMarker)
    }

    fn complete(
        &self,
        home: &Path,
        mut restored: impl FnMut(usize),
        retired: impl FnOnce(),
    ) -> Result<bool, String> {
        use progress::{Phase, Slots};
        self.verify_home(home)?;
        let pointer = home.join(POINTER);
        let Some(op) = &self.operation else {
            if !absent(&pointer)? {
                return Err(unsettled());
            }
            self.clean_binding_stage(home)?;
            return Ok(false);
        };
        op.stage.verify(home)?;
        let stage = op.stage.path(home);
        let mut committed = false;
        if let Some(plan) = &op.ready {
            let next = plan.pointer_after;
            let mut slots = Slots::open(&stage, self.digest()?, plan.steps())?;
            match slots.current().phase {
                Phase::Ready => {
                    if Image::at(&pointer)? != op.before {
                        return Err(at(&pointer, unsettled()));
                    }
                }
                Phase::Committed => {
                    slots.sync()?; // The last marker can be visible after a reported sync failure.
                    committed = true;
                    if Image::at(&pointer)? != next {
                        return Err(at(&pointer, unsettled()));
                    }
                    for i in 0..plan.directories.len() {
                        let (_, dst, id) = plan.directory(home, &stage, i)?;
                        if Identity::of(&dst)? != id {
                            return Err(at(&dst, unsettled()));
                        }
                    }
                    for file in &plan.files {
                        let dst = target(home, file)?;
                        if Image::at(&dst)? != file.after {
                            return Err(at(&dst, unsettled()));
                        }
                    }
                }
                Phase::Publishing | Phase::Undoing => {
                    if slots.current().phase == Phase::Publishing {
                        slots.advance(Phase::Undoing, slots.current().frontier)?;
                    }
                    let directories = plan.directories.len();
                    while slots.current().frontier > 0 {
                        let step = slots.current().frontier - 1;
                        if step == plan.steps() - 1 {
                            restore(
                                &pointer,
                                &stage.join("pointer-old"),
                                &stage.join("pointer-discarded"),
                                op.before,
                                next,
                            )?;
                        } else if step < directories {
                            let (src, dst, id) = plan.directory(home, &stage, step)?;
                            restore_directory(&src, &dst, id)?;
                        } else {
                            let index = step - directories;
                            let spec = &plan.files[index];
                            let dst = target(home, spec)?;
                            restore(
                                &dst,
                                &stage.join(format!("old-{index}")),
                                &stage.join(format!("discarded-{index}")),
                                spec.before,
                                spec.after,
                            )?;
                        }
                        restored(step);
                        slots.advance(Phase::Undoing, step)?;
                    }
                    if Image::at(&pointer)? != op.before {
                        return Err(at(&pointer, unsettled()));
                    }
                }
            }
        } else if Image::at(&pointer)? != op.before {
            return Err(at(&pointer, unsettled()));
        }
        let file = home.join(RECORD);
        if !committed {
            if let Some(binding) = self.undo_binding()? {
                let saved = Self::preallocated_binding(&stage, &binding)?;
                durable::restore_move(&saved, &file).map_err(|e| at(&saved, e))?;
                retired();
                let _ = binding.clean_binding_stage(home);
                return Ok(false);
            }
        }
        let retired_file = home.join(RETIRED);
        if !absent(&retired_file)? {
            return Err(at(&retired_file, unsettled()));
        }
        durable::mark_retired(home, &file, &retired_file).map_err(|e| at(&file, e))?;
        retired();
        // This writer already established the retirement barrier. If cleanup fails, leave
        // proof for the next admission (which checks the barrier again before removing it).
        let _ = remove_retired(home, &retired_file, &op.stage);
        Ok(committed)
    }
}

/// Prove the directory is back on the private volume before advancing the undo frontier.
fn restore_directory(src: &Path, dst: &Path, id: Identity) -> Result<(), String> {
    if !absent(dst)? {
        if Identity::of(dst)? != id || fs::read_dir(dst).map_err(|e| at(dst, e))?.next().is_some() {
            return Err(at(dst, unsettled()));
        }
        return durable::move_new_directory(dst, src).map_err(|e| at(dst, e));
    }
    #[cfg(not(windows))]
    {
        if Identity::of(src)? != id {
            return Err(at(src, unsettled()));
        }
        durable::sync_restored(dst, src, true).map_err(|e| at(dst, e))
    }
    #[cfg(windows)]
    {
        // A visible name alone cannot stand in for the same-volume write-through move.
        // Retry in the private stage, never temporarily returning the directory to live.
        // If killed between moves, the same identity can be at exactly one of these names.
        let replay = src.with_extension("undo-directory");
        if !absent(src)? {
            if Identity::of(src)? != id {
                return Err(at(src, unsettled()));
            }
            durable::move_new_directory(src, &replay).map_err(|e| at(src, e))?;
        } else if Identity::of(&replay)? != id {
            return Err(at(&replay, unsettled()));
        }
        durable::move_new_directory(&replay, src).map_err(|e| at(src, e))
    }
}

fn restore(
    dst: &Path,
    backup: &Path,
    discarded: &Path,
    before: Image,
    after: Image,
) -> Result<(), String> {
    let live = Image::at(dst)?;
    if live != before {
        if live != after || Image::at(backup)? != before {
            return Err(at(dst, unsettled()));
        }
        if before == Image::Missing {
            durable::restore_move(dst, discarded)
        } else {
            durable::restore_move(backup, dst)
        }
        .map_err(|e| at(dst, e))?;
    }
    durable::sync_restored(dst, backup, before == Image::Missing).map_err(|e| at(dst, e))
}

fn absent(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(at(path, e)),
        Ok(_) => Ok(false),
    }
}

#[cfg(test)]
mod tests;
