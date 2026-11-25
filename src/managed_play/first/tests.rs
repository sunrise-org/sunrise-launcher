use super::*;
use std::{
    fs,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const NAME: &str =
    "managed_play::first::tests::interrupted_prepare_retains_the_same_build_and_profile_exclusion";
const BUILD: &str = "v0.1.7";

#[test]
fn interrupted_prepare_retains_the_same_build_and_profile_exclusion() {
    let actor = |role| {
        let mut actor = Command::new(std::env::current_exe().unwrap());
        actor
            .args(["--exact", NAME, "--nocapture"])
            .env("FIRST_BINDING_ACTOR", role);
        actor
    };
    let Ok(role) = std::env::var("FIRST_BINDING_ACTOR") else {
        let temp = tempfile::tempdir().unwrap();
        let root = backend::plain(temp.path().canonicalize().unwrap());
        assert!(actor("scenario")
            .env("SUNRISE_HOME", root.join("game"))
            .env("SUNRISE_LAUNCHER_HOME", root.join("launcher"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .status()
            .unwrap()
            .success());
        return;
    };
    let home = backend::game_home();
    let fixture = home.with_file_name("prepare-ready");
    let finish = home.with_file_name("finish-prepare");
    match role.as_str() {
        "prepare" => {
            fs::create_dir(&fixture).unwrap();
            await_fixture(|| finish.is_file());
            let resources = backend::resources(BUILD);
            fs::create_dir_all(&resources).unwrap();
            fs::write(resources.join("sunrise-resources.json"), b"{}").unwrap();
        }
        "interrupted-launcher" => {
            let mut profile = Profile::open(&home, None).unwrap();
            let first = profile
                .begin_first(|| FirstBuild::installed(BUILD), |_| {})
                .unwrap();
            let preparation = backend::prepare_lock(BUILD).unwrap();
            let handoff = preparation.handoff(Some(first.handoff()));
            crate::preparer::run_staged(actor("prepare"), &handoff, &|_| {}).unwrap();
            Resources::prepare(BUILD, &preparation, Some(first.handoff())).unwrap();
            panic!("test should have killed this launcher while the preparer was held");
        }
        "already-open-launcher" => {
            let other_home = home.join("instances/other-game");
            fs::create_dir_all(&other_home).unwrap();
            let mut other = Profile::open(&other_home, None).unwrap();
            fs::create_dir(home.with_file_name("contender-ready")).unwrap();
            await_fixture(|| home.with_file_name("try-admission").is_file());
            assert!(Profile::open(&home, None).is_err());
            let second = other
                .begin_first(|| FirstBuild::installed(BUILD), |_| {})
                .unwrap();
            let error = second
                .prepare(&|_| {})
                .expect_err("the live child owns this shared base, not just one profile");
            assert!(error.contains("already being prepared"), "{error}");
        }
        "scenario" => {
            fs::create_dir_all(&home).unwrap();
            let cache = backend::version_dir(BUILD);
            fs::create_dir_all(&cache).unwrap();
            fs::write(cache.join(".installed"), b"fixture installed archive").unwrap();
            fs::write(cache.join(backend::CLIENT), b"fixture client bytes").unwrap();
            let mut contender = Running(
                actor("already-open-launcher")
                    .stdin(Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            await_fixture(|| home.with_file_name("contender-ready").is_dir());
            let mut interrupted = Running(
                actor("interrupted-launcher")
                    .stdin(Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            await_fixture(|| fixture.is_dir());
            let frozen = profile_txn::references(&home).unwrap();
            assert!(frozen.pointer.is_none());
            assert!(frozen.targets.contains(BUILD));
            interrupted.0.kill().unwrap();
            assert!(!interrupted.0.wait().unwrap().success());
            fs::write(home.with_file_name("try-admission"), []).unwrap();
            assert!(contender.0.wait().unwrap().success());
            fs::write(&finish, []).unwrap();
            let mut opened = None;
            await_fixture(|| {
                opened = Profile::open(&home, None).ok();
                opened.is_some()
            });
            let mut profile = opened.unwrap();
            let attempt = profile
                .begin_first(|| panic!("first build resolved a second time"), |_| {})
                .unwrap();
            assert_eq!(attempt.base(), BUILD);
            assert_eq!(attempt.preferred_image(), BUILD);
            attempt
                .prepare(&|_| panic!("a complete preparation was launched again"))
                .unwrap();
            let cancellation = native_mods::Cancellation::default();
            let image = attempt
                .exact_image(native_mods::Profile::Optimized, &cancellation, |_| {}, None)
                .unwrap();
            let active = attempt.commit(&image, &cancellation).unwrap();
            assert_eq!(profile.active().unwrap(), Some(active.clone()));
            active.verify_artifacts().unwrap();
            assert_eq!(
                fs::read(active.resource_path().join("sunrise-resources.json")).unwrap(),
                b"{}"
            );
        }
        role => panic!("unexpected role: {role}"),
    }
}

#[test]
fn game_replacements_leave_registry_rows_alone_and_advance_the_same_base() {
    use serde_json::{json, Value};
    use sunrise_launcher_smod::{index, manifest, mod_list};
    const CASE: &str = "managed_play::first::tests::game_replacements_leave_registry_rows_alone_and_advance_the_same_base";
    if std::env::var_os("PLAY_ADOPTION_FIXTURE").is_none() {
        let temp = tempfile::tempdir().unwrap();
        let root = backend::plain(temp.path().canonicalize().unwrap());
        assert!(Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CASE, "--nocapture"])
            .env("PLAY_ADOPTION_FIXTURE", "1")
            .env("SUNRISE_HOME", root.join("game"))
            .env("SUNRISE_LAUNCHER_HOME", root.join("launcher"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .status()
            .unwrap()
            .success());
        return;
    }
    let home = backend::game_home();
    let folder = backend::version_dir(BUILD);
    let resources = backend::resources(BUILD);
    fs::create_dir_all(&resources).unwrap();
    fs::write(resources.join("sunrise-resources.json"), b"{}").unwrap();
    fs::write(folder.join(".installed"), []).unwrap();
    fs::write(folder.join(backend::CLIENT), "unchanged client").unwrap();
    fs::create_dir_all(home.join(mod_list::DIR)).unwrap();
    let relative = format!("{}/lantern.smod", mod_list::DIR);
    let package = home.join(&relative);
    let write_package = |version| {
        let manifest = format!("id='lantern'\nname='Lantern'\nversion='{version}'\napi='{}'\ncapabilities=['registries']\n", manifest::API);
        fs::write(
            &package,
            manifest::Package::write_files(&manifest, &[]).unwrap(),
        )
        .unwrap();
    };
    write_package("1.0.0");
    let row = json!({
        "source": "sunrise", "project": "lantern", "slug": "lantern", "title": "Lantern",
        "kind": "mod", "version": "1.0.0", "number": "1.0.0", "file": relative,
        "checksum": format!("SHA-256:{}", index::sha256_file(&package).unwrap()),
        "page": "registry page", "future-field": {"keep": [1, 2]}
    });
    let opaque =
        json!({"source": "future", "file": "resourcepacks/other.zip", "opaque": [null, {}]});
    let path = home.join(sunrise_launcher_smod::index::browser_state::MANIFEST);
    let initial = json!({"entries": [opaque, row], "future-top": [false]});
    fs::write(&path, serde_json::to_vec(&initial).unwrap()).unwrap();
    let cancellation = native_mods::Cancellation::default();
    let mut profile = Profile::open(&home, None).unwrap();
    let first = profile
        .begin_first(|| FirstBuild::installed(BUILD), |_| {})
        .unwrap()
        .commit(BUILD, &cancellation)
        .unwrap();
    let mut previous = first;
    let mut expected = initial.clone();
    for version in ["1.1.0", "1.2.0"] {
        write_package(version);
        assert!(profile
            .verified_handoff::<(), String>(&previous, &cancellation, |_| panic!(
                "stale game launched"
            ))
            .is_err());
        // Play adopts the new bytes as its input on the same base; the registry row stays as
        // it was, since only a registry install or update restamps it.
        let current = profile
            .refresh(
                native_mods::Profile::Optimized,
                &cancellation,
                |_| {},
                None,
                None,
            )
            .unwrap()
            .unwrap();
        assert_eq!(current.profile, previous.profile);
        assert_eq!(current.generation, previous.generation + 1);
        assert_eq!(current.base, previous.base);
        assert_eq!(current.resources, previous.resources);
        assert_eq!(profile.active().unwrap(), Some(current.clone()));
        assert_eq!(inputs(&home).unwrap(), current.inputs);
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap(),
            expected
        );
        profile
            .verified_handoff(&current, &cancellation, |_| Ok::<_, String>(()))
            .unwrap();
        previous = current;
    }
    write_package("1.3.0");
    // An opaque row naming a different hard link still claims the same exact file.
    let alias = home.join("same-package.smod");
    fs::hard_link(&package, alias).unwrap();
    expected["entries"][0]["file"] = "same-package.smod".into();
    let before = serde_json::to_vec(&expected).unwrap();
    fs::write(&path, &before).unwrap();
    assert!(profile
        .refresh(
            native_mods::Profile::Optimized,
            &cancellation,
            |_| {},
            None,
            None,
        )
        .is_err());
    assert_eq!(profile.active().unwrap(), Some(previous));
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn receipt_acknowledgement_recovers_the_exact_pointer_across_all_durable_cuts() {
    use crate::profile_txn::Boundary;
    use sunrise_launcher_smod::manifest;
    const CASE: &str = "managed_play::first::tests::receipt_acknowledgement_recovers_the_exact_pointer_across_all_durable_cuts";
    const ID: [u8; 16] = [7; 16];
    let actor = |role| {
        let mut actor = Command::new(std::env::current_exe().unwrap());
        actor
            .args(["--exact", CASE, "--nocapture"])
            .env("RECEIPT_FIXTURE_ROLE", role);
        actor
    };
    let Ok(role) = std::env::var("RECEIPT_FIXTURE_ROLE") else {
        for cut in [
            "pointer-before",
            "pointer-after",
            "marker-before",
            "marker-after",
            "marker-torn",
        ] {
            let scratch = tempfile::tempdir().unwrap();
            let root = backend::plain(scratch.path().canonicalize().unwrap());
            assert!(
                actor("scenario")
                    .env("RECEIPT_FIXTURE_CUT", cut)
                    .env("SUNRISE_HOME", root.join("game"))
                    .env("SUNRISE_LAUNCHER_HOME", root.join("launcher"))
                    .env("XDG_CONFIG_HOME", root.join("config"))
                    .status()
                    .unwrap()
                    .success(),
                "failed receipt cut {cut}"
            );
        }
        return;
    };
    let cut = std::env::var("RECEIPT_FIXTURE_CUT").unwrap();
    let home = backend::game_home();
    let ready = home.join("receipt-ready");
    let pointer = home.join("mod-browser/active-build.json");
    let load = || fs::read(&pointer).unwrap();
    if role == "publish" {
        let mut profile = Profile::open(&home, None).unwrap();
        let shown = profile.active().unwrap().unwrap();
        profile
            .acknowledge_receipt_at(&shown, ID, |index, boundary| {
                let target = match cut.as_str() {
                    "pointer-before" => (0, Boundary::BeforeRename),
                    "pointer-after" => (0, Boundary::Renamed),
                    "marker-before" => (1, Boundary::BeforeMarker),
                    "marker-after" => (1, Boundary::DurableMarker),
                    _ => unreachable!(),
                };
                if (index, boundary) == target {
                    fs::write(&ready, []).unwrap();
                    await_fixture(|| false); // The parent kills the launcher at this actual cut.
                }
                Ok(())
            })
            .unwrap();
        panic!("the receipt publisher passed the selected cut without stopping");
    }
    assert_eq!(role, "scenario");
    let resources = backend::resources(BUILD);
    fs::create_dir_all(&resources).unwrap();
    fs::write(resources.join("sunrise-resources.json"), b"{}").unwrap();
    let build = backend::version_dir(BUILD);
    fs::write(build.join(".installed"), []).unwrap();
    fs::write(build.join(backend::CLIENT), b"local executable fixture").unwrap();
    let root = if cut == "marker-after" {
        HeldRoot::Content
    } else {
        HeldRoot::Native
    };
    fs::create_dir_all(home.join(root.directory())).unwrap();
    let mod_path = home.join(root.directory()).join("lantern.smod");
    let spec = format!(
        "id='lantern'\nname='Lantern'\nversion='1.0.0'\napi='{}'\ncapabilities=['registries']\n",
        manifest::API
    );
    let package = match root {
        HeldRoot::Native => manifest::Package::write_files(
            &format!(
                "{spec}[native]\nsunrise='{}'\ngame='26.3'\n",
                "a".repeat(40)
            ),
            &[
                ("Cargo.toml", b"[package]\nname='lantern'\nversion='1.0.0'"),
                ("src/lib.rs", b""),
            ],
        ),
        HeldRoot::Content => {
            manifest::Package::write_files(&spec, &[("data/lantern/fixture.json", b"{}")])
        }
    }
    .unwrap();
    fs::write(&mod_path, &package).unwrap();
    let mut profile = Profile::open(&home, None).unwrap();
    let mut shown = profile
        .begin_first(|| FirstBuild::installed(BUILD), |_| {})
        .unwrap()
        .commit(BUILD, &native_mods::Cancellation::default())
        .unwrap();
    use crate::managed_play::{Held, HeldReason, HeldRoot, HeldSource, Receipt};
    use sha2::{Digest, Sha256};
    let hold = Held::new(
        "lantern".into(),
        "Lanterns".into(),
        Sha256::digest(&package).into(),
        (root, "lantern.smod".into()),
        HeldSource::Local {
            version: "1.0.0".into(),
        },
        true,
        HeldReason::NoFit {
            detail: None,
            build: BUILD.into(),
        },
    )
    .unwrap();
    assert_eq!(home.join(hold.path()), mod_path);
    assert_eq!(hold.root(), root);
    for root in [HeldRoot::Native, HeldRoot::Content] {
        assert!(Held::new(
            hold.id().into(),
            hold.title().into(),
            *hold.digest(),
            (root, "CON.smod".into()),
            hold.source().clone(),
            hold.enabled_before(),
            hold.reason().clone(),
        )
        .is_err());
    }
    shown.receipts = vec![
        Receipt {
            id: ID,
            waiting_for_update: vec![hold.clone()],
            report: None,
            removed: Vec::new(),
            acknowledged: false,
        },
        Receipt {
            id: [8; 16],
            waiting_for_update: Vec::new(),
            report: None,
            removed: vec![("another".into(), "Another".into())],
            acknowledged: false,
        },
    ];
    let mut before = serde_json::to_vec_pretty(&shown).unwrap();
    assert_eq!(Active::parse(&before).unwrap(), shown);
    let seed: serde_json::Value = serde_json::from_slice(&before).unwrap();
    for (field, invalid) in [
        ("digest", serde_json::json!(vec![0; 16])),
        ("root", serde_json::json!("another_root")),
        ("leaf", serde_json::json!("../lantern.smod")),
        ("leaf", serde_json::json!("CON.smod")),
        ("leaf", serde_json::json!("LANTER~1.smod")),
        (
            "source",
            serde_json::json!({"kind":"other", "version":"1.0.0"}),
        ),
        (
            "reason",
            serde_json::json!({"kind":"no_fit", "build":"../somewhere"}),
        ),
        ("enabled_before", serde_json::json!("true")),
        ("title", serde_json::json!("\n")),
        ("unexpected", serde_json::json!("not authority")),
    ] {
        let mut invalid_pointer = seed.clone();
        invalid_pointer["receipts"][0]["waiting_for_update"][0][field] = invalid;
        assert!(
            Active::parse(&serde_json::to_vec(&invalid_pointer).unwrap()).is_err(),
            "accepted invalid {field}"
        );
    }
    let mut ambiguous = seed;
    ambiguous["receipts"][1]["waiting_for_update"] = serde_json::json!([hold.clone()]);
    assert!(Active::parse(&serde_json::to_vec(&ambiguous).unwrap()).is_err());
    let second = &mut ambiguous["receipts"][1]["waiting_for_update"][0];
    second["id"] = serde_json::json!("different");
    assert!(Active::parse(&serde_json::to_vec(&ambiguous).unwrap()).is_err());
    let second = &mut ambiguous["receipts"][1]["waiting_for_update"][0];
    second["root"] = serde_json::json!(match root {
        HeldRoot::Native => HeldRoot::Content,
        HeldRoot::Content => HeldRoot::Native,
    });
    second["leaf"] = serde_json::json!("Lantern.smod");
    assert!(Active::parse(&serde_json::to_vec(&ambiguous).unwrap()).is_err());
    before.push(b'\n');
    fs::write(&pointer, &before).unwrap();
    let mut stale = shown.clone();
    stale.generation += 1;
    assert!(profile.acknowledge_receipt(&stale, ID).is_err());
    assert_eq!(load(), before);
    drop(profile);

    if cut == "marker-torn" {
        let mut profile = Profile::open(&home, None).unwrap();
        profile_txn::tear_next_commit_marker();
        let error = profile.acknowledge_receipt(&shown, ID).unwrap_err();
        assert!(error.contains("28") || error.contains("112"), "{error}"); // ENOSPC / ERROR_DISK_FULL
        assert_eq!(load(), before);
    } else {
        let mut writer = Running(actor("publish").spawn().unwrap());
        await_fixture(|| ready.is_file());
        assert!(Profile::open(&home, None).is_err());
        assert_eq!(load() == before, cut == "pointer-before");
        writer.0.kill().unwrap();
        assert!(!writer.0.wait().unwrap().success());
    }
    let mut profile = Profile::open(&home, None).unwrap();
    let committed = cut == "marker-after";
    assert_eq!(load() == before, !committed);
    assert_eq!(profile.acknowledge_receipt(&shown, ID).unwrap(), !committed);
    let after = profile.active().unwrap().unwrap();
    let mut expected = shown.clone();
    expected.generation += 1;
    expected.receipts[0].acknowledged = true;
    assert_eq!(after, expected);
    assert_eq!(after.holds().collect::<Vec<_>>(), [&hold]);
    assert!(!profile.acknowledge_receipt(&shown, ID).unwrap());
    assert_eq!(fs::read(mod_path).unwrap(), package);
    assert_eq!(profile.active().unwrap(), Some(after));
}

#[test]
fn home_read_recovers_the_first_switch_receipt_after_a_killed_commit() {
    use crate::managed_play::{
        build_changes_hint, digest_file, display_active, read_build_changes, Held, HeldReason,
        HeldRoot, HeldSource, Receipt, ReceiptHint, ReceiptReport,
    };
    use crate::profile_txn::Boundary;
    use std::collections::BTreeSet;
    use sunrise_launcher_smod::{manifest, mod_list, Package};
    const CASE: &str = "managed_play::first::tests::home_read_recovers_the_first_switch_receipt_after_a_killed_commit";
    const ID: [u8; 16] = [6; 16];
    let actor = |role| {
        let mut actor = Command::new(std::env::current_exe().unwrap());
        actor
            .args(["--exact", CASE, "--nocapture"])
            .env("SWITCH_RECEIPT_ACTOR", role);
        actor
    };
    let Ok(role) = std::env::var("SWITCH_RECEIPT_ACTOR") else {
        let scratch = tempfile::tempdir().unwrap();
        let root = backend::plain(scratch.path().canonicalize().unwrap());
        assert!(actor("scenario")
            .env("SUNRISE_HOME", root.join("game"))
            .env("SUNRISE_LAUNCHER_HOME", root.join("launcher"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .status()
            .unwrap()
            .success());
        return;
    };
    let home = backend::game_home();
    let ready = home.join("marker-ready");
    let relative = Path::new(mod_list::DIR).join("fireflies.smod");
    let package = |id, name, version| {
        let manifest = format!(
            "id='{id}'\nname='{name}'\nversion='{version}'\napi='{}'\ncapabilities=['registries']",
            manifest::API
        );
        Package::write_files(&manifest, &[]).unwrap()
    };
    if role == "publish" {
        let mut profile = Profile::open(&home, None).unwrap();
        let _references = ReferencePublication::begin(&home).unwrap();
        let stage = profile
            .0
            .reserve(Intent::Switch, BTreeSet::from([BUILD.into()]))
            .unwrap();
        let mut pointer = Active::capture(
            BUILD,
            BUILD,
            &home,
            inputs(&home).unwrap(),
            Generation::First {
                profile: [4; 16],
                resources: Resources::capture(BUILD, &stage).unwrap(),
                variant: None,
            },
        )
        .unwrap();
        let holds = vec![Held::new(
            "fireflies".into(),
            "Fireflies".into(),
            digest_file(&home.join(&relative)).unwrap().unwrap(),
            (HeldRoot::Content, "fireflies.smod".into()),
            HeldSource::Registry {
                project: "fireflies".into(),
                version: "1.0".into(),
                repository: None,
            },
            true,
            HeldReason::NoFit {
                detail: None,
                build: BUILD.into(),
            },
        )
        .unwrap()];
        pointer.receipts.push(Receipt {
            id: ID,
            report: Some(ReceiptReport::new(Some(BUILD.into()), &holds)),
            waiting_for_update: holds,
            removed: vec![("Lamp".into(), "Taken down".into())],
            acknowledged: false,
        });
        stage
            .seal(serde_json::to_vec(&pointer).unwrap(), &[])
            .unwrap()
            .publish_observed(
                || Ok(()),
                |_, cut| {
                    if cut == Boundary::DurableMarker {
                        fs::write(&ready, []).unwrap();
                        await_fixture(|| false);
                    }
                    Ok(())
                },
            )
            .unwrap();
        panic!("publisher did not stop at the durable marker");
    }
    if role == "restart" {
        assert_eq!(
            build_changes_hint(&home).unwrap(),
            Some(ReceiptHint::Recovery)
        );
        assert!(
            display_active(&home).unwrap().is_none(),
            "the repaint snapshot still sees the pre-Switch pointer"
        );
        let (_, active) = read_build_changes(&home).unwrap().unwrap();
        let receipt = active.pending_changes().unwrap();
        assert_eq!(receipt.id, ID);
        assert_eq!(receipt.base(), Some(BUILD));
        assert_eq!(receipt.removed[0].0, "Lamp");
        assert_eq!(
            receipt.held().collect::<Vec<_>>(),
            [("Fireflies", true, false)]
        );
        assert!(!profile_txn::recovery_pending(&home).unwrap());

        // Exercise the real same-base publication after the caller has checked a replacement.
        // No test override is granted registry authority by this fixture.
        let mut profile = Profile::open(&home, None).unwrap();
        assert_eq!(
            profile
                .enable_unheld(&["fireflies", "moths"], |home, _, ids| {
                    sunrise_launcher_smod::mod_list::set_all_enabled(home, ids, true)
                })
                .unwrap(),
            [(
                "fireflies",
                HeldReason::NoFit {
                    detail: None,
                    build: BUILD.into()
                }
            )]
        );
        assert!(!sunrise_launcher_smod::mod_list::is_enabled(
            &home,
            "fireflies"
        ));
        assert!(sunrise_launcher_smod::mod_list::is_enabled(&home, "moths"));
        let edit = profile
            .begin_edit(|| panic!("the build is already committed"), |_| {})
            .unwrap();
        let candidate = edit.candidate().unwrap();
        let replacement = package("fireflies", "Fireflies", "2.0");
        fs::write(candidate.path().join(&relative), &replacement).unwrap();
        sunrise_launcher_smod::mod_list::set_enabled(candidate.path(), "fireflies", true).unwrap();
        let cancellation = native_mods::Cancellation::default();
        let proof = edit
            .prove(
                candidate.path(),
                native_mods::Profile::Optimized,
                &cancellation,
                |_| {},
                None,
            )
            .unwrap();
        let updated = edit
            .publish_install(
                proof,
                &cancellation,
                &BTreeSet::from(["fireflies".into()]),
                || Ok(()),
            )
            .unwrap();
        assert_eq!(updated.holds().count(), 0);
        assert_eq!(fs::read(home.join(relative)).unwrap(), replacement);
        let original_report = updated.pending_changes().unwrap();
        assert_eq!(original_report.id, ID);
        assert_eq!(original_report.base(), Some(BUILD));
        assert_eq!(
            original_report.held().collect::<Vec<_>>(),
            [("Fireflies", true, false)]
        );
        assert_eq!(original_report.removed, receipt.removed);
        return;
    }
    assert_eq!(role, "scenario");
    fs::create_dir_all(home.join(mod_list::DIR)).unwrap();
    fs::write(
        home.join(relative),
        package("fireflies", "Fireflies", "1.0"),
    )
    .unwrap();
    fs::write(
        home.join(mod_list::DIR).join("moths.smod"),
        package("moths", "Moths", "1.0"),
    )
    .unwrap();
    let resources = backend::resources(BUILD);
    fs::create_dir_all(&resources).unwrap();
    fs::write(resources.join("sunrise-resources.json"), b"{}").unwrap();
    let build = backend::version_dir(BUILD);
    fs::write(build.join(".installed"), []).unwrap();
    fs::write(build.join(backend::CLIENT), b"local executable fixture").unwrap();
    let mut writer = Running(actor("publish").spawn().unwrap());
    await_fixture(|| ready.is_file());
    writer.0.kill().unwrap();
    assert!(!writer.0.wait().unwrap().success());
    assert!(actor("restart").status().unwrap().success());
}

#[cfg(unix)]
#[test]
fn first_play_preparer_subcommands_keep_admission_and_never_trust_an_unrecorded_cache() {
    use std::os::unix::fs::PermissionsExt;

    const CASE: &str = "managed_play::first::tests::first_play_preparer_subcommands_keep_admission_and_never_trust_an_unrecorded_cache";
    let actor = |role| {
        let mut actor = Command::new(std::env::current_exe().unwrap());
        actor
            .args(["--exact", CASE, "--nocapture"])
            .env("PREPARE_CASE_ROLE", role);
        actor
    };
    let Ok(role) = std::env::var("PREPARE_CASE_ROLE") else {
        for pass in ["needs", "extras", "data-file"] {
            let temp = tempfile::tempdir().unwrap();
            let root = backend::plain(temp.path().canonicalize().unwrap());
            assert!(actor("scenario")
                .env("PREPARE_CASE_PASS", pass)
                .env("SUNRISE_HOME", root.join("game"))
                .env("SUNRISE_LAUNCHER_HOME", root.join("launcher"))
                .env("XDG_CONFIG_HOME", root.join("config"))
                .status()
                .unwrap()
                .success());
        }
        return;
    };
    let pass = std::env::var("PREPARE_CASE_PASS").unwrap();
    let home = backend::game_home();
    let resources = backend::resources(BUILD);
    let root = home.parent().unwrap();
    let prepared = || {
        fs::create_dir_all(&resources).unwrap();
        fs::write(resources.join("sunrise-resources.json"), "{}").unwrap();
    };
    let admit = || Profile::open(&home, None);
    if role == "other-home" {
        fs::create_dir_all(&home).unwrap();
        let mut profile = admit().unwrap();
        fs::write(root.join("other-admitted"), []).unwrap();
        await_fixture(|| root.join("try-other").is_file());
        let attempt = profile
            .begin_first(|| FirstBuild::installed(BUILD), |_| {})
            .unwrap();
        let result = if pass == "needs" {
            attempt.prepare(&|_| panic!("a second preparer started"))
        } else {
            attempt
                .commit(BUILD, &native_mods::Cancellation::default())
                .map(drop)
        };
        let error = result.expect_err("another home cannot write or seal this version");
        assert!(error.contains("already being prepared"), "{error}");
        assert!(
            backend::prepare_lock(BUILD).is_err(),
            "explicit Prepare uses the same lock"
        );
        fs::write(root.join("other-refused"), []).unwrap();
        return;
    }
    if role == "startup" {
        backend::reuse_prepared_everywhere();
        backend::reuse_prepared_for(BUILD);
        assert!(
            !root.join("startup-wrote").exists(),
            "startup started a preparer while its version was locked"
        );
        return;
    }
    if role == "attempt" {
        let mut profile = admit().unwrap();
        let attempt = profile
            .begin_first(|| FirstBuild::installed(BUILD), |_| {})
            .unwrap();
        attempt.prepare(&|_| {}).unwrap();
        attempt
            .commit(BUILD, &native_mods::Cancellation::default())
            .unwrap();
        panic!("the scenario should kill Play while its preparer is running");
    }
    assert_eq!(role, "scenario");
    fs::create_dir_all(&home).unwrap();
    let cache = backend::version_dir(BUILD);
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join(".installed"), []).unwrap();
    fs::write(cache.join(backend::CLIENT), b"local fixture client").unwrap();
    let binary = cache.join(crate::preparer::BINARY);
    fs::write(
        &binary,
        r#"#!/bin/sh
set -eu
cmd=$1
shift
out=
while [ "$#" -ne 0 ]; do
  if [ "$1" = --output ]; then out=$2; shift; fi
  shift
done
root=${SUNRISE_HOME%/*}
if [ "$PREPARE_CASE_ROLE" = startup ]; then : > "$root/startup-wrote"; exit 2; fi
# Write the accelerator before announcing the interruption, while it still has no evidence.
if [ "$cmd" = data-file ]; then printf 'fixture data' > "$out/sunrise-data.bin"; fi
if [ "$cmd" = "$PREPARE_CASE_PASS" ] && [ ! -f "$root/finish" ]; then
  : > "$root/entered"
  n=0
  while [ ! -f "$root/finish" ] && [ "$n" -lt 1500 ]; do sleep 0.01; n=$((n+1)); done
  [ -f "$root/finish" ] || exit 1
fi
case "$cmd" in
  needs) printf '%s\n' '{"format":1,"server_jar":false,"jdk":false}';;
  missing-extras) printf '%s\n' '{"format":1,"missing":[{"kind":"sounds","what":"sounds"}]}';;
  extras) printf 'extra' > "$out/extra"; printf '%s\n' '{"format":1,"added":["sounds"]}';;
  data-file) printf '%s\n' '{"format":1,"written":true}';;
  *) exit 2;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    if pass != "needs" {
        prepared();
    }
    let mut contender = Running(
        actor("other-home")
            .env("SUNRISE_HOME", root.join("other-game"))
            .spawn()
            .unwrap(),
    );
    await_fixture(|| root.join("other-admitted").is_file());
    let mut attempt = Running(actor("attempt").spawn().unwrap());
    await_fixture(|| root.join("entered").is_file());
    attempt.0.kill().unwrap();
    assert!(!attempt.0.wait().unwrap().success());
    assert!(
        admit().is_err(),
        "the live {pass} child lost admission when Play died"
    );
    assert!(profile_txn::references(&home).unwrap().pointer.is_none());
    fs::write(root.join("try-other"), []).unwrap();
    await_fixture(|| root.join("other-refused").is_file());
    assert!(contender.0.wait().unwrap().success());
    assert!(actor("startup")
        .env("SUNRISE_HOME", root.join("other-game"))
        .status()
        .unwrap()
        .success());
    fs::write(root.join("finish"), []).unwrap();
    let mut resumed = None;
    await_fixture(|| {
        resumed = admit().ok();
        resumed.is_some()
    });
    if pass == "needs" {
        // The fixture does not implement Mojang input resolution. Supply prepared inputs
        // after proving the real `needs` call survived and held off the next launcher.
        prepared();
    }
    let mut profile = resumed.unwrap();
    let attempt = profile
        .begin_first(|| panic!("the unstamped local selection changed"), |_| {})
        .unwrap();
    let active = attempt
        .commit(BUILD, &native_mods::Cancellation::default())
        .unwrap();
    assert_eq!(
        fs::read(active.resource_path().join("extra")).unwrap(),
        b"extra"
    );
    assert_eq!(
        fs::read(active.resource_path().join(crate::preparer::DATA_FILE)).unwrap(),
        b"fixture data"
    );
    assert_eq!(active.allow_accelerator(), pass != "data-file");
    active.verify_artifacts().unwrap();
}

struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn await_fixture(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !ready() {
        assert!(Instant::now() < deadline, "first-Play test stage timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The pointer the released v1.2.0 launcher wrote on its first Play of a Dev profile, byte for
/// byte: no receipts, no build choice, no mod locks.
#[test]
fn a_pointer_written_by_the_released_launcher_is_read_and_kept() {
    let bytes = include_bytes!("../../../tests/fixtures/active-build-v1.2.0.json");
    let active = Active::parse(bytes).unwrap();
    assert_eq!(active.base(), "dev-2026-11-22-7cfdf81");
    assert_eq!(active.executable(), "dev-2026-11-22-7cfdf81");
    assert_eq!(active.generation(), 1);
    assert!(active.build_choice().is_none() && active.mod_locks().is_empty());
    // Written back, it stays the released shape, so that launcher can still read it.
    assert_eq!(serde_json::to_vec(&active).unwrap(), bytes);

    // A refusal names the check that made it.
    let mut zero = serde_json::from_slice::<serde_json::Value>(bytes).unwrap();
    zero["generation"] = 0.into();
    let refused = Active::parse(&serde_json::to_vec(&zero).unwrap()).unwrap_err();
    assert!(refused.contains("managed_play.rs:"), "{refused}");
}
