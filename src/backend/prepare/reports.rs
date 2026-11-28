//! The preparer and what it reports: whether its output fits this build, its sources'
//! fingerprint, the runtime reports, and running it.
use super::{
    candidate_sources, download_for_owner, ensure_server_jar, files_under, java, mojang_os,
    resources, resources_at, resources_for_game, resources_owner, resources_owner_in,
    set_prepare_step, with_java, OwnerDownloadError, PrepareStep, Unprepared, PREPARATION_STAMP,
    RESOURCES_MANIFEST,
};
use crate::backend::{
    data_dir, fetch_bytes_cancellable, game_version, installed_in, installed_resources_owner,
    prepare_lock_at, read_json, version_dir, Settings,
};
use crate::managed_play::Resources;
use crate::{mojang, sources};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(in crate::backend) fn preparer_source_error(error: &std::io::Error) -> String {
    crate::i18n::trf(
        "Could not read the build's preparer source: {error}",
        &[("error", &error.to_string())],
    )
}

pub(in crate::backend) fn server_only_resources() -> String {
    crate::i18n::tr("These files are for a dedicated server only.").into()
}

pub(in crate::backend) fn server_profile(manifest: &serde_json::Value) -> bool {
    manifest["profile"] == "server"
}

fn play_manifest(resources: &Path, required: Option<u32>) -> Result<serde_json::Value, Unprepared> {
    let manifest = read_json(&resources.join(RESOURCES_MANIFEST)).ok_or(Unprepared::Incomplete)?;
    if server_profile(&manifest) {
        return Err(Unprepared::ServerOnly);
    }
    if let Some(required) = required {
        let stamped = read_json(&resources.join(PREPARATION_STAMP))
            .and_then(|stamp| stamp["output_version"].as_u64());
        if stamped != Some(u64::from(required)) {
            return Err(Unprepared::Incomplete);
        }
    }
    Ok(manifest)
}

/// Check profile and stamp on a sealed tree, whose files are verified separately by digest.
pub(crate) fn check_play_output(root: &Path, resources: &Path) -> Result<(), Unprepared> {
    let required = output_version_at(root).map_err(Unprepared::PreparerSource)?;
    play_manifest(resources, required).map(drop)
}

/// A full, current preparation whose top-level tables were not left empty by an interrupted run.
pub(crate) fn check_play_resources(root: &Path, resources: &Path) -> Result<(), Unprepared> {
    check_play_output(root, resources)?;
    prepared_tables_present(resources)
        .then_some(())
        .ok_or(Unprepared::Incomplete)
}

/// The manifest and tables are present, without checking for a playable profile or stamp.
pub(crate) fn prepared_at(resources: &Path) -> bool {
    read_json(&resources.join(RESOURCES_MANIFEST)).is_some() && prepared_tables_present(resources)
}

fn prepared_tables_present(resources: &Path) -> bool {
    let Ok(entries) = fs::read_dir(resources) else {
        return false;
    };
    entries.flatten().all(|entry| {
        let path = entry.path();
        path.extension() != Some(std::ffi::OsStr::new("json"))
            || fs::metadata(&path).is_ok_and(|meta| meta.len() > 0)
    })
}

/// Whether a version's resources hold Minecraft's sounds, which `sounds` copies from the player's own
/// install beside the preparation. A version prepared while that copy failed (Mojang's republished 26.3
/// asset index, #1076), or one that reused such a version's resources, plays no sound at all (#1035).
pub fn has_sounds(tag: &str) -> bool {
    sounds_in(&resources(tag))
}

pub(in crate::backend) fn sounds_in(resources: &Path) -> bool {
    // What the game's sound registry reads (sunrise-client sound_events.rs).
    resources.join("assets/minecraft/sounds.json").is_file()
}

/// What the Play page says of a version's resources.
pub fn resources_summary(prepared: bool, sounds: bool, version: &str) -> String {
    use crate::i18n::trf;
    match (prepared, sounds) {
        (false, _) => trf(
            "Not prepared. Sunrise needs the files of your own Minecraft {version}.",
            &[("version", version)],
        ),
        (true, true) => trf("Prepared for Minecraft {version}", &[("version", version)]),
        (true, false) => trf(
            "Prepared for Minecraft {version}, without its sounds: the game will be silent. \
             Prepare again to copy them from your Minecraft install.",
            &[("version", version)],
        ),
    }
}

