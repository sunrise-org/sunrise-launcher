//! The published Play generation and the binding retained while admitting an old profile.
use super::{artifact::Artifact, durable, resources::Resources, unsettled, Image, Lease};
use crate::{backend, native_mods, native_status};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub(super) const FILE: &str = "mod-browser/active-build.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    base: String,
    preferred_image: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) download: Option<backend::GameDownload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    installed: Option<Artifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preferred: Option<Artifact>,
}

impl Target {
    /// A legacy profile may already point at a derived image. Preserve that exact choice.
    pub fn legacy(tag: &str) -> Result<Self, String> {
        validate_tag(tag)?;
        let base = if native_mods::is_native(tag) {
            native_mods::about(tag).ok_or_else(unsettled)?.base
        } else {
            tag.to_owned()
        };
        validate_tag(&base)?;
        Ok(Self {
            base,
            preferred_image: tag.into(),
            download: None,
            installed: None,
            preferred: None,
        })
    }

    /// Bind an installed base's bytes, or the actual archive and architecture this front end
    /// resolved, while the profile is still uninitialized. Later Settings changes are irrelevant.
    pub fn first_play(
        tag: &str,
        release: impl FnOnce(&str) -> Result<Option<backend::Release>, String>,
    ) -> Result<Self, String> {
        let mut target = Self::legacy(tag)?;
        if backend::installed().contains(&target.base) {
            target.installed = Some(Artifact::capture(target.base())?);
            if target.base != target.preferred_image {
                target.preferred = Some(Artifact::capture(target.preferred_image())?);
            }
        } else {
            let release = release(target.base())?
                .filter(|release| release.tag == target.base)
                .ok_or_else(|| {
                    crate::i18n::tr(
                        "No downloadable build matches this profile. Choose a game build.",
                    )
                    .to_string()
                })?;
            target.download = Some(backend::GameDownload::select(&release)?);
        }
        Ok(target)
    }

    /// Both the headless Play path and the first managed Install resolve through the
    /// same pin/channel rules as Home. Call this only while holding profile admission.
    pub fn for_profile(instance: &crate::instances::Instance) -> Result<Self, String> {
        let installed = backend::installed();
        let selected = backend::Settings::load()
            .selected
            .filter(|tag| installed.contains(tag));
        let rows = if instance.version.is_none() {
            backend::releases()
        } else {
            Ok(Vec::new())
        };
        let tag = crate::instances::profile_target(
            instance.version.as_ref(),
            instance.channel,
            rows.as_deref().unwrap_or(&[]),
            &installed,
            selected.as_ref(),
        )
        .cloned()
        .ok_or_else(|| {
            rows.as_ref()
                .err()
                .cloned()
                .unwrap_or_else(backend::no_version_installed)
        })?;
        Self::first_play(&tag, |base| {
            let rows = if instance.version.is_some() {
                backend::releases()?
            } else {
                rows?
            };
            Ok(rows.into_iter().find(|release| release.tag == base))
        })
    }

