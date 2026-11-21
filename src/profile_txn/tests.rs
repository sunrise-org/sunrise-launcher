use super::*;
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
use sunrise_launcher_smod::{manifest, Package};

const HOME: &str = "SUNRISE_PROFILE_JOURNAL_TEST_HOME";
const ACTION: &str = "SUNRISE_PROFILE_JOURNAL_TEST_ACTION";
const PACK: &str = "sunrise-mods/lantern.smod";
const LIST: &str = "sunrise-mods.txt";
const KEEP: &str = "sunrise-native-mods/unchanged.smod";
const OLD: &[u8] = b"{\"base\":\"nightly-old\",\"generation\":1,\"receipts\":[\"old\"]}\n";
const SWITCHED: &[u8] =
    b"{\"base\":\"nightly-new\",\"generation\":2,\"receipts\":[\"old\",\"new\"]}\n";
const INSTALLED: &[u8] = b"{\"base\":\"nightly-old\",\"generation\":2,\"receipts\":[\"old\"]}\n";

fn package(version: &str) -> Vec<u8> {
    Package::write_files(&format!("id='lantern'\nname='Lantern'\nversion='{version}'\napi='{}'\ncapabilities=['registries']\n", manifest::API), &[]).unwrap()
}

fn initialized(home: &Path, installed: bool) {
    fs::create_dir(home.join("mod-browser")).unwrap();
    fs::create_dir(home.join("sunrise-native-mods")).unwrap();
    fs::write(home.join(POINTER), OLD).unwrap();
    fs::write(home.join(LIST), "mod.lantern:false\n").unwrap();
    fs::write(home.join(KEEP), package("1.0.0")).unwrap();
    if installed {
        fs::create_dir(home.join("sunrise-mods")).unwrap();
        fs::write(home.join(PACK), package("1.0.0")).unwrap();
    }
}

fn seal(owner: &mut Admission, switching: bool) -> Sealed<'_> {
    let intent = if switching {
        Intent::Switch
    } else {
        Intent::Install
    };
    let pointer = if switching { SWITCHED } else { INSTALLED };
    let staging = owner
        .reserve(intent, BTreeSet::from(["nightly-new".into()]))
        .unwrap();
    let package_path = staging.path().join("candidate.smod");
    let list = staging.path().join("candidate-list");
    fs::write(&package_path, package("2.0.0")).unwrap();
    fs::write(&list, "mod.lantern:true\n").unwrap();
    staging
        .seal(
            pointer.to_vec(),
            &[(PACK.into(), Some(package_path)), (LIST.into(), Some(list))],
        )
        .unwrap()
}

fn pair(home: &Path, switching: bool, committed: bool) {
    assert_eq!(fs::read(home.join(KEEP)).unwrap(), package("1.0.0"));
    if committed {
        assert_eq!(
            fs::read(home.join(POINTER)).unwrap(),
            if switching { SWITCHED } else { INSTALLED }
        );
        assert_eq!(fs::read(home.join(PACK)).unwrap(), package("2.0.0"));
        assert_eq!(fs::read(home.join(LIST)).unwrap(), b"mod.lantern:true\n");
    } else {
        assert_eq!(fs::read(home.join(POINTER)).unwrap(), OLD);
        assert_eq!(fs::read(home.join(LIST)).unwrap(), b"mod.lantern:false\n");
        if switching {
            assert!(!home.join("sunrise-mods").exists());
        } else {
            assert_eq!(fs::read(home.join(PACK)).unwrap(), package("1.0.0"));
        }
    }
}

const FIRST_POINTER: &[u8] = b"{\"base\":\"nightly-original\",\"generation\":1}\n";

fn first_binding() -> Bootstrap {
    Bootstrap::new(
        "nightly-original".into(),
        "preferred-first-image".into(),
        [7; 16],
        [3; 32],
        serde_json::json!({"archive":"sunrise-linux-aarch64.tar.gz","revision":"original"}),
    )
}

