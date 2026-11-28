//! Explicitly trusted checkout installations, assembled by the release packager.
use crate::{
    backend,
    i18n::{tr, trf},
};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, process::Command};

pub fn is_developer(version: &str) -> bool {
    version.starts_with("developer-")
}

pub fn uses_developer(version: &str) -> bool {
    is_developer(version)
        || crate::native_mods::about(version).is_some_and(|a| is_developer(&a.base))
        || crate::build_import::about(version).is_some_and(|a| is_developer(&a.base))
}

/// A local build's name, or a test build's ([`crate::draft_releases::label`]).
pub fn label(version: &str) -> Option<String> {
    if !uses_developer(version) {
        return crate::draft_releases::label(version);
    }
    let base = crate::native_mods::about(version)
        .map(|a| a.base)
        .or_else(|| crate::build_import::about(version).map(|a| a.base))
        .unwrap_or_else(|| version.to_owned());
    let commit = revision(&base).unwrap_or_else(|_| tr("Unknown commit").into());
    Some(trf(
        "Local unsigned developer build · {commit} · {id}",
        &[("commit", &commit), ("id", version)],
    ))
}

/// The instance row keeps the trust warning even before resources have been prepared.
pub fn instance_status(version: &str, prepared: bool) -> Option<String> {
    label(version).map(|label| {
        format!(
            "{label} · {}",
            if prepared {
                tr("Ready")
            } else {
                tr("Needs preparing")
            }
        )
    })
}

pub fn revision(version: &str) -> Result<String, String> {
    revision_in(&backend::version_dir(version))
}

pub(crate) fn revision_in(directory: &Path) -> Result<String, String> {
    let record = fs::read_to_string(directory.join("developer.json")).map_err(|e| e.to_string())?;
    #[derive(serde::Deserialize)]
    struct Record {
        revision: String,
    }
    let record: Record = serde_json::from_str(&record).map_err(|e| e.to_string())?;
    if record.revision.len() != 40 || !record.revision.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(tr("Invalid developer build revision.").into());
    }
    Ok(record.revision)
}

// Do not follow symlinks: only the regular files the packager produced are installable.
fn digest_tree(root: &Path, hash: &mut Sha256) -> Result<(), String> {
    let mut entries = fs::read_dir(root)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| tr("Developer package contains an invalid file name.").to_string())?;
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        if kind.is_dir() {
            hash.update(b"directory");
            digest_tree(&entry.path(), hash)?;
            hash.update(b"end");
        } else if kind.is_file() {
            let bytes = fs::read(entry.path()).map_err(|e| e.to_string())?;
            hash.update(b"file");
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        } else {
            return Err(
                tr("Developer package must contain only regular files and directories.").into(),
            );
        }
    }
    Ok(())
}

/// Where a checkout keeps the resources its own preparation made, one folder per Minecraft version.
const CHECKOUT_RESOURCES: &str = ".local/sunrise-client";