/// Whether the resources `from` prepared are the ones preparing `to` would make: both pin the same
/// Minecraft jars (research-lock.json) and their preparers make the same output. A preparer
/// declares what it makes with `OUTPUT_VERSION` (crates/sunrise-prepare/src/lib.rs, whose sources
/// ship in the release folder), bumped by hand when the output changes and checked by CI
/// (tools/prepare_output.py), so a release that changes the preparer's code but not its output
/// keeps the resources. Preparers from before the number are known by their sources
/// (`LEGACY_OUTPUT_VERSIONS`); older ones still are compared file by file (`same_preparer`).
pub(in crate::backend) fn resources_compatible(
    from: &Path,
    to: &Path,
    resources: &Path,
    legacy: &[(&str, u32)],
) -> bool {
    let pins = |dir: &Path| {
        read_json(&dir.join("research-lock.json")).map(|lock| {
            ["version", "client_sha1", "server_sha1"].map(|k| lock["minecraft"][k].clone())
        })
    };
    let (Some(a), Some(b)) = (pins(from), pins(to)) else {
        return false;
    };
    if a != b || a.iter().any(|v| !v.is_string()) {
        return false;
    }
    let declared = |root: &Path| {
        output_version_at(root).inspect_err(|error| {
            eprintln!("{}: {}", root.display(), preparer_source_error(error));
        })
    };
    let (Ok(source), Ok(target)) = (declared(from), declared(to)) else {
        return false;
    };
    // Reject unstamped output from a declared preparer even when the destination's same
    // compatibility version came from a historical source fingerprint.
    let Ok(manifest) = play_manifest(resources, source.or(target)) else {
        return false;
    };
    let compat_version =
        |dir, declared: Option<u32>| declared.or_else(|| legacy_output_version(dir, legacy));
    match (compat_version(from, source), compat_version(to, target)) {
        (Some(a), Some(b)) => a == b,
        (None, None) => same_preparer(from, to, &manifest),
        _ => false,
    }
}

