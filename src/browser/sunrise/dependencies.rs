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

/// Installs the `asked` projects, each at its version, and what they need, as one attempt: the
/// plan, the world question and the publish cover the whole set, so a failure anywhere leaves
/// none of it published.
#[cfg(test)]
pub(super) fn install(
    install: &Install<'_>,
    asked: &[(&index::Project, &index::Version)],
    native_allowed: bool,
    turn_on: bool,
    waiting: &BTreeMap<String, super::txn::WaitingMod>,
) -> Result<Vec<installed::Entry>, installed::InstallError> {
    install_inner(install, asked, native_allowed, turn_on, waiting, false)
}

pub(super) fn stage(
    install: &Install<'_>,
    asked: &[(&index::Project, &index::Version)],
    native_allowed: bool,
    waiting: &BTreeMap<String, super::txn::WaitingMod>,
) -> Result<Vec<installed::Entry>, installed::InstallError> {
    install_inner(install, asked, native_allowed, true, waiting, true)
}

fn install_inner(
    install: &Install<'_>,
    asked: &[(&index::Project, &index::Version)],
    native_allowed: bool,
    turn_on: bool,
    waiting: &BTreeMap<String, super::txn::WaitingMod>,
    private: bool,
) -> Result<Vec<installed::Entry>, installed::InstallError> {
    let home = install.home;
    let game = asked
        .first()
        .map_or("", |(_, version)| version.game.as_str());
    let _lock = crate::browser::lock_home(home).map_err(|e| e.to_string())?;
    let staged = tempfile::Builder::new()
        .prefix(".installed-mods-")
        .tempdir_in(home)
        .map_err(|e| format!("{}: {e}", home.display()))?;
    // An install an earlier run left half done is undone first; while it can't be, nothing new
    // is begun on top of its list.
    if !private {
        super::txn::settle_owned(home)?;
    }
    let mut installed = installed::Installed::load(home);
    // An approval that waited in a queue is read again here, under the lock: each mod it
    // removes must still be the file the player saw, and a world that uses it must have been
    // agreed to (worlds that can't be looked at stop the install).
    // The exact staged API does not accept a legacy approval to remove a conflict.
    let approvals = if private { &[][..] } else { install.approvals };
    let removes: Vec<&installed::Replace> = approvals
        .iter()
        .filter_map(|a| match a {
            installed::Approval::Remove(r) => Some(r),
            installed::Approval::DropContent { .. } => None,
        })
        .collect();
    let agreed = |id: &str| {
        approvals
            .iter()
            .any(|a| matches!(a, installed::Approval::DropContent { id: agreed } if agreed == id))
    };
    for r in &removes {
        let now = installed
            .entries
            .iter()
            .find(|e| e.source == SourceId::Sunrise && e.project == r.id);
        if now.is_some_and(|e| e.checksum != r.checksum) {
            return Err(trf(
                "{mod} changed since you approved removing it. Install again to review it.",
                &[("mod", &r.id)],
            )
            .into());
        }
    }
    let unagreed: Vec<String> = removes
        .iter()
        .filter(|r| !r.worlds_ok && !agreed(&r.id))
        .map(|r| r.id.clone())
        .collect();
    let question = if private || unagreed.is_empty() {
        None
    } else {
        crate::worlds::question(home, &unagreed)?
    };
    if let Some(question) = question {
        return Err(installed::InstallError::NeedsWorlds {
            mods: question.mods,
        });
    }
    // The mods this install replaces are out of the picture for the plan and go with it.
    let (removing, kept): (Vec<_>, Vec<_>) = installed
        .entries
        .drain(..)
        .partition(|e| e.source == SourceId::Sunrise && removes.iter().any(|r| r.id == e.project));
    installed.entries = kept;
    // The index's claims plan the versions first: an unsatisfiable set is refused with the planner's
    // reason before anything downloads, and each dependency is fetched at the version it chose.
    let held = super::held_mods(&installed.entries);
    let planned = resolve::install(
        install.index,
        &asked
            .iter()
            .map(|(project, version)| Request {
                id: project.id.clone(),
                version: Some(version.version.clone()),
            })
            .collect::<Vec<_>>(),
        &held,
        game,
        Side::Both,
        |_, v| super::fits_game(v, game),
    );
    for (project, _) in asked {
        if let Some(RequestResult::Refused { reason }) = planned.requested.get(&project.id) {
            return Err(super::explain(
                install.index,
                std::slice::from_ref(&project.id),
                &held,
                game,
                format!("{} can't be installed: {reason}", project.name),
            ));
        }
    }
    // Native code the plan names is refused before anything downloads; the files are checked
    // again below, for what the index doesn't declare.
    if !native_allowed {
        let planned_native = planned
            .install
            .iter()
            .map(|m| &m.id)
            .chain(planned.update.iter().map(|u| &u.id))
            .filter_map(|id| install.index.project(id))
            .find(|p| p.kind == index::Kind::NativeMod);
        if let Some(native) = planned_native {
            return Err(installed::InstallError::NeedsNativeConsent {
                native: native.name.clone(),
            });
        }
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
        selected: Vec::new(),
        unreadable: Vec::new(),
        turn_on,
        chosen,
        waiting,
    };
    for (project, version) in asked {
        // One that an earlier project of the set needs is already in the plan, and is now also
        // one the player asked for.
        if let Some((entry, _)) = plan
            .selected
            .iter_mut()
            .find(|(e, _)| e.project == project.id)
        {
            if entry.version != version.version {
                return Err(format!(
                    "{} {} was asked for, but another mod of the set needs {}.",
                    project.name, version.version, entry.version
                )
                .into());
            }
            entry.dependency = false;
            continue;
        }
        plan.add(project, version, 0)?;
    }
    // Native code follows the player's one-time choice, whatever brings it: the mod asked
    // for or any of what it needs, by the files themselves, not by what the index declares.
    // The install interface asks first; this is what holds for the headless ones.
    if !native_allowed {
        if let Some((entry, _)) = plan
            .selected
            .iter()
            .find(|(e, _)| crate::native_status::is_native_file(&e.file))
        {
            return Err(installed::InstallError::NeedsNativeConsent {
                native: entry.title.clone(),
            });
        }
    }

    // Every package this install replaces is compared with its staged replacement, under the
    // lock: a mod that loses content (a block, an item) that a saved world may use asks the
    // world question before anything is published, whoever brings the new version (the mod
    // asked for or a dependency moving to another version). Worlds that can't be looked at,
    // or a package that can't be compared, stop the install.
    let mut drops = Vec::new();
    for (entry, file) in plan.selected.iter().filter(|_| !private) {
        let (Some(file), Some(old)) =
            (file, plan.installed.find(SourceId::Sunrise, &entry.project))
        else {
            continue;
        };
        let Some(old_path) = old.path(home).filter(|p| !plan.unreadable.contains(p)) else {
            continue;
        };
        if old_path.is_file()
            && !agreed(&entry.project)
            && !super::content_loss::dropped(&old_path, file)?.is_empty()
        {
            drops.push(entry.project.clone());
        }
    }
    if !drops.is_empty() {
        if let Some(question) = crate::worlds::question(home, &drops)? {
            return Err(installed::InstallError::NeedsWorlds {
                mods: question.mods,
            });
        }
    }

    let mut changes = Vec::new();
    let mut done = Vec::new();
    // Unreadable files whose new version goes elsewhere: published over nothing, so removed.
    let mut stale = Vec::new();
    // Keep the player's off choice. An explicitly updated waiting mod was switched off by the
    // launcher, so restore its previous on choice only when its replacement is staged.
    let off: Vec<String> = plan
        .installed
        .entries
        .iter()
        .filter(|e| e.source == SourceId::Sunrise && mod_list::is_off(home, &e.project))
        .map(|e| e.project.clone())
        .collect();
    let wanted = |id: &str| asked.iter().any(|(project, _)| project.id == id);
    let mut enable: Vec<String> = asked
        .iter()
        .map(|(project, _)| project.id.clone())
        .filter(|id| turn_on && !off.contains(id))
        .collect();
    for (entry, file) in plan.selected {
        if let Some(file) = file {
            let was_off = off.contains(&entry.project);
            if let Some(old) = plan.installed.find(SourceId::Sunrise, &entry.project) {
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
                    )
                    .into());
                }
                if old.file != entry.file {
                    stale.extend(old_path.filter(|path| plan.unreadable.contains(path)));
                }
            }
            changes.push((Some(file), home.join(&entry.file)));
            plan.installed
                .entries
                .retain(|e| e.source != SourceId::Sunrise || e.project != entry.project);
            plan.installed.entries.push(entry.clone());
            let resumes = was_off
                && wanted(&entry.project)
                && waiting.get(&entry.project).is_some_and(|mod_| mod_.was_on);
            if turn_on && (resumes || !was_off && !wanted(&entry.project)) {
                enable.push(entry.project.clone());
            }
            done.push(entry);
        } else if wanted(&entry.project) {
            done.push(entry);
        }
    }
    // Publishing drops what it replaces, so a file that no longer parses is copied into the
    // staging folder first and published into `mod-browser/unreadable` with the rest: a
    // step of the transaction, put back (removed) if it fails. So is the removal of a stale
    // file whose replacement goes elsewhere.
    for (n, broken) in plan.unreadable.iter().enumerate() {
        let copy = staged.path().join(format!("aside-{n}"));
        fs::copy(broken, &copy).map_err(|e| format!("{}: {e}", broken.display()))?;
        changes.push((Some(copy), super::unreadable_slot(home, broken)?));
    }
    for path in stale {
        changes.push((None, path));
    }
    // What it replaces is moved aside with the rest, so a failure brings it back.
    let mut disable = Vec::new();
    for old in &removing {
        if home.join(&old.file).symlink_metadata().is_ok() {
            changes.push((None, home.join(&old.file)));
        }
        disable.extend(super::mod_id(old).map(str::to_owned));
    }
    if !changes.is_empty() || !removing.is_empty() {
        plan.installed.save(staged.path())?;
        changes.push((
            Some(staged.path().join(installed::MANIFEST)),
            home.join(installed::MANIFEST),
        ));
    }
    let stays_on = |id: &String| !mod_list::is_enabled(home, id);
    if enable.iter().any(stays_on) || disable.iter().any(|id| mod_list::is_enabled(home, id)) {
        let saved = home.join(ENABLED);
        let next = staged.path().join(ENABLED);
        match fs::copy(&saved, &next) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", saved.display()).into()),
        }
        let ids = enable.iter().map(String::as_str).collect::<Vec<_>>();
        mod_list::set_all_enabled(staged.path(), &ids, true)?;
        let ids = disable.iter().map(String::as_str).collect::<Vec<_>>();
        mod_list::set_all_enabled(staged.path(), &ids, false)?;
        changes.push((Some(next), saved));
    }
    if private {
        // These paths belong to the reserved candidate, never the live profile. Only the
        // sealed outer attempt may publish, after the exact image and world check.
        for (from, to) in changes {
            if let Some(from) = from {
                if let Some(parent) = to.parent() {
                    fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
                }
                fs::copy(from, &to).map_err(|e| format!("{}: {e}", to.display()))?;
            } else {
                match fs::remove_file(&to) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(format!("{}: {e}", to.display()).into()),
                }
            }
        }
    } else {
        let brings_native = done
            .iter()
            .any(|e| crate::native_status::is_native_file(&e.file));
        let (staged, attempt) = publish(staged, changes, install.approvals)?;
        super::txn::commit(home, &attempt, staged, brings_native)?;
    }
    Ok(done)
}

