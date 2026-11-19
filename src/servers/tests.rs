use super::*;
use std::io::{BufRead, BufReader, Write};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sunrise-servers-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn folder_names_are_plain() {
    assert_eq!(slug("Ludo's Survival!"), "ludo-s-survival");
    assert_eq!(slug("  "), "server");
    assert_eq!(slug("Été 2026"), "t-2026");
    assert_eq!(service_suffix(Path::new("/x/My World")), "my-world");
}

#[cfg(unix)]
#[test]
fn unregister_keeps_running_servers_and_never_moves_worlds() {
    const CHILD: &str = "SUNRISE_TEST_UNREGISTER";
    if std::env::var_os(CHILD).is_none() {
        let root = tempfile::tempdir().unwrap();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "servers::tests::unregister_keeps_running_servers_and_never_moves_worlds"
            ])
            .env(CHILD, "1")
            .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
            .env("SUNRISE_HOME", root.path().join("game"))
            .status()
            .unwrap()
            .success());
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        let dir = root.path().join(name);
        fs::create_dir_all(dir.join(".sunrise")).unwrap();
        fs::write(dir.join("level.dat"), name).unwrap();
        Entry {
            name: name.to_owned(),
            dir,
            host: None,
        }
    };
    let stopped = make("Stopped world");
    let running = make("Running world");
    let active = fs::File::create(running.dir.join(".sunrise/lock")).unwrap();
    active.try_lock().unwrap();
    let current = [stopped.clone(), running.clone()];
    let list = root.path().join("servers.json");
    save_entries(&list, &current).unwrap();
    let report = forget_reviewed_in(&list, &current).unwrap();
    assert_eq!(report.removed, 1);
    assert_eq!(report.refused.len(), 1);
    assert_eq!(report.refused[0].0, running);
    assert_eq!(
        serde_json::from_slice::<Vec<Entry>>(&fs::read(list).unwrap()).unwrap(),
        [running]
    );
    assert_eq!(
        fs::read_to_string(stopped.dir.join("level.dat")).unwrap(),
        stopped.name
    );
}

#[test]
fn create_asks_init_and_config_set() {
    let spec = NewServer {
        name: "Friends".into(),
        world: World::New {
            seed: " 12345 ".into(),
        },
        players: vec!["Alex".into(), "Steve".into()],
        eula: true,
        check_accounts: false,
        ..NewServer::default()
    };
    assert_eq!(
        init_args(&spec, "world", 25566).join(" "),
        "init --yes --eula --world world --seed 12345 --port 25566 --whitelist --offline \
         --player Alex --player Steve --op Alex"
    );
    let open = NewServer {
        friends_only: false,
        world: World::Copy("/saves/Old".into()),
        ..NewServer::default()
    };
    assert_eq!(
        init_args(&open, "old", 25565).join(" "),
        "init --yes --world old --port 25565 --no-whitelist"
    );
    assert_eq!(
        settings_args(&spec, 8131).join(" "),
        "config set world.game_mode=survival world.difficulty=normal server.max_players=10 \
         server.motd=Friends panel.bind=127.0.0.1:8131"
    );
}

#[cfg(unix)]
#[test]
fn programs_are_laid_out_as_install_sh_does() {
    let root = scratch("runtime");
    let version = root.join("versions/v1");
    fs::create_dir_all(version.join(".local/sunrise-client/26.3")).unwrap();
    fs::write(version.join(PROGRAM), "server").unwrap();
    fs::write(version.join("sunrise-client"), "game").unwrap();
    fs::write(version.join(".installed"), "").unwrap();
    crate::game_app::ensure(&version).unwrap();
    fs::write(
        version.join(".local/sunrise-client/26.3/sunrise-resources.json"),
        "{}",
    )
    .unwrap();
    let rt = root.join("runtime");
    set_up_in(&rt, &version, "v1", "26.3").unwrap();
    assert_eq!(
        fs::read_to_string(rt.join("current").join(PROGRAM)).unwrap(),
        "server"
    );
    assert!(!rt.join("releases/v1/.installed").exists());
    // The game's macOS bundle is the launcher's, not the server's.
    assert!(!rt.join("releases/v1/Sunrise.app").exists());
    // The release finds the game files where sunrise-server looks: .local beside it.
    assert!(rt
        .join("current/.local/sunrise-client/26.3/sunrise-resources.json")
        .is_file());
    // Again with newer game files: only they change, swapped in whole.
    fs::write(
        version.join(".local/sunrise-client/26.3/sunrise-resources.json"),
        "{\"new\":1}",
    )
    .unwrap();
    set_up_in(&rt, &version, "v1", "26.3").unwrap();
    assert_eq!(
        fs::read_to_string(game_files(&rt, "26.3").join("sunrise-resources.json")).unwrap(),
        "{\"new\":1}"
    );
    assert!(!game_files(&rt, "26.3").with_extension("old").exists());
    fs::remove_dir_all(&root).unwrap();
}