#[test]
fn first_binding_and_signed_proof_survive_an_abandoned_prepare_and_pointer_undo() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let mut owner = Admission::open(home, None).unwrap();
    assert!(owner
        .bind_bootstrap(
            || Ok(first_binding()),
            |_| Err::<(), _>("asset gone".into())
        )
        .is_err());
    assert!(!home.join(RECORD).exists());
    let chosen = owner
        .bind_bootstrap(|| Ok(first_binding()), |_| Ok(()))
        .unwrap();
    let proven = owner
        .prove_bootstrap(|binding| {
            assert_eq!(binding, &chosen);
            Ok(serde_json::json!({"sha256":"fixture-signed-proof"}))
        })
        .unwrap();
    assert!(proven.proof.is_some());
    let refs = references(home).unwrap();
    assert_eq!(refs.targets, chosen.targets());
    assert!(refs.before.is_none() && refs.after.is_none() && refs.pointer.is_none());
    drop(owner);

    for boundary in ["bootstrap:prepare", "bootstrap:pointer-renamed"] {
        let actor = pause_actor(home, boundary);
        assert!(
            Admission::open(home, None).is_err(),
            "ongoing first Play holds admission"
        );
        let refs = references(home).unwrap(); // Reader takes no profile admission.
        assert_eq!(refs.targets, chosen.targets());
        assert!(refs.before.is_none());
        if boundary.ends_with("prepare") {
            assert!(refs.after.is_none() && refs.pointer.is_none());
        } else {
            assert_eq!(refs.after.as_deref(), Some(FIRST_POINTER));
            assert_eq!(refs.pointer.as_deref(), Some(FIRST_POINTER));
        }
        let stage = read(home)
            .unwrap()
            .unwrap()
            .operation()
            .unwrap()
            .stage
            .path(home);
        drop(actor);
        let mut next = Admission::open(home, None).unwrap();
        assert!(pointer_bytes(home).unwrap().is_none());
        assert!(!stage.exists(), "settlement retires only the old attempt");
        assert_eq!(
            next.bind_bootstrap::<()>(
                || panic!("new Nightly must not be selected"),
                |_| panic!("existing reference is not being created")
            )
            .unwrap(),
            proven
        );
        assert_eq!(
            next.prove_bootstrap(|_| panic!("new SDK proof must not be fetched"))
                .unwrap(),
            proven
        );
    }
    let mut owner = Admission::open(home, None).unwrap();
    owner
        .resume_bootstrap()
        .unwrap()
        .seal(FIRST_POINTER.to_vec(), &[])
        .unwrap()
        .publish(|| Ok(()))
        .unwrap();
    let refs = references(home).unwrap();
    assert!(refs.targets.is_empty() && refs.before.is_none() && refs.after.is_none());
    assert_eq!(refs.pointer.as_deref(), Some(FIRST_POINTER));
    assert!(!home.join(RECORD).exists());
    assert!(owner
        .bind_bootstrap(|| panic!("a committed profile cannot rebind"), |_| Ok(()))
        .is_err());
}

#[test]
fn reference_snapshot_reads_exact_old_and_new_pointers_and_refuses_unowned_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    initialized(home, true);
    let mut owner = Admission::open(home, None).unwrap();
    let staged = seal(&mut owner, false);
    let refs = references(home).unwrap();
    assert_eq!(refs.before.as_deref(), Some(OLD));
    assert_eq!(refs.after.as_deref(), Some(INSTALLED));
    assert_eq!(refs.pointer.as_deref(), Some(OLD));
    fs::write(home.join(POINTER), SWITCHED).unwrap();
    assert!(references(home).is_err());
    fs::write(home.join(POINTER), OLD).unwrap();
    drop(staged);
    drop(owner);
    drop(Admission::open(home, None).unwrap());
    fs::write(home.join(RECORD), b"{\"version\":1}").unwrap();
    assert!(references(home).is_err());
}

#[test]
fn refused_cutoff_keeps_the_original_bytes_and_success_uses_the_sealed_pointer() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    initialized(home, true);
    let mut admission = Admission::open(home, None).unwrap();
    let cause = seal(&mut admission, false)
        .publish(|| Err::<(), _>("declined".into()))
        .unwrap_err();
    assert_eq!(cause, "declined");
    pair(home, false, false);
    assert!(!home.join(RECORD).exists());
    seal(&mut admission, false).publish(|| Ok(())).unwrap();
    pair(home, false, true);
    assert!(!home.join(RECORD).exists());
    assert!(!home.join(RETIRED).exists());
}

