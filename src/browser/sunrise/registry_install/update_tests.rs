//! Exercise real staged Update with a local catalogue vouched for only by this test module.
use super::*;
use serde_json::json;
use std::{path::PathBuf, process::Command};
use sunrise_launcher_nbt::{self as nbt, Limits, NamedTag, Tag};
use sunrise_launcher_smod::Package;

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
mod native;

const BUILD: &str = "developer-local";
const LANTERN: &str = "sunrise-mods/lantern.smod";
const CORE: &str = "sunrise-mods/light_core.smod";
const AMBER: &str = "data/lantern/sunrise/block/amber.json";
const EARLIER_BLOCK: &[u8] = br#"{"properties":{"lit":[true,false]}}"#;

struct Fixture {
    root: PathBuf,
    instance: Instance,
    catalogue: index::Index,
}

impl Fixture {
    fn run(test: &str, scenario: impl FnOnce(Self)) {
        // Scope launcher paths to one libtest process, not the other unit tests. The
        // launcher binary has neither this worker nor the test authority constructor.
        const WORKER: &str = "SUNRISE_REGISTRY_UPDATE_TEST_ROOT";
        let Some(root) = std::env::var_os(WORKER).map(PathBuf::from) else {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let (_, test) = test.split_once("::").unwrap(); // libtest omits the crate name
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", test, "--nocapture", "--include-ignored"])
                .env(WORKER, &root)
                .env("SUNRISE_HOME", root.join("game"))
                .env("SUNRISE_LAUNCHER_HOME", root.join("launcher"))
                .env("XDG_CONFIG_HOME", root.join("config"))
                .env("XDG_DATA_HOME", root.join("xdg"))
                .env("SUNRISE_MOD_INDEX", root.join("index.json"))
                .env("SUNRISE_LAUNCHER_DEVELOPER_TOOL", root.join("sdk-tool"))
                .env("SUNRISE_LAUNCHER_DEVELOPER_SDK", root.join("sdk"))
                .env("FIXTURE_NATIVE_ROOT", &root)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success() && stdout.contains("running 1 test"),
                "{test}: {}\n{stdout}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        };
        let base = backend::version_dir(BUILD);
        let resources = backend::resources(BUILD);
        fs::create_dir_all(&resources).unwrap();
        fs::write(resources.join("sunrise-resources.json"), b"{}").unwrap();
        fs::write(base.join(".installed"), []).unwrap();
        fs::write(base.join(backend::CLIENT), b"fixture client").unwrap();
        fs::write(
            base.join("developer.json"),
            json!({"revision": "a".repeat(40)}).to_string(),
        )
        .unwrap();
        let instance = crate::instances::selection().unwrap();
        scenario(Self {
            root,
            instance,
            catalogue: index::Index {
                schema: index::SCHEMA,
                ..Default::default()
            },
        });
    }

    fn home(&self) -> &Path {
        &self.instance.home
    }

    fn project(&mut self, id: &str, name: &str, kind: index::Kind) {
        self.catalogue.projects.push(
            serde_json::from_value(json!({
                "id": id, "name": name, "kind": kind, "summary": "Test mod",
                "license": "MIT", "source": "https://example.invalid/", "versions": []
            }))
            .unwrap(),
        );
    }

    fn release(
        &mut self,
        id: &str,
        version: &str,
        tables: &str,
        files: &[(&str, &[u8])],
    ) -> Vec<u8> {
        let project = self
            .catalogue
            .projects
            .iter_mut()
            .find(|p| p.id == id)
            .unwrap();
        let name = &project.name;
        let manifest = format!(
            "id='{id}'\nname='{name}'\nversion='{version}'\napi='{}'\nside='both'\ncapabilities=['registries']\n{tables}",
            sunrise_launcher_smod::manifest::API
        );
        let bytes = Package::write_files(&manifest, files).unwrap();
        let name = format!("{id}-{version}.smod");
        let archive = self.root.join(&name);
        fs::write(&archive, &bytes).unwrap();
        let facts = index::inspect(&project.kind, &archive).unwrap();
        project.versions.insert(
            0,
            index::Version {
                version: version.into(),
                published: String::new(),
                changelog: String::new(),
                api: facts.api,
                side: facts.side,
                capabilities: facts.capabilities.into(),
                game: backend::PINNED_GAME.into(),
                requires: Vec::new(),
                dependencies: Some(
                    facts
                        .dependencies
                        .into_iter()
                        .map(|(id, dependency)| {
                            let details = dependency.details();
                            index::Dependency {
                                id,
                                version: details.version.to_string(),
                                optional: details.optional,
                                side: (details.side != Side::Both).then_some(details.side),
                            }
                        })
                        .collect(),
                ),
                native_binaries: Vec::new(),
                file: index::File {
                    name,
                    url: format!("file://{}", archive.display()),
                    size: bytes.len() as u64,
                    sha256: index::sha256(&bytes),
                    signature: None,
                },
            },
        );
        fs::write(
            self.root.join("index.json"),
            serde_json::to_vec(&self.catalogue).unwrap(),
        )
        .unwrap();
        bytes
    }

    fn apply(&self, action: Action<'_>) -> Result<InstalledBatch, InstallError> {
        registry_edit(
            &self.instance,
            install_target(&self.instance).unwrap(),
            || action,
            || FirstBuild::installed(BUILD),
            InstallHooks {
                progress: &|_| {},
                native: &|question| panic!("unexpected native consent question: {question:?}"),
                registry_crates: None,
                continue_after_failure: &|_| false,
            },
            |source, local| {
                let (catalogue, bytes, authority) = fetch_with_official(source, local)?;
                assert!(
                    authority.is_none(),
                    "a local override must never carry authority"
                );
                let authority = OfficialRegistry::fixture(&catalogue);
                Ok((catalogue, bytes, Some(authority)))
            },
        )
    }

    fn install(&self, ids: &[&str]) {
        self.apply(Action::Install(projects(ids))).unwrap();
    }

    fn update(
        &self,
        ids: &[&str],
        approve: &dyn Fn(&ChangeImpact) -> bool,
    ) -> Result<InstalledBatch, InstallError> {
        self.apply(Action::Update(
            UpdateRequest::Strict(projects(ids)),
            approve,
        ))
    }

    fn snapshot(&self, archives: &[&str]) -> Vec<Vec<u8>> {
        [
            crate::profile_txn::POINTER,
            installed::MANIFEST,
            sunrise_launcher_smod::mod_list::FILE,
        ]
        .into_iter()
        .chain(archives.iter().copied())
        .map(|path| fs::read(self.home().join(path)).unwrap())
        .collect()
    }
}

fn projects(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).into()).collect()
}

