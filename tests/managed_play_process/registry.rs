//! Shipped registry commands and Play using a local unsigned game and registry on every CI host.
use super::*;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use sunrise_launcher_smod::{index, mod_list, Package};

pub(super) fn ordinary_install() {
    const TAG: &str = "developer-local";
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let home = root.join("game");
    let data = root.join("launcher");
    let build = data.join("versions").join(TAG);
    let resources = build.join(".local/sunrise-client/26.3");
    fs::create_dir(&home).unwrap();
    fs::create_dir_all(&resources).unwrap();
    fs::write(resources.join("sunrise-resources.json"), "{}").unwrap();
    fs::write(build.join(".installed"), []).unwrap();
    fs::write(
        build.join("developer.json"),
        json!({"revision": "a".repeat(40)}).to_string(),
    )
    .unwrap();
    fs::write(
        build.join("research-lock.json"),
        r#"{"minecraft":{"version":"26.3"}}"#,
    )
    .unwrap();
    copy_game_image(&build.join(format!("sunrise-client{}", std::env::consts::EXE_SUFFIX)));
    fs::write(
        data.join("instances.json"),
        json!({"instances":{"sunrise":{"version":TAG}}}).to_string(),
    )
    .unwrap();

    let mut index =
        index::Index::parse(include_str!("../../crates/smod/tests/index/index-v1.json")).unwrap();
    let template = index.project("coords_hud").unwrap().clone();
    let files = ["lanterns", "fireflies"].map(|id| {
        let bytes = Package::write_files(&format!("id='{id}'\nname='{id}'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n"), &[]).unwrap();
        let name = format!("{id}.smod");
        let path = root.join(&name);
        fs::write(&path, &bytes).unwrap();
        let facts = index::inspect(&index::Kind::Mod, &path).unwrap();
        let mut version = template.versions[0].clone();
        version.version = "1.0.0".into();
        version.api = facts.api;
        version.side = facts.side;
        version.game = "26.3".into();
        version.capabilities = facts.capabilities.into();
        version.dependencies = None;
        version.file = index::File {
            name: name.clone(), url: url::Url::from_file_path(&path).unwrap().to_string(),
            size: bytes.len() as u64, sha256: index::sha256(&bytes), signature: None,
        };
        (index::Project { id: id.into(), name: id.into(), key: None, versions: vec![version], ..template.clone() }, name, bytes)
    });
    index.projects = files
        .iter()
        .map(|(project, _, _)| project.clone())
        .collect();
    let registry = root.join("index.json");
    let text = serde_json::to_string(&index).unwrap();
    let parsed = index::Index::parse_local(&text).unwrap();
    assert!(parsed.left_out.is_empty(), "{:?}", parsed.left_out);
    fs::write(&registry, text).unwrap();
    let launcher = |args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sunrise-launcher"));
        command
            .args(args)
            .env(ROLE, "game")
            .env(MODE, "registry-install")
            .env("SUNRISE_HOME", &home)
            .env("SUNRISE_LAUNCHER_HOME", &data)
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("SUNRISE_MOD_INDEX", &registry)
            .env("SUNRISE_LAUNCHER_DEVELOPER_OFFLINE", "1")
            .env("SUNRISE_NO_KEYCHAIN", "1")
            .env("SUNRISE_TELEMETRY_URL", "off");
        command
    };
    println!("registry Play fixture: CLI Install");
    let installed = launcher(&[
        "--json", "mods", "install", "lanterns", "--source", "sunrise",
    ])
    .output()
    .unwrap();
    assert!(installed.status.success(), "{installed:?}");
    assert_eq!(
        fs::read(home.join(mod_list::DIR).join(&files[0].1)).unwrap(),
        files[0].2
    );
    let pointer = home.join("mod-browser/active-build.json");
    let before = fs::read(&pointer).unwrap();
    println!("registry Play fixture: CLI Play");
    let mut game = Guard(launcher(&["play"]).stdin(Stdio::null()).spawn().unwrap());
    await_event(|| {
        assert!(
            game.0.try_wait().unwrap().is_none(),
            "Play exited before it reached its game"
        );
        home.join("game-ready").is_file()
    });
    let mut mcp = Guard(
        launcher(&["mcp"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut to = mcp.0.stdin.take().unwrap();
    let mut from = BufReader::new(mcp.0.stdout.take().unwrap());
    let mut rpc = |id, method, params: Value| {
        writeln!(
            to,
            "{}",
            json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params})
        )
        .unwrap();
        let mut line = String::new();
        assert_ne!(from.read_line(&mut line).unwrap(), 0);
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], id);
        assert!(response.get("error").is_none(), "{response}");
        response["result"].clone()
    };
    rpc(1, "initialize", json!({}));
    let install =
        json!({"name":"mods_install", "arguments":{"project":"fireflies", "source":"sunrise"}});
    println!("registry Play fixture: MCP Install with Play active");
    let busy = rpc(2, "tools/call", install.clone());
    assert_eq!(busy["isError"], true, "{busy}");
    assert_eq!(fs::read(&pointer).unwrap(), before);
    assert!(!home.join(mod_list::DIR).join(&files[1].1).exists());
    fs::write(home.join("game-stop"), []).unwrap();
    assert_eq!(game.0.wait().unwrap().code(), Some(23));
    println!("registry Play fixture: MCP Install after Play exits");
    let done = rpc(3, "tools/call", install);
    assert_ne!(done["isError"], true, "{done}");
    assert_ne!(fs::read(pointer).unwrap(), before);
    for (project, file, bytes) in files {
        assert_eq!(
            fs::read(home.join(mod_list::DIR).join(file)).unwrap(),
            bytes
        );
        assert!(sunrise_launcher::browser::mod_list::is_enabled(
            &home,
            &project.id
        ));
    }
    drop(to);
    assert!(mcp.0.wait().unwrap().success());
    println!("Shipped CLI Install/Play and MCP Install advanced only a free profile");
}
