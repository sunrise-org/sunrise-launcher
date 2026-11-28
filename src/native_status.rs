//! Whether an instance plays its native mods (docs/modding/src/how-to/native-mods.md): the mods turned on in its
//! folder against the ones its version was built with (`native-mods.json`, written by
//! `native_mods::build`). The Mods page shows the answer beside its Build button.
use crate::{backend, browser::mod_list, native_mods};
use std::fs;
use std::path::Path;
use std::sync::Arc;

/// A native mod: its id, its version and a digest of its files (None for a build from before
/// `native-mods.json` kept them).
pub type Mod = (String, String, Option<String>);

/// Whether an installed file (`installed::Entry::file`) is a native mod.
pub fn is_native_file(file: &str) -> bool {
    file.strip_prefix(native_mods::INSTALLED)
        .is_some_and(|rest| rest.starts_with('/') && rest.ends_with(".smod"))
}

/// Where an instance's native mods stand.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    /// No native mod is turned on: nothing to build.
    NoMods,
    /// The version it plays isn't a native build.
    NotBuilt,
    /// The version it plays is a native build, made with other mods than those turned on.
    Outdated,
    /// The version it plays was built with exactly these mods.
    UpToDate,
}

/// The state for the mods turned on, `enabled`, and those of the version played, `built` (None
/// when it is no native build). Order doesn't matter; a mod at another version, or with other
/// files at the same version (a reinstalled local `.smod`), needs a build.
pub fn state(enabled: &[Mod], built: Option<&[Mod]>) -> State {
    if enabled.is_empty() {
        return State::NoMods;
    }
    let sorted = |mods: &[Mod]| {
        let mut v = mods.to_vec();
        v.sort();
        v.dedup();
        v
    };
    let same = |a: &Mod, b: &Mod| {
        a.0 == b.0 && a.1 == b.1 && (a.2.is_none() || b.2.is_none() || a.2 == b.2)
    };
    match built {
        None => State::NotBuilt,
        Some(built) => {
            let (built, enabled) = (sorted(built), sorted(enabled));
            if built.len() == enabled.len() && built.iter().zip(&enabled).all(|(a, b)| same(a, b)) {
                State::UpToDate
            } else {
                State::Outdated
            }
        }
    }
}

/// A `.smod`'s digest as a build records it: its files without the uses table, which the
/// mod tool packs beside the source but the build's own digest of the copied source never holds
/// (else every mod packed with a uses table read Needs rebuild after each build).
fn archive_digest<'a>(files: impl Iterator<Item = (&'a str, &'a [u8])>) -> String {
    use sunrise_launcher_smod::manifest::USES_FILE;
    native_mods::digest(files.filter(|(name, _)| *name != USES_FILE))
}

/// The mods turned on that the played build doesn't have as they are now: each one's own "Needs
/// build" on the Mods tab, so changing one mod never marks the others. Every one when the
/// version played is no native build.
pub fn unbuilt(enabled: &[Mod], built: Option<&[Mod]>) -> Vec<String> {
    let same = |a: &Mod, b: &Mod| {
        a.0 == b.0 && a.1 == b.1 && (a.2.is_none() || b.2.is_none() || a.2 == b.2)
    };
    enabled
        .iter()
        .filter(|m| !built.is_some_and(|built| built.iter().any(|b| same(m, b))))
        .map(|m| m.0.clone())
        .collect()
}

/// Enabled native packages that Play would have to compile: not in the profile's active
/// executable and not already satisfied by a proved, admitted installed build (`reuse::lookup`).
pub fn pending(home: &Path, active: Option<&crate::managed_play::Active>) -> Vec<String> {
    pending_for(home, active, native_mods::Profile::default(), None)
}

/// Like [`pending`], using the profile's chosen native build `mode` and an optional fallback
/// `base` when the profile has no active pointer yet.
pub fn pending_for(
    home: &Path,
    active: Option<&crate::managed_play::Active>,
    mode: native_mods::Profile,
    base: Option<&str>,
) -> Vec<String> {
    let compiled = active.and_then(|active| built(active.executable()));
    let missing = unbuilt(&enabled(home), compiled.as_deref());
    if missing.is_empty() {
        return missing;
    }
    let base = active.map(crate::managed_play::Active::base).or(base);
    if base.is_some_and(|base| native_mods::exact_client_reusable(base, home, mode)) {
        return Vec::new();
    }
    missing
}

