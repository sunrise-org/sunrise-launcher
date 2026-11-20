//! Save evidence held through a Switch's decision and local publishing cutoff.
use super::{level_dat, lock_world, scanned_world, WorldLock};
use crate::i18n::{tr, trf};
use cap_fs_ext::MetadataExt;
use cap_std::{ambient_authority, fs::Dir};
use sha2::{Digest, Sha256};
use std::{
    cmp::Ordering,
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
};
use sunrise_launcher_nbt::Tag;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldRisk {
    pub world: String,
    pub mods: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Question {
    pub affected: Vec<WorldRisk>,
    pub newer: Vec<String>,
    pub unknown_order: Vec<String>,
}

impl Question {
    pub fn needed(&self) -> bool {
        !self.affected.is_empty() || !self.newer.is_empty() || !self.unknown_order.is_empty()
    }

    pub fn lines(&self) -> Vec<String> {
        let mut out: Vec<_> = self
            .affected
            .iter()
            .map(|risk| {
                trf(
                    "{mods} — content may be lost when you open the world {world}.",
                    &[("mods", &risk.mods.join(", ")), ("world", &risk.world)],
                )
            })
            .collect();
        if !self.newer.is_empty() {
            out.push(trf(
                "These worlds were last played on a newer game: {worlds}",
                &[("worlds", &self.newer.join(", "))],
            ));
        }
        if !self.unknown_order.is_empty() {
            out.push(trf(
                "The last game build is unknown for these worlds: {worlds}",
                &[("worlds", &self.unknown_order.join(", "))],
            ));
        }
        out
    }
}

/// The affected package's manifest ID, title and defined namespaces, not the registry slug.
pub(crate) struct Changed<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub namespaces: &'a [String],
}

#[derive(Clone, PartialEq, Eq)]
struct Evidence {
    path: PathBuf,
    directory: (u64, u64),
    primary: Option<[u8; 32]>,
    previous: Option<[u8; 32]>,
}

pub(crate) struct Fence {
    evidence: Vec<Evidence>,
    _held: Vec<WorldLock>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Age {
    NotNewer,
    Newer,
    Unknown,
}

pub(crate) struct TargetAge<'a> {
    pub data_version: Option<i32>,
    pub compare_saved: &'a mut dyn FnMut(&str) -> Option<Ordering>,
}

/// Compare world format first. Only equal known formats ask for full signed build evidence.
fn age(saved: Option<i32>, target: Option<i32>, compare: impl FnOnce() -> Option<Ordering>) -> Age {
    let order = match (saved, target) {
        (Some(a), Some(b)) if a == b => compare(),
        (Some(a), Some(b)) => Some(a.cmp(&b)),
        _ => None,
    };
    match order {
        Some(Ordering::Greater) => Age::Newer,
        Some(_) => Age::NotNewer,
        None => Age::Unknown,
    }
}

