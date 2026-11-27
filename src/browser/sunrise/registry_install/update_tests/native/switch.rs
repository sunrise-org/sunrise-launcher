//! Exercise Switch through native proof, the world question and journal publication.
use super::*;
use crate::github_mods::{self, Origin, SourceOrigin, SourceTrack};

const OTHER: &str = "developer-other";
const ORIGIN_COMMIT: &str = "dddddddddddddddddddddddddddddddddddddddd";
const TITLE: &str = "Native Lantern";
const MIXINS: &str = "cccccccccccccccccccccccccccccccccccccccc";

struct Pinned {
    archive: Vec<u8>,
    receipt: Vec<u8>,
    preimage: Vec<Vec<u8>>,
}

impl Pinned {
    fn install(fixture: &mut Fixture, source: &[u8], mixins: Option<&str>) -> Self {
        configure_tool(fixture);
        fixture.project("extra_mod", "Fireflies", V2Kind::Mod);
        fixture.release("extra_mod", "1.0.0", "", &[]);
        fixture.install(&["extra_mod"]);
        let home = fixture.home();
        let archive = Package::write_files(
            &format!(
                "id='{ID}'\nname='{TITLE}'\nversion='1.0.0'\napi='{}'\nside='both'\ncapabilities=['registries']\n[native]\nsunrise='{}'\ngame='{}'\n{}",
                sunrise_launcher_smod::manifest::API,
                "a".repeat(40),
                backend::PINNED_GAME,
                mixins.map_or_else(String::new, |rev| format!("mixins='{rev}'\n")),
            ),
            &[
                ("Cargo.toml", b"[package]\nname='native_lantern'\nversion='1.0.0'\n"),
                ("src/lib.rs", source),
                (sunrise_launcher_smod::manifest::USES_FILE, b"{}"),
            ],
        )
        .unwrap();
        let path = home.join(ARCHIVE);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &archive).unwrap();
        let receipt = serde_json::to_vec(&Origin {
            id: ID.into(),
            version: "1.0.0".into(),
            owner: "Fixture".into(),
            repo: "Lantern".into(),
            tag: None,
            commit: ORIGIN_COMMIT.into(),
            asset: None,
            sha256: index::sha256(&archive),
            added_from: None,
            source: Some(SourceOrigin {
                repo_id: 42,
                manifest: "sunrise-mod.toml".into(),
                track: SourceTrack::Branch("main".into()),
                published_at: None,
            }),
        })
        .unwrap();
        fs::write(github_mods::origin_path(home, ID), &receipt).unwrap();
        // The player confirmed this source's unreviewed code when it was added.
        backend::allow_native_files([index::sha256(&archive)]).unwrap();
        mod_list::set_enabled(home, ID, true).unwrap();
        let (active, _) = Profile::open(home, None)
            .unwrap()
            .refresh_for_play(
                fixture.instance.native_profile,
                &native_mods::Cancellation::default(),
                |_| {},
                None,
                None,
            )
            .unwrap()
            .unwrap();
        assert_eq!(active.base(), BUILD);
        assert_ne!(active.executable(), BUILD);
        assert_eq!(compiled(fixture), "1.0.0\n");
        world(home, "Uses Native Lantern", roster(ID));
        Self {
            archive,
            receipt,
            preimage: fixture.snapshot(&[ARCHIVE, EXTRA]),
        }
    }

    fn assert_kept_off(&self, fixture: &Fixture, target: &str, active: &Active) -> HeldReason {
        let home = fixture.home();
        assert_eq!(active.base(), target);
        assert_eq!(Active::read_pointer(home).unwrap().as_ref(), Some(active));
        let holds: Vec<_> = active.holds().collect();
        let [hold] = holds.as_slice() else {
            panic!("expected one exact source hold: {holds:?}")
        };
        assert_eq!(hold.id(), ID);
        assert_eq!(
            hold.source(),
            &HeldSource::Github {
                owner: "Fixture".into(),
                repo: "Lantern".into(),
                commit: ORIGIN_COMMIT.into(),
                version: "1.0.0".into(),
            }
        );
        assert!(hold.enabled_before());
        assert!(hold.matches_archive(home));
        assert_eq!(fs::read(home.join(ARCHIVE)).unwrap(), self.archive);
        assert_eq!(
            fs::read(github_mods::origin_path(home, ID)).unwrap(),
            self.receipt
        );
        assert!(!mod_list::is_enabled(home, ID));
        assert!(mod_list::is_enabled(home, "extra_mod"));
        assert_eq!(fs::read(home.join(EXTRA)).unwrap(), self.preimage[4]);
        hold.reason().clone()
    }
}

