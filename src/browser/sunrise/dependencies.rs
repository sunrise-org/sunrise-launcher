//! A manual registry install stages and verifies the whole set before changing the profile.
use super::{fs, index, installed, mod_list, Install, SourceId};
use crate::i18n::trf;
use semver::{Version, VersionReq};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use sunrise_launcher_smod::index::resolve::{self, Request, RequestResult};
use sunrise_launcher_smod::manifest::Side;
use sunrise_launcher_smod::mod_list::FILE as ENABLED;

pub(super) fn install(
    install: &Install<'_>,
    project: &index::Project,
    version: &index::Version,
    native_allowed: bool,
    turn_on: bool,
) -> Result<Vec<installed::Entry>, String> {
    let home = install.home;
    let _lock = crate::browser::lock_home(home).map_err(|e| e.to_string())?;
    installed::recover_game_install(home)?;
    let staged = tempfile::Builder::new()
        .prefix(".installed-mods-")
        .tempdir_in(home)
        .map_err(|e| format!("{}: {e}", home.display()))?;
    let installed = installed::Installed::read(home)?;
    let claims = index::browser_state::Installed::read(home).map_err(|error| error.details(trf))?;
    // The index's claims plan the versions first: an unsatisfiable set is refused with the planner's
    // reason before anything downloads, and each dependency is fetched at the version it chose.
    let held = super::held_mods(&installed.entries);
    let planned = resolve::install_with_messages(
        install.index,
        &[Request {
            id: project.id.clone(),
            version: Some(version.version.clone()),
        }],
        &held,
        &version.game,
        Side::Both,
        |_, v| super::fits_game(v, &version.game),
        trf,
    );
    if let Some(RequestResult::Refused { reason }) = planned.requested.get(&project.id) {
        return Err(format!("{} can't be installed: {reason}", project.name));
    }
    let chosen: BTreeMap<String, String> = planned
        .install
        .into_iter()
        .map(|m| (m.id, m.version))
        .chain(planned.update.into_iter().map(|u| (u.id, u.to)))
        .collect();
    let mut plan = Plan {
        install,
        staged: staged.path(),
        installed,
        claims,
        selected: Vec::new(),
        unreadable: Vec::new(),
        native_allowed,
        turn_on,
        chosen,
    };
    plan.add(project, version, 0)?;

    let claims = index::browser_state::Installed::read(home).map_err(|error| error.details(trf))?;
    let mut changes = Vec::new();
    let mut done = Vec::new();
    // Unreadable files whose new version goes elsewhere: published over nothing, so removed.
    let mut stale = Vec::new();
    let mut enable = if turn_on {
        vec![project.id.clone()]
    } else {
        Vec::new()
    };
    for (entry, file) in plan.selected {
        if let Some(file) = file {
            let old = plan.installed.find(SourceId::Sunrise, &entry.project);
            super::check_replacement_claims(
                &claims,
                home,
                &entry.project,
                &entry.file,
                old.map(|old| old.file.as_str()),
            )?;
            if let Some(old) = old {
                // A file that is gone or no longer parses is replaced wherever the new one goes
                // (a native mod and a plain one live in different places); a good one is not.
                let old_path = old.path(home);
                let replaceable = old_path.as_ref().is_some_and(|path| {
                    plan.unreadable.contains(path) || path.try_exists().is_ok_and(|there| !there)
                });
                if old.file != entry.file && !replaceable {
                    return Err(format!(
                        "{} is installed in a different location: remove it first.",
                        entry.title
                    ));
                }
                if old.file != entry.file {
                    stale.extend(old_path.filter(|path| plan.unreadable.contains(path)));
                }
            }
            changes.push((file, home.join(&entry.file)));
            plan.installed
                .entries
                .retain(|e| e.source != SourceId::Sunrise || e.project != entry.project);
            plan.installed.entries.push(entry.clone());
            if turn_on && entry.project != project.id {
                enable.push(entry.project.clone());
            }
            done.push(entry);
        } else if entry.project == project.id {
            done.push(entry);
        }
    }
    // Publishing drops what it replaces, so a file that no longer parses is copied into the
    // staging folder first, and kept in `mod-browser/unreadable` only once publishing worked.
    let mut aside = Vec::new();
    for (n, broken) in plan.unreadable.iter().enumerate() {
        let copy = staged.path().join(format!("aside-{n}"));
        fs::copy(broken, &copy).map_err(|e| format!("{}: {e}", broken.display()))?;
        aside.push((broken.clone(), copy));
    }
    if !changes.is_empty() {
        plan.installed.save(staged.path())?;
        changes.push((
            staged.path().join(installed::MANIFEST),
            home.join(installed::MANIFEST),
        ));
    }
    if enable.iter().any(|id| !mod_list::is_enabled(home, id)) {
        let saved = home.join(ENABLED);
        let next = staged.path().join(ENABLED);
        match fs::copy(&saved, &next) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", saved.display())),
        }
        let ids = enable.iter().map(String::as_str).collect::<Vec<_>>();
        mod_list::set_all_enabled(staged.path(), &ids, true)?;
        changes.push((next, saved));
    }
    let _staged = publish(staged, changes)?;
    for (broken, copy) in aside {
        let to = super::unreadable_slot(home, &broken)?;
        fs::copy(copy, &to).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    for path in stale {
        let _ = fs::remove_file(path);
    }
    Ok(done)
}