    pub(super) fn check_installed(&self) -> Result<(), String> {
        for bound in self.installed.iter().chain(self.preferred.iter()) {
            if *bound != Artifact::capture(&bound.tag)? {
                return Err(unsettled());
            }
        }
        Ok(())
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn preferred_image(&self) -> &str {
        &self.preferred_image
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        validate_tag(&self.base)?;
        validate_tag(&self.preferred_image)?;
        if let Some(download) = &self.download {
            if self.installed.is_some() || self.preferred_image != self.base {
                return Err(unsettled());
            }
            download.validate(&self.base)?;
        }
        if self
            .installed
            .as_ref()
            .is_some_and(|image| image.tag != self.base)
        {
            return Err(unsettled());
        }
        if self.preferred.as_ref().is_some_and(|image| {
            self.installed.is_none() || image.tag != self.preferred_image || image.tag == self.base
        }) {
            return Err(unsettled());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Bootstrap {
    pub(super) target: Target,
    pub(super) profile: [u8; 16],
    inputs: [u8; 32],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) proof: Option<backend::GameProof>,
}

impl Bootstrap {
    pub(super) fn bind(home: &Path, target: Target) -> Result<Self, String> {
        let mut profile = [0; 16];
        getrandom::fill(&mut profile).map_err(|e| e.to_string())?;
        Ok(Self {
            target,
            profile,
            inputs: inputs(home)?,
            proof: None,
        })
    }

    pub(super) fn verify(&self, home: &Path) -> Result<(), String> {
        if inputs(home)? != self.inputs {
            return Err(crate::i18n::tr("The profile changed while its first build was being prepared. Nothing was switched.").into());
        }
        Ok(())
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        self.target.validate()?;
        if self
            .proof
            .as_ref()
            .is_some_and(|proof| self.target.download.is_none() || !proof.validate())
        {
            return Err(unsettled());
        }
        Ok(())
    }

    pub(super) fn verify_artifact(&self) -> Result<(), String> {
        self.target.check_installed()?;
        if let Some(download) = &self.target.download {
            if !self
                .proof
                .as_ref()
                .is_some_and(|proof| download.is_installed(self.target.base(), proof))
            {
                return Err(unsettled());
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn test_active(&self) -> Active {
        let base = Artifact::test(self.target.base.clone());
        Active {
            version: 1,
            profile: self.profile,
            generation: 1,
            executable: base.clone(),
            base,
            resources: Resources::test(),
            inputs: self.inputs,
            receipts: Vec::new(),
            waiting: Default::default(),
        }
    }

    pub(super) fn active(&self, image: &str, home: &Path) -> Result<Active, String> {
        self.verify(home)?;
        self.first_generation(image, home, self.inputs)
    }

    /// The frozen candidate has its own inputs; the caller's admission separately checks
    /// the saved legacy inputs before selecting or publishing this first generation.
    pub(super) fn first_generation(
        &self,
        image: &str,
        home: &Path,
        inputs: [u8; 32],
    ) -> Result<Active, String> {
        self.verify_artifact()?;
        Active::capture(
            self.profile,
            1,
            (&self.target.base, image),
            inputs,
            Vec::new(),
            home,
            None,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Active {
    version: u32,
    pub(super) profile: [u8; 16],
    pub(super) generation: u64,
    base: Artifact,
    executable: Artifact,
    resources: Resources,
    pub(super) inputs: [u8; 32],
    receipts: Vec<Receipt>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    waiting: std::collections::BTreeMap<String, WaitingMod>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaitingMod {
    pub(super) file: PathBuf,
    pub(super) digest: [u8; 32],
    pub reason: String,
    pub(in crate::browser::sunrise) was_on: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwitchEffects {
    pub waiting_for_update: Vec<(String, String)>,
    pub removed: Vec<(String, String)>,
}

impl SwitchEffects {
    pub fn is_empty(&self) -> bool {
        self.waiting_for_update.is_empty() && self.removed.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    id: [u8; 16],
    effects: SwitchEffects,
}

impl Receipt {
    pub fn id(&self) -> [u8; 16] {
        self.id
    }
    pub fn effects(&self) -> &SwitchEffects {
        &self.effects
    }
}

impl Active {
    pub fn receipts(&self) -> &[Receipt] {
        &self.receipts
    }
    fn capture(
        profile: [u8; 16],
        generation: u64,
        (base, image): (&str, &str),
        inputs: [u8; 32],
        receipts: Vec<Receipt>,
        home: &Path,
        existing: Option<&Resources>,
    ) -> Result<Self, String> {
        verify_included(base, image, home)?;
        if self::inputs(home)? != inputs {
            return Err(unsettled());
        }
        let resources = match existing {
            Some(resources) => {
                resources.verify(base)?;
                resources.clone()
            }
            None => Resources::capture(base)?,
        };
        let base = Artifact::capture(base)?;
        let executable = if image == base.tag {
            base.clone()
        } else {
            Artifact::capture(image)?
        };
        Ok(Self {
            version: 1,
            profile,
            generation,
            base,
            executable,
            resources,
            inputs,
            receipts,
            waiting: Default::default(),
        })
    }

    pub(super) fn adopt(&self, image: &str, inputs: [u8; 32], home: &Path) -> Result<Self, String> {
        let generation = self.generation.checked_add(1).ok_or_else(unsettled)?;
        let mut next = Self::capture(
            self.profile,
            generation,
            (self.base(), image),
            inputs,
            self.receipts.clone(),
            home,
            Some(&self.resources),
        )?;
        if next.base != self.base {
            return Err(unsettled());
        }
        for (id, item) in &self.waiting {
            if item.matches(home)? {
                next.waiting.insert(id.clone(), item.clone());
            }
        }
        next.require_waiting_off(home)?;
        Ok(next)
    }

    pub(super) fn switched(
        &self,
        base: &str,
        executable: &str,
        staged: &Path,
        effects: SwitchEffects,
        waiting: std::collections::BTreeMap<String, WaitingMod>,
    ) -> Result<Self, String> {
        Self::capture(
            self.profile,
            self.generation.checked_add(1).ok_or_else(unsettled)?,
            (base, executable),
            inputs(staged)?,
            self.receipts.clone(),
            staged,
            None,
        )?
        .with_switch_effects(staged, effects, waiting)
    }

    pub(super) fn with_switch_effects(
        mut self,
        staged: &Path,
        effects: SwitchEffects,
        waiting: std::collections::BTreeMap<String, WaitingMod>,
    ) -> Result<Self, String> {
        if !effects.is_empty() {
            let mut id = [0; 16];
            getrandom::fill(&mut id).map_err(|e| e.to_string())?;
            self.receipts.push(Receipt { id, effects });
        }
        self.waiting = waiting;
        self.require_waiting_off(staged)?;
        Ok(self)
    }

    pub(super) fn require_waiting_off(&self, home: &Path) -> Result<(), String> {
        for (id, item) in &self.waiting {
            if crate::browser::mod_list::is_enabled(home, id) && item.matches(home)? {
                return Err(crate::i18n::trf(
                    "{mod} needs an update from its author before it can be turned on.",
                    &[("mod", id)],
                ));
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn test_receipt(mut self, sequence: u8) -> Self {
        self.receipts.push(Receipt {
            id: [sequence; 16],
            effects: SwitchEffects {
                waiting_for_update: self
                    .waiting
                    .keys()
                    .map(|id| (id.clone(), "no fit".into()))
                    .collect(),
                removed: vec![(format!("mod-{sequence}"), format!("reason-{sequence}"))],
            },
        });
        self
    }

    #[cfg(test)]
    pub(super) fn test_waiting(mut self, id: &str, item: WaitingMod) -> Self {
        self.waiting.insert(id.to_owned(), item);
        self
    }

    #[cfg(test)]
    pub(super) fn test_switch(&self, tag: &str) -> Self {
        let mut active = self.clone();
        active.base.tag = tag.to_owned();
        active.executable.tag = tag.to_owned();
        active.generation += 1;
        active
    }

    pub fn base(&self) -> &str {
        &self.base.tag
    }

    pub(super) fn same_base_artifact(&self, other: &Self) -> bool {
        self.base == other.base
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn executable(&self) -> &str {
        &self.executable.tag
    }

    pub(crate) fn resources(&self) -> PathBuf {
        self.resources.path(self.base())
    }

    pub(super) fn verify_resources(&self) -> Result<(), String> {
        self.resources.verify(self.base())
    }

    pub(crate) fn allow_accelerator(&self) -> bool {
        self.resources.allow_accelerator(self.base())
    }

    pub(in crate::browser::sunrise) fn verify_program(&self, path: &Path) -> Result<(), String> {
        let root = backend::version_dir(self.executable());
        if path != root.join(backend::CLIENT) {
            #[cfg(target_os = "macos")]
            crate::game_app::verify(&root, path).map_err(|e| format!("{}: {e}", path.display()))?;
            #[cfg(not(target_os = "macos"))]
            return Err(unsettled());
        }
        self.executable.verify_program(path)
    }

    #[cfg(windows)]
    pub(crate) fn verify_runtime_copy(&self, root: &Path) -> Result<(), String> {
        if Artifact::capture_at(self.executable(), root)? != self.executable {
            return Err(unsettled());
        }
        Ok(())
    }

    pub fn verify_artifacts(&self) -> Result<(), String> {
        if Artifact::capture(self.base())? != self.base
            || (self.executable != self.base
                && Artifact::capture(self.executable())? != self.executable)
        {
            return Err(crate::i18n::tr(
                "The profile's saved game build does not match the files on disk.",
            )
            .into());
        }
        Ok(())
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        if self.version != 1 || self.generation == 0 || self.profile == [0; 16] {
            return Err(unsettled());
        }
        let mut receipts = std::collections::HashSet::new();
        if self.receipts.iter().any(|receipt| {
            receipt.id == [0; 16] || !receipts.insert(receipt.id) || receipt.effects.is_empty()
        }) {
            return Err(unsettled());
        }
        for (id, mod_) in &self.waiting {
            if !sunrise_launcher_smod::index::valid_id(id)
                || !super::switch_record::valid_package_destination(&mod_.file)
            {
                return Err(unsettled());
            }
        }
        validate_tag(self.base())?;
        validate_tag(self.executable())
    }
}

pub(super) fn verify_included(base: &str, image: &str, home: &Path) -> Result<(), String> {
    if image != base
        && native_mods::about(image)
            .is_none_or(|about| about.base != base || !about.left_out.is_empty())
    {
        return Err(unsettled());
    }
    let included = native_status::included(image)?;
    native_status::check_compiled_profile(&included, home)
}

pub(super) fn verify_packages(home: &Path) -> Result<(), String> {
    scan_packages(home).map(drop)
}

pub(in crate::browser::sunrise) fn scan_packages(
    home: &Path,
) -> Result<Vec<(sunrise_launcher_smod::manifest::Manifest, PathBuf)>, String> {
    use sunrise_launcher_smod::{manifest, mod_list};
    let mut packages = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for directory in [mod_list::DIR, mod_list::NATIVE_DIR] {
        let root = home.join(directory);
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("{}: {e}", root.display())),
        };
        for entry in entries {
            let path = entry
                .map_err(|e| format!("{}: {e}", root.display()))?
                .path();
            if path.extension().is_none_or(|ext| ext != "smod") {
                continue;
            }
            let package =
                manifest::Package::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if (directory == mod_list::NATIVE_DIR) != package.manifest.native.is_some()
                || !ids.insert(package.manifest.id.clone())
            {
                return Err(unsettled());
            }
            packages.push((package.manifest, path));
        }
    }
    Ok(packages)
}

pub(super) fn validate_tag(tag: &str) -> Result<(), String> {
    if tag.is_empty()
        || !tag.as_bytes()[0].is_ascii_alphanumeric()
        || tag.ends_with('.')
        || !tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.+@".contains(&b))
    {
        return Err(unsettled());
    }
    Ok(())
}

pub(super) fn read(home: &Path) -> Result<Option<Active>, String> {
    let path = home.join(FILE);
    let Some(bytes) = optional_bytes(&path)? else {
        return Ok(None);
    };
    parse(&bytes)
        .map(Some)
        .map_err(|e| format!("{}: {e}", path.display()))
}

pub(super) fn parse(bytes: &[u8]) -> Result<Active, String> {
    let active: Active = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    active.validate()?;
    Ok(active)
}

pub(super) fn publish(home: &Path, active: &Active) -> Result<(), String> {
    let path = home.join(FILE);
    let bytes = serde_json::to_vec(active).map_err(|e| e.to_string())?;
    durable::replace(home, &path, &bytes).map_err(|e| format!("{}: {e}", path.display()))
}

// An advisory label for lists only. Launches and mutations must use an admitted lease.
pub fn displayed_image(home: &Path) -> Option<String> {
    read(home)
        .ok()
        .flatten()
        .map(|active| active.executable.tag)
}

impl WaitingMod {
    #[cfg(test)]
    pub(in crate::browser::sunrise) fn test_package(
        home: &Path,
        file: PathBuf,
        was_on: bool,
    ) -> Self {
        let Image::Sha256(digest) = Image::at(&home.join(&file)).unwrap() else {
            panic!("expected a waiting package file");
        };
        Self {
            file,
            digest,
            reason: "no fit".into(),
            was_on,
        }
    }

    pub(super) fn matches(&self, home: &Path) -> Result<bool, String> {
        Ok(Image::at(&home.join(&self.file))? == Image::Sha256(self.digest))
    }

    /// The newest candidate the registry can offer that is not the exact disabled bytes.
    /// The install still has to verify its dependencies and, for native code, build it.
    pub fn replacement<'a>(
        &self,
        index: &sunrise_launcher_smod::index::Index,
        project: &'a sunrise_launcher_smod::index::Project,
        game: &str,
    ) -> Option<&'a sunrise_launcher_smod::index::Version> {
        project.versions.iter().find(|v| {
            crate::browser::sunrise::fits_game(v, game)
                && !self.same_package(v)
                && index.revoked(&project.id, &v.file.sha256).is_none()
        })
    }

    pub(in crate::browser::sunrise) fn same_package(
        &self,
        version: &sunrise_launcher_smod::index::Version,
    ) -> bool {
        version.file.sha256.eq_ignore_ascii_case(&format!(
            "{:x}",
            sha2::digest::Output::<Sha256>::from(self.digest)
        ))
    }
}

pub(in crate::browser::sunrise) fn waiting_packages(
    home: &Path,
) -> Result<std::collections::BTreeMap<String, WaitingMod>, String> {
    let Some(active) = read(home)? else {
        return Ok(Default::default());
    };
    let mut waiting = std::collections::BTreeMap::new();
    for (id, item) in active.waiting {
        if item.matches(home)? {
            waiting.insert(id, item);
        }
    }
    Ok(waiting)
}

/// Installed display only. Admission reads and validates the same file independently.
pub fn waiting_for_update(home: &Path) -> std::collections::BTreeMap<String, WaitingMod> {
    waiting_packages(home).unwrap_or_default()
}

pub fn active_stamp(home: &Path) -> Option<(u64, Option<std::time::SystemTime>)> {
    let meta = fs::metadata(home.join(FILE)).ok()?;
    Some((meta.len(), meta.modified().ok()))
}

pub(crate) fn require_current_update(home: &Path, ids: &[&str]) -> Result<(), String> {
    let Some(active) = read(home)? else {
        return Ok(());
    };
    for id in ids {
        if let Some(item) = active.waiting.get(*id) {
            if item.matches(home)? {
                return Err(crate::i18n::trf(
                    "{mod} needs an update from its author before it can be turned on.",
                    &[("mod", id)],
                ));
            }
        }
    }
    Ok(())
}

impl Lease {
    /// A delayed acknowledgement owns no authority over any other receipt or generation.
    pub fn acknowledge_switch(&self, id: [u8; 16]) -> Result<bool, String> {
        let _lock = super::locked(self.home())?;
        if super::read(self.home())?.is_some() {
            return Err(unsettled());
        }
        let Some(mut active) = self.active_build()? else {
            return Ok(false);
        };
        let before = active.receipts.len();
        active.receipts.retain(|receipt| receipt.id != id);
        if active.receipts.len() == before {
            return Ok(false);
        }
        let _publication =
            crate::run_leases::installation().map_err(crate::native_mods::cache_error)?;
        publish(self.home(), &active)?;
        Ok(true)
    }

    pub fn active_build(&self) -> Result<Option<Active>, String> {
        read(self.home())
    }

    /// Copy only the admitted generation, not pending work or another profile's receipts.
    pub(crate) fn stage_copy(&self, destination: &Path) -> Result<Option<Active>, String> {
        if super::read(self.home())?.is_some() {
            return Err(unsettled());
        }
        let Some(mut active) = self.active_build()? else {
            return Ok(None);
        };
        active.verify_artifacts()?;
        active.verify_resources()?;
        if active.inputs != inputs(self.home())? || active.inputs != inputs(destination)? {
            return Err(unsettled());
        }
        active.require_waiting_off(destination)?;
        verify_included(active.base(), active.executable(), destination)?;
        getrandom::fill(&mut active.profile).map_err(|e| e.to_string())?;
        active.generation = 1;
        active.receipts.clear();
        active.validate()?;
        publish(destination, &active)?;
        Ok(Some(active))
    }
}

pub(super) fn optional_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let fail = |e| format!("{}: {e}", path.display());
    match fs::symlink_metadata(path) {
        Ok(meta) if regular(&meta) && meta.is_file() => fs::read(path).map(Some).map_err(fail),
        Ok(_) => Err(fail(io::Error::from(io::ErrorKind::InvalidData))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(fail(e)),
    }
}

pub(super) fn regular(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            == 0
    }
    #[cfg(not(windows))]
    !meta.file_type().is_symlink()
}

pub(super) fn require_dir(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.is_dir() && regular(&meta) {
        Ok(())
    } else {
        Err(format!(
            "{}: {}",
            path.display(),
            io::ErrorKind::InvalidData
        ))
    }
}

/// Take the identity from the actual bytes, not a display/cache manifest. Order and file
/// boundaries are explicit; a rebuild cannot substitute a changed package at the same path.
pub(super) fn inputs(home: &Path) -> Result<[u8; 32], String> {
    fn item(hash: &mut Sha256, root: &Path, rel: &str) -> Result<(), String> {
        hash.update((rel.len() as u64).to_le_bytes());
        hash.update(rel.as_bytes());
        let path = root.join(rel);
        let io_error = |e| format!("{}: {e}", path.display());
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) if regular(&meta) => meta,
            Ok(_) => return Err(io_error(io::Error::from(io::ErrorKind::InvalidData))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                hash.update(b"-");
                return Ok(());
            }
            Err(e) => return Err(io_error(e)),
        };
        if meta.is_dir() {
            hash.update(b"[");
            let mut names: Vec<_> = fs::read_dir(&path)
                .map_err(io_error)?
                .map(|e| e.map(|e| e.file_name()))
                .collect::<Result<_, _>>()
                .map_err(io_error)?;
            names.sort_unstable();
            for name in names {
                let name = name.to_str().ok_or_else(unsettled)?;
                item(hash, root, &format!("{rel}/{name}"))?;
            }
            hash.update(b"]");
        } else if meta.is_file() {
            let Image::Sha256(digest) = Image::at(&path)? else {
                return Err(unsettled());
            };
            hash.update(b"f");
            hash.update(digest);
        } else {
            return Err(io_error(io::Error::from(io::ErrorKind::InvalidData)));
        }
        Ok(())
    }

    use sunrise_launcher_smod::mod_list::{DIR, FILE, NATIVE_DIR};
    let mut hash = Sha256::new();
    for rel in [DIR, NATIVE_DIR, FILE, crate::browser::installed::MANIFEST] {
        item(&mut hash, home, rel)?;
    }
    Ok(hash.finalize().into())
}
