use super::switch_v2::{make_v2_hold, plan_slot_for_build, SlotPlan};
use super::*;
use crate::attestation::{Attested, RELEASE_WORKFLOW_PATH};
use crate::managed_play::{HeldReason, HeldSource, ModLock};
use semver::VersionReq;
use sunrise_launcher_smod::{
    index::browser_state,
    index_v2::{
        Build, Category, Dependency, File as V2File, Repository, Revoked, Screenshot, SCHEMA,
    },
    manifest::JarRef,
    Package,
};

const WORKFLOW_V1: &str =
    "sunrise-org/sunrise-mods/.github/workflows/release.yml@refs/tags/release-v1";
const WORKFLOW_V2: &str =
    "sunrise-org/sunrise-mods/.github/workflows/release.yml@refs/tags/release-v2";
const REAL_RUFF_BUNDLE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/crates/attestation/tests/fixtures/ruff-source.sigstore.json"
));

fn smod_bytes(id: &str, name: &str, version: &str, deps: &str) -> Vec<u8> {
    Package::write_files(
        &format!(
            "id='{id}'\nname='{name}'\nversion='{version}'\napi='0.2'\nside='both'\ncapabilities=['registries']\n{deps}"
        ),
        &[],
    )
    .unwrap()
}

#[allow(clippy::too_many_arguments)]
fn make_release(
    repo_name: &str,
    version: &str,
    sunrise: &str,
    workflow: &str,
    fits: &[&str],
    does_not_fit: &[&str],
    bytes: &[u8],
    dependencies: Vec<Dependency>,
) -> Release {
    let sha = index::sha256(bytes);
    Release {
        version: version.into(),
        published: "2026-11-22".into(),
        sunrise: sunrise.into(),
        workflow: workflow.into(),
        repository: None,
        fits: fits.iter().map(|s| (*s).into()).collect(),
        does_not_fit: does_not_fit.iter().map(|s| (*s).into()).collect(),
        jar_unavailable: Vec::new(),
        dependency_unavailable: Vec::new(),
        file: V2File {
            url: format!("https://github.com/{repo_name}/releases/download/v{version}/mod.smod"),
            size: bytes.len() as u64,
            sha256: sha,
            attestation: format!(
                "https://github.com/{repo_name}/releases/download/v{version}/mod.smod.sigstore.json"
            ),
        },
        dependencies,
    }
}

fn make_project(
    id: &str,
    slug: &str,
    mod_id: &str,
    name: &str,
    repo_id: u64,
    repo_name: &str,
    releases: Vec<Release>,
) -> Project {
    Project {
        id: id.into(),
        slug: slug.into(),
        mod_id: mod_id.into(),
        kind: V2Kind::Mod,
        name: name.into(),
        summary: "Test mod".into(),
        description: "Test mod description".into(),
        authors: vec!["tester".into()],
        owners: vec!["tester".into()],
        license: "MIT".into(),
        homepage: None,
        issues: None,
        source: None,
        icon: format!("pages/{id}/{}.png", "0".repeat(64)),
        screenshots: vec![Screenshot {
            path: format!("pages/{id}/{}.png", "1".repeat(64)),
            caption: "Preview".into(),
        }],
        categories: vec!["utility".into()],
        repository: Repository {
            id: repo_id,
            name: repo_name.into(),
            path: ".".into(),
        },
        workflow: RELEASE_WORKFLOW_PATH.into(),
        releases,
    }
}

fn make_index(projects: Vec<Project>, revoked: Vec<Revoked>) -> Index {
    let index = Index {
        schema: SCHEMA,
        serial: 1,
        generated: "2026-11-22T00:00:00Z".into(),
        builds: vec![
            Build {
                id: "26.3-0.1.0".into(),
                commit: "a".repeat(40),
            },
            Build {
                id: "26.3-0.1.1".into(),
                commit: "b".repeat(40),
            },
        ],
        categories: vec![Category {
            id: "utility".into(),
            icon: "wrench".into(),
            kinds: vec![V2Kind::Mod, V2Kind::NativeMod],
        }],
        projects,
        revoked,
        collections: Vec::new(),
    };
    index
        .check()
        .expect("test index must satisfy schema-2 invariants");
    index
}

fn write_installed_mod(
    home: &Path,
    mod_id: &str,
    bytes: &[u8],
    entry: Option<installed::Entry>,
    enabled: bool,
) {
    let rel = format!("{}/{mod_id}.smod", mod_list::MODS_DIR);
    let path = home.join(&rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, bytes).unwrap();
    mod_list::set_enabled(home, mod_id, enabled).unwrap();
    if let Some(entry) = entry {
        let mut doc = Document::read(home).unwrap();
        doc.replace(&entry).unwrap();
        doc.write(home).unwrap();
    }
}

fn v2_entry(
    project: &Project,
    release: &Release,
    bytes: &[u8],
    dependency: bool,
) -> installed::Entry {
    installed::Entry {
        source: SourceId::Sunrise,
        project: project.id.clone(),
        mod_id: Some(project.mod_id.clone()),
        slug: project.slug.clone(),
        title: project.name.clone(),
        kind: Kind::Mod,
        version: release.version.clone(),
        number: release.version.clone(),
        file: format!("{}/{}.smod", mod_list::MODS_DIR, project.mod_id),
        checksum: format!("SHA-256:{}", index::sha256(bytes)),
        page: format!("{SITE}/{}", project.slug),
        dependency,
        verdict: Some("Verified".into()),
        install_hold: None,
        repository: Some(installed::VerifiedRepository {
            id: project.repository.id,
            name: project.repository.name.clone(),
            path: project.repository.path.clone(),
            workflow: release.workflow.clone(),
        }),
    }
}

fn v1_entry(mod_id: &str, name: &str, version: &str, bytes: &[u8]) -> installed::Entry {
    installed::Entry {
        source: SourceId::Sunrise,
        project: mod_id.into(),
        mod_id: None,
        slug: mod_id.into(),
        title: name.into(),
        kind: Kind::Mod,
        version: version.into(),
        number: version.into(),
        file: format!("{}/{mod_id}.smod", mod_list::MODS_DIR),
        checksum: format!("SHA-256:{}", index::sha256(bytes)),
        page: String::new(),
        dependency: false,
        verdict: None,
        install_hold: None,
        repository: None,
    }
}

fn policy_verifier_for(
    repo_id: u64,
    repo_name: &'static str,
    workflow: &'static str,
) -> impl Fn(&[u8], [u8; 32], &ExpectedRelease<'_>) -> Result<VerifiedRelease, attestation::Error> {
    move |_bundle, _digest, expected| {
        let attested = Attested {
            repository_id: repo_id,
            owner_id: repo_id,
            repository_name: repo_name.into(),
            signer_workflow_ref: workflow.into(),
            source_ref: "refs/tags/v1.0.0".into(),
            source_commit: "c".repeat(40),
            predicate_type: "https://slsa.dev/provenance/v1".into(),
        };
        attestation::check_release_policy(attested, expected)
    }
}

