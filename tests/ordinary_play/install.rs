//! Stage and publish real registry archives through the CLI and generated MCP command.
use super::*;
use std::collections::BTreeMap;
use sunrise_launcher::{
    browser::sunrise::*,
    instances,
    managed_play::{Active, HeldSource, Profile},
    native_mods::Cancellation,
};
use sunrise_launcher_nbt as nbt;
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
            let definitions: &[(&str, &[u8])] = if id == "required_mod" {
                &[(
                    "data/required_mod/sunrise/block/core.json",
                    br#"{"properties":{"lit":[false,true]}}"#,
                )]
            } else {
                &[]
            };
            let archive = Package::write_files(
                &format!("id='{id}'\nname='{name}'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n{dependencies}"),
                definitions,
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
        || managed_play::headless_first(&instance),
        InstallHooks {
            progress: &|_| {},
            native: &|_| panic!("content-only fixture"),
            registry_crates: None,
            continue_after_failure: &|_| panic!("content-only fixture"),
            resolve: None,
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
        || managed_play::headless_first(&instance),
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
            resolve: None,
        },
    );
    assert!(
        matches!(&result, Err(InstallError::Unproved(text)) if text.contains("registry changed")),
        "{result:?}"
    );
}

const OTHER: &str = "developer-other";

impl Fixture {
    fn other_build(&self) {
        let folder = self.data.join("versions").join(OTHER);
        let resources = folder.join(".local/sunrise-client/26.4");
        fs::create_dir_all(&resources).unwrap();
        fs::write(folder.join(".installed"), []).unwrap();
        fs::write(
            folder.join("developer.json"),
            json!({"revision":"b".repeat(40)}).to_string(),
        )
        .unwrap();
        fs::write(
            folder.join("research-lock.json"),
            r#"{"minecraft":{"version":"26.4"}}"#,
        )
        .unwrap();
        fs::write(
            resources.join("sunrise-resources.json"),
            r#"{"data_version":4100}"#,
        )
        .unwrap();
        script(&folder.join("sunrise-client"), "#!/bin/sh\nexit 99\n");
    }

