//! Old pack records remain readable; all routes that would write reject the request first.
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[test]
fn legacy_modpack_writers_refuse_in_cli_and_mcp_without_touching_the_pack() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let data = root.join("launcher");
    let game = root.join("game");
    let packs = game.join("sunrise-packs");
    let work = packs.join("cozy");
    let save = work.join("home/saves/Quiet World");
    fs::create_dir_all(&save).unwrap();
    for (path, bytes) in [
        (work.join("pack.json"), br#"{"id":"cozy","name":"Cozy","version":"0.9.0","sha256":"0000","mods":["coords_hud 0.1.0"]}"#.as_slice()),
        (save.join("level.dat"), b"existing world"),
        (work.join("home/options.txt"), b"existing options"),
        (work.join("cozy-0.9.0.spack"), b"existing pack"),
    ] {
        fs::write(path, bytes).unwrap();
    }
    let enabled = work
        .join("home")
        .join(sunrise_launcher_smod::mod_list::FILE);
    fs::write(enabled, b"existing choices").unwrap();
    fs::create_dir_all(&data).unwrap();
    fs::create_dir_all(game.join("instances")).unwrap();
    let profiles = data.join("instances.json");
    let stale = b"{\n \"instances\": {\"already-gone\": {\"name\": \"Old profile\"}}\n}\n";
    fs::write(&profiles, stale).unwrap();
    fs::write(data.join("settings.json"), br#"{"instance":"pack:cozy"}"#).unwrap();
    let mut index: Value =
        serde_json::from_str(include_str!("../crates/smod/tests/index/index-v1.json")).unwrap();
    let projects = index["projects"].as_array_mut().unwrap();
    let mut fresh = projects.iter().find(|p| p["id"] == "cozy").unwrap().clone();
    fresh["id"] = json!("fresh");
    fresh["name"] = json!("Fresh");
    projects.push(fresh);
    let index_path = root.join("index.json");
    fs::write(&index_path, serde_json::to_vec(&index).unwrap()).unwrap();
    let launcher = |args: &[&str]| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_sunrise-launcher"));
        cmd.args(args)
            .env("SUNRISE_HOME", &game)
            .env("SUNRISE_LAUNCHER_HOME", &data)
            .env("SUNRISE_LAUNCHER_LOCATION_FILE", root.join("location.json"))
            .env("SUNRISE_MOD_INDEX", &index_path)
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("SUNRISE_NO_KEYCHAIN", "1")
            .env("SUNRISE_TELEMETRY_URL", "off");
        cmd
    };
    let listed = launcher(&["--json", "modpack", "list"]).output().unwrap();
    assert!(listed.status.success(), "{listed:?}");
    let row: Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(row["pack"], "cozy");
    assert_eq!(row["version"], "0.9.0");

    const INSTALL: &str = "Installing and updating Sunrise modpacks is paused.";
    const PLAY: &str = "Playing Sunrise modpacks is paused.";
    const REMOVE: &str = "Removing Sunrise modpacks is paused.";
    let before = bytes_in(&game);
    for (args, expected) in [
        (
            &["mods", "install", "fresh", "--source", "sunrise"][..],
            INSTALL,
        ),
        (&["mods", "install", "cozy", "--source", "sunrise"], INSTALL),
        (
            &[
                "mods",
                "install",
                "cozy",
                "--source",
                "sunrise",
                "--instance",
                "already-gone",
            ],
            INSTALL,
        ),
        (&["modpack", "play", "cozy"], PLAY),
        (&["play", "--instance", "pack:cozy"], PLAY),
        (&["play"], PLAY),
        (&["modpack", "remove", "cozy"], REMOVE),
        (&["instance", "delete", "pack:cozy"], REMOVE),
    ] {
        let out = launcher(args).output().unwrap();
        assert!(!out.status.success(), "{args:?}: {out:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(expected),
            "{args:?}: {out:?}"
        );
        assert_eq!(bytes_in(&game), before, "{args:?}");
        assert_eq!(fs::read(&profiles).unwrap(), stale, "{args:?}");
    }

    let mut mcp = launcher(&["mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = mcp.stdin.take().unwrap();
    let mut output = BufReader::new(mcp.stdout.take().unwrap());
    let mut rpc = |id, method, params| {
        writeln!(
            input,
            "{}",
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        )
        .unwrap();
        let mut line = String::new();
        assert_ne!(output.read_line(&mut line).unwrap(), 0);
        let result: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(result["id"], id);
        result["result"].clone()
    };
    rpc(1, "initialize", json!({}));
    for (i, (name, arguments, expected)) in [
        (
            "mods_install",
            json!({"project":"cozy","source":"sunrise"}),
            INSTALL,
        ),
        ("modpack_play", json!({"pack":"cozy"}), PLAY),
        ("play", json!({"instance":"pack:cozy"}), PLAY),
        ("play", json!({}), PLAY),
        (
            "modpack_remove",
            json!({"pack":"cozy","confirm":true}),
            REMOVE,
        ),
        (
            "instance_delete",
            json!({"instance":"pack:cozy","confirm":true}),
            REMOVE,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let answer = rpc(
            i + 2,
            "tools/call",
            json!({"name":name,"arguments":arguments}),
        );
        assert_eq!(answer["isError"], true, "{answer}");
        assert!(
            answer["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains(expected),
            "{name}: {answer}"
        );
        assert_eq!(bytes_in(&game), before, "{name}");
        assert_eq!(fs::read(&profiles).unwrap(), stale, "{name}");
    }
    drop(input);
    assert!(mcp.wait().unwrap().success());
}

#[cfg(debug_assertions)]
#[test]
fn ordinary_pack_installs_reuse_the_only_successful_registry_reply() {
    use std::{
        net::TcpListener,
        sync::atomic::{AtomicBool, Ordering},
        thread,
        time::Duration,
    };
    use sunrise_launcher_smod::index;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut catalogue: index::Index =
        serde_json::from_str(include_str!("../crates/smod/tests/index/index-v1.json")).unwrap();
    let mut shader = catalogue.project("glow").unwrap().clone();
    let shader_file = root.join(&shader.versions[0].file.name);
    fs::write(
        &shader_file,
        include_bytes!("../crates/smod/tests/index/glow-1.0.0.sshader"),
    )
    .unwrap();
    shader.versions[0].file.url = url::Url::from_file_path(&shader_file).unwrap().into();
    catalogue.projects = vec![shader.clone()];
    for (id, kind, namespace) in [
        (
            "textures",
            index::Kind::ResourcePack,
            "assets/lanterns/fixture.json",
        ),
        (
            "lanterns",
            index::Kind::DataPack,
            "data/lanterns/function/light.mcfunction",
        ),
    ] {
        let name = format!("{id}-1.0.0.zip");
        let path = root.join(&name);
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        for (name, bytes) in [
            (
                "pack.mcmeta",
                br#"{"pack":{"pack_format":94,"description":"Lanterns"}}"#.as_slice(),
            ),
            (namespace, b"say lit"),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        let mut project = shader.clone();
        project.id = id.into();
        project.kind = kind;
        project.name = id.into();
        project.versions[0].api = "94".into();
        project.versions[0].file = index::File {
            name,
            url: url::Url::from_file_path(&path).unwrap().into(),
            size: fs::metadata(&path).unwrap().len(),
            sha256: index::sha256_file(&path).unwrap(),
            signature: None,
        };
        catalogue.projects.push(project);
    }
    let body = serde_json::to_vec(&catalogue).unwrap();
    for (id, folder, file) in [
        ("textures", "resourcepacks", "textures-1.0.0.zip"),
        ("glow", "shaderpacks", "glow-1.0.0.sshader"),
        ("lanterns", "saves/World/datapacks", "lanterns-1.0.0.zip"),
    ] {
        let game = root.join(id).join("game");
        let save = game.join("saves/World");
        fs::create_dir_all(&save).unwrap();
        fs::write(save.join("level.dat"), b"fixture world").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let finished = AtomicBool::new(false);
        let (out, requests) = thread::scope(|scope| {
            let server = scope.spawn(|| {
                let mut requests = 0;
                while !finished.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                            let mut headers = BufReader::new(&stream);
                            let mut line = String::new();
                            headers.read_line(&mut line).unwrap();
                            assert!(line.starts_with("GET /registry.json HTTP/"), "{line:?}");
                            loop {
                                line.clear();
                                if headers.read_line(&mut line).unwrap() == 0 || line == "\r\n" { break }
                            }
                            requests += 1;
                            if requests == 1 {
                                write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                                stream.write_all(&body).unwrap();
                            } else {
                                stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(5)),
                        Err(e) => panic!("{e}"),
                    }
                }
                requests
            });
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_sunrise-launcher"));
            cmd.args(["mods", "install", id, "--source", "sunrise"])
                .env("SUNRISE_HOME", &game)
                .env("SUNRISE_LAUNCHER_HOME", root.join(id).join("launcher"))
                .env(
                    "SUNRISE_LAUNCHER_LOCATION_FILE",
                    root.join(id).join("location.json"),
                )
                .env(
                    "SUNRISE_MOD_INDEX",
                    format!("http://{address}/registry.json"),
                )
                .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
                .env("SUNRISE_NO_KEYCHAIN", "1")
                .env("SUNRISE_TELEMETRY_URL", "off");
            if id == "lanterns" {
                cmd.args(["--world", "World"]);
            }
            let out = cmd.output();
            finished.store(true, Ordering::Relaxed);
            (out.unwrap(), server.join().unwrap())
        });
        assert!(
            out.status.success(),
            "{id}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            requests, 1,
            "{id} must use the one accepted registry response"
        );
        assert_eq!(
            fs::read(game.join(folder).join(file)).unwrap(),
            fs::read(root.join(file)).unwrap()
        );
    }
}

fn bytes_in(dir: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .flat_map(|path| {
            if path.is_dir() {
                let mut rows = bytes_in(&path);
                rows.insert(path, None);
                rows
            } else {
                let data = fs::read(&path).unwrap();
                BTreeMap::from([(path, Some(data))])
            }
        })
        .collect()
}
