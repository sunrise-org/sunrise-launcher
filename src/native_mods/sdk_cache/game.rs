//! Install the two signed SDK slices that make up a playable game without a Rust toolchain.
use super::*;
use crate::backend::InstallPhase;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GameSlices {
    pub(crate) index: String,
    commit: String,
    slices: BTreeMap<String, String>,
}

/// The game release must sign both `sdk.json` (listing both client slices for this target) and
/// `sunrise-release.json`, matching the exact proof required by `GameDownload::prove_with`.
pub(crate) fn offers_game(
    version: &str,
    source: &str,
    triple: &str,
    base: &Base,
    keys: &[PublicKey],
) -> bool {
    check_game_offer(version, source, triple, base, keys).unwrap_or(false)
}

/// Checks whether the game release signs an index listing both client slices for this target.
/// Returns `Ok(true)` when both slices are listed, `Ok(false)` when the verified index omits a
/// slice (or the build tag does not match its channel), and `Err(Refusal)` when fetching or
/// verifying the index fails so explicit installs can report the actual failure.
pub(crate) fn check_game_offer(
    version: &str,
    source: &str,
    triple: &str,
    base: &Base,
    keys: &[PublicKey],
) -> Result<bool, Refusal> {
    let channel = sunrise_launcher_release_verify::Channel::of(source);
    if channel.is_some_and(|channel| !sunrise_launcher_release_verify::is_build(channel, version)) {
        return Ok(false);
    }
    let fetch_error = std::cell::RefCell::new(None);
    let proof = crate::backend::installation::prove_release_files(
        source,
        version,
        INDEX,
        true,
        keys,
        |name| match base.release_file(source, version, name) {
            Ok(bytes) => Ok(bytes),
            Err(error) => {
                let text = error.to_string();
                let mut slot = fetch_error.borrow_mut();
                if slot.is_none() {
                    *slot = Some(error);
                }
                Err(text)
            }
        },
    )
    .map_err(|why| {
        fetch_error
            .into_inner()
            .unwrap_or_else(|| Refusal::Refused {
                build: version.to_owned(),
                why,
            })
    })?;
    let Some(bytes) = base.release_file(source, version, INDEX)? else {
        return Err(Refusal::Host);
    };
    sunrise_launcher_release_verify::check(INDEX, proof.archive(), &sha256(&bytes)).map_err(
        |why| Refusal::Refused {
            build: version.to_owned(),
            why,
        },
    )?;
    let commit = game_commit(version, channel, &bytes)?;
    let index = checked_game_index(version, &bytes, &commit)?;
    Ok(game_slices(triple)
        .iter()
        .all(|(_, name)| index.asset(name).is_ok()))
}

fn checked_game_index(version: &str, bytes: &[u8], commit: &str) -> Result<Index, Refusal> {
    Index::read(bytes, commit).map_err(|error| Refusal::Refused {
        build: version.to_owned(),
        why: error.to_string(),
    })
}

fn game_slices(triple: &str) -> [(Kind, String); 2] {
    [Kind::Runtime, Kind::Programs].map(|kind| (kind, Asset::name(kind, triple, Side::Client)))
}

/// `game_index` and its digest come from the signed game release, not from the SDK host.
pub(crate) fn stage_game(
    stage: &Path,
    version: &str,
    triple: &str,
    (game_index, digest): (&[u8], &str),
    (base, keys): (&Base, &[PublicKey]),
    (stop, progress): (
        Option<&crate::native_mods::Cancellation>,
        &dyn Fn(InstallPhase),
    ),
) -> Result<(PathBuf, GameSlices), Refusal> {
    if let Some(stop) = stop {
        stop.check().map_err(|why| Refusal::Refused {
            build: version.to_owned(),
            why,
        })?;
    }
    let commit = game_commit(
        version,
        sunrise_launcher_release_verify::Channel::of(version),
        game_index,
    )?;
    let index = signed_sdk_index(base, &commit, keys)?;
    let refused = |why| Refusal::Refused {
        build: version.to_owned(),
        why,
    };
    let sha256 = sha256(&index);
    sunrise_launcher_release_verify::check(INDEX, digest, &sha256).map_err(refused)?;
    let sdk = Sdk {
        index: checked_game_index(version, &index, &commit)?,
        sha256,
        dir: stage.to_owned(),
    };
    let mut receipt = GameSlices {
        index: sdk.sha256.clone(),
        commit,
        slices: BTreeMap::new(),
    };
    let wanted = game_slices(triple).map(|(kind, name)| (name, stage.join(kind.name())));
    let mut fetched = Fetched::default();
    for (name, _) in &wanted {
        let asset = sdk.index.asset(name).map_err(|_| Refusal::Host)?;
        fetched.total = fetched.total.saturating_add(asset.size);
        receipt.slices.insert(name.clone(), asset.sha256.clone());
    }
    let report = |phase| match phase {
        SliceProgress::Downloading(bytes) if bytes.total > 0 => progress(
            InstallPhase::Downloading((bytes.done as f32 / bytes.total as f32).min(1.0)),
        ),
        SliceProgress::Downloading(_) => {}
        SliceProgress::Unpacking => progress(InstallPhase::Unpacking),
    };
    // Always unpack into the private installation stage. SDK cache stamps check sizes, not
    // file hashes, and are not proof of the bytes in a newly installed game.
    for (name, into) in &wanted {
        fetch_into_cancellable(&sdk, base, name, into, &mut fetched, stop, &report)?;
    }
    progress(InstallPhase::Finishing);
    let [(_, runtime), (_, programs)] = wanted;
    let bin = runtime.join("bin");
    let io = |error: std::io::Error| Refusal::Cache(error.to_string());
    // The executable and std are already together in bin/. Move the game libraries there
    // too: ELF's $ORIGIN, Mach-O's @executable_path and Windows all search beside the program.
    for (folder, libraries_only) in [(runtime.join("deps"), true), (programs, false)] {
        for entry in fs::read_dir(&folder).map_err(io)? {
            let entry = entry.map_err(io)?;
            let name = entry.file_name();
            if libraries_only
                && (!entry.file_type().map_err(io)?.is_file()
                    || !loadable_library(&name.to_string_lossy(), triple))
            {
                continue;
            }
            let to = bin.join(name);
            if to.try_exists().map_err(io)? {
                return Err(io(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    to.display().to_string(),
                )));
            }
            fs::rename(entry.path(), to).map_err(io)?;
        }
    }
    if !bin.join(crate::preparer::BINARY).is_file() {
        return Err(refused(
            sdk::Refusal::Missing(vec![crate::preparer::BINARY.into()]).to_string(),
        ));
    }
    Ok((bin, receipt))
}

fn loadable_library(name: &str, triple: &str) -> bool {
    let name = name.to_ascii_lowercase();
    match triple {
        crate::rust_toolchain::WINDOWS_X64_TARGET => name.ends_with(".dll"),
        "aarch64-apple-darwin" => name.ends_with(".dylib"),
        _ => {
            name.ends_with(".so")
                || name.split_once(".so.").is_some_and(|(_, version)| {
                    version
                        .split('.')
                        .all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                })
        }
    }
}
