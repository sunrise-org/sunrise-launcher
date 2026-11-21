//! A proved native no-fit can change the verified closure before any compiler work.
use super::*;

#[test]
fn a_different_native_dependency_needs_a_new_answer_after_no_fit() {
    let fixture = Fixture::new();
    let mut registry = fixture.registry();
    let template = registry.project("coords_hud").unwrap().clone();
    registry.projects.clear();
    for (id, name, code, releases) in [
        (
            "first_light",
            "First Light",
            Some('b'),
            vec![("1.0.0", vec![])],
        ),
        ("new_light", "New Light", Some('a'), vec![("1.0.0", vec![])]),
        (
            "shared_light",
            "Shared Light",
            Some('a'),
            vec![("1.0.0", vec![])],
        ),
        (
            "root_mod",
            "Lanterns",
            None,
            vec![
                ("2.0.0", vec!["first_light", "shared_light"]),
                ("1.0.0", vec!["new_light", "shared_light"]),
            ],
        ),
    ] {
        let kind = if code.is_some() {
            index::Kind::NativeMod
        } else {
            index::Kind::Mod
        };
        let mut versions = Vec::new();
        for (version, dependencies) in releases {
            let mut manifest = format!("id='{id}'\nname='{name}'\nversion='{version}'\napi='0.2'\nside='both'\ncapabilities=['registries']\n");
            if !dependencies.is_empty() {
                manifest.push_str("[dependencies]\n");
                for dep in &dependencies {
                    manifest.push_str(&format!("{dep}='^1'\n"));
                }
            }
            let cargo = format!("[package]\nname='{id}'\nversion='{version}'\n");
            let files: &[(&str, &[u8])] = if let Some(pin) = code {
                manifest.push_str(&format!(
                    "[native]\nsunrise='{}'\nmixins='{}'\ngame='26.3'\n",
                    pin.to_string().repeat(40),
                    "c".repeat(40)
                ));
                &[("Cargo.toml", cargo.as_bytes()), ("src/lib.rs", b"")]
            } else {
                &[]
            };
            let bytes = Package::write_files(&manifest, files).unwrap();
            let filename = format!("{id}-{version}.smod");
            let file = fixture.root.join(&filename);
            fs::write(&file, &bytes).unwrap();
            let facts = index::inspect(&kind, &file).unwrap();
            let mut row = template.versions[0].clone();
            row.version = version.into();
            row.game = "26.3".into();
            row.api = facts.api;
            row.side = facts.side;
            row.capabilities = serde_json::from_value(json!(facts.capabilities)).unwrap();
            row.dependencies = Some(
                dependencies
                    .into_iter()
                    .map(|id| index::Dependency {
                        id: id.into(),
                        version: "^1".into(),
                        optional: false,
                        side: None,
                    })
                    .collect(),
            );
            row.file = index::File {
                name: filename,
                url: format!("file://{}", file.display()),
                size: bytes.len() as u64,
                sha256: index::sha256(&bytes),
                signature: None,
            };
            versions.push(row);
        }
        registry.projects.push(index::Project {
            id: id.into(),
            name: name.into(),
            kind,
            key: None,
            versions,
            ..template.clone()
        });
    }
    fixture.publish_registry(&registry);
    let tool = fixture.root.join("fixture-tool");
    script(
        &tool,
        "#!/bin/sh\ntouch \"$FIXTURE_UNEXPECTED_WORK\"\nexit 7\n",
    );
    let tool_ran = fixture.root.join("tool-ran");
    let mut worker = Command::new(std::env::current_exe().unwrap());
    for (key, value) in fixture.registry_client(&[]).get_envs() {
        worker.env(key, value.unwrap());
    }
    let report = worker
        .args(["--exact", "install::native_retry::retry_worker"])
        .env("FIXTURE_UNEXPECTED_WORK", &tool_ran)
        .env("SUNRISE_LAUNCHER_DEVELOPER_TOOL", &tool)
        .env("SUNRISE_LAUNCHER_DEVELOPER_SDK", &fixture.root)
        .output()
        .unwrap();
    assert!(
        report.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&report.stdout),
        String::from_utf8_lossy(&report.stderr)
    );
    assert!(!tool_ran.exists());
    assert!(!fixture.home.join(index::browser_state::MANIFEST).exists());
    assert!(!fixture.home.join(mod_list::DIR).exists());
    assert!(!fixture.home.join(mod_list::NATIVE_DIR).exists());
    assert!(!fixture.home.join("mod-browser/active-build.json").exists());
}

#[test]
fn retry_worker() {
    if std::env::var_os("FIXTURE_UNEXPECTED_WORK").is_none() {
        return;
    }
    use sunrise_launcher::{backend::Settings, browser::sunrise::*, instances, managed_play};
    let instance = instances::selection().unwrap();
    let questions = std::cell::RefCell::new(Vec::new());
    let done = install_registry(
        &instance,
        install_target(&instance).unwrap(),
        || vec!["root_mod".into()],
        || managed_play::headless_first(&instance, None),
        InstallHooks {
            progress: &|_| {},
            native: &|q| {
                questions.borrow_mut().push((q.retry, q.names.clone()));
                if q.retry {
                    return Ok(false);
                }
                Settings::load().allow_native_mods()?;
                Ok(true)
            },
            registry_crates: None,
            continue_after_failure: &|_| panic!("no compile can run"),
        },
    );
    assert!(matches!(done, Err(InstallError::Cancelled)), "{done:?}");
    assert_eq!(
        questions.into_inner(),
        [
            (
                false,
                vec!["First Light".to_owned(), "Shared Light".to_owned()]
            ),
            (true, vec!["New Light".to_owned()])
        ]
    );
}