fn target(tag: &str, pin: char) {
    let base = backend::version_dir(tag);
    fs::create_dir_all(&base).unwrap();
    fs::write(base.join(".installed"), []).unwrap();
    fs::write(base.join(backend::CLIENT), b"fixture target client").unwrap();
    fs::write(
        base.join("developer.json"),
        json!({"revision": pin.to_string().repeat(40)}).to_string(),
    )
    .unwrap();
    fs::write(
        base.join("research-lock.json"),
        json!({"minecraft": {"version": backend::PINNED_GAME}}).to_string(),
    )
    .unwrap();
    let resources = backend::resources(tag);
    fs::create_dir_all(&resources).unwrap();
    fs::write(resources.join("sunrise-resources.json"), b"{}").unwrap();
}

fn run_switch(
    fixture: &Fixture,
    tag: &str,
    approve: &dyn Fn(&SwitchQuestion) -> bool,
) -> Result<Switched, String> {
    switch_with(fixture, tag, &|q| {
        assert_eq!(q.worlds.affected[0].world, "Uses Native Lantern");
        Ok(if approve(q) {
            SwitchDecision::TurnOff
        } else {
            SwitchDecision::Cancel
        })
    })
}

fn switch_with(
    fixture: &Fixture,
    tag: &str,
    approve: &dyn Fn(&SwitchQuestion) -> Result<SwitchDecision, String>,
) -> Result<Switched, String> {
    v2::switch_profile_build_v2_with(
        &fixture.instance,
        install_target(&fixture.instance).unwrap(),
        tag,
        None,
        v2::V2SwitchHooks {
            switch: SwitchHooks {
                cancellation: &native_mods::Cancellation::default(),
                progress: &|_| {},
                approve,
                at_cutoff: &|| Ok(()),
            },
            confirm_publisher_change: None,
            pin_dir: None,
            recover_held: false,
        },
        || Ok(fixture.index_for(tag)),
        &fetch_fixture,
        &verify_fixture,
    )
}

#[test]
#[ignore = "daily; build the launcher binary first"]
fn switch_holds_exact_source_when_sdk_is_missing_but_not_on_local_io_failure() {
    Fixture::run(
        concat!(
            module_path!(),
            "::switch_holds_exact_source_when_sdk_is_missing_but_not_on_local_io_failure"
        ),
        |mut fixture| {
            let pinned = Pinned::install(&mut fixture, SOURCE, Some(MIXINS));
            target(SIGNED_BUILD, 'a');
            // Keep the valid signed game descriptor, but refuse the separate SDK release.
            let checked = sdk_check_fixture(&fixture, 'a', Value::Null);
            let sdk = fixture
                .root
                .join("signed-sdk")
                .join(format!("sdk-{}", "a".repeat(40)));
            fs::remove_file(sdk.join(sunrise_launcher_release_verify::SIGNATURE)).unwrap();
            let check_unknown = |q: &SwitchQuestion| {
                assert_eq!(q.unproved, [TITLE]);
                assert!(q.held.is_empty());
                assert!(q
                    .hold_lines()
                    .next()
                    .unwrap()
                    .contains("Fit could not be verified"));
            };
            let refused = run_switch(&fixture, SIGNED_BUILD, &|q| {
                check_unknown(q);
                false
            });
            assert!(matches!(refused.unwrap(), Switched::Stayed));
            assert_eq!(fixture.snapshot(&[ARCHIVE, EXTRA]), pinned.preimage);
            fs::remove_dir_all(&sdk).unwrap();
            // An unreadable local file is not absent remote evidence. Force a local I/O failure
            // even as root by placing a directory at the SDK index's path.
            fs::create_dir_all(sdk.join(sunrise_launcher_sdk::INDEX)).unwrap();
            let refused = run_switch(&fixture, SIGNED_BUILD, &|_| {
                panic!("local I/O reached review")
            });
            assert!(refused
                .err()
                .unwrap()
                .contains("Could not keep the mod tools"));
            assert_eq!(fixture.snapshot(&[ARCHIVE, EXTRA]), pinned.preimage);
            fs::remove_dir_all(sdk).unwrap();
            let Switched::Changed(active) = run_switch(&fixture, SIGNED_BUILD, &|q| {
                check_unknown(q);
                true
            })
            .unwrap() else {
                panic!("expected a switch with a held source")
            };
            assert_eq!(active.executable(), SIGNED_BUILD);
            assert_eq!(
                pinned.assert_kept_off(&fixture, SIGNED_BUILD, &active),
                HeldReason::Unproved {
                    build: SIGNED_BUILD.into(),
                    detail: None,
                }
            );
            assert!(
                !checked.exists(),
                "refused or missing SDK must not run the tool"
            );
            assert_eq!(compiled(&fixture), "1.0.0\n");
        },
    );
}

