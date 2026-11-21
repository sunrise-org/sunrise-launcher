//! Exercise the mixed worker and journal with local, inert registry and source inputs.
use super::*;
use crate::{
    browser::sunrise::sources::{self, Source, Tracker},
    github_mods,
    native_mods::Cancellation,
};
use std::cell::{Cell, RefCell};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Case {
    Approve,
    Decline,
    ChangedPostimage,
    RegistryCutoff,
    RegistryUnavailable,
    SourceBuild,
    CombinedGraph,
}

fn exercise(test: &str, case: Case) {
    Fixture::run(test, |mut fixture| {
        configure_tool(&fixture);
        fixture.project("lantern", "Lanterns", index::Kind::Mod);
        let old_registry = fixture.release("lantern", "1.0.0", "", &[(AMBER, EARLIER_BLOCK)]);
        fixture.install(&["lantern"]);
        let new_registry = fixture.release("lantern", "2.0.0", "", &[(AMBER, EARLIER_BLOCK)]);

        let home = fixture.home();
        let identity = install_target(&fixture.instance).unwrap();
        let fixtures = fixture.root.join("github");
        std::env::set_var("SUNRISE_SOURCE_FIXTURES", &fixtures);
        let repo = fixtures.join("repos/Fixture/Lantern");
        fs::create_dir_all(repo.join("src")).unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .current_dir(&repo)
                .args([
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "-c",
                    "commit.gpgSign=false",
                    "-c",
                    "core.hooksPath=/dev/null",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success(), "{:?}", output);
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        };
        git(&["init", "-q", "--initial-branch=main", "--template="]);
        let api = fixtures.join("api/repos/Fixture");
        let branch = api.join("Lantern/git/ref/heads");
        fs::create_dir_all(&branch).unwrap();
        fs::write(
            api.join("Lantern.json"),
            json!({"id": 42, "full_name": "Fixture/Lantern", "default_branch": "main", "private": false}).to_string(),
        )
        .unwrap();
        let put = |version: &str, code: &[u8], dependencies: &str| {
            let manifest = format!(
                "id='{ID}'\nname='Native Lantern'\nversion='{version}'\napi='{}'\nside='both'\ncapabilities=['registries']\n{dependencies}\n[native]\nsunrise='{}'\nmixins='{}'\ngame='{}'\n",
                sunrise_launcher_smod::manifest::API,
                "a".repeat(40),
                "c".repeat(40),
                backend::PINNED_GAME,
            );
            fs::write(repo.join("sunrise-mod.toml"), &manifest).unwrap();
            fs::write(
                repo.join("Cargo.toml"),
                format!("[package]\nname='{ID}'\nversion='{version}'\n"),
            )
            .unwrap();
            fs::write(repo.join("src/lib.rs"), code).unwrap();
            git(&["add", "."]);
            git(&["commit", "-qm", version]);
            let sha = git(&["rev-parse", "HEAD"]);
            let raw = fixtures.join("raw/Fixture/Lantern").join(&sha);
            fs::create_dir_all(&raw).unwrap();
            fs::write(raw.join("sunrise-mod.toml"), &manifest).unwrap();
            fs::write(
                branch.join("main.json"),
                json!({"ref": "refs/heads/main", "object": {"type": "commit", "sha": &sha}})
                    .to_string(),
            )
            .unwrap();
        };
        let listing = Source::for_origin(
            ID,
            "Fixture/Lantern",
            42,
            "sunrise-mod.toml",
            Tracker::Branch("main".into()),
        )
        .unwrap();
        let hooks = InstallHooks {
            progress: &|_| {},
            native: &|_| panic!("the fixture has native permission"),
            registry_crates: None,
            continue_after_failure: &|_| false,
            resolve: None,
        };
        let cancellation = Cancellation::default();
        let prepare = |request| {
            prepare_source(&fixture.instance, identity, BUILD, request, &cancellation).unwrap()
        };
        let pin = || sources::resolve(&listing, true).unwrap();
        put("1.0.0", SOURCE, "");
        let old = prepare(SourceRequest::Add(pin()));
        apply_sources(
            &fixture.instance,
            identity,
            &[old],
            || panic!("the registry install has already selected the base"),
            SourceHooks {
                install: hooks,
                cancellation: &Cancellation::default(),
                authorize: &|_| Ok(true),
                approve_worlds: &no_world_question,
            },
        )
        .unwrap();
        assert_eq!(compiled(&fixture), "1.0.0\n");
        let old_origin = github_mods::origin(home, ID).unwrap();
        let origin = "sunrise-native-mods/native_lantern.origin.json";
        let snapshot = || fixture.snapshot(&[LANTERN, ARCHIVE, origin]);
        let before = snapshot();
        put(
            "1.1.0",
            if case == Case::SourceBuild {
                b"compile_error!(\"inert mixed-update fixture\");"
            } else {
                SOURCE
            },
            if case == Case::CombinedGraph {
                // This source pin was checked against lantern 1.0.0. Only the
                // combined graph on the finalized registry candidate rejects it.
                "[incompatible]\nlantern='>=2'\n"
            } else {
                ""
            },
        );
        let mut batch = [prepare(SourceRequest::Update {
            pin: pin(),
            predecessor: Box::new(old_origin.clone()),
        })];
        let new_origin = batch[0].origin().clone();

        let reviews = Cell::new(0);
        let authorizations = Cell::new(0);
        let between = RefCell::new(None);
        let stopped_at_cutoff = Cell::new(false);
        let source_hooks = InstallHooks {
            progress: &|step| {
                if case == Case::Approve && step == InstallProgress::Publishing {
                    stopped_at_cutoff.set(true);
                    cancellation.stop();
                }
            },
            ..hooks
        };
        let start_sources = || {
            assert!(case != Case::Decline);
            assert_eq!(
                reviews.get(),
                usize::from(case != Case::RegistryUnavailable)
            );
            if case == Case::ChangedPostimage {
                // Change the checked install inputs without advancing the admitted pointer.
                mod_list::set_enabled(home, "lantern", false).unwrap();
            }
            between.replace(Some(snapshot()));
        };
        let authorize = |_: &[PreparedSource]| {
            assert_eq!(reviews.get(), 1);
            authorizations.set(authorizations.get() + 1);
            Ok(true)
        };
        let review = |first: &ChangeImpact, second: Option<&ChangeImpact>| {
            assert_eq!(reviews.replace(1), 0, "exactly one update review");
            assert_eq!(authorizations.get(), 0);
            assert_eq!(snapshot(), before, "review is before either publication");
            assert_eq!(
                compiled(&fixture),
                "1.0.0\n",
                "no new source work before consent"
            );
            let source = if case == Case::RegistryUnavailable {
                assert!(second.is_none());
                Some(first)
            } else {
                assert!(!first.risks_worlds());
                assert!(first.detail().contains("Lanterns 1.0.0 → 2.0.0"));
                second
            };
            if case == Case::CombinedGraph {
                assert!(source.is_none());
            } else {
                let source = source.expect("the checked source is named on the only review");
                assert!(!source.risks_worlds());
                let (_, copy) = source.gui_review(&Default::default());
                assert!(
                    copy.contains("Native Lantern will go from 1.0.0 to 1.1.0."),
                    "{copy}"
                );
                assert!(copy.contains("Sunrise can't check whether this update removes blocks or items your worlds use."), "{copy}");
            }
            if case == Case::RegistryCutoff {
                // The plan still parses, but the same response bytes no longer bind cutoff.
                use std::io::Write;
                fs::OpenOptions::new()
                    .append(true)
                    .open(fixture.root.join("index.json"))
                    .unwrap()
                    .write_all(b"\n")
                    .unwrap();
            }
            case != Case::Decline
        };
        let choose = || panic!("the profile is active on the fixture base");
        let result = if case == Case::RegistryUnavailable {
            fs::remove_file(fixture.root.join("index.json")).unwrap();
            update_registry_and_sources(
                &fixture.instance,
                identity,
                projects(&["lantern"]),
                &mut batch,
                choose,
                hooks,
                source_hooks,
                &cancellation,
                &authorize,
                &review,
                start_sources,
            )
        } else {
            source::update_registry_and_sources_with_registry(
                &fixture.instance,
                identity,
                projects(&["lantern"]),
                &mut batch,
                choose,
                hooks,
                source_hooks,
                &cancellation,
                &authorize,
                &review,
                start_sources,
                |source, local| {
                    let (catalogue, bytes, authority) = fetch_with_official(source, local)?;
                    assert!(authority.is_none());
                    let authority = OfficialRegistry::fixture(&catalogue);
                    Ok((catalogue, bytes, Some(authority)))
                },
            )
        };
        assert_eq!(reviews.get(), 1);
        assert_eq!(stopped_at_cutoff.get(), case == Case::Approve);
        assert!(!cancellation.is_stopped());
        let after = snapshot();
        if case == Case::Decline {
            assert!(matches!(result.registry, Err(InstallError::Cancelled)));
            assert_eq!(result.sources, Err(SourceError::Cancelled));
            assert_eq!(authorizations.get(), 0);
            assert!(between.into_inner().is_none());
            assert_eq!(after, before);
            return;
        }
        let between = between.into_inner().expect("source phase began");
        match case {
            Case::RegistryCutoff => assert!(matches!(
                result.registry,
                Err(InstallError::Unproved(ref reason)) if reason == &registry_changed()
            )),
            Case::RegistryUnavailable => assert!(result.registry.is_err()),
            _ => {
                assert_eq!(result.registry.unwrap().updated.len(), 1);
                assert_ne!(between[0], before[0]);
            }
        }
        let registry_failed = matches!(case, Case::RegistryCutoff | Case::RegistryUnavailable);
        if registry_failed {
            assert_eq!(between, before);
        }
        assert_eq!(
            after[3],
            if registry_failed {
                old_registry
            } else {
                new_registry
            }
        );
        match case {
            Case::ChangedPostimage => {
                assert!(matches!(result.sources, Err(SourceError::Changed(_))))
            }
            Case::SourceBuild => assert!(matches!(
                result.sources,
                Err(SourceError::Build { log: Some(_), .. })
            )),
            Case::CombinedGraph => assert!(matches!(
                result.sources,
                Err(SourceError::Dependency { .. })
            )),
            _ => result.sources.unwrap(),
        }
        if matches!(
            case,
            Case::ChangedPostimage | Case::SourceBuild | Case::CombinedGraph
        ) {
            assert_eq!(
                after, between,
                "the failed source edit leaves the registry postimage alone"
            );
            assert_eq!(
                &after[4..],
                &before[4..],
                "source archive and receipt stay exact"
            );
            assert_eq!(authorizations.get(), usize::from(case == Case::SourceBuild));
        } else {
            assert_ne!(after[0], between[0]);
            assert_eq!(github_mods::origin(home, ID).unwrap(), new_origin);
            assert_ne!(after[4], before[4]);
            assert_eq!(authorizations.get(), 1);
        }
    });
}