/// Names the folder [`copy_prepared`] builds beside a base; one call's own, as an install and a
/// started game can both copy.
static PARTIAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn plain_directory_exists(path: &Path) -> std::io::Result<bool> {
    use std::io::ErrorKind;
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !crate::native_cache::linked(&meta) => Ok(true),
        Ok(_) => Err(ErrorKind::InvalidData.into()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// The old directory stays recoverable until the new directory is visible at its final path.
fn publish_prepared(staged: &Path, to: &Path, previous: &Path) -> std::io::Result<()> {
    use crate::profile_txn::durable::move_new_directory;
    let saved = plain_directory_exists(to)?;
    let publish = (|| {
        if saved {
            move_new_directory(to, previous)?;
        }
        move_new_directory(staged, to)
    })();
    if let Err(error) = publish {
        // The rename helper can fail syncing the parent after either move has happened.
        // Restore only if the destination really is missing; otherwise keep the backup.
        let restore = (|| {
            if saved && !plain_directory_exists(to)? && plain_directory_exists(previous)? {
                move_new_directory(previous, to)?;
            }
            Ok::<(), std::io::Error>(())
        })();
        return match restore {
            Ok(()) => Err(error),
            Err(restore) => Err(std::io::Error::other(format!("{error}; {restore}"))),
        };
    }
    if saved {
        fs::remove_dir_all(previous)?;
    }
    Ok(())
}

/// Copy current checkout resources into a missing or stale full folder. A copy, not a link:
/// preparing again must not change the checkout. Never replace a server-only preparation.
fn copy_prepared(root: &Path, from: &Path, to: &Path) -> std::io::Result<bool> {
    use crate::profile_txn::durable;
    let version = to.file_name().unwrap_or_default().to_string_lossy();
    let previous = to.with_file_name(format!(".{version}.checkout-previous"));
    // The preparation lock excludes both Play's preparation and another developer import.
    // Finish or undo the previous namespace change before deciding if this copy is needed.
    if plain_directory_exists(&previous)? {
        if !plain_directory_exists(to)? {
            durable::move_new_directory(&previous, to)?;
        } else if backend::check_play_resources(root, to).is_ok() {
            fs::remove_dir_all(&previous)?;
        } else {
            // A new tree appeared after the interruption; neither tree can be discarded.
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                previous.display().to_string(),
            ));
        }
    }
    let replaceable = || match backend::check_play_resources(root, to) {
        Err(backend::Unprepared::Incomplete) => Ok(true),
        Ok(()) | Err(backend::Unprepared::ServerOnly) => Ok(false),
        Err(issue) => Err(std::io::Error::other(issue.message(String::new))),
    };
    if !replaceable()? || backend::check_play_resources(root, from).is_err() {
        return Ok(false);
    }
    // Without the data file: it names its own root, so a copy of it could not be read.
    let data_file = from.join(crate::preparer::DATA_FILE);
    let partial = to.with_file_name(format!(
        ".{version}.partial-{}-{}",
        std::process::id(),
        PARTIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&partial);
    let copied = backend::copy_dir_except(from, &partial, Some(&data_file)).and_then(|()| {
        durable::flush_tree(&partial)?;
        if !replaceable()? {
            return Ok(false); // Prepared meanwhile, by Play or another command.
        }
        crate::managed_play::Resources::invalidate(root)?;
        publish_prepared(&partial, to, &previous).map(|()| true)
    });
    let _ = fs::remove_dir_all(&partial);
    copied
}

/// Gives the developer base `tag` the resources the checkout it was built from prepared in its own
/// folder, so the base is ready to play without the official launcher's folder, a signed-in account
/// or a `prepare` that has no Minecraft install to read (#4315). Returns whether it copied them.
pub(crate) fn reuse_checkout_resources(checkout: &Path, tag: &str) -> std::io::Result<bool> {
    let _preparation = backend::prepare_lock(tag).map_err(std::io::Error::other)?;
    let to = backend::resources(tag);
    // The packager ships the checkout's pin, so both name the same Minecraft version.
    let Some(version) = to.file_name() else {
        return Ok(false);
    };
    let from = checkout.join(CHECKOUT_RESOURCES).join(version);
    let copied = copy_prepared(&backend::version_dir(tag), &from, &to)?;
    backend::forget_prepared();
    Ok(copied)
}

pub fn install(checkout: &Path, program: &Path, trusted: bool) -> Result<String, String> {
    if !trusted {
        return Err(tr("Developer installation runs the checkout's build code and an unsigned game with your rights. Pass --trust-unsigned only if you trust both.").into());
    }
    let checkout = checkout.canonicalize().map_err(|e| e.to_string())?;
    let program = program.canonicalize().map_err(|e| e.to_string())?;
    if program.file_name().and_then(|n| n.to_str()) != Some(backend::CLIENT)
        || !crate::build_import::holds_game(&program)
    {
        return Err(tr("Choose a real sunrise-client executable built for this computer.").into());
    }
    let mut missing = Vec::new();
    for name in [
        "sunrise-ctl",
        "sunrise-perf",
        "sunrise-prepare",
        "sunrise-server",
    ] {
        let name = format!("{name}{}", std::env::consts::EXE_SUFFIX);
        if !program.with_file_name(&name).is_file() {
            missing.push(name);
        }
    }
    if !checkout.join("exporters/manifest.json").is_file() {
        missing.push("exporters/manifest.json".into());
    }
    if !missing.is_empty() {
        return Err(trf(
            "Missing developer packaging inputs: {files}. Build the game program (the --program file) and its companions with cargo build -p sunrise-game -p sunrise-client-lib --bin sunrise-ctl -p sunrise-perf --bin sunrise-perf -p sunrise-prepare -p sunrise-server. Run sunrise-prepare compile-exporters --root CHECKOUT, then copy its server, client and manifest.json into CHECKOUT/exporters/.",
            &[("files", &missing.join(", "))],
        ));
    }
    let (revision, snapshot) = committed_snapshot(&checkout)?;
    let _publication =
        crate::run_leases::installation().map_err(crate::native_mods::cache_error)?;
    let root = backend::version_dir("");
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let staging = tempfile::Builder::new()
        .prefix(".developer-")
        .tempdir_in(&root)
        .map_err(|e| e.to_string())?;
    let package = staging.path().join("package");
    let status = Command::new("cargo")
        .current_dir(&checkout)
        .args(["run", "--locked", "-p", "xtask", "--", "package"])
        .arg(&program)
        .arg(&package)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(tr(
            "The checkout's release packager failed; no developer build was installed.",
        )
        .into());
    }
    let exporters_ship = crate::preparer::binary(&package)
        .map(|binary| crate::preparer::needs(&binary, &package, &|_| {}, None))
        .transpose()?
        .flatten()
        .is_some_and(|needs| !needs.jdk);
    if !exporters_ship {
        return Err(tr("The packaged exporters are missing, changed or do not match the checkout sources. Recompile the exporters before installing.").into());
    }
    fs::write(
        package.join("developer.json"),
        serde_json::json!({"revision": revision}).to_string(),
    )
    .map_err(|e| e.to_string())?;
    fs::write(package.join("developer-source.tar"), snapshot).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    digest_tree(&package, &mut hash)?;
    let tag = format!("developer-{:x}", hash.finalize());
    let destination = root.join(&tag);
    if !destination.exists() {
        crate::run_leases::publish(&package).map_err(crate::native_mods::cache_error)?;
        fs::write(package.join(".installed"), "").map_err(|e| e.to_string())?;
        fs::rename(package, destination).map_err(|e| e.to_string())?;
    }
    // A checkout that prepared its own resources hands them over, so the base needs no separate
    // `prepare` and no official launcher folder (#4315). Run again for the same checkout to pick
    // them up after preparing it; the tag has not changed, so nothing else is installed.
    if let Err(error) = reuse_checkout_resources(&checkout, &tag) {
        crate::elog!(
            "{}",
            trf(
                "{tag}: the checkout's prepared resources were not copied: {error}",
                &[("tag", &tag), ("error", &error.to_string())]
            )
        );
    }
    Ok(tag)
}

