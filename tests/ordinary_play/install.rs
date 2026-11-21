//! Stage and publish real registry archives through the CLI and generated MCP command.
use super::*;
use std::collections::BTreeMap;
use sunrise_launcher_smod::{index, Package};

#[path = "install/native_retry.rs"]
mod native_retry;

impl Fixture {
    fn registry(&self) -> index::Index {
        index::Index::parse(include_str!("../../crates/smod/tests/index/index-v1.json")).unwrap()
    }

    fn publish_registry(&self, registry: &index::Index) {
        fs::write(
            self.root.join("index.json"),
            serde_json::to_vec(registry).unwrap(),
        )
        .unwrap();
    }

    fn registry_client(&self, args: &[&str]) -> Command {
        let mut cmd = self.launcher(args);
        cmd.env("SUNRISE_MOD_INDEX", self.root.join("index.json"));
        cmd
    }

    fn content_index(&self) -> (index::Index, BTreeMap<String, Vec<u8>>) {
        let mut registry = self.registry();
        let template = registry.project("coords_hud").unwrap().clone();
        registry.projects.clear();
        let mut archives = BTreeMap::new();
        for (id, name, dependencies) in [
            (
                "root_mod",
                "Lanterns",
                "[dependencies]\nrequired_mod = '^1'",
            ),
            ("required_mod", "Lantern Core", ""),
            ("extra_mod", "Fireflies", ""),
        ] {
            let archive = Package::write_files(
                &format!("id='{id}'\nname='{name}'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n{dependencies}"),
                &[],
            ).unwrap();
            let file_name = format!("{id}-1.0.0.smod");
            let file = self.root.join(&file_name);
            fs::write(&file, &archive).unwrap();
            let facts = index::inspect(&index::Kind::Mod, &file).unwrap();
            let mut version = template.versions[0].clone();
            version.version = "1.0.0".into();
            version.game = "26.3".into();
            version.api = facts.api;
            version.side = facts.side;
            version.capabilities = serde_json::from_value(json!(facts.capabilities)).unwrap();
            version.dependencies = Some(if id == "root_mod" {
                vec![index::Dependency {
                    id: "required_mod".into(),
                    version: "^1".into(),
                    optional: false,
                    side: None,
                }]
            } else {
                Vec::new()
            });
            version.file = index::File {
                name: file_name,
                url: format!("file://{}", file.display()),
                size: archive.len() as u64,
                sha256: index::sha256(&archive),
                signature: None,
            };
            registry.projects.push(index::Project {
                id: id.into(),
                name: name.into(),
                key: None,
                versions: vec![version],
                ..template.clone()
            });
            archives.insert(id.to_owned(), archive);
        }
        (registry, archives)
    }
}

#[test]
fn rejected_dependency_leaves_the_old_profile_unchanged_then_cli_install_and_play_use_the_same_base(
) {
    let fixture = Fixture::new();
    let (registry, archives) = fixture.content_index();
    let cli = || {
        fixture
            .registry_client(&[
                "--json", "mods", "install", "root_mod", "--source", "sunrise",
            ])
            .output()
            .unwrap()
    };
    let mut bad = registry.clone();
    bad.projects
        .iter_mut()
        .find(|p| p.id == "required_mod")
        .unwrap()
        .versions[0]
        .file
        .sha256 = "0".repeat(64);
    let installed = fixture.home.join(index::browser_state::MANIFEST);
    let opaque = json!({"entries":[{"future_row":{"owner":"game"}}],"future_top":[0, "opaque"]});
    let before = serde_json::to_vec(&opaque).unwrap();
    fs::write(&installed, &before).unwrap();
    fixture.publish_registry(&bad);
    let out = cli();
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("checksum")
            || String::from_utf8_lossy(&out.stderr).contains("SHA-256"),
        "{out:?}"
    );
    assert_eq!(fs::read(&installed).unwrap(), before);
    assert!(!fixture.home.join(mod_list::DIR).exists());
    assert!(!fixture.home.join(mod_list::FILE).exists());
    assert!(!fixture.home.join("mod-browser/active-build.json").exists());
    assert!(!fixture.home.join("args").exists());

    fixture.publish_registry(&registry);
    let out = cli();
    assert!(out.status.success(), "{out:?}");
    let rows: Vec<Value> = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .any(|r| r["project"] == "root_mod" && r["dependency"] == false));
    assert!(rows
        .iter()
        .any(|r| r["project"] == "required_mod" && r["dependency"] == true));
    let after: Value = serde_json::from_slice(&fs::read(&installed).unwrap()).unwrap();
    assert_eq!(after["entries"][0], opaque["entries"][0]);
    assert_eq!(after["future_top"], opaque["future_top"]);
    for id in ["required_mod", "root_mod"] {
        let file = fixture.home.join(mod_list::DIR).join(format!("{id}.smod"));
        assert_eq!(fs::read(file).unwrap(), archives[id]);
        assert!(sunrise_launcher::browser::mod_list::is_enabled(
            &fixture.home,
            id
        ));
    }
    let pointer = fixture.pointer();
    let mut play = Running(fixture.registry_client(&["play"]).spawn().unwrap());
    let args = fixture.game_arguments();
    assert_resource_args(&fixture, &args);
    fs::write(fixture.home.join("finish"), []).unwrap();
    assert_eq!(play.0.wait().unwrap().code(), Some(7));
    assert_eq!(fixture.pointer(), pointer);
}

