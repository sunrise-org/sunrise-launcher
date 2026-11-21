//! Player-facing copy, separate from the exact archive details shown by command-line clients.
use super::Change;
use crate::i18n::{tr, trf};
use std::collections::BTreeMap;

impl super::ReviewedMod {
    fn gui_name<'a>(&'a self, names: &'a BTreeMap<String, String>) -> &'a str {
        names.get(&self.id).unwrap_or(&self.name).as_str()
    }
}

impl Change {
    pub(super) fn gui_review(
        &self,
        world_count: usize,
        unchecked_update: bool,
        displayed_names: &BTreeMap<String, String>,
    ) -> (String, String) {
        let mut lines = Vec::new();
        let (heading, one_mod) = match self {
            Self::Update(updates) => {
                let mut updated = Vec::new();
                for update in updates {
                    if let Some(installed) = &update.installed {
                        updated.push(update.name.as_str());
                        lines.push(trf(
                            "{name} will go from {old} to {new}.",
                            &[
                                ("old", installed),
                                ("new", &update.replacement),
                                ("name", &update.name),
                            ],
                        ));
                    } else {
                        lines.push(trf(
                            "{name} {version} will also be installed.",
                            &[("version", &update.replacement), ("name", &update.name)],
                        ));
                    }
                }
                let heading = match updated.as_slice() {
                    [name] => trf("Update {name}?", &[("name", name)]),
                    _ => trf(
                        "Update {count} mods?",
                        &[("count", &updated.len().to_string())],
                    ),
                };
                (heading, updated.len() == 1)
            }
            Self::GithubUpdate { target, .. } => {
                lines.push(
                    tr("This replaces the installed version with the GitHub release you selected.")
                        .into(),
                );
                (trf("Update {name}?", &[("name", &target.name)]), true)
            }
            Self::Remove { removed, disabled } => {
                let heading = match removed.as_slice() {
                    [archive] => trf(
                        "Remove {name}?",
                        &[("name", archive.target.gui_name(displayed_names))],
                    ),
                    _ => {
                        let names: Vec<_> = removed
                            .iter()
                            .map(|archive| archive.target.gui_name(displayed_names))
                            .collect();
                        lines.push(trf("Remove: {mods}.", &[("mods", &joined_names(&names))]));
                        trf(
                            "Remove {count} mods?",
                            &[("count", &removed.len().to_string())],
                        )
                    }
                };
                match (removed.len() == 1, disabled.as_slice()) {
                    (_, []) => (),
                    (true, [one]) => lines.push(trf(
                        "{name} needs it. It'll be turned off, and its files stay installed.",
                        &[("name", one.gui_name(displayed_names))],
                    )),
                    (false, [one]) => lines.push(trf(
                        "{name} needs one or more of these mods. It'll be turned off, and its files stay installed.",
                        &[("name", one.gui_name(displayed_names))],
                    )),
                    (single_target, many) => {
                        let names = many.iter().map(|item| item.gui_name(displayed_names)).collect::<Vec<_>>();
                        let names = joined_names(&names);
                        lines.push(if single_target {
                            trf("{names} need it. They'll be turned off, and their files stay installed.", &[("names", &names)])
                        } else {
                            trf("{names} need one or more of these mods. They'll be turned off, and their files stay installed.", &[("names", &names)])
                        });
                    }
                }
                (heading, removed.len() == 1 && disabled.is_empty())
            }
        };
        match (world_count, one_mod) {
            (0, _) if unchecked_update => lines.push(
                tr("Sunrise can't check whether this update removes blocks or items your worlds use.")
                    .into(),
            ),
            (0, _) => (),
            (1, true) => {
                lines.push(tr("1 saved world may lose blocks and items from this mod.").into())
            }
            (1, false) => {
                lines.push(tr("1 saved world may lose blocks and items from these mods.").into())
            }
            (count, true) => lines.push(trf(
                "{count} saved worlds may lose blocks and items from this mod.",
                &[("count", &count.to_string())],
            )),
            (count, false) => lines.push(trf(
                "{count} saved worlds may lose blocks and items from these mods.",
                &[("count", &count.to_string())],
            )),
        }
        (heading, lines.join("\n\n"))
    }
}

