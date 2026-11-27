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
mod tests {
    #[test]
    fn non_files_with_smod_names_are_not_archive_candidates() {
        let home = tempfile::tempdir().unwrap();
        let native = home.path().join(native_mods::INSTALLED);
        let content = home.path().join(sunrise_launcher_smod::mod_list::DIR);
        for dir in [&native, &content] {
            fs::create_dir_all(dir.join("not-an-archive.smod")).unwrap();
        }
        assert!(hmods(home.path()).is_empty());
        assert!(content_files(home.path()).is_empty());
        assert!(enabled(home.path()).is_empty());
        assert!(untracked(home.path()).is_empty());
    }

    #[test]
    fn an_unreadable_file_is_no_build_input_and_never_stands_for_a_mod_of_its_name() {
        let home = tempfile::tempdir().unwrap();
        let native = home.path().join(native_mods::INSTALLED);
        fs::create_dir_all(&native).unwrap();
        fs::write(native.join("ruby.smod"), b"not a package").unwrap();
        // Another mod called `ruby` is on, as its own file or its content says.
        mod_list::set_enabled(home.path(), "ruby", true).unwrap();
        assert!(enabled(home.path()).is_empty());
    }

    #[test]
    fn a_native_file_the_build_cannot_read_is_listed_unreadable() {
        use std::io::Write;
        let home = tempfile::tempdir().unwrap();
        let native = home.path().join(native_mods::INSTALLED);
        fs::create_dir_all(&native).unwrap();
        // A well-formed package with no `[native]` table: the build reads it as no native mod.
        let mut zip = zip::ZipWriter::new(fs::File::create(native.join("plain.smod")).unwrap());
        zip.start_file("sunrise-mod.toml", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(
            b"id='plain'\nname='Plain'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n",
        )
        .unwrap();
        zip.finish().unwrap();
        mod_list::set_enabled(home.path(), "plain", true).unwrap();
        let found = untracked(home.path());
        assert_eq!(found.len(), 1);
        assert!(found[0].icon.is_err());
        assert!(enabled(home.path()).is_empty());
    }

    #[test]
    fn the_uses_table_is_not_part_of_a_mods_digest() {
        let source = [("Cargo.toml", b"a".as_slice()), ("src/lib.rs", b"b")];
        let packed = [source[0], source[1], ("uses.json", b"{}".as_slice())];
        assert_eq!(
            archive_digest(packed.into_iter()),
            archive_digest(source.into_iter())
        );
        assert_eq!(
            archive_digest(source.into_iter()),
            native_mods::digest(source.into_iter())
        );
    }

    #[test]
    fn only_the_changed_mod_needs_a_build() {
        let m = |id: &str, v: &str, d: Option<&str>| (id.into(), v.into(), d.map(Into::into));
        let built = [m("ores", "1.0", Some("a")), m("maps", "2.0", Some("b"))];
        // `maps` updated, `ores` untouched, `tools` new: only the changed and new ones build.
        let enabled = [
            m("ores", "1.0", Some("a")),
            m("maps", "2.1", Some("c")),
            m("tools", "1.0", None),
        ];
        assert_eq!(unbuilt(&enabled, Some(&built)), ["maps", "tools"]);
        // Same version, other files (a reinstalled local `.smod`): that one builds again.
        assert_eq!(
            unbuilt(&[m("ores", "1.0", Some("z"))], Some(&built)),
            ["ores"]
        );
        // An old record without digests still counts as built.
        assert!(unbuilt(
            &[m("ores", "1.0", Some("a"))],
            Some(&[m("ores", "1.0", None)])
        )
        .is_empty());
        assert_eq!(unbuilt(&enabled[..1], None), ["ores"]);
    }

    use super::*;

    #[test]
    fn a_kept_fingerprint_is_read_again_when_the_glance_moves_when_due_and_for_another_version() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let mods = home.join(sunrise_launcher_smod::mod_list::DIR);
        fs::create_dir_all(&mods).unwrap();
        fs::write(mods.join("a.smod"), b"one").unwrap();
        let mut kept = Fingerprints::default();
        let t0 = std::time::Instant::now();
        let first = kept.read_glanced(home, None, t0, "a".into());
        assert_eq!(first, fingerprint(home, None));

        // A mod rewritten where it lies: the full fingerprint moves, the glance doesn't.
        fs::write(mods.join("a.smod"), b"one, longer").unwrap();
        assert_ne!(fingerprint(home, None), first);
        assert_eq!(kept.read_glanced(home, None, t0, "a".into()), first, "kept");
        assert!(!kept.due(t0));
        // A glance that moved is read again at once.
        let moved = kept.read_glanced(home, None, t0, "b".into());
        assert_eq!(moved, fingerprint(home, None));

        // Rewritten again: kept until the full read is due, then read.
        fs::write(mods.join("a.smod"), b"one, longer still").unwrap();
        let later = t0 + FULL_EVERY;
        assert!(kept.due(later));
        assert_eq!(
            kept.read_glanced(home, None, later, "b".into()),
            fingerprint(home, None)
        );
        assert!(!kept.due(later), "read again at its time");

        // Only the version differs: the same folder, glance and clock are not enough.
        let versioned = kept.read_glanced(home, Some("26.3"), later, "b".into());
        assert_eq!(versioned, fingerprint(home, Some("26.3")));
        assert_ne!(versioned, fingerprint(home, None));
    }