pub(super) fn remove_checked(
    home: &Path,
    wanted: &[(String, String)],
) -> Result<Vec<String>, String> {
    let _lease = super::txn::Lease::admit(home)?;
    let _lock = crate::browser::lock_home(home).map_err(|e| e.to_string())?;
    super::txn::settle_owned(home)?;
    if super::txn::has_build(home)? {
        return Err(super::txn::unsettled());
    }
    let mut list = installed::Installed::load(home);
    let (gone, kept): (Vec<_>, Vec<_>) = list.entries.drain(..).partition(|e| {
        e.source == SourceId::Sunrise
            && wanted
                .iter()
                .any(|(id, checksum)| *id == e.project && *checksum == e.checksum)
    });
    list.entries = kept;
    if gone.is_empty() {
        return Ok(Vec::new());
    }
    let staged = tempfile::Builder::new()
        .prefix(".installed-mods-")
        .tempdir_in(home)
        .map_err(|e| format!("{}: {e}", home.display()))?;
    let mut changes = Vec::new();
    let mut disable = Vec::new();
    for entry in &gone {
        if let Some(path) = entry.path(home) {
            if path.symlink_metadata().is_ok_and(|m| !m.is_file()) {
                return Err(trf(
                    "{file} is a folder, not a file the browser installed. Nothing was removed: move it away yourself.",
                    &[("file", &entry.file)],
                ));
            }
            changes.push((None, path));
        }
        disable.extend(super::mod_id(entry).map(str::to_owned));
    }
    list.save(staged.path())?;
    changes.push((
        Some(staged.path().join(installed::MANIFEST)),
        home.join(installed::MANIFEST),
    ));
    if disable.iter().any(|id| mod_list::is_enabled(home, id)) {
        let saved = home.join(ENABLED);
        let next = staged.path().join(ENABLED);
        match fs::copy(&saved, &next) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", saved.display())),
        }
        let ids = disable.iter().map(String::as_str).collect::<Vec<_>>();
        mod_list::set_all_enabled(staged.path(), &ids, false)?;
        changes.push((Some(next), saved));
    }
    let (staged, attempt) = publish(staged, changes, &[])?;
    super::txn::commit(home, &attempt, staged, false)?;
    Ok(gone.into_iter().map(|e| e.project).collect())
}