/// The build's own preparer declares the output version it records in its stamp.
pub(in crate::backend) fn output_version_at(dir: &Path) -> std::io::Result<Option<u32>> {
    let lib = match fs::read_to_string(dir.join(PREPARER_CRATE).join("src/lib.rs")) {
        Ok(lib) => lib,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(lib.lines().find_map(|line| {
        line.strip_prefix("pub const OUTPUT_VERSION: u32 = ")?
            .strip_suffix(';')?
            .parse()
            .ok()
    }))
}

/// Fingerprints let two preparers agree where older builds did not declare a version or write a
/// stamp. Use them for comparing preparers, not for requiring a stamp that was never written.
pub(in crate::backend) fn legacy_output_version(dir: &Path, legacy: &[(&str, u32)]) -> Option<u32> {
    let fingerprint = legacy_fingerprint(dir)?;
    legacy
        .iter()
        .find(|(known, _)| *known == fingerprint)
        .map(|&(_, version)| version)
}

/// Preparers from before `OUTPUT_VERSION`, by `legacy_fingerprint`, whose output
/// tools/prepare_output.py measured: what they prepare is version 1's.
pub(in crate::backend) const LEGACY_OUTPUT_VERSIONS: [(&str, u32); 2] = [
    // v0.2.52 to v0.2.64
    (
        "54218ade1c058b1f15ea4094bf8c1a7893adb37b1ed5cc803ddae8e950c31704",
        1,
    ),
    // v0.2.49 to v0.2.51
    (
        "da9ffe7558129c38b44abf43289e81946fe9f2512e5e07272eeae0cf0b4f1254",
        1,
    ),
];

/// The vanilla exporters those preparers ran (the `*_exporters_sha256` of their manifests), in
/// mods/sunrise/src/test/java/dev/sunriselab/parity.
pub(in crate::backend) const LEGACY_EXPORTERS: [&str; 18] = [
    "ParityData.java",
    "VanillaBlockBehaviourExporter.java",
    "VanillaBlockLandingOracle.java",
    "VanillaBlockStateBindingOracle.java",
    "VanillaCollisionOracle.java",
    "VanillaCommandParseOracle.java",
    "VanillaCreativeTabsExporter.java",
    "VanillaCuboidModelOracle.java",
    "VanillaDamageSourceOracle.java",
    "VanillaEntityAttributeOracle.java",
    "VanillaEntityModelExporter.java",
    "VanillaEntityTypesExporter.java",
    "VanillaFaceVisibilityOracle.java",
    "VanillaGameplayFactsExporter.java",
    "VanillaHeightmapOracle.java",
    "VanillaMobDataExporter.java",
    "VanillaMovementRegistryOracle.java",
    "VanillaPlayerHealthOracle.java",
];

/// The digest preparers before `OUTPUT_VERSION` gave their crate's sources: `Cargo.toml`, `build.rs` and `src/*.rs`, CRs dropped.
fn preparer_sources_sha256(crate_dir: &Path) -> Option<String> {
    use sha2::Digest;
    let mut files = Vec::new();
    for name in ["Cargo.toml", "build.rs"] {
        files.push((name.to_string(), fs::read(crate_dir.join(name)).ok()?));
    }
    let mut sources: Vec<String> = fs::read_dir(crate_dir.join("src"))
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".rs"))
        .collect();
    sources.sort();
    for name in sources {
        let data = fs::read(crate_dir.join("src").join(&name)).ok()?;
        files.push((format!("src/{name}"), data));
    }
    let mut hash = sha2::Sha256::new();
    for (name, data) in files {
        let data: Vec<u8> = data.into_iter().filter(|b| *b != b'\r').collect();
        hash.update(format!("{name}\0{}\0", data.len()));
        hash.update(&data);
    }
    Some(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// A version folder's preparer as the launchers before `OUTPUT_VERSION` compared it: the digest
/// of its sources ([`preparer_sources_sha256`]) and of the exporters it ran, line endings
/// as checked out ignored.
pub(in crate::backend) fn legacy_fingerprint(dir: &Path) -> Option<String> {
    use sha2::Digest;
    let sources = preparer_sources_sha256(&dir.join(PREPARER_CRATE))?;
    let mut hash = sha2::Sha256::new();
    hash.update(sources.as_bytes());
    for name in LEGACY_EXPORTERS {
        let path = format!("mods/sunrise/src/test/java/dev/sunriselab/parity/{name}");
        let data: Vec<u8> = fs::read(dir.join(&path))
            .ok()?
            .into_iter()
            .filter(|b| *b != b'\r')
            .collect();
        hash.update(format!("{path}\0{}\0", data.len()));
        hash.update(&data);
    }
    Some(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// How launchers before `OUTPUT_VERSION` compared two preparers, for versions older than the
/// ones `LEGACY_OUTPUT_VERSIONS` knows. The manifest records the hashes of the vanilla exporters
/// the resources were made with (mods/sunrise/src/test/java/.../parity), and the preparer writes
/// every file and the manifest's `schema`, so both need the same preparer and the same exporter
/// sources. The preparer is crates/sunrise-prepare and tools/prepare_native_client.py (the
/// Python preparer of older versions, a wrapper of the binary since); a file only one of the two
/// versions has is a different preparer. The helpers the Python preparer imported (lab.py and
/// the test_*_parity.py modules) change between releases in how they find and check the
/// player's files, which the output does not depend on; they are not compared.
fn same_preparer(from: &Path, to: &Path, manifest: &serde_json::Value) -> bool {
    let Some(fields) = manifest.as_object() else {
        return false;
    };
    let mut preparer = vec![PathBuf::from("tools/prepare_native_client.py")];
    for dir in [from, to] {
        preparer.extend(files_under(dir, Path::new(PREPARER_CRATE)));
    }
    let same_or_absent = |path: &Path| match (fs::read(from.join(path)), fs::read(to.join(path))) {
        (Ok(a), Ok(b)) => a == b,
        (Err(a), Err(b)) => {
            a.kind() == std::io::ErrorKind::NotFound && b.kind() == std::io::ErrorKind::NotFound
        }
        _ => false,
    };
    if !preparer.iter().all(|p| same_or_absent(p)) {
        return false;
    }
    let mut sources = vec![
        "mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaHeightmapOracle.java".to_string(),
    ];
    for (_, value) in fields
        .iter()
        .filter(|(key, _)| key.ends_with("_exporters_sha256"))
    {
        let Some(files) = value.as_object() else {
            return false;
        };
        sources.extend(files.keys().cloned());
    }
    let same = |source: &str| {
        let path = Path::new(source);
        path.is_relative()
            && path
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
            && matches!((fs::read(from.join(path)), fs::read(to.join(path))), (Ok(a), Ok(b)) if a == b)
    };
    sources.iter().all(|s| same(s))
}

/// Where a version folder keeps the preparer's sources (the release package copies them).
pub(in crate::backend) const PREPARER_CRATE: &str = "crates/sunrise-prepare";

/// The data generator reports the game reads at run time: the built-in registries' network ids
/// (without them no server can be joined or hosted) and the command tree.
pub(in crate::backend) const RUNTIME_REPORTS: [&str; 2] = ["registries.json", "commands.json"];

pub(in crate::backend) fn has_runtime_reports(resources: &Path) -> bool {
    RUNTIME_REPORTS
        .iter()
        .all(|name| resources.join(name).is_file())
}

/// Puts the data generator reports the game reads at run time (`RUNTIME_REPORTS`) into a prepared version's
/// resources when they are missing. Preparers before they shipped there left them only in the
/// scratch output of the version that ran the data generator (`.local/datagen/out/reports`),
/// and resources copied from another version (`reuse_prepared`) came without them. They depend
/// only on the server jar, so the version's own scratch output or any installed version pinning
/// the same jar serves, and no new preparation is needed. Returns whether all are there.
pub(in crate::backend) fn complete_reports(versions: &Path, tag: &str) -> bool {
    let target = versions.join(tag);
    complete_reports_at(versions, tag, &target, &resources_at(&target))
}

/// Complete the reports in the already resolved hosting resource folder.
pub(crate) fn complete_reports_at(versions: &Path, tag: &str, target: &Path, to: &Path) -> bool {
    if !to.join(RESOURCES_MANIFEST).exists() {
        return false;
    }
    let missing: Vec<_> = RUNTIME_REPORTS
        .iter()
        .filter(|name| !to.join(name).is_file())
        .collect();
    if missing.is_empty() {
        return true;
    }
    let server = |dir: &Path| {
        read_json(&dir.join("research-lock.json")).and_then(|lock| {
            let pins = ["version", "server_sha1"].map(|k| lock["minecraft"][k].clone());
            pins.iter().all(|v| v.is_string()).then_some(pins)
        })
    };
    let Some(pins) = server(target) else {
        return false;
    };
    let reports = |dir: &Path| dir.join(".local/datagen/out/reports");
    let mut sources = vec![reports(target)];
    let mut source_claims = Vec::new();
    for other in installed_in(versions)
        .into_iter()
        .filter(|t| t != tag && resources_owner_in(versions, t) == *t)
    {
        let dir = versions.join(&other);
        if server(&dir).as_ref() == Some(&pins) {
            if let Ok(preparation) = prepare_lock_at(&dir, &other) {
                sources.push(resources_at(&dir));
                sources.push(reports(&dir));
                source_claims.push(preparation);
            }
        }
    }
    let mut complete = true;
    for name in missing {
        let Some(from) = sources.iter().map(|s| s.join(name)).find(|p| p.is_file()) else {
            complete = false;
            continue;
        };
        // Copied beside it and renamed: an interrupted copy never looks complete.
        // The name is this call's own, as the window's start and Play can both complete one version.
        static COPIES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = COPIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let partial = to.join(format!(".{name}.partial-{}-{n}", std::process::id()));
        let copied = Resources::invalidate(target)
            .and_then(|_| fs::copy(&from, &partial))
            .and_then(|_| fs::rename(&partial, to.join(name)));
        if copied.is_err() {
            let _ = fs::remove_file(&partial);
            complete = false;
        }
    }
    complete
}

/// Checks what the preparation needs and picks how to run it. PLAY.txt describes the same
/// steps: the pinned Minecraft files assembled in Sunrise's own store (`data_dir()/minecraft`),
/// the server jar from that version's own json, and Java. `explicit_minecraft` is set only by the
/// CLI's `prepare --minecraft <folder>` developer route. Returns the command and Sunrise's own
/// Minecraft folder to prepare from.
pub fn preparer(
    tag: &str,
    explicit_minecraft: Option<&Path>,
    log: &dyn Fn(String),
    preparation: &crate::preparer::Preparation,
) -> Result<(Command, PathBuf), String> {
    preparer_staged(tag, explicit_minecraft, log, &preparation.handoff(None))
}

pub fn preparer_cancellable(
    tag: &str,
    explicit_minecraft: Option<&Path>,
    log: &dyn Fn(String),
    preparation: &crate::preparer::Preparation,
    stop: &crate::native_mods::Cancellation,
) -> Result<(Command, PathBuf), String> {
    preparer_staged(
        tag,
        explicit_minecraft,
        log,
        &preparation.handoff_cancellable(None, stop),
    )
}

pub(crate) fn preparer_staged(
    tag: &str,
    explicit_minecraft: Option<&Path>,
    log: &dyn Fn(String),
    handoff: &crate::preparer::Handoff<'_>,
) -> Result<(Command, PathBuf), String> {
    let stop = handoff.stop();
    if let Some(stop) = stop {
        stop.check()?;
    }
    let cancel = stop.map_or(&mojang::CANCEL, |stop| stop.flag());
    set_prepare_step(PrepareStep::Files);
    // A native build reads the resources of its base, so it is the base that gets prepared.
    let owner = installed_resources_owner(tag)?;
    let tag = owner.as_str();
    let version = game_version(tag);
    let dir = version_dir(tag);
    let resources = resources_for_game(&dir, &version);
    if read_json(&resources.join(RESOURCES_MANIFEST)).is_some_and(|m| server_profile(&m)) {
        return Err(server_only_resources());
    }
    output_version_at(&dir).map_err(|error| preparer_source_error(&error))?;
    let binary = crate::preparer::binary(&dir).ok_or_else(|| crate::preparer::too_old(tag))?;
    // The version's own answer: a sibling's verified import is linked in when found, and whether its exporters ship.
    let needs = crate::preparer::needs(&binary, &dir, log, Some(handoff))?
        .ok_or_else(|| crate::preparer::too_old(tag))?;
    if let Some(stop) = stop {
        stop.check()?;
    }
    let reused = !needs.server_jar;
    let dev_offline = crate::developer_offline::enabled(tag);
    let chosen = explicit_minecraft
        .map(Path::to_path_buf)
        .or_else(|| Settings::load().minecraft_dir)
        .unwrap_or_else(|| crate::preparer::minecraft_dir(None));
    // Always assemble into Sunrise's own folder from the pinned version file, even when the
    // import is reused (so sounds and assets come from a complete pinned store).
    let minecraft = {
        let pins = read_json(&dir.join("research-lock.json"))
            .and_then(|lock| sources::Pins::read(&lock))
            .ok_or_else(|| {
                crate::i18n::tr("The version's research-lock.json doesn't pin the Minecraft files")
                    .to_string()
            })?;
        let own = data_dir().join("minecraft");
        sources::remove_legacy_linked(&data_dir().join("linked-minecraft"));
        let candidates = candidate_sources(Some(&chosen), &own);
        let local_sources = match explicit_minecraft {
            Some(explicit) if !dev_offline => sources::candidate_bases(Some(explicit), &[], &own),
            _ => candidates.clone(),
        };
        let store = sources::Store {
            dir: &own,
            sources: &candidates,
        };
        let options = PrepareStoreOptions {
            os: mojang_os(),
            reused,
            allow_local_without_owner: explicit_minecraft.is_some() || dev_offline,
            local_sources: &local_sources,
        };
        let fetch = |url: &str| fetch_bytes_cancellable(url, stop);
        prepare_store(&pins, &store, &options, &fetch, cancel, log)?
    };
    if let Some(stop) = stop {
        stop.check()?;
    }
    let manifest = minecraft
        .join("versions")
        .join(&version)
        .join(format!("{version}.json"));
    // The Java the version pins, found as sunrise-prepare finds it, so a missing one is
    // reported (or downloaded) before the server jar is downloaded. Versions from before
    // java_major ran Java 25.
    let major = read_json(&dir.join("research-lock.json"))
        .and_then(|lock| lock["minecraft"]["java_major"].as_i64())
        .unwrap_or(25);
    let mut games = vec![chosen, minecraft.clone(), data_dir().join("minecraft")];
    games.extend(
        sources::Folders::current()
            .map(|f| sources::known_folders(sources::Os::current(), &f))
            .unwrap_or_default()
            .into_iter()
            .map(|(_, dir)| dir),
    );
    games.extend(
        sources::Folders::current()
            .map(|f| sources::java_only_folders(sources::Os::current(), &f))
            .unwrap_or_default(),
    );
    set_prepare_step(PrepareStep::Java);
    let java_home = java(major, needs.jdk, &games, &manifest, stop, log)?;
    if let Some(stop) = stop {
        stop.check()?;
    }
    let jar = data_dir()
        .join("server-jars")
        .join(format!("{version}.jar"));
    if !reused {
        let manifest_text = fs::read_to_string(&manifest).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                sources::missing_version_file(&manifest) + &sources::prepare_advice()
            } else {
                e.to_string()
            }
        })?;
        let json = serde_json::from_str(&manifest_text).map_err(|e| e.to_string())?;
        let fetch = |url: &str| fetch_bytes_cancellable(url, stop);
        ensure_server_jar(&jar, &json, &fetch, log)?;
    }
    if let Some(stop) = stop {
        stop.check()?;
    }
    let mut cmd = Command::new(binary);
    with_java(&mut cmd, &java_home);
    cmd.current_dir(&dir).arg("--root").arg(&dir);
    if !reused {
        cmd.args(["--prepare", "--server-jar"]).arg(&jar);
    }
    cmd.arg("--minecraft-dir").arg(&minecraft);
    // This also precedes the sounds pass shared by GUI, CLI and staged Prepare.
    Resources::invalidate(&dir).map_err(|e| e.to_string())?;
    Ok((cmd, minecraft))
}