    fn install_mod(&self, id: &str) {
        let result = self
            .registry_client(&["mods", "install", id, "--source", "sunrise"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    fn decision_worker(&self, mode: &str) {
        let mut worker = Command::new(std::env::current_exe().unwrap());
        for (key, value) in self.registry_client(&[]).get_envs() {
            worker.env(key, value.unwrap());
        }
        let out = worker
            .args(["--exact", "install::profile_decision_worker", "--nocapture"])
            .env("FIXTURE_PROFILE_DECISION", mode)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn new_version(
        &self,
        project: &index::Project,
        version: &str,
        game: &str,
        need: Option<&str>,
    ) -> (index::Version, Vec<u8>) {
        let dependency = need
            .map(|range| format!("[dependencies]\nrequired_mod='{range}'"))
            .unwrap_or_default();
        let definitions: &[(&str, &[u8])] = if project.id == "required_mod" {
            &[(
                "data/required_mod/sunrise/block/core.json",
                br#"{"properties":{}}"#,
            )]
        } else {
            &[("data/lantern/function/lit.mcfunction", b"say lit")]
        };
        let bytes = Package::write_files(
            &format!("id='{}'\nname='{}'\nversion='{version}'\napi='{}'\nside='both'\ncapabilities=['registries']\n{dependency}", project.id, project.name, manifest::API),
            definitions,
        ).unwrap();
        let name = format!("{}-{version}.smod", project.id);
        let path = self.root.join(&name);
        fs::write(&path, &bytes).unwrap();
        let mut row = project.versions[0].clone();
        row.version = version.into();
        row.game = game.into();
        row.dependencies = Some(
            need.into_iter()
                .map(|range| index::Dependency {
                    id: "required_mod".into(),
                    version: range.into(),
                    optional: false,
                    side: None,
                })
                .collect(),
        );
        row.file = index::File {
            name,
            url: url::Url::from_file_path(path).unwrap().into(),
            size: bytes.len() as u64,
            sha256: index::sha256(&bytes),
            signature: None,
        };
        (row, bytes)
    }
}

fn write_world(home: &Path, version: i32) -> PathBuf {
    use nbt::Tag;
    let save = home.join("saves/Glass Basin");
    fs::create_dir_all(&save).unwrap();
    let path = save.join("level.dat");
    let bytes = nbt::write_named(
        &nbt::NamedTag {
            name: "".into(),
            tag: Tag::compound([
                (
                    "Data",
                    Tag::compound([
                        ("LevelName", Tag::String("Glass Basin".into())),
                        ("DataVersion", Tag::Int(version)),
                    ]),
                ),
                (
                    "sunrise",
                    Tag::compound([(
                        "BlockNamespaces",
                        Tag::List(vec![Tag::String("lantern".into())]),
                    )]),
                ),
            ]),
        },
        nbt::Limits::default(),
    )
    .unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&bytes).unwrap();
    fs::write(&path, gz.finish().unwrap()).unwrap();
    path
}

#[test]
fn switch_holds_content_in_place_and_an_override_cannot_replace_it() {
    for initial_enabled in [true, false] {
        let fixture = Fixture::new();
        let (mut registry, archives) = fixture.content_index();
        fixture.publish_registry(&registry);
        fixture.other_build();
        fixture.install_mod("extra_mod");
        fixture.decision_worker("preview");
        let package = fixture.home.join(mod_list::DIR).join("extra_mod.smod");
        if !initial_enabled {
            sunrise_launcher::browser::mod_list::set_enabled(&fixture.home, "extra_mod", false)
                .unwrap();
        }
        let pointer = fixture.pointer();
        if initial_enabled {
            let world = write_world(&fixture.home, 4200);
            let bytes = fs::read(&world).unwrap();
            let refused = fixture
                .registry_client(&["switch", OTHER])
                .output()
                .unwrap();
            assert!(!refused.status.success());
            assert!(
                String::from_utf8_lossy(&refused.stderr).contains("run Switch in a terminal"),
                "{refused:?}"
            );
            for decision in [
                "switch-stay",
                "switch-changed-world",
                "switch-changed-index",
            ] {
                fixture.decision_worker(decision);
                assert_eq!(fixture.pointer(), pointer);
                assert_eq!(fs::read(&package).unwrap(), archives["extra_mod"]);
                assert!(sunrise_launcher::browser::mod_list::is_enabled(
                    &fixture.home,
                    "extra_mod"
                ));
                fs::write(&world, &bytes).unwrap();
                fixture.publish_registry(&registry);
            }
            fixture.decision_worker("switch-accept");
            assert_eq!(fs::read(&world).unwrap(), bytes);
            // Isolate replacement authority from the separate saved-world decision.
            fs::rename(world.parent().unwrap(), fixture.root.join("verified-save")).unwrap();
        } else {
            let switched = fixture
                .registry_client(&["--json", "switch", OTHER])
                .output()
                .unwrap();
            assert!(switched.status.success(), "{switched:?}");
            assert_eq!(
                serde_json::from_slice::<Value>(&switched.stdout).unwrap()["build"],
                OTHER
            );
        }
        let active = Active::parse(&fixture.pointer()).unwrap();
        assert_eq!(active.base(), OTHER);
        assert_eq!(active.generation(), 2);
        let hold = active.holds().next().unwrap();
        assert_eq!(hold.path(), Path::new(mod_list::DIR).join("extra_mod.smod"));
        assert_eq!(hold.enabled_before(), initial_enabled);
        assert!(matches!(hold.source(), HeldSource::Local { version } if version == "1.0.0"));
        assert_eq!(fs::read(&package).unwrap(), archives["extra_mod"]);
        assert!(!sunrise_launcher::browser::mod_list::is_enabled(
            &fixture.home,
            "extra_mod"
        ));
        let receipt = active.pending_changes().unwrap().id;
        if !initial_enabled {
            fixture.decision_worker("ack");
            assert!(Active::parse(&fixture.pointer())
                .unwrap()
                .pending_changes()
                .is_none());
        }
        assert_eq!(
            Active::parse(&fixture.pointer()).unwrap().holds().count(),
            1
        );
        let pointer = fixture.pointer();
        let enable = fixture
            .registry_client(&["mods", "enable", "extra_mod"])
            .output()
            .unwrap();
        assert!(!enable.status.success(), "{enable:?}");
        assert!(String::from_utf8_lossy(&enable.stderr).contains("Needs an update from its author"));
        if !initial_enabled {
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
            let refused = mcp_request(
                &mut input,
                &mut output,
                2,
                "tools/call",
                json!({"name":"mods_enable", "arguments":{"project":"extra_mod"}}),
            );
            assert_eq!(refused["isError"], true, "{refused}");
            assert!(
                refused
                    .to_string()
                    .contains("Needs an update from its author"),
                "{refused}"
            );
            drop(input);
            assert!(mcp.0.wait().unwrap().success());
        }
        assert_eq!(fixture.pointer(), pointer);
        assert!(!sunrise_launcher::browser::mod_list::is_enabled(
            &fixture.home,
            "extra_mod"
        ));

        let project = registry
            .projects
            .iter_mut()
            .find(|p| p.id == "extra_mod")
            .unwrap();
        let (row, _) = fixture.new_version(project, "2.0.0", "26.4", None);
        project.versions.insert(0, row);
        fixture.publish_registry(&registry);
        let snapshot = fixture.pointer();
        let update = fixture
            .registry_client(&["mods", "install", "extra_mod", "--source", "sunrise"])
            .output()
            .unwrap();
        assert!(
            !update.status.success(),
            "a test index cannot authorize replacing held bytes: {update:?}"
        );
        assert_eq!(fixture.pointer(), snapshot);
        let updated = Active::parse(&snapshot).unwrap();
        assert_eq!(updated.base(), OTHER);
        assert_eq!(updated.holds().count(), 1);
        if initial_enabled {
            let report = updated.pending_changes().unwrap();
            assert_eq!(report.id, receipt);
            assert_eq!(report.base(), Some(OTHER));
            assert_eq!(
                report.held().collect::<Vec<_>>(),
                [("Fireflies", true, false)]
            );
        } else {
            assert!(updated.pending_changes().is_none());
        }
        assert_eq!(fs::read(&package).unwrap(), archives["extra_mod"]);
        assert!(!sunrise_launcher::browser::mod_list::is_enabled(
            &fixture.home,
            "extra_mod"
        ));
        let shown = fixture
            .launcher(&["--json", "instance", "show"])
            .output()
            .unwrap();
        assert!(shown.status.success(), "{shown:?}");
        assert_eq!(
            serde_json::from_slice::<Value>(&shown.stdout).unwrap()["build"],
            OTHER
        );
        let wrong = fixture
            .launcher(&["native-build", "--version", BUILD])
            .output()
            .unwrap();
        assert!(!wrong.status.success());
        assert!(
            String::from_utf8_lossy(&wrong.stderr).contains("Use Switch"),
            "{wrong:?}"
        );
        assert_eq!(fixture.pointer(), serde_json::to_vec(&updated).unwrap());
    }
}

#[test]
fn switch_keeps_an_unlisted_exact_content_archive_that_fits() {
    let fixture = Fixture::new();
    let (registry, _) = fixture.content_index();
    fixture.publish_registry(&registry);
    fixture.other_build();
    fixture.install_mod("extra_mod");
    let path = fixture.home.join(mod_list::DIR).join("extra_mod.smod");
    let private = Package::write_files(&format!("id='extra_mod'\nname='Private Fireflies'\nversion='2'\napi='{}'\ncapabilities=['registries']", manifest::API), &[]).unwrap();
    fs::write(&path, &private).unwrap();
    let switched = fixture
        .registry_client(&["--json", "switch", OTHER])
        .output()
        .unwrap();
    assert!(switched.status.success(), "{switched:?}");
    let active = Active::parse(&fixture.pointer()).unwrap();
    assert_eq!(active.base(), OTHER);
    assert!(active.holds().next().is_none());
    assert!(sunrise_launcher::browser::mod_list::is_enabled(
        &fixture.home,
        "extra_mod"
    ));
    assert_eq!(fs::read(path).unwrap(), private);
}

#[test]
fn switch_warns_about_saved_definitions_even_when_its_target_uses_the_same_archive() {
    let fixture = Fixture::new();
    let (mut registry, archives) = fixture.content_index();
    fixture.publish_registry(&registry);
    fixture.other_build();
    fixture.install_mod("extra_mod");
    let extra = registry
        .projects
        .iter_mut()
        .find(|p| p.id == "extra_mod")
        .unwrap();
    extra.versions[0].game = "26.4".into();
    fixture.publish_registry(&registry);
    let world = write_world(&fixture.home, 4000);
    let bytes = fs::read(&world).unwrap();
    let before = fixture.pointer();
    fixture.decision_worker("switch-same-archive-stay");
    assert_eq!(fixture.pointer(), before);
    assert_eq!(fs::read(world).unwrap(), bytes);
    assert_eq!(
        fs::read(fixture.home.join(mod_list::DIR).join("extra_mod.smod")).unwrap(),
        archives["extra_mod"]
    );
}

#[test]
fn switch_takedowns_never_delete_old_files_on_the_word_of_an_override() {
    for (github, restamped) in [(false, false), (true, true)] {
        let fixture = Fixture::new();
        let (mut registry, archives) = fixture.content_index();
        fixture.publish_registry(&registry);
        fixture.other_build();
        fixture.install_mod("root_mod");
        let mods = fixture.home.join(mod_list::DIR);
        let document = fixture.home.join(index::browser_state::MANIFEST);
        let mut rows: Value = serde_json::from_slice(&fs::read(&document).unwrap()).unwrap();
        let entries = rows["entries"].as_array_mut().unwrap();
        // The override quotes the actual digest of an untracked installation below.
        entries.retain(|row| row["project"] != "required_mod");
        let fork = Package::write_files(
            &format!("id='root_mod'\nname='Private Lanterns'\nversion='1.0.0'\napi='{}'\ncapabilities=['registries']\n[dependencies]\nrequired_mod='^1'", manifest::API),
            &[("data/private/function/light.mcfunction", b"say private")],
        ).unwrap();
        let row = entries
            .iter_mut()
            .find(|row| row["project"] == "root_mod")
            .unwrap();
        if restamped {
            row["checksum"] = json!(format!("SHA-256:{}", index::sha256(&fork)));
        }
        fs::write(&document, serde_json::to_vec(&rows).unwrap()).unwrap();
        let fork_path = mods.join("root_mod.smod");
        fs::write(&fork_path, &fork).unwrap();
        let untracked = mods.join("extra_mod.smod");
        fs::write(&untracked, &archives["extra_mod"]).unwrap();
        let origin = github.then(|| {
            let origin = fixture
                .home
                .join(mod_list::NATIVE_DIR)
                .join("extra_mod.origin.json");
            fs::create_dir_all(origin.parent().unwrap()).unwrap();
            let bytes = serde_json::to_vec(&sunrise_launcher::github_mods::Origin {
                id: "extra_mod".into(),
                version: "1.0.0".into(),
                owner: "example".into(),
                repo: "extra".into(),
                tag: None,
                commit: "c".repeat(40),
                asset: None,
                sha256: index::sha256(&archives["extra_mod"]),
            })
            .unwrap();
            fs::write(&origin, &bytes).unwrap();
            (origin, bytes)
        });
        registry.projects.retain(|project| project.id != "root_mod");
        for (id, sha256) in [
            ("root_mod", None),
            ("extra_mod", None),
            (
                "required_mod",
                Some(index::sha256(&archives["required_mod"])),
            ),
        ] {
            registry.revoked.push(index::Revoked {
                id: id.into(),
                sha256,
                reason: "Fixture takedown".into(),
                date: "2026-11-20".into(),
            });
        }
        fixture.publish_registry(&registry);
        let result = fixture
            .registry_client(&["--json", "switch", OTHER])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let active = Active::parse(&fixture.pointer()).unwrap();
        assert!(active.pending_changes().unwrap().removed.is_empty());
        let holds = active.holds().collect::<Vec<_>>();
        assert_eq!(
            holds.iter().map(|hold| hold.id()).collect::<Vec<_>>(),
            ["required_mod", "root_mod"]
        );
        assert!(holds
            .iter()
            .all(|hold| matches!(hold.source(), HeldSource::Local { .. })));
        assert_eq!(fs::read(fork_path).unwrap(), fork);
        assert_eq!(fs::read(untracked).unwrap(), archives["extra_mod"]);
        assert_eq!(
            fs::read(mods.join("required_mod.smod")).unwrap(),
            archives["required_mod"]
        );
        if let Some((origin, bytes)) = origin {
            assert_eq!(fs::read(origin).unwrap(), bytes);
        }
        let latest_rows: Value = serde_json::from_slice(&fs::read(document).unwrap()).unwrap();
        assert_eq!(
            latest_rows, rows,
            "an override neither grants nor rewrites origin"
        );
    }
}

#[test]
fn switch_uses_short_game_versions_and_refuses_an_unindexed_installed_project() {
    let fixture = Fixture::new();
    let (mut registry, _) = fixture.content_index();
    let core = registry
        .projects
        .iter_mut()
        .find(|p| p.id == "required_mod")
        .unwrap();
    let (core_row, core_bytes) = fixture.new_version(core, "2", "26.3", None);
    core.versions = vec![core_row];
    let root = registry
        .projects
        .iter_mut()
        .find(|p| p.id == "root_mod")
        .unwrap();
    let (root_row, root_bytes) = fixture.new_version(root, "2.1", "26.3", Some("^2"));
    root.versions = vec![root_row];
    fixture.publish_registry(&registry);
    fixture.other_build();
    fixture.install_mod("root_mod");
    let mut core = registry.projects.remove(
        registry
            .projects
            .iter()
            .position(|p| p.id == "required_mod")
            .unwrap(),
    );
    registry
        .projects
        .iter_mut()
        .find(|p| p.id == "root_mod")
        .unwrap()
        .versions[0]
        .game = "26.4".into();
    fixture.publish_registry(&registry);
    let before = fixture.pointer();
    let switch = || {
        fixture
            .registry_client(&["--json", "switch", OTHER])
            .output()
            .unwrap()
    };
    let refused = switch();
    assert!(!refused.status.success(), "{refused:?}");
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("couldn't verify the full mod set"),
        "{refused:?}"
    );
    let mods = fixture.home.join(mod_list::DIR);
    assert_eq!(fixture.pointer(), before);
    assert_eq!(
        fs::read(mods.join("required_mod.smod")).unwrap(),
        core_bytes
    );
    core.versions[0].game = "26.4".into();
    registry.projects.push(core);
    fixture.publish_registry(&registry);
    let switched = switch();
    assert!(
        switched.status.success(),
        "{}",
        String::from_utf8_lossy(&switched.stderr)
    );
    let active = Active::parse(&fixture.pointer()).unwrap();
    assert_eq!(active.base(), OTHER);
    assert_eq!(active.holds().count(), 0);
    assert_eq!(
        fs::read(mods.join("required_mod.smod")).unwrap(),
        core_bytes
    );
    assert_eq!(fs::read(mods.join("root_mod.smod")).unwrap(), root_bytes);
}

#[test]
fn resolve_default_and_stale_approval_keep_exact_bytes_then_only_proved_blocker_turns_off() {
    let fixture = Fixture::new();
    let (mut registry, archives) = fixture.content_index();
    fixture.publish_registry(&registry);
    fixture.install_mod("root_mod");
    // Installing a new mod can require disabling a blocker without replacing any
    // old archive. This fixture deliberately uses a nonofficial registry override.
    let project = registry
        .projects
        .iter_mut()
        .find(|p| p.id == "extra_mod")
        .unwrap();
    let row = &mut project.versions[0].file;
    let extra = Package::write_files(
        &format!("id='extra_mod'\nname='Fireflies'\nversion='1.0.0'\napi='{}'\nside='both'\ncapabilities=['registries']\n[incompatible]\nroot_mod='^1'", manifest::API),
        &[],
    ).unwrap();
    fs::write(fixture.root.join(&row.name), &extra).unwrap();
    row.sha256 = index::sha256(&extra);
    row.size = extra.len() as u64;
    fixture.publish_registry(&registry);
    let world = write_world(&fixture.home, 4200);
    let saves = fs::read(&world).unwrap();
    let pointer = fixture.pointer();
    let list = fs::read(fixture.home.join(mod_list::FILE)).unwrap();
    let rows = fs::read(fixture.home.join(index::browser_state::MANIFEST)).unwrap();
    let mods = fixture.home.join(mod_list::DIR);
    let headless = fixture
        .registry_client(&["mods", "install", "extra_mod", "--source", "sunrise"])
        .output()
        .unwrap();
    assert!(!headless.status.success(), "{headless:?}");
    for decision in [
        "resolve-stay",
        "resolve-changed-world",
        "resolve-changed-index",
    ] {
        fixture.decision_worker(decision);
        assert_eq!(fixture.pointer(), pointer);
        assert_eq!(fs::read(fixture.home.join(mod_list::FILE)).unwrap(), list);
        assert_eq!(
            fs::read(fixture.home.join(index::browser_state::MANIFEST)).unwrap(),
            rows
        );
        assert_eq!(
            fs::read(mods.join("root_mod.smod")).unwrap(),
            archives["root_mod"]
        );
        assert_eq!(
            fs::read(mods.join("required_mod.smod")).unwrap(),
            archives["required_mod"]
        );
        assert!(!mods.join("extra_mod.smod").exists());
        fs::write(&world, &saves).unwrap();
        fixture.publish_registry(&registry);
    }
    fixture.decision_worker("resolve-accept");
    let active = Active::parse(&fixture.pointer()).unwrap();
    assert_eq!(active.base(), BUILD);
    assert_eq!(active.generation(), 2);
    assert_eq!(
        fs::read(mods.join("root_mod.smod")).unwrap(),
        archives["root_mod"]
    );
    assert_eq!(
        fs::read(mods.join("required_mod.smod")).unwrap(),
        archives["required_mod"]
    );
    assert_eq!(fs::read(mods.join("extra_mod.smod")).unwrap(), extra);
    assert!(!sunrise_launcher::browser::mod_list::is_enabled(
        &fixture.home,
        "root_mod"
    ));
    assert!(sunrise_launcher::browser::mod_list::is_enabled(
        &fixture.home,
        "required_mod"
    ));
    assert!(sunrise_launcher::browser::mod_list::is_enabled(
        &fixture.home,
        "extra_mod"
    ));
    assert_eq!(fs::read(world).unwrap(), saves);
}

#[test]
fn queued_switch_refuses_a_replaced_profile_before_recovery() {
    let fixture = Fixture::new();
    fixture.decision_worker("switch-replaced-profile");
    assert!(!fixture.home.join(".install-txn.lock").exists());
    assert!(!fixture.home.join("mod-browser/active-build.json").exists());
}

#[test]
fn profile_decision_worker() {
    let Ok(mode) = std::env::var("FIXTURE_PROFILE_DECISION") else {
        return;
    };
    let instance = instances::selection().unwrap();
    let identity = install_target(&instance).unwrap();
    if mode == "switch-replaced-profile" {
        fs::rename(&instance.home, instance.home.with_extension("old")).unwrap();
        fs::create_dir(&instance.home).unwrap();
        let result = switch_build(
            &instance,
            identity,
            OTHER,
            SwitchHooks {
                cancellation: &Cancellation::default(),
                progress: &|_| {},
                registry_crates: None,
                approve: &|_| panic!("replacement was admitted"),
                at_cutoff: &|| panic!("replacement reached publication"),
            },
        );
        assert!(result.err().unwrap().contains("profile was replaced"));
        return;
    }
    if mode == "switch-native-catalogue-mismatch" {
        sunrise_launcher::backend::Settings::load()
            .allow_native_mods()
            .unwrap();
        let trials = std::cell::Cell::new(0);
        let result = switch_build(
            &instance,
            identity,
            OTHER,
            SwitchHooks {
                cancellation: &Cancellation::default(),
                progress: &|step| {
                    if matches!(step, InstallProgress::Preparing) {
                        trials.set(trials.get() + 1);
                    }
                },
                registry_crates: None,
                approve: &|_| panic!("no worlds in the fixture"),
                at_cutoff: &|| Ok(()),
            },
        );
        assert!(matches!(result.unwrap(), Switched::Changed(_)));
        assert_eq!(trials.get(), 2, "try native against the exact target, then keep the archive off only after a proved revision mismatch");
        return;
    }
    let path = PathBuf::from(std::env::var_os("SUNRISE_MOD_INDEX").unwrap());
    let approved = std::cell::Cell::new(0);
    let decide = || {
        approved.set(approved.get() + 1);
        if mode.ends_with("changed-index") {
            let mut bytes = fs::read(&path).unwrap();
            bytes.push(b' ');
            fs::write(&path, bytes).unwrap();
        }
        if mode.ends_with("changed-world") {
            write_world(&instance.home, 4300);
        }
        !mode.ends_with("stay")
    };
    if mode == "preview" {
        use sunrise_launcher::browser::sunrise::checked_switch_fit;
        assert!(checked_switch_fit(&instance.home, BUILD));
        assert!(!checked_switch_fit(&instance.home, OTHER));
        return;
    }
    if mode == "ack" {
        let mut owner = Profile::open(&instance.home, None).unwrap();
        let active = owner.active().unwrap().unwrap();
        assert!(owner
            .acknowledge_receipt(&active, active.pending_changes().unwrap().id)
            .unwrap());
        return;
    }
    if mode.starts_with("switch-") {
        let cutoff = std::cell::Cell::new(0);
        let result = switch_build(
            &instance,
            identity,
            OTHER,
            SwitchHooks {
                cancellation: &Cancellation::default(),
                progress: &|_| {},
                registry_crates: None,
                approve: &|q| {
                    if mode == "switch-same-archive-stay" {
                        assert!(q.held.is_empty());
                        assert!(q.worlds.newer.is_empty());
                        assert!(q.worlds.unknown_order.is_empty());
                        assert_eq!(q.worlds.affected[0].mods, ["Fireflies"]);
                    } else {
                        assert_eq!(q.held, ["Fireflies"]);
                        assert_eq!(q.worlds.newer, ["Glass Basin"]);
                    }
                    assert_eq!(q.worlds.affected[0].world, "Glass Basin");
                    Ok(decide())
                },
                at_cutoff: &|| {
                    cutoff.set(cutoff.get() + 1);
                    Ok(())
                },
            },
        );
        match mode.as_str() {
            "switch-stay" | "switch-same-archive-stay" => {
                assert!(matches!(result, Ok(Switched::Stayed)))
            }
            "switch-changed-index" => assert!(result.err().unwrap().contains("registry changed")),
            "switch-changed-world" => assert!(result.err().unwrap().contains("worlds changed")),
            "switch-accept" => assert!(matches!(result, Ok(Switched::Changed(_)))),
            _ => unreachable!(),
        }
        assert_eq!(cutoff.get(), u32::from(mode == "switch-accept"));
    } else {
        let result = install_registry(
            &instance,
            sunrise_launcher::browser::sunrise::install_target(&instance).unwrap(),
            || vec!["extra_mod".into()],
            || panic!("already bound"),
            InstallHooks {
                progress: &|_| {},
                native: &|_| panic!("content only"),
                registry_crates: None,
                continue_after_failure: &|_| panic!("content only"),
                resolve: Some(&|q| {
                    assert_eq!(q.requested, ["Fireflies"]);
                    assert_eq!(q.blockers.len(), 1);
                    assert_eq!(q.blockers[0].0, "Lanterns");
                    assert!(q.blockers[0].1.contains("Fireflies 1.0.0"), "{q:?}");
                    assert_eq!(q.worlds.affected[0].world, "Glass Basin");
                    assert_eq!(
                        q.worlds.affected[0].mods,
                        ["Lanterns"],
                        "unchanged independent definitions don't add to the question: {q:?}"
                    );
                    decide()
                }),
            },
        );
        match mode.as_str() {
            "resolve-stay" => assert!(matches!(result, Err(InstallError::Cancelled)), "{result:?}"),
            "resolve-changed-index" => {
                assert!(result.unwrap_err().to_string().contains("registry changed"))
            }
            "resolve-changed-world" => {
                assert!(result.unwrap_err().to_string().contains("worlds changed"))
            }
            "resolve-accept" => assert!(result.is_ok(), "{result:?}"),
            _ => unreachable!(),
        }
    }
    assert_eq!(approved.get(), 1);
}
