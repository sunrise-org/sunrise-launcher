//! Real launched images (including the copied Windows runtime); no account or signed game.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use sunrise_launcher::{
    backend,
    launch::Launch,
    managed_play::{FirstBuild, Profile, StartError},
    native_mods::{Cancellation, Profile as BuildMode},
};

const ROLE: &str = "SUNRISE_MANAGED_PLAY_PROCESS_TEST";
const MODE: &str = "SUNRISE_MANAGED_PLAY_PROCESS_MODE";
const BUILD: &str = "v0.7.1";
const FINAL_STDOUT: &str = "game stdout just before exit 23";
const FINAL_STDERR: &str = "game stderr just before exit 23";

fn main() {
    let mut args = std::env::args_os().skip(1);
    if let Some(mode) = args.next() {
        match mode.to_str() {
            Some("--launch-api") => {
                println!(r#"{{"options":[{{"name":"--developer-single-player"}}]}}"#);
                return;
            }
            Some("--fixture-wrapper") => return wrapper(args),
            Some("--fixture-late") => return late(args),
            Some("--fixture-wrapper-linger") => {
                let home = backend::game_home();
                fs::write(home.join("linger-live"), []).unwrap();
                await_event(|| home.join("linger-stop").is_file());
                fs::write(home.join("linger-stopped"), []).unwrap();
                return;
            }
            Some("--internal-game-relay") if std::env::var_os("FIXTURE_RELAY_CUT").is_some() => {
                return post_commit_fixture(args);
            }
            _ => {
                if let Some(status) = sunrise_launcher::process::relay::private(&mode, args) {
                    std::process::exit(status);
                }
            }
        }
    }
    match std::env::var(ROLE).as_deref() {
        Ok("owner") => owner(),
        Ok("contender") => contender(),
        Ok("game") => game(),
        Ok(role) => panic!("unexpected fixture role {role}"),
        Err(_) => {
            scenario("direct");
            scenario("wrapped");
            scenario("different-status");
            scenario("early-wrapper");
            scenario("failed");
            registry::ordinary_install();
            #[cfg(windows)]
            ordinary_cli();
        }
    }
}

#[path = "managed_play_process/registry.rs"]
mod registry;

fn scenario(mode: &str) {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let home = root_path.join("game");
    fs::create_dir(&home).unwrap();
    println!("checking {mode} managed Play");
    let process = |role| {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.env(ROLE, role)
            .env(MODE, mode)
            .env("SUNRISE_HOME", &home)
            .env("SUNRISE_LAUNCHER_HOME", root_path.join("launcher"))
            .env("XDG_CONFIG_HOME", root_path.join("config"))
            .stdin(Stdio::null());
        cmd
    };
    if matches!(mode, "different-status" | "early-wrapper") {
        assert!(process("owner").status().unwrap().success());
        await_event(|| home.join("linger-stopped").is_file());
        println!("{mode} reported the game's actual status without waiting for unrelated wrapper descendants");
        return;
    }
    if mode == "failed" {
        assert!(process("owner").status().unwrap().success());
        assert!(home.join("late-refused").is_file());
        assert!(!home.join("game-ready").exists());
        return;
    }
    let contender = Guard(process("contender").spawn().unwrap());
    await_event(|| home.join("contender-ready").is_file());
    let mut owner = Guard(process("owner").spawn().unwrap());
    await_event(|| {
        if let Some(status) = owner.0.try_wait().unwrap() {
            panic!("{mode} owner exited before the game was ready: {status}");
        }
        home.join("game-ready").is_file() && home.join("game-spawned").is_file()
    });
    owner.0.kill().unwrap();
    assert!(!owner.0.wait().unwrap().success());
    fs::write(home.join("try-while-live"), []).unwrap();
    await_event(|| home.join("still-locked").is_file());
    fs::write(home.join("game-stop"), []).unwrap();
    await_event(|| home.join("unlocked").is_file());
    let mut contender = contender;
    assert!(contender.0.wait().unwrap().success());
    if mode == "wrapped" {
        assert!(home.join("wrapper-ran").is_file());
    }
    println!("{mode} managed Play retained cache and profile claims after launcher death");
}

/// Unlike the library scenarios, this enters through the shipped `play` command and its
/// offline capability check, pin, jar, hook and attached exit-status handling.
#[cfg(windows)]
fn ordinary_cli() {
    use serde_json::json;
    const TAG: &str = "developer-local";
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let home = root_path.join("game");
    let data = root_path.join("launcher");
    let build = data.join("versions").join(TAG);
    let resources = build.join(".local/sunrise-client/26.3");
    fs::create_dir(&home).unwrap();
    fs::create_dir_all(&resources).unwrap();
    fs::write(resources.join("sunrise-resources.json"), "{}").unwrap();
    fs::write(build.join(".installed"), []).unwrap();
    fs::write(
        build.join("developer.json"),
        json!({"revision":"a".repeat(40)}).to_string(),
    )
    .unwrap();
    fs::write(
        build.join("research-lock.json"),
        r#"{"minecraft":{"version":"26.3"}}"#,
    )
    .unwrap();
    fs::copy(
        std::env::current_exe().unwrap(),
        build.join("sunrise-client.exe"),
    )
    .unwrap();
    fs::write(
        data.join("instances.json"),
        json!({"instances": {"sunrise": {
            "version": TAG,
            "launch": {
                "wrapper": "sunrise-client.exe --fixture-wrapper different-status",
                "pre_launch": "echo ready>fixture-prelaunch",
                "env": {
                    "PATH": build,
                    "SUNRISE_SESSION_FILE": "not-the-selected-session",
                    "SUNRISE_LAUNCHER_EXE": "not-the-launcher",
                    "SUNRISE_LAUNCHER_DEVELOPER_OFFLINE": "0",
                },
            },
        }}})
        .to_string(),
    )
    .unwrap();
    let mut launcher = Guard(
        Command::new(env!("CARGO_BIN_EXE_sunrise-launcher"))
            .arg("play")
            .env(ROLE, "game")
            .env(MODE, "ordinary-cli")
            .env("SUNRISE_HOME", &home)
            .env("SUNRISE_LAUNCHER_HOME", &data)
            .env("SUNRISE_LAUNCHER_DEVELOPER_OFFLINE", "1")
            .env("SUNRISE_SESSION_FILE", "inherited-stale-session")
            .env("SUNRISE_NO_KEYCHAIN", "1")
            .env("SUNRISE_TELEMETRY_URL", "off")
            .stdin(Stdio::null())
            .spawn()
            .unwrap(),
    );
    await_event(|| {
        assert!(
            launcher.0.try_wait().unwrap().is_none(),
            "CLI exited before Play"
        );
        home.join("game-ready").is_file() && home.join("linger-live").is_file()
    });
    assert!(home.join("mod-browser/active-build.json").is_file());
    assert_eq!(
        fs::read_to_string(home.join("fixture-prelaunch"))
            .unwrap()
            .trim(),
        "ready"
    );
    fs::write(home.join("game-stop"), []).unwrap();
    let mut status = None;
    await_event(|| {
        status = launcher.0.try_wait().unwrap();
        status.is_some()
    });
    assert!(!home.join("linger-stopped").exists());
    fs::write(home.join("linger-stop"), []).unwrap();
    await_event(|| home.join("linger-stopped").is_file());
    assert_eq!(status.unwrap().code(), Some(23));
    println!("Windows ordinary CLI Play used its exact copied game and returned its status, not the wrapper's");
}

fn owner() {
    let mode = std::env::var(MODE).unwrap();
    let phase = |step| eprintln!("{mode} owner: {step}");
    let home = backend::game_home();
    phase("copying game image");
    let executable = backend::version_dir(BUILD).join(if cfg!(windows) {
        "sunrise-client.exe"
    } else {
        "sunrise-client"
    });
    let resources = backend::resources(BUILD);
    fs::create_dir_all(&resources).unwrap();
    fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
    fs::write(executable.with_file_name(".installed"), []).unwrap();
    fs::write(resources.join("sunrise-resources.json"), b"{}").unwrap();
    phase("opening profile and binding game");
    let mut profile = Profile::open(&home, None).unwrap();
    let cancellation = Cancellation::default();
    let first = profile
        .begin_first(|| FirstBuild::installed(BUILD), |_| {})
        .unwrap();
    phase("proving bound image");
    let image = first
        .exact_image(BuildMode::Optimized, &cancellation, |_| {}, None)
        .unwrap();
    phase("committing profile");
    let active = first.commit(&image, &cancellation).unwrap();
    let command = || {
        let mut cmd = Command::new(&executable);
        cmd.current_dir(&home)
            .env(ROLE, "game")
            .env("SUNRISE_RUNTIME_ENTRY", "fake record")
            .env("SUNRISE_RUNTIME_HOLDER", "fake holder")
            .arg("--resources")
            .arg(active.resource_path());
        cmd
    };
    let outside = home.with_file_name("other");
    fs::create_dir(&outside).unwrap();
    phase("rejecting invalid command arguments");
    for flag in [
        "--home",
        "--gameDir",
        "--dev-mod",
        "--resources",
        "--dedicated",
    ] {
        let mut wrong = command();
        wrong.arg(flag).arg(&outside);
        assert!(
            profile
                .spawn_direct(&active, &cancellation, wrong, false)
                .is_err(),
            "accepted {flag}"
        );
        assert!(!home.join("game-ready").exists());
    }
    let wrap = |case| {
        #[cfg(unix)]
        let program = std::env::current_exe().unwrap().into_os_string();
        #[cfg(windows)]
        let program = executable.file_name().unwrap().to_owned();
        let mut launch = Launch {
            wrapper: format!(
                "\"{}\" --fixture-wrapper {case}",
                program
                    .to_string_lossy()
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
            ),
            ..Launch::default()
        };
        // Windows also proves that a bare wrapper is found on its configured PATH.
        if cfg!(windows) {
            launch.env.insert(
                "PATH".into(),
                executable.parent().unwrap().display().to_string(),
            );
        }
        launch
    };
    if matches!(mode.as_str(), "different-status" | "early-wrapper") {
        let (mut child, runtime) = profile
            .spawn_configured(
                &active,
                &cancellation,
                command(),
                &wrap(mode.as_str()),
                true,
            )
            .unwrap();
        await_event(|| home.join("game-ready").is_file() && home.join("linger-live").is_file());
        if mode == "early-wrapper" {
            await_event(|| home.join("wrapper-finished").is_file());
            assert!(
                child.try_wait().unwrap().is_none(),
                "the live game was confused with its wrapper"
            );
        }
        drop(profile);
        let log = backend::Logged::monitor_game(child, runtime, Default::default());
        assert!(log.finished().is_none());
        fs::write(home.join("game-stop"), []).unwrap();
        let mut result = None;
        let deadline = Instant::now() + Duration::from_secs(10);
        while result.is_none() {
            result = log.finished();
            assert!(
                Instant::now() < deadline,
                "the launcher waited for wrapper diagnostics or another Job member"
            );
            std::thread::sleep(Duration::from_millis(15));
        }
        let status = result.unwrap().unwrap_err();
        assert!(status.contains("23") && !status.contains("43"), "{status}");
        let tail = log.tail(20);
        assert!(tail.iter().any(|line| line == FINAL_STDOUT), "{tail:?}");
        assert!(tail.iter().any(|line| line == FINAL_STDERR), "{tail:?}");
        assert!(!home.join("linger-stopped").exists());
        fs::write(home.join("linger-stop"), []).unwrap();
        return;
    }
    if mode == "failed" {
        for case in ["none", "stall", "late"] {
            let result =
                profile.spawn_configured(&active, &cancellation, command(), &wrap(case), false);
            let error = match result {
                Err(error) => error,
                Ok(_) => panic!("wrapper {case} started a game without the relay"),
            };
            let error = String::from(error);
            assert!(
                error.contains("Could not start Sunrise through"),
                "wrapper {case}: {error}"
            );
            assert!(!active.resource_path().join(".last-used").exists());
            if case == "stall" {
                assert!(home.join("stall-ready").is_file());
                fs::write(home.join("stall-stop"), []).unwrap();
            }
            assert!(!home.join("game-ready").exists());
        }
        assert!(home.join("late-waiting").is_file());
        for case in ["post-commit-death", "post-commit-refusal"] {
            let result =
                profile.spawn_configured(&active, &cancellation, command(), &wrap(case), false);
            let error = match result {
                Err(error) => error,
                Ok(_) => panic!("wrapper {case} did not send a successful spawn proof"),
            };
            assert!(home.join(case).is_file(), "fixture did not pass Commit");
            assert_eq!(
                matches!(error, StartError::UnknownSpawn(_)),
                case == "post-commit-death"
            );
            let message = String::from(error);
            assert!(
                message.contains(if case == "post-commit-death" {
                    "may have started"
                } else {
                    "fixture refused child creation"
                }),
                "{message}"
            );
            assert!(!home.join("game-ready").exists());
        }
        drop(profile);
        // The independently started late relay is still alive. Revocation must release
        // both claims before it is allowed to try its formerly valid invocation.
        drop(Profile::open(&home, None).unwrap());
        assert!(can_lock_build(&backend::version_dir(BUILD)));
        fs::write(home.join("try-late"), []).unwrap();
        await_event(|| home.join("late-refused").is_file());
        println!("wrapper exit revoked an unused token; its late relay could not start a game");
        return;
    }
    phase("launching game");
    let (_child, _runtime) = if mode == "wrapped" {
        use std::io::Read as _;
        let (mut child, runtime) = profile
            .spawn_configured(&active, &cancellation, command(), &wrap("close"), true)
            .unwrap();
        for (pipe, letter) in [(&mut child.stdout, b'o'), (&mut child.stderr, b'e')] {
            let mut prefix = [0; 4096];
            pipe.as_mut().unwrap().read_exact(&mut prefix).unwrap();
            assert_eq!(prefix, [letter; 4096]);
        }
        (child, runtime)
    } else {
        profile
            .spawn_direct(&active, &cancellation, command(), false)
            .unwrap()
    };
    phase("game spawned");
    drop(profile);
    fs::write(home.join("game-spawned"), []).unwrap();
    await_event(|| false);
}

fn contender() {
    let home = backend::game_home();
    fs::write(home.join("contender-ready"), []).unwrap();
    await_event(|| home.join("try-while-live").is_file());
    assert!(Profile::open(&home, None).is_err());
    // Test a fresh runtime claim, not a higher-level check of the active pointer.
    assert!(!can_lock_build(&backend::version_dir(BUILD)));
    fs::write(home.join("still-locked"), []).unwrap();
    await_event(|| Profile::open(&home, None).is_ok());
    await_event(|| can_lock_build(&backend::version_dir(BUILD)));
    fs::write(home.join("unlocked"), []).unwrap();
}

fn can_lock_build(path: &Path) -> bool {
    use sunrise_launcher_smod::{
        locks::LockMode,
        runtime_leases::{open_directory, Registry},
    };
    let dir = open_directory(path).unwrap();
    let registry = Registry::existing(&backend::data_dir()).unwrap();
    registry.lock(&dir, LockMode::Exclusive).is_ok()
}

fn wrapper(mut args: impl Iterator<Item = std::ffi::OsString>) {
    let mode = args.next().unwrap();
    let home = backend::game_home();
    assert!(std::env::var_os("SUNRISE_RUNTIME_LEASE_FDS").is_none());
    assert!(std::env::var_os("SUNRISE_RUNTIME_LEASE_HANDLES").is_none());
    // The actual wrapper may close everything except stdio and still invoke its program.
    #[cfg(unix)]
    for fd in 3..1024 {
        // SAFETY: this private fixture is single-threaded and doesn't own any live Rust IO
        // objects in the inherited descriptor range; no claim numbers came from the parent.
        unsafe {
            libc::close(fd);
        }
    }
    if mode == "none" {
        return;
    }
    if mode == "stall" {
        fs::write(home.join("stall-ready"), []).unwrap();
        await_event(|| home.join("stall-stop").is_file());
        return;
    }
    if mode == "late" {
        std::mem::forget(
            Command::new(std::env::current_exe().unwrap())
                .arg("--fixture-late")
                .args(args)
                .spawn()
                .unwrap(),
        );
        await_event(|| home.join("late-waiting").is_file());
        return;
    }
    if mode
        .to_str()
        .is_some_and(|mode| mode.starts_with("post-commit-"))
    {
        assert!(Command::new(args.next().unwrap())
            .args(args)
            .env("FIXTURE_RELAY_CUT", mode)
            .status()
            .unwrap()
            .success());
        return;
    }
    if mode == "different-status" || mode == "early-wrapper" {
        let _linger = Guard(
            Command::new(std::env::current_exe().unwrap())
                .arg("--fixture-wrapper-linger")
                .spawn()
                .unwrap(),
        );
        let mut relay = Command::new(args.next().unwrap())
            .args(args)
            .spawn()
            .unwrap();
        await_event(|| home.join("game-ready").is_file());
        if mode == "different-status" {
            assert_eq!(relay.wait().unwrap().code(), Some(23));
        }
        fs::write(home.join("wrapper-finished"), []).unwrap();
        std::process::exit(43);
    }
    assert_eq!(mode, "close");
    use std::io::Write as _;
    std::io::stdout().write_all(&vec![b'o'; 256 << 10]).unwrap();
    std::io::stderr().write_all(&vec![b'e'; 256 << 10]).unwrap();
    fs::write(home.join("wrapper-ran"), []).unwrap();
    let status = Command::new(args.next().unwrap())
        .args(args)
        .env("SUNRISE_HOME", home.with_file_name("wrong-home"))
        .env("SUNRISE_MANAGED_WRAPPER_OVERLAY", "kept")
        .status()
        .unwrap();
    std::process::exit(status.code().unwrap_or(1));
}

/// A peer of the same installed test image and Job, acquiring the real claims before
/// exiting without an ack or explicitly refusing to create its child.
fn post_commit_fixture(mut args: impl Iterator<Item = std::ffi::OsString>) {
    use serde_json::{json, Value};
    use sunrise_launcher_smod::local_control::{self, Channel, Marker};
    let command = args.next().unwrap();
    let invocation: Value = serde_json::from_str(command.to_str().unwrap()).unwrap();
    let marker: Marker = serde_json::from_value(invocation["marker"].clone()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    #[cfg(unix)]
    let stream = local_control::connect(&marker, deadline).unwrap();
    #[cfg(windows)]
    let stream = local_control::connect_relay(
        &marker,
        invocation["issuer"].as_u64().unwrap() as u32,
        deadline,
    )
    .unwrap();
    let channel = Channel::authenticate_relay(stream, &marker, false, deadline, None).unwrap();
    let (channel, offer) = channel.receive::<Value>().unwrap();
    let count = offer["count"].as_u64().unwrap() as usize;
    #[cfg(unix)]
    let (channel, claims) = channel.receive_relay_claims(count).unwrap();
    #[cfg(windows)]
    let (channel, claims) = channel.receive::<Vec<usize>>().unwrap();
    assert_eq!(claims.len(), count);
    let (channel, permit) = channel
        .send(&"Acquired")
        .unwrap()
        .receive::<String>()
        .unwrap();
    assert_eq!(permit, "Commit");
    let case = std::env::var("FIXTURE_RELAY_CUT").unwrap();
    fs::write(backend::game_home().join(&case), []).unwrap();
    if case == "post-commit-refusal" {
        channel
            .relay_committed(deadline)
            .send(&json!({"Refused":"fixture refused child creation"}))
            .unwrap();
    }
    // On Windows the process exit closes the duplicated handles, just as a dying relay does.
}

fn late(mut args: impl Iterator<Item = std::ffi::OsString>) {
    let home = backend::game_home();
    fs::write(home.join("late-waiting"), []).unwrap();
    await_event(|| home.join("try-late").is_file());
    let result = Command::new(args.next().unwrap())
        .args(args)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(78));
    assert!(String::from_utf8_lossy(&result.stderr).contains("game launch is no longer active"));
    fs::write(home.join("late-refused"), []).unwrap();
}

fn game() {
    let home = backend::game_home();
    let cmd: Vec<_> = std::env::args_os().collect();
    assert_eq!(cmd[1], "--resources");
    let resources = PathBuf::from(&cmd[2]);
    assert_eq!(
        fs::read(resources.join("sunrise-resources.json")).unwrap(),
        b"{}"
    );
    assert_ne!(
        std::env::var("SUNRISE_RUNTIME_ENTRY").ok().as_deref(),
        Some("fake record")
    );
    assert!(std::env::var_os("SUNRISE_RUNTIME_HOLDER").is_none());
    assert_eq!(std::env::var("SUNRISE_NO_DATA_FILE").unwrap(), "1");
    if std::env::var(MODE).as_deref() == Ok("wrapped") {
        assert_eq!(
            std::env::var("SUNRISE_MANAGED_WRAPPER_OVERLAY").unwrap(),
            "kept"
        );
    }
    assert_eq!(
        std::env::current_dir().unwrap().canonicalize().unwrap(),
        home.canonicalize().unwrap()
    );
    #[cfg(windows)]
    {
        assert!(std::env::current_exe()
            .unwrap()
            .canonicalize()
            .unwrap()
            .starts_with(backend::data_dir().canonicalize().unwrap().join("run")));
        if std::env::var(MODE).as_deref() == Ok("ordinary-cli") {
            assert!(cmd.iter().any(|arg| arg == "--developer-single-player"));
            assert!(std::env::var_os("SUNRISE_SESSION_FILE").is_none());
            assert!(std::env::var_os("SUNRISE_LAUNCHER_EXE").is_none());
            assert!(std::env::var("SUNRISE_LAUNCHER_DEVELOPER_OFFLINE").is_ok_and(|v| v == "1"));
        }
    }
    fs::write(home.join("game-ready"), []).unwrap();
    await_event(|| home.join("game-stop").is_file());
    if matches!(
        std::env::var(MODE).as_deref(),
        Ok("different-status" | "early-wrapper" | "ordinary-cli" | "registry-install")
    ) {
        use std::io::Write as _;
        println!("{FINAL_STDOUT}");
        eprintln!("{FINAL_STDERR}");
        std::io::stdout().flush().unwrap();
        std::process::exit(23);
    }
}

#[track_caller]
fn await_event(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "managed Play process fixture timed out"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct Guard(Child);
impl Drop for Guard {
    fn drop(&mut self) {
        if self.0.try_wait().ok() == Some(None) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