/// What a native build was made from, as the Mods page and the instance list show it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Build {
    /// The release or Nightly it was built on (`native_mods::About::base`).
    pub base: String,
    /// The Sunrise commit it compiled.
    pub sunrise: String,
    /// The mods whose pin made it compile `sunrise` rather than `base`'s own commit.
    pub pinned_by: Vec<String>,
    /// It compiled another commit than its Nightly's, with no record of why: built before the
    /// launcher kept which mods pinned it ([`pinning`] reads them from the instance's mods).
    pub pinned_unknown: bool,
    /// The mods turned on that it was built without, and why.
    pub left_out: Vec<native_mods::LeftOut>,
}

impl Build {
    /// Installed native build `version`'s, from its `native-mods.json`; None for another version.
    pub fn read(version: &str) -> Option<Build> {
        Some(Build::of(native_mods::about(version)?))
    }

    /// What build record `about` says.
    fn of(about: native_mods::About) -> Build {
        // The fork's manifest pins the exact Sunrise commit, so a build is never on a commit
        // its mods did not name: nothing is unknown and no pin list is recorded.
        Build {
            pinned_unknown: false,
            base: about.base,
            sunrise: about.sunrise,
            pinned_by: Vec::new(),
            left_out: about.left_out,
        }
    }

    /// The Sunrise it was built on, as a player reads it (`sunrise_label`).
    pub fn sunrise(&self) -> String {
        sunrise_label(&self.base)
    }

    /// The same, short enough for the instance list: `Nightly 10-28`.
    pub fn sunrise_short(&self) -> String {
        match crate::nightly::date_of(&self.base) {
            Some(date) => format!("Nightly {}", &date[5..]),
            None => crate::nightly::short_commit(&self.base),
        }
    }
}

/// A release or Nightly as a player reads it beside "Sunrise": `v0.2.93` as it is,
/// `nightly-2026-10-29-4baa571` as `Nightly 2026-10-29`.
pub fn sunrise_label(version: &str) -> String {
    match crate::nightly::date_of(version) {
        Some(date) => format!("Nightly {date}"),
        None => crate::nightly::short_commit(version),
    }
}

/// The native mods turned on in game folder `home`, read from their `.smod`s. One that can't be
/// read is left out: the on/off list is keyed by the manifest id, which an unreadable file can't
/// give, so it is neither on nor a build input (the game doesn't load it either). Its file name
/// is no id: it could be a working mod's, and turning the file off would turn that mod off.
pub fn enabled(home: &Path) -> Vec<Mod> {
    enabled_files(home).into_iter().map(|(_, m)| m).collect()
}

/// [`enabled`], each with the file it was read from: the files a build takes in.
fn enabled_files(home: &Path) -> Vec<(std::path::PathBuf, Mod)> {
    let mut out = Vec::new();
    for path in hmods(home) {
        let Ok(b) = sunrise_launcher_smod::index::native::Bundle::read(&path) else {
            continue;
        };
        if mod_list::is_enabled(home, &b.manifest.id) {
            let digest = archive_digest(b.files.iter().map(|(n, d)| (n.as_str(), d.as_slice())));
            out.push((path, (b.manifest.id, b.manifest.version, Some(digest))));
        }
    }
    out
}

/// The native mods turned on in game folder `home` that pin Sunrise commit `revision`.
pub fn pinning(home: &Path, revision: &str) -> Vec<String> {
    bundles_on(home)
        .into_iter()
        .filter(|b| b.manifest.native.as_ref().expect("native bundle").sunrise == revision)
        .map(|b| b.manifest.id)
        .collect()
}

/// The readable native mods turned on in game folder `home`.
pub fn bundles_on(home: &Path) -> Vec<sunrise_launcher_smod::index::native::Bundle> {
    hmods(home)
        .into_iter()
        .filter_map(|path| sunrise_launcher_smod::index::native::Bundle::read(&path).ok())
        .filter(|b| mod_list::is_enabled(home, &b.manifest.id))
        .collect()
}

/// A build row's identity and name, taken from the actual enabled archive even when the
/// Mods page hasn't loaded a snapshot or the registry isn't reachable.
pub struct BuildMod {
    pub id: String,
    pub title: String,
    pub version: String,
}

pub fn build_mods(home: &Path) -> Vec<BuildMod> {
    bundles_on(home)
        .into_iter()
        .map(|bundle| BuildMod {
            id: bundle.manifest.id,
            title: bundle.manifest.name,
            version: bundle.manifest.version,
        })
        .collect()
}

/// Paths declared by a code package, in manifest order.
pub fn declared_internals(manifest: &sunrise_launcher_smod::Manifest) -> Vec<String> {
    manifest
        .internals
        .iter()
        .map(|entry| entry.path.clone())
        .collect()
}