impl Fence {
    pub(crate) fn hold(
        home: &Path,
        changed: &[Changed<'_>],
        mut target: Option<TargetAge<'_>>,
    ) -> Result<(Self, Question), String> {
        // Take the whole roster first, then re-read it after taking all locks. Metadata from a
        // path reused between the two snapshots cannot authorize the replacement directory.
        let evidence = scan(home)?;
        let mut held = Vec::new();
        let mut question = Question::default();
        for file in &evidence {
            let world = scanned_world(&file.path)
                .map_err(|e| bad(&file.path, e))?
                .ok_or_else(|| malformed(&file.path))?
                .world;
            let lock = file.path.join(super::LOCK);
            if fs::symlink_metadata(&lock).is_ok_and(|meta| {
                sunrise_launcher_smod::runtime_leases::linked(&meta) || !meta.is_file()
            }) {
                return Err(malformed(&lock));
            }
            held.push(lock_world(&world).map_err(|e| bad(&file.path, e))?);
            let mut used = BTreeSet::new();
            let mut namespaces = BTreeSet::new();
            let mut version = None;
            let mut commit = None;
            for (present, name) in [
                (file.primary, "level.dat"),
                (file.previous, "level.dat_old"),
            ] {
                if present.is_none() {
                    continue;
                }
                let path = file.path.join(name);
                let root = level_dat(&path).ok_or_else(|| malformed(&path))?;
                let data = root
                    .get("Data")
                    .filter(|t| matches!(t, Tag::Compound(_)))
                    .ok_or_else(|| malformed(&path))?;
                let sunrise = compound(root.get("sunrise"), &path)?;
                let forge = compound(root.get("fml"), &path)?;
                let mod_list = strings(
                    sunrise.and_then(|s| s.get("ContentModList")),
                    Some("ModId"),
                    &path,
                )?;
                let native_list = strings(
                    forge.and_then(|s| s.get("LoadingModList")),
                    Some("ModId"),
                    &path,
                )?;
                let namespace_list =
                    strings(sunrise.and_then(|s| s.get("BlockNamespaces")), None, &path)?;
                if name == "level.dat" {
                    version = match data.get("DataVersion") {
                        Some(Tag::Int(version)) => Some(*version),
                        Some(_) => return Err(malformed(&path)),
                        None => None,
                    };
                    commit = match sunrise.and_then(|s| s.get("LastSavedCommit")) {
                        Some(Tag::String(value)) => Some(value.to_string_lossy()),
                        Some(_) => return Err(malformed(&path)),
                        None => None,
                    };
                }
                used.extend(mod_list);
                used.extend(native_list);
                namespaces.extend(namespace_list);
            }
            // A current nonempty list cannot rule out historic content by itself. With no
            // authoritative definitions/history for a changed package, report potential loss.
            let (known_use, possible_use): (Vec<_>, Vec<_>) = changed.iter().partition(|item| {
                used.contains(item.id)
                    || namespaces.contains(item.id)
                    || item.namespaces.iter().any(|name| namespaces.contains(name))
            });
            let mods: Vec<_> = known_use
                .into_iter()
                .chain(possible_use)
                .map(|item| item.title.to_owned())
                .collect();
            if !mods.is_empty() {
                question.affected.push(WorldRisk {
                    world: world.name.clone(),
                    mods,
                });
            }
            if let Some(target) = &mut target {
                match age(version, target.data_version, || {
                    commit.as_deref().and_then(&mut target.compare_saved)
                }) {
                    Age::Newer => question.newer.push(world.name),
                    Age::Unknown => question.unknown_order.push(world.name),
                    Age::NotNewer => {}
                }
            }
        }
        let fence = Self {
            evidence,
            _held: held,
        };
        fence.revalidate(home)?;
        Ok((fence, question))
    }

    pub(crate) fn revalidate(&self, home: &Path) -> Result<(), String> {
        if scan(home)? != self.evidence {
            return Err(tr("The saved worlds changed while the switch was being prepared. Nothing was switched.").into());
        }
        Ok(())
    }
}

fn bad(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}

fn malformed(path: &Path) -> String {
    bad(path, io::ErrorKind::InvalidData)
}

fn compound<'a>(tag: Option<&'a Tag>, path: &Path) -> Result<Option<&'a Tag>, String> {
    match tag {
        Some(Tag::Compound(_)) | None => Ok(tag),
        _ => Err(malformed(path)),
    }
}

fn strings(tag: Option<&Tag>, field: Option<&str>, path: &Path) -> Result<Vec<String>, String> {
    match tag {
        None => Ok(Vec::new()),
        Some(Tag::List(list)) => list
            .iter()
            .map(|entry| {
                let value = field.map_or(Some(entry), |f| entry.get(f));
                match value {
                    Some(Tag::String(s)) if !s.0.is_empty() => Ok(s.to_string_lossy()),
                    _ => Err(malformed(path)),
                }
            })
            .collect(),
        _ => Err(malformed(path)),
    }
}