#[test]
fn torn_marker_enospc_undo_consumes_the_old_pointer_and_reuses_both_progress_slots() {
    use progress::{Phase, Slots};
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    initialized(home, true);
    let mut admission = Admission::open(home, None).unwrap();
    let sealed = seal(&mut admission, false);
    let disk = read(home).unwrap().unwrap();
    let op = disk.operation().unwrap();
    let steps = op.ready.as_ref().unwrap().steps();
    let stage = op.stage.path(home);
    let old = stage.join("pointer-old");
    let pointer_preimage = Identity::file_at(&old).unwrap().unwrap();
    assert_eq!(fs::read(&old).unwrap(), OLD);
    let progress_file = stage.join("progress");
    let slots_preimage = Identity::file_at(&progress_file).unwrap().unwrap();
    let live_record = home.join(RECORD);
    let record_preimage = Identity::file_at(&live_record).unwrap().unwrap();
    progress::fail_marker(progress::MarkerFault::TornWrite);
    let error = disk.publish_at(home, || Ok(()), |_, _| Ok(())).unwrap_err();
    assert_eq!(error, at(&progress_file, progress::storage_full()));
    pair(home, false, true);
    let slots = || {
        Slots::open(&stage, disk.digest().unwrap(), steps)
            .unwrap()
            .current()
    };
    assert_eq!(
        (slots().phase, slots().frontier),
        (Phase::Publishing, steps)
    );
    drop(sealed);
    drop(admission);
    for (index, boundary) in ["undo-pointer", "undo-list"].into_iter().enumerate() {
        let actor = pause_actor(home, boundary);
        assert!(Admission::open(home, None).is_err());
        let progress = slots();
        assert_eq!(
            (progress.phase, progress.frontier),
            (Phase::Undoing, steps - index)
        );
        assert!(
            absent(&old).unwrap(),
            "restore must consume its staged preimage"
        );
        assert!(Identity::file_at(&home.join(POINTER)).unwrap() == Some(pointer_preimage));
        assert!(Identity::file_at(&progress_file).unwrap() == Some(slots_preimage));
        assert!(Identity::file_at(&live_record).unwrap() == Some(record_preimage));
        drop(actor);
    }
    drop(Admission::open(home, None).unwrap());
    pair(home, false, false);
    assert!(!live_record.exists());
    assert!(!stage.exists());
}

#[test]
fn whole_marker_after_failed_flush_is_retried_and_never_undone() {
    for attempts in [1, 2] {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        initialized(home, true);
        let mut admission = Admission::open(home, None).unwrap();
        let sealed = seal(&mut admission, false);
        let stage = sealed.0.path().to_owned();
        progress::fail_marker(progress::MarkerFault::Flush(attempts));
        let published = sealed.publish(|| Ok(()));
        if attempts == 1 {
            assert!(
                published.is_ok(),
                "a successful reflush confirms the commit"
            );
        } else {
            assert_eq!(
                published.unwrap_err(),
                at(&stage.join("progress"), progress::storage_full())
            );
            assert!(stage.join("pointer-old").is_file());
            assert!(home.join(RECORD).is_file());
        }
        pair(home, false, true);
        drop(admission);
        drop(Admission::open(home, None).unwrap());
        pair(home, false, true);
        assert!(!home.join(RECORD).exists() && !stage.exists());
    }
}

#[test]
fn composed_and_decomposed_destinations_cannot_enter_one_plan() {
    let composed = PathBuf::from("sunrise-mods/café.smod");
    let decomposed = PathBuf::from("sunrise-mods/cafe\u{301}.smod");
    assert_ne!(composed, decomposed);
    for existing in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        initialized(home, existing);
        if existing {
            fs::write(home.join(&composed), package("1.0.0")).unwrap();
            // On normalization-insensitive volumes the second spelling can already open
            // the first file. On other volumes it is a prospective across-volume alias.
            if let Some(other) = Identity::file_at(&home.join(&decomposed)).unwrap() {
                assert!(Identity::file_at(&home.join(&composed)).unwrap() == Some(other));
            }
        }
        let mut owner = Admission::open(home, None).unwrap();
        let stage = owner.reserve(Intent::Install, BTreeSet::new()).unwrap();
        let one = stage.path().join("one");
        let two = stage.path().join("two");
        fs::write(&one, package("2.0.0")).unwrap();
        fs::write(&two, package("3.0.0")).unwrap();
        assert!(stage
            .seal(
                INSTALLED.to_vec(),
                &[
                    (composed.clone(), Some(one)),
                    (decomposed.clone(), Some(two))
                ]
            )
            .is_err());
        assert_eq!(fs::read(home.join(POINTER)).unwrap(), OLD);
        if existing {
            assert_eq!(fs::read(home.join(&composed)).unwrap(), package("1.0.0"));
        } else {
            assert!(!home.join("sunrise-mods").exists());
        }
    }
}

