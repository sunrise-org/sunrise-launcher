use super::*;
#[cfg(unix)]
use std::cell::RefCell;

/// A tool that reads the plan, then says what `events` holds, one line each; `CONSENT` waits for the answer.
#[cfg(unix)]
fn fake(dir: &Path, events: &[&str]) -> std::process::Command {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).unwrap();
    let mut script =
        String::from("#!/bin/sh\nread plan\necho \"$plan\" > \"$(dirname \"$0\")/plan\"\n");
    for event in events {
        match *event {
            "CONSENT" => script.push_str(
                "echo '{\"event\":\"consent\",\"added_crates\":2,\"build_scripts\":[\"syn\"],\"proc_macros\":[]}'\nread answer\necho \"$answer\" > \"$(dirname \"$0\")/answer\"\n",
            ),
            "QUIET" => script.push_str(
                "echo '{\"event\":\"consent\",\"added_crates\":3,\"build_scripts\":[],\"proc_macros\":[]}'\nread answer\necho \"$answer\" > \"$(dirname \"$0\")/answer\"\n",
            ),
            "MACRO" => script.push_str(
                "echo '{\"event\":\"consent\",\"added_crates\":1,\"build_scripts\":[],\"proc_macros\":[\"serde_derive\"]}'\nread answer\necho \"$answer\" > \"$(dirname \"$0\")/answer\"\n",
            ),
            "ERRORS" => script.push_str("printf 'warming up\\n\\377 unknown contract\\n' >&2\n"),
            "EXIT1" => script.push_str("exit 1\n"),
            "EXIT2" => script.push_str("exit 2\n"),
            "EXIT3" => script.push_str("exit 3\n"),
            "SIGNAL" => script.push_str("kill -TERM $$\n"),
            event => script.push_str(&format!("echo '{event}'\n")),
        }
    }
    let program = dir.join("tool");
    std::fs::write(&program, script).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::process::Command::new(program)
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn a_tool_run_reports_its_mods_asks_consent_and_ends_built_or_blamed() {
    let dir = std::env::temp_dir().join(format!("sunrise-native-tool-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let started = RefCell::new(Vec::new());
    let on_mod = |id: &str, done: usize, total: usize| {
        started.borrow_mut().push(format!("{id} {done}/{total}"))
    };
    let asked = RefCell::new(0);
    let consent = |question: &Consent| {
        *asked.borrow_mut() += question.added_crates;
        true
    };
    let built = run(
        fake(
            &dir,
            &[
                "not an event",
                r#"{"event":"mod","id":"greeter","done":1,"total":1}"#,
                "CONSENT",
                r#"{"event":"built","route":"prebuilt","binary":"/b/game","libraries":["/b/lib.so"],"records":[{"file":"lib.so","sha256":"abc","kind":"mod","mod":"greeter","identity":"fixture"}],"mods":["greeter"],"internals":{"greeter":[]},"mixins":"abc","used":[]}"#,
            ],
        ),
        r#"{"contract":2}"#,
        Some(&consent),
        &on_mod,
        &Cancellation::default(),
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        (built.route.name(), built.binary.as_path()),
        ("prebuilt", Path::new("/b/game"))
    );
    assert_eq!(*started.borrow(), ["greeter 1/1"]);
    assert_eq!(*asked.borrow(), 2);
    assert_eq!(
        std::fs::read_to_string(dir.join("answer")).unwrap().trim(),
        "true"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("plan")).unwrap().trim(),
        r#"{"contract":2}"#
    );

    let blamed = r#"{"event":"failed","error":"Compiling greeter failed (101)","blame":{"mod":"greeter","kind":"compile"},"used":[]}"#;
    let failed = run(
        fake(&dir, &[blamed, "EXIT1"]),
        "{}",
        None,
        &on_mod,
        &Cancellation::default(),
        None,
        None,
    )
    .unwrap_err();
    assert_eq!(
        failed.blame,
        Some(Blame {
            mod_id: "greeter".into(),
            kind: BlameKind::Compile
        })
    );
    for events in [&[blamed][..], &[blamed, "EXIT3"][..]] {
        assert!(run(
            fake(&dir, events),
            "{}",
            None,
            &on_mod,
            &Cancellation::default(),
            None,
            None,
        )
        .unwrap_err()
        .blame
        .is_none());
    }
    let silent = run(
        fake(&dir, &[]),
        "{}",
        None,
        &on_mod,
        &Cancellation::default(),
        None,
        None,
    )
    .unwrap_err();
    assert!(
        silent.error.contains("without saying why"),
        "{}",
        silent.error
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Crates with no build script and no proc macro run nothing while building, so the player is
/// not asked about them and the tool is told yes (#104).
#[cfg(unix)]
#[test]
fn crates_that_run_nothing_while_building_are_not_asked_about() {
    let dir = tempfile::tempdir().unwrap();
    let built = r#"{"event":"built","route":"prebuilt","binary":"/b/game","libraries":[],"records":[],"mods":[],"internals":{},"mixins":"x","used":[]}"#;
    run(
        fake(dir.path(), &["QUIET", built]),
        "{}",
        Some(&|_: &Consent| unreachable!("nothing runs while building")),
        &|_, _, _| {},
        &Cancellation::default(),
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("answer"))
            .unwrap()
            .trim(),
        "true"
    );

    // A proc macro alone still runs while building, so it is asked about.
    let macros = tempfile::tempdir().unwrap();
    let asked = RefCell::new(Vec::new());
    run(
        fake(macros.path(), &["MACRO", built]),
        "{}",
        Some(&|question: &Consent| {
            asked.borrow_mut().extend(question.proc_macros.clone());
            false
        }),
        &|_, _, _| {},
        &Cancellation::default(),
        None,
        None,
    )
    .unwrap();
    assert_eq!(*asked.borrow(), ["serde_derive"]);
    assert_eq!(
        std::fs::read_to_string(macros.path().join("answer"))
            .unwrap()
            .trim(),
        "false"
    );
}

/// The yes given without asking is bound to the attempt's original permission like an asked
/// one: a revoke and regrant before the question refuses it (#104 on #157).
#[test]
#[cfg(unix)]
fn a_regrant_refuses_the_yes_given_without_asking() {
    let _scratch = super::super::tests::scratch_state();
    let settings = RefCell::new(crate::backend::Settings::default());
    settings.borrow_mut().allow_native_mods().unwrap();
    let current = Cancellation::default();
    current.admit_native_work().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let failure = run(
        fake(
            dir.path(),
            &[
                r#"{"event":"mod","id":"greeter","done":1,"total":1}"#,
                "QUIET",
                "EXIT1",
            ],
        ),
        "{}",
        Some(&|_: &Consent| unreachable!("nothing runs while building")),
        &|_, _, _| {
            settings.borrow_mut().revoke_native_mods().unwrap();
            settings.borrow_mut().allow_native_mods().unwrap();
        },
        &current,
        None,
        None,
    )
    .unwrap_err();
    assert!(
        failure.error.contains("Native mod permission changed"),
        "{}",
        failure.error
    );
    assert_ne!(
        std::fs::read_to_string(dir.path().join("answer"))
            .unwrap_or_default()
            .trim(),
        "true"
    );
}

#[test]
#[cfg(unix)]
fn a_regrant_cannot_admit_a_stale_sdk_check_or_registry_yes() {
    let _scratch = super::super::tests::scratch_state();
    let mut settings = crate::backend::Settings::default();
    settings.allow_native_mods().unwrap();
    let cancellation = Cancellation::default();
    cancellation.admit_native_work().unwrap();
    settings.revoke_native_mods().unwrap();
    settings.allow_native_mods().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let error = ask(
        fake(dir.path(), &[]),
        "native-check",
        &serde_json::json!({}),
        &cancellation,
    )
    .unwrap_err();
    assert!(error.contains("Native mod permission changed"), "{error}");
    assert!(!dir.path().join("plan").exists());

    let current = Cancellation::default();
    current.admit_native_work().unwrap();
    let settings = RefCell::new(settings);
    let consent = |_: &Consent| {
        settings.borrow_mut().revoke_native_mods().unwrap();
        settings.borrow_mut().allow_native_mods().unwrap();
        true
    };
    let failure = run(
        fake(dir.path(), &["CONSENT", "EXIT1"]),
        "{}",
        Some(&consent),
        &|_, _, _| unreachable!(),
        &current,
        None,
        None,
    )
    .unwrap_err();
    assert!(
        failure.error.contains("Native mod permission changed"),
        "{}",
        failure.error
    );
    assert_ne!(
        std::fs::read_to_string(dir.path().join("answer"))
            .unwrap_or_default()
            .trim(),
        "true"
    );
    assert!(
        dir.path().join("plan").is_file(),
        "the revision was valid at spawn"
    );
}

#[test]
#[cfg(unix)]
#[ignore = "daily"]
fn not_starting_a_worker_does_not_retain_its_staged_directory() {
    let dir = tempfile::tempdir().unwrap();
    let _scratch = super::super::tests::scratch_state();
    let mut settings = crate::backend::Settings::default();
    settings.allow_native_mods().unwrap();
    let stale = Cancellation::default();
    stale.admit_native_work().unwrap();
    settings.revoke_native_mods().unwrap();
    let no_consent = Cancellation::default();
    for (command, cancellation, expected) in [
        (
            std::process::Command::new(dir.path().join("missing-program")),
            &no_consent,
            "couldn't run",
        ),
        (fake(dir.path(), &[]), &stale, "permission changed"),
    ] {
        let mut workspace = Workspace::new(dir.path()).unwrap();
        let path = workspace.path().to_owned();
        std::fs::write(path.join("staged-sdk"), b"private").unwrap();
        let failure = run(
            command,
            "{}",
            None,
            &|_, _, _| unreachable!(),
            cancellation,
            Some(&mut workspace),
            None,
        )
        .unwrap_err();
        assert!(failure.error.contains(expected), "{}", failure.error);
        drop(workspace);
        assert!(!path.exists());
        assert!(!dir.path().join("plan").exists());
    }
}

#[test]
#[cfg(target_os = "linux")]
fn an_sdk_exit_or_stop_finishes_even_while_a_descendant_keeps_both_pipes_open() {
    for case in ["exit", "stop", "check"] {
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().to_owned();
        let cancellation = Cancellation::default();
        let (done, result) = std::sync::mpsc::channel();
        let actor = path.clone();
        let active = cancellation.clone();
        let run_fixture = std::thread::spawn(move || {
            let mut command = std::process::Command::new("/bin/sh");
            command.args(["-ec", r#"
read -r plan
setsid /bin/sh -ec 'touch "$1/holding"; while [ ! -e "$1/release" ]; do sleep .02; done; printf done > "$1/finished"' child "$1" &
while [ ! -e "$1/holding" ]; do sleep .02; done
case "$2" in
  stop) while :; do sleep .02; done ;;
  check) echo '{"contract":2,"route":null,"mods":[]}' ;;
esac
exit 125
"#, "sdk"]).arg(&actor).arg(case);
            let mut workspace = Workspace::new(&actor).unwrap();
            let workspace_path = workspace.path().to_owned();
            let error = if case == "check" {
                ask(command, "native-check", &serde_json::json!({}), &active).unwrap_err()
            } else {
                run(
                    command,
                    "{}",
                    None,
                    &|_, _, _| unreachable!(),
                    &active,
                    Some(&mut workspace),
                    None,
                )
                .unwrap_err()
                .error
            };
            drop(workspace);
            done.send((error, workspace_path.exists())).unwrap();
        });
        let until = Instant::now() + Duration::from_secs(10);
        while !path.join("holding").exists() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        let inherited = path.join("holding").exists();
        if case == "stop" {
            cancellation.stop();
        }
        let outcome = result.recv_timeout(Duration::from_secs(8));
        let writing = !path.join("finished").exists();
        // Always release the test child, including if the old launcher behavior regresses.
        std::fs::write(path.join("release"), b"release").unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        while !path.join("finished").exists() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(inherited && writing && path.join("finished").exists());
        let (error, retained) = outcome.expect("the runner must finish without its pipe EOF");
        run_fixture.join().unwrap();
        match case {
            "stop" => {
                assert_eq!(error, super::super::CANCELLED);
                assert!(retained);
            }
            "exit" => {
                assert!(error.contains("output was still open"), "{error}");
                assert!(retained);
            }
            _ => assert!(error.contains("output was still open"), "{error}"),
        }
    }
}

#[test]
#[cfg(unix)]
fn the_runner_refuses_duplicate_terminal_events_and_wrong_or_extra_check_replies() {
    let dir = tempfile::tempdir().unwrap();
    let good = r#"{"contract":2,"route":null,"mods":[]}"#;
    let refusal = r#"{"contract":2,"error":{"kind":"unread","message":"fixture refused"}}"#;
    let ask_sdk = |lines: &[&str]| {
        ask(
            fake(dir.path(), lines),
            "native-check",
            &serde_json::json!({}),
            &Cancellation::default(),
        )
    };
    assert!(ask_sdk(&[good]).is_ok());
    assert!(ask_sdk(&[refusal, "EXIT1"])
        .unwrap_err()
        .contains("fixture refused"));
    for lines in [
        &[good, good][..],
        &[good, "EXIT1"],
        &[refusal],
        &[refusal, "EXIT2"],
        &[refusal, "EXIT3"],
        &[refusal, "SIGNAL"],
    ] {
        assert!(ask_sdk(lines)
            .unwrap_err()
            .contains("incomplete or inconsistent"));
    }
    assert!(ask_sdk(&["EXIT2"]).unwrap_err().contains("didn't accept"));
    assert!(ask_sdk(&["EXIT3"]).unwrap_err().contains("too old"));
    let built = r#"{"event":"built","route":"prebuilt","binary":"/b/game","libraries":[],"records":[],"mods":[],"internals":{},"mixins":"x","used":[]}"#;
    let failed = r#"{"event":"failed","error":"another ending","blame":null,"used":[]}"#;
    let result = run(
        fake(dir.path(), &[built, failed]),
        "{}",
        None,
        &|_, _, _| unreachable!(),
        &Cancellation::default(),
        None,
        None,
    )
    .unwrap_err();
    assert!(
        result.error.contains("incomplete or inconsistent"),
        "{}",
        result.error
    );
}

#[test]
#[cfg(unix)]
#[ignore = "daily"]
fn leader_success_or_blamed_failure_is_not_a_compiler_handoff() {
    let dir = tempfile::tempdir().unwrap();
    let built = r#"{"event":"built","route":"prebuilt","binary":"/b/game","libraries":[],"records":[],"mods":[],"internals":{},"mixins":"x","used":[]}"#;
    let failed = r#"{"event":"failed","error":"compile error","blame":{"mod":"greeter","kind":"compile"},"used":[]}"#;
    for (lines, success) in [(&[built][..], true), (&[failed, "EXIT1"], false)] {
        let terminal = || {
            run(
                fake(dir.path(), lines),
                "{}",
                None,
                &|_, _, _| unreachable!(),
                &Cancellation::default(),
                None,
                None,
            )
        };
        let unproved = compiler_handoff(terminal(), false).unwrap_err();
        assert!(unproved
            .error
            .contains("couldn't prove all native build workers stopped"));
        assert!(unproved.blame.is_none());
        assert_eq!(compiler_handoff(terminal(), true).is_ok(), success);
    }
}

#[test]
#[cfg(target_os = "linux")]
#[ignore = "daily; build the launcher binary first"]
fn actual_sdk_build_runner_keeps_a_surviving_attempt_private_and_retries() {
    use std::{fs, os::unix::fs::PermissionsExt};

    let _scratch = super::super::tests::scratch_state();
    let root = tempfile::tempdir().unwrap();
    let rust = Toolchain {
        dir: root.path().join("rust"),
        host: crate::rust_toolchain::host().unwrap(),
    };
    let write_tool = |name, script| {
        let path = root.path().join(name);
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    };
    let detached = write_tool(
        "stop-tool",
        r#"#!/bin/sh
set -eu
[ "$1" = native-build ]
read -r plan
setsid /bin/sh -ec '
  touch "$TMPDIR/holding"
  for _ in $(seq 400); do
    if [ -f "$TMPDIR/release" ]; then
      for dir in "$CARGO_HOME" "$CARGO_TARGET_DIR" "$TMPDIR"; do printf old > "$dir/proof"; done
      touch "$TMPDIR/finished"
      exit 0
    fi
    sleep .025
  done
  exit 1
' &
for _ in $(seq 400); do [ ! -f "$TMPDIR/holding" ] || break; sleep .025; done
[ -f "$TMPDIR/holding" ]
printf '%s\n' '{"event":"mod","id":"greeter","done":1,"total":1}'
wait
"#,
    );
    let succeeds = write_tool(
        "retry-tool",
        r#"#!/bin/sh
set -eu
[ "$1" = native-build ]
read -r plan
for dir in "$CARGO_HOME" "$CARGO_TARGET_DIR" "$TMPDIR"; do printf retry > "$dir/proof"; done
printf '%s\n' '{"event":"built","route":"prebuilt-api","binary":"unused","libraries":[],"records":[],"mods":[],"internals":{}}'
"#,
    );
    let revoke_tool = write_tool(
        "registry-tool",
        r#"#!/bin/sh
set -eu
read -r plan
printf '%s\n' '{"event":"consent","added_crates":1,"build_scripts":["syn"],"proc_macros":[]}'
read -r answer
printf '%s' "$answer" > "$TMPDIR/answer"
"#,
    );
    use crate::profile_txn::{Admission, Intent};
    let home = root.path().join("game");
    fs::create_dir(&home).unwrap();
    let mut profile = Admission::open(&home, None).unwrap();
    let stage = profile
        .reserve(Intent::Install, Default::default())
        .unwrap();
    let build_fixture = |program: &Path,
                         workspace: &mut Workspace,
                         cancellation: &Cancellation,
                         on_mod: &dyn Fn(&str, usize, usize),
                         consent: Option<&dyn Fn(&Consent) -> bool>| {
        let saved = workspace.path().to_owned();
        build(
            &rust,
            program,
            Plan {
                workspace,
                work: &saved.join("work"),
                package: Package {
                    runtime: &saved.join("runtime"),
                    api: &saved.join("api"),
                    engine: &saved.join("engine"),
                },
                mods: &saved.join("mods"),
                server: false,
                mixins_from_source: false,
            },
            consent,
            on_mod,
            cancellation,
            Some(stage.handoff()),
        )
    };
    let mut settings = crate::backend::Settings::default();
    settings.allow_native_mods().unwrap();
    let stop = Cancellation::default();
    stop.admit_native_work().unwrap();
    let mut first = Workspace::new(root.path()).unwrap();
    let old = first.path().to_owned();
    let error =
        build_fixture(&detached, &mut first, &stop, &|_, _, _| stop.stop(), None).unwrap_err();
    assert_eq!(error.error, super::super::CANCELLED);
    drop(first);
    assert!(old.join("tmp/holding").exists() && !old.join("tmp/finished").exists());

    let mut revoke = Workspace::new(root.path()).unwrap();
    let stale = Cancellation::default();
    stale.admit_native_work().unwrap();
    let settings = RefCell::new(settings);
    let failure = build_fixture(
        &revoke_tool,
        &mut revoke,
        &stale,
        &|_, _, _| unreachable!(),
        Some(&|_| {
            settings.borrow_mut().revoke_native_mods().unwrap();
            settings.borrow_mut().allow_native_mods().unwrap();
            true
        }),
    )
    .unwrap_err();
    assert!(
        failure.error.contains("permission changed"),
        "{}",
        failure.error
    );
    assert!(!revoke.path().join("tmp/answer").exists());

    let mut retry = Workspace::new(root.path()).unwrap();
    let fresh = Cancellation::default();
    fresh.admit_native_work().unwrap();
    let result = build_fixture(
        &succeeds,
        &mut retry,
        &fresh,
        &|_, _, _| unreachable!(),
        None,
    );
    assert_eq!(result.unwrap().route, CheckRoute::PrebuiltApi);
    assert!(
        !old.join("tmp/finished").exists(),
        "retry finished independently"
    );
    // The escaped SDK writer inherited the admitted profile through the real
    // Linux subreaper. Neither cancellation nor the launcher's last copy unlocks it.
    let stage_root = stage.path().to_owned();
    drop(stage);
    drop(profile);
    assert!(Admission::open(&home, None).is_err());
    fs::write(old.join("tmp/release"), b"release").unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    while !old.join("tmp/finished").exists() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(old.join("tmp/finished").is_file());
    for part in ["cargo", "target", "tmp"] {
        assert_eq!(fs::read(old.join(part).join("proof")).unwrap(), b"old");
        assert_eq!(
            fs::read(retry.path().join(part).join("proof")).unwrap(),
            b"retry"
        );
    }
    let until = Instant::now() + Duration::from_secs(10);
    while Admission::open(&home, None).is_err() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !stage_root.exists(),
        "only reclaim after the real staged writer exits"
    );
    let clean = retry.path().to_owned();
    drop(retry);
    assert!(!clean.exists());
}

#[test]
fn build_plan_and_process_use_the_same_private_cargo_home() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::new(dir.path()).unwrap();
    let path = workspace.path().to_owned();
    let rust = Toolchain {
        dir: path.join("rust"),
        host: crate::rust_toolchain::hosts().first().unwrap(),
    };
    let plan = Plan {
        work: &path.join("work"),
        workspace: &mut workspace,
        package: Package {
            runtime: &path,
            api: &path,
            engine: &path,
        },
        mods: &path.join("mods"),
        server: false,
        mixins_from_source: false,
    };
    let line: Value = serde_json::from_str(&plan_line(&rust, &plan)).unwrap();
    let mut command = std::process::Command::new("sdk-tool");
    workspace.isolate(&mut command).unwrap();
    let cargo_home = command
        .get_envs()
        .find(|(name, _)| *name == "CARGO_HOME")
        .unwrap()
        .1
        .unwrap();
    assert_eq!(
        line["toolchain"]["cargo_home"],
        serde_json::json!(Path::new(cargo_home))
    );
    assert_eq!(Path::new(cargo_home), path.join("cargo"));
    assert_eq!(line["work"], serde_json::json!(path.join("work")));
}