struct Plan<'a> {
    install: &'a Install<'a>,
    staged: &'a Path,
    installed: installed::Installed,
    selected: Vec<(installed::Entry, Option<PathBuf>)>,
    /// Installed files that no longer parse and are about to be replaced.
    unreadable: Vec<PathBuf>,
    /// False when an installed mod that is off is being updated and must stay off.
    turn_on: bool,
    /// The version the index plan chose for each mod it installs or updates.
    chosen: BTreeMap<String, String>,
    waiting: &'a BTreeMap<String, super::txn::WaitingMod>,
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
            &self.staged.join(&p.id),
            self.waiting.get(&p.id),
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
        let candidates: Vec<_> = candidates
            .filter(|v| v.capabilities.unknown().is_empty())
            .filter(|v| {
                self.chosen
                    .get(id)
                    .is_none_or(|chosen| *chosen == v.version)
            })
            .collect();
        let v = if let Some(old) = self.installed.find(SourceId::Sunrise, id) {
            if let Some(kept) = candidates.iter().find(|v| v.version == old.version) {
                if self.turn_on && !mod_list::is_enabled(self.install.home, id) {
                    return Err("it is installed but off; turn it on first".into());
                }
                kept
            } else if self.chosen.contains_key(id) {
                // The plan moves it: the version it chose replaces the installed one, which
                // stays on or off as it is (`Plan::kept_off`).
                let mut taken_down = None;
                candidates
                    .iter()
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
                    })?
            } else {
                return Err(format!(
                    "installed {} does not match; change it first",
                    old.version
                ));
            }
        } else {
            let mut taken_down = None;
            let v = candidates
                .iter()
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
            v
        };
        self.add(p, v, depth)
    }
}

