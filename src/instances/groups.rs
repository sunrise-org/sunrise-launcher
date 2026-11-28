//! The sidebar's groups: the order the player gave them, which ones are folded, and renaming or
//! dissolving one. A group has no record of its own: it is the `group` of its profiles, so a
//! group exists while a profile is in it, and these choices are kept beside the profiles' in
//! `instances.json` (`group_order`, `collapsed_groups`, `profiles_folded`). A file without them lists the groups by
//! name with none folded, as before.
use super::*;

/// A saved list that is unreadable is an empty one, and never a reason to lose the instances'
/// own choices beside it.
pub(super) fn tolerant<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned + Default,
{
    Ok(serde_json::from_value(serde_json::Value::deserialize(deserializer)?).unwrap_or_default())
}

/// What the player chose about the groups.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GroupChoices {
    /// The groups moved so far, top first. Others come after them, by name.
    pub order: Vec<String>,
    /// The groups folded under their heading.
    pub collapsed: BTreeSet<String>,
    /// The whole profile list folded under the sidebar's "Profiles" heading.
    pub profiles_folded: bool,
}

/// The saved group choices.
pub fn group_choices() -> GroupChoices {
    group_choices_in(&backend::data_dir())
}

pub(super) fn group_choices_in(data: &Path) -> GroupChoices {
    let store = Store::load(data);
    GroupChoices {
        order: store.group_order,
        collapsed: store.collapsed_groups,
        profiles_folded: store.profiles_folded,
    }
}

/// Where a group sorts: ungrouped profiles first, then the groups the player placed in that order,
/// then the others by name (the sort puts the name after this).
pub(super) fn group_rank(order: &[String], group: Option<&str>) -> (u8, usize) {
    match group {
        None => (0, 0),
        Some(name) => match order.iter().position(|g| g == name) {
            Some(place) => (1, place),
            None => (2, 0),
        },
    }
}

/// The groups of `list` (the order the sidebar shows) top to bottom, each once. The default
/// profile listed first, never moved, heads no group in the sidebar, so a group only it is in
/// isn't one.
pub fn group_names(list: &[Instance]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for (n, instance) in list.iter().enumerate() {
        if n == 0 && instance.kind == Kind::Default {
            continue;
        }
        let group = instance.group.as_deref().filter(|g| !g.trim().is_empty());
        if let Some(group) = group {
            if !names.iter().any(|seen| seen == group) {
                names.push(group.to_owned());
            }
        }
    }
    names
}

/// Whether `group` has a place above and below it among the groups of `list`.
pub fn move_group_options(list: &[Instance], group: &str) -> (bool, bool) {
    let names = group_names(list);
    let at = names.iter().position(|g| g == group);
    (
        at.is_some_and(|n| n > 0),
        at.is_some_and(|n| n + 1 < names.len()),
    )
}

fn no_group(group: &str) -> String {
    crate::i18n::trf(
        "No group {name}: sunrise-launcher instance groups shows them",
        &[("name", group)],
    )
}

fn members<'a>(all: &'a [Instance], group: &str) -> Vec<&'a Instance> {
    all.iter()
        .filter(|i| i.group.as_deref() == Some(group))
        .collect()
}

/// Moves the group `group` one place up or down among the groups, in the order the sidebar
/// shows them. A group already at the end it is sent to stays where it is.
pub fn move_group(group: &str, up: bool) -> Result<(), String> {
    move_group_in(&backend::game_home(), &backend::data_dir(), group, up)
}

pub(super) fn move_group_in(
    game_home: &Path,
    data: &Path,
    group: &str,
    up: bool,
) -> Result<(), String> {
    let mut names = group_names(&list_in(game_home, data));
    let at = names
        .iter()
        .position(|g| g == group)
        .ok_or_else(|| no_group(group))?;
    let Some(with) =
        (if up { at.checked_sub(1) } else { Some(at + 1) }).filter(|n| *n < names.len())
    else {
        return Ok(());
    };
    // Every group is written with its place, so one never moved keeps the place it showed in.
    names.swap(at, with);
    Store::mutate(data, |store| {
        store.group_order = names;
        Ok(())
    })
}

/// Gives every profile in the group `group` the group `new`. A name another group has joins
/// the two, and the folded and placed state of the old name goes to the new one.
pub fn rename_group(group: &str, new: &str) -> Result<(), String> {
    rename_group_in(&backend::game_home(), &backend::data_dir(), group, new)
}

pub(super) fn rename_group_in(
    game_home: &Path,
    data: &Path,
    group: &str,
    new: &str,
) -> Result<(), String> {
    let new = new.trim();
    if new.is_empty() {
        return Err(crate::i18n::tr("A group needs a name.").into());
    }
    let all = list_in(game_home, data);
    let inside = members(&all, group);
    if inside.is_empty() {
        return Err(no_group(group));
    }
    if new == group {
        return Ok(());
    }
    Store::mutate(data, |store| {
        for member in &inside {
            store
                .instances
                .entry(member.id.clone())
                .or_insert_with(|| initial_choices(&member.home).unwrap_or_default())
                .group = Some(new.to_owned());
        }
        if store.group_order.iter().any(|g| g == new) {
            store.group_order.retain(|g| g != group);
        } else {
            for g in store.group_order.iter_mut().filter(|g| g.as_str() == group) {
                *g = new.to_owned();
            }
        }
        if store.collapsed_groups.remove(group) {
            store.collapsed_groups.insert(new.to_owned());
        }
        Ok(())
    })
}

/// Takes every profile out of the group `group` (they keep existing, with no group) and forgets
/// its place and fold. Nothing happens to a group nobody is in.
pub fn ungroup_group(group: &str) -> Result<(), String> {
    ungroup_group_in(&backend::game_home(), &backend::data_dir(), group)
}

pub(super) fn ungroup_group_in(game_home: &Path, data: &Path, group: &str) -> Result<(), String> {
    let all = list_in(game_home, data);
    let inside = members(&all, group);
    Store::mutate(data, |store| {
        for member in &inside {
            store
                .instances
                .entry(member.id.clone())
                .or_insert_with(|| initial_choices(&member.home).unwrap_or_default())
                .group = None;
        }
        store.group_order.retain(|g| g != group);
        store.collapsed_groups.remove(group);
        Ok(())
    })
}

/// Folds the group `group` under its heading, or unfolds it.
pub fn collapse_group(group: &str, collapsed: bool) -> Result<(), String> {
    collapse_group_in(&backend::data_dir(), group, collapsed)
}

pub(super) fn collapse_group_in(data: &Path, group: &str, collapsed: bool) -> Result<(), String> {
    Store::mutate(data, |store| {
        if collapsed {
            store.collapsed_groups.insert(group.to_owned());
        } else {
            store.collapsed_groups.remove(group);
        }
        Ok(())
    })
}

/// Folds the whole profile list under the sidebar's "Profiles" heading, or unfolds it.
pub fn fold_profiles(folded: bool) -> Result<(), String> {
    fold_profiles_in(&backend::data_dir(), folded)
}

pub(super) fn fold_profiles_in(data: &Path, folded: bool) -> Result<(), String> {
    Store::mutate(data, |store| {
        store.profiles_folded = folded;
        Ok(())
    })
}

#[cfg(test)]
mod tests;
