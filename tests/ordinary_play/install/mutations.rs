//! Real CLI/MCP Update and Remove, the exact enabled graph, and world evidence at cutoff.
use super::*;
use sunrise_launcher::browser::mod_list as selection;
use sunrise_launcher_nbt::{self as nbt, Limits, NamedTag, Tag};

fn world(home: &Path, folder: &str, ids: &[&str]) {
    let tags = ids
        .iter()
        .map(|id| {
            Tag::compound([
                ("ModId", Tag::String((*id).into())),
                ("ModVersion", Tag::String("1.0.0".into())),
            ])
        })
        .collect();
    world_with_evidence(
        home,
        folder,
        Tag::compound([("ContentModList", Tag::List(tags))]),
    );
}

fn world_with_evidence(home: &Path, folder: &str, sunrise: Tag) {
    let dest = home.join("saves").join(folder);
    fs::create_dir_all(&dest).unwrap();
    let root = Tag::compound([
        (
            "Data",
            Tag::compound([("LevelName", Tag::String(folder.into()))]),
        ),
        ("sunrise", sunrise),
    ]);
    fs::write(
        dest.join("level.dat"),
        nbt::write_named(
            &NamedTag {
                name: "".into(),
                tag: root,
            },
            Limits::default(),
        )
        .unwrap(),
    )
    .unwrap();
}

fn release(
    fixture: &Fixture,
    registry: &mut index::Index,
    id: &str,
    version: &str,
    files: &[(&str, &[u8])],
) -> Vec<u8> {
    let project = registry.projects.iter_mut().find(|p| p.id == id).unwrap();
    let mut row = project.versions[0].clone();
    row.version = version.into();
    let deps = match id {
        "root_mod" => "[dependencies]\nrequired_mod='^1'\n",
        _ => "",
    };
    let package = Package::write_files(&format!("id='{id}'\nname='{}'\nversion='{version}'\napi='0.2'\nside='both'\ncapabilities=['registries']\n{deps}", project.name), files).unwrap();
    let path = fixture.root.join(format!("{id}-{version}.smod"));
    fs::write(&path, &package).unwrap();
    row.file = index::File {
        name: path.file_name().unwrap().to_string_lossy().into_owned(),
        url: format!("file://{}", path.display()),
        size: package.len() as u64,
        sha256: index::sha256(&package),
        signature: None,
    };
    project.versions.retain(|v| v.version != version);
    project.versions.insert(0, row);
    package
}

