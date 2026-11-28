//! Recovery of an interrupted same-tag install runs before the CLI sees installed versions.
use std::{fs, path::Path, process::Command};

const TAG: &str = "nightly-2026-11-20-aaaaaaa";

fn installed_on_restart(home: &Path) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_sunrise-launcher"))
        .args(["--json", "versions", "--installed"])
        .current_dir(home)
        .env("SUNRISE_LAUNCHER_HOME", home.join("launcher"))
        .env("SUNRISE_HOME", home.join("game"))
        .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
        .env("SUNRISE_TELEMETRY_URL", "off")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn restarting_restores_the_previous_build_until_the_replacement_is_complete() {
    // The three disk cuts are after detaching the old tree, after publishing the new tree,
    // and after committing it. No catalogue (including an offer of this exact Nightly) is needed.
    for (new_on_disk, committed) in [(false, false), (true, false), (true, true)] {
        let home = tempfile::tempdir().unwrap();
        let launcher = home.path().join("launcher");
        let versions = launcher.join("versions");
        let repair = versions.join(format!(".repair-{TAG}"));
        let previous = repair.join("previous");
        let target = versions.join(TAG);
        fs::create_dir_all(&previous).unwrap();
        fs::write(previous.join(".installed"), "").unwrap();
        fs::write(previous.join("origin"), "before").unwrap();
        if new_on_disk {
            fs::create_dir(&target).unwrap();
            fs::write(target.join("origin"), "after").unwrap();
            if committed {
                fs::write(target.join(".installed"), "").unwrap();
            }
        }
        let profiles = serde_json::json!({"instances":{"sunrise":{"version":TAG}}}).to_string();
        fs::write(launcher.join("instances.json"), &profiles).unwrap();
        for _ in 0..2 {
            let data = installed_on_restart(home.path());
            assert_eq!(data["installed"], serde_json::json!([TAG]));
            assert_eq!(data["selected"], TAG);
            assert_eq!(
                fs::read_to_string(target.join("origin")).unwrap(),
                if committed { "after" } else { "before" }
            );
            assert!(!repair.exists(), "successful recovery clears the marker");
            assert_eq!(
                fs::read_to_string(launcher.join("instances.json")).unwrap(),
                profiles
            );
        }
    }
}

#[test]
fn restarting_recovers_an_incomplete_predecessor_without_marking_it_installed() {
    for (detached, uncommitted) in [
        (false, None),
        (true, None),
        (true, Some(true)),
        (true, Some(false)),
    ] {
        let home = tempfile::tempdir().unwrap();
        let versions = home.path().join("launcher/versions");
        let repair = versions.join(format!(".repair-{TAG}"));
        let target = versions.join(TAG);
        let previous = repair.join("previous");
        fs::create_dir_all(&repair).unwrap();
        let old = if detached { &previous } else { &target };
        fs::create_dir(old).unwrap();
        fs::write(old.join("origin"), "before").unwrap();
        if let Some(visible) = uncommitted {
            let candidate = if visible {
                target.clone()
            } else {
                repair.join("incomplete")
            };
            fs::create_dir(&candidate).unwrap();
            fs::write(candidate.join("origin"), "uncommitted").unwrap();
        }
        let data = installed_on_restart(home.path());
        assert_eq!(data["installed"], serde_json::json!([]));
        assert_eq!(fs::read_to_string(target.join("origin")).unwrap(), "before");
        assert!(!target.join(".installed").exists());
        assert!(!repair.exists());
    }
}

#[test]
fn next_install_recovers_before_requiring_a_download() {
    const ACTOR: &str = "SUNRISE_TEST_REATTACH_ON_INSTALL";
    if std::env::var_os(ACTOR).is_none() {
        let home = tempfile::tempdir().unwrap();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "next_install_recovers_before_requiring_a_download"
            ])
            .env(ACTOR, "1")
            .env("SUNRISE_LAUNCHER_HOME", home.path().join("launcher"))
            .env("SUNRISE_HOME", home.path().join("game"))
            .status()
            .unwrap()
            .success());
        return;
    }
    use sunrise_launcher::backend;
    let repair = backend::data_dir()
        .join("versions")
        .join(format!(".repair-{TAG}"));
    let previous = repair.join("previous");
    fs::create_dir_all(&previous).unwrap();
    fs::write(previous.join(".installed"), "").unwrap();
    fs::write(previous.join("origin"), "before").unwrap();
    let release = backend::Release {
        tag: TAG.into(),
        name: String::new(),
        body: String::new(),
        date: String::new(),
        time: String::new(),
        published_at: String::new(),
        asset: None,
        prerelease: false,
        nightly: Some("nightly".into()),
    };
    let stop = sunrise_launcher::native_mods::Cancellation::default();
    stop.stop();
    assert!(backend::install_cancellable(&release, Some(&stop), |_| {
        panic!("pre-cancelled install must not report a phase");
    })
    .is_err());
    assert!(
        previous.exists(),
        "pre-cancelled install must not reconcile the tree"
    );
    let phases = std::cell::RefCell::new(Vec::new());
    assert!(
        backend::install_cancellable(&release, None, |phase| phases.borrow_mut().push(phase))
            .is_err(),
        "there is no archive to fetch"
    );
    assert_eq!(phases.into_inner(), [backend::InstallPhase::Checking]);
    assert_eq!(backend::installed(), [TAG]);
    assert_eq!(
        fs::read_to_string(backend::version_dir(TAG).join("origin")).unwrap(),
        "before"
    );
    assert!(!repair.exists());
}
