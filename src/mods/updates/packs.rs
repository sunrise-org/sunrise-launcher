//! Check for updates on Home's Packs and Shaders tabs. A pack the browser installed from
//! Modrinth has its project and version in `mod-browser.json`, so it can be asked about like a
//! mod; a pack the player copied in from a file has no such record and is never listed.
use super::Found;
use std::collections::BTreeSet;
use std::path::Path;
use sunrise_launcher::browser::installed::{self, Installed, PackUpdate};
use sunrise_launcher::browser::{Kind, SourceId};

/// Asks Modrinth about each pack of `kind` the browser installed. A pack that can't be asked
/// about (its file moved, one lookup failed) is left out; if every lookup failed the check says
/// why.
pub(crate) fn check(home: &Path, kind: Kind, game: &str) -> Result<Vec<Found>, String> {
    let entries: Vec<_> = Installed::load(home)
        .entries
        .into_iter()
        .filter(|entry| entry.kind == kind && entry.source != SourceId::Sunrise)
        .collect();
    let mut found = Vec::new();
    let mut failure = None;
    let mut answered = false;
    for entry in &entries {
        let update = match installed::update_pack(home, entry, game, true, &|_, _| {}) {
            Ok(PackUpdate::Available(number)) => Some(number),
            Ok(_) => None,
            Err(why) => {
                failure.get_or_insert(why);
                continue;
            }
        };
        answered = true;
        found.push(Found {
            id: entry.project.clone(),
            name: entry.title.clone(),
            project: None,
            current: entry.number.clone(),
            update,
            ..Found::default()
        });
    }
    match failure {
        Some(why) if !answered => Err(why),
        _ => Ok(found),
    }
}

/// Updates the packs of `kind` whose project is in `projects`, one after another. Returns how
/// many were updated, or the first failure with what it had done by then.
pub(crate) fn update(
    home: &Path,
    kind: Kind,
    game: &str,
    projects: &BTreeSet<String>,
) -> Result<usize, String> {
    let entries: Vec<_> = Installed::load(home)
        .entries
        .into_iter()
        .filter(|entry| entry.kind == kind && projects.contains(&entry.project))
        .collect();
    let mut done = 0;
    for entry in &entries {
        match installed::update_pack(home, entry, game, false, &|_, _| {}) {
            Ok(PackUpdate::Updated(list)) => done += list.len().max(1),
            Ok(_) => {}
            Err(why) => return Err(why),
        }
    }
    Ok(done)
}
