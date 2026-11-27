//! Resolve the mixins tool pins of native projects before handing them to an SDK's tool.
use super::{sdk_cache, Bundle, Failed, MIXINS_REV, NATIVE_MANIFEST};
use crate::i18n::trf;
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::Path,
};

/// The revisions of the native mods in order. An explicit pin never needs the mod's SDK;
/// otherwise read the SDK of the mod's pin, not necessarily the version being built.
pub(super) fn pins(
    mods: &[Bundle],
    build_sdk: Option<&sdk_cache::Sdk>,
    base: &sdk_cache::Base,
    local: Option<&Path>,
) -> Result<Vec<String>, Failed> {
    let keys = sunrise_launcher_release_verify::sdk_keys_for(crate::backend::GAME_REPO);
    read_pins(mods, build_sdk, local, |pin| {
        Ok(sdk_cache::index(base, pin, &keys)?)
    })
    .map(|(pins, _)| pins)
}

/// The SDK indexes a set of pins was read from: what a build made with those pins depends on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Read {
    /// The SHA-256 of each signed index read, by build commit.
    pub(super) sdk: BTreeMap<String, String>,
    /// The SHA-256 of the developer SDK's index, when a pin was read from it.
    pub(super) local: Option<String>,
}

/// [`pins`], with each index taken from `index` (the verified one, or only the one kept on disk)
/// and the digests of those read.
pub(super) fn read_pins(
    mods: &[Bundle],
    build_sdk: Option<&sdk_cache::Sdk>,
    local: Option<&Path>,
    mut index: impl FnMut(&str) -> Result<sdk_cache::Sdk, Failed>,
) -> Result<(Vec<String>, Read), Failed> {
    let mut read = Read::default();
    let pins = resolve(mods, |pin| {
        if let Some(sdk) = build_sdk.filter(|sdk| sdk.commit() == pin) {
            read.sdk.insert(pin.to_owned(), sdk.signed().to_owned());
            return Ok(sdk.index.mixins.rev.clone());
        }
        if let Some(local) = local {
            let path = local.join(sunrise_launcher_sdk::INDEX);
            match fs::read(&path) {
                Ok(bytes) => {
                    let index = sunrise_launcher_sdk::Index::read(&bytes, pin)
                        .map_err(|e| format!("{}: {e}", path.display()))?;
                    read.local = Some(sunrise_launcher_release_verify::sha256_hex(&bytes));
                    return Ok(index.mixins.rev);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("{}: {e}", path.display()).into()),
            }
        }
        let sdk = index(pin)?;
        read.sdk.insert(pin.to_owned(), sdk.signed().to_owned());
        Ok(sdk.index.mixins.rev)
    })?;
    Ok((pins, read))
}

fn resolve(
    mods: &[Bundle],
    mut sdk_revision: impl FnMut(&str) -> Result<String, Failed>,
) -> Result<Vec<String>, Failed> {
    let mut by_build = HashMap::<&str, String>::new();
    mods.iter()
        .map(|bundle| {
            let native = bundle.native();
            if let Some(pin) = &native.mixins {
                return Ok(pin.clone());
            }
            match by_build.entry(&native.sunrise) {
                std::collections::hash_map::Entry::Occupied(pin) => Ok(pin.get().clone()),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    // The game's mirrored index reader only parses the revision. Before
                    // inheriting it here, require the same exact SHA as for an explicit pin.
                    let pin = sdk_revision(entry.key())
                        .and_then(|rev| {
                            if super::bundle::is_revision(&rev) {
                                Ok(rev)
                            } else {
                                Err(format!("sdk.json mixins.rev {rev:?} is not a 40-digit lowercase hex commit").into())
                            }
                        })
                        .map_err(|e| match e {
                            Failed::Sdk { refusal, .. } => Failed::Sdk {
                                refusal,
                                mod_id: Some(bundle.manifest.id.clone()),
                            },
                            other => trf(
                                "{id}: couldn't find the mixin tool version this mod needs: {error}",
                                &[("id", &bundle.manifest.id), ("error", &other.to_string())],
                            )
                            .into(),
                        })?;
                    Ok(entry.insert(pin).clone())
                }
            }
        })
        .collect()
}

/// Keep the launcher's existing reconciliation for different pins. A lone pin always wins;
/// otherwise take the newest of the pins and the launcher's own tool by commit time.
pub(super) fn for_key(
    pins: &[String],
    date: impl Fn(&str) -> Result<String, String>,
) -> Result<String, String> {
    let mut pins: Vec<&str> = pins.iter().map(String::as_str).collect();
    pins.sort_unstable();
    pins.dedup();
    match pins.as_slice() {
        [] => Ok(MIXINS_REV.into()),
        [one] => Ok((*one).into()),
        _ => {
            if !pins.contains(&MIXINS_REV) {
                pins.push(MIXINS_REV);
            }
            pins.into_iter()
                .map(|pin| date(pin).map(|when| (when, pin)))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| {
                    trf(
                        "Couldn't tell which version of the mixin tool is newest: {error}",
                        &[("error", &e.to_string())],
                    )
                })
                .map(|dated| {
                    dated
                        .into_iter()
                        .max()
                        .expect("different pins")
                        .1
                        .to_owned()
                })
        }
    }
}

/// Write an inferred pin only on a launcher-owned copy. Older SDK tools require the field;
/// the source or downloaded archive and its digest stay as authored. Writing the same pin again
/// leaves the copy as it is.
pub(super) fn materialize(bundle: &Bundle, dir: &Path, revision: &str) -> Result<(), String> {
    if bundle.native().mixins.is_some() {
        return Ok(());
    }
    let path = dir.join(NATIVE_MANIFEST);
    let error = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let text = fs::read_to_string(&path).map_err(|e| error(&e))?;
    let mut doc: toml::Table = text.parse().map_err(|e| error(&e))?;
    doc.get_mut("native")
        .and_then(toml::Value::as_table_mut)
        .expect("validated native table")
        .insert("mixins".into(), revision.into());
    fs::write(&path, toml::to_string(&doc).map_err(|e| error(&e))?).map_err(|e| error(&e))
}

/// A private check folder, so read-only planning never changes the caller's inputs. Copy only
/// the active mods on every reduced retry, including those with already resolved pins.
pub(super) fn check_inputs(mods: &[Bundle], pins: &[String]) -> Result<tempfile::TempDir, String> {
    let root = super::root();
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let scratch = tempfile::Builder::new()
        .prefix("mixins-check-")
        .tempdir_in(root)
        .map_err(|e| e.to_string())?;
    for (i, (bundle, pin)) in mods.iter().zip(pins).enumerate() {
        let dir = scratch.path().join(format!("{i:08}"));
        super::copy_bundle(bundle, &dir)?;
        materialize(bundle, &dir, pin)?;
    }
    Ok(scratch)
}

#[cfg(test)]
mod tests;