    #[test]
    fn the_glance_sees_the_lists_text_and_times_it_watches() {
        use std::time::{Duration, SystemTime};
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let set_time = |path: &Path, time: SystemTime| {
            fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(time)
                .unwrap();
        };
        // The on/off list edited to the same length, its time put back: still seen.
        let list = home.join(mod_list::MODS_LIST);
        fs::write(&list, "mod.a=true\n").unwrap();
        set_time(&list, at);
        let before = glance(home);
        fs::write(&list, "mod.b=true\n").unwrap();
        set_time(&list, at);
        assert_ne!(glance(home), before, "which mods are on changed");

        // The browser's list rewritten: its time moves.
        let browser = home.join("mod-browser.json");
        fs::write(&browser, b"{}").unwrap();
        set_time(&browser, at);
        let before = glance(home);
        set_time(&browser, at + Duration::from_secs(1));
        assert_ne!(glance(home), before);

        // A mod added to a folder moves the folder's time (set here, not waited for).
        #[cfg(unix)]
        {
            let mods = home.join(sunrise_launcher_smod::mod_list::DIR);
            fs::create_dir_all(&mods).unwrap();
            let set_dir = |time: SystemTime| {
                fs::File::open(&mods).unwrap().set_modified(time).unwrap();
            };
            set_dir(at);
            let before = glance(home);
            set_dir(at + Duration::from_secs(1));
            assert_ne!(glance(home), before);
        }
    }

    fn m(id: &str, version: &str) -> Mod {
        (id.into(), version.into(), None)
    }

    fn d(id: &str, version: &str, digest: &str) -> Mod {
        (id.into(), version.into(), Some(digest.into()))
    }

    #[test]
    fn declared_paths_use_root_internals() {
        let manifest = sunrise_launcher_smod::Manifest::parse(
            "id='owner'\nname='Owner'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\ninternals=[{path='Game',hash='game'},{path='Game::world',kind='field',hash='world'}]\n[native]\ngame='26.3'\nsunrise='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'\nmixins='bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'\n",
        )
        .unwrap();
        assert_eq!(declared_internals(&manifest), ["Game", "Game::world"]);
    }

