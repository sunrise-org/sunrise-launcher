//! Exercise real staged Update with a local catalogue vouched for only by this test module.
use super::*;
use crate::{attestation, backend};
use serde_json::json;
use std::{path::PathBuf, process::Command};
use sunrise_launcher_nbt::{self as nbt, Limits, NamedTag, Tag};
use sunrise_launcher_smod::index_v2::{self, Kind as V2Kind};
use sunrise_launcher_smod::Package;
use v2::{install_registry_v2_with, update_registry_v2_with, V2EditRequest, V2Transfer};

mod build_switch;
#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
mod native;
#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
mod source_dependencies;
#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
mod source_picker;

const BUILD: &str = "developer-local";
const LANTERN: &str = "sunrise-mods/lantern.smod";
const CORE: &str = "sunrise-mods/light_core.smod";
const AMBER: &str = "data/lantern/sunrise/block/amber.json";
const EARLIER_BLOCK: &[u8] = br#"{"properties":{"lit":[true,false]}}"#;

struct Fixture {
    root: PathBuf,
    instance: Instance,
    catalogue: index_v2::Index,
}

/// A local test action: the v2 install or update the GUI would run.
enum Action<'a> {
    Install(Vec<String>),
    Update(UpdateRequest, &'a dyn Fn(&ChangeImpact) -> bool),
}

/// The fixture's 8-character project ID for in-game mod `id`.
fn project_id(id: &str) -> String {
    let mut out: String = id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(8)
        .collect();
    while out.len() < 8 {
        out.push('0');
    }
    out
}

const FIXTURE_REPOSITORY: &str = "fixture/mod";