pub(crate) fn committed_snapshot(checkout: &Path) -> Result<(String, Vec<u8>), String> {
    let revision = Command::new("git")
        .current_dir(checkout)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|e| e.to_string())?;
    if !revision.status.success() {
        return Err(tr("The developer checkout has no HEAD commit.").into());
    }
    let revision = String::from_utf8(revision.stdout)
        .map_err(|e| e.to_string())?
        .trim()
        .to_owned();
    let clean = Command::new("git")
        .current_dir(checkout)
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()
        .map_err(|e| e.to_string())?;
    if !clean.status.success() || !clean.stdout.is_empty() {
        return Err(tr("The developer checkout has dirty tracked inputs. Commit your changes first, then rebuild the game and retry.").into());
    }
    let snapshot = Command::new("git")
        .current_dir(checkout)
        .args([
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.eol=lf",
            "archive",
            "--format=tar",
            "HEAD",
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !snapshot.status.success() {
        return Err(tr("Could not snapshot the developer checkout's committed source.").into());
    }
    Ok((revision, snapshot.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn digest(root: &Path) -> Vec<u8> {
        let mut hash = Sha256::new();
        digest_tree(root, &mut hash).unwrap();
        hash.finalize().to_vec()
    }
    #[test]
    fn dirty_tracked_checkout_is_refused_but_local_commit_is_snapshotted() {
        let folder = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .current_dir(folder.path())
                .args([
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.invalid",
                    "-c",
                    "commit.gpgSign=false",
                    "-c",
                    "tag.gpgSign=false",
                    "-c",
                    "core.hooksPath=/dev/null",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "-q", "--template="]);
        fs::write(folder.path().join("input"), "committed").unwrap();
        git(&["add", "input"]);
        git(&["commit", "-qm", "local only"]);
        let (commit, snapshot) = committed_snapshot(folder.path()).unwrap();
        assert_eq!(commit.len(), 40);
        let output = tempfile::tempdir().unwrap();
        tar::Archive::new(snapshot.as_slice())
            .unpack(output.path())
            .unwrap();
        assert_eq!(
            fs::read_to_string(output.path().join("input")).unwrap(),
            "committed"
        );
        fs::write(folder.path().join("input"), "dirty").unwrap();
        assert!(committed_snapshot(folder.path())
            .unwrap_err()
            .contains("Commit your changes first"));
    }
    /// A checkout with the exporters manifest, and a game program beside the given companions.
    fn packaging_inputs(
        companions: &[&str],
    ) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let folder = tempfile::tempdir().unwrap();
        let checkout = folder.path().join("checkout");
        fs::create_dir_all(checkout.join("exporters")).unwrap();
        fs::write(checkout.join("exporters/manifest.json"), "{}").unwrap();
        let bin = folder.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let magic: &[u8] = if cfg!(windows) {
            b"MZ"
        } else if cfg!(target_os = "macos") {
            &[0xcf, 0xfa, 0xed, 0xfe]
        } else {
            b"ELF"
        };
        let program = bin.join(backend::CLIENT);
        fs::write(
            &program,
            [magic, &sunrise_launcher_smod::game_identity::MARK[..]].concat(),
        )
        .unwrap();
        for name in companions {
            fs::write(
                bin.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)),
                "",
            )
            .unwrap();
        }
        (folder, checkout, program)
    }
    const COMPANIONS: [&str; 4] = [
        "sunrise-ctl",
        "sunrise-perf",
        "sunrise-prepare",
        "sunrise-server",
    ];
    const REMEDY: &str = "Build the game program (the --program file) and its companions with cargo build -p sunrise-game -p sunrise-client-lib --bin sunrise-ctl -p sunrise-perf --bin sunrise-perf -p sunrise-prepare -p sunrise-server. Run sunrise-prepare compile-exporters --root CHECKOUT, then copy its server, client and manifest.json into CHECKOUT/exporters/.";
    #[test]
    fn missing_packaging_inputs_are_listed_before_running_the_packager() {
        let (_folder, checkout, program) = packaging_inputs(&["sunrise-ctl"]);
        let error = install(&checkout, &program, true).unwrap_err();
        let missing = ["sunrise-perf", "sunrise-prepare", "sunrise-server"]
            .map(|name| format!("{name}{}", std::env::consts::EXE_SUFFIX))
            .join(", ");
        assert_eq!(
            error,
            format!("Missing developer packaging inputs: {missing}. {REMEDY}")
        );
    }
    #[test]
    fn all_packaging_inputs_pass_preflight() {
        let (_folder, checkout, program) = packaging_inputs(&COMPANIONS);
        // The checkout has no Git repository, so the install stops before running the packager.
        assert_eq!(
            install(&checkout, &program, true).unwrap_err(),
            "The developer checkout has no HEAD commit."
        );
    }
    #[test]
    fn consent_precedes_checkout_access() {
        let error = install(Path::new("missing"), Path::new("missing"), false).unwrap_err();
        assert!(error.contains("--trust-unsigned"));
    }
    #[test]
    fn digest_covers_names_contents_and_nested_inputs() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        fs::create_dir(root.join("exporters")).unwrap();
        fs::write(root.join("sunrise-client"), b"game A").unwrap();
        fs::write(root.join("exporters/manifest.json"), b"manifest A").unwrap();
        let original = digest(root);
        assert_eq!(original, digest(root));
        fs::write(root.join("sunrise-client"), b"game B").unwrap();
        assert_ne!(original, digest(root));
        fs::write(root.join("sunrise-client"), b"game A").unwrap();
        fs::write(root.join("exporters/manifest.json"), b"manifest B").unwrap();
        assert_ne!(original, digest(root));
        fs::write(root.join("exporters/manifest.json"), b"manifest A").unwrap();
        fs::rename(
            root.join("exporters/manifest.json"),
            root.join("exporters/other.json"),
        )
        .unwrap();
        assert_ne!(original, digest(root));
    }
    #[test]
    #[cfg(unix)]
    fn package_symlinks_are_refused() {
        let folder = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/etc/passwd", folder.path().join("outside")).unwrap();
        assert!(digest_tree(folder.path(), &mut Sha256::new()).is_err());
    }
    /// A prepared folder as `sunrise-prepare` leaves one: the manifest, its tables and the run-time
    /// data file that names the folder it was written for.
    fn prepared_folder(root: &Path, table: &str) {
        fs::create_dir_all(root.join("assets/minecraft")).unwrap();
        fs::write(root.join("sunrise-resources.json"), r#"{"version":1}"#).unwrap();
        fs::write(root.join("registries.json"), table).unwrap();
        fs::write(root.join("assets/minecraft/sounds.json"), "sounds").unwrap();
        fs::write(
            root.join(crate::preparer::DATA_FILE),
            root.display().to_string(),
        )
        .unwrap();
    }
    #[test]
    fn a_checkout_prepared_folder_lands_beside_the_base_without_its_data_file() {
        let folder = tempfile::tempdir().unwrap();
        let from = folder
            .path()
            .join("CHECKOUT")
            .join(CHECKOUT_RESOURCES)
            .join("26.3");
        prepared_folder(&from, "checkout tables");
        let root = folder.path().join("versions/developer-abc");
        let to = root.join(".local/sunrise-client/26.3");
        assert!(copy_prepared(&root, &from, &to).unwrap());
        // What `prepare` counts as prepared: the manifest and its tables (`backend::prepared_at`).
        assert!(backend::prepared_at(&to));
        assert_eq!(
            fs::read_to_string(to.join("registries.json")).unwrap(),
            "checkout tables"
        );
        assert_eq!(
            fs::read_to_string(to.join("assets/minecraft/sounds.json")).unwrap(),
            "sounds"
        );
        // The data file names its own root, so the base writes its own (the game does it on start).
        assert!(!to.join(crate::preparer::DATA_FILE).exists());
        // A copy, not a link: preparing the base again rewrites its files only.
        fs::write(to.join("registries.json"), "reprepared").unwrap();
        assert_eq!(
            fs::read_to_string(from.join("registries.json")).unwrap(),
            "checkout tables"
        );
        assert!(from.join(crate::preparer::DATA_FILE).exists());
    }
    #[test]
    fn developer_reimport_replaces_stale_full_output_but_not_server_output() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().join("versions/developer-abc");
        let lib = root.join("crates/sunrise-prepare/src/lib.rs");
        fs::create_dir_all(lib.parent().unwrap()).unwrap();
        fs::write(lib, "pub const OUTPUT_VERSION: u32 = 8;\n").unwrap();
        let from = folder.path().join("checkout/.local/sunrise-client/26.3");
        let to = root.join(".local/sunrise-client/26.3");
        prepared_folder(&from, "current checkout tables");
        prepared_folder(&to, "stale installed tables");
        let stamp = "sunrise-prepare.json";
        let current = r#"{"output_version":8}"#;
        fs::write(from.join(stamp), current).unwrap();
        fs::write(to.join(stamp), r#"{"output_version":7}"#).unwrap();

        assert!(copy_prepared(&root, &from, &to).unwrap());
        assert_eq!(fs::read_to_string(to.join(stamp)).unwrap(), current);
        assert_eq!(
            fs::read_to_string(to.join("registries.json")).unwrap(),
            "current checkout tables"
        );

        let manifest = to.join("sunrise-resources.json");
        let server = r#"{"profile":"server"}"#;
        fs::write(&manifest, server).unwrap();
        fs::write(to.join(stamp), r#"{"output_version":1}"#).unwrap();
        assert!(!copy_prepared(&root, &from, &to).unwrap());
        assert_eq!(fs::read_to_string(manifest).unwrap(), server);
    }

    #[test]
    fn failed_developer_publication_restores_the_old_resources() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let to = root.join(".local/sunrise-client/26.3");
        let previous = to.with_file_name(".26.3.checkout-previous");
        prepared_folder(&to, "old prepared tables");

        // The staged directory disappeared before publication. The old tree was already
        // renamed aside when the second rename failed, and must be restored as a whole.
        let staged = to.with_file_name(".missing-staged-copy");
        let error = publish_prepared(&staged, &to, &previous).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        assert_eq!(
            fs::read_to_string(to.join("registries.json")).unwrap(),
            "old prepared tables"
        );
        assert!(!previous.exists());

        // A stopped launcher at the same cut leaves the old tree under its recovery name.
        // Re-import restores it before inspecting even an absent checkout.
        crate::profile_txn::durable::move_new_directory(&to, &previous).unwrap();
        assert!(!copy_prepared(root, &staged, &to).unwrap());
        assert_eq!(
            fs::read_to_string(to.join("registries.json")).unwrap(),
            "old prepared tables"
        );
        assert!(!previous.exists());
    }

    #[test]
    fn resources_the_base_gained_meanwhile_are_left_alone() {
        let folder = tempfile::tempdir().unwrap();
        let from = folder
            .path()
            .join("CHECKOUT")
            .join(CHECKOUT_RESOURCES)
            .join("26.3");
        prepared_folder(&from, "checkout tables");
        let root = folder.path().join("versions/developer-abc");
        let to = root.join(".local/sunrise-client/26.3");
        prepared_folder(&to, "the base's own tables");
        assert!(!copy_prepared(&root, &from, &to).unwrap());
        assert_eq!(
            fs::read_to_string(to.join("registries.json")).unwrap(),
            "the base's own tables"
        );
        // No partial copy is left beside it.
        let beside: Vec<_> = fs::read_dir(to.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name != "26.3")
            .collect();
        assert_eq!(beside, Vec::<String>::new());
    }
}
