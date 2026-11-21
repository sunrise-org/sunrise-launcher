//! Native Update through the existing developer SDK fixture and compiler-worker protocol.
use super::*;
use crate::managed_play::{Active, Held, HeldReason, HeldSource};
use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;

const ID: &str = "native_lantern";
const ARCHIVE: &str = "sunrise-native-mods/native_lantern.smod";
const EXTRA: &str = "sunrise-mods/extra_mod.smod";
const SOURCE: &[u8] = b"#[no_mangle]\npub extern \"C\" fn fixture_version() -> u32 { 1 }\n";

fn release(fixture: &mut Fixture, version: &str, pin: char, source: &[u8]) -> Vec<u8> {
    let tables = format!(
        "[native]\nsunrise='{}'\nmixins='{}'\ngame='{}'\n",
        pin.to_string().repeat(40),
        "c".repeat(40),
        backend::PINNED_GAME
    );
    let cargo = format!("[package]\nname='{ID}'\nversion='{version}'\n");
    fixture.release(
        ID,
        version,
        &tables,
        &[("Cargo.toml", cargo.as_bytes()), ("src/lib.rs", source)],
    )
}

fn configured(fixture: &mut Fixture) -> Vec<u8> {
    let runtime = fixture.root.join("sdk/runtime/bin");
    fs::create_dir_all(&runtime).unwrap();
    fs::copy(
        backend::version_dir(BUILD).join(backend::CLIENT),
        runtime.join(backend::CLIENT),
    )
    .unwrap();
    let rust_version = crate::rust_toolchain::VERSION;
    let rust =
        crate::rust_toolchain::root().join(format!("{rust_version}-x86_64-unknown-linux-gnu"));
    fs::create_dir_all(&rust).unwrap();
    fs::write(rust.join(".installed"), []).unwrap();
    fs::write(fixture.root.join("rust-version"), rust_version).unwrap();
    let tool = fixture.root.join("sdk-tool");
    fs::write(
        &tool,
        include_str!("../../../../../tests/ordinary_play/install/mutations/native_tool.py"),
    )
    .unwrap();
    fs::set_permissions(tool, fs::Permissions::from_mode(0o755)).unwrap();
    backend::Settings::default().allow_native_mods().unwrap();
    fixture.project(ID, "Native Lantern", index::Kind::NativeMod);
    let bytes = release(fixture, "1.0.0", 'a', SOURCE);
    fixture.install(&[ID]);
    assert!(mod_list::is_enabled(fixture.home(), ID));
    assert_eq!(fs::read(fixture.home().join(ARCHIVE)).unwrap(), bytes);
    assert_eq!(compiled(fixture), "1.0.0\n");
    bytes
}