#[test]
fn v2_plan_uses_authoritative_fits_and_never_infers_from_sunrise_or_semver() {
    let bytes = smod_bytes("wayfinder", "Wayfinder", "1.0.0", "");
    // `sunrise` names 26.3-0.1.0, yet `fits` is empty and `does_not_fit` names 26.3-0.1.1!
    let proj_unknown = make_project(
        "Ab3dE9xQ",
        "wayfinder-unknown",
        "wayfinder",
        "Wayfinder Unknown",
        1001,
        "alice/wayfinder",
        vec![make_release(
            "alice/wayfinder",
            "1.0.0",
            "26.3-0.1.0",
            WORKFLOW_V1,
            &[],
            &["26.3-0.1.1"],
            &bytes,
            Vec::new(),
        )],
    );
    let proj_fits = make_project(
        "Cd4fG0yR",
        "lantern",
        "lantern",
        "Lantern",
        1002,
        "alice/lantern",
        vec![make_release(
            "alice/lantern",
            "1.0.0",
            "26.3-0.1.0",
            WORKFLOW_V1,
            &["26.3-0.1.0"],
            &[],
            &smod_bytes("lantern", "Lantern", "1.0.0", ""),
            Vec::new(),
        )],
    );
    let idx = make_index(vec![proj_unknown, proj_fits], Vec::new());

    // Even though `sunrise == "26.3-0.1.0"`, it is not in `fits` -> Unknown -> refused!
    let unknown_err = plan_v2_install(
        &idx,
        "26.3-0.1.0",
        &["Ab3dE9xQ".into()],
        &BTreeMap::new(),
        &BTreeMap::new(),
        &V2InstallOptions::default(),
        None,
    )
    .unwrap_err();
    assert!(
        matches!(&unknown_err, InstallError::NotForVersion(msg) if msg.contains("not known yet")),
        "expected not-known-yet error, got {unknown_err:?}"
    );

    // Build in `does_not_fit` -> definitively refused as does-not-fit!
    let no_fit_err = plan_v2_install(
        &idx,
        "26.3-0.1.1",
        &["Ab3dE9xQ".into()],
        &BTreeMap::new(),
        &BTreeMap::new(),
        &V2InstallOptions::default(),
        None,
    )
    .unwrap_err();
    assert!(
        matches!(&no_fit_err, InstallError::NotForVersion(msg) if msg.contains("does not fit")),
        "expected does-not-fit error, got {no_fit_err:?}"
    );

    // Build in `fits` -> succeeds!
    let ok = plan_v2_install(
        &idx,
        "26.3-0.1.0",
        &["Cd4fG0yR".into()],
        &BTreeMap::new(),
        &BTreeMap::new(),
        &V2InstallOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(ok.steps.len(), 1);
    assert_eq!(ok.steps[0].project_id, "Cd4fG0yR");
}

#[test]
fn v2_plan_auto_installs_required_dependencies_and_skips_optional() {
    let req_bytes = smod_bytes("core_lib", "Core Lib", "1.0.0", "");
    let opt_bytes = smod_bytes("fancy_ui", "Fancy UI", "1.0.0", "");
    let app_bytes = smod_bytes(
        "compass",
        "Compass",
        "1.0.0",
        "[dependencies]\ncore_lib = '^1.0'\n",
    );
    let req_proj = make_project(
        "Core1234",
        "core-lib",
        "core_lib",
        "Core Lib",
        2001,
        "alice/core-lib",
        vec![make_release(
            "alice/core-lib",
            "1.0.0",
            "26.3-0.1.0",
            WORKFLOW_V1,
            &["26.3-0.1.0"],
            &[],
            &req_bytes,
            Vec::new(),
        )],
    );
    let opt_proj = make_project(
        "Opti5678",
        "fancy-ui",
        "fancy_ui",
        "Fancy UI",
        2002,
        "alice/fancy-ui",
        vec![make_release(
            "alice/fancy-ui",
            "1.0.0",
            "26.3-0.1.0",
            WORKFLOW_V1,
            &["26.3-0.1.0"],
            &[],
            &opt_bytes,
            Vec::new(),
        )],
    );
    let app_proj = make_project(
        "App90123",
        "compass",
        "compass",
        "Compass",
        2003,
        "alice/compass",
        vec![make_release(
            "alice/compass",
            "1.0.0",
            "26.3-0.1.0",
            WORKFLOW_V1,
            &["26.3-0.1.0"],
            &[],
            &app_bytes,
            vec![
                Dependency {
                    mod_id: "core_lib".into(),
                    version: VersionReq::STAR,
                    project: None,
                    required: true,
                },
                Dependency {
                    mod_id: "fancy_ui".into(),
                    version: VersionReq::STAR,
                    project: None,
                    required: false,
                },
            ],
        )],
    );
    let idx = make_index(vec![req_proj, opt_proj, app_proj], Vec::new());

    let plan = plan_v2_install(
        &idx,
        "26.3-0.1.0",
        &["App90123".into()],
        &BTreeMap::new(),
        &BTreeMap::new(),
        &V2InstallOptions::default(),
        None,
    )
    .unwrap();
    let planned_ids: Vec<&str> = plan.steps.iter().map(|s| s.project_id.as_str()).collect();
    assert_eq!(planned_ids, ["App90123", "Core1234"]);
    assert!(!plan.steps[0].dependency);
    assert!(plan.steps[1].dependency);
    assert_eq!(plan.optional_dependencies.len(), 1);
    assert_eq!(plan.optional_dependencies[0].mod_id, "fancy_ui");
}

#[test]
fn v2_plan_never_silently_picks_among_providers_and_honors_explicit_choice_or_pin() {
    let port_a_bytes = smod_bytes("wayfinder", "Wayfinder A", "1.0.0", "");
    let port_b_bytes = smod_bytes("wayfinder", "Wayfinder B", "1.0.0", "");
    let app_bytes = smod_bytes(
        "explorer",
        "Explorer",
        "1.0.0",
        "[dependencies]\nwayfinder = '^1.0'\n",
    );
    let port_a = make_project(
        "Ab3dE9xQ",
        "wayfinder",
        "wayfinder",
        "Wayfinder A",
        3001,
        "alice/wayfinder",
        vec![make_release(
            "alice/wayfinder",
            "1.0.0",
            "26.3-0.1.0",
            WORKFLOW_V1,
            &["26.3-0.1.0"],
            &[],
            &port_a_bytes,
            Vec::new(),
        )],
    );
    let port_b = make_project(
        "Zz91kLmN",
        "wayfinder-alt",
        "wayfinder",
        "Wayfinder B",
        3002,
        "bob/wayfinder",
        vec![make_release(
            "bob/wayfinder",
            "1.0.0",
            "26.3-0.1.0",
            WORKFLOW_V1,
            &["26.3-0.1.0"],
            &[],
            &port_b_bytes,
            Vec::new(),
        )],
    );
    let app_unpinned = make_project(
        "Expl1111",
        "explorer",
        "explorer",
        "Explorer",
        3003,
        "carol/explorer",
        vec![make_release(
            "carol/explorer",
            "1.0.0",
            "26.3-0.1.0",
            WORKFLOW_V1,
            &["26.3-0.1.0"],
            &[],
            &app_bytes,
            vec![Dependency {
                mod_id: "wayfinder".into(),
                version: VersionReq::STAR,
                project: None,
                required: true,
            }],
        )],
    );
    let app_pinned = make_project(
        "Expl2222",
        "explorer-pinned",
        "explorer_pinned",
        "Explorer Pinned",
        3004,
        "carol/explorer-pinned",
        vec![make_release(
            "carol/explorer-pinned",
            "1.0.0",
            "26.3-0.1.0",
            WORKFLOW_V1,
            &["26.3-0.1.0"],
            &[],
            &smod_bytes(
                "explorer_pinned",
                "Explorer Pinned",
                "1.0.0",
                "[dependencies]\nwayfinder = '^1.0'\n",
            ),
            vec![Dependency {
                mod_id: "wayfinder".into(),
                version: VersionReq::STAR,
                project: Some("Zz91kLmN".into()),
                required: true,
            }],
        )],
    );
    let idx = make_index(
        vec![port_b.clone(), port_a.clone(), app_unpinned, app_pinned],
        Vec::new(),
    );

    // 1. Ambiguous without caller choice -> MUST NOT silently pick!
    let err = plan_v2_install(
        &idx,
        "26.3-0.1.0",
        &["Expl1111".into()],
        &BTreeMap::new(),
        &BTreeMap::new(),
        &V2InstallOptions::default(),
        None,
    )
    .unwrap_err();
    match err {
        InstallError::AmbiguousProvider(need) => {
            assert_eq!(need.mod_id, "wayfinder");
            let ids: Vec<&str> = need
                .candidates
                .iter()
                .map(|c| c.project_id.as_str())
                .collect();
            assert_eq!(ids, ["Ab3dE9xQ", "Zz91kLmN"]);
        }
        other => panic!("expected AmbiguousProvider, got {other:?}"),
    }

    // 2. Explicit choice via callback selects chosen provider.
    let plan = plan_v2_install(
        &idx,
        "26.3-0.1.0",
        &["Expl1111".into()],
        &BTreeMap::new(),
        &BTreeMap::new(),
        &V2InstallOptions::default(),
        Some(&|_| Some("Zz91kLmN".into())),
    )
    .unwrap();
    assert_eq!(plan.steps[1].project_id, "Zz91kLmN");

    // 3. Dependency with explicit `dep.project` resolves directly without ambiguity.
    let pinned_plan = plan_v2_install(
        &idx,
        "26.3-0.1.0",
        &["Expl2222".into()],
        &BTreeMap::new(),
        &BTreeMap::new(),
        &V2InstallOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(pinned_plan.steps[1].project_id, "Zz91kLmN");

    // 4. Attempting to install `Ab3dE9xQ` while `Zz91kLmN` already occupies slot `wayfinder`
    // returns `ProviderConflict` (never silently swaps!).
    let temp = tempfile::tempdir().unwrap();
    write_installed_mod(
        temp.path(),
        "wayfinder",
        &port_b_bytes,
        Some(v2_entry(&port_b, &port_b.releases[0], &port_b_bytes, false)),
        true,
    );
    let inv = inventory_v2(temp.path(), &idx).unwrap();
    let conflict = plan_v2_install(
        &idx,
        "26.3-0.1.0",
        &["Ab3dE9xQ".into()],
        &inv,
        &BTreeMap::new(),
        &V2InstallOptions::default(),
        None,
    )
    .unwrap_err();
    assert!(matches!(
        conflict,
        InstallError::ProviderConflict(conflict)
            if conflict.mod_id == "wayfinder"
                && conflict.installed_project.as_deref() == Some("Zz91kLmN")
                && conflict.requested_project == "Ab3dE9xQ"
    ));
}

#[test]
fn v2_update_keeps_chosen_project_and_honors_chosen_versions_pins() {
    let a_v1 = smod_bytes("wayfinder", "Wayfinder A", "1.0.0", "");
    let a_v2 = smod_bytes("wayfinder", "Wayfinder A", "1.1.0", "");
    let b_v9 = smod_bytes("wayfinder", "Wayfinder B", "9.0.0", "");
    let port_a = make_project(
        "Ab3dE9xQ",
        "wayfinder",
        "wayfinder",
        "Wayfinder A",
        4001,
        "alice/wayfinder",
        vec![
            make_release(
                "alice/wayfinder",
                "1.0.0",
                "26.3-0.1.0",
                WORKFLOW_V1,
                &["26.3-0.1.0"],
                &[],
                &a_v1,
                Vec::new(),
            ),
            make_release(
                "alice/wayfinder",
                "1.1.0",
                "26.3-0.1.0",
                WORKFLOW_V1,
                &["26.3-0.1.0"],
                &[],
                &a_v2,
                Vec::new(),
            ),
        ],
    );
    let port_b = make_project(
        "Zz91kLmN",
        "wayfinder-alt",
        "wayfinder",
        "Wayfinder B",
        4002,
        "bob/wayfinder",
        vec![make_release(
            "bob/wayfinder",
            "9.0.0",
            "26.3-0.1.0",
            WORKFLOW_V1,
            &["26.3-0.1.0"],
            &[],
            &b_v9,
            Vec::new(),
        )],
    );
    let idx = make_index(vec![port_a.clone(), port_b], Vec::new());

    let temp = tempfile::tempdir().unwrap();
    write_installed_mod(
        temp.path(),
        "wayfinder",
        &a_v1,
        Some(v2_entry(&port_a, &port_a.releases[0], &a_v1, false)),
        true,
    );
    let inv = inventory_v2(temp.path(), &idx).unwrap();

    // Without pin: Update All upgrades Ab3dE9xQ 1.0.0 -> 1.1.0, NEVER switching to Zz91kLmN 9.0.0!
    let plan = plan_v2_update(
        &idx,
        "26.3-0.1.0",
        &UpdateRequest::Bulk(vec!["wayfinder".into()]),
        &inv,
        &BTreeMap::new(),
        &V2InstallOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].project_id, "Ab3dE9xQ");
    assert_eq!(plan.steps[0].version, "1.1.0");

    // With chosen-versions.json pin on project ID: Bulk update keeps pinned 1.0.0 untouched!
    let pins = BTreeMap::from([("Ab3dE9xQ".into(), "1.0.0".into())]);
    let pinned_bulk = plan_v2_update(
        &idx,
        "26.3-0.1.0",
        &UpdateRequest::Bulk(vec!["wayfinder".into()]),
        &inv,
        &pins,
        &V2InstallOptions::default(),
        None,
    )
    .unwrap();
    assert!(pinned_bulk.steps.is_empty());

    // Legacy mod-id pin in chosen-versions.json is also honored!
    let legacy_pins = BTreeMap::from([("wayfinder".into(), "1.0.0".into())]);
    let install_pinned = plan_v2_install(
        &idx,
        "26.3-0.1.0",
        &["Ab3dE9xQ".into()],
        &BTreeMap::new(),
        &legacy_pins,
        &V2InstallOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(install_pinned.steps[0].version, "1.0.0");
}

#[test]
fn v2_stage_and_verify_checks_digest_real_attestation_and_publisher_pins() {
    let bytes = smod_bytes("wayfinder", "Wayfinder", "1.0.0", "");
    let v2_bytes = smod_bytes("wayfinder", "Wayfinder", "1.1.0", "");
    let proj = make_project(
        "Ab3dE9xQ",
        "wayfinder",
        "wayfinder",
        "Wayfinder",
        5001,
        "alice/wayfinder",
        vec![
            make_release(
                "alice/wayfinder",
                "1.0.0",
                "26.3-0.1.0",
                WORKFLOW_V1,
                &["26.3-0.1.0"],
                &[],
                &bytes,
                Vec::new(),
            ),
            make_release(
                "alice/wayfinder",
                "1.1.0",
                "26.3-0.1.0",
                WORKFLOW_V2,
                &["26.3-0.1.0"],
                &[],
                &v2_bytes,
                Vec::new(),
            ),
        ],
    );
    let idx = make_index(vec![proj.clone()], Vec::new());
    let step = planned_from(&proj, &proj.releases[0], false);
    let pin_dir = tempfile::tempdir().unwrap();
    let stage = tempfile::tempdir().unwrap();

    // 1. Real offline Sigstore attestation verification MUST reject a non-matching bundle.
    let fetch_real_bundle = |url: &str, _max: u64| -> Result<Vec<u8>, String> {
        if url.ends_with(".sigstore.json") {
            Ok(REAL_RUFF_BUNDLE.to_vec())
        } else {
            Ok(bytes.clone())
        }
    };
    let bad_attest = stage_and_verify_v2_release(
        &idx,
        stage.path(),
        &step,
        pin_dir.path(),
        &BTreeSet::new(),
        None,
        V2Transfer {
            fetch_bytes: &fetch_real_bundle,
            verify_release: &attestation::verify_release,
        },
    )
    .unwrap_err();
    assert!(matches!(bad_attest, InstallError::Refused(_)));

    // 2. Tampered SHA-256 bytes (same size) MUST fail with Download error.
    let mut bad_bytes = bytes.clone();
    bad_bytes[0] ^= 0xff;
    let fetch_bad_sha = |url: &str, _max: u64| -> Result<Vec<u8>, String> {
        if url.ends_with(".sigstore.json") {
            Ok(b"{}".to_vec())
        } else {
            Ok(bad_bytes.clone())
        }
    };
    let sha_err = stage_and_verify_v2_release(
        &idx,
        stage.path(),
        &step,
        pin_dir.path(),
        &BTreeSet::new(),
        None,
        V2Transfer {
            fetch_bytes: &fetch_bad_sha,
            verify_release: &policy_verifier_for(5001, "alice/wayfinder", WORKFLOW_V1),
        },
    )
    .unwrap_err();
    assert!(matches!(sha_err, InstallError::Download(_)));

    // 3. Good archive + passing attestation policy stages v2 receipt and records publisher pin.
    let fetch_good = |url: &str, _max: u64| -> Result<Vec<u8>, String> {
        if url.ends_with(".sigstore.json") {
            Ok(b"{}".to_vec())
        } else if url.contains("v1.1.0") {
            Ok(v2_bytes.clone())
        } else {
            Ok(bytes.clone())
        }
    };
    let (entry, pending_pin) = stage_and_verify_v2_release(
        &idx,
        stage.path(),
        &step,
        pin_dir.path(),
        &BTreeSet::new(),
        None,
        V2Transfer {
            fetch_bytes: &fetch_good,
            verify_release: &policy_verifier_for(5001, "alice/wayfinder", WORKFLOW_V1),
        },
    )
    .unwrap();
    assert!(entry.is_v2_registry());
    assert_eq!(entry.project, "Ab3dE9xQ");
    assert_eq!(entry.mod_id.as_deref(), Some("wayfinder"));
    assert_eq!(entry.repository.as_ref().unwrap().id, 5001);
    assert_eq!(entry.repository.as_ref().unwrap().workflow, WORKFLOW_V1);
    commit_pending_pins(pin_dir.path(), &[pending_pin]).unwrap();

    // 4. Newer release-v2 tag on SAME repo & workflow path is accepted automatically.
    let step_v2 = planned_from(&proj, &proj.releases[1], false);
    let (_, pin_v2) = stage_and_verify_v2_release(
        &idx,
        stage.path(),
        &step_v2,
        pin_dir.path(),
        &BTreeSet::new(),
        None,
        V2Transfer {
            fetch_bytes: &fetch_good,
            verify_release: &policy_verifier_for(5001, "alice-renamed/wayfinder", WORKFLOW_V2),
        },
    )
    .unwrap();
    commit_pending_pins(pin_dir.path(), &[pin_v2]).unwrap();

    // 5. Changed repository ID triggers PublisherChanged (never silently accepted) and updates
    // pin ONLY when explicitly confirmed.
    let moved_proj = make_project(
        "Ab3dE9xQ",
        "wayfinder",
        "wayfinder",
        "Wayfinder",
        9999,
        "newowner/wayfinder",
        vec![make_release(
            "newowner/wayfinder",
            "1.1.0",
            "26.3-0.1.0",
            WORKFLOW_V2,
            &["26.3-0.1.0"],
            &[],
            &v2_bytes,
            Vec::new(),
        )],
    );
    let moved_idx = make_index(vec![moved_proj.clone()], Vec::new());
    let moved_step = planned_from(&moved_proj, &moved_proj.releases[0], false);
    let unconfirmed = stage_and_verify_v2_release(
        &moved_idx,
        stage.path(),
        &moved_step,
        pin_dir.path(),
        &BTreeSet::new(),
        Some(&|_| false),
        V2Transfer {
            fetch_bytes: &fetch_good,
            verify_release: &policy_verifier_for(9999, "newowner/wayfinder", WORKFLOW_V2),
        },
    )
    .unwrap_err();
    assert!(matches!(
        unconfirmed,
        InstallError::PublisherChanged(ref change)
            if change.project_id == "Ab3dE9xQ"
                && change.pinned.repository_id == 5001
                && change.incoming.repository_id == 9999
    ));
    // Pin on disk remained 5001 after rejection.
    assert_eq!(
        attestation::load_pins_in(pin_dir.path()).unwrap()["Ab3dE9xQ"].repository_id,
        5001
    );

    // Explicit confirmation allows staging and committing the updated pin.
    let (_, accepted_pin) = stage_and_verify_v2_release(
        &moved_idx,
        stage.path(),
        &moved_step,
        pin_dir.path(),
        &BTreeSet::new(),
        Some(&|question| question.change.incoming.repository_id == 9999),
        V2Transfer {
            fetch_bytes: &fetch_good,
            verify_release: &policy_verifier_for(9999, "newowner/wayfinder", WORKFLOW_V2),
        },
    )
    .unwrap();
    commit_pending_pins(pin_dir.path(), &[accepted_pin]).unwrap();
    assert_eq!(
        attestation::load_pins_in(pin_dir.path()).unwrap()["Ab3dE9xQ"].repository_id,
        9999
    );
}

/// An attestation verifier for releases signed from `repo_id`, owned by `owner_id`.
fn owned_verifier(
    owner_id: u64,
    repo_id: u64,
) -> impl Fn(&[u8], [u8; 32], &ExpectedRelease<'_>) -> Result<VerifiedRelease, attestation::Error> {
    move |_bundle, _digest, expected| {
        let attested = Attested {
            repository_id: repo_id,
            owner_id,
            repository_name: format!("owner{owner_id}/repo{repo_id}"),
            signer_workflow_ref: WORKFLOW_V1.into(),
            source_ref: "refs/tags/v1.0.0".into(),
            source_commit: "c".repeat(40),
            predicate_type: "https://slsa.dev/provenance/v1".into(),
        };
        attestation::check_release_policy(attested, expected)
    }
}

#[test]
fn an_older_release_of_a_moved_project_verifies_against_its_own_repository() {
    let old_bytes = smod_bytes("wayfinder", "Wayfinder", "1.0.0", "");
    let new_bytes = smod_bytes("wayfinder", "Wayfinder", "1.1.0", "");
    let release = |version: &str, bytes: &[u8], repository: Option<u64>| Release {
        repository,
        ..make_release(
            "alice/ports",
            version,
            "26.3-0.1.0",
            WORKFLOW_V1,
            &["26.3-0.1.0"],
            &[],
            bytes,
            Vec::new(),
        )
    };
    // The project now publishes from 7002; 1.0.0 was signed from 7001 before the move.
    let moved = make_project(
        "Ab3dE9xQ",
        "wayfinder",
        "wayfinder",
        "Wayfinder",
        7002,
        "alice/ports",
        vec![
            release("1.0.0", &old_bytes, Some(7001)),
            release("1.1.0", &new_bytes, Some(7002)),
        ],
    );
    let idx = make_index(vec![moved.clone()], Vec::new());
    let pin_dir = tempfile::tempdir().unwrap();
    let stage = tempfile::tempdir().unwrap();
    let fetch = |url: &str, _max: u64| -> Result<Vec<u8>, String> {
        Ok(if url.ends_with(".sigstore.json") {
            b"{}".to_vec()
        } else if url.contains("v1.1.0") {
            new_bytes.clone()
        } else {
            old_bytes.clone()
        })
    };
    let stage_release = |release: &Release, verifier: ReleaseVerifier<'_>| {
        let index = make_index(
            vec![Project {
                releases: vec![release.clone()],
                ..moved.clone()
            }],
            Vec::new(),
        );
        let step = planned_from(&index.projects[0], release, false);
        stage_and_verify_v2_release(
            &index,
            stage.path(),
            &step,
            pin_dir.path(),
            &BTreeSet::new(),
            Some(&|_| false),
            V2Transfer {
                fetch_bytes: &fetch,
                verify_release: verifier,
            },
        )
    };

    // The older release verifies against the repository its row records, with no prompt.
    let (entry, pin) = stage_release(&moved.releases[0], &owned_verifier(42, 7001)).unwrap();
    assert_eq!(entry.repository.as_ref().unwrap().id, 7001);
    commit_pending_pins(pin_dir.path(), &[pin]).unwrap();
    // The newer one, from the same owner's other repository, asks nothing either.
    let (_, pin) = stage_release(&moved.releases[1], &owned_verifier(42, 7002)).unwrap();
    commit_pending_pins(pin_dir.path(), &[pin]).unwrap();
    // Another owner's release in the project's repository is a publisher change.
    assert!(matches!(
        stage_release(&moved.releases[1], &owned_verifier(43, 7002)),
        Err(InstallError::PublisherChanged(_))
    ));
    // A row from an index without the field is checked against the project's repository.
    assert!(matches!(
        stage_release(
            &release("1.0.0", &old_bytes, None),
            &owned_verifier(42, 7001)
        ),
        Err(InstallError::Refused(_))
    ));

    // The installed older release still counts as this project's verified install after the move.
    let home = tempfile::tempdir().unwrap();
    write_installed_mod(home.path(), "wayfinder", &old_bytes, Some(entry), true);
    let inv = inventory_v2(home.path(), &idx).unwrap();
    assert!(matches!(
        inv["wayfinder"].ownership,
        V2Ownership::Verified { ref project_id, .. } if project_id == "Ab3dE9xQ"
    ));
}

#[test]
fn v2_adoption_adopts_v1_only_on_unique_verified_digest_match() {
    let bytes_a = smod_bytes("wayfinder", "Wayfinder", "1.0.0", "");
    let bytes_other = smod_bytes("wayfinder", "Wayfinder Fork", "1.0.0", "");
    let proj_a = make_project(
        "Ab3dE9xQ",
        "wayfinder",
        "wayfinder",
        "Wayfinder",
        6001,
        "alice/wayfinder",
        vec![make_release(
            "alice/wayfinder",
            "1.0.0",
            "26.3-0.1.0",
            WORKFLOW_V1,
            &["26.3-0.1.0"],
            &[],
            &bytes_a,
            Vec::new(),
        )],
    );
    let idx_unique = make_index(vec![proj_a.clone()], Vec::new());

    // 1. v1 row + unique digest match -> AdoptedV1!
    let home1 = tempfile::tempdir().unwrap();
    write_installed_mod(
        home1.path(),
        "wayfinder",
        &bytes_a,
        Some(v1_entry("wayfinder", "Wayfinder", "1.0.0", &bytes_a)),
        true,
    );
    let inv1 = inventory_v2(home1.path(), &idx_unique).unwrap();
    assert!(matches!(
        &inv1["wayfinder"].ownership,
        V2Ownership::AdoptedV1 { project_id, .. } if project_id == "Ab3dE9xQ"
    ));

    // 2. v1 row + DIFFERENT bytes (same mod_id & version, unverified digest) -> stays Local!
    let home2 = tempfile::tempdir().unwrap();
    write_installed_mod(
        home2.path(),
        "wayfinder",
        &bytes_other,
        Some(v1_entry("wayfinder", "Wayfinder", "1.0.0", &bytes_other)),
        true,
    );
    let inv2 = inventory_v2(home2.path(), &idx_unique).unwrap();
    assert_eq!(inv2["wayfinder"].ownership, V2Ownership::Local);

    // 3. Manual local file with NO mod-browser.json row (even matching digest) -> stays Local!
    let home3 = tempfile::tempdir().unwrap();
    write_installed_mod(home3.path(), "wayfinder", &bytes_a, None, true);
    let inv3 = inventory_v2(home3.path(), &idx_unique).unwrap();
    assert_eq!(inv3["wayfinder"].ownership, V2Ownership::Local);

    // 4. Ambiguous digest shared by TWO v2 projects -> stays Local!
    let proj_dup = make_project(
        "Zz91kLmN",
        "wayfinder-dup",
        "wayfinder",
        "Wayfinder Dup",
        6002,
        "bob/wayfinder",
        vec![make_release(
            "bob/wayfinder",
            "1.0.0",
            "26.3-0.1.0",
            WORKFLOW_V1,
            &["26.3-0.1.0"],
            &[],
            &bytes_a,
            Vec::new(),
        )],
    );
    let idx_ambig = make_index(vec![proj_a, proj_dup], Vec::new());
    let inv_ambig = inventory_v2(home1.path(), &idx_ambig).unwrap();
    assert_eq!(inv_ambig["wayfinder"].ownership, V2Ownership::Local);

    // 5. Present `.origin.json` sidecar (GitHub / local source mod) is never adopted as v2.
    let home_origin = tempfile::tempdir().unwrap();
    write_installed_mod(
        home_origin.path(),
        "wayfinder",
        &bytes_a,
        Some(v1_entry("wayfinder", "Wayfinder", "1.0.0", &bytes_a)),
        true,
    );
    let sidecar = home_origin
        .path()
        .join(mod_list::MODS_DIR)
        .join("wayfinder.origin.json");
    fs::write(&sidecar, "{}").unwrap();
    let inv_origin = inventory_v2(home_origin.path(), &idx_unique).unwrap();
    assert_eq!(inv_origin["wayfinder"].ownership, V2Ownership::Local);
}

#[test]
fn v2_revocations_respect_project_ownership_and_always_block_exact_digest() {
    let bytes_owned = smod_bytes("wayfinder", "Wayfinder A", "1.0.0", "");
    let bytes_local = smod_bytes("wayfinder", "Wayfinder Local", "1.0.0", "");
    let bytes_bad_hash = smod_bytes("lantern", "Lantern", "1.0.0", "");
    let bad_sha = index::sha256(&bytes_bad_hash);

    let proj_a = make_project(
        "Ab3dE9xQ",
        "wayfinder",
        "wayfinder",
        "Wayfinder A",
        7001,
        "alice/wayfinder",
        vec![make_release(
            "alice/wayfinder",
            "1.0.0",
            "26.3-0.1.0",
            WORKFLOW_V1,
            &["26.3-0.1.0"],
            &[],
            &bytes_owned,
            Vec::new(),
        )],
    );
    let proj_lantern = make_project(
        "Cd4fG0yR",
        "lantern",
        "lantern",
        "Lantern",
        7002,
        "alice/lantern",
        vec![make_release(
            "alice/lantern",
            "1.0.0",
            "26.3-0.1.0",
            WORKFLOW_V1,
            &["26.3-0.1.0"],
            &[],
            &bytes_bad_hash,
            Vec::new(),
        )],
    );
    let idx = make_index(
        vec![proj_a.clone(), proj_lantern],
        vec![
            Revoked {
                project: "Ab3dE9xQ".into(),
                sha256: None,
                reason: "malware".into(),
                date: "2026-11-22".into(),
            },
            Revoked {
                project: "Cd4fG0yR".into(),
                sha256: Some(bad_sha),
                reason: "compromised release".into(),
                date: "2026-11-22".into(),
            },
        ],
    );

    // 1. Owned v2 mod hit by whole-project takedown IS turned off.
    let home_owned = tempfile::tempdir().unwrap();
    write_installed_mod(
        home_owned.path(),
        "wayfinder",
        &bytes_owned,
        Some(v2_entry(&proj_a, &proj_a.releases[0], &bytes_owned, false)),
        true,
    );
    let notices = turn_off_revoked_v2(&idx, home_owned.path()).unwrap();
    assert_eq!(notices.len(), 1);
    assert!(!mod_list::is_enabled(home_owned.path(), "wayfinder"));

    // 2. Unowned local mod with SAME mod_id is NOT turned off by whole-project takedown,
    //    BUT an unowned local mod matching an exact-digest takedown IS turned off!
    let home_local = tempfile::tempdir().unwrap();
    write_installed_mod(home_local.path(), "wayfinder", &bytes_local, None, true);
    write_installed_mod(home_local.path(), "lantern", &bytes_bad_hash, None, true);
    let notices_local = turn_off_revoked_v2(&idx, home_local.path()).unwrap();
    assert_eq!(notices_local.len(), 1);
    assert!(mod_list::is_enabled(home_local.path(), "wayfinder"));
    assert!(!mod_list::is_enabled(home_local.path(), "lantern"));
}

#[test]
fn v2_switch_slot_planning_keeps_chosen_project_maps_verdicts_and_honors_pins() {
    let a_v1 = smod_bytes("wayfinder", "Wayfinder A", "1.0.0", "");
    let a_v2 = smod_bytes("wayfinder", "Wayfinder A", "1.1.0", "");
    let b_v1 = smod_bytes("wayfinder", "Wayfinder B", "1.0.0", "");
    let port_a = make_project(
        "Ab3dE9xQ",
        "wayfinder",
        "wayfinder",
        "Wayfinder A",
        8001,
        "alice/wayfinder",
        vec![
            make_release(
                "alice/wayfinder",
                "1.0.0",
                "26.3-0.1.0",
                WORKFLOW_V1,
                &["26.3-0.1.0"],
                &["26.3-0.1.1", "26.3-0.1.2"],
                &a_v1,
                Vec::new(),
            ),
            make_release(
                "alice/wayfinder",
                "1.1.0",
                "26.3-0.1.1",
                WORKFLOW_V1,
                &["26.3-0.1.1"],
                &["26.3-0.1.2"],
                &a_v2,
                Vec::new(),
            ),
        ],
    );
    let port_b = make_project(
        "Zz91kLmN",
        "wayfinder-alt",
        "wayfinder",
        "Wayfinder B",
        8002,
        "bob/wayfinder",
        vec![make_release(
            "bob/wayfinder",
            "1.0.0",
            "26.3-0.1.2",
            WORKFLOW_V1,
            &["26.3-0.1.2"],
            &[],
            &b_v1,
            Vec::new(),
        )],
    );
    let idx = make_index(vec![port_a.clone(), port_b], Vec::new());
    let home = tempfile::tempdir().unwrap();
    write_installed_mod(
        home.path(),
        "wayfinder",
        &a_v1,
        Some(v2_entry(&port_a, &port_a.releases[0], &a_v1, false)),
        true,
    );
    let inv = inventory_v2(home.path(), &idx).unwrap();
    let item = &inv["wayfinder"];

    // 1. Target 26.3-0.1.1 fits Ab3dE9xQ 1.1.0 -> Replace with SAME project Ab3dE9xQ!
    assert!(matches!(
        plan_slot_for_build(&idx, "wayfinder", item, "26.3-0.1.1", &BTreeMap::new()),
        SlotPlan::Replace(p, r) if p.id == "Ab3dE9xQ" && r.version == "1.1.0"
    ));
    assert!(checked_switch_fit_v2(&idx, home.path(), "26.3-0.1.1"));

    // 2. Target 26.3-0.1.2 is in Ab3dE9xQ's does_not_fit (even though Zz91kLmN fits 26.3-0.1.2!)
    //    -> MUST hold Ab3dE9xQ as NoFit (NEVER switch to Zz91kLmN!)
    let plan_no_fit = plan_slot_for_build(&idx, "wayfinder", item, "26.3-0.1.2", &BTreeMap::new());
    assert!(matches!(
        plan_no_fit,
        SlotPlan::Hold(HeldReason::NoFit { ref build, .. }) if build == "26.3-0.1.2"
    ));
    assert!(!checked_switch_fit_v2(&idx, home.path(), "26.3-0.1.2"));
    // The same check by name and reason, per build: Home says which build the mods have
    // releases for, or what each build leaves out and what is known of it.
    let wayfinder = |issue| Blocker {
        name: "Wayfinder A".into(),
        issue,
    };
    assert_eq!(
        switch_fit_blockers_v2(&idx, home.path(), "26.3-0.1.1"),
        Some(Vec::new())
    );
    let builds = [
        "26.3-0.1.2".to_string(),
        "26.3-0.9.0".to_string(),
        "26.3-0.1.1".to_string(),
    ];
    assert_eq!(
        mods_fit_by_build_v2(&idx, home.path(), &builds),
        Some(vec![
            // Marked as not fitting by every release it could use: confirmed.
            (
                "26.3-0.1.2".to_string(),
                vec![wayfinder(FitIssue::NoRelease)]
            ),
            // Marked neither way: not known, not "no".
            (
                "26.3-0.9.0".to_string(),
                vec![wayfinder(FitIssue::Unproved)]
            ),
            ("26.3-0.1.1".to_string(), Vec::new()),
        ])
    );

    // 3. Target 26.3-0.9.0 is in neither fits nor does_not_fit -> Unproved hold!
    let plan_unknown = plan_slot_for_build(&idx, "wayfinder", item, "26.3-0.9.0", &BTreeMap::new());
    assert!(matches!(
        plan_unknown,
        SlotPlan::Hold(HeldReason::Unproved { ref build, .. }) if build == "26.3-0.9.0"
    ));

    // 4. Pinned version 1.0.0 in chosen-versions.json does NOT fit 26.3-0.1.1 -> Hold (NoFit),
    //    does NOT silently jump to 1.1.0!
    let pins = BTreeMap::from([("Ab3dE9xQ".into(), "1.0.0".into())]);
    assert!(matches!(
        plan_slot_for_build(&idx, "wayfinder", item, "26.3-0.1.1", &pins),
        SlotPlan::Hold(HeldReason::NoFit { .. })
    ));

    // 5. V2 hold carries 8-char project ID + VerifiedRepository and passes Held validation!
    let hold = make_v2_hold(
        Path::new(""),
        "wayfinder",
        item,
        item.enabled,
        HeldReason::NoFit {
            build: "26.3-0.1.2".into(),
            detail: None,
        },
    )
    .unwrap();
    assert!(matches!(
        hold.source(),
        HeldSource::Registry {
            project,
            repository: Some(repo),
            ..
        } if project == "Ab3dE9xQ" && repo.id == 8001
    ));
}

#[test]
fn v2_provider_switch_detects_pinned_project_and_semver_dependents() {
    let wf_bytes = smod_bytes("wayfinder", "Wayfinder A", "1.0.0", "");
    let dep_pinned_bytes = smod_bytes(
        "addon_pin",
        "Addon Pin",
        "1.0.0",
        "[dependencies]\nwayfinder = '^1.0'\n",
    );
    let dep_semver_bytes = smod_bytes(
        "addon_sem",
        "Addon Semver",
        "1.0.0",
        "[dependencies]\nwayfinder = '^1.0'\n",
    );
    let proj_wf = make_project(
        "Ab3dE9xQ",
        "wayfinder",
        "wayfinder",
        "Wayfinder A",
        9001,
        "alice/wayfinder",
        vec![make_release(
            "alice/wayfinder",
            "1.0.0",
            "26.3-0.1.0",
            WORKFLOW_V1,
            &["26.3-0.1.0"],
            &[],
            &wf_bytes,
            Vec::new(),
        )],
    );
    let proj_pin = make_project(
        "Pin11111",
        "addon-pin",
        "addon_pin",
        "Addon Pin",
        9002,
        "alice/addon-pin",
        vec![make_release(
            "alice/addon-pin",
            "1.0.0",
            "26.3-0.1.0",
            WORKFLOW_V1,
            &["26.3-0.1.0"],
            &[],
            &dep_pinned_bytes,
            vec![Dependency {
                mod_id: "wayfinder".into(),
                version: VersionReq::STAR,
                project: Some("Ab3dE9xQ".into()),
                required: true,
            }],
        )],
    );
    let proj_sem = make_project(
        "Sem22222",
        "addon-sem",
        "addon_sem",
        "Addon Semver",
        9003,
        "alice/addon-sem",
        vec![make_release(
            "alice/addon-sem",
            "1.0.0",
            "26.3-0.1.0",
            WORKFLOW_V1,
            &["26.3-0.1.0"],
            &[],
            &dep_semver_bytes,
            vec![Dependency {
                mod_id: "wayfinder".into(),
                version: VersionReq::STAR,
                project: None,
                required: true,
            }],
        )],
    );
    let idx = make_index(
        vec![proj_wf.clone(), proj_pin.clone(), proj_sem.clone()],
        Vec::new(),
    );
    let home = tempfile::tempdir().unwrap();
    write_installed_mod(
        home.path(),
        "wayfinder",
        &wf_bytes,
        Some(v2_entry(&proj_wf, &proj_wf.releases[0], &wf_bytes, false)),
        true,
    );
    write_installed_mod(
        home.path(),
        "addon_pin",
        &dep_pinned_bytes,
        Some(v2_entry(
            &proj_pin,
            &proj_pin.releases[0],
            &dep_pinned_bytes,
            false,
        )),
        true,
    );
    write_installed_mod(
        home.path(),
        "addon_sem",
        &dep_semver_bytes,
        Some(v2_entry(
            &proj_sem,
            &proj_sem.releases[0],
            &dep_semver_bytes,
            false,
        )),
        true,
    );
    let inv = inventory_v2(home.path(), &idx).unwrap();

    // Switching to Zz91kLmN 1.5.0 breaks ONLY `addon_pin` (pinned to Ab3dE9xQ).
    let aff_15 = find_affected_dependents(&idx, &inv, "wayfinder", "Zz91kLmN", "1.5.0");
    assert_eq!(aff_15.len(), 1);
    assert_eq!(aff_15[0].mod_id, "addon_pin");

    // Switching to Zz91kLmN 2.0.0 breaks BOTH `addon_pin` and `addon_sem` (^1.0).
    let aff_20 = find_affected_dependents(&idx, &inv, "wayfinder", "Zz91kLmN", "2.0.0");
    let ids: BTreeSet<&str> = aff_20.iter().map(|a| a.mod_id.as_str()).collect();
    assert_eq!(ids, BTreeSet::from(["addon_pin", "addon_sem"]));
}

#[test]
fn v2_document_replace_upgrades_v1_row_without_duplicate_and_mod_lock_round_trips() {
    let bytes = smod_bytes("wayfinder", "Wayfinder", "1.0.0", "");
    let proj = make_project(
        "Ab3dE9xQ",
        "wayfinder",
        "wayfinder",
        "Wayfinder",
        9100,
        "alice/wayfinder",
        vec![make_release(
            "alice/wayfinder",
            "1.0.0",
            "26.3-0.1.0",
            WORKFLOW_V1,
            &["26.3-0.1.0"],
            &[],
            &bytes,
            Vec::new(),
        )],
    );
    let home = tempfile::tempdir().unwrap();
    // Seed a legacy v1 row, then replace with v2 receipt in place.
    write_installed_mod(
        home.path(),
        "wayfinder",
        &bytes,
        Some(v1_entry("wayfinder", "Wayfinder", "1.0.0", &bytes)),
        true,
    );
    let mut doc = Document::read(home.path()).unwrap();
    let v2 = v2_entry(&proj, &proj.releases[0], &bytes, false);
    doc.replace(&v2).unwrap();
    doc.write(home.path()).unwrap();

    let installed = installed::Installed::read(home.path()).unwrap();
    assert_eq!(installed.entries.len(), 1);
    assert_eq!(installed.entries[0].project, "Ab3dE9xQ");
    assert_eq!(installed.entries[0].mod_id.as_deref(), Some("wayfinder"));
    assert!(installed.entries[0].is_v2_registry());

    // browser_state::Installed::single finds the row by either mod_id or project_id.
    let raw = browser_state::Installed::read(home.path()).unwrap();
    assert_eq!(raw.single("wayfinder").unwrap(), Some(0));
    assert_eq!(raw.single("Ab3dE9xQ").unwrap(), Some(0));

    // ModLock JSON round-trip preserves exact project, mod_id, version, sha256, and repository.
    let lock = ModLock {
        project: proj.id.clone(),
        mod_id: proj.mod_id.clone(),
        version: "1.0.0".into(),
        sha256: index::sha256(&bytes),
        repository: v2.repository.unwrap(),
    };
    let encoded = serde_json::to_vec(&lock).unwrap();
    let decoded: ModLock = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, lock);
}