#[test]
fn absent_nonportable_leaves_and_ascii_case_aliases_are_refused_before_cutoff() {
    for names in [
        vec!["lan\u{034f}tern.smod"], // Default ignorable, not just a non-ASCII letter.
        vec!["LANTER~1.smod"],
        vec!["CONOUT$.smod"],
        vec!["lantern.smod", "Lantern.smod"],
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        initialized(home, false);
        let mut owner = Admission::open(home, None).unwrap();
        let stage = owner.reserve(Intent::Install, BTreeSet::new()).unwrap();
        let changes = names
            .into_iter()
            .enumerate()
            .map(|(i, name)| {
                let source = stage.path().join(i.to_string());
                fs::write(&source, package("2.0.0")).unwrap();
                (Path::new("sunrise-mods").join(name), Some(source))
            })
            .collect::<Vec<_>>();
        assert!(stage.seal(INSTALLED.to_vec(), &changes).is_err());
        assert!(!home.join("sunrise-mods").exists());
        assert_eq!(fs::read(home.join(POINTER)).unwrap(), OLD);
    }
}

#[test]
fn different_basenames_that_gain_the_same_file_identity_are_refused_before_cutoff() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    initialized(home, true);
    let first = home.join(PACK);
    let second = PathBuf::from("sunrise-mods/second.smod");
    fs::write(home.join(&second), package("1.0.0")).unwrap();
    let mut owner = Admission::open(home, None).unwrap();
    let stage = owner.reserve(Intent::Install, BTreeSet::new()).unwrap();
    let sealed = stage
        .seal(
            INSTALLED.to_vec(),
            &[(PACK.into(), None), (second.clone(), None)],
        )
        .unwrap();
    // Preserve both before-hashes but alias the second entry just before publish.
    fs::remove_file(home.join(&second)).unwrap();
    fs::hard_link(&first, home.join(&second)).unwrap();
    assert!(sealed
        .publish::<()>(|| panic!("alias must be refused before cutoff"))
        .is_err());
    assert_eq!(fs::read(&first).unwrap(), package("1.0.0"));
    assert_eq!(fs::read(home.join(&second)).unwrap(), package("1.0.0"));
    assert_eq!(fs::read(home.join(POINTER)).unwrap(), OLD);
}

#[test]
fn unowned_bytes_and_unknown_records_are_never_replaced_or_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    initialized(home, false);
    fs::write(home.join(RECORD), b"{\"version\":1}").unwrap();
    assert!(Admission::open(home, None).is_err());
    assert_eq!(fs::read(home.join(RECORD)).unwrap(), b"{\"version\":1}");
    fs::remove_file(home.join(RECORD)).unwrap();
    let mut admission = Admission::open(home, None).unwrap();
    let sealed = seal(&mut admission, true);
    fs::create_dir(home.join("sunrise-mods")).unwrap();
    fs::write(home.join(PACK), package("9.0.0")).unwrap();
    assert!(sealed
        .publish::<()>(|| panic!("occupied target must be rejected before cutoff"))
        .is_err());
    assert_eq!(fs::read(home.join(PACK)).unwrap(), package("9.0.0"));
    assert_eq!(fs::read(home.join(POINTER)).unwrap(), OLD);
    let staging = admission.reserve(Intent::Install, BTreeSet::new()).unwrap();
    let outside = home.join("outside");
    fs::write(&outside, b"keep").unwrap();
    assert!(staging
        .seal(
            INSTALLED.to_vec(),
            &[(("../outside").into(), Some(outside.clone()))]
        )
        .is_err());
    assert_eq!(fs::read(outside).unwrap(), b"keep");
}

struct Actor(Child);
impl Drop for Actor {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn actor_command(home: &Path, action: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "profile_txn::tests::journal_actor",
            "--nocapture",
        ])
        .env(HOME, home)
        .env(ACTION, action)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    command
}

fn pause_actor(home: &Path, action: &str) -> Actor {
    let mut child = Actor(actor_command(home, action).spawn().unwrap());
    let stdout = child.0.stdout.take().unwrap();
    let (send, recv) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line == "txn-paused" {
                let _ = send.send(());
            }
        }
    });
    recv.recv_timeout(Duration::from_secs(20))
        .expect("actor reached the journal boundary");
    child
}

fn pause_here() {
    println!("txn-paused");
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().lock().read_line(&mut input).unwrap();
    assert_eq!(
        input, "release\n",
        "parent should stop the child or release the inherited lease"
    );
}