/// A native file not tracked by the registry/browser, including GitHub installs.
#[derive(Clone)]
pub struct Installed {
    pub file: String,
    pub id: String,
    /// The name its manifest gives it; its ID when the manifest can't be read.
    pub name: String,
    pub authors: Vec<String>,
    pub version: String,
    pub internals: Vec<String>,
    pub origin: Option<crate::github_mods::Origin>,
    /// The native mod that serves this content package (`served_by`), when it names one.
    pub served_by: Option<String>,
    /// The validated package icon, or the package error (the file remains removable).
    pub icon: Result<Option<Arc<sunrise_launcher_smod::icon::Icon>>, String>,
}

/// Discover the actual files, not just GitHub sidecars. Registry files already have a row.
pub fn untracked(home: &Path) -> Vec<Installed> {
    untracked_in(home, native_mods::INSTALLED, hmods(home), true)
}

/// The packages put in `sunrise-mods/` by hand: content-only mods,
/// which the registry browser has no record of. One that can't be read is not a mod the game
/// loads and is left out, as is one whose ID the registry's mods or the native files have.
pub fn untracked_content(home: &Path) -> Vec<Installed> {
    let tracked = crate::browser::installed::Installed::load(home);
    let mut taken: std::collections::HashSet<String> = tracked
        .entries
        .iter()
        .filter_map(|e| crate::browser::sunrise::mod_id(e).map(str::to_owned))
        .chain(hmods(home).iter().map(|path| stem(path)))
        .collect();
    let dir = sunrise_launcher_smod::mod_list::DIR;
    let mut found = untracked_in(home, dir, content_files(home), false);
    found.retain(|package| package.icon.is_ok() && taken.insert(package.id.clone()));
    found
}

fn untracked_in(
    home: &Path,
    dir: &str,
    paths: Vec<std::path::PathBuf>,
    native: bool,
) -> Vec<Installed> {
    let tracked = crate::browser::installed::Installed::load(home);
    paths
        .into_iter()
        .filter_map(|path| {
            let file = format!("{dir}/{}", path.file_name()?.to_str()?);
            if !path.is_file() || tracked.entries.iter().any(|e| e.file == file) {
                return None;
            }
            let mut package = sunrise_launcher_smod::manifest::Package::read(&path);
            // The build and the enabled list read a native file as a bundle; a file that is not
            // one is no mod there, so the page must not show it as one either.
            if native {
                if let (Ok(_), Err(e)) = (
                    &package,
                    sunrise_launcher_smod::index::native::Bundle::read(&path),
                ) {
                    package = Err(e);
                }
            }
            let (id, version) = package
                .as_ref()
                .map(|p| (p.manifest.id.clone(), p.manifest.version.clone()))
                .unwrap_or_else(|_| (stem(&path), String::new()));
            let name = package
                .as_ref()
                .map_or_else(|_| id.clone(), |p| p.manifest.name.clone());
            let origin = (package.is_ok() && path.file_name()?.to_str()? == format!("{id}.smod"))
                .then(|| crate::github_mods::origin_for_archive(&path))
                .flatten()
                .filter(|origin| native || origin.local_folder_path().is_some())
                .filter(|origin| {
                    sunrise_launcher_smod::index::sha256_file(&path)
                        .is_ok_and(|digest| origin.matches_archive(&id, &version, &digest))
                });
            let internals = package
                .as_ref()
                .map(|p| declared_internals(&p.manifest))
                .unwrap_or_default();
            Some(Installed {
                file,
                id,
                name,
                authors: package
                    .as_ref()
                    .map(|p| p.manifest.authors.clone())
                    .unwrap_or_default(),
                version,
                internals,
                origin,
                served_by: package
                    .as_ref()
                    .ok()
                    .and_then(|p| p.manifest.served_by.clone()),
                icon: package.map(|p| p.icon.map(Arc::new)),
            })
        })
        .collect()
}

/// The mods installed version `version` was built with, or None when it isn't a native build
/// (or its `native-mods.json` can't be read: the Mods page then offers Build).
pub fn built(version: &str) -> Option<Vec<Mod>> {
    Some(parse_built(&record(version)?))
}

/// A mod actually linked into a build, not one the build left out.
#[derive(serde::Deserialize)]
pub(crate) struct Included {
    pub id: String,
    pub version: String,
    pub sha256: String,
}

#[derive(serde::Deserialize)]
struct IncludedRecord {
    mods: Vec<Included>,
}

