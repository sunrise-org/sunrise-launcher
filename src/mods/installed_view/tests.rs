use super::items::{installed_items_key, InstalledItem};
use super::*;

/// Runs `test` again in a child process with scratch homes, as the other window tests do:
/// `None` here, the scratch root in the child.
fn scratch_child(test: &str) -> Option<std::path::PathBuf> {
    const CHILD: &str = "SUNRISE_INSTALLED_VIEW_TEST";
    if let Some(root) = std::env::var_os(CHILD) {
        return Some(root.into());
    }
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let nowhere = "http://127.0.0.1:9";
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &format!("mods::installed_view::tests::{test}")])
        .env(CHILD, root)
        .env("SUNRISE_HOME", root.join("game"))
        .env("SUNRISE_LAUNCHER_HOME", root.join("launcher"))
        .env("SUNRISE_LAUNCHER_LOCATION_FILE", root.join("location.json"))
        .env("SUNRISE_NO_KEYCHAIN", "1")
        .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
        .env("SUNRISE_TELEMETRY_URL", "off")
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("HTTP_PROXY", nowhere)
        .env("HTTPS_PROXY", nowhere)
        .env("ALL_PROXY", nowhere)
        .env_remove("NO_PROXY")
        .env_remove("SUNRISE_GITHUB_MIRROR")
        .env_remove("SUNRISE_LAUNCHER_BENCH")
        .status()
        .unwrap();
    assert!(status.success(), "{test} failed in its child process");
    None
}