struct Plan<'a> {
    install: &'a Install<'a>,
    staged: &'a Path,
    installed: installed::Installed,
    claims: index::browser_state::Installed,
    selected: Vec<(installed::Entry, Option<PathBuf>)>,
    /// Installed files that no longer parse and are about to be replaced.
    unreadable: Vec<PathBuf>,
    native_allowed: bool,
    /// False when an installed mod that is off is being updated and must stay off.
    turn_on: bool,
    /// The version the index plan chose for each mod it installs or updates.
    chosen: BTreeMap<String, String>,
}

impl Plan<'_> {
    fn add(&mut self, p: &index::Project, v: &index::Version, depth: usize) -> Result<(), String> {
        if depth > installed::MAX_DEPTH {
            return Err(format!("{}'s dependencies go too deep.", p.name));
        }
        let (mut entry, download, unreadable) = self.install.prepare(
            p,
            v,
            &self.installed,
            &self.claims,
            &self.staged.join(&p.id),
        )?;
        self.unreadable.extend(unreadable);
        let (path, facts) = match download {
            Some((path, facts)) => {
                entry.dependency = depth != 0
                    || self
                        .installed
                        .find(SourceId::Sunrise, &p.id)
                        .is_some_and(|e| e.dependency);
                (Some(path), facts)
            }
            None => {
                let (_, facts) = index::check_file_with_facts(
                    self.install.index,
                    p,
                    v,
                    &self.install.home.join(&entry.file),
                )
                .map_err(|e| format!("Cannot use installed {}: {e}", p.name))?;
                (None, facts)
            }
        };
        self.selected.push((entry, path));
        for (id, dependency) in facts.dependencies {
            let dep = dependency.details();
            if !dep.optional {
                self.require(&id, &dep.version, &v.game, depth + 1)
                    .map_err(|e| format!("{} requires {id} {}: {e}", p.name, dep.version))?;
            }
        }
        Ok(())
    }

    fn require(
        &mut self,
        id: &str,
        range: &VersionReq,
        game: &str,
        depth: usize,
    ) -> Result<(), String> {
        let index = self.install.index;
        let matches = |version: &str| Version::parse(version).is_ok_and(|v| range.matches(&v));
        if let Some((entry, _)) = self.selected.iter().find(|(e, _)| e.project == id) {
            return if matches(&entry.version) {
                Ok(())
            } else {
                Err(format!(
                    "{} is already selected for this install",
                    entry.version
                ))
            };
        }
        let p = index
            .project(id)
            .filter(|p| matches!(p.kind, index::Kind::Mod | index::Kind::NativeMod))
            .ok_or("no such mod in the Sunrise registry")?;
        let candidates = p
            .versions
            .iter()
            .filter(|v| v.game == game && matches(&v.version));
        // Only a mod the plan names is held to the planned version: one it doesn't list (its owner
        // carried no claim, or the plan kept an installed copy) takes the first version in range.
        let mut candidates = candidates
            .filter(|v| v.capabilities.unknown().is_empty())
            .filter(|v| {
                self.chosen
                    .get(id)
                    .is_none_or(|chosen| *chosen == v.version)
            });
        let v = if let Some(old) = self.installed.find(SourceId::Sunrise, id) {
            let v = candidates
                .find(|v| v.version == old.version)
                .ok_or_else(|| {
                    format!("installed {} does not match; change it first", old.version)
                })?;
            if self.turn_on && !mod_list::is_enabled(self.install.home, id) {
                return Err("it is installed but off; turn it on first".into());
            }
            v
        } else {
            let mut taken_down = None;
            let v = candidates
                .find(|v| {
                    if let Some(r) = index.revoked(id, &v.file.sha256) {
                        taken_down = Some(&r.reason);
                        false
                    } else {
                        true
                    }
                })
                .ok_or_else(|| match taken_down {
                    Some(reason) => format!("taken down: {reason}"),
                    None => format!("no matching version for Minecraft {game}"),
                })?;
            if p.kind == index::Kind::NativeMod && !self.native_allowed {
                return Err(
                    "it runs native code; install it explicitly to grant permission first".into(),
                );
            }
            v
        };
        self.add(p, v, depth)
    }
}