#[test]
fn generated_mcp_install_advances_only_a_free_profile_and_keeps_existing_packages() {
    let fixture = Fixture::new();
    let (registry, archives) = fixture.content_index();
    fixture.publish_registry(&registry);
    let mut mcp = Running(
        fixture
            .registry_client(&["mcp"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut to_mcp = mcp.0.stdin.take().unwrap();
    let mut from_mcp = BufReader::new(mcp.0.stdout.take().unwrap());
    let mut rpc = |id, method, params| mcp_request(&mut to_mcp, &mut from_mcp, id, method, params);
    rpc(1, "initialize", json!({}));
    let install = |project| json!({"name":"mods_install", "arguments":{"project":project,"source":"sunrise"}});
    let answer = rpc(2, "tools/call", install("root_mod"));
    assert_ne!(answer["isError"], true, "{answer}");
    let pointer = fixture.pointer();
    let mut play = Running(fixture.registry_client(&["play"]).spawn().unwrap());
    assert_resource_args(&fixture, &fixture.game_arguments());
    let refused = rpc(3, "tools/call", install("extra_mod"));
    assert_eq!(refused["isError"], true, "{refused}");
    assert_eq!(fixture.pointer(), pointer);
    assert!(!fixture
        .home
        .join(mod_list::DIR)
        .join("extra_mod.smod")
        .exists());
    fs::write(fixture.home.join("finish"), []).unwrap();
    assert_eq!(play.0.wait().unwrap().code(), Some(7));
    let answer = rpc(4, "tools/call", install("extra_mod"));
    assert_ne!(answer["isError"], true, "{answer}");
    assert_ne!(fixture.pointer(), pointer);
    for (id, bytes) in archives {
        assert_eq!(
            fs::read(fixture.home.join(mod_list::DIR).join(format!("{id}.smod"))).unwrap(),
            bytes
        );
        assert!(sunrise_launcher::browser::mod_list::is_enabled(
            &fixture.home,
            &id
        ));
    }
    drop(to_mcp);
    assert!(mcp.0.wait().unwrap().success());
}

#[test]
fn registry_change_after_proof_cannot_publish_any_candidate_file_or_pointer() {
    let fixture = Fixture::new();
    let (registry, archives) = fixture.content_index();
    fixture.publish_registry(&registry);
    let out = fixture
        .registry_client(&["mods", "install", "root_mod", "--source", "sunrise"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let pointer = fixture.pointer();
    let installed = fixture.home.join(index::browser_state::MANIFEST);
    let rows = fs::read(&installed).unwrap();
    let list = fs::read(fixture.home.join(mod_list::FILE)).unwrap();
    let mut worker = Command::new(std::env::current_exe().unwrap());
    for (key, value) in fixture.registry_client(&[]).get_envs() {
        worker.env(key, value.unwrap());
    }
    let out = worker
        .args(["--exact", "install::cutoff_worker", "--nocapture"])
        .env("FIXTURE_REGISTRY_CUTOFF", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(fixture.pointer(), pointer);
    assert_eq!(fs::read(installed).unwrap(), rows);
    assert_eq!(fs::read(fixture.home.join(mod_list::FILE)).unwrap(), list);
    assert!(!fixture
        .home
        .join(mod_list::DIR)
        .join("extra_mod.smod")
        .exists());
    for id in ["root_mod", "required_mod"] {
        assert_eq!(
            fs::read(fixture.home.join(mod_list::DIR).join(format!("{id}.smod"))).unwrap(),
            archives[id]
        );
    }
}

#[test]
fn install_retries_only_the_checked_conflicting_pair() {
    for (required_range, expected) in [("=2.0.0", "2.0.0"), (">=1.0.0", "1.0.0")] {
        let fixture = Fixture::new();
        let (mut registry, _) = fixture.content_index();
        for (id, peer, range) in [
            ("root_mod", "extra_mod", required_range),
            ("extra_mod", "root_mod", required_range),
        ] {
            let project = registry
                .projects
                .iter_mut()
                .find(|project| project.id == id)
                .unwrap();
            let mut release = project.versions[0].clone();
            release.version = "2.0.0".into();
            release.dependencies = Some(Vec::new());
            let file = fixture.root.join(format!("{id}-2.0.0.smod"));
            let bytes = Package::write_files(&format!(
                "id='{id}'\nname='{}'\nversion='2.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n[incompatible]\n{peer}='{range}'\n", project.name
            ), &[]).unwrap();
            fs::write(&file, &bytes).unwrap();
            release.file = index::File {
                name: file.file_name().unwrap().to_str().unwrap().to_owned(),
                url: format!("file://{}", file.display()),
                size: bytes.len() as u64,
                sha256: index::sha256(&bytes),
                signature: None,
            };
            project.versions.insert(0, release);
        }
        fixture.publish_registry(&registry);
        let mut worker = Command::new(std::env::current_exe().unwrap());
        for (key, value) in fixture.registry_client(&[]).get_envs() {
            worker.env(key, value.unwrap());
        }
        let out = worker
            .args(["--exact", "install::conflicting_pair_worker"])
            .env("FIXTURE_EXPECTED_EXTRA", expected)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        for (id, version) in [("root_mod", "1.0.0"), ("extra_mod", expected)] {
            assert_eq!(
                fs::read(fixture.home.join(mod_list::DIR).join(format!("{id}.smod"))).unwrap(),
                fs::read(fixture.root.join(format!("{id}-{version}.smod"))).unwrap()
            );
        }
    }
}

#[test]
fn conflicting_pair_worker() {
    let Ok(expected) = std::env::var("FIXTURE_EXPECTED_EXTRA") else {
        return;
    };
    use sunrise_launcher::{browser::sunrise::*, instances, managed_play};
    let instance = instances::selection().unwrap();
    let done = install_registry(
        &instance,
        install_target(&instance).unwrap(),
        || vec!["root_mod".into(), "extra_mod".into()],
        || managed_play::headless_first(&instance, None),
        InstallHooks {
            progress: &|_| {},
            native: &|_| panic!("content-only fixture"),
            registry_crates: None,
            continue_after_failure: &|_| panic!("content-only fixture"),
        },
    )
    .unwrap();
    assert!(
        done.added
            .iter()
            .any(|entry| entry.project == "extra_mod" && entry.version == expected),
        "{done:?}"
    );
}

#[test]
fn cutoff_worker() {
    if std::env::var_os("FIXTURE_REGISTRY_CUTOFF").is_none() {
        return;
    }
    use sunrise_launcher::{browser::sunrise::*, instances, managed_play};
    let path = PathBuf::from(std::env::var_os("SUNRISE_MOD_INDEX").unwrap());
    let instance = instances::selection().unwrap();
    let result = install_registry(
        &instance,
        install_target(&instance).unwrap(),
        || vec!["extra_mod".into()],
        || managed_play::headless_first(&instance, None),
        InstallHooks {
            progress: &|step| {
                if step == InstallProgress::Publishing {
                    let mut now: index::Index =
                        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                    now.revoked.push(index::Revoked {
                        id: "extra_mod".into(),
                        sha256: None,
                        reason: "Removed while proving the batch".into(),
                        date: "2026-11-20".into(),
                    });
                    fs::write(&path, serde_json::to_vec(&now).unwrap()).unwrap();
                }
            },
            native: &|_| panic!("content-only fixture"),
            registry_crates: None,
            continue_after_failure: &|_| panic!("content-only fixture"),
        },
    );
    assert!(
        matches!(&result, Err(InstallError::Unproved(text)) if text.contains("registry changed")),
        "{result:?}"
    );
}
