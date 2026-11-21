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

/// Check at acceptance and again after session and jar preparation, just before handing off
/// the game command. An allowed Play is not permission cached for the duration of preparation.
pub fn check_permission(version: &str) -> Result<(), String> {
    check_permission_in(version, &backend::data_dir())
}

fn check_permission_in(version: &str, data: &Path) -> Result<(), String> {
    if native_mods::is_native(version)
        && backend::Settings::load_from(data)
            .native_mods_allowed_at
            .is_none()
    {
        return Err(crate::i18n::tr(
            "This profile plays a build made with native mods, and native mods are turned off. Choose another game build for it in Versions.",
        )
        .into());
    }
    Ok(())
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

/// One package a build takes in, as the native-code question names it.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildInput {
    /// The manifest's id: the key of the on/off list.
    pub id: String,
    /// The package's file name in the game folder.
    pub file: String,
    pub registry: Registry,
}

/// What the registry says about a package's file.
#[derive(Clone, Debug, PartialEq)]
pub enum Registry {
    /// This very file is listed for the package's id, under this name and artwork.
    Listed { name: String, icon: Option<String> },
    /// The registry lists no such file for that id.
    NotListed,
    /// The registry has taken this file or its project down, even if it still lists it.
    Revoked,
    /// The registry hasn't answered in this run, so nothing says whose file it is.
    Unchecked,
}

