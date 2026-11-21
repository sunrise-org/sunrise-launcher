use serde::Deserialize;
use std::{collections::BTreeMap, path::Path};
use sunrise_launcher_smod::{
    index::{
        Index,
        resolve::{self, InstallPlan, InstalledMod, ModVersion, Request, RequestResult},
    },
    manifest::Side,
};

#[derive(Deserialize)]
struct Fixture {
    index: serde_json::Value,
    game: String,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    requests: Vec<Request>,
    installed: Vec<InstalledMod>,
    side: Side,
    native_fits: BTreeMap<String, BTreeMap<String, bool>>,
    expected: InstallPlan,
    refusals: BTreeMap<String, Vec<String>>,
    removal: Option<Removal>,
}

#[derive(Deserialize)]
struct Removal {
    id: String,
    expected: Vec<ModVersion>,
}

#[test]
fn whole_solver_exposes_the_template_and_keeps_the_registry_values_as_arguments() {
    use sunrise_launcher_smod::index::{Index, resolve};
    use sunrise_launcher_smod::manifest::Side;
    let index = Index::parse(include_str!("index/index-v1.json")).unwrap();
    let result = resolve::install_whole(
        &index,
        &[resolve::Request {
            id: "coords_hud".into(),
            version: Some("9.0.0".into()),
        }],
        &[],
        "26.3",
        Side::Both,
        resolve::WholeOptions {
            conflicts: &[],
            native_fits: |_: &_, _: &_| true,
            messages: |template, args| match template {
                "requested {version}" => format!("pinned {}", args[0].1),
                "{mod} ({requirements})" => format!("{}: {}", args[0].1, args[1].1),
                "{mod}: no listed version satisfies all requirements" => {
                    format!("not found: {}", args[0].1)
                }
                _ => panic!("unexpected template: {template}"),
            },
        },
    );
    assert!(
        matches!(result, Err(resolve::WholeRefusal::Inconsistent(reason)) if reason == "not found: coords_hud: pinned 9.0.0")
    );
}

#[test]
fn bulk_update_keeps_exact_unfit_roots_and_does_not_downgrade_dependencies() {
    use sunrise_launcher_smod::index::{Dependency, Kind, Project, Version};
    let mut index = Index::parse(include_str!("index/index-v1.json")).unwrap();
    let template = index.project("coords_hud").unwrap().clone();
    let project = |id: &str, kind: Kind, versions: &[&str]| Project {
        id: id.into(),
        kind,
        versions: versions
            .iter()
            .map(|version| Version {
                version: (*version).into(),
                dependencies: Some(Vec::new()),
                ..template.versions[0].clone()
            })
            .collect(),
        ..template.clone()
    };
    index.projects = vec![
        project(
            "native",
            Kind::NativeMod,
            &["1.0.0", "2.0.0", "3.0.0", "4.0.0"],
        ),
        project("sibling", Kind::Mod, &["1.0.0", "2.0.0"]),
        project("library", Kind::Mod, &["1.0.0", "2.0.0"]),
    ];
    let installed = [
        ("native", "2.0.0"),
        ("sibling", "1.0.0"),
        ("library", "2.0.0"),
    ]
    .map(|(id, version)| InstalledMod {
        id: id.into(),
        version: version.into(),
        sha256: Some(template.versions[0].file.sha256.clone()),
    });
    let minimum = installed
        .iter()
        .map(|m| (m.id.clone(), m.version.parse().unwrap()))
        .collect::<BTreeMap<_, semver::Version>>();
    let above = BTreeMap::from([
        ("native".into(), minimum["native"].clone()),
        ("sibling".into(), minimum["sibling"].clone()),
    ]);
    let bulk = |index: &Index, allow_intermediate: bool| {
        resolve::update_whole(
            index,
            resolve::UpdateTargets {
                above: &above,
                minimum: &minimum,
                retain_unfit: true,
            },
            &installed,
            "26.3",
            Side::Both,
            resolve::WholeOptions {
                conflicts: &[],
                native_fits: |_: &Project, v: &Version| match v.version.as_str() {
                    "4.0.0" => false,
                    "3.0.0" => allow_intermediate,
                    _ => true,
                },
                messages: |text, _| text.to_owned(),
            },
        )
    };
    let plan = bulk(&index, false).unwrap();
    assert_eq!(plan.update.len(), 1);
    assert_eq!(plan.update[0].id, "sibling");
    let plan = bulk(&index, true).unwrap();
    assert_eq!(plan.update.len(), 2);
    assert_eq!(
        (&*plan.update[0].id, &*plan.update[0].to),
        ("native", "3.0.0")
    );

    for id in ["native", "library"] {
        index.projects[1].versions[1].dependencies = Some(vec![Dependency {
            id: id.into(),
            version: "=1.0.0".into(),
            optional: false,
            side: None,
        }]);
        assert!(
            matches!(
                bulk(&index, false),
                Err(resolve::WholeRefusal::Inconsistent(_))
            ),
            "must not downgrade {id}"
        );
    }
}

#[test]
fn registry_resolution_fixtures() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/registry-resolution");
    let mut fixtures: Vec<_> = std::fs::read_dir(folder)
        .unwrap()
        .map(|file| file.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    fixtures.sort();
    assert!(!fixtures.is_empty());
    for file in fixtures {
        let Fixture { index, game, cases } =
            serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
        let index = Index::parse(&index.to_string()).unwrap();
        assert!(index.left_out.is_empty(), "{:?}", index.left_out);
        for Case {
            name,
            requests,
            installed,
            side,
            native_fits,
            expected,
            refusals,
            removal,
        } in cases
        {
            let install = |requests: &[Request]| {
                resolve::install(&index, requests, &installed, &game, side, |p, v| {
                    native_fits[&p.id][&v.version]
                })
            };
            let mut plan = install(&requests);
            // Reversing and rotating covers every order for the three-request recovery cases.
            let mut reordered = requests;
            for _ in 0..2 {
                reordered.reverse();
                for _ in 0..reordered.len() {
                    assert_eq!(install(&reordered), plan, "{name}: request order");
                    reordered.rotate_left(1);
                }
            }
            for (id, words) in refusals {
                let Some(RequestResult::Refused { reason }) = plan.requested.remove(&id) else {
                    panic!("{name}: expected a refusal for {id}: {plan:?}");
                };
                for word in words {
                    assert!(
                        reason.contains(&word),
                        "{name}: missing {word:?} in {reason:?}"
                    );
                }
            }
            assert_eq!(plan, expected, "{name}");
            if let Some(removal) = removal {
                assert_eq!(
                    resolve::removal_dependents(&index, &installed, &removal.id, side).unwrap(),
                    removal.expected,
                    "{name}",
                );
            }
        }
    }
}