#[cfg(unix)]
#[test]
fn start_and_restart_publish_the_configured_resources_from_the_engine_command() {
    use std::os::unix::fs::PermissionsExt;
    use sunrise_launcher_smod::{
        locks::LockMode,
        runtime_leases::{open_directory, Registry},
    };
    const CHILD: &str = "SUNRISE_TEST_HOST_RUNTIME";
    if std::env::var_os(CHILD).is_none() {
        let root = tempfile::tempdir().unwrap();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "servers::tests::start_and_restart_publish_the_configured_resources_from_the_engine_command"
            ])
            .env(CHILD, "1")
            .env("SUNRISE_LAUNCHER_HOME", root.path().join("data"))
            .env("SUNRISE_HOME", root.path().join("game"))
            .status()
            .unwrap()
            .success());
        return;
    }
    let root = runtime_root();
    let release = root.join("releases/nightly");
    let data = backend::data_dir();
    let resources = data.join("configured game files");
    assert!(!game_files(&root, "26.3").exists());
    let server = data.join("servers/friends");
    for path in [&release, &resources, &server] {
        fs::create_dir_all(path).unwrap();
    }
    point(&root.join("current"), &release).unwrap();
    let program = release.join(PROGRAM);
    let reply = serde_json::json!({"format":1, "program":"engine", "args":["--resources", resources], "cwd":server, "resources":resources});
    fs::write(
        &program,
        format!("#!/bin/sh\n[ \"$3\" = engine-command ] || exit 1\ncat <<'EOF'\n{reply}\nEOF\n"),
    )
    .unwrap();
    fs::set_permissions(program, fs::Permissions::from_mode(0o755)).unwrap();
    let marker = release.join(".installed");
    command(&server, &["status"]).unwrap();
    assert!(!marker.exists(), "browsing must not modify a runtime");
    for verb in ["start", "restart"] {
        command(&server, &[verb]).unwrap();
        assert_eq!(fs::read(&marker).unwrap(), b"");
        fs::remove_file(&marker).unwrap();
    }
    let registry = Registry::existing(&data).unwrap();
    for path in [&release, &resources, &server] {
        for unit in path.ancestors().take_while(|p| *p != data) {
            registry
                .lock(&open_directory(unit).unwrap(), LockMode::Shared)
                .unwrap();
        }
    }
    let custom = data.parent().unwrap().join("custom");
    fs::create_dir(&custom).unwrap();
    fs::write(custom.join(PROGRAM), []).unwrap();
    point(&root.join("current"), &custom).unwrap();
    command(&server, &["start"]).unwrap();
    assert!(!custom.join(".installed").exists());
}