/// The mods in the selected binary. Unlike [`built`], these exclude `left_out`.
pub(crate) fn included(version: &str) -> Result<Vec<Included>, String> {
    if !native_mods::is_native(version) {
        return Ok(Vec::new());
    }
    record(version).and_then(parse_included).ok_or_else(|| {
        crate::i18n::tr(
            "This native build doesn't say exactly which mods it includes. Rebuild the mods or choose another game build.",
        )
        .into()
    })
}

fn parse_included(record: serde_json::Value) -> Option<Vec<Included>> {
    let IncludedRecord { mods } = serde_json::from_value(record).ok()?;
    let mut ids = std::collections::HashSet::new();
    if mods.is_empty()
        || !mods.iter().all(|m| {
            !m.id.is_empty()
                && !m.version.is_empty()
                && ids.insert(&m.id)
                && m.sha256.len() == 64
                && m.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        })
    {
        return None;
    }
    Some(mods)
}

/// Only linked mods need to match; skipped entries do not run. Compare the exact enabled
/// version and source digest, not just the package id. A plain build has nothing to check.
pub(crate) fn check_compiled_profile(included: &[Included], home: &Path) -> Result<(), String> {
    if !included.is_empty() && !compiled_match(included, &enabled(home)) {
        return Err(crate::i18n::tr(
            "This build includes native mods that don't match this profile. Rebuild the mods or choose another game build.",
        )
        .into());
    }
    Ok(())
}

/// Managed Play cannot silently drop additional enabled native packages, even when
/// every linked package would individually match an enabled archive.
pub(crate) fn check_exact_compiled_profile(
    included: &[Included],
    home: &Path,
) -> Result<(), String> {
    let enabled = enabled(home);
    if included.len() != enabled.len() || !compiled_match(included, &enabled) {
        return Err(crate::i18n::tr(
            "The selected game doesn't include exactly the enabled native mods.",
        )
        .into());
    }
    Ok(())
}

fn compiled_match(included: &[Included], enabled: &[Mod]) -> bool {
    included.iter().all(|m| {
        let mut same_id = enabled.iter().filter(|(id, ..)| id == &m.id);
        same_id.next().is_some_and(|(_, version, sha)| {
            version == &m.version
                && sha
                    .as_ref()
                    .is_some_and(|sha| sha.eq_ignore_ascii_case(&m.sha256))
        }) && same_id.next().is_none()
    })
}