fn joined_names(names: &[&str]) -> String {
    match names {
        [rest @ .., last] if !rest.is_empty() => trf(
            "{names} and {last}",
            &[("names", &rest.join(", ")), ("last", last)],
        ),
        _ => names.join(", "),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        ChangeImpact, ReviewedArchive, ReviewedMod, ReviewedRelease, ReviewedUpdate,
    };
    use super::*;
    use crate::browser::installed::{colliding_mod_labels, ModLabel, ModLabelSource};

    fn item(name: &str, id: &str) -> ReviewedMod {
        ReviewedMod {
            name: name.into(),
            id: id.into(),
        }
    }

    fn remove(targets: &[(&str, &str)], disabled: Vec<ReviewedMod>) -> Change {
        Change::Remove {
            removed: targets
                .iter()
                .map(|&(name, id)| ReviewedArchive {
                    target: item(name, id),
                    file: format!("sunrise-mods/{id}.smod"),
                    digest: "reviewed digest".into(),
                })
                .collect(),
            disabled,
        }
    }

    #[test]
    fn single_removal_only_explains_the_consequences_that_apply() {
        let core = [("Lantern Core", "lantern_core")];
        let bare = remove(&core, vec![]);
        assert_eq!(
            bare.gui_review(0, false, &BTreeMap::new()),
            (String::from("Remove Lantern Core?"), String::new())
        );
        assert_eq!(
            bare.gui_review(1, false, &BTreeMap::new()).1,
            "1 saved world may lose blocks and items from this mod."
        );
        let one = remove(&core, vec![item("Fireflies", "fireflies")]);
        assert_eq!(
            one.gui_review(1, false, &BTreeMap::new()).1,
            concat!(
                "Fireflies needs it. It'll be turned off, and its files stay installed.\n\n",
                "1 saved world may lose blocks and items from these mods."
            )
        );
        let two = remove(
            &core,
            vec![item("Fireflies", "fireflies"), item("Lanterns", "lanterns")],
        );
        assert_eq!(two.gui_review(3, false, &BTreeMap::new()).1, concat!(
            "Fireflies and Lanterns need it. They'll be turned off, and their files stay installed.\n\n",
            "3 saved worlds may lose blocks and items from these mods."
        ));
    }

    #[test]
    fn bulk_removal_names_the_targets_by_display_name() {
        let change = remove(
            &[
                ("Lantern Core", "lantern_core"),
                ("Night Lights (Deluxe)", "night_lights"),
                ("Torches", "torches"),
            ],
            vec![item("Fireflies", "fireflies"), item("Lanterns", "lanterns")],
        );
        let (heading, detail) = change.gui_review(0, false, &BTreeMap::new());
        assert_eq!(heading, "Remove 3 mods?");
        assert_eq!(
            detail,
            concat!(
                "Remove: Lantern Core, Night Lights (Deluxe) and Torches.\n\n",
                "Fireflies and Lanterns need one or more of these mods. ",
                "They'll be turned off, and their files stay installed."
            )
        );
    }

    #[test]
    fn removal_disambiguates_only_duplicate_display_names_without_paths() {
        let targets = [
            ("Lanterns", "lanterns"),
            ("Lanterns", "lanterns_extra"),
            ("Fireflies", "fireflies"),
            ("Fireflies", "fireflies_extra"),
            ("Torches", "torches"),
        ];
        let authors = ["Ann", "Bo", "Ann", "Ann", "Ann"].map(|name| [name.to_owned()]);
        let versions = ["1.0.0", "1.0.0", "1.1.0", "1.2.0", "1.0.0"];
        let labels = colliding_mod_labels(targets.iter().zip(&authors).zip(versions).map(
            |((&(name, id), authors), version)| ModLabel {
                id,
                name,
                authors,
                version,
                source: ModLabelSource::Registry,
            },
        ));
        let names: std::collections::BTreeMap<_, _> = targets
            .iter()
            .zip(labels)
            .filter_map(|(&(_, id), label)| label.map(|label| (id.to_owned(), label)))
            .collect();

        let impact = ChangeImpact {
            change: remove(&targets, vec![]),
            evidence: vec![],
            unchecked_update: false,
        };
        assert_eq!(
            impact.gui_review(&names),
            (
                "Remove 5 mods?".into(),
                concat!(
                    "Remove: Lanterns · by Ann, Lanterns · by Bo, ",
                    "Fireflies · 1.1.0, Fireflies · 1.2.0 and Torches."
                )
                .into()
            )
        );
        let detail = impact.detail();
        assert!(detail.contains("sunrise-mods/torches.smod\nSHA-256: reviewed digest"));

        // The other namesake is installed and will be disabled, not removed.
        let one = remove(&targets[..1], vec![item("Lanterns", "lanterns_extra")]);
        assert_eq!(
            one.gui_review(0, false, &names),
            (
                "Remove Lanterns · by Ann?".into(),
                "Lanterns · by Bo needs it. It'll be turned off, and its files stay installed."
                    .into(),
            )
        );
    }

    #[test]
    fn removal_labels_identical_metadata_by_id_without_exposing_archive_identity() {
        // Installed can show b above a (e.g. different status groups). Remove uses ID order.
        let targets = [("Lanterns", "lanterns_a"), ("Lanterns", "lanterns_b")];
        let authors = ["Ann".to_owned()];
        let label = |(name, id), source| ModLabel {
            id,
            name,
            authors: &authors,
            version: "1.0.0",
            source,
        };
        let installed = colliding_mod_labels(
            targets
                .into_iter()
                .rev()
                .map(|target| label(target, ModLabelSource::Local)),
        );
        assert_eq!(
            installed,
            [Some("Lanterns · 2".into()), Some("Lanterns · 1".into())]
        );
        let sources = colliding_mod_labels([
            label(targets[0], ModLabelSource::Registry),
            label(targets[1], ModLabelSource::Github),
            label(("Lanterns", "lanterns_c"), ModLabelSource::Local),
        ]);
        assert_eq!(
            sources,
            [
                Some("Lanterns · Sunrise registry".into()),
                Some("Lanterns · GitHub".into()),
                Some("Lanterns · Local file".into())
            ]
        );

        let mut change = remove(&targets, vec![]);
        let Change::Remove { removed, .. } = &mut change else {
            unreachable!();
        };
        // The exact files/digests are distinct and need not sort like manifest IDs.
        removed[0].file = "sunrise-mods/z.smod".into();
        removed[0].digest = "a".repeat(64);
        removed[1].file = "sunrise-mods/a.smod".into();
        removed[1].digest = "b".repeat(64);
        let labels = colliding_mod_labels(
            targets
                .into_iter()
                .map(|target| label(target, ModLabelSource::Local)),
        );
        assert!(labels.iter().eq(installed.iter().rev()));
        let names = targets
            .into_iter()
            .zip(labels)
            .filter_map(|((_, id), label)| label.map(|label| (id.to_owned(), label)))
            .collect();
        let impact = ChangeImpact {
            change,
            evidence: vec![],
            unchecked_update: false,
        };
        assert_eq!(
            impact.gui_review(&names),
            (
                "Remove 2 mods?".into(),
                "Remove: Lanterns · 1 and Lanterns · 2.".into()
            )
        );
        let detail = impact.detail();
        for (file, digest) in [("z.smod", "a"), ("a.smod", "b")] {
            assert!(detail.contains(&format!(
                "sunrise-mods/{file}\nSHA-256: {}",
                digest.repeat(64)
            )));
        }
    }

    #[test]
    fn removal_uses_combinations_before_numbering_namesakes() {
        let ids = ["ann_first", "ann_second", "lee_first", "lee_second"];
        let ann = ["Ann".to_owned()];
        let lee = ["Lee".to_owned()];
        let labels = |authors: [&[String]; 4], versions: [&str; 4], sources| {
            colliding_mod_labels(ids.into_iter().zip(authors).zip(versions).zip(sources).map(
                |(((id, authors), version), source)| ModLabel {
                    id,
                    name: "Lanterns",
                    authors,
                    version,
                    source,
                },
            ))
        };
        let authors = [
            ann.as_slice(),
            ann.as_slice(),
            lee.as_slice(),
            lee.as_slice(),
        ];
        let versions = ["1.0.0", "1.0.1", "1.0.0", "1.0.1"];
        assert_eq!(
            labels(authors, versions, [ModLabelSource::Local; 4]),
            [
                Some("Lanterns · by Ann · 1.0.0".into()),
                Some("Lanterns · by Ann · 1.0.1".into()),
                Some("Lanterns · by Lee · 1.0.0".into()),
                Some("Lanterns · by Lee · 1.0.1".into()),
            ]
        );
        let sources = [
            ModLabelSource::Local,
            ModLabelSource::Github,
            ModLabelSource::Local,
            ModLabelSource::Github,
        ];
        assert_eq!(
            labels([&[]; 4], ["1.0.0", "1.0.0", "1.0.1", "1.0.1"], sources),
            [
                Some("Lanterns · 1.0.0 · Local file".into()),
                Some("Lanterns · 1.0.0 · GitHub".into()),
                Some("Lanterns · 1.0.1 · Local file".into()),
                Some("Lanterns · 1.0.1 · GitHub".into()),
            ]
        );
        assert_eq!(
            labels(authors, [""; 4], sources),
            [
                Some("Lanterns · by Ann · Local file".into()),
                Some("Lanterns · by Ann · GitHub".into()),
                Some("Lanterns · by Lee · Local file".into()),
                Some("Lanterns · by Lee · GitHub".into()),
            ]
        );
    }

    #[test]
    fn hidden_unpacked_namesake_does_not_renumber_the_reviewed_archive() {
        use super::super::inventory;
        use crate::native_status;
        use std::fs;
        use sunrise_launcher_smod::{manifest, mod_list, Package};

        let scratch = tempfile::tempdir().unwrap();
        let home = scratch.path().canonicalize().unwrap();
        let mods = home.join(mod_list::DIR);
        let manifest = |id| {
            format!(
            "id='{id}'\nname='Lanterns'\nauthors=['Ann']\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n"
        )
        };
        fs::create_dir_all(mods.join("a")).unwrap();
        fs::write(mods.join("a").join(manifest::MANIFEST), manifest("a")).unwrap();
        for id in ["b", "c"] {
            fs::write(
                mods.join(format!("{id}.smod")),
                Package::write_files(&manifest(id), &[]).unwrap(),
            )
            .unwrap();
        }
        let scanned = inventory(&home, None).unwrap();
        assert_eq!(scanned.len(), 3);
        assert!(scanned["a"].package.directory);
        // Installed lists regular archives only, even when an unpacked namesake is valid.
        let visible = native_status::untracked_content(&home);
        assert_eq!(visible.len(), 2);
        let labels = colliding_mod_labels(visible.iter().map(|file| ModLabel {
            id: &file.id,
            name: &file.name,
            authors: &file.authors,
            version: &file.version,
            source: ModLabelSource::Local,
        }));
        let names: BTreeMap<_, _> = visible
            .into_iter()
            .zip(labels)
            .map(|(file, label)| (file.id, label.unwrap()))
            .collect();
        assert!(!names.contains_key("a"));
        assert_eq!(names["b"], "Lanterns · 1");
        assert_eq!(names["c"], "Lanterns · 2");
        let impact = ChangeImpact {
            change: remove(&[("Lanterns", "b")], vec![]),
            evidence: vec![],
            unchecked_update: false,
        };
        assert_eq!(impact.gui_review(&names).0, "Remove Lanterns · 1?");
    }

    #[test]
    fn registry_update_distinguishes_a_new_dependency_from_an_update() {
        let change = Change::Update(vec![
            ReviewedUpdate {
                name: "Lanterns".into(),
                installed: Some("1.0.0".into()),
                replacement: "1.1.0".into(),
            },
            ReviewedUpdate {
                name: "Lantern Core".into(),
                installed: None,
                replacement: "1.0.0".into(),
            },
        ]);
        let (heading, detail) = change.gui_review(2, false, &BTreeMap::new());
        assert_eq!(heading, "Update Lanterns?");
        assert_eq!(
            detail,
            concat!(
                "Lanterns will go from 1.0.0 to 1.1.0.\n\n",
                "Lantern Core 1.0.0 will also be installed.\n\n",
                "2 saved worlds may lose blocks and items from this mod."
            )
        );
    }

    #[test]
    fn github_gui_does_not_show_archive_identity_but_detail_does() {
        let impact = ChangeImpact {
            change: Change::GithubUpdate {
                target: item("Lanterns", "lanterns"),
                file: "sunrise-mods/lanterns.smod".into(),
                releases: [
                    ReviewedRelease {
                        label: "old release".into(),
                        sha256: "old digest".into(),
                    },
                    ReviewedRelease {
                        label: "new release".into(),
                        sha256: "new digest".into(),
                    },
                ],
            },
            evidence: vec![],
            unchecked_update: true,
        };
        let (heading, warning) = impact.gui_review(&BTreeMap::new());
        assert_eq!(heading, "Update Lanterns?");
        assert_eq!(
            warning,
            concat!(
                "This replaces the installed version with the GitHub release you selected.\n\n",
                "Sunrise can't check whether this update removes blocks or items your worlds use."
            )
        );
        assert!(!impact.risks_worlds());
        assert_eq!(
            impact.detail(),
            concat!(
            "Update: Lanterns (lanterns).\n\n",
            "Installed archive: old release\nsunrise-mods/lanterns.smod\nSHA-256: old digest\n\n",
            "Replacement archive: new release\nsunrise-mods/lanterns.smod\nSHA-256: new digest\n\n",
            "Sunrise can't check whether this update removes blocks or items your worlds use."
        )
        );
    }
}