fn scan(home: &Path) -> Result<Vec<Evidence>, String> {
    let saves = home.join("saves");
    let dir = match fs::symlink_metadata(&saves) {
        Ok(meta) if meta.is_dir() && !sunrise_launcher_smod::runtime_leases::linked(&meta) => {
            fs::read_dir(&saves)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => Err(e),
        Ok(_) => return Err(malformed(&saves)),
    }
    .map_err(|e| bad(&saves, e))?;
    let mut out = Vec::new();
    for entry in dir {
        let path = entry.map_err(|e| bad(&saves, e))?.path();
        let meta = fs::symlink_metadata(&path).map_err(|e| bad(&path, e))?;
        if sunrise_launcher_smod::runtime_leases::linked(&meta) {
            return Err(malformed(&path));
        }
        if !meta.is_dir() {
            continue;
        }
        let opened =
            Dir::open_ambient_dir(&path, ambient_authority()).map_err(|e| bad(&path, e))?;
        let meta = opened.dir_metadata().map_err(|e| bad(&path, e))?;
        let read = |name| -> Result<Option<[u8; 32]>, String> {
            let dat = path.join(name);
            match fs::symlink_metadata(&dat) {
                Ok(meta)
                    if meta.is_file() && !sunrise_launcher_smod::runtime_leases::linked(&meta) =>
                {
                    let file = opened.open(name).map_err(|e| bad(&dat, e))?;
                    let mut hash = Sha256::new();
                    io::copy(&mut file.into_std(), &mut hash).map_err(|e| bad(&dat, e))?;
                    Ok(Some(hash.finalize().into()))
                }
                Ok(_) => Err(malformed(&dat)),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(bad(&dat, e)),
            }
        };
        let primary = read("level.dat")?;
        let previous = read("level.dat_old")?;
        if primary.is_some() || previous.is_some() {
            out.push(Evidence {
                path,
                directory: (meta.dev(), meta.ino()),
                primary,
                previous,
            });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_question_lists_removed_content_newer_worlds_and_unknown_order() {
        use std::io::Write;
        use sunrise_launcher_nbt::{write_named, Limits, NamedTag};
        let home = tempfile::tempdir().unwrap();
        let commit = "1234567890123456789012345678901234567890";
        for (name, version, saved) in [
            ("Format ahead", 9, None),
            ("Build ahead", 8, Some(commit)),
            ("Unknown", 8, None),
        ] {
            let dir = home.path().join("saves").join(name);
            fs::create_dir_all(&dir).unwrap();
            let data = Tag::compound([
                ("LevelName", Tag::String(name.into())),
                ("DataVersion", Tag::Int(version)),
            ]);
            let sunrise = Tag::compound(
                saved
                    .into_iter()
                    .map(|id| ("LastSavedCommit", Tag::String(id.into()))),
            );
            let tag = Tag::compound([("Data", data), ("sunrise", sunrise)]);
            let bytes = write_named(
                &NamedTag {
                    name: "".into(),
                    tag,
                },
                Limits::default(),
            )
            .unwrap();
            let mut file = flate2::write::GzEncoder::new(
                fs::File::create(dir.join("level.dat")).unwrap(),
                flate2::Compression::fast(),
            );
            file.write_all(&bytes).unwrap();
            file.finish().unwrap();
        }
        let (_, result) = Fence::hold(
            home.path(),
            &[Changed {
                id: "removed",
                title: "Lost Content",
                namespaces: &[],
            }],
            Some(TargetAge {
                data_version: Some(8),
                compare_saved: &mut |saved| {
                    assert_eq!(saved, commit);
                    Some(Ordering::Greater)
                },
            }),
        )
        .unwrap();
        assert_eq!(result.newer, ["Build ahead", "Format ahead"]);
        assert_eq!(result.unknown_order, ["Unknown"]);
        assert_eq!(result.affected.len(), 3);
        assert!(result.lines().iter().any(|line| line
            == "These worlds were last played on a newer game: Build ahead, Format ahead"));
    }

    #[test]
    fn world_format_wins_then_equal_formats_need_comparable_build_order() {
        assert_eq!(age(Some(8), Some(7), || unreachable!()), Age::Newer);
        assert_eq!(age(Some(6), Some(7), || unreachable!()), Age::NotNewer);
        assert_eq!(
            age(Some(8), Some(8), || Some(Ordering::Greater)),
            Age::Newer
        );
        assert_eq!(
            age(Some(8), Some(8), || Some(Ordering::Equal)),
            Age::NotNewer
        );
        assert_eq!(age(Some(8), Some(8), || None), Age::Unknown);
        assert_eq!(age(None, Some(8), || unreachable!()), Age::Unknown);
    }
}