/// Native build `version`'s `native-mods.json`.
fn record(version: &str) -> Option<serde_json::Value> {
    if !native_mods::is_native(version) {
        return None;
    }
    let path = backend::version_dir(version).join("native-mods.json");
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

/// The profile native build `version` was built with (`native-mods.json`'s `profile`).
pub fn built_profile(version: &str) -> Option<native_mods::Profile> {
    serde_json::from_value(record(version)?.get("profile")?.clone()).ok()
}

/// `state`, once the mode counts: a build made in another mode than `chosen` is not this mode's
/// build, so it needs its own.
pub fn in_mode(
    state: State,
    built: Option<native_mods::Profile>,
    chosen: native_mods::Profile,
) -> State {
    if state == State::UpToDate && built != Some(chosen) {
        State::Outdated
    } else {
        state
    }
}

/// The mods a build tried: both those it holds and those it left out.
fn parse_built(about: &serde_json::Value) -> Vec<Mod> {
    let mods = about["mods"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let left_out = about["left_out"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    mods.iter()
        .chain(left_out)
        .filter_map(|m| {
            Some((
                m["id"].as_str()?.into(),
                m["version"].as_str()?.into(),
                m["sha256"].as_str().map(String::from),
            ))
        })
        .collect()
}

/// What [`state`] depends on: the folder, the version, the on/off list and each `.smod`'s size
/// and time. The Mods page reads the mods again when it changes. It reads every mod's file, so a
/// page asking on every frame keeps it with [`Fingerprints`].
pub fn fingerprint(home: &Path, version: Option<&str>) -> String {
    let mut out = format!("{}\n{version:?}\n", home.display());
    // The on/off list (`browser::mod_list::is_enabled`).
    out += &mod_list::selection_text(home);
    for path in hmods(home)
        .into_iter()
        .chain(content_files(home))
        .flat_map(|p| [p.clone(), p.with_extension("origin.json")])
        .chain([
            home.join("mod-browser.json"),
            home.join("mod-browser/active-build.json"),
            home.join(sunrise_launcher_smod::index::browser_state::PENDING),
            home.join(crate::profile_txn::POINTER),
        ])
    {
        let meta = fs::symlink_metadata(&path).ok();
        let time = meta.as_ref().and_then(|m| m.modified().ok());
        out += &format!("\n{} {:?} {time:?}", path.display(), meta.map(|m| m.len()));
    }
    out += &installed_versions_stamp();
    out
}

/// How long [`Fingerprints`] trusts a glance before reading every mod again.
pub const FULL_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

/// [`fingerprint`] for a page that asks on every frame. A pack of a few hundred mods made the
/// full read cost about 7 ms a frame (a `stat` of each archive and its origin, and a line for
/// each), so it is kept, and read again only when a glance says something moved: the two mod
/// folders, whose time changes when a mod is added, removed or renamed into place, the lists
/// beside them, and the on/off list's text itself. A mod rewritten where it lies changes neither,
/// so the full read is also redone every [`FULL_EVERY`], and the message pump asks for it when
/// it is due ([`Fingerprints::due`]) so an idle page sees it too.
#[derive(Default)]
pub struct Fingerprints {
    kept: Option<Kept>,
}

struct Kept {
    home: std::path::PathBuf,
    version: Option<String>,
    glance: String,
    full: String,
    at: std::time::Instant,
}

impl Fingerprints {
    /// [`fingerprint`]`(home, version)`, as of `now`.
    pub fn read(&mut self, home: &Path, version: Option<&str>, now: std::time::Instant) -> String {
        self.read_glanced(home, version, now, glance(home))
    }

    /// Whether the full read is due at `now`: a mod may have been rewritten where it lies.
    pub fn due(&self, now: std::time::Instant) -> bool {
        self.kept
            .as_ref()
            .is_some_and(|kept| now.saturating_duration_since(kept.at) >= FULL_EVERY)
    }

    fn read_glanced(
        &mut self,
        home: &Path,
        version: Option<&str>,
        now: std::time::Instant,
        glance: String,
    ) -> String {
        if let Some(kept) = &self.kept {
            if kept.home == home
                && kept.version.as_deref() == version
                && kept.glance == glance
                && now.saturating_duration_since(kept.at) < FULL_EVERY
            {
                return kept.full.clone();
            }
        }
        let full = fingerprint(home, version);
        self.kept = Some(Kept {
            home: home.to_owned(),
            version: version.map(str::to_owned),
            glance,
            full: full.clone(),
            at: now,
        });
        full
    }
}

/// The few things whose time and size change whenever the mods do, but for a mod rewritten in
/// place ([`Fingerprints`]). The on/off list is read whole, as [`fingerprint`] reads it: an edit
/// of the same length within one tick of the file's time would change neither.
fn glance(home: &Path) -> String {
    let mut out = mod_list::selection_text(home);
    for path in [
        home.join(native_mods::INSTALLED),
        home.join(sunrise_launcher_smod::mod_list::DIR),
        home.join("mod-browser.json"),
        home.join(sunrise_launcher_smod::index::browser_state::PENDING),
        // The active build (`mod-browser/active-build.json`).
        home.join(crate::profile_txn::POINTER),
    ] {
        let meta = fs::symlink_metadata(&path).ok();
        let time = meta.as_ref().and_then(|m| m.modified().ok());
        out += &format!("{:?} {time:?}\n", meta.map(|m| m.len()));
    }
    out += &installed_versions_stamp();
    out
}

/// Installed versions and their reuse evidence so removing or sealing a kept build in Storage
/// invalidates both Mods and Home snapshots immediately.
fn installed_versions_stamp() -> String {
    let mut entries: Vec<_> = fs::read_dir(backend::versions_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let dir = entry.path();
            let installed = fs::symlink_metadata(dir.join(".installed"))
                .ok()
                .and_then(|meta| meta.modified().ok());
            let evidence = fs::symlink_metadata(dir.join(".native-reuse.json"))
                .ok()
                .map(|meta| (meta.len(), meta.modified().ok()));
            Some((name, installed, evidence))
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = String::new();
    for (name, installed, evidence) in entries {
        out += &format!("\n{name} {installed:?} {evidence:?}");
    }
    out
}

fn content_files(home: &Path) -> Vec<std::path::PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(home.join(sunrise_launcher_smod::mod_list::DIR))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "smod") && p.is_file())
        .collect();
    paths.sort();
    paths
}

fn hmods(home: &Path) -> Vec<std::path::PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(home.join(native_mods::INSTALLED))
        .map(|dir| dir.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    // A directory or pipe named `.smod` is not an archive to open.
    paths.retain(|p| p.extension().is_some_and(|e| e == "smod") && p.is_file());
    paths.sort();
    paths
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