#[test]
fn only_the_last_error_lines_are_kept_read_lossily() {
    let kept = tail(b"first\n\xff second\r\n".as_slice());
    assert_eq!(kept, "first\n\u{fffd} second");
    let many: String = (0..2000)
        .map(|n| format!("line {n:04} {}\n", "y".repeat(40)))
        .collect();
    let kept = tail(many.as_bytes());
    assert!(kept.len() <= MAX_ERRORS && kept.ends_with(&format!("line 1999 {}", "y".repeat(40))));
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn a_tool_on_another_contract_says_to_update_the_launcher_and_logs_its_errors() {
    let _guard = super::super::tests::scratch_state();
    let dir = std::env::temp_dir().join(format!(
        "sunrise-native-tool-contract-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    super::super::clear_log().unwrap();
    let failed = run(
        fake(&dir, &["ERRORS", "EXIT3"]),
        "{}",
        None,
        &|_, _, _| {},
        &Cancellation::default(),
        None,
        None,
    )
    .unwrap_err();
    assert!(
        failed.error.contains("update the launcher"),
        "{}",
        failed.error
    );
    let log = std::fs::read_to_string(super::super::log_path()).unwrap();
    assert!(
        log.contains("warming up\n\u{fffd} unknown contract"),
        "{log}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn the_tool_hears_the_launchers_proxy_and_github_mirror() {
    use std::os::unix::fs::PermissionsExt;
    let _guard = super::super::tests::scratch_state();
    let dir = std::env::temp_dir().join(format!("sunrise-native-tool-env-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    let program = dir.join("tool");
    std::fs::write(&program, "#!/bin/sh\nprintf '%s|%s|%s' \"$HTTPS_PROXY\" \"$HTTP_PROXY\" \"$SUNRISE_GITHUB_MIRROR\"\n").unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    let rust = Toolchain {
        dir: dir.clone(),
        host: crate::rust_toolchain::hosts().first().unwrap(),
    };
    crate::proxy::announce(Some("http://127.0.0.1:9".into()));
    std::env::set_var("SUNRISE_GITHUB_MIRROR", "https://mirror.invalid");
    let output = command(&rust, &program).output();
    std::env::remove_var("SUNRISE_GITHUB_MIRROR");
    crate::proxy::announce(None);
    let seen = String::from_utf8(output.unwrap().stdout).unwrap();
    assert_eq!(
        seen,
        "http://127.0.0.1:9|http://127.0.0.1:9|https://mirror.invalid"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_tool_command_carries_the_proxy_and_mirror() {
    let _guard = super::super::tests::scratch_state();
    let rust = Toolchain {
        dir: std::env::temp_dir().join("sunrise-native-tool-command"),
        host: crate::rust_toolchain::hosts().first().unwrap(),
    };
    crate::proxy::announce(Some("socks5://h:2".into()));
    std::env::set_var("SUNRISE_GITHUB_MIRROR", "https://mirror.invalid");
    let built = command(&rust, Path::new("tool"));
    std::env::remove_var("SUNRISE_GITHUB_MIRROR");
    crate::proxy::announce(None);
    let env = |name: &str| {
        built
            .get_envs()
            .find(|(key, _)| *key == name)
            .and_then(|(_, value)| value)
            .map(|value| value.to_string_lossy().into_owned())
    };
    assert_eq!(env("HTTPS_PROXY").as_deref(), Some("socks5h://h:2"));
    assert_eq!(env("HTTP_PROXY").as_deref(), Some("socks5h://h:2"));
    assert_eq!(
        env("SUNRISE_GITHUB_MIRROR").as_deref(),
        Some("https://mirror.invalid")
    );
}

#[test]
fn a_check_reply_reads_its_verdicts_and_an_old_tool_or_an_error_says_so() {
    use super::super::fit::{Change, Verdict};
    let line = r#"{"contract":2,"route":"prebuilt-api","mods":[
        {"id":"here","refusal":null,"verdict":{"kind":"made-for"},"needs":null},
        {"id":"gone","refusal":null,"verdict":{"kind":"does-not-fit","made_for":"b","misfits":[
            {"id":"fn:give","change":"removed"},
            {"id":"type:T","change":{"lost-impl":"Clone"}},
            {"id":"hook:h","change":{"version":{"named":1,"now":2}}},
            {"id":"type:S","change":{"fields":{"gained":["x"],"at":"src/lib.rs:3"}}}]},"needs":null},
        {"id":"later","refusal":null,"verdict":null,"needs":"surface"},
        {"id":"refused","refusal":"it has a build script","verdict":null,"needs":null}]}"#
        .replace('\n', "");
    let checked: Checked = serde_json::from_value(reply(Some(&line), true).unwrap()).unwrap();
    assert_eq!(checked.mods[0].verdict, Some(Verdict::MadeFor));
    let Some(Verdict::DoesNotFit { misfits, .. }) = &checked.mods[1].verdict else {
        panic!("{:?}", checked.mods[1]);
    };
    let changes: Vec<&Change> = misfits.iter().map(|m| &m.change).collect();
    assert_eq!(
        changes,
        [
            &Change::Removed,
            &Change::LostImpl("Clone".into()),
            &Change::Version { named: 1, now: 2 },
            &Change::Fields {
                gained: vec!["x".into()],
                at: "src/lib.rs:3".into()
            }
        ]
    );
    assert_eq!(checked.mods[2].needs, Some(Need::Surface));
    assert!(reply(None, true).unwrap_err().contains("too old"));
    let error = r#"{"contract":2,"error":{"kind":"unread","message":"mods: not a folder"}}"#;
    let refused = reply(Some(error), false).unwrap_err();
    assert!(refused.ends_with(": mods: not a folder"), "{refused}");
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn a_check_by_a_tool_on_another_contract_or_from_before_the_command_says_which_to_update() {
    let _guard = super::super::tests::scratch_state();
    let dir =
        std::env::temp_dir().join(format!("sunrise-native-check-exit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let plan = serde_json::json!({"contract": CONTRACT});
    fake(&dir, &["ERRORS", "EXIT3"]);
    let newer = check(&dir.join("tool"), &plan, &Cancellation::default(), None).unwrap_err();
    assert!(
        matches!(&newer, CheckFailure::Refused(text) if text.contains("update the launcher")),
        "{newer:?}"
    );
    fake(&dir, &["ERRORS"]);
    let older = check(&dir.join("tool"), &plan, &Cancellation::default(), None).unwrap_err();
    assert!(
        matches!(&older, CheckFailure::Refused(text) if text.contains("install a newer version")),
        "{older:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn a_one_line_command_gets_its_name_and_a_contract_and_answers_one_line() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("sunrise-native-ask-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join("tool");
    std::fs::write(
        &program,
        "#!/bin/sh\nread plan\nprintf '%s\\n' \"$1\" \"$plan\" > \"$(dirname \"$0\")/asked\"\necho '{\"contract\":2,\"lock\":\"/m/Cargo.lock\"}'\n",
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    let reply = ask(
        std::process::Command::new(&program),
        "native-lock",
        &serde_json::json!({"mods": "/m", "update": true}),
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(reply["lock"], "/m/Cargo.lock");
    let asked = std::fs::read_to_string(dir.join("asked")).unwrap();
    let (command, plan) = asked.split_once('\n').unwrap();
    assert_eq!(command, "native-lock");
    let plan: Value = serde_json::from_str(plan.trim()).unwrap();
    assert_eq!(
        (plan["contract"].as_u64(), plan["update"].as_bool()),
        (Some(2), Some(true))
    );
    let _ = std::fs::remove_dir_all(&dir);
}