fn fixture_workflow() -> String {
    format!(
        "{}@refs/tags/release-v1",
        attestation::RELEASE_WORKFLOW_PATH
    )
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
                .env("SUNRISE_LAUNCHER_DEVELOPER_OFFLINE", "1")
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
            catalogue: index_v2::Index {
                schema: index_v2::SCHEMA,
                serial: 1,
                generated: "2026-11-24T00:00:00Z".into(),
                builds: vec![index_v2::Build {
                    id: BUILD.into(),
                    commit: "a".repeat(40),
                }],
                categories: Vec::new(),
                projects: Vec::new(),
                revoked: Vec::new(),
                collections: Vec::new(),
            },
        });
    }

    fn home(&self) -> &Path {
        &self.instance.home
    }

    /// The catalogue as offered on `base`: every release fits it as well as the builds it
    /// was published for.
    fn index_for(&self, base: &str) -> Arc<index_v2::Index> {
        let mut index = self.catalogue.clone();
        if !index.builds.iter().any(|build| build.id == base) {
            index.builds.push(index_v2::Build {
                id: base.into(),
                commit: "a".repeat(40),
            });
            for project in &mut index.projects {
                for release in &mut project.releases {
                    release.fits.push(base.into());
                }
            }
        }
        Arc::new(index)
    }

    fn project(&mut self, id: &str, name: &str, kind: V2Kind) {
        let project_id = project_id(id);
        assert!(
            self.catalogue.project(&project_id).is_none(),
            "fixture project IDs must be unique: {id}"
        );
        let repository = self.catalogue.projects.len() as u64 + 1000;
        self.catalogue.projects.push(index_v2::Project {
            id: project_id.clone(),
            slug: id.into(),
            mod_id: id.into(),
            kind,
            name: name.into(),
            summary: "Test mod".into(),
            description: String::new(),
            authors: vec!["tester".into()],
            owners: vec!["tester".into()],
            license: "MIT".into(),
            homepage: None,
            issues: None,
            source: None,
            icon: format!("pages/{project_id}/{}.png", "0".repeat(64)),
            screenshots: Vec::new(),
            categories: Vec::new(),
            repository: index_v2::Repository {
                id: repository,
                name: FIXTURE_REPOSITORY.into(),
                path: ".".into(),
            },
            workflow: attestation::RELEASE_WORKFLOW_PATH.into(),
            releases: Vec::new(),
        });
    }

    fn release(
        &mut self,
        id: &str,
        version: &str,
        tables: &str,
        files: &[(&str, &[u8])],
    ) -> Vec<u8> {
        let fits = self
            .catalogue
            .builds
            .iter()
            .map(|build| build.id.clone())
            .collect();
        let project = self
            .catalogue
            .projects
            .iter_mut()
            .find(|p| p.mod_id == id)
            .unwrap();
        let name = &project.name;
        let manifest = format!(
            "id='{id}'\nname='{name}'\nversion='{version}'\napi='{}'\nside='both'\ncapabilities=['registries']\n{tables}",
            sunrise_launcher_smod::manifest::API
        );
        let bytes = Package::write_files(&manifest, files).unwrap();
        let archive = self.root.join(format!("{id}-{version}.smod"));
        fs::write(&archive, &bytes).unwrap();
        let dependencies = Package::read(&archive)
            .unwrap()
            .manifest
            .dependencies
            .into_iter()
            .map(|(mod_id, dependency)| {
                let details = dependency.details();
                index_v2::Dependency {
                    mod_id,
                    version: details.version.to_string().parse().unwrap(),
                    project: None,
                    required: !details.optional,
                }
            })
            .collect();
        project.releases.insert(
            0,
            index_v2::Release {
                version: version.into(),
                published: "2026-11-24".into(),
                sunrise: BUILD.into(),
                workflow: fixture_workflow(),
                fits,
                does_not_fit: Vec::new(),
                file: index_v2::File {
                    url: format!("file://{}", archive.display()),
                    size: bytes.len() as u64,
                    sha256: index::sha256(&bytes),
                    attestation: format!("file://{}.sigstore.json", archive.display()),
                },
                dependencies,
            },
        );
        bytes
    }

    fn apply(&self, action: Action<'_>) -> Result<InstalledBatch, InstallError> {
        self.apply_on(BUILD, action)
    }

    fn apply_on(&self, base: &str, action: Action<'_>) -> Result<InstalledBatch, InstallError> {
        let request = V2EditRequest {
            instance: &self.instance,
            identity: install_target(&self.instance).unwrap(),
            index: self.index_for(base),
            options: V2InstallOptions::default(),
            transfer: V2Transfer {
                fetch_bytes: &fetch_fixture,
                verify_release: &verify_fixture,
            },
        };
        let hooks = V2Hooks {
            install: InstallHooks {
                progress: &|_| {},
                continue_after_failure: &|_| false,
                cancellation: None,
            },
            choose_provider: None,
            confirm_publisher_change: None,
        };
        let choose = || FirstBuild::installed(base);
        match action {
            Action::Install(ids) => install_registry_v2_with(request, ids, choose, hooks),
            Action::Update(requested, approve) => {
                update_registry_v2_with(request, requested, choose, hooks, approve)
            }
        }
    }

    fn install(&self, ids: &[&str]) {
        self.apply(Action::Install(projects(ids))).unwrap();
        self.first_play();
    }

    fn first_play(&self) {
        self.play_on(BUILD);
    }

    fn play_on(&self, base: &str) {
        let cancel = native_mods::Cancellation::default();
        let mut profile = Profile::open(self.home(), None).unwrap();
        if profile
            .refresh_for_play(self.instance.native_profile, &cancel, |_| {}, None, None)
            .unwrap()
            .is_some()
        {
            return;
        }
        let first = profile
            .begin_first(|| FirstBuild::installed(base), |_| {})
            .unwrap();
        first.prepare(&|_| {}).unwrap();
        let image = first
            .exact_image(self.instance.native_profile, &cancel, |_| {}, None)
            .unwrap();
        first.commit(&image, &cancel).unwrap();
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

/// Reads a fixture release from its `file://` URL; attestation bundles are placeholders.
fn fetch_fixture(url: &str, _max: u64) -> Result<Vec<u8>, String> {
    if url.ends_with(".sigstore.json") {
        return Ok(b"{}".to_vec());
    }
    let path = url.strip_prefix("file://").unwrap();
    fs::read(path).map_err(|error| error.to_string())
}

/// Accepts every fixture attestation as signed by the release's own repository and workflow.
fn verify_fixture(
    _bundle: &[u8],
    _digest: [u8; 32],
    expected: &attestation::ExpectedRelease<'_>,
) -> Result<attestation::VerifiedRelease, attestation::Error> {
    attestation::check_release_policy(
        attestation::Attested {
            repository_id: expected.repository_id,
            repository_name: FIXTURE_REPOSITORY.into(),
            signer_workflow_ref: expected.workflow.into(),
            source_ref: "refs/tags/v1.0.0".into(),
            source_commit: "c".repeat(40),
            predicate_type: "https://slsa.dev/provenance/v1".into(),
        },
        expected,
    )
}

/// The fixture project IDs of in-game mods `ids`.
fn projects(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| project_id(id)).collect()
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

pub(super) fn roster(id: &str) -> Tag {
    Tag::compound([(
        "ContentModList",
        Tag::List(vec![Tag::compound([
            ("ModId", Tag::String(id.into())),
            ("ModVersion", Tag::String("1.0.0".into())),
        ])]),
    )])
}

pub(super) fn world(home: &Path, folder: &str, sunrise: Tag) {
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
fn same_registered_ids_have_no_warning_but_a_dropped_id_requires_approval() {
    Fixture::run(
        concat!(
            module_path!(),
            "::same_registered_ids_have_no_warning_but_a_dropped_id_requires_approval"
        ),
        |mut fixture| {
            fixture.project("lantern", "Lanterns", V2Kind::Mod);
            fixture.release("lantern", "1.0.0", "", &[(AMBER, EARLIER_BLOCK)]);
            fixture.install(&["lantern"]);
            world(fixture.home(), "Uses Lanterns", roster("lantern"));
            // Even a valid roster with no match cannot prove historic absence.
            world(fixture.home(), "Other mods", roster("another_mod"));
            let same_ids = fixture.release("lantern", "1.0.1", "", &[(AMBER, b"{}")]);
            let batch = fixture
                .update(&["lantern"], &|impact| {
                    assert!(!impact.risks_worlds());
                    assert_eq!(
                        impact.gui_review(&Default::default()).1,
                        "Lanterns will go from 1.0.0 to 1.0.1."
                    );
                    true
                })
                .unwrap();
            assert_eq!(batch.updated.len(), 1);
            assert_eq!(batch.updated[0].1.version, "1.0.1");
            assert_eq!(fs::read(fixture.home().join(LANTERN)).unwrap(), same_ids);

            let changed = fixture.release("lantern", "1.1.0", "", &[]);
            let old = fixture.snapshot(&[LANTERN]);
            let questions = std::cell::Cell::new(0);
            let decide = |approved| {
                fixture.update(&["lantern"], &|impact| {
                    questions.set(questions.get() + 1);
                    assert!(impact.risks_worlds() && !impact.is_removal());
                    let detail = impact.detail();
                    assert!(detail.contains("Lanterns 1.0.1 → 1.1.0"), "{detail}");
                    assert_world_warning(&detail, 2);
                    assert!(impact
                        .gui_review(&Default::default())
                        .1
                        .contains("2 saved worlds may lose"));
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
            fixture.project("lantern", "Amber Lanterns", V2Kind::Mod);
            fixture.release("lantern", "1.0.0", "", &[(AMBER, EARLIER_BLOCK)]);
            fixture.install(&["lantern"]);
            world(fixture.home(), "Uses Lanterns", roster("lantern"));
            fixture.release("lantern", "1.1.0", "", &[(AMBER, b"{}")]);
            fixture.project("lamp_post", "Lamp Posts", V2Kind::Mod);
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
            fixture.project("light_core", "Light Core", V2Kind::Mod);
            fixture.project("lantern", "Lanterns", V2Kind::Mod);
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
fn approved_update_refuses_changed_worlds_at_cutoff() {
    Fixture::run(
        concat!(
            module_path!(),
            "::approved_update_refuses_changed_worlds_at_cutoff"
        ),
        |mut fixture| {
            fixture.project("lantern", "Lanterns", V2Kind::Mod);
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
        },
    );
}

#[test]
fn install_resolves_newest_matching_dependency_and_refuses_conflicting_ranges() {
    Fixture::run(
        concat!(
            module_path!(),
            "::install_resolves_newest_matching_dependency_and_refuses_conflicting_ranges"
        ),
        |mut fixture| {
            fixture.project("light_core", "Light Core", V2Kind::Mod);
            fixture.project("fireflies", "Fireflies", V2Kind::Mod);
            fixture.project("two", "Two", V2Kind::Mod);
            // `release` prepends, so the index lists 1.0.0, 1.1.0, 2.0.0: the older match first.
            fixture.release("light_core", "2.0.0", "", &[]);
            let core_1_1 = fixture.release("light_core", "1.1.0", "", &[]);
            fixture.release("light_core", "1.0.0", "", &[]);
            fixture.release(
                "fireflies",
                "1.0.0",
                "[dependencies]\nlight_core='^1'\n",
                &[],
            );
            fixture.release("two", "1.0.0", "[dependencies]\nlight_core='^2'\n", &[]);

            // Conflicting dependency ranges refuse the whole set without touching the profile.
            let error = fixture
                .apply(Action::Install(projects(&["fireflies", "two"])))
                .unwrap_err()
                .to_string();
            assert!(error.contains("Light Core ^2"), "{error}");
            assert!(!fixture.home().join("sunrise-mods/fireflies.smod").exists());
            assert!(!fixture.home().join("sunrise-mods/two.smod").exists());
            assert!(!fixture.home().join(CORE).exists());

            // Installing one picks the newest dependency version matching its range.
            let batch = fixture
                .apply(Action::Install(projects(&["fireflies"])))
                .unwrap();
            let core = batch
                .added
                .iter()
                .find(|entry| entry.project == project_id("light_core"))
                .unwrap();
            let fireflies = batch
                .added
                .iter()
                .find(|entry| entry.project == project_id("fireflies"))
                .unwrap();
            assert_eq!(core.version, "1.1.0");
            assert!(core.dependency);
            assert!(!fireflies.dependency);
            assert_eq!(fs::read(fixture.home().join(CORE)).unwrap(), core_1_1);
            assert!(fixture.home().join("sunrise-mods/fireflies.smod").is_file());
        },
    );
}
