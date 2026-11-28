//! Deleting an instance: its folder to the Trash, its choices and selection forgotten.
use super::*;

/// Moves an instance's folder to the Trash (where the player can get it back, worlds and all)
/// and forgets it: its choices, and the selection when Play started it. Sunrise's own instance,
/// the game's home, stays.
pub fn delete(query: &str) -> Result<Instance, String> {
    let (gone, saved) = delete_with_selection(query)?;
    saved?;
    Ok(gone)
}

/// A completed deletion stays completed even if saving the new selection fails.
pub fn delete_with_selection(query: &str) -> Result<(Instance, Result<(), String>), String> {
    let game_home = backend::game_home();
    let data = backend::data_dir();
    register_profile_names(&list_with_snapshot(&game_home, &data, false, || {}));
    let gone = delete_in(&game_home, &data, query, &crate::desktop_seams::to_trash)
        .map_err(|error| error.to_string())?;
    let mut settings = backend::Settings::load();
    let saved = if clear_deleted_selection(&mut settings, &gone.id) {
        settings.save()
    } else {
        Ok(())
    };
    Ok((gone, saved))
}

/// Takes the same fallback as the command, without overwriting other live window settings.
pub fn clear_deleted_selection(settings: &mut backend::Settings, id: &str) -> bool {
    settings
        .instance
        .take_if(|selected| selected == id)
        .is_some()
}

/// Move the exact folder shown in the window's confirmation, not a fresh lookup by slug.
/// The caller saves its selection once the move settles.
pub fn trash_confirmed(instance: Instance) -> Result<Instance, DeleteError> {
    register_profile(&instance);
    delete_selected(
        &backend::game_home(),
        &backend::data_dir(),
        instance,
        &crate::desktop_seams::to_trash,
    )
}

#[derive(Debug)]
pub struct DeleteError {
    pub detail: String,
    /// Set when the folder left its original path but could not reach the Trash.
    pub retained: Option<InterruptedDeletion>,
}

impl From<String> for DeleteError {
    fn from(detail: String) -> Self {
        Self {
            detail,
            retained: None,
        }
    }
}

impl std::fmt::Display for DeleteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(path) = &self.retained {
            let folder = path.folder().to_string_lossy();
            write!(
                f,
                "{}: {}",
                crate::i18n::trf("The profile was kept in {folder}", &[("folder", &folder)]),
                self.detail
            )
        } else {
            f.write_str(&self.detail)
        }
    }
}

pub(super) fn delete_in(
    game_home: &Path,
    data: &Path,
    query: &str,
    trash: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<Instance, DeleteError> {
    let instance = find_in(&list_with_snapshot(game_home, data, false, || {}), query)?;
    delete_selected(game_home, data, instance, trash)
}

pub(super) fn delete_selected(
    game_home: &Path,
    data: &Path,
    instance: Instance,
    trash: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<Instance, DeleteError> {
    if instance.kind == Kind::Pack {
        return Err(registry_pack::remove_unavailable().into());
    }
    if instance.kind == Kind::Default {
        return Err(crate::i18n::trf(
            "{name} is the game's own folder: it can't be deleted.",
            &[("name", &instance.name)],
        )
        .into());
    }
    delete_selected_at(
        game_home,
        data,
        instance,
        durable::move_new_directory,
        trash,
    )
}

pub(super) fn delete_selected_at(
    game_home: &Path,
    data: &Path,
    instance: Instance,
    detach: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
    trash: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<Instance, DeleteError> {
    let work = &instance.work;
    let expected = instance.incarnation.ok_or_else(lifecycle::changed)?;
    let profile =
        ManagedProfile::open_instance(game_home, data, &instance).map_err(String::from)?;
    let problem = |error: &dyn std::fmt::Display| {
        crate::i18n::trf(
            "{name} could not go to the Trash: {error}",
            &[("name", &instance.name), ("error", &error.to_string())],
        )
    };
    let root = game_home.canonicalize().map_err(|e| problem(&e))?;
    let retired = creation_parent(&root, RETIRED).map_err(|e| problem(&e))?;
    let private = tempfile::tempdir_in(&retired).map_err(|e| problem(&e))?;
    let gone = private
        .path()
        .join(work.file_name().ok_or_else(lifecycle::changed)?);
    let recovery = InterruptedDeletion::record(&retired, gone.clone(), instance.id.clone())
        .map_err(|e| problem(&e))?;
    // From this point neither an ancillary error nor a destructor may empty the detached
    // folder. If Trash is unavailable, preserve the whole profile in the recovery folder.
    let _ = private.keep();
    let detached = (|| {
        let _references = ReferencePublication::begin_at(data, game_home, profile.home())?;
        Store::mutate(data, |store| {
            detach(work, &gone).map_err(|e| problem(&e))?;
            if let Err(changed) = expected.at_detachment(&instance, &gone) {
                // The name still belongs to this operation, but an external writer may
                // have filled it. Never restore by overwriting that directory.
                return match durable::move_new_directory(&gone, work) {
                    Ok(()) => Err(changed),
                    Err(error) => Err(crate::i18n::trf(
                        "The profile changed and could not be restored: {error}.",
                        &[("error", &error.to_string())],
                    )),
                };
            }
            store.instances.remove(&instance.id);
            Ok(())
        })
    })();
    let trashed = detached.and_then(|()| trash(&gone));
    let retained = recovery.retained();
    trashed.map_err(|detail| DeleteError { retained, detail })?;
    Ok(instance)
}