#[test]
fn journal_actor() {
    let Some(home) = std::env::var_os(HOME).map(PathBuf::from) else {
        return;
    };
    let action = std::env::var(ACTION).unwrap();
    match action.as_str() {
        "check-exclusion" => {
            pause_here();
            assert!(Admission::open(&home, None)
                .err()
                .unwrap()
                .contains("profile is in use"));
            return;
        }
        "staged-worker" => {
            let stage = read(&home)
                .unwrap()
                .unwrap()
                .operation()
                .unwrap()
                .stage
                .path(&home);
            fs::write(stage.join("worker.smod"), package("2.0.0")).unwrap();
            fs::create_dir(stage.join("worker-ready")).unwrap();
            await_fixture(|| stage.join("finish-worker").is_file());
            return;
        }
        _ => {}
    }
    let mut owner = Admission::claim(&home, None).unwrap();
    match action.as_str() {
        "launcher-with-staged-worker" => {
            use crate::process::{leases::StagedChild, owned::Owned};
            let stage = owner.reserve(Intent::Install, BTreeSet::new()).unwrap();
            let _worker = Owned::spawn_staged(
                actor_command(&home, "staged-worker"),
                stage.handoff(),
                StagedChild::Prepare,
            )
            .unwrap();
            await_fixture(|| stage.path().join("worker-ready").is_dir());
            pause_here();
        }
        "bootstrap:prepare" | "bootstrap:pointer-renamed" => {
            let staging = owner.resume_bootstrap().unwrap();
            if action.ends_with("prepare") {
                fs::write(staging.path().join("incomplete-worker-output"), b"private").unwrap();
                pause_here();
                return;
            }
            let sealed = staging.seal(FIRST_POINTER.to_vec(), &[]).unwrap();
            sealed
                .0
                .record
                .publish_at(
                    &home,
                    || Ok(()),
                    |_, boundary| {
                        if boundary == Boundary::Renamed {
                            pause_here();
                        }
                        Ok(())
                    },
                )
                .unwrap();
        }
        "undo-pointer" | "undo-list" | "retired" => {
            let disk = read(&home).unwrap().unwrap();
            let last = disk.operation().unwrap().ready.as_ref().unwrap().steps() - 1;
            disk.complete(
                &home,
                |i| {
                    if (action == "undo-pointer" && i == last)
                        || (action == "undo-list" && i == last - 1)
                    {
                        pause_here();
                    }
                },
                || {
                    if action == "retired" {
                        pause_here();
                    }
                },
            )
            .unwrap();
        }
        _ => {
            let (intent, stage) = action.split_once(':').unwrap();
            let switching = intent == "switch";
            let sealed = seal(&mut owner, switching);
            let record = &sealed.0.record;
            let steps = record.operation().unwrap().ready.as_ref().unwrap().steps();
            let index = usize::from(switching);
            let stop = match stage {
                "directory-before" => (0, Boundary::BeforeRename),
                "directory-renamed" => (0, Boundary::Renamed),
                "file-before" => (index, Boundary::BeforeRename),
                "file-renamed" => (index, Boundary::Renamed),
                "file-durable" => (index, Boundary::Durable),
                "pointer-before" => (steps - 1, Boundary::BeforeRename),
                "pointer-renamed" => (steps - 1, Boundary::Renamed),
                "pointer-durable" => (steps - 1, Boundary::Durable),
                "marker-before" => (steps, Boundary::BeforeMarker),
                "marker-written" => (steps, Boundary::WrittenMarker),
                "marker-durable" => (steps, Boundary::DurableMarker),
                _ => panic!("unknown boundary {stage}"),
            };
            record
                .publish_at(
                    &home,
                    || Ok(()),
                    |i, event| {
                        if (i, event) == stop {
                            pause_here();
                        }
                        Ok(())
                    },
                )
                .unwrap();
        }
    }
}

#[test]
fn move_recovers_a_killed_file_publication_before_copying() {
    use crate::data_location::{self, Roots};
    let root = tempfile::tempdir().unwrap();
    let source = Roots {
        launcher: root.path().join("launcher"),
        game: root.path().join("game"),
        build: root.path().join("build"),
    };
    for dir in [&source.launcher, &source.build, &source.game] {
        fs::create_dir(dir).unwrap();
    }
    initialized(&source.game, true);
    // This is a first file plan, with no previous committed Active pointer.
    fs::remove_file(source.game.join(POINTER)).unwrap();
    let mut actor = pause_actor(&source.game, "install:file-renamed");
    assert_eq!(fs::read(source.game.join(PACK)).unwrap(), package("2.0.0"));
    assert!(source.game.join(RECORD).is_file());
    actor.0.kill().unwrap();
    assert!(!actor.0.wait().unwrap().success());

    let mut reached_copy = false;
    let moved = data_location::migrate(
        &source,
        &root.path().join("destination"),
        &root.path().join("location.json"),
        |progress| {
            if progress == "Copying and verifying launcher data" {
                reached_copy = true;
                assert_eq!(fs::read(source.game.join(PACK)).unwrap(), package("1.0.0"));
                assert!(!source.game.join(RECORD).exists());
            }
        },
    )
    .unwrap();
    assert!(reached_copy);
    assert_eq!(fs::read(moved.game.join(PACK)).unwrap(), package("1.0.0"));
    assert_eq!(
        fs::read(moved.game.join(LIST)).unwrap(),
        b"mod.lantern:false\n"
    );
    assert!(!moved.game.join(RECORD).exists());
    assert!(!moved.game.join(POINTER).exists());
}