#[test]
#[ignore = "daily; build the launcher binary first"]
fn switch_keeps_the_registry_mod_when_only_a_sources_original_sdk_is_missing() {
    use crate::browser::{Kind, SourceId};
    use sunrise_launcher_sdk::{Kind as Slice, INDEX};

    Fixture::run(
        concat!(
            module_path!(),
            "::switch_keeps_the_registry_mod_when_only_a_sources_original_sdk_is_missing"
        ),
        |mut fixture| {
            // The source's omitted pin was resolved by its original developer SDK at install.
            signed_sdk_fixture(&fixture, 'a', &[]);
            let original_sdk = fixture
                .root
                .join("signed-sdk")
                .join(format!("sdk-{}", "a".repeat(40)));
            let local_sdk = fixture.root.join("sdk");
            fs::create_dir(&local_sdk).unwrap();
            fs::copy(original_sdk.join(INDEX), local_sdk.join(INDEX)).unwrap();
            let pinned = Pinned::install(&mut fixture, SOURCE, None);
            fs::remove_dir_all(&original_sdk).unwrap();

            const REGISTRY: &str = "registry_lantern";
            const FILE: &str = "sunrise-native-mods/registry_lantern.smod";
            fixture.project(REGISTRY, "Registry Lantern", V2Kind::NativeMod);
            let registry = fixture.release(
                REGISTRY,
                "2.0.0",
                &format!(
                    "[native]\nsunrise='{}'\nmixins='{MIXINS}'\ngame='{}'\n",
                    "b".repeat(40),
                    backend::PINNED_GAME
                ),
                &[
                    (
                        "Cargo.toml",
                        b"[package]\nname='registry_lantern'\nversion='2.0.0'\n",
                    ),
                    ("src/lib.rs", SOURCE),
                ],
            );
            let home = fixture.home();
            fs::write(home.join(FILE), &registry).unwrap();
            let mut installed = installed::Installed::read(home).unwrap();
            let project = fixture.catalogue.project(&project_id(REGISTRY)).unwrap();
            installed.entries.push(installed::Entry {
                source: SourceId::Sunrise,
                project: project.id.clone(),
                mod_id: Some(REGISTRY.into()),
                slug: REGISTRY.into(),
                title: "Registry Lantern".into(),
                kind: Kind::Mod,
                version: "2.0.0".into(),
                number: "2.0.0".into(),
                file: FILE.into(),
                checksum: format!("SHA-256:{}", index::sha256(&registry)),
                page: String::new(),
                dependency: false,
                verdict: None,
                install_hold: None,
                repository: Some(installed::VerifiedRepository {
                    id: project.repository.id,
                    name: project.repository.name.clone(),
                    path: project.repository.path.clone(),
                    workflow: project.releases[0].workflow.clone(),
                }),
            });
            installed.save(home).unwrap();
            // A registry install records its reviewed archive as allowed to build.
            backend::allow_native_files([index::sha256(&registry)]).unwrap();
            mod_list::set_enabled(home, REGISTRY, true).unwrap();

            // The target SDK and tools are available; only the source's original index is gone.
            // The tool builds a tiny inert library, never runs any game, and records the check.
            target(SIGNED_BUILD, 'b');
            let pack = |name: &str, from: &Path, file: &str| {
                let tar = fixture.root.join(name);
                assert!(Command::new("tar")
                    .arg("-cJf")
                    .arg(&tar)
                    .arg("-C")
                    .arg(from)
                    .arg(file)
                    .status()
                    .unwrap()
                    .success());
                fs::read(tar).unwrap()
            };
            let tool = fixture.root.join("cargo-sunrise-mod");
            fs::copy(fixture.root.join("sdk-tool"), &tool).unwrap();
            let tool = pack("tool.tar.xz", &fixture.root, "cargo-sunrise-mod");
            let runtime = pack(
                "runtime.tar.xz",
                &local_sdk.join("runtime"),
                "bin/sunrise-client",
            );
            let api = include_bytes!("../../../../../../crates/sdk/src/sdk/fixture.tar.xz");
            signed_sdk_fixture(
                &fixture,
                'b',
                &[
                    (Slice::Tool, &tool),
                    (Slice::Runtime, &runtime),
                    (Slice::Api, api),
                ],
            );

            let before = fixture.snapshot(&[ARCHIVE, FILE, EXTRA]);
            // Local I/O must still abort even though the error belongs to this source pin.
            fs::create_dir_all(original_sdk.join(INDEX)).unwrap();
            let error = run_switch(&fixture, SIGNED_BUILD, &|_| {
                panic!("local I/O reached review")
            })
            .err()
            .unwrap();
            assert!(error.contains("Could not keep the mod tools"), "{error}");
            assert_eq!(fixture.snapshot(&[ARCHIVE, FILE, EXTRA]), before);
            fs::remove_dir_all(&original_sdk).unwrap();

            let Switched::Changed(active) = run_switch(&fixture, SIGNED_BUILD, &|q| {
                assert_eq!(q.unproved, [TITLE]);
                assert!(q.held.is_empty());
                true
            })
            .unwrap() else {
                panic!("expected the registry mod and a held source")
            };
            assert_ne!(active.executable(), SIGNED_BUILD);
            assert_eq!(
                pinned.assert_kept_off(&fixture, SIGNED_BUILD, &active),
                HeldReason::Unproved {
                    build: SIGNED_BUILD.into(),
                    detail: None,
                }
            );
            assert!(mod_list::is_enabled(home, REGISTRY));
            assert_eq!(fs::read(home.join(FILE)).unwrap(), registry);
            assert_eq!(
                fs::read_to_string(fixture.root.join("checked-mods.json")).unwrap(),
                r#"["registry_lantern"]"#
            );
            assert_eq!(compiled(&fixture), "1.0.0\n2.0.0\n");
        },
    );
}

