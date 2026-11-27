//! The order the player gave the profiles in the sidebar, within their group.
use super::*;

/// Moves the profile `query` one place up or down among the profiles of its group, in the order
/// the sidebar shows them. The default profile stays first and does not move; a profile already at
/// the end it is sent to stays where it is.
pub fn move_profile(query: &str, up: bool) -> Result<(), String> {
    move_profile_in(&backend::game_home(), &backend::data_dir(), query, up)
}

pub(super) fn move_profile_in(
    game_home: &Path,
    data: &Path,
    query: &str,
    up: bool,
) -> Result<(), String> {
    let all = list_in(game_home, data);
    let instance = find_in(&all, query)?;
    if instance.kind == Kind::Default {
        return Err(crate::i18n::tr("The default profile stays first.").into());
    }
    let mut group: Vec<&Instance> = all[1..]
        .iter()
        .filter(|i| i.group == instance.group)
        .collect();
    let Some(at) = group.iter().position(|i| i.id == instance.id) else {
        return Ok(());
    };
    let Some(with) =
        (if up { at.checked_sub(1) } else { Some(at + 1) }).filter(|n| *n < group.len())
    else {
        return Ok(());
    };
    group.swap(at, with);
    Store::mutate(data, |store| {
        for (place, member) in group.iter().enumerate() {
            let meta = store
                .instances
                .entry(member.id.clone())
                .or_insert_with(|| initial_choices(&member.home).unwrap_or_default());
            meta.order = Some(u32::try_from(place).unwrap_or(u32::MAX));
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests;