#[test]
fn problems_say_what_to_do() {
    let dir = scratch("problems");
    let port = Problem::explain_with(
        "",
        "[Server thread/WARN]: **** FAILED TO BIND TO PORT!\nThe exception was: Address already in use (os error 48)",
        &dir,
    );
    assert!(port.message.contains("already used"), "{}", port.message);
    assert!(matches!(port.fix, Some(Fix::UsePort(p)) if p > 25565));
    let files = Problem::explain_with(
        "sunrise-server: no prepared game files. Sunrise reads them from",
        "",
        &dir,
    );
    assert_eq!(files.fix, Some(Fix::Prepare));
    let reports = Problem::explain_with(
        "",
        "**** FAILED TO BIND TO PORT!\nThe exception was: registries.json not found next to the resources",
        &dir,
    );
    assert_eq!(reports.fix, Some(Fix::Prepare));
    let other = Problem::explain_with("sunrise-server: a server is already running in x", "", &dir);
    assert_eq!(other, Problem::plain("A server is already running in x"));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn lists_and_status_read_as_vanilla_and_the_supervisor_write_them() {
    let dir = scratch("lists");
    fs::write(
        dir.join("whitelist.json"),
        r#"[{"uuid": "a", "name": "Alex"}, {"uuid": "b", "name": "Steve"}]"#,
    )
    .unwrap();
    assert_eq!(names(&dir, "whitelist.json"), ["Alex", "Steve"]);
    assert!(names(&dir, "ops.json").is_empty());
    fs::remove_dir_all(&dir).unwrap();

    let s = Status::parse(&json!({
        "phase": "running", "engine_uptime_seconds": 61, "crashes": 1, "last_crash": "signal: 9",
        "sample": { "rss_bytes": 800_000_000u64, "cpu_percent": 3.5, "game": {
            "tps": 20.0, "mspt": { "average": 4.2 }, "max_players": 10, "paused": false,
            "players": [{ "name": "Alex", "uuid": "u1", "ping": 12 }] } },
        "update": { "installed": "v1", "available": "v2", "busy": null, "last": "Not updated: x", "ok": false }
    }));
    assert_eq!(s.phase, "running");
    assert_eq!(s.players[0].name, "Alex");
    assert_eq!(s.players[0].ping, Some(12));
    assert_eq!(
        (s.tps, s.mspt, s.max_players),
        (Some(20.0), Some(4.2), Some(10))
    );
    assert_eq!(s.update_available.as_deref(), Some("v2"));
    assert_eq!(s.update_last, Some(("Not updated: x".into(), false)));
    assert!(!Status::stopped().running());
}

/// The control API: the token goes with every call, and errors come back as the server words them.
#[test]
fn the_api_sends_the_token() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let server = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for reply in [
            r#"{"phase":"running","sample":{"game":{"players":[]}}}"#,
            r#"{"error":"the game didn't answer"}"#,
        ] {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                head.push_str(&line);
            }
            // Read the body too: closing with it unread would reset the connection.
            let length = head
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|n| n.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            let mut body = vec![0; length];
            std::io::Read::read_exact(&mut reader, &mut body).unwrap();
            seen.push(head);
            let code = if reply.contains("error") {
                "409 Conflict"
            } else {
                "200 OK"
            };
            let mut stream = stream;
            write!(
                stream,
                "HTTP/1.1 {code}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            )
            .unwrap();
        }
        seen
    });
    let dir = scratch("api");
    fs::create_dir_all(dir.join(".sunrise")).unwrap();
    fs::write(
        dir.join(".sunrise/api.json"),
        json!({ "address": address, "token": "t0k", "pid": 1 }).to_string(),
    )
    .unwrap();
    assert_eq!(status(&dir).phase, "running");
    let api = Api::open(&dir).unwrap();
    assert_eq!(
        api.command("list").unwrap_err(),
        "the game didn't answer".to_string()
    );
    let seen = server.join().unwrap();
    assert!(
        seen[0].starts_with("GET /api/v1/status HTTP/1.1"),
        "{}",
        seen[0]
    );
    assert!(seen
        .iter()
        .all(|h| h.to_ascii_lowercase().contains("authorization: bearer t0k")));
    assert!(seen[1].starts_with("POST /api/v1/command"));
    fs::remove_dir_all(&dir).unwrap();
    // Nothing answers: stopped.
    assert_eq!(status(Path::new("/nonexistent")).phase, "stopped");
}