/// Same-filesystem moves with an undo list; even a failed metadata write restores old files.
/// The profile lock keeps another launcher or game start from seeing an intermediate state.
fn publish(
    staged: tempfile::TempDir,
    changes: Vec<(PathBuf, PathBuf)>,
) -> Result<tempfile::TempDir, String> {
    let mut undo = Vec::new();
    for (i, (from, to)) in changes.into_iter().enumerate() {
        let result = (|| -> io::Result<()> {
            if let Some(dir) = to.parent() {
                fs::create_dir_all(dir)?;
            }
            let backup = match fs::symlink_metadata(&to) {
                Ok(m) if m.is_file() => {
                    let backup = staged.path().join(format!("old-{i}"));
                    fs::rename(&to, &backup)?;
                    Some(backup)
                }
                Ok(_) => return Err(io::Error::other("not a regular file")),
                Err(e) if e.kind() == io::ErrorKind::NotFound => None,
                Err(e) => return Err(e),
            };
            undo.push((to.clone(), backup, false));
            fs::rename(&from, &to)?;
            undo.last_mut().expect("just recorded").2 = true;
            Ok(())
        })();
        if let Err(e) = result {
            let mut rollback_errors = Vec::new();
            for (file, backup, placed) in undo.into_iter().rev() {
                let restored = match backup {
                    Some(backup) => fs::rename(backup, &file),
                    None if placed => fs::remove_file(&file),
                    None => Ok(()),
                };
                if let Err(e) = restored {
                    rollback_errors.push(format!("{}: {e}", file.display()));
                }
            }
            let message = format!("Cannot install {}: {e}", to.display());
            return Err(if rollback_errors.is_empty() {
                message
            } else {
                format!(
                    "{message}; rollback failed: {}. Backups: {}",
                    rollback_errors.join("; "),
                    staged.keep().display()
                )
            });
        }
    }
    Ok(staged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_later_publish_failure_restores_the_archive_that_was_already_moved() {
        let home = tempfile::tempdir().unwrap();
        let stage = tempfile::tempdir_in(home.path()).unwrap();
        let old = home.path().join("ruby.smod");
        let next = stage.path().join("ruby.smod");
        let manifest = home.path().join(installed::MANIFEST);
        fs::write(&old, b"old archive").unwrap();
        fs::write(&next, b"new archive").unwrap();
        fs::create_dir(&manifest).unwrap();
        let next_manifest = stage.path().join(installed::MANIFEST);
        fs::write(&next_manifest, b"new manifest").unwrap();

        let error = publish(
            stage,
            vec![(next, old.clone()), (next_manifest, manifest.clone())],
        )
        .unwrap_err();
        assert!(error.contains("not a regular file"), "{error}");
        assert_eq!(fs::read(old).unwrap(), b"old archive");
        assert!(manifest.is_dir());
    }
}
