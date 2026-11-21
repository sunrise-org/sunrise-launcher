//! Save evidence held through a staged profile edit's decision and local publishing cutoff.
use super::{lock_reviewed, review_risk, ReviewedWorldLocks, SuspectMods};
use crate::i18n::{tr, trf};
use std::{cmp::Ordering, path::Path};

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

pub(crate) struct Fence {
    sources: SuspectMods,
    evidence: Vec<super::WorldRisk>,
    _held: ReviewedWorldLocks,
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
    pub(crate) fn hold<'a>(
        home: &Path,
        sources: SuspectMods,
        title: impl Fn(&str) -> &'a str,
        mut target: Option<TargetAge<'_>>,
    ) -> Result<(Self, Question), String> {
        // Lock the full roster before interpreting it, including unrelated saves. Re-read
        // after taking the locks so a reused path cannot authorize a different directory.
        let evidence = review_risk(home, &sources)?;
        if let Some(world) = evidence.iter().find(|world| world.malformed) {
            return Err(trf(
                "Cannot verify the saved-world data in {world}. The profile was left unchanged.",
                &[("world", &world.name)],
            ));
        }
        let held = lock_reviewed(&evidence)?;
        let fence = Self {
            sources,
            evidence,
            _held: held,
        };
        fence.revalidate(home)?;
        let mut question = Question::default();
        for world in &fence.evidence {
            // A nonempty last-loaded list cannot rule out blocks in older chunks. Put
            // positively identified mods first, then include the unproved historic ones.
            let mods: Vec<_> = world
                .confirmed_mods
                .iter()
                .chain(
                    fence
                        .sources
                        .keys()
                        .filter(|id| !world.confirmed_mods.contains(id)),
                )
                .map(|id| title(id).to_owned())
                .collect();
            if !mods.is_empty() {
                question.affected.push(WorldRisk {
                    world: world.name.clone(),
                    mods,
                });
            }
            if let Some(target) = &mut target {
                match age(world.data_version, target.data_version, || {
                    world
                        .saved_commit
                        .as_deref()
                        .and_then(&mut target.compare_saved)
                }) {
                    Age::Newer => question.newer.push(world.name.clone()),
                    Age::Unknown => question.unknown_order.push(world.name.clone()),
                    Age::NotNewer => {}
                }
            }
        }
        Ok((fence, question))
    }

    pub(crate) fn revalidate(&self, home: &Path) -> Result<(), String> {
        if review_risk(home, &self.sources)? != self.evidence {
            return Err(tr("The saved worlds changed while the changes were being prepared. The profile was left unchanged.").into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeSet, fs, io::Write};
    use sunrise_launcher_nbt::{write_named, Limits, NamedTag, Tag};

    fn write_world(home: &Path, name: &str, version: i32, sunrise: Tag) {
        let dir = home.join("saves").join(name);
        fs::create_dir_all(&dir).unwrap();
        let data = Tag::compound([
            ("LevelName", Tag::String(name.into())),
            ("DataVersion", Tag::Int(version)),
        ]);
        let bytes = write_named(
            &NamedTag {
                name: "".into(),
                tag: Tag::compound([("Data", data), ("sunrise", sunrise)]),
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

    #[test]
    fn one_question_lists_removed_content_newer_worlds_and_unknown_order() {
        let home = tempfile::tempdir().unwrap();
        let commit = "1234567890123456789012345678901234567890";
        for (name, version, saved) in [
            ("Format ahead", 9, None),
            ("Build ahead", 8, Some(commit)),
            ("Unknown", 8, None),
        ] {
            write_world(
                home.path(),
                name,
                version,
                Tag::compound(
                    saved
                        .into_iter()
                        .map(|id| ("LastSavedCommit", Tag::String(id.into()))),
                ),
            );
        }
        let (_, result) = Fence::hold(
            home.path(),
            SuspectMods::from([("removed".into(), BTreeSet::new())]),
            |_| "Lost Content",
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
    fn a_complete_recent_mod_list_does_not_clear_historic_risk_and_bad_versions_refuse() {
        let home = tempfile::tempdir().unwrap();
        let last_loaded = |version| {
            Tag::compound([(
                "ContentModList",
                Tag::List(vec![Tag::compound([
                    ("ModId", Tag::String("another_mod".into())),
                    ("ModVersion", version),
                ])]),
            )])
        };
        let review = || {
            Fence::hold(
                home.path(),
                SuspectMods::from([("removed".into(), BTreeSet::new())]),
                |_| "Lost Content",
                None,
            )
        };
        write_world(
            home.path(),
            "Old Village",
            8,
            last_loaded(Tag::String("1.0".into())),
        );
        assert_eq!(
            review().unwrap().1.affected,
            [WorldRisk {
                world: "Old Village".into(),
                mods: vec!["Lost Content".into()]
            }]
        );
        write_world(home.path(), "Old Village", 8, last_loaded(Tag::Int(1)));
        assert!(review().err().unwrap().contains("Old Village"));
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