#[test]
fn deleting_a_stopped_server_moves_its_world_and_backups_to_trash() {
    let root = scratch("delete-stopped");
    let dir = root.join("server");
    let trash = root.join("trash");
    fs::create_dir_all(dir.join("world/region")).unwrap();
    fs::create_dir_all(dir.join("backups")).unwrap();
    fs::write(dir.join("server.toml"), "[server]\n").unwrap();
    fs::write(dir.join("world/region/region.mca"), b"world").unwrap();
    fs::write(dir.join("backups/backup.zip"), b"backup").unwrap();
    trash_stopped_server(&dir, |path| crate::desktop_seams::to_folder(path, &trash)).unwrap();
    assert!(!dir.exists());
    assert_eq!(
        fs::read(trash.join("server/world/region/region.mca")).unwrap(),
        b"world"
    );
    assert_eq!(
        fs::read(trash.join("server/backups/backup.zip")).unwrap(),
        b"backup"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn deleting_refuses_a_live_supervisor_even_when_its_api_is_unavailable() {
    let root = scratch("delete-running");
    fs::create_dir_all(root.join(".sunrise")).unwrap();
    fs::write(root.join("server.toml"), "[server]\n").unwrap();
    let lock = fs::File::create(root.join(".sunrise/lock")).unwrap();
    lock.try_lock().unwrap();
    let result = trash_stopped_server(&root, |_| panic!("must not move a running server"));
    assert!(result.unwrap_err().contains("Stop the server"));
    assert!(root.join("server.toml").exists());
    drop(lock);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn deleting_refuses_an_unrecognized_folder_and_keeps_files_on_trash_failure() {
    let root = scratch("delete-invalid");
    assert!(trash_stopped_server(&root, |_| panic!("not a server")).is_err());
    fs::write(root.join("server.toml"), "[server]\n").unwrap();
    assert!(
        trash_stopped_server(&root, |_| Err("trash unavailable".into()))
            .unwrap_err()
            .contains("trash unavailable")
    );
    assert!(root.join("server.toml").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_game_mode_comes_from_server_toml() {
    let dir = scratch("game-mode");
    assert_eq!(game_mode(&dir), None);
    fs::write(
        dir.join("server.toml"),
        "[server]\nport = 25565\n\n[world]\nname = \"world\"\ngame_mode = \"creative\"\n",
    )
    .unwrap();
    assert_eq!(game_mode(&dir).as_deref(), Some("creative"));
    fs::write(dir.join("server.toml"), "[server]\nport = 25565\n").unwrap();
    assert_eq!(game_mode(&dir), None);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_engine_command_reply_becomes_the_command_and_its_problems() {
    let reply = br#"{"format": 1, "program": "/opt/sunrise/sunrise-engine", "args": ["--dedicated", "--nogui"],
        "cwd": "/srv/mc", "env": {"SUNRISE_NO_AUDIO": "1", "GONE": null}, "resources": "/srv/mc/resources"}"#;
    let (engine, resources) = engine_from_reply(reply).unwrap().unwrap();
    assert_eq!(resources, Path::new("/srv/mc/resources"));
    assert_eq!(engine.get_program(), "/opt/sunrise/sunrise-engine");
    assert_eq!(
        engine.get_args().collect::<Vec<_>>(),
        ["--dedicated", "--nogui"]
    );
    assert_eq!(engine.get_current_dir(), Some(Path::new("/srv/mc")));
    let env: Vec<_> = engine.get_envs().collect();
    assert!(env.contains(&(
        std::ffi::OsStr::new("SUNRISE_NO_AUDIO"),
        Some(std::ffi::OsStr::new("1"))
    )));
    assert!(env.contains(&(std::ffi::OsStr::new("GONE"), None)));

    let missing =
        br#"{"format": 1, "error": {"kind": "resources", "message": "no prepared game files"}}"#;
    let problem = engine_from_reply(missing).unwrap_err();
    assert_eq!(
        (problem.message.as_str(), problem.fix),
        ("no prepared game files", Some(Fix::Prepare))
    );
    let other =
        br#"{"format": 1, "error": {"kind": "configuration", "message": "server.toml: bad"}}"#;
    assert_eq!(engine_from_reply(other).unwrap_err().fix, None);

    // Builds from before engine-command: a usage line (exit 1, "unknown command"), nothing, or no object.
    for usage in [
        &b"error: unknown command engine-command\nusage: sunrise-server ..."[..],
        b"",
        b"[]",
    ] {
        assert!(engine_from_reply(usage).unwrap().is_none());
    }
    let odd = engine_from_reply(
        br#"{"format": 1, "program": "x", "args": ["--dedicated", 7], "cwd": "/"}"#,
    );
    assert!(odd.unwrap_err().message.contains("can't read"));
    let newer = engine_from_reply(br#"{"format": 2, "program": "x"}"#).unwrap_err();
    assert!(
        newer.message.contains("update the launcher"),
        "{}",
        newer.message
    );
}