#[test]
fn another_process_sees_only_the_exact_old_or_committed_generation_after_kills() {
    let ordinary = [
        "file-before",
        "file-renamed",
        "file-durable",
        "pointer-before",
        "pointer-renamed",
        "pointer-durable",
        "marker-before",
        "marker-written",
        "marker-durable",
    ];
    for (intent, boundary) in ["install", "switch"]
        .into_iter()
        .flat_map(|intent| ordinary.map(|at| (intent, at)))
        .chain([
            ("switch", "directory-before"),
            ("switch", "directory-renamed"),
        ])
    {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let switching = intent == "switch";
        initialized(home, !switching);
        let actor = pause_actor(home, &format!("{intent}:{boundary}"));
        // The observer is already running and tries admission on both sides of the kill.
        assert!(Admission::open(home, None).is_err(), "{intent}:{boundary}");
        let pointer_replaced = matches!(
            boundary,
            "pointer-renamed"
                | "pointer-durable"
                | "marker-before"
                | "marker-written"
                | "marker-durable"
        );
        assert_eq!(
            fs::read(home.join(POINTER)).unwrap(),
            if pointer_replaced {
                if switching {
                    SWITCHED
                } else {
                    INSTALLED
                }
            } else {
                OLD
            }
        );
        let expected_package = if matches!(
            boundary,
            "directory-before" | "directory-renamed" | "file-before"
        ) {
            (!switching).then(|| package("1.0.0"))
        } else {
            Some(package("2.0.0"))
        };
        assert_eq!(fs::read(home.join(PACK)).ok(), expected_package);
        drop(actor);
        if switching && boundary == "pointer-renamed" {
            for undo in ["undo-pointer", "undo-list"] {
                let actor = pause_actor(home, undo);
                assert!(Admission::open(home, None).is_err());
                assert_eq!(fs::read(home.join(POINTER)).unwrap(), OLD);
                assert_eq!(fs::read(home.join(PACK)).unwrap(), package("2.0.0"));
                drop(actor);
            }
        }
        drop(Admission::open(home, None).unwrap());
        // A process kill leaves a whole cached marker visible; the next admission flushes it.
        // A torn marker (e.g. interrupted write) is exercised separately in the slot test.
        pair(
            home,
            switching,
            matches!(boundary, "marker-written" | "marker-durable"),
        );
        assert!(!home.join(RECORD).exists());
    }
}

#[cfg(unix)]
#[test]
fn undo_restores_restricted_pointer_and_package_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    initialized(home, true);
    for (file, mode) in [(POINTER, 0o600), (PACK, 0o640), (LIST, 0o604)] {
        fs::set_permissions(home.join(file), fs::Permissions::from_mode(mode)).unwrap();
    }
    drop(pause_actor(home, "install:pointer-renamed"));
    assert_eq!(fs::read(home.join(POINTER)).unwrap(), INSTALLED);
    drop(Admission::open(home, None).unwrap());
    pair(home, false, false);
    for (file, mode) in [(POINTER, 0o600), (PACK, 0o640), (LIST, 0o604)] {
        assert_eq!(
            fs::metadata(home.join(file)).unwrap().permissions().mode() & 0o7777,
            mode
        );
    }
}

#[cfg(windows)]
#[test]
fn a_read_only_file_that_cannot_be_replaced_is_refused_before_seal() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    initialized(home, true);
    let pointer = home.join(POINTER);
    let original = fs::metadata(&pointer).unwrap().permissions();
    let mut restricted = original.clone();
    restricted.set_readonly(true);
    fs::set_permissions(&pointer, restricted).unwrap();
    let mut owner = Admission::open(home, None).unwrap();
    assert!(owner.reserve(Intent::Install, BTreeSet::new()).is_err());
    assert!(!home.join(RECORD).exists());
    pair(home, false, false);
    fs::set_permissions(pointer, original).unwrap();
}