    #[test]
    fn a_content_package_served_by_a_native_mod_is_listed_with_its_host() {
        let home = tempfile::tempdir().unwrap();
        let folder = home.path().join(sunrise_launcher_smod::mod_list::DIR);
        fs::create_dir_all(&folder).unwrap();
        let bytes = sunrise_launcher_smod::manifest::Package::write_files(
            "id='runes'\nname='Runes'\nversion='1.0.0'\napi='0.2'\nserved_by='java_host'\n",
            &[],
        )
        .unwrap();
        fs::write(folder.join("runes.smod"), bytes).unwrap();
        let listed = untracked_content(home.path());
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "runes");
        assert_eq!(listed[0].served_by.as_deref(), Some("java_host"));
    }

    #[test]
    fn content_package_changes_invalidate_the_local_mods_view() {
        let home = tempfile::tempdir().unwrap();
        let folder = home.path().join(sunrise_launcher_smod::mod_list::DIR);
        fs::create_dir_all(&folder).unwrap();
        let package = folder.join("example.smod");
        fs::write(&package, b"a").unwrap();
        let before = fingerprint(home.path(), None);
        fs::write(&package, b"ab").unwrap();
        assert_ne!(before, fingerprint(home.path(), None));
    }

    #[test]
    fn github_and_local_files_are_listed_once_and_drive_build_state() {
        use crate::browser::{installed, Kind, SourceId};
        use std::io::Write;
        let home = std::env::temp_dir().join(format!("native-list-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(home.join(native_mods::INSTALLED)).unwrap();
        let pin = "a".repeat(40);
        for (file, id) in [
            ("registry", "registry"),
            ("github", "github"),
            ("renamed", "local"),
        ] {
            let path = home
                .join(native_mods::INSTALLED)
                .join(format!("{file}.smod"));
            let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
            let manifest = format!(
                "id = '{id}'\nname = '{id}'\nversion = '1.2'\napi = '0.2'\ncapabilities = ['registries']\n[native]\ngame = '26.3'\nsunrise = '{pin}'\nmixins = '{pin}'\n"
            );
            for (name, bytes) in [
                ("sunrise-mod.toml", manifest.as_str()),
                (
                    "Cargo.toml",
                    "[package]\nname = 'unrelated'\nversion = '9'\n",
                ),
                ("src/lib.rs", "pub fn hooks() {}\n"),
            ] {
                zip.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(bytes.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        let record = crate::github_mods::Origin {
            id: "github".into(),
            version: "1.2".into(),
            owner: "example".into(),
            repo: "mods".into(),
            tag: None,
            commit: pin,
            asset: None,
            source: None,
            added_from: None,
            sha256: sunrise_launcher_smod::index::sha256_file(
                &home.join(native_mods::INSTALLED).join("github.smod"),
            )
            .unwrap(),
        };
        fs::write(
            home.join(native_mods::INSTALLED).join("github.origin.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        installed::Installed {
            entries: vec![installed::Entry {
                source: SourceId::Sunrise,
                project: "registry".into(),
                mod_id: None,
                slug: "registry".into(),
                title: "Registry mod".into(),
                kind: Kind::Mod,
                version: "1.2".into(),
                number: "1.2".into(),
                file: format!("{}/registry.smod", native_mods::INSTALLED),
                checksum: String::new(),
                page: String::new(),
                dependency: false,
                verdict: None,
                install_hold: None,
                repository: None,
            }],
        }
        .save(&home)
        .unwrap();
        let files = untracked(&home);
        let seen: Vec<_> = files
            .iter()
            .map(|m| (m.id.as_str(), m.version.as_str(), m.origin.is_some()))
            .collect();
        assert_eq!(seen, [("github", "1.2", true), ("local", "1.2", false)]);
        assert!(files.iter().all(|m| matches!(&m.icon, Ok(None))));
        for id in ["registry", "github", "local"] {
            mod_list::set_enabled(&home, id, true).unwrap();
        }
        assert_eq!(state(&enabled(&home), None), State::NotBuilt);
        let mut pins = pinning(&home, &"a".repeat(40));
        pins.sort();
        assert_eq!(
            pins,
            ["github", "local", "registry"],
            "fit reads the enabled flat packages, including a renamed local file"
        );
        assert!(pinning(&home, &"b".repeat(40)).is_empty());
        for id in ["registry", "github", "local"] {
            mod_list::set_enabled(&home, id, false).unwrap();
        }
        assert_eq!(state(&enabled(&home), None), State::NoMods);
        assert!(
            bundles_on(&home).is_empty(),
            "disabled packages pin nothing"
        );
        assert_eq!(
            untracked(&home).len(),
            2,
            "disabled mods remain in the list"
        );
        let before = fingerprint(&home, None);
        fs::remove_file(home.join(&files[0].file)).unwrap();
        assert_ne!(before, fingerprint(&home, None));
        assert_eq!(
            untracked(&home).len(),
            1,
            "a stale origin is not an installed mod"
        );
        fs::write(
            home.join(native_mods::INSTALLED).join("broken.smod"),
            b"broken",
        )
        .unwrap();
        let files = untracked(&home);
        assert_eq!(
            files[0].id, "broken",
            "unreadable files can still be removed"
        );
        assert!(files[0].icon.is_err(), "a refusal is not an absent icon");
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_build_made_in_another_mode_needs_a_build() {
        use native_mods::Profile::Optimized;
        assert_eq!(in_mode(State::UpToDate, None, Optimized), State::Outdated);
        assert_eq!(
            in_mode(State::UpToDate, Some(Optimized), Optimized),
            State::UpToDate
        );
        assert_eq!(in_mode(State::NoMods, None, Optimized), State::NoMods);
    }

    #[test]
    fn a_build_is_needed_until_the_version_holds_the_mods_turned_on() {
        let on = [m("veinminer", "1.0"), m("zoom", "2.1")];
        assert_eq!(state(&[], None), State::NoMods);
        assert_eq!(
            state(&[], Some(&on)),
            State::NoMods,
            "nothing on: nothing to build"
        );
        assert_eq!(state(&on, None), State::NotBuilt, "not a native build");
        let reordered = [m("zoom", "2.1"), m("veinminer", "1.0")];
        assert_eq!(state(&on, Some(&reordered)), State::UpToDate);
        assert_eq!(
            state(&on, Some(&[m("veinminer", "1.0")])),
            State::Outdated,
            "a mod turned on since"
        );
        assert_eq!(
            state(&on[..1], Some(&on)),
            State::Outdated,
            "a mod turned off since"
        );
        assert_eq!(
            state(&on, Some(&[m("veinminer", "1.1"), m("zoom", "2.1")])),
            State::Outdated,
            "a mod updated since"
        );
    }

    #[test]
    fn the_same_version_with_other_files_needs_a_build() {
        let on = [d("zoom", "2.1", "aa")];
        assert_eq!(state(&on, Some(&[d("zoom", "2.1", "aa")])), State::UpToDate);
        assert_eq!(
            state(&on, Some(&[d("zoom", "2.1", "bb")])),
            State::Outdated,
            "reinstalled without a version bump"
        );
        assert_eq!(
            state(&on, Some(&[m("zoom", "2.1")])),
            State::UpToDate,
            "a build from before the digest was kept"
        );
    }

    #[test]
    fn native_mods_json_gives_the_built_mods() {
        let about = serde_json::json!({
            "base": "v0.1.0", "sunrise": "abc", "rust": "1.99.1",
            "mods": [{"id": "zoom", "version": "2.1"}, {"id": "x"}, {"id": "y", "version": "1", "sha256": "ab"}],
        });
        assert_eq!(parse_built(&about), [m("zoom", "2.1"), d("y", "1", "ab")]);
        assert!(parse_built(&serde_json::json!({})).is_empty());
        let about = serde_json::json!({
            "mods": [{"id": "zoom", "version": "2.1"}],
            "left_out": [{"id": "hitbox_plus", "version": "1.3.0", "sha256": "aa", "why": "other-version"}],
        });
        let build = parse_built(&about);
        let on = [m("zoom", "2.1"), d("hitbox_plus", "1.3.0", "aa")];
        assert_eq!(state(&on, Some(build.as_slice())), State::UpToDate);
        let on = [m("zoom", "2.1"), m("hitbox_plus", "1.3.1")];
        assert_eq!(state(&on, Some(build.as_slice())), State::Outdated);
        let on = [m("zoom", "2.1"), d("hitbox_plus", "1.3.0", "bb")];
        assert_eq!(state(&on, Some(build.as_slice())), State::Outdated);
        assert_eq!(built("v0.1.0"), None, "not a native build");
    }

    #[test]
    fn launch_requires_a_complete_inventory_and_an_exact_enabled_match() {
        use serde_json::json;
        let hash = "a".repeat(64);
        let linked = json!({"id": "zoom", "version": "2.1", "sha256": hash});
        let included = parse_included(json!({
            "mods": [linked],
            "left_out": [{"id": "hitbox_plus", "version": "1.3.0"}],
        }))
        .unwrap();
        // Left-out mods don't run, so they don't need to be installed or enabled.
        assert_eq!(included.len(), 1);
        assert!(compiled_match(
            &included,
            &[("zoom".into(), "2.1".into(), Some(hash.clone()))]
        ));
        // An empty profile, missing digest, or wrong version or digest must not play the binary.
        assert!(!compiled_match(&included, &[]));
        for (version, sha) in [
            ("2.1", None),
            ("2.2", Some(hash)),
            ("2.1", Some("b".repeat(64))),
        ] {
            assert!(!compiled_match(
                &included,
                &[("zoom".into(), version.into(), sha)]
            ));
        }
        for record in [
            json!({}),
            json!({"mods": []}),
            json!({"mods": [{"id": "zoom", "version": "2.1"}]}),
            json!({"mods": [{"id": "zoom", "version": "2.1", "sha256": "xyz"}]}),
            json!({"mods": [linked, linked]}),
            json!({"mods": [linked, null]}),
        ] {
            assert!(parse_included(record).is_none());
        }
    }

    #[test]
    fn builds_name_the_sunrise_they_were_built_on() {
        assert_eq!(sunrise_label("v0.2.93"), "v0.2.93");
        assert_eq!(
            sunrise_label("nightly-2026-10-29-4baa571"),
            "Nightly 2026-10-29"
        );
        assert_eq!(sunrise_label("nightly-2026-10-29"), "Nightly 2026-10-29");
        let build = Build {
            base: "nightly-2026-10-28-0c02067".into(),
            ..Build::default()
        };
        assert_eq!(build.sunrise(), "Nightly 2026-10-28");
        assert_eq!(build.sunrise_short(), "Nightly 10-28");
        let release = Build {
            base: "v0.2.93".into(),
            ..Build::default()
        };
        assert_eq!(release.sunrise_short(), "v0.2.93");
        assert_eq!(Build::read("v0.1.0"), None, "not a native build");
    }

    /// A build from before pins were recorded that compiled another commit than its Nightly's
    #[test]
    fn only_hmods_in_the_native_folder_are_native() {
        assert!(is_native_file("sunrise-native-mods/veinminer.smod"));
        assert!(!is_native_file("sunrise-native-mods-old/x.smod"));
        assert!(!is_native_file("sunrise-mods/zoom.smod"));
        assert!(!is_native_file("resourcepacks/x.zip"));
    }
}