#[test]
fn v2_stage_rejects_unlisted_required_dependency_in_manifest() {
    let bytes = smod_bytes(
        "wayfinder",
        "Wayfinder",
        "1.0.0",
        "[dependencies]\nhidden_dep = '^1.0'\n",
    );
    let proj = make_project(
        "Ab3dE9xQ",
        "wayfinder",
        "wayfinder",
        "Wayfinder",
        9200,
        "alice/wayfinder",
        vec![make_release(
            "alice/wayfinder",
            "1.0.0",
            "26.3-0.1.0",
            WORKFLOW_V1,
            &["26.3-0.1.0"],
            &[],
            &bytes,
            Vec::new(),
        )],
    );
    let idx = make_index(vec![proj.clone()], Vec::new());
    let step = planned_from(&proj, &proj.releases[0], false);
    let pin_dir = tempfile::tempdir().unwrap();
    let stage = tempfile::tempdir().unwrap();
    let fetch = |url: &str, _max: u64| -> Result<Vec<u8>, String> {
        if url.ends_with(".sigstore.json") {
            Ok(b"{}".to_vec())
        } else {
            Ok(bytes.clone())
        }
    };
    let err = stage_and_verify_v2_release(
        &idx,
        stage.path(),
        &step,
        pin_dir.path(),
        &BTreeSet::new(),
        None,
        V2Transfer {
            fetch_bytes: &fetch,
            verify_release: &policy_verifier_for(9200, "alice/wayfinder", WORKFLOW_V1),
        },
    )
    .unwrap_err();
    assert!(
        matches!(&err, InstallError::Download(msg) if msg.contains("hidden_dep")),
        "expected unlisted required dependency error, got {err:?}"
    );
}

