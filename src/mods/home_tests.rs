use super::*;

#[test]
fn home_finds_resource_packs_after_leaving_a_different_installed_shelf() {
    in_scratch_launcher(
        "mods::home_tests::home_finds_resource_packs_after_leaving_a_different_installed_shelf",
        |launcher| {
            for shelf in [Shelf::Shader, Shelf::Data] {
                launcher.ui.page = Page::Home;
                launcher.mods_state.shelf = shelf;
                launcher.mods_state.packs.status = pack_discover::Status::Idle;
                let before = launcher.mods_state.packs.seq;
                launcher.show_resource_pack_discover();
                assert!(launcher.ui.page == Page::PackFind);
                assert_eq!(
                    launcher.mods_state.packs.scope,
                    pack_discover::Scope::Resource
                );
                assert_eq!(
                    launcher.mods_state.packs.seq,
                    before + 2,
                    "one scope change and search"
                );
            }
        },
    );
}

#[test]
fn home_labels_follow_installed_replies_with_the_same_inventory() {
    in_scratch_launcher(
        "mods::home_tests::home_labels_follow_installed_replies_with_the_same_inventory",
        |launcher| {
            let home = launcher.mods_home();
            let (key, _, _) = launcher.mods_snapshot_key(&home, std::time::Instant::now());
            // The first Installed read has not answered.
            let inventory = 1;
            launcher.mods_state.snapshot = Some(Snapshot {
                home,
                key: key.clone(),
                rows: InstalledRows::default(),
                reading: true,
                error: None,
                backup_notice: None,
            });
            let id = "lantern";
            let installed = || InstalledRows {
                local: vec![model::Local {
                    id: id.into(),
                    file: "lantern.smod".into(),
                    name: "Lantern".into(),
                    version: "1.0.0".into(),
                    github: None,
                    native: true,
                    served_by: None,
                    unreadable: false,
                }],
                enabled: HashSet::from([id.into()]),
                ..Default::default()
            };
            assert!(launcher.home_mod_facts(inventory).is_empty());
            // No forwarding thread: every injected worker reply is available for this drain.
            let (tx, rx) = std::sync::mpsc::channel();
            launcher.mods_state.rx = rx;
            tx.send(Event::Local(
                "old fingerprint".into(),
                Box::new(Ok(Some((installed(), None)))),
            ))
            .unwrap();
            launcher.mods_drain();
            assert!(launcher.home_mod_facts(inventory).is_empty());
            tx.send(Event::Local(
                key.clone(),
                Box::new(Ok(Some((installed(), None)))),
            ))
            .unwrap();
            launcher.mods_drain();
            let facts = launcher.home_mod_facts(inventory);
            assert_eq!(facts[id].kind, tr("Native"));
            assert!(
                facts[id].status.is_none(),
                "native code needs no permission"
            );

            let mut updated = installed();
            updated.local[0].native = false;
            tx.send(Event::Local(key, Box::new(Ok(Some((updated, None))))))
                .unwrap();
            launcher.mods_drain();
            let facts = launcher.home_mod_facts(inventory);
            assert_eq!(facts[id].kind, tr("Content"));
            assert!(facts[id].status.is_none());
        },
    );
}

// Launcher startup uses process settings and starts workers. Run it with its own folders,
// no keychain, and a proxy that can't reach the registry or update services.
pub(crate) fn in_scratch_launcher(test: &str, run: impl FnOnce(&mut Launcher)) {
    const CHILD: &str = "SUNRISE_HOME_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let root = tempfile::tempdir().unwrap();
        let nowhere = "http://127.0.0.1:9";
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--nocapture"])
            .env(CHILD, "1")
            .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
            .env("SUNRISE_HOME", root.path().join("game"))
            .env(
                "SUNRISE_LAUNCHER_LOCATION_FILE",
                root.path().join("location.json"),
            )
            .env("SUNRISE_NO_KEYCHAIN", "1")
            .env("SUNRISE_TELEMETRY_URL", "off")
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("XDG_DATA_HOME", root.path().join("data"))
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("HTTPS_PROXY", nowhere)
            .env("HTTP_PROXY", nowhere)
            .env("ALL_PROXY", nowhere)
            .env_remove("NO_PROXY")
            .env_remove("SUNRISE_GITHUB_MIRROR")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let mut app = gpui::TestAppContext::single();
    let (shell, cx) = app.add_window_view(crate::window_root::Shell::new_for_test);
    let launcher = shell.read_with(cx, |shell, _| shell.launcher());
    launcher.update(cx, |launcher, _| run(launcher));
}
