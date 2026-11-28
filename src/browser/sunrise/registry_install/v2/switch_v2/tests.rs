use super::*;
use crate::{
    instances::{Channel, Incarnation, Instance, Kind as InstanceKind},
    native_mods::{Cancellation, LeftOut, Why},
};

#[test]
fn v2_profile_switch_refuses_pack_without_fetching_index() {
    let dir = tempfile::tempdir().unwrap();
    let instance = Instance {
        id: "pack-1".into(),
        name: "Pack".into(),
        home: dir.path().to_path_buf(),
        work: dir.path().to_path_buf(),
        version: None,
        channel: Channel::Dev,
        kind: InstanceKind::Pack,
        group: None,
        icon: None,
        native_profile: Default::default(),
        last_played: None,
        played: Default::default(),
        launch: Default::default(),
        incarnation: Some(Incarnation::of(dir.path(), dir.path()).unwrap()),
    };
    let cancel = Cancellation::default();
    let hooks = V2SwitchHooks {
        switch: SwitchHooks {
            cancellation: &cancel,
            progress: &|_| {},
            approve: &|_| Ok(SwitchDecision::Cancel),
            at_cutoff: &|| Ok(()),
        },
        confirm_publisher_change: None,
        pin_dir: None,
        recover_held: false,
    };
    let target = Target::of(&instance).unwrap();
    let Err(err) = switch_profile_build_v2_with(
        &target,
        target.identity(),
        "dev-2026-11-24-abc",
        Some(BuildChoice::Follow(Channel::Dev)),
        hooks,
        || panic!("pack refusal must happen before fetching v2 index"),
        &fetch_release_bytes,
        &attestation::verify_release,
    ) else {
        panic!("expected pack refusal error");
    };
    assert!(err.contains("ordinary profile"), "unexpected error: {err}");
}

#[test]
fn classify_prove_failure_maps_unfit_unproved_and_build_failed_to_holds() {
    let left = vec![
        LeftOut {
            id: "local_a".into(),
            version: "1.0.0".into(),
            why: Why::Unfit("API changed".into()),
            detail: None,
        },
        LeftOut {
            id: "local_b".into(),
            version: "1.0.0".into(),
            why: Why::Unproved("no uses list".into()),
            detail: None,
        },
        LeftOut {
            id: "dep_c".into(),
            version: "1.0.0".into(),
            why: Why::Needs("local_a".into()),
            detail: None,
        },
    ];
    let pre = classify_prove_failure(&left, false, "dev-2026-11-24-abc");
    assert_eq!(pre.len(), 2);
    assert!(matches!(
        &pre[0],
        (id, HeldReason::NoFit { build, detail: Some(d) })
            if id == "local_a" && build == "dev-2026-11-24-abc" && d == "API changed"
    ));
    assert!(matches!(
        &pre[1],
        (id, HeldReason::Unproved { build, detail: Some(d) })
            if id == "local_b" && build == "dev-2026-11-24-abc" && d == "no uses list"
    ));

    let compile_left = vec![LeftOut {
        id: "local_native".into(),
        version: "1.0.0".into(),
        why: Why::DidNotBuild,
        detail: None,
    }];
    let during = classify_prove_failure(&compile_left, true, "dev-2026-11-24-abc");
    assert_eq!(during.len(), 1);
    assert!(matches!(
        &during[0],
        (id, HeldReason::BuildFailed { build })
            if id == "local_native" && build == "dev-2026-11-24-abc"
    ));
}