#[test]
fn v2_plan_picks_the_newest_library_release_in_the_range_the_dependent_asks_for() {
    let lib = |version: &str, fits: &[&str]| {
        let bytes = smod_bytes("core_lib", "Core Lib", version, "");
        make_release(
            "alice/core-lib",
            version,
            "26.3-0.1.0",
            WORKFLOW_V1,
            fits,
            &[],
            &bytes,
            Vec::new(),
        )
    };
    let app = |range: &str| {
        let bytes = smod_bytes(
            "compass",
            "Compass",
            "1.0.0",
            &format!("[dependencies]\ncore_lib = '{range}'\n"),
        );
        make_project(
            "App90123",
            "compass",
            "compass",
            "Compass",
            2003,
            "alice/compass",
            vec![make_release(
                "alice/compass",
                "1.0.0",
                "26.3-0.1.0",
                WORKFLOW_V1,
                &["26.3-0.1.0"],
                &[],
                &bytes,
                vec![Dependency {
                    mod_id: "core_lib".into(),
                    version: VersionReq::parse(range).unwrap(),
                    project: None,
                    required: true,
                }],
            )],
        )
    };
    let library = make_project(
        "Core1234",
        "core-lib",
        "core_lib",
        "Core Lib",
        2001,
        "alice/core-lib",
        vec![
            lib("2.0.0", &["26.3-0.1.0"]),
            lib("1.4.0", &["26.3-0.1.1"]),
            lib("1.3.0", &["26.3-0.1.0"]),
            lib("1.2.0", &["26.3-0.1.0"]),
        ],
    );
    let plan = |range: &str| {
        plan_v2_install(
            &make_index(vec![library.clone(), app(range)], Vec::new()),
            "26.3-0.1.0",
            &["App90123".into()],
            &BTreeMap::new(),
            &BTreeMap::new(),
            &V2InstallOptions::default(),
            None,
        )
    };

    // 2.0.0 is newest but out of range; 1.4.0 is in range but listed for another build.
    let steps = plan("^1.2").unwrap().steps;
    assert_eq!(steps[1].project_id, "Core1234");
    assert_eq!(steps[1].version, "1.3.0");

    // Nothing in range fits this build: refused, naming the range.
    let refused = plan(">=3").unwrap_err().to_string();
    assert!(refused.contains(">=3"), "{refused}");
}