#[test]
#[ignore = "daily; build the launcher binary first"]
fn switch_holds_exact_source_when_its_rebuild_fails() {
    Fixture::run(
        concat!(
            module_path!(),
            "::switch_holds_exact_source_when_its_rebuild_fails"
        ),
        |mut fixture| {
            let source = [
                SOURCE,
                b"#[cfg(fixture_switch_target)]\ncompile_error!(\"fixture rebuild fails\");\n",
            ]
            .concat();
            let pinned = Pinned::install(&mut fixture, &source, Some(MIXINS));
            target(OTHER, 'a');
            fs::write(fixture.root.join("fixture-switch-target"), []).unwrap();
            let Switched::Changed(active) = run_switch(&fixture, OTHER, &|q| {
                assert_eq!(q.held, [TITLE]);
                assert!(q.unproved.is_empty());
                true
            })
            .unwrap() else {
                panic!("expected a switch with a held source")
            };
            assert_eq!(active.executable(), OTHER);
            assert_eq!(
                pinned.assert_kept_off(&fixture, OTHER, &active),
                HeldReason::BuildFailed {
                    build: OTHER.into()
                }
            );
            assert_eq!(
                compiled(&fixture),
                "1.0.0\n1.0.0\n",
                "the same source version had to be tried against the target"
            );
        },
    );
}

#[test]
#[ignore = "daily; build the launcher binary first"]
fn switch_keeps_the_specific_no_fit_reason_for_a_pinned_source() {
    Fixture::run(
        concat!(
            module_path!(),
            "::switch_keeps_the_specific_no_fit_reason_for_a_pinned_source"
        ),
        |mut fixture| {
            let pinned = Pinned::install(&mut fixture, SOURCE, Some(MIXINS));
            target(SIGNED_BUILD, 'b');
            let checked = sdk_check_fixture(
                &fixture,
                'b',
                json!({
                    "kind": "does-not-fit", "made_for": "a".repeat(40),
                    "misfits": [{"id": "game:fn:world::tick_light", "change": "changed"}],
                }),
            );
            let Switched::Changed(active) = run_switch(&fixture, SIGNED_BUILD, &|q| {
                assert_eq!(q.held, [TITLE]);
                assert!(q.unproved.is_empty());
                true
            })
            .unwrap() else {
                panic!("expected a switch with a held source")
            };
            assert_eq!(active.executable(), SIGNED_BUILD);
            let reason = pinned.assert_kept_off(&fixture, SIGNED_BUILD, &active);
            assert!(matches!(&reason, HeldReason::NoFit { build, .. } if build == SIGNED_BUILD));
            let detail = reason.detail().unwrap();
            assert!(detail.contains("tick_light (changed)"), "{detail}");
            assert!(checked.is_file());
            assert_eq!(compiled(&fixture), "1.0.0\n");
        },
    );
}