#[cfg(windows)]
#[test]
fn an_existing_named_data_stream_is_refused_before_seal_and_left_intact() {
    for zone in [&b"[ZoneTransfer]\r\nZoneId=3\r\n"[..], &b""[..]] {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        initialized(home, true);
        let mut stream = home.join(PACK).into_os_string();
        stream.push(":Zone.Identifier");
        fs::write(&stream, zone).unwrap();
        let mut owner = Admission::open(home, None).unwrap();
        let stage = owner.reserve(Intent::Install, BTreeSet::new()).unwrap();
        let source = stage.path().join("replace.smod");
        fs::write(&source, package("2.0.0")).unwrap();
        assert_eq!(
            stage
                .seal(INSTALLED.to_vec(), &[(PACK.into(), Some(source))])
                .err(),
            Some(at(
                &home.join(PACK),
                io::Error::from(io::ErrorKind::Unsupported)
            ))
        );
        pair(home, false, false);
        assert_eq!(fs::read(&stream).unwrap(), zone);
    }
}

#[test]
fn directory_restore_does_not_replace_an_occupied_stage_destination() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    initialized(home, false);
    let mut owner = Admission::open(home, None).unwrap();
    let sealed = seal(&mut owner, true);
    let record = &sealed.0.record;
    let op = record.operation().unwrap();
    let plan = op.ready.as_ref().unwrap();
    let stage = op.stage.path(home);
    let (src, dst, id) = plan.directory(home, &stage, 0).unwrap();
    let mut slots = progress::Slots::open(&stage, record.digest().unwrap(), plan.steps()).unwrap();
    slots.advance(progress::Phase::Publishing, 1).unwrap();
    drop(slots);
    durable::move_new_directory(&src, &dst).unwrap();
    fs::create_dir(&src).unwrap();
    assert!(record.complete(home, |_| {}, || {}).is_err());
    assert!(Identity::of(&dst).unwrap() == id);
    assert!(Identity::of(&src).unwrap() != id);
    fs::remove_dir(&src).unwrap();
    assert!(!record.complete(home, |_| {}, || {}).unwrap());
    pair(home, true, false);
}

#[cfg(windows)]
#[test]
fn interrupted_private_directory_replay_recovers_only_its_recorded_identity() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    initialized(home, false);
    let mut owner = Admission::open(home, None).unwrap();
    let sealed = seal(&mut owner, true);
    let record = &sealed.0.record;
    let op = record.operation().unwrap();
    let plan = op.ready.as_ref().unwrap();
    let stage = op.stage.path(home);
    let (src, dst, id) = plan.directory(home, &stage, 0).unwrap();
    let mut slots = progress::Slots::open(&stage, record.digest().unwrap(), plan.steps()).unwrap();
    slots.advance(progress::Phase::Publishing, 1).unwrap();
    slots.advance(progress::Phase::Undoing, 1).unwrap();
    drop(slots);
    let replay = src.with_extension("undo-directory");
    // Recreate a process killed during the private barrier's first rename.
    durable::move_new_directory(&src, &replay).unwrap();
    assert!(absent(&dst).unwrap() && Identity::of(&replay).unwrap() == id);
    assert!(!record.complete(home, |_| {}, || {}).unwrap());
    assert!(absent(&replay).unwrap());
    pair(home, true, false);
}

#[test]
fn a_killed_retirement_reclaims_its_exact_tree_but_never_a_loose_one() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    initialized(home, true);
    let unrelated = home.join("mod-browser/.install-txn-unowned");
    fs::create_dir(&unrelated).unwrap();
    fs::write(unrelated.join("keep"), b"not part of the journal").unwrap();
    drop(pause_actor(home, "install:marker-durable"));
    let stage = read(home)
        .unwrap()
        .unwrap()
        .operation()
        .unwrap()
        .stage
        .path(home);
    let cleanup = pause_actor(home, "retired");
    assert!(stage.is_dir());
    assert!(home.join(RETIRED).exists());
    assert!(Admission::open(home, None).is_err());
    drop(cleanup);
    drop(Admission::open(home, None).unwrap());
    assert!(!stage.exists());
    assert!(!home.join(RETIRED).exists());
    assert_eq!(
        fs::read(unrelated.join("keep")).unwrap(),
        b"not part of the journal"
    );
    pair(home, false, true);
}