/// Core Lib 2.0.0 and 1.3.0 fit 26.3-0.1.0, 1.4.0 is not judged for it yet, and 1.2.0 does
/// not fit it. Compass needs `core_lib` in `range`, naming the Core Lib project or not.
fn library_and_dependent(range: &str, named: bool) -> Vec<Project> {
    let lib = |version: &str, fits: &[&str], does_not_fit: &[&str]| {
        let bytes = smod_bytes("core_lib", "Core Lib", version, "");
        make_release(
            "alice/core-lib",
            version,
            "26.3-0.1.0",
            WORKFLOW_V1,
            fits,
            does_not_fit,
            &bytes,
            Vec::new(),
        )
    };
    let library = make_project(
        "Core1234",
        "core-lib",
        "core_lib",
        "Core Lib",
        2001,
        "alice/core-lib",
        vec![
            lib("2.0.0", &["26.3-0.1.0"], &[]),
            lib("1.4.0", &["26.3-0.1.1"], &[]),
            lib("1.3.0", &["26.3-0.1.0"], &[]),
            lib("1.2.0", &["26.3-0.1.1"], &["26.3-0.1.0"]),
        ],
    );
    let bytes = smod_bytes(
        "compass",
        "Compass",
        "1.0.0",
        &format!("[dependencies]\ncore_lib = '{range}'\n"),
    );
    let app = make_project(
        "App90123",
        "compass",
        "compass",
        "Compass",
        2003,
        "alice/compass",
        vec![make_release(
            "alice/compass",
            "1.0.0",
            "26.3-0.1.0",
            WORKFLOW_V1,
            &["26.3-0.1.0"],
            &[],
            &bytes,
            vec![Dependency {
                mod_id: "core_lib".into(),
                version: VersionReq::parse(range).unwrap(),
                project: named.then(|| "Core1234".into()),
                required: true,
            }],
        )],
    );
    vec![library, app]
}