/// What a build of game folder `home` takes in ([`enabled`]'s packages), each named from its own
/// file: a package gets the registry's name only when its bytes are a file `index` lists for its
/// id and has not taken down. A package someone put there under a registry mod's id stays an
/// id and a file name.
/// `index` is the registry's own answer (`sunrise::fetched_index`), never the copy kept in the
/// game folder, which a package there could have written to list itself.
pub fn build_inputs(
    home: &Path,
    index: Option<&sunrise_launcher_smod::index::Index>,
) -> Vec<BuildInput> {
    enabled_files(home)
        .into_iter()
        .map(|(path, (id, _, _))| {
            let registry = match index {
                None => Registry::Unchecked,
                Some(index) => match sunrise_launcher_smod::index::sha256_file(&path) {
                    Ok(hash) if index.revoked(&id, &hash).is_some() => Registry::Revoked,
                    Ok(hash) => index
                        .project(&id)
                        .filter(|p| {
                            p.versions
                                .iter()
                                .any(|v| v.file.sha256.eq_ignore_ascii_case(&hash))
                        })
                        .map_or(Registry::NotListed, |p| Registry::Listed {
                            name: p.name.clone(),
                            icon: p.icon.clone(),
                        }),
                    Err(_) => Registry::NotListed,
                },
            };
            let file = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            BuildInput { id, file, registry }
        })
        .collect()
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
    pub version: String,
    pub internals: Vec<String>,
    pub origin: Option<crate::github_mods::Origin>,
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
            let origin = (native && path.file_stem()?.to_str()? == id)
                .then(|| crate::github_mods::origin(home, &id))
                .flatten();
            let internals = package
                .as_ref()
                .map(|p| declared_internals(&p.manifest))
                .unwrap_or_default();
            Some(Installed {
                file,
                id,
                name,
                version,
                internals,
                origin,
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

/// What [`state`] depends on, cheap to read on every frame: the folder, the version, the on/off
/// list and each `.smod`'s size and time. The Mods page reads the mods again when it changes.
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
            home.join(sunrise_launcher_smod::index::browser_state::PENDING),
            home.join(crate::profile_txn::POINTER),
        ])
    {
        let meta = fs::symlink_metadata(&path).ok();
        let time = meta.as_ref().and_then(|m| m.modified().ok());
        out += &format!("\n{} {:?} {time:?}", path.display(), meta.map(|m| m.len()));
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
        assert!(build_inputs(home.path(), None).is_empty());
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
    fn permission_taken_back_while_preparing_blocks_native_spawn() {
        let data = tempfile::tempdir().unwrap();
        let settings = data.path().join("settings.json");
        fs::write(&settings, r#"{"native_mods_allowed_at":123}"#).unwrap();
        assert_eq!(check_permission_in("native-test", data.path()), Ok(()));

        fs::write(&settings, "{}").unwrap();
        assert!(check_permission_in("native-test", data.path()).is_err());
        assert_eq!(check_permission_in("v1.2.3", data.path()), Ok(()));
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
            sha256: "ab".repeat(32),
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
    fn a_local_package_never_takes_the_name_of_the_registry_mod_with_its_id() {
        use std::io::Write;
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/smod/tests/index");
        let mut index = sunrise_launcher_smod::index::Index::parse(
            &fs::read_to_string(fixtures.join("index-v1.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(index.project("coords_hud").unwrap().name, "Coordinates HUD");
        let home = tempfile::tempdir().unwrap();
        let folder = home.path().join(native_mods::INSTALLED);
        fs::create_dir_all(&folder).unwrap();
        // A package of someone's own under the registry mod's id, in a file named like its file.
        let local = folder.join("coords_hud-0.1.0.smod");
        let pin = "a".repeat(40);
        let mut zip = zip::ZipWriter::new(fs::File::create(&local).unwrap());
        let manifest = format!(
            "id = 'coords_hud'\nname = 'Coordinates HUD'\nversion = '0.1.0'\napi = '0.2'\ncapabilities = ['registries']\n[native]\ngame = '26.3'\nsunrise = '{pin}'\nmixins = '{pin}'\n"
        );
        for (name, bytes) in [
            ("sunrise-mod.toml", manifest.as_str()),
            ("Cargo.toml", "[package]\nname = 'x'\nversion = '1'\n"),
            ("src/lib.rs", "pub fn hooks() {}\n"),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        mod_list::set_enabled(home.path(), "coords_hud", true).unwrap();
        let input = |registry: Registry| BuildInput {
            id: "coords_hud".into(),
            file: "coords_hud-0.1.0.smod".into(),
            registry,
        };
        let not_listed = [input(Registry::NotListed)];
        let unchecked = [input(Registry::Unchecked)];
        assert_eq!(build_inputs(home.path(), Some(&index)), not_listed);
        assert_eq!(build_inputs(home.path(), None), unchecked);
        // The same bytes as a file the registry lists for that id: the registry's name.
        let listed = &mut index
            .projects
            .iter_mut()
            .find(|p| p.id == "coords_hud")
            .unwrap();
        listed.versions[0].file.sha256 = sunrise_launcher_smod::index::sha256_file(&local).unwrap();
        assert_eq!(
            build_inputs(home.path(), Some(&index)),
            [input(Registry::Listed {
                name: "Coordinates HUD".into(),
                icon: index.project("coords_hud").unwrap().icon.clone(),
            })]
        );
        assert_eq!(build_inputs(home.path(), None), unchecked);
        let hash = index.project("coords_hud").unwrap().versions[0]
            .file
            .sha256
            .clone();
        // The exact bytes still appear in the catalogue, but the takedown wins.
        index.revoked.push(sunrise_launcher_smod::index::Revoked {
            id: "coords_hud".into(),
            sha256: Some(hash.clone()),
            reason: "Test takedown".into(),
            date: String::new(),
        });
        assert_eq!(
            build_inputs(home.path(), Some(&index)),
            [input(Registry::Revoked)]
        );
        assert_eq!(build_inputs(home.path(), None), unchecked);
        index.revoked.clear();
        // Listed under another project's id only: still not the registry's.
        for p in &mut index.projects {
            let own = p.id == "coords_hud";
            p.versions[0].file.sha256 = if own { "0".repeat(64) } else { hash.clone() };
        }
        assert_eq!(build_inputs(home.path(), Some(&index)), not_listed);
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

    /// Old switches and notices never count as native-code consent.
    #[test]
    fn old_settings_with_the_native_mods_switch_still_read() {
        let s: backend::Settings =
            serde_json::from_str(r#"{"native_mods": true, "native_mods_noticed": true}"#).unwrap();
        assert!(s.native_mods_allowed_at.is_none());
        let s: backend::Settings = serde_json::from_str(r#"{"native_mods": false}"#).unwrap();
        assert!(s.native_mods_allowed_at.is_none());
        let saved = serde_json::to_string(&s).unwrap();
        assert!(!saved.contains("native_mods"), "{saved}");
    }
}