fn compiled(fixture: &Fixture) -> String {
    fs::read_to_string(fixture.root.join("compiled")).unwrap()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HoldCase {
    PriorOn,
    PriorOff,
    Digest,
    Location,
    Source,
    Base,
}

fn hold_updates(fixture: &mut Fixture, succeeds: HoldCase, refusals: &[HoldCase]) {
    let predecessor = configured(fixture);
    mod_list::set_enabled(fixture.home(), ID, false).unwrap();
    let active = Profile::open(fixture.home(), None)
        .unwrap()
        .refresh(
            fixture.instance.native_profile,
            &native_mods::Cancellation::default(),
            |_| {},
            None,
            None,
        )
        .unwrap()
        .unwrap();
    assert_eq!(active.executable(), BUILD);
    let pointer = fixture.home().join(crate::profile_txn::POINTER);
    let base: Value = serde_json::from_slice(&fs::read(&pointer).unwrap()).unwrap();
    let next = release(fixture, "1.1.0", 'a', SOURCE);
    for case in refusals.iter().copied().chain([succeeds]) {
        let hold = Held::new(
            ID.into(),
            if case == HoldCase::Digest {
                [7; 32]
            } else {
                Sha256::digest(&predecessor).into()
            },
            if case == HoldCase::Location {
                "sunrise-native-mods/other.smod"
            } else {
                ARCHIVE
            }
            .into(),
            if case == HoldCase::Source {
                HeldSource::Local {
                    version: "1.0.0".into(),
                }
            } else {
                HeldSource::Registry {
                    project: ID.into(),
                    version: "1.0.0".into(),
                }
            },
            case != HoldCase::PriorOff,
            HeldReason::NoFit {
                build: if case == HoldCase::Base {
                    "v0.1.8"
                } else {
                    BUILD
                }
                .into(),
            },
        )
        .unwrap();
        // Switch publishes these typed receipts. Seed one on the real disabled generation.
        let mut with_hold = base.clone();
        let receipt_id = [8_u8; 16];
        with_hold["receipts"] = json!([{"id": receipt_id, "waiting_for_update": [hold], "removed": [], "acknowledged": true}]);
        fs::write(&pointer, serde_json::to_vec(&with_hold).unwrap()).unwrap();
        assert_eq!(
            Active::read_pointer(fixture.home())
                .unwrap()
                .unwrap()
                .holds()
                .count(),
            1,
            "{case:?}"
        );
        let before = fixture.snapshot(&[ARCHIVE]);
        let result = fixture.update(&[ID], &no_world_question);
        if case != succeeds {
            assert!(
                matches!(result, Err(InstallError::Refused(ref reason)) if reason.contains("installed hold")),
                "{case:?}: {result:?}"
            );
            assert_eq!(fixture.snapshot(&[ARCHIVE]), before, "{case:?}");
            assert_eq!(compiled(fixture), "1.0.0\n", "{case:?}");
            assert!(check_activation(fixture.home(), &[ID]).is_err(), "{case:?}");
            // Recovery retires the refused stage against its original pointer. Seed the
            // next independent fixture only after that exact preimage is settled.
            drop(Profile::open(fixture.home(), None).unwrap());
            assert_eq!(fixture.snapshot(&[ARCHIVE]), before, "{case:?}");
            continue;
        }
        assert_eq!(result.unwrap().updated.len(), 1);
        let after = Active::read_pointer(fixture.home()).unwrap().unwrap();
        let reenables = case == HoldCase::PriorOn;
        assert_eq!(mod_list::is_enabled(fixture.home(), ID), reenables);
        assert_eq!(after.executable() != BUILD, reenables);
        assert_eq!(after.holds().count(), 0);
        check_activation(fixture.home(), &[ID]).unwrap();
        assert_eq!(fs::read(fixture.home().join(ARCHIVE)).unwrap(), next);
        assert_eq!(compiled(fixture), "1.0.0\n1.1.0\n");
    }
}

#[test]
#[ignore = "daily; build the launcher binary first"]
fn exact_prior_on_hold_retires_and_restores_activation_after_native_proof() {
    Fixture::run(
        concat!(
            module_path!(),
            "::exact_prior_on_hold_retires_and_restores_activation_after_native_proof"
        ),
        |mut fixture| {
            hold_updates(&mut fixture, HoldCase::PriorOn, &[]);
        },
    );
}

#[test]
#[ignore = "daily; build the launcher binary first"]
fn mismatched_holds_refuse_and_exact_prior_off_retires_without_activation() {
    Fixture::run(
        concat!(
            module_path!(),
            "::mismatched_holds_refuse_and_exact_prior_off_retires_without_activation"
        ),
        |mut fixture| {
            hold_updates(
                &mut fixture,
                HoldCase::PriorOff,
                &[
                    HoldCase::Digest,
                    HoldCase::Location,
                    HoldCase::Source,
                    HoldCase::Base,
                ],
            );
        },
    );
}

#[test]
#[ignore = "daily; build the launcher binary first"]
fn bulk_update_can_keep_a_proved_unfit_root_and_advance_an_independent_sibling() {
    Fixture::run(
        concat!(
            module_path!(),
            "::bulk_update_can_keep_a_proved_unfit_root_and_advance_an_independent_sibling"
        ),
        |mut fixture| {
            let predecessor = configured(&mut fixture);
            fixture.project("extra_mod", "Fireflies", index::Kind::Mod);
            fixture.release("extra_mod", "1.0.0", "", &[]);
            fixture.install(&["extra_mod"]);
            let before = fixture.snapshot(&[ARCHIVE, EXTRA]);
            let ids = &[ID, "extra_mod"];
            let unfit = release(&mut fixture, "1.1.0", 'b', SOURCE);
            let bulk = |fixture: &Fixture| {
                fixture.apply(Action::Update(
                    UpdateRequest::Bulk(projects(ids)),
                    &no_world_question,
                ))
            };

            let strict = fixture.update(&[ID], &no_world_question).unwrap_err();
            assert!(
                matches!(strict, InstallError::Refused(ref detail) if detail.contains("Made for other Sunrise versions")),
                "{strict}"
            );
            assert_eq!(fixture.snapshot(&[ARCHIVE, EXTRA]), before);
            assert!(bulk(&fixture).unwrap().updated.is_empty());
            assert_eq!(fixture.snapshot(&[ARCHIVE, EXTRA]), before);

            let next = fixture.release("extra_mod", "1.2.0", "", &[]);
            assert!(matches!(
                fixture.update(ids, &no_world_question),
                Err(InstallError::Refused(_))
            ));
            assert_eq!(fixture.snapshot(&[ARCHIVE, EXTRA]), before);
            let archive = fixture.root.join(format!("{ID}-1.1.0.smod"));
            fs::remove_file(&archive).unwrap();
            assert!(matches!(bulk(&fixture), Err(InstallError::Download(_))));
            assert_eq!(fixture.snapshot(&[ARCHIVE, EXTRA]), before);

            fs::write(archive, unfit).unwrap();
            let batch = bulk(&fixture).unwrap();
            assert_eq!(batch.updated.len(), 1);
            let (previous, entry) = &batch.updated[0];
            assert_eq!(previous, "1.0.0");
            assert_eq!(entry.project, "extra_mod");
            assert_eq!(entry.version, "1.2.0");
            let after = fixture.snapshot(&[ARCHIVE, EXTRA]);
            assert_ne!(after[0], before[0]);
            assert_eq!(after[2], before[2]);
            assert_eq!(after[3], predecessor);
            assert_eq!(after[4], next);
            let row = |bytes: &[u8]| {
                let document: Value = serde_json::from_slice(bytes).unwrap();
                document["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|row| row["project"] == ID)
                    .unwrap()
                    .clone()
            };
            assert_eq!(row(&after[1]), row(&before[1]));
            assert_eq!(compiled(&fixture), "1.0.0\n");
        },
    );
}

#[test]
#[ignore = "daily; build the launcher binary first"]
fn native_compile_failure_keeps_the_working_predecessor() {
    Fixture::run(
        concat!(
            module_path!(),
            "::native_compile_failure_keeps_the_working_predecessor"
        ),
        |mut fixture| {
            configured(&mut fixture);
            let before = fixture.snapshot(&[ARCHIVE]);
            assert_ne!(Active::parse(&before[0]).unwrap().executable(), BUILD);
            release(
                &mut fixture,
                "1.1.0",
                'a',
                b"compile_error!(\"fixture compile error\");",
            );
            let error = fixture.update(&[ID], &no_world_question).unwrap_err();
            let InstallError::Compile(failure) = error else {
                panic!("expected a compiler failure, got {error:?}")
            };
            assert!(
                failure.detail.contains("Could not compile Native Lantern."),
                "{failure:?}"
            );
            assert_eq!(failure.changes.len(), 1);
            assert_eq!(failure.changes[0].predecessor.as_deref(), Some("1.0.0"));
            assert_eq!(compiled(&fixture), "1.0.0\n1.1.0\n");
            assert_eq!(fixture.snapshot(&[ARCHIVE]), before);
        },
    );
}