#[test]
#[ignore = "daily; build the launcher binary first"]
fn no_worlds_still_requires_one_combined_review() {
    exercise(
        concat!(
            module_path!(),
            "::no_worlds_still_requires_one_combined_review"
        ),
        Case::Approve,
    );
}

#[test]
#[ignore = "daily; build the launcher binary first"]
fn declined_review_leaves_both_predecessors_untouched() {
    exercise(
        concat!(
            module_path!(),
            "::declined_review_leaves_both_predecessors_untouched"
        ),
        Case::Decline,
    );
}

#[test]
#[ignore = "daily; build the launcher binary first"]
fn source_refuses_an_unadmitted_registry_postimage() {
    exercise(
        concat!(
            module_path!(),
            "::source_refuses_an_unadmitted_registry_postimage"
        ),
        Case::ChangedPostimage,
    );
}

#[test]
#[ignore = "daily; build the launcher binary first"]
fn registry_cutoff_failure_does_not_undo_a_valid_source_update() {
    exercise(
        concat!(
            module_path!(),
            "::registry_cutoff_failure_does_not_undo_a_valid_source_update"
        ),
        Case::RegistryCutoff,
    );
}

#[test]
#[ignore = "daily; build the launcher binary first"]
fn registry_unavailable_still_requires_review_before_a_source_update() {
    exercise(
        concat!(
            module_path!(),
            "::registry_unavailable_still_requires_review_before_a_source_update"
        ),
        Case::RegistryUnavailable,
    );
}

#[test]
#[ignore = "daily; build the launcher binary first"]
fn source_build_failure_does_not_undo_the_registry_update() {
    exercise(
        concat!(
            module_path!(),
            "::source_build_failure_does_not_undo_the_registry_update"
        ),
        Case::SourceBuild,
    );
}

#[test]
#[ignore = "daily; build the launcher binary first"]
fn source_graph_must_fit_the_staged_registry_not_the_predecessor() {
    exercise(
        concat!(
            module_path!(),
            "::source_graph_must_fit_the_staged_registry_not_the_predecessor"
        ),
        Case::CombinedGraph,
    );
}
