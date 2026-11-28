//! Editing an instance's choices, and keeping its build choice in step with its profile.
use super::*;

pub(super) fn apply_committed_choice(
    meta: &mut Meta,
    choice: &crate::managed_play::CommittedBuildChoice,
) -> bool {
    if meta.reflects_build_choice(choice.id) {
        return false;
    }
    meta.version = choice.version.clone();
    if let Some(channel) = choice.channel {
        meta.channel = Some(channel);
    }
    meta.note_build_choice(choice.id);
    true
}

pub(super) fn pending_build_choice(
    data: &Path,
    home: &Path,
) -> Option<crate::managed_play::CommittedBuildChoice> {
    crate::managed_play::try_display_active_at(data, home)
        .ok()
        .flatten()
        .and_then(|active| active.build_choice().cloned())
}

/// Persists a switch's already-committed channel or pin choice into `instances.json`.
pub(crate) fn sync_build_choice(
    id: &str,
    home: &Path,
    choice: &crate::managed_play::CommittedBuildChoice,
) -> Result<(), String> {
    Store::mutate(&backend::data_dir(), |store| {
        let meta = store
            .instances
            .entry(id.to_owned())
            .or_insert_with(|| initial_choices(home).unwrap_or_default());
        apply_committed_choice(meta, choice);
        Ok(())
    })
}

/// Changes what the player chose about an instance: its name, icon or group. No folder moves.
pub fn edit(query: &str, change: impl FnOnce(&mut Meta)) -> Result<Instance, String> {
    let game_home = backend::game_home();
    let data = backend::data_dir();
    register_profile_names(&list_in(&game_home, &data));
    let mut proposed_name = None;
    let result = edit_in(&game_home, &data, query, |meta| {
        change(meta);
        proposed_name = meta.name.clone();
    });
    if let Some(name) = proposed_name {
        crate::launcher_log::hide_profile_name(&name);
    }
    if let Ok(instance) = &result {
        register_profile(instance);
    }
    result
}

pub(super) fn edit_in(
    game_home: &Path,
    data: &Path,
    query: &str,
    change: impl FnOnce(&mut Meta),
) -> Result<Instance, String> {
    let all = list_in(game_home, data);
    let instance = find_in(&all, query)?;
    Store::mutate(data, |store| {
        let meta = store
            .instances
            .entry(instance.id.clone())
            .or_insert_with(|| initial_choices(&instance.home).unwrap_or_default());
        if instance.kind != Kind::Pack {
            if let Some(choice) = pending_build_choice(data, &instance.home) {
                apply_committed_choice(meta, &choice);
            }
        }
        change(meta);
        meta.name = meta.name.take().map(|n| n.trim().to_string());
        meta.group = meta
            .group
            .take()
            .map(|g| g.trim().to_string())
            .filter(|g| !g.is_empty());
        if let Some(name) = meta.name.clone() {
            if name.is_empty() {
                return Err(crate::i18n::tr("A profile needs a name.").into());
            }
            // Only a new name is checked: an import can share a name, and changing anything
            // else about it must still work.
            let renamed = !name.eq_ignore_ascii_case(&instance.name);
            if renamed
                && all.iter().any(|i| {
                    i.id != instance.id
                        && store
                            .instances
                            .get(&i.id)
                            .and_then(|m| m.name.as_deref())
                            .unwrap_or(&i.name)
                            .eq_ignore_ascii_case(&name)
                })
            {
                return Err(name_taken(&name));
            }
        }
        Ok(())
    })?;
    find_in(&list_in(game_home, data), &instance.id)
}