fn run(fixture: &Fixture, args: &[&str]) -> (bool, String) {
    let out = fixture
        .registry_client(args)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (
        out.status.success(),
        format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

fn install_root(fixture: &Fixture, registry: &index::Index) {
    fixture.publish_registry(registry);
    let (ok, text) = run(
        fixture,
        &["mods", "install", "root_mod", "--source", "sunrise"],
    );
    assert!(ok, "{text}");
}

fn live_state(fixture: &Fixture) -> [Vec<u8>; 4] {
    [
        fixture.pointer(),
        fs::read(fixture.home.join(index::browser_state::MANIFEST)).unwrap(),
        fs::read(fixture.home.join(mod_list::FILE)).unwrap(),
        fs::read(fixture.home.join(mod_list::DIR).join("root_mod.smod")).unwrap(),
    ]
}

#[test]
fn a_pack_row_cannot_update_or_remove_an_installed_mod_archive() {
    let fixture = Fixture::new();
    let (mut registry, archives) = fixture.content_index();
    let mut shader = fixture.registry().project("glow").unwrap().clone();
    let shader_name = &shader.versions[0].file.name;
    let download = fixture.root.join(shader_name);
    let shader_bytes = include_bytes!("../../../crates/smod/tests/index/glow-1.0.0.sshader");
    fs::write(&download, shader_bytes).unwrap();
    let shader_file = fixture.home.join("shaderpacks").join(shader_name);
    shader.versions[0].file.url = format!("file://{}", download.display());
    registry.projects.push(shader);
    fixture.publish_registry(&registry);

    let archive = format!("{}/root_mod.smod", mod_list::DIR);
    fs::create_dir_all(fixture.home.join(mod_list::DIR)).unwrap();
    fs::write(fixture.home.join(&archive), &archives["root_mod"]).unwrap();
    let manifest = fixture.home.join(index::browser_state::MANIFEST);
    let mut rows = json!({"entries": [{
        "source": "sunrise", "project": "glow", "slug": "glow", "title": "Soft Glow",
        "kind": "shader", "version": "0.9.0", "number": "0.9.0", "file": archive,
        "checksum": format!("SHA-256:{}", index::sha256(&archives["root_mod"])), "page": ""
    }]});
    let before = serde_json::to_vec(&rows).unwrap();
    fs::write(&manifest, &before).unwrap();
    for args in [
        &["mods", "update", "glow", "--check"][..],
        &["mods", "update", "glow"],
        &["mods", "remove", "glow"],
    ] {
        let (ok, text) = run(&fixture, args);
        assert!(
            !ok && text.contains("isn't a pack file"),
            "{args:?}: {text}"
        );
        assert_eq!(
            fs::read(fixture.home.join(&archive)).unwrap(),
            archives["root_mod"]
        );
        assert_eq!(fs::read(&manifest).unwrap(), before);
        assert!(!shader_file.exists());
    }

    // The same commands still work on an actual shader pack, including replacing its older file.
    let old_shader = fixture.home.join("shaderpacks/glow-0.9.0.sshader");
    fs::create_dir_all(old_shader.parent().unwrap()).unwrap();
    fs::write(&old_shader, shader_bytes).unwrap();
    rows["entries"][0]["file"] = json!("shaderpacks/glow-0.9.0.sshader");
    rows["entries"][0]["checksum"] = json!(format!("SHA-256:{}", index::sha256(shader_bytes)));
    fs::write(&manifest, serde_json::to_vec(&rows).unwrap()).unwrap();
    let (ok, text) = run(&fixture, &["mods", "update", "glow", "--check"]);
    assert!(ok && text.contains("1.0.0 is available"), "{text}");
    let (ok, text) = run(&fixture, &["mods", "update", "glow"]);
    assert!(ok, "{text}");
    assert!(!old_shader.exists());
    assert_eq!(fs::read(&shader_file).unwrap(), shader_bytes);
    let (ok, text) = run(&fixture, &["mods", "remove", "glow"]);
    assert!(ok, "{text}");
    assert!(!shader_file.exists());
    assert_eq!(
        fs::read(fixture.home.join(archive)).unwrap(),
        archives["root_mod"]
    );
}

#[test]
fn a_self_consistent_local_index_cannot_update_but_explicit_remove_can_remove_its_bytes() {
    let fixture = Fixture::new();
    let (mut registry, _) = fixture.content_index();
    install_root(&fixture, &registry);
    let foreign = release(
        &fixture,
        &mut registry,
        "root_mod",
        "1.0.0",
        &[("data/root_mod/sunrise/block/local_lamp.json", b"{}")],
    );
    release(&fixture, &mut registry, "root_mod", "1.1.0", &[]);
    fixture.publish_registry(&registry);
    let archive = fixture.home.join(mod_list::DIR).join("root_mod.smod");
    fs::write(&archive, &foreign).unwrap();
    let digest = index::sha256(&foreign);
    let rows = fixture.home.join(index::browser_state::MANIFEST);
    let mut forged: serde_json::Value = serde_json::from_slice(&fs::read(&rows).unwrap()).unwrap();
    let row = forged["entries"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|row| row["project"] == "root_mod")
        .unwrap();
    row["checksum"] = json!(format!("SHA-256:{digest}"));
    fs::write(rows, serde_json::to_vec(&forged).unwrap()).unwrap();
    let old = live_state(&fixture);
    let (ok, text) = run(
        &fixture,
        &["mods", "update", "root_mod", "--confirm-world-loss"],
    );
    assert!(!ok && text.contains("registry"), "{text}");
    assert_eq!(live_state(&fixture), old);

    let mut mcp = Running(
        fixture
            .registry_client(&["mcp"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut input = mcp.0.stdin.take().unwrap();
    let mut output = BufReader::new(mcp.0.stdout.take().unwrap());
    mcp_request(&mut input, &mut output, 1, "initialize", json!({}));
    let denied = mcp_request(
        &mut input,
        &mut output,
        2,
        "tools/call",
        json!({"name":"mods_update", "arguments":{"project":"root_mod","confirm_world_loss":true}}),
    );
    assert_eq!(denied["isError"], true, "{denied}");
    assert!(denied.to_string().contains("registry"), "{denied}");
    assert_eq!(live_state(&fixture), old);

    let (ok, text) = run(&fixture, &["mods", "remove", "root_mod"]);
    assert!(
        ok && text.contains("sunrise-mods/root_mod.smod") && text.contains(&digest),
        "{text}"
    );
    assert!(!archive.exists());
}

#[test]
fn namespace_only_and_malformed_worlds_cannot_bypass_the_decision() {
    let fixture = Fixture::new();
    let (mut registry, _) = fixture.content_index();
    let defs = [("data/root_mod/sunrise/block/amber.json", b"{}".as_slice())];
    release(&fixture, &mut registry, "root_mod", "1.0.0", &defs);
    install_root(&fixture, &registry);
    let old = live_state(&fixture);
    for evidence in [
        Tag::compound([(
            "BlockNamespaces",
            Tag::List(vec![Tag::String("root_mod".into())]),
        )]),
        Tag::compound([(
            "ContentModList",
            Tag::List(vec![Tag::compound([
                ("ModId", Tag::String("unrelated".into())),
                ("ModVersion", Tag::Int(7)),
            ])]),
        )]),
    ] {
        world_with_evidence(&fixture.home, "Same world", evidence);
        let (ok, text) = run(&fixture, &["mods", "remove", "root_mod"]);
        assert!(
            !ok && text.contains("1 saved world") && text.contains("--confirm-world-loss"),
            "{text}"
        );
        assert_eq!(live_state(&fixture), old);
    }
}

#[test]
fn explicit_remove_refuses_changed_worlds_or_archive_bytes_at_cutoff() {
    for mode in ["add-world", "replace-world", "replace-archive"] {
        let fixture = Fixture::new();
        let (registry, archives) = fixture.content_index();
        install_root(&fixture, &registry);
        world(&fixture.home, "First world", &["root_mod"]);
        let old = live_state(&fixture);
        let required = fixture.home.join(mod_list::DIR).join("required_mod.smod");
        let mut worker = Command::new(std::env::current_exe().unwrap());
        for (key, value) in fixture.registry_client(&[]).get_envs() {
            worker.env(key, value.unwrap());
        }
        let output = worker
            .args(["--exact", "install::mutations::remove_cutoff_worker"])
            .env("FIXTURE_REMOVE_CUTOFF", mode)
            .output()
            .unwrap();
        assert!(output.status.success(), "{mode}: {output:?}");
        match mode {
            "replace-world" => assert_eq!(
                fs::read(fixture.home.join("saves/First world/level.dat")).unwrap(),
                fs::read(fixture.home.join("previous-world/level.dat")).unwrap(),
            ),
            "add-world" => assert!(fixture.home.join("saves/New world/level.dat").is_file()),
            _ => (),
        }
        assert_eq!(live_state(&fixture), old);
        let expected = if mode == "replace-archive" {
            replacement()
        } else {
            archives["required_mod"].clone()
        };
        assert_eq!(fs::read(required).unwrap(), expected);
    }
}

fn replacement() -> Vec<u8> {
    Package::write_files(
        "id='required_mod'\nname='A different file'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n",
        &[],
    ).unwrap()
}

#[test]
fn remove_cutoff_worker() {
    let Ok(mode) = std::env::var("FIXTURE_REMOVE_CUTOFF") else {
        return;
    };
    use sunrise_launcher::{browser::sunrise::*, instances, managed_play};
    let instance = instances::selection().unwrap();
    let identity = install_target(&instance).unwrap();
    let archive = instance.home.join(mod_list::DIR).join("required_mod.smod");
    let digest = index::sha256_file(&archive).unwrap();
    let decide = |impact: &ChangeImpact| {
        let detail = impact.detail();
        assert!(detail.contains("1 saved world"), "{detail}");
        assert!(detail.contains("Lanterns (root_mod)"), "{detail}");
        assert!(
            detail.contains("sunrise-mods/required_mod.smod") && detail.contains(&digest),
            "{detail}"
        );
        match mode.as_str() {
            "replace-archive" => fs::write(&archive, replacement()).unwrap(),
            "replace-world" => {
                fs::rename(
                    instance.home.join("saves/First world"),
                    instance.home.join("previous-world"),
                )
                .unwrap();
                world(&instance.home, "First world", &["root_mod"]);
            }
            _ => world(&instance.home, "New world", &["root_mod"]),
        }
        true
    };
    let hooks = InstallHooks {
        progress: &|_| {},
        native: &|_| panic!("content-only"),
        registry_crates: None,
        continue_after_failure: &|_| panic!("content-only"),
    };
    let error = remove_mods(
        &instance,
        identity,
        &["required_mod".into()],
        || managed_play::headless_first(&instance, None),
        hooks,
        &decide,
    )
    .unwrap_err();
    let expected = if mode == "replace-archive" {
        "Sunrise couldn't verify the profile's saved game build."
    } else {
        "The affected worlds changed after review. Nothing was changed; try again."
    };
    assert!(error.to_string().contains(expected), "{error}");
}

#[test]
fn remove_names_dependents_even_without_worlds_and_requires_compatible_activation() {
    let fixture = Fixture::new();
    let (registry, _) = fixture.content_index();
    install_root(&fixture, &registry);
    let (ok, text) = run(&fixture, &["mods", "remove", "required_mod"]);
    assert!(ok && text.contains("Lanterns (root_mod)"), "{text}");
    assert!(!text.contains("saved world"), "{text}");
    assert_eq!(text.matches("will be turned off").count(), 1, "{text}");
    assert!(!selection::is_enabled(&fixture.home, "root_mod"));
    let (ok, text) = run(&fixture, &["mods", "enable", "root_mod"]);
    assert!(!ok && text.contains("needs required_mod"), "{text}");
    let (ok, text) = run(
        &fixture,
        &["mods", "install", "required_mod", "--source", "sunrise"],
    );
    assert!(ok, "{text}");
    assert!(!selection::is_enabled(&fixture.home, "root_mod"));
    let (ok, text) = run(&fixture, &["mods", "enable", "root_mod"]);
    assert!(ok && !text.contains("saved world"), "{text}");
    assert!(selection::is_enabled(&fixture.home, "root_mod"));
}

fn local(fixture: &Fixture, id: &str, required: &str, native: bool) -> (PathBuf, Vec<u8>) {
    let native_manifest = if native {
        format!("[native]\nsunrise='{}'\ngame='26.3'\n", "a".repeat(40))
    } else {
        String::new()
    };
    let text = format!("id='{id}'\nname='{id} title'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n[dependencies]\n{required}\n{native_manifest}");
    let cargo = format!("[package]\nname='{id}'\nversion='1.0.0'\n");
    let native_files: &[(&str, &[u8])] = &[("Cargo.toml", cargo.as_bytes()), ("src/lib.rs", b"")];
    let archive = Package::write_files(&text, if native { native_files } else { &[] }).unwrap();
    let directory = fixture.home.join(if native {
        mod_list::NATIVE_DIR
    } else {
        mod_list::DIR
    });
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join(format!("{id}.smod"));
    fs::write(&path, &archive).unwrap();
    (path, archive)
}

#[test]
fn remove_accepts_local_and_github_archives_but_not_unpacked_folders() {
    let fixture = Fixture::new();
    let (registry, _) = fixture.content_index();
    install_root(&fixture, &registry);
    let loose = local(&fixture, "local_lamp", "", false);
    let github = local(&fixture, "github_lamp", "", true);
    let sidecar = github.0.with_extension("origin.json");
    fs::write(&sidecar, json!({"id":"github_lamp", "version":"1.0.0", "owner":"fixture", "repo":"lanterns", "commit":"a".repeat(40), "sha256":index::sha256(&github.1)}).to_string()).unwrap();
    let unpacked = fixture.home.join(mod_list::DIR).join("unpacked_lamp");
    fs::create_dir(&unpacked).unwrap();
    let manifest = unpacked.join("sunrise-mod.toml");
    fs::write(&manifest, "id='unpacked_lamp'\nname='Unpacked Lamp'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n").unwrap();
    let expected = live_state(&fixture);
    let (ok, text) = run(&fixture, &["mods", "remove", "unpacked_lamp"]);
    assert!(!ok && text.contains("Nothing was removed"), "{text}");
    assert!(manifest.is_file());
    assert_eq!(live_state(&fixture), expected);

    for (id, (path, bytes)) in [("github_lamp", github), ("local_lamp", loose)] {
        let (ok, text) = run(&fixture, &["mods", "remove", id]);
        assert!(ok && text.contains(&index::sha256(&bytes)), "{text}");
        assert!(!path.exists());
    }
    assert!(!sidecar.exists());
    assert!(manifest.is_file());
    assert_eq!(
        fs::read(fixture.home.join(index::browser_state::MANIFEST)).unwrap(),
        expected[1]
    );
}

#[test]
fn remove_refuses_running_game_and_disables_exact_dependents_in_place_in_one_commit() {
    let fixture = Fixture::new();
    let (registry, archives) = fixture.content_index();
    install_root(&fixture, &registry);
    let mut play = Running(fixture.registry_client(&["play"]).spawn().unwrap());
    assert_resource_args(&fixture, &fixture.game_arguments());
    let old = live_state(&fixture);
    let (ok, text) = run(
        &fixture,
        &["mods", "remove", "required_mod", "--confirm-world-loss"],
    );
    assert!(
        !ok && text.contains("This profile is in use. Close the game"),
        "{text}"
    );
    assert_eq!(live_state(&fixture), old);
    fs::write(fixture.home.join("finish"), []).unwrap();
    assert_eq!(play.0.wait().unwrap().code(), Some(7));
    fs::remove_file(fixture.home.join("args")).unwrap();

    let transitive = local(&fixture, "local_native", "root_mod='^1'", true);
    let github = local(&fixture, "github_native", "local_native='^1'", true);
    let origin = github.0.with_extension("origin.json");
    let origin_bytes = serde_json::to_vec(&json!({"id":"github_native", "version":"1.0.0", "owner":"fixture", "repo":"native", "commit":"a".repeat(40), "sha256":index::sha256(&github.1)})).unwrap();
    fs::write(&origin, &origin_bytes).unwrap();
    let optional = local(
        &fixture,
        "optional_mod",
        "required_mod={version='^1',optional=true}",
        false,
    );
    let off = local(&fixture, "off_mod", "required_mod='^1'", false);
    selection::set_all_enabled(
        &fixture.home,
        &["local_native", "github_native", "optional_mod"],
        true,
    )
    .unwrap();
    world(&fixture.home, "Native world", &["local_native"]);
    fs::create_dir(fixture.home.join("backups")).unwrap();
    fs::write(fixture.home.join("backups/unchanged.zip"), b"older backup").unwrap();
    let old = live_state(&fixture);
    let (ok, text) = run(&fixture, &["mods", "remove", "required_mod"]);
    assert!(
        !ok && text.contains("1 saved world") && text.contains("--confirm-world-loss"),
        "{text}"
    );
    for name in [
        "Lanterns (root_mod)",
        "local_native title (local_native)",
        "github_native title (github_native)",
    ] {
        assert!(text.contains(name), "{text}");
    }
    assert!(
        !text.contains("optional_mod") && !text.contains("off_mod"),
        "{text}"
    );
    assert_eq!(live_state(&fixture), old);

    let mut mcp = Running(
        fixture
            .registry_client(&["mcp"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut input = mcp.0.stdin.take().unwrap();
    let mut output = BufReader::new(mcp.0.stdout.take().unwrap());
    let mut rpc = |id, method, params| mcp_request(&mut input, &mut output, id, method, params);
    rpc(1, "initialize", json!({}));
    let declined = rpc(
        2,
        "tools/call",
        json!({"name":"mods_remove", "arguments":{"project":"required_mod","confirm":true}}),
    );
    assert_eq!(declined["isError"], true, "{declined}");
    assert!(declined.to_string().contains("1 saved world"), "{declined}");
    assert_eq!(live_state(&fixture), old);
    let accepted = rpc(
        3,
        "tools/call",
        json!({"name":"mods_remove", "arguments":{"project":"required_mod","confirm":true,"confirm_world_loss":true}}),
    );
    assert_ne!(accepted["isError"], true, "{accepted}");
    assert!(!fixture
        .home
        .join(mod_list::DIR)
        .join("required_mod.smod")
        .exists());
    assert_eq!(
        fs::read(fixture.home.join(mod_list::DIR).join("root_mod.smod")).unwrap(),
        archives["root_mod"]
    );
    assert_eq!(fs::read(origin).unwrap(), origin_bytes);
    for (path, archive) in [transitive, github, optional, off] {
        assert_eq!(fs::read(path).unwrap(), archive);
    }
    for id in ["root_mod", "local_native", "github_native", "off_mod"] {
        assert!(!selection::is_enabled(&fixture.home, id));
    }
    assert!(selection::is_enabled(&fixture.home, "optional_mod"));
    assert_eq!(
        fs::read(fixture.home.join("backups/unchanged.zip")).unwrap(),
        b"older backup"
    );
    let reduced = fixture.pointer();
    let (ok, text) = run(&fixture, &["mods", "enable", "root_mod"]);
    assert!(!ok && text.contains("needs required_mod"), "{text}");
    assert_eq!(fixture.pointer(), reduced);
    let mut play = Running(fixture.registry_client(&["play"]).spawn().unwrap());
    assert_resource_args(&fixture, &fixture.game_arguments());
    assert_eq!(play.0.wait().unwrap().code(), Some(7));
    assert_eq!(fixture.pointer(), reduced);
}