pub(in crate::backend) struct PrepareStoreOptions<'a> {
    pub os: &'a str,
    pub reused: bool,
    /// Only true for the developer route (`prepare --minecraft <folder>` or an eligible unsigned
    /// developer build with `SUNRISE_LAUNCHER_DEVELOPER_OFFLINE=1`): ordinary players who are not
    /// signed in never borrow another launcher's install.
    pub allow_local_without_owner: bool,
    /// Candidate folders for local-only developer assembly when no verified owner is signed in.
    pub local_sources: &'a [PathBuf],
}

/// Assembles the pinned Minecraft files into `store.dir`: for a signed-in verified owner, copies
/// matching files from `store.sources` and downloads the rest from Mojang; without a verified
/// owner, requires sign-in unless the developer route (`allow_local_without_owner`) is active.
fn prepare_store(
    pins: &sources::Pins,
    store: &sources::Store<'_>,
    options: &PrepareStoreOptions<'_>,
    fetch: sources::Fetch,
    cancel: &std::sync::atomic::AtomicBool,
    log: &dyn Fn(String),
) -> Result<PathBuf, String> {
    prepare_store_with(
        pins,
        store,
        options,
        cancel,
        &|| download_for_owner(pins, store, fetch, Some(cancel), log),
        log,
    )
}