fn plan_together(projects: Vec<Project>, requested: &[&str]) -> Result<V2Plan, InstallError> {
    plan_v2_install(
        &make_index(projects, Vec::new()),
        "26.3-0.1.0",
        &requested
            .iter()
            .map(|id| (*id).to_owned())
            .collect::<Vec<_>>(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &V2InstallOptions::default(),
        None,
    )
}

#[test]
fn v2_plan_says_why_no_release_of_a_dependency_can_be_used() {
    let refused = |range: &str| {
        plan_together(library_and_dependent(range, false), &["App90123"])
            .unwrap_err()
            .to_string()
    };
    // 1.2.0 is the only release in range, and it does not fit this build.
    assert_eq!(
        refused("~1.2"),
        "Compass needs core_lib ~1.2, which has no release fitting Sunrise 26.3-0.1.0."
    );
    // 1.4.0 is in range but not judged for this build yet.
    assert_eq!(
        refused("^1.4"),
        "Compass needs core_lib, whose fit on Sunrise 26.3-0.1.0 is not known yet."
    );
}

#[test]
fn v2_plan_checks_the_range_of_a_dependency_planned_in_the_same_install() {
    // Installing the library with its dependent plans the library's newest fitting release,
    // 2.0.0, which Compass's `^1` does not allow, whichever is asked for first and whether
    // Compass names the library's project or only its mod id.
    for named in [true, false] {
        for requested in [["Core1234", "App90123"], ["App90123", "Core1234"]] {
            let refused = plan_together(library_and_dependent("^1", named), &requested)
                .unwrap_err()
                .to_string();
            assert_eq!(
                refused, "Compass needs Core Lib ^1, but 2.0.0 is planned for another mod.",
                "named {named}, requested {requested:?}"
            );
        }
    }

    // A range the planned release is in installs it once.
    let steps = plan_together(
        library_and_dependent(">=2", false),
        &["Core1234", "App90123"],
    )
    .unwrap()
    .steps;
    let versions: Vec<_> = steps
        .iter()
        .map(|s| (s.project_id.as_str(), s.version.as_str()))
        .collect();
    assert_eq!(versions, [("Core1234", "2.0.0"), ("App90123", "1.0.0")]);
}

#[test]
fn a_verified_native_release_is_allowed_to_build_without_the_warning() {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let manifest = format!(
        "id='trail'\nname='Trail'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='{}'\nsunrise='{}'\nmixins='{}'\n",
        crate::backend::PINNED_GAME,
        "a".repeat(40),
        "b".repeat(40)
    );
    for (name, text) in [
        ("Cargo.toml", "[package]\nname='trail'\nversion='1.0.0'\n"),
        ("src/lib.rs", "fn hook() {}"),
        ("sunrise-mod.toml", manifest.as_str()),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    let bytes = zip.finish().unwrap().into_inner();
    let mut proj = make_project(
        "Tr4iLxQ1",
        "trail",
        "trail",
        "Trail",
        5002,
        "alice/trail",
        vec![make_release(
            "alice/trail",
            "1.0.0",
            "26.3-0.1.0",
            WORKFLOW_V1,
            &["26.3-0.1.0"],
            &[],
            &bytes,
            Vec::new(),
        )],
    );
    proj.kind = V2Kind::NativeMod;
    let idx = make_index(vec![proj.clone()], Vec::new());
    let step = planned_from(&proj, &proj.releases[0], false);
    let pin_dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let fetch = |url: &str, _max: u64| -> Result<Vec<u8>, String> {
        if url.ends_with(".sigstore.json") {
            Ok(b"{}".to_vec())
        } else {
            Ok(bytes.clone())
        }
    };
    let refused = stage_and_verify_v2_release(
        &idx,
        home.path(),
        &step,
        pin_dir.path(),
        &BTreeSet::new(),
        None,
        V2Transfer {
            fetch_bytes: &fetch,
            verify_release: &policy_verifier_for(9999, "alice/trail", WORKFLOW_V1),
        },
    );
    assert!(refused.is_err());
    assert!(backend::allowed_native_files_in(pin_dir.path())
        .unwrap()
        .is_empty());
    stage_and_verify_v2_release(
        &idx,
        home.path(),
        &step,
        pin_dir.path(),
        &BTreeSet::new(),
        None,
        V2Transfer {
            fetch_bytes: &fetch,
            verify_release: &policy_verifier_for(5002, "alice/trail", WORKFLOW_V1),
        },
    )
    .unwrap();
    assert_eq!(
        backend::allowed_native_files_in(pin_dir.path()).unwrap(),
        BTreeSet::from([index::sha256(&bytes).to_ascii_lowercase()])
    );
}

fn jar(id: &str, version: &str, digit: char) -> JarRef {
    JarRef::new(id.into(), version.into(), digit.to_string().repeat(128)).unwrap()
}

/// A project whose 1.0.0 is installed and whose 1.1.0 lists two jars Modrinth no longer serves.
fn project_with_marked_update(marked: &[JarRef], v1: &[u8], v2: &[u8]) -> Project {
    let release = |version: &str, bytes: &[u8]| {
        make_release(
            "alice/wayfinder",
            version,
            "26.3-0.1.0",
            WORKFLOW_V1,
            &["26.3-0.1.0"],
            &[],
            bytes,
            Vec::new(),
        )
    };
    let mut newer = release("1.1.0", v2);
    newer.jar_unavailable = marked.to_vec();
    make_project(
        "Ab3dE9xQ",
        "wayfinder",
        "wayfinder",
        "Wayfinder",
        4001,
        "alice/wayfinder",
        vec![release("1.0.0", v1), newer],
    )
}

#[test]
fn v2_install_planning_refuses_a_release_whose_original_jar_modrinth_no_longer_serves() {
    let gone = [
        jar("macaws_bridges", "3.0.0", 'a'),
        jar("other_lib", "1.2.0", 'b'),
    ];
    let v1 = smod_bytes("wayfinder", "Wayfinder", "1.0.0", "");
    let v2 = smod_bytes("wayfinder", "Wayfinder", "1.1.0", "");
    let project = project_with_marked_update(&gone, &v1, &v2);
    let idx = make_index(vec![project.clone()], Vec::new());
    let cache = tempfile::tempdir().unwrap();
    let options = V2InstallOptions {
        jar_cache: Some(cache.path().to_owned()),
        ..V2InstallOptions::default()
    };
    let lines = "The original file macaws_bridges 3.0.0 is no longer on Modrinth.\n\
                 The original file other_lib 1.2.0 is no longer on Modrinth.";

    // The pinned release is refused with one line per jar.
    let pinned = V2InstallOptions {
        versions: BTreeMap::from([("Ab3dE9xQ".into(), "1.1.0".into())]),
        ..options.clone()
    };
    let error = plan_v2_install(
        &idx,
        "26.3-0.1.0",
        &["Ab3dE9xQ".into()],
        &BTreeMap::new(),
        &BTreeMap::new(),
        &pinned,
        None,
    )
    .unwrap_err();
    assert!(matches!(&error, InstallError::Refused(message) if message == lines));

    // Without a pin the install takes the newest release it can fetch, not the marked one.
    let plan = plan_v2_install(
        &idx,
        "26.3-0.1.0",
        &["Ab3dE9xQ".into()],
        &BTreeMap::new(),
        &BTreeMap::new(),
        &options,
        None,
    )
    .unwrap();
    assert_eq!(plan.steps[0].version, "1.0.0");

    // An update to the marked release is refused the same way.
    let temp = tempfile::tempdir().unwrap();
    write_installed_mod(
        temp.path(),
        "wayfinder",
        &v1,
        Some(v2_entry(&project, &project.releases[0], &v1, false)),
        true,
    );
    let inv = inventory_v2(temp.path(), &idx).unwrap();
    let error = plan_v2_update(
        &idx,
        "26.3-0.1.0",
        &UpdateRequest::Strict(vec!["wayfinder".into()]),
        &inv,
        &BTreeMap::new(),
        &options,
        None,
    )
    .unwrap_err();
    assert!(matches!(&error, InstallError::Refused(message) if message == lines));

    // Update all leaves the marked release alone.
    let bulk = plan_v2_update(
        &idx,
        "26.3-0.1.0",
        &UpdateRequest::Bulk(vec!["wayfinder".into()]),
        &inv,
        &BTreeMap::new(),
        &options,
        None,
    )
    .unwrap();
    assert!(bulk.steps.is_empty());
}

#[test]
fn v2_an_installed_marked_release_with_its_jar_cached_is_not_flagged() {
    let gone = [jar("macaws_bridges", "3.0.0", 'a')];
    let v1 = smod_bytes("wayfinder", "Wayfinder", "1.0.0", "");
    let v2 = smod_bytes("wayfinder", "Wayfinder", "1.1.0", "");
    let project = project_with_marked_update(&gone, &v1, &v2);
    let idx = make_index(vec![project.clone()], Vec::new());
    let cache = tempfile::tempdir().unwrap();
    fs::write(gone[0].cache_path(cache.path()), b"the jar").unwrap();
    let options = V2InstallOptions {
        jar_cache: Some(cache.path().to_owned()),
        ..V2InstallOptions::default()
    };

    let temp = tempfile::tempdir().unwrap();
    write_installed_mod(
        temp.path(),
        "wayfinder",
        &v1,
        Some(v2_entry(&project, &project.releases[0], &v1, false)),
        true,
    );
    let inv = inventory_v2(temp.path(), &idx).unwrap();
    let plan = plan_v2_update(
        &idx,
        "26.3-0.1.0",
        &UpdateRequest::Strict(vec!["wayfinder".into()]),
        &inv,
        &BTreeMap::new(),
        &options,
        None,
    )
    .unwrap();
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].version, "1.1.0");
}
