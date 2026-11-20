//! Publish complete native installations in one no-replace move.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn install(
    base: &str,
    version: &str,
    target: Target,
    binary: &Path,
    libraries: &[PathBuf],
    about: &serde_json::Value,
    mods: &[Bundle],
    cancellation: &Cancellation,
) -> Result<(), String> {
    let _publication = crate::run_leases::installation().map_err(cache_error)?;
    let from = backend::version_dir(base);
    let dir = target.directory(version);
    let root = dir.parent().expect("build collection");
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let stage = tempfile::Builder::new()
        .prefix(".native-stage-")
        .tempdir_in(root)
        .map_err(|e| e.to_string())?;
    let partial = stage.path().join("image");
    fs::create_dir(&partial).map_err(|e| e.to_string())?;
    copy_base(&from, &partial, target, target == Target::Client)?;
    let installed = partial.join(target.binary());
    fs::copy(binary, &installed).map_err(|e| format!("The built game: {e}"))?;
    // The prebuilt game's libraries load from beside the executable.
    for library in libraries {
        let name = library.file_name().ok_or("invalid library path")?;
        fs::copy(library, partial.join(name)).map_err(|e| format!("{}: {e}", library.display()))?;
    }
    fs::write(partial.join("native-mods.json"), about.to_string()).map_err(|e| e.to_string())?;
    for bundle in mods {
        copy_bundle(
            bundle,
            &partial
                .join(sunrise_launcher_smod::mod_list::NATIVE_DIR)
                .join(&bundle.manifest.id),
        )?;
    }
    cancellation.check()?;
    crate::run_leases::publish_build(&partial, &dir).map_err(cache_error)
}
