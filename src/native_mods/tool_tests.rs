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
            "ERRORS" => script.push_str("printf 'warming up\\n\\377 unknown contract\\n' >&2\n"),
            "EXIT1" => script.push_str("exit 1\n"),
            "EXIT2" => script.push_str("exit 2\n"),
            "EXIT3" => script.push_str("exit 3\n"),
            "SIGNAL" => script.push_str("kill -TERM $$\n"),
            event => script.push_str(&format!("printf '%s\\n' '{}'\n", event.replace('\'', "'\\''"))),
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
fn a_tool_run_reports_its_mods_answers_its_crates_question_and_ends_built_or_blamed() {
    let dir = std::env::temp_dir().join(format!("sunrise-native-tool-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let started = RefCell::new(Vec::new());
    let on_mod = |id: &str, done: usize, total: usize| {
        started.borrow_mut().push(format!("{id} {done}/{total}"))
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

/// Captured from the published Dev SDK 7cfdf81463eb with an API-only `camera_blur`
/// package pinned to log 0.4.35. Only the absolute scratch path was replaced.
#[cfg(unix)]
#[test]
fn published_sdk_reply_names_the_archive_id_not_the_cargo_package() {
    use super::super::{registry_failure, ConflictRequirement, Failed, RegistryConflict};
    let dir = tempfile::tempdir().unwrap();
    let reply =
        include_str!("../../tests/fixtures/native-registry-sdk-7cfdf81463eb.jsonl").trim_end();
    let reported = run(
        fake(dir.path(), &[reply, "EXIT1"]),
        "{}",
        &|_, _, _| (),
        &Cancellation::default(),
        None,
        None,
    )
    .unwrap_err();
    assert!(reported.blame.is_none());
    let raw = reported.error.clone();
    let Failed::RegistryConflict { conflicts, error } =
        registry_failure::classify(reported.error, ["camera_blur", "loose_mod"].into_iter())
    else {
        panic!("expected an attributed SDK resolver conflict")
    };
    assert_eq!(error, raw);
    assert_eq!(
        conflicts,
        [RegistryConflict {
            mod_id: "camera_blur".into(),
            package: "log".into(),
            requirement: ConflictRequirement::Version("=0.4.35".into()),
            game: "0.4.34".into(),
        }]
    );
}

#[cfg(unix)]
#[test]
fn resolver_failure_uses_cargos_requiring_chains_not_the_mods_lock_versions() {
    use super::super::{registry_failure, ConflictRequirement, Failed, RegistryConflict};
    let dir = tempfile::tempdir().unwrap();
    let details = include_str!("../../tests/fixtures/native-registry-conflicts.txt");
    let reply = serde_json::json!({ "event": "failed", "error": details }).to_string();
    let mut reported = run(
        fake(dir.path(), &[&reply, "EXIT1"]),
        "{}",
        &|_, _, _| (),
        &Cancellation::default(),
        None,
        None,
    )
    .unwrap_err();
    assert!(reported.blame.is_none());
    assert_eq!(reported.error, details);
    // A path printed by the SDK must name a mod that was actually staged.
    let foreign = "mod_not_staged";
    reported
        .error
        .push_str(&details.replace("mod_motion_blur", foreign));
    let Failed::RegistryConflict { conflicts, error } = registry_failure::classify(
        reported.error,
        ["mod_motion_blur", "mod_lights", "loose_mod"].into_iter(),
    ) else {
        panic!("expected attributed conflicts")
    };
    assert!(error.contains(foreign));
    assert_eq!(
        conflicts,
        [
            RegistryConflict {
                mod_id: "mod_motion_blur".into(),
                package: "syn".into(),
                requirement: ConflictRequirement::Version("=3.0.10".into()),
                game: "3.0.6".into(),
            },
            RegistryConflict {
                mod_id: "mod_lights".into(),
                package: "log".into(),
                requirement: ConflictRequirement::Version("=0.4.35".into()),
                game: "0.4.34".into(),
            },
        ]
    );
}

#[test]
fn feature_conflicts_use_the_requiring_mod_even_when_its_version_requirement_fits() {
    use super::super::{registry_failure, ConflictRequirement, Failed};
    let error = r#"Mods in Cargo's dependency report: mod_motion_blur.

syn: the game locks 3.0.6, but resolution without the game's constraint selected 3.0.10.

error: failed to select a version for `syn`.
    ... required by package `mod_16 v0.1.1 (C:\temp\build\native-api\mod_motion_blur)`
versions that meet the requirements `^3` are: 3.0.10, 3.0.6

package `mod_16` depends on `syn` with feature `extra` but `syn` does not have that feature.

all possible versions conflict with previously selected packages

  previously selected package `syn v3.0.6`
    ... which satisfies dependency `syn = "^3"` of package `mod_17 v0.1.1 (C:\temp\build\native-api\unrelated)`
"#;
    let Failed::RegistryConflict { conflicts, .. } =
        registry_failure::classify(error.into(), ["mod_motion_blur", "unrelated"].into_iter())
    else {
        panic!("expected one feature conflict")
    };
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].mod_id, "mod_motion_blur");
    assert_eq!(
        conflicts[0].requirement,
        ConflictRequirement::Feature("extra".into())
    );
}

#[test]
fn original_lock_comparison_and_network_failure_keep_distinct_actions() {
    use super::super::{registry_failure, Failed};
    for (error, expected) in [
        ("syn: game locks [2.0.119, 3.0.6], mod locks 3.0.10; the game graph must not move", false),
        ("Downloading the mods' pinned crates failed (exit status: 101).\nerror: failed to download any data for `syn v3.0.6` within 30s", true),
        ("error: failed to verify the checksum of downloaded crate `syn v3.0.6`", false),
        ("error: failed to get successful HTTP response from `https://example.invalid`, got 404", false),
        ("error: failed to get successful HTTP response from `https://example.invalid`, got 503", true),
    ] {
        assert!(matches!(registry_failure::classify(error.into(), ["mod_motion_blur"].into_iter()),
            Failed::Tool { retryable, error: original } if retryable == expected && original == error));
    }
}

#[test]
#[cfg(unix)]
#[ignore = "daily"]
fn not_starting_a_worker_does_not_retain_its_staged_directory() {
    let dir = tempfile::tempdir().unwrap();
    let _scratch = super::super::tests::scratch_state();
    let stopped = Cancellation::default();
    stopped.stop();
    for (command, cancellation, expected) in [
        (
            std::process::Command::new(dir.path().join("missing-program")),
            &Cancellation::default(),
            "couldn't run",
        ),
        (fake(dir.path(), &[]), &stopped, super::super::CANCELLED),
    ] {
        let mut workspace = Workspace::new(dir.path()).unwrap();
        let path = workspace.path().to_owned();
        std::fs::write(path.join("staged-sdk"), b"private").unwrap();
        let failure = run(
            command,
            "{}",
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
                         on_mod: &dyn Fn(&str, usize, usize)| {
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
            on_mod,
            cancellation,
            Some(stage.handoff()),
        )
    };
    let stop = Cancellation::default();
    let mut first = Workspace::new(root.path()).unwrap();
    let old = first.path().to_owned();
    let error = build_fixture(&detached, &mut first, &stop, &|_, _, _| stop.stop()).unwrap_err();
    assert_eq!(error.error, super::super::CANCELLED);
    drop(first);
    assert!(old.join("tmp/holding").exists() && !old.join("tmp/finished").exists());

    let mut retry = Workspace::new(root.path()).unwrap();
    let fresh = Cancellation::default();
    let result = build_fixture(&succeeds, &mut retry, &fresh, &|_, _, _| unreachable!());
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

/// A Cancel that wins before a piped preparer is registered ends it through its job alone: what
/// the leader left running in the job ends, and no `taskkill` runs, so nothing waits on one
/// (review of #670). Only Windows CI runs this.
#[cfg(windows)]
#[test]
fn a_stop_before_a_preparers_registration_ends_its_job_without_taskkill() {
    let lock = tempfile::tempfile().unwrap();
    let mut command = std::process::Command::new("cmd");
    command.args(["/c", "start", "", "/b", "ping", "-n", "30", "127.0.0.1"]);
    let mut child = crate::process::leases::spawn_preparer(command, &lock, None, true).unwrap();
    let job = child
        .preparer_job()
        .expect("a piped preparer runs in a job of its own");
    let pid = child.id();
    let _ = child.wait();
    let active = || job.accounting().unwrap().ActiveProcesses;
    let waiting = Instant::now();
    while active() == 0 && waiting.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(active() >= 1, "the leader left ping running in its job");
    let stop = Cancellation::default();
    stop.stop();
    let kills = KILLS.with(std::cell::Cell::get);
    let result = stop.track_preparer(pid, Some(Arc::clone(&job)));
    assert_eq!(result.err().as_deref(), Some(crate::native_mods::CANCELLED));
    assert_eq!(KILLS.with(std::cell::Cell::get), kills, "a taskkill ran");
    let waiting = Instant::now();
    while active() != 0 && waiting.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(active(), 0, "the job ended, with what the leader left");
}