fn no_world_question(question: &ChangeImpact) -> bool {
    panic!("unexpected world warning: {}", question.detail());
}

fn assert_world_warning(detail: &str, count: usize) {
    let warning = format!(
        concat!(
            "You may lose blocks and items from the affected mods when you open ",
            "{} saved worlds without their definitions."
        ),
        count
    );
    assert_eq!(detail.matches(&warning).count(), 1, "{detail}");
    assert!(
        detail.contains(
            "Sunrise can't tell which affected blocks and items are still in those worlds."
        ),
        "{detail}"
    );
}

fn roster(id: &str) -> Tag {
    Tag::compound([(
        "ContentModList",
        Tag::List(vec![Tag::compound([
            ("ModId", Tag::String(id.into())),
            ("ModVersion", Tag::String("1.0.0".into())),
        ])]),
    )])
}

fn world(home: &Path, folder: &str, sunrise: Tag) {
    let save = home.join("saves").join(folder);
    fs::create_dir_all(&save).unwrap();
    let root = Tag::compound([
        (
            "Data",
            Tag::compound([("LevelName", Tag::String(folder.into()))]),
        ),
        ("sunrise", sunrise),
    ]);
    fs::write(
        save.join("level.dat"),
        nbt::write_named(
            &NamedTag {
                name: "".into(),
                tag: root,
            },
            Limits::saved(),
        )
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn unchanged_leaf_advances_quietly_but_changed_definition_requires_approval() {
    Fixture::run(
        concat!(
            module_path!(),
            "::unchanged_leaf_advances_quietly_but_changed_definition_requires_approval"
        ),
        |mut fixture| {
            fixture.project("lantern", "Lanterns", index::Kind::Mod);
            fixture.release("lantern", "1.0.0", "", &[(AMBER, EARLIER_BLOCK)]);
            fixture.install(&["lantern"]);
            world(fixture.home(), "Uses Lanterns", roster("lantern"));
            // Even a valid roster with no match cannot prove historic absence.
            world(fixture.home(), "Other mods", roster("another_mod"));
            let unchanged = fixture.release("lantern", "1.1.0", "", &[(AMBER, EARLIER_BLOCK)]);
            let batch = fixture.update(&["lantern"], &no_world_question).unwrap();
            assert_eq!(batch.updated.len(), 1);
            assert_eq!(batch.updated[0].1.version, "1.1.0");
            assert_eq!(fs::read(fixture.home().join(LANTERN)).unwrap(), unchanged);

            let changed = fixture.release("lantern", "1.2.0", "", &[(AMBER, b"{}")]);
            let old = fixture.snapshot(&[LANTERN]);
            let questions = std::cell::Cell::new(0);
            let decide = |approved| {
                fixture.update(&["lantern"], &|impact| {
                    questions.set(questions.get() + 1);
                    assert!(impact.risks_worlds() && !impact.is_removal());
                    let detail = impact.detail();
                    assert!(detail.contains("Lanterns 1.1.0 → 1.2.0"), "{detail}");
                    assert_world_warning(&detail, 2);
                    approved
                })
            };
            assert!(matches!(decide(false), Err(InstallError::Cancelled)));
            assert_eq!(questions.get(), 1);
            assert_eq!(fixture.snapshot(&[LANTERN]), old);
            assert_eq!(decide(true).unwrap().updated.len(), 1);
            assert_eq!(questions.get(), 2);
            assert_eq!(fs::read(fixture.home().join(LANTERN)).unwrap(), changed);
            assert_ne!(fixture.snapshot(&[LANTERN])[0], old[0]);
        },
    );
}

#[test]
fn install_names_an_existing_mod_when_its_replacement_would_risk_saved_worlds() {
    Fixture::run(
        concat!(
            module_path!(),
            "::install_names_an_existing_mod_when_its_replacement_would_risk_saved_worlds"
        ),
        |mut fixture| {
            fixture.project("lantern", "Amber Lanterns", index::Kind::Mod);
            fixture.release("lantern", "1.0.0", "", &[(AMBER, EARLIER_BLOCK)]);
            fixture.install(&["lantern"]);
            world(fixture.home(), "Uses Lanterns", roster("lantern"));
            fixture.release("lantern", "1.1.0", "", &[(AMBER, b"{}")]);
            fixture.project("lamp_post", "Lamp Posts", index::Kind::Mod);
            fixture.release(
                "lamp_post",
                "1.0.0",
                "[dependencies]\nlantern='^1.1'\n",
                &[],
            );

            let before = fixture.snapshot(&[LANTERN]);
            let error = fixture
                .apply(Action::Install(projects(&["lamp_post"])))
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "Amber Lanterns is already installed. Update it from Installed."
            );
            assert_eq!(fixture.snapshot(&[LANTERN]), before);
            assert!(!fixture.home().join("sunrise-mods/lamp_post.smod").exists());
        },
    );
}

#[test]
fn bulk_update_reviews_dependency_changes_once_for_all_worlds() {
    Fixture::run(
        concat!(
            module_path!(),
            "::bulk_update_reviews_dependency_changes_once_for_all_worlds"
        ),
        |mut fixture| {
            const DEPENDS: &str = "[dependencies]\nlight_core='^1'\n";
            fixture.project("light_core", "Light Core", index::Kind::Mod);
            fixture.project("lantern", "Lanterns", index::Kind::Mod);
            fixture.release(
                "light_core",
                "1.0.0",
                "",
                &[("data/light_core/sunrise/item/amber.json", b"{}")],
            );
            fixture.release("lantern", "1.0.0", DEPENDS, &[(AMBER, EARLIER_BLOCK)]);
            fixture.install(&["lantern"]);
            world(fixture.home(), "Dependent", roster("lantern"));
            world(fixture.home(), "Unrelated", roster("another_mod"));
            world(
                fixture.home(),
                "Namespaces only",
                Tag::compound([(
                    "BlockNamespaces",
                    Tag::List(vec![Tag::String("light_core".into())]),
                )]),
            );
            let malformed = fixture.home().join("saves/Malformed");
            fs::create_dir(&malformed).unwrap();
            fs::write(malformed.join("level.dat"), b"invalid data").unwrap();

            let next_core = fixture.release("light_core", "1.1.0", "", &[]);
            let before = fixture.snapshot(&[LANTERN, CORE]);
            assert!(matches!(
                fixture.update(&["light_core"], &|impact| {
                    let detail = impact.detail();
                    assert!(detail.contains("Light Core 1.0.0 → 1.1.0"), "{detail}");
                    assert_world_warning(&detail, 4);
                    false
                }),
                Err(InstallError::Cancelled)
            ));
            assert_eq!(fixture.snapshot(&[LANTERN, CORE]), before);

            let next_lantern = fixture.release("lantern", "1.1.0", DEPENDS, &[(AMBER, b"{}")]);
            let calls = std::cell::Cell::new(0);
            let decide = |approved| {
                fixture.apply(Action::Update(
                    UpdateRequest::Bulk(projects(&["light_core", "lantern"])),
                    &|impact| {
                        calls.set(calls.get() + 1);
                        let detail = impact.detail();
                        assert!(detail.contains("Lanterns 1.0.0 → 1.1.0"), "{detail}");
                        assert!(detail.contains("Light Core 1.0.0 → 1.1.0"), "{detail}");
                        assert_world_warning(&detail, 4);
                        approved
                    },
                ))
            };
            assert!(matches!(decide(false), Err(InstallError::Cancelled)));
            assert_eq!(calls.get(), 1);
            assert_eq!(fixture.snapshot(&[LANTERN, CORE]), before);
            assert_eq!(decide(true).unwrap().updated.len(), 2);
            assert_eq!(calls.get(), 2);
            assert_eq!(
                fs::read(fixture.home().join(LANTERN)).unwrap(),
                next_lantern
            );
            assert_eq!(fs::read(fixture.home().join(CORE)).unwrap(), next_core);
        },
    );
}

#[test]
fn approved_update_refuses_changed_worlds_or_catalogue_at_cutoff() {
    Fixture::run(
        concat!(
            module_path!(),
            "::approved_update_refuses_changed_worlds_or_catalogue_at_cutoff"
        ),
        |mut fixture| {
            fixture.project("lantern", "Lanterns", index::Kind::Mod);
            fixture.release("lantern", "1.0.0", "", &[(AMBER, EARLIER_BLOCK)]);
            fixture.install(&["lantern"]);
            fixture.release("lantern", "1.1.0", "", &[]);
            world(fixture.home(), "First world", roster("lantern"));
            let before = fixture.snapshot(&[LANTERN]);
            for replace in [false, true] {
                let error = fixture
                    .update(&["lantern"], &|_| {
                        if replace {
                            // Identical level bytes in a substituted save are still a different world.
                            fs::rename(
                                fixture.home().join("saves/First world"),
                                fixture.root.join("old-save"),
                            )
                            .unwrap();
                            world(fixture.home(), "First world", roster("lantern"));
                        } else {
                            world(fixture.home(), "New world", roster("lantern"));
                        }
                        true
                    })
                    .unwrap_err();
                assert!(
                    error
                        .to_string()
                        .contains("The affected worlds changed after review."),
                    "{error}"
                );
                assert_eq!(fixture.snapshot(&[LANTERN]), before);
            }
            let error = fixture
                .update(&["lantern"], &|_| {
                    let path = fixture.root.join("index.json");
                    let text = fs::read_to_string(&path).unwrap();
                    fs::write(path, format!("{text}\n")).unwrap();
                    true
                })
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("The mod registry changed while the install was being prepared."),
                "{error}"
            );
            assert_eq!(fixture.snapshot(&[LANTERN]), before);
        },
    );
}