pub(in crate::backend) fn prepare_store_with(
    pins: &sources::Pins,
    store: &sources::Store<'_>,
    options: &PrepareStoreOptions<'_>,
    cancel: &std::sync::atomic::AtomicBool,
    owner_download: &dyn Fn() -> Result<(), OwnerDownloadError>,
    log: &dyn Fn(String),
) -> Result<PathBuf, String> {
    match owner_download() {
        Ok(()) => Ok(store.dir.to_path_buf()),
        Err(e) if e.verified_owner || !options.allow_local_without_owner => {
            Err(e.message.trim_start_matches("\n  • ").to_string())
        }
        Err(e) => {
            let local = sources::Store {
                dir: store.dir,
                sources: options.local_sources,
            };
            match mojang::assemble_local(pins, &local, options.os, cancel) {
                Ok(()) => Ok(store.dir.to_path_buf()),
                Err(why) if options.reused => {
                    log(format!(
                        "Minecraft {}'s assets were not found in any launcher folder: the game \
                         will be silent and the title screen plain ({why})",
                        pins.version
                    ));
                    Ok(if store.dir.is_dir() {
                        store.dir.to_path_buf()
                    } else {
                        options
                            .local_sources
                            .first()
                            .cloned()
                            .unwrap_or_else(|| store.dir.to_path_buf())
                    })
                }
                Err(why) => Err(format!("{why}{}{}", sources::prepare_advice(), e.message)),
            }
        }
    }
}

/// Complete resources and publish their managed-Play snapshot while Prepare still holds its lock.
pub fn finish_preparation(
    tag: &str,
    preparation: &crate::preparer::Preparation,
) -> Result<(), String> {
    finish_preparation_cancellable(tag, preparation, None)
}

pub(in crate::backend) fn finish_preparation_cancellable(
    tag: &str,
    preparation: &crate::preparer::Preparation,
    stop: Option<&crate::native_mods::Cancellation>,
) -> Result<(), String> {
    Resources::prepare_cancellable(&resources_owner(tag), preparation, None, stop).map(drop)
}

pub(in crate::backend) fn finish_preparation_with_missing(
    tag: &str,
    preparation: &crate::preparer::Preparation,
    missing: Option<crate::preparer::Extras>,
) -> Result<(), String> {
    Resources::prepare_with_missing(&resources_owner(tag), preparation, None, missing).map(drop)
}