#[test]
#[ignore = "daily; build the launcher binary first"]
fn a_held_source_is_offered_again_on_each_switch_and_its_toggle_checks_it() {
    Fixture::run(
        concat!(
            module_path!(),
            "::a_held_source_is_offered_again_on_each_switch_and_its_toggle_checks_it"
        ),
        |mut fixture| {
            let source = [
                SOURCE,
                b"#[cfg(fixture_switch_target)]\ncompile_error!(\"fixture rebuild fails\");\n",
            ]
            .concat();
            let pinned = Pinned::install(&mut fixture, &source, Some(MIXINS));
            let home = fixture.home();
            target(SIGNED_BUILD, 'b');
            sdk_check_fixture(
                &fixture,
                'b',
                json!({
                    "kind": "does-not-fit", "made_for": "a".repeat(40),
                    "misfits": [{"id": "game:fn:world::tick_light", "change": "changed"}],
                }),
            );
            // The prompt names the reason beside the held mod.
            let Switched::Changed(active) = run_switch(&fixture, SIGNED_BUILD, &|q| {
                assert_eq!(q.held, [TITLE]);
                assert!(q.reasons[TITLE].contains("tick_light (changed)"), "{q:?}");
                let line = q.hold_lines().next().unwrap();
                assert!(line.contains("Not compatible with this build."), "{line}");
                assert!(line.contains("tick_light (changed)"), "{line}");
                true
            })
            .unwrap() else {
                panic!("expected a switch with a held source")
            };
            let reason = pinned.assert_kept_off(&fixture, SIGNED_BUILD, &active);
            assert!(reason.detail().unwrap().contains("tick_light (changed)"));

            // The signed index's takedowns are always asked, even by a profile without registry
            // receipts: while it can't be read, the toggle refuses and changes nothing.
            let identity = install_target(&fixture.instance).unwrap();
            let cancellation = native_mods::Cancellation::default();
            let before = fixture.snapshot(&[ARCHIVE, EXTRA]);
            let signed = fixture.root.join("index-v2.signed");
            std::env::set_var("SUNRISE_MOD_INDEX_V2", &signed);
            let error = restore_held(&fixture.instance, identity, ID, &cancellation).unwrap_err();
            assert!(error.contains("takedowns"), "{error}");
            assert_eq!(fixture.snapshot(&[ARCHIVE, EXTRA]), before);
            let v2 = Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/smod/tests/index_v2");
            fs::write(
                &signed,
                [
                    fs::read(v2.join("two-ports-index.json.minisig")).unwrap(),
                    fs::read(v2.join("two-ports-index.json")).unwrap(),
                ]
                .concat(),
            )
            .unwrap();

            // While the check still fails, the toggle says why and changes nothing.
            let error = restore_held(&fixture.instance, identity, ID, &cancellation).unwrap_err();
            assert!(error.contains("tick_light"), "{error}");
            assert_eq!(fixture.snapshot(&[ARCHIVE, EXTRA]), before);
            assert_eq!(Active::read_pointer(home).unwrap().as_ref(), Some(&*active));

            // The next switch checks the held bytes again rather than skipping them; this one
            // holds them again for its own reason, replacing the old hold.
            target(OTHER, 'a');
            fs::write(fixture.root.join("fixture-switch-target"), []).unwrap();
            let Switched::Changed(active) = switch_with(&fixture, OTHER, &|q| {
                assert_eq!(q.held, [TITLE]);
                assert!(q.unproved.is_empty());
                Ok(SwitchDecision::TurnOff)
            })
            .unwrap() else {
                panic!("expected a switch that holds the source again")
            };
            assert_eq!(
                pinned.assert_kept_off(&fixture, OTHER, &active),
                HeldReason::BuildFailed {
                    build: OTHER.into()
                }
            );

            // Once the exact bytes build for this build, the toggle turns the mod on.
            fs::remove_file(fixture.root.join("fixture-switch-target")).unwrap();
            restore_held(&fixture.instance, identity, ID, &cancellation).unwrap();
            let active = Active::read_pointer(home).unwrap().unwrap();
            assert_eq!(active.base(), OTHER);
            assert_eq!(active.holds().count(), 0);
            assert!(mod_list::is_enabled(home, ID));
            assert_eq!(fs::read(home.join(ARCHIVE)).unwrap(), pinned.archive);
            assert_eq!(compiled(&fixture), "1.0.0\n1.0.0\n1.0.0\n");
        },
    );
}