/// Same-filesystem moves under a record that is written first (`txn::begin`): if it can't be,
/// nothing moves; if a move fails, the attempt is undone on the spot. The record stays until
/// the caller commits (`txn::commit`): publishing is not the end. The profile lock keeps
/// another launcher or game start from seeing an intermediate state.
fn publish(
    staged: tempfile::TempDir,
    changes: Vec<(Option<PathBuf>, PathBuf)>,
    approvals: &[installed::Approval],
) -> Result<(tempfile::TempDir, String), String> {
    let home = staged
        .path()
        .parent()
        .expect("staged in the profile")
        .to_path_buf();
    let id = super::txn::begin(&home, staged.path(), &changes, approvals)?;
    for (i, (from, to)) in changes.into_iter().enumerate() {
        let result = (|| -> io::Result<()> {
            if let Some(dir) = to.parent() {
                fs::create_dir_all(dir)?;
            }
            match fs::symlink_metadata(&to) {
                Ok(m) if m.is_file() => fs::rename(&to, staged.path().join(format!("old-{i}")))?,
                Ok(_) => return Err(io::Error::other("not a regular file")),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            // No file to put there: the one that was is removed.
            if let Some(from) = from {
                fs::rename(&from, &to)?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            let message = trf(
                "Cannot install {path}: {error}",
                &[
                    ("path", &to.display().to_string()),
                    ("error", &e.to_string()),
                ],
            );
            return Err(super::txn::rollback(&home, &id, staged, message));
        }
    }
    Ok((staged, id))
}