#[test]
fn collect_rechecked_local_holds_reoffers_matching_local_holds_and_skips_registry_holds() {
    let index: Index = serde_json::from_value(serde_json::json!({
        "schema": 2,
        "serial": 1,
        "generated": "2026-11-24T00:00:00Z",
        "builds": [],
        "categories": [],
        "collections": [],
        "projects": [],
        "revoked": []
    }))
    .unwrap();
    let digest_hex = "11".repeat(32);
    let digest_bytes = [0x11u8; 32];
    let local_hold = Held::new(
        "local_mod".into(),
        "Local Mod".into(),
        digest_bytes,
        (HeldRoot::Native, "local_mod.smod".into()),
        HeldSource::Local {
            version: "1.0.0".into(),
        },
        true,
        HeldReason::BuildFailed {
            build: "dev-2026-11-20-old".into(),
        },
    )
    .unwrap();
    let registry_hold = Held::new(
        "reg_mod".into(),
        "Reg Mod".into(),
        digest_bytes,
        (HeldRoot::Content, "reg_mod.smod".into()),
        HeldSource::Registry {
            project: "reg_mod".into(),
            version: "1.0.0".into(),
            repository: None,
        },
        true,
        HeldReason::NoFit {
            build: "dev-2026-11-20-old".into(),
            detail: None,
        },
    )
    .unwrap();
    let pkg = |id: &str| {
        let manifest: sunrise_launcher_smod::manifest::Manifest = toml::from_str(&format!(
            "id = \"{id}\"\nname = \"{id}\"\nversion = \"1.0.0\"\napi = \"1\"\n"
        ))
        .unwrap();
        crate::managed_play::ScannedPackage {
            path: PathBuf::from(format!("mods/{id}.smod")),
            directory: false,
            manifest,
        }
    };
    let before = BTreeMap::from([
        (
            "local_mod".into(),
            InstalledV2Mod {
                package: pkg("local_mod"),
                digest: digest_hex.clone(),
                size: 0,
                enabled: false,
                ownership: V2Ownership::Local,
            },
        ),
        (
            "reg_mod".into(),
            InstalledV2Mod {
                package: pkg("reg_mod"),
                digest: digest_hex,
                size: 0,
                enabled: false,
                ownership: V2Ownership::Local,
            },
        ),
    ]);
    let rechecked = collect_rechecked_local_holds(&[local_hold, registry_hold], &before, &index);
    assert_eq!(rechecked, BTreeSet::from(["local_mod".to_owned()]));
}

fn manifest(id: &str, native: bool) -> sunrise_launcher_smod::manifest::Manifest {
    let native = if native {
        format!(
            "[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        )
    } else {
        String::new()
    };
    toml::from_str(&format!(
        "id = \"{id}\"\nname = \"{id}\"\nversion = \"1.0.0\"\napi = \"1\"\n{native}"
    ))
    .unwrap()
}

fn installed(path: PathBuf, id: &str, native: bool, digest: &str, enabled: bool) -> InstalledV2Mod {
    InstalledV2Mod {
        package: crate::managed_play::ScannedPackage {
            path,
            directory: false,
            manifest: manifest(id, native),
        },
        digest: digest.into(),
        size: 0,
        enabled,
        ownership: V2Ownership::Local,
    }
}

fn local_hold(id: &str, enabled_before: bool) -> Held {
    Held::new(
        id.into(),
        id.into(),
        [0x11; 32],
        (HeldRoot::Content, format!("{id}.smod")),
        HeldSource::Local {
            version: "1.0.0".into(),
        },
        enabled_before,
        HeldReason::BuildFailed {
            build: "dev-2026-11-20-old".into(),
        },
    )
    .unwrap()
}

fn no_fit() -> HeldReason {
    HeldReason::NoFit {
        build: "dev-2026-11-24-abc".into(),
        detail: None,
    }
}

#[test]
fn a_mod_held_again_keeps_the_on_state_from_before_its_first_hold() {
    let dir = tempfile::tempdir().unwrap();
    let digest = "11".repeat(32);
    let path = |id: &str| dir.path().join(format!("mods/{id}.smod"));
    // Re-offered after an earlier hold, then failing again: off on disk, wanted on.
    let rechecked = installed(path("rechecked"), "rechecked", false, &digest, false);
    // Turned off by the player before any hold.
    let off = installed(path("off"), "off", false, &digest, false);
    let previous = [local_hold("rechecked", true), local_hold("off", false)];
    let holder = Holder {
        home: dir.path(),
        wanted: previous
            .iter()
            .filter(|hold| hold.enabled_before())
            .map(|hold| hold.id().to_owned())
            .collect(),
    };
    let again = holder.hold("rechecked", &rechecked, no_fit()).unwrap();
    assert!(again.enabled_before());
    assert!(!holder.hold("off", &off, no_fit()).unwrap().enabled_before());
}

#[test]
fn a_held_github_mod_keeps_its_github_source() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let digest = "22".repeat(32);
    let archive = home
        .join(crate::native_mods::INSTALLED)
        .join("from_github.smod");
    fs::create_dir_all(archive.parent().unwrap()).unwrap();
    let receipt = serde_json::json!({
        "id": "from_github",
        "version": "1.0.0",
        "owner": "camp",
        "repo": "from-github",
        "commit": "c".repeat(40),
        "sha256": digest,
        "added_from": "github",
    });
    fs::write(
        crate::github_mods::origin_path(home, "from_github"),
        receipt.to_string(),
    )
    .unwrap();
    let item = installed(archive, "from_github", true, &digest, true);
    let hold = make_v2_hold(home, "from_github", &item, true, no_fit()).unwrap();
    assert_eq!(
        hold.source(),
        &HeldSource::Github {
            owner: "camp".into(),
            repo: "from-github".into(),
            commit: "c".repeat(40),
            version: "1.0.0".into(),
        }
    );

    // A receipt for other bytes is not this archive's.
    let other = installed(
        item.package.path.clone(),
        "from_github",
        true,
        &"33".repeat(32),
        true,
    );
    let hold = make_v2_hold(home, "from_github", &other, true, no_fit()).unwrap();
    assert!(matches!(hold.source(), HeldSource::Local { .. }));
}