#[test]
fn retirement_retries_a_failed_post_rename_barrier_before_erasing_preimages() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    initialized(home, true);
    let mut owner = Admission::open(home, None).unwrap();
    let sealed = seal(&mut owner, false);
    let record = &sealed.0.record;
    let stage = record.operation().unwrap().stage.path(home);
    let retired = home.join(RETIRED);
    record.publish_at(home, || Ok(()), |_, _| Ok(())).unwrap();
    durable::fail_next_retirement_barrier(&retired);
    assert!(record
        .complete(home, |_| {}, || {})
        .unwrap_err()
        .contains("retirement barrier failure"));
    assert!(retired.is_file() && !home.join(RECORD).exists());
    assert_eq!(fs::read(stage.join("pointer-old")).unwrap(), OLD);
    let refs = references(home).unwrap();
    assert_eq!(refs.before.as_deref(), Some(OLD));
    assert_eq!(refs.after.as_deref(), Some(INSTALLED));
    drop(sealed);
    drop(owner);

    durable::fail_next_retirement_barrier(&retired);
    drop(Admission::open(home, None).unwrap());
    assert!(
        retired.is_file() && stage.is_dir(),
        "a retry failure must keep both sources of proof"
    );
    assert_eq!(fs::read(stage.join("pointer-old")).unwrap(), OLD);
    drop(Admission::open(home, None).unwrap());
    assert!(!retired.exists() && !stage.exists());
    pair(home, false, true);
}

fn await_fixture(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the staged actor"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
#[test]
fn admission_without_successful_handoff_does_not_leave_an_incidental_fork_holding_the_gate() {
    use crate::process::{leases::StagedChild, owned::Owned};
    for fail_spawn in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        initialized(home, true);
        let owner = Admission::open(home, None).unwrap();
        // A fork before exec has the same open description, even with CLOEXEC set.
        let not_an_admitted_child = owner.gate.file.try_clone().unwrap();
        if fail_spawn {
            let child = Owned::spawn_staged(
                std::process::Command::new(home.join("no-such-executable")),
                owner.handoff(),
                StagedChild::Prepare,
            );
            assert_eq!(child.err().unwrap().kind(), io::ErrorKind::NotFound);
        }
        drop(owner);
        assert!(Admission::open(home, None).is_ok());
        drop(not_an_admitted_child);
    }
}

#[test]
fn staged_child_survives_launcher_death_and_blocks_an_already_open_launcher() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    initialized(home, true);
    let mut contender = pause_actor(home, "check-exclusion");
    let launcher = pause_actor(home, "launcher-with-staged-worker");
    let stage = read(home)
        .unwrap()
        .unwrap()
        .operation()
        .unwrap()
        .stage
        .path(home);
    drop(launcher); // Kill the real owner process, not merely its Rust lock object.
    contender
        .0
        .stdin
        .take()
        .unwrap()
        .write_all(b"release\n")
        .unwrap();
    assert!(contender.0.wait().unwrap().success());
    assert_eq!(
        fs::read(stage.join("worker.smod")).unwrap(),
        package("2.0.0")
    );
    fs::write(stage.join("finish-worker"), []).unwrap();
    await_fixture(|| Admission::open(home, None).is_ok());
    assert!(!stage.exists() && !home.join(RECORD).exists());
    pair(home, false, false);
}

#[test]
fn the_same_admission_seals_after_a_staged_worker_exits() {
    use crate::process::{leases::StagedChild, owned::Owned};
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    initialized(home, true);
    let mut owner = Admission::open(home, None).unwrap();
    let stage = owner.reserve(Intent::Install, BTreeSet::new()).unwrap();
    let mut worker = Owned::spawn_staged(
        actor_command(home, "staged-worker"),
        stage.handoff(),
        StagedChild::Compiler,
    )
    .unwrap();
    await_fixture(|| stage.path().join("worker-ready").is_dir());
    assert!(Admission::open(home, None).is_err());
    fs::write(stage.path().join("finish-worker"), []).unwrap();
    assert!(worker.wait().unwrap().success());
    assert!(
        Admission::open(home, None).is_err(),
        "worker exit must not unlock the parent"
    );
    let output = stage.path().join("worker.smod");
    stage
        .seal(
            INSTALLED.to_vec(),
            &[(PACK.into(), Some(output)), (LIST.into(), None)],
        )
        .unwrap()
        .publish(|| Ok(()))
        .unwrap();
    assert_eq!(fs::read(home.join(POINTER)).unwrap(), INSTALLED);
    assert_eq!(fs::read(home.join(PACK)).unwrap(), package("2.0.0"));
    assert!(
        Admission::open(home, None).is_err(),
        "publication keeps the owning profile"
    );
    drop(owner);
    // An unrelated fork may still briefly own a CLOEXEC reference to the handed-off file.
    await_fixture(|| Admission::open(home, None).is_ok());
}