#[test]
fn a_mod_rewritten_in_place_shows_on_an_idle_page_once_due() {
    if scratch_child("a_mod_rewritten_in_place_shows_on_an_idle_page_once_due").is_none() {
        return;
    }
    let profile = sunrise_launcher::instances::create("Pack", None).unwrap();
    std::fs::create_dir_all(profile.home.join("sunrise-mods")).unwrap();
    let entries: Vec<_> = (0..3)
        .map(|n| {
            let id = format!("mod-{n}");
            let file = format!("sunrise-mods/{id}.smod");
            std::fs::write(profile.home.join(&file), b"one").unwrap();
            serde_json::json!({
                "source": "sunrise", "project": id, "slug": id, "title": format!("Mod {n}"),
                "kind": "mod", "version": "1.0.0", "number": "1.0.0", "file": file,
                "checksum": format!("SHA-256:{}", "0".repeat(64)), "page": ""
            })
        })
        .collect();
    std::fs::write(
        profile.home.join("mod-browser.json"),
        serde_json::json!({ "entries": entries }).to_string(),
    )
    .unwrap();
    let mut settings = sunrise_launcher::backend::Settings::load();
    settings.instance = Some(profile.id.clone());
    settings.save().unwrap();

    let mut app = gpui::TestAppContext::single();
    let (shell, cx) = app.add_window_view(crate::window_root::Shell::new_for_test);
    let launcher = shell.read_with(cx, |shell, _| shell.launcher());
    launcher.update(cx, |l, cx| {
        l.show_installed_mods();
        cx.notify();
    });
    let frame = |cx: &mut gpui::VisualTestContext| {
        launcher.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
    };
    let read = |cx: &mut gpui::VisualTestContext| {
        launcher.read_with(cx, |l, _| {
            l.mods_state
                .snapshot
                .as_ref()
                .filter(|s| !s.reading)
                .map(|s| s.key.clone())
        })
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let wait = |cx: &mut gpui::VisualTestContext, not: Option<&str>| loop {
        assert!(
            std::time::Instant::now() < deadline,
            "the mods were never read"
        );
        frame(cx);
        if let Some(key) = read(cx).filter(|key| Some(key.as_str()) != not) {
            return key;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let first = wait(cx, None);

    // Rewritten where it lies: the next frame keeps the fingerprint it has.
    std::fs::write(profile.home.join("sunrise-mods/mod-1.smod"), b"one, longer").unwrap();
    frame(cx);
    assert_eq!(read(cx).as_deref(), Some(first.as_str()));
    // Before it is due the pump sees nothing to do; once due it sees the change, with no
    // input in between, and the next frame reads the mods again.
    let now = std::time::Instant::now();
    assert!(!launcher.update(cx, |l, _| l.mods_check_due(now)));
    let due = now + sunrise_launcher::native_status::FULL_EVERY;
    assert!(launcher.update(cx, |l, _| l.mods_check_due(due)));
    let second = wait(cx, Some(&first));
    assert_ne!(second, first);
    // An unchanged folder asks for nothing, even when due.
    let again = due + sunrise_launcher::native_status::FULL_EVERY;
    assert!(!launcher.update(cx, |l, _| l.mods_check_due(again)));
}

#[test]
fn a_mod_far_down_the_list_is_built_and_revealed() {
    if scratch_child("a_mod_far_down_the_list_is_built_and_revealed").is_none() {
        return;
    }
    // A profile with 300 mods, far more than a screen.
    let profile = sunrise_launcher::instances::create("Pack", None).unwrap();
    std::fs::create_dir_all(profile.home.join("sunrise-mods")).unwrap();
    let entries: Vec<_> = (0..300)
        .map(|n| {
            let id = format!("mod-{n:03}");
            let file = format!("sunrise-mods/{id}.smod");
            std::fs::write(profile.home.join(&file), b"").unwrap();
            serde_json::json!({
                "source": "sunrise", "project": id, "slug": id, "title": format!("Mod {n:03}"),
                "kind": "mod", "version": "1.0.0", "number": "1.0.0", "file": file,
                "checksum": format!("SHA-256:{}", "0".repeat(64)), "page": ""
            })
        })
        .collect();
    std::fs::write(
        profile.home.join("mod-browser.json"),
        serde_json::json!({ "entries": entries }).to_string(),
    )
    .unwrap();
    let mut settings = sunrise_launcher::backend::Settings::load();
    settings.instance = Some(profile.id.clone());
    settings.save().unwrap();

    let mut app = gpui::TestAppContext::single();
    let (shell, cx) = app.add_window_view(crate::window_root::Shell::new_for_test);
    let launcher = shell.read_with(cx, |shell, _| shell.launcher());
    // Opened at the 251st mod, as Home's mod count opens it at one.
    launcher.update(cx, |l, cx| {
        l.open_installed(super::super::search::Installed::Mod("mod-250".into()));
        cx.notify();
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let frame = |cx: &mut gpui::VisualTestContext| {
        launcher.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
    };
    while !launcher.read_with(cx, |l, _| l.mods_state.revealed.get()) {
        assert!(
            std::time::Instant::now() < deadline,
            "the mod was never scrolled to"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        frame(cx);
    }
    // Draw a few more frames so any remaining layout adjustments settle.
    for _ in 0..4 {
        frame(cx);
    }
    let scroll = cx
        .update(|_, _| crate::frame::drawn_scroll("installed"))
        .unwrap();
    let view = scroll.bounds();
    assert!(view.size.height > px(0.));
    let cull = launcher.read_with(cx, |l, _| l.mods.installed_list.clone());
    assert!(cull.settled(), "the list did not settle after reveal");
    let row = cull
        .drawn_bounds(250)
        .expect("mod-250 was not built after settling");
    assert!(
        row.top() >= view.top() && row.bottom() <= view.bottom(),
        "mod-250 ({row:?}) is outside the viewport ({view:?})"
    );
}

#[test]
fn shift_click_resolves_its_archive_after_a_catalogue_reply_regroups_the_list() {
    let mut mods = ModsState::default();
    let local = ["ant", "bird", "cat", "dog"].map(|id| model::Local {
        id: id.into(),
        file: format!("sunrise-native-mods/{id}.smod"),
        name: id.into(),
        version: "1.0.0".into(),
        github: None,
        native: true,
        served_by: None,
        unreadable: false,
    });
    let mut rows = model::rows(
        &[],
        &model::World {
            v2: None,
            v2_failure: None,
            build: None,
            unfit_updates: &BTreeSet::new(),
            enabled: &HashSet::new(),
            unbuilt: &HashSet::new(),
            chosen: &BTreeMap::new(),
            local: &local,
        },
    );
    let shown = |rows: &[Row]| -> Vec<_> {
        rows.iter()
            .map(|row| (row.archive_file().unwrap().to_owned(), row.id.clone()))
            .collect()
    };
    let before = shown(&rows);
    mods.select_installed_row(&local[2].file, &before, false);
    // The catalogue lists Cat in a collection after the first click.
    rows[2].collection = Some(model::CollectionGroup {
        id: "pets".into(),
        name: "Pets".into(),
    });
    model::group_installed(&mut rows);
    let after = shown(&rows);
    assert_eq!(after[0].0, local[2].file);
    mods.select_installed_row(&local[0].file, &after, true);
    assert_eq!(mods.selection, ["ant".to_owned(), "cat".to_owned()].into());
    // If the anchor no longer exists, treat a Shift-click like a normal click.
    let remaining = shown(&rows[2..]);
    mods.select_installed_row(&remaining[1].0, &remaining, true);
    assert!(!mods.selection.contains("bird"));
    assert!(mods.selection.contains("dog"));
}

#[test]
fn confirmed_empty_installed_resets_filters_selection_and_anchor() {
    let mut mods = ModsState::default();
    mods.row_filter.show = model::Show::On;
    mods.row_filter.makes.insert(model::Make::Content);
    mods.selection.insert("fern".into());
    mods.anchor = Some("sunrise-mods/fern.smod".into());
    let counts = mods.installed_filters(&[]);
    assert!(counts.shows.is_empty() && counts.kinds.is_empty());
    assert_eq!(mods.row_filter, model::RowFilter::default());
    assert!(mods.selection.is_empty() && mods.anchor.is_none());
}

#[test]
fn show_resource_pack_discover_re_searches_after_clearing_a_previous_resource_query() {
    if scratch_child(
        "show_resource_pack_discover_re_searches_after_clearing_a_previous_resource_query",
    )
    .is_none()
    {
        return;
    }
    let mut app = gpui::TestAppContext::single();
    let (launcher, cx) =
        app.add_window_view(|w, cx| Launcher::new_with_startup_checks(w, cx, true));
    launcher.update(cx, |l, _| {
        l.ui.page = Page::Home;
        l.mods_state.find.set("faithful".into());
        l.mods_state.packs.scope = pack_discover::Scope::Resource;
        l.mods_state.packs.status = pack_discover::Status::Done;
        let seq_before = l.mods_state.packs.seq;

        l.show_resource_pack_discover();

        assert_eq!(l.ui.page, Page::PackFind);
        assert_eq!(l.mods_state.shelf, Shelf::Resource);
        assert!(l.mods_state.find.text().is_empty());
        assert!(
            l.mods_state.packs.seq > seq_before,
            "entering through Find resource packs must trigger a fresh search for the cleared query"
        );
    });
}

#[test]
fn installed_items_key_distinguishes_local_archives_sharing_a_mod_id() {
    let local = [
        model::Local {
            id: "lantern".into(),
            file: "sunrise-mods/lantern-a.smod".into(),
            name: "Lantern A".into(),
            version: "1.0.0".into(),
            github: None,
            native: false,
            served_by: None,
            unreadable: false,
        },
        model::Local {
            id: "lantern".into(),
            file: "sunrise-mods/lantern-b.smod".into(),
            name: "Lantern B".into(),
            version: "1.0.0".into(),
            github: None,
            native: false,
            served_by: None,
            unreadable: false,
        },
    ];
    let rows = model::rows(
        &[],
        &model::World {
            v2: None,
            v2_failure: None,
            build: None,
            unfit_updates: &BTreeSet::new(),
            enabled: &HashSet::new(),
            unbuilt: &HashSet::new(),
            chosen: &BTreeMap::new(),
            local: &local,
        },
    );
    assert_eq!(rows.len(), 2);
    let key_a = installed_items_key(&[InstalledItem::Row(0, 0)], &rows);
    let key_b = installed_items_key(&[InstalledItem::Row(1, 0)], &rows);
    assert_ne!(
        key_a, key_b,
        "swapping local archives with the same mod ID must invalidate cached row heights"
    );
}