#[test]
fn a_mod_held_again_retires_its_earlier_hold() {
    let previous = [
        local_hold("revoked_local", true),
        local_hold("still_held", true),
    ];
    let final_holds = [local_hold("revoked_local", true)];
    let retired = retired_holds(
        BTreeSet::from(["rechecked".to_owned()]),
        &previous,
        &final_holds,
    );
    assert_eq!(
        retired,
        BTreeSet::from(["rechecked".to_owned(), "revoked_local".to_owned()])
    );
    // What publication keeps: the earlier holds not retired, plus today's.
    let mut ids = BTreeSet::new();
    for hold in previous
        .iter()
        .filter(|hold| !retired.contains(hold.id()))
        .chain(&final_holds)
    {
        assert!(ids.insert(hold.id()), "two holds for {}", hold.id());
    }
}

#[test]
fn holding_a_staged_replacement_puts_back_its_archive_row_and_pin() {
    let dir = tempfile::tempdir().unwrap();
    let old_home = dir.path().join("old");
    let stage_home = dir.path().join("stage");
    let rel = PathBuf::from(mod_list::MODS_DIR).join("swapped.smod");
    for home in [&old_home, &stage_home] {
        fs::create_dir_all(home.join(mod_list::MODS_DIR)).unwrap();
    }
    fs::write(old_home.join(&rel), b"old bytes").unwrap();
    fs::write(stage_home.join(&rel), b"new bytes").unwrap();
    let row = |version: &str| serde_json::json!({ "entries": [{ "source": "sunrise", "project": "Ab3dE9xQ", "mod_id": "swapped", "version": version }] });
    let manifest = crate::browser::installed::MANIFEST;
    fs::create_dir_all(old_home.join(manifest).parent().unwrap()).unwrap();
    fs::create_dir_all(stage_home.join(manifest).parent().unwrap()).unwrap();
    fs::write(old_home.join(manifest), row("1.0.0").to_string()).unwrap();
    fs::write(stage_home.join(manifest), row("2.0.0").to_string()).unwrap();

    let before = BTreeMap::from([(
        "swapped".to_owned(),
        installed(
            old_home.join(&rel),
            "swapped",
            false,
            &"11".repeat(32),
            true,
        ),
    )]);
    let original_doc = Document::read(&old_home).unwrap();
    let mut doc = Document::read(&stage_home).unwrap();
    let mut swaps = BTreeMap::from([(
        "swapped".to_owned(),
        StagedSwap {
            project_id: "Ab3dE9xQ".into(),
            archive: stage_home.join(&rel),
        },
    )]);
    let mut staged_ids = BTreeSet::from(["swapped".to_owned()]);
    let pin = |project_id: &str| PendingPin {
        project_id: project_id.into(),
        pin: attestation::PublisherPin {
            owner_id: None,
            repository_id: 1,
            repository_name: "camp/swapped".into(),
            workflow_path: ".github/workflows/release.yml".into(),
        },
        accepted_change: None,
    };
    let mut pins = vec![pin("Ab3dE9xQ"), pin("Other123")];
    let holds = [Held::new(
        "swapped".into(),
        "swapped".into(),
        [0x11; 32],
        (HeldRoot::Content, "swapped.smod".into()),
        HeldSource::Local {
            version: "1.0.0".into(),
        },
        true,
        no_fit(),
    )
    .unwrap()];
    SwapUndo {
        old_home: &old_home,
        stage_home: &stage_home,
        before: &before,
        original_doc: &original_doc,
    }
    .held(&holds, &mut swaps, &mut staged_ids, &mut doc, &mut pins)
    .unwrap();

    assert_eq!(fs::read(stage_home.join(&rel)).unwrap(), b"old bytes");
    assert_eq!(doc.row("swapped").unwrap().unwrap()["version"], "1.0.0");
    assert!(staged_ids.is_empty() && swaps.is_empty());
    assert_eq!(
        pins.iter()
            .map(|p| p.project_id.as_str())
            .collect::<Vec<_>>(),
        ["Other123"]
    );
}
