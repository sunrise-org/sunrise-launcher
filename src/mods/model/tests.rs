use super::*;
use sunrise_launcher::browser::{installed::Entry, mod_list};

#[test]
fn source_links_only_trim_repositories_on_github() {
    for (source, expected) in [
        (
            "https://github.com/Hollowfen/Trail-Markers.git/tree/main/mods?tab=readme#install",
            Some(SourceLink::GitHub(
                "https://github.com/Hollowfen/Trail-Markers".into(),
            )),
        ),
        (
            "https://github.com/Hollowfen/Trail-Markers/blob/main/README.md",
            Some(SourceLink::GitHub(
                "https://github.com/Hollowfen/Trail-Markers".into(),
            )),
        ),
        (
            "https://github.com.example.test/owner/repo.git/tree/main?keep=1#readme",
            Some(SourceLink::Other(
                "https://github.com.example.test/owner/repo.git/tree/main?keep=1#readme".into(),
            )),
        ),
        (
            "https://code.example.test/owner/repo?keep=1#readme",
            Some(SourceLink::Other(
                "https://code.example.test/owner/repo?keep=1#readme".into(),
            )),
        ),
        ("https://github.com/owner", None),
        ("https://github.com/owner/../real/repo", None),
        ("https://@github.com/owner/repo", None),
        ("https://user:pass@github.com/owner/repo", None),
        ("http://github.com/owner/repo", None),
        ("https:///owner/repo", None),
        ("", None),
    ] {
        assert_eq!(SourceLink::read(source), expected, "{source}");
    }
}

fn entry(id: &str, version: &str, native: bool, sha: &str) -> Entry {
    let folder = if native {
        "sunrise-native-mods"
    } else {
        "sunrise-mods"
    };
    serde_json::from_value(serde_json::json!({
        "source": "sunrise", "project": id, "slug": id, "title": id.to_uppercase(), "kind": "mod",
        "version": version, "number": version, "file": format!("{folder}/{id}.smod"),
        "checksum": format!("SHA-256:{sha}"), "page": ""
    }))
    .unwrap()
}

#[test]
fn installed_sections_keep_collection_members_together() {
    let idx = index(vec![], vec![]);
    let none = HashSet::new();
    let entries = [
        entry("ant", "1.0.0", true, A),
        entry("bird", "1.0.0", true, A),
        entry("cat", "1.0.0", true, A),
        entry("dog", "1.0.0", true, A),
    ];
    let mut rows = rows(&entries, &world(&idx, &none, &none));
    let collection = |id: &str| {
        Some(CollectionGroup {
            id: id.into(),
            name: "Wildlife".into(),
        })
    };
    rows[0].collection = collection("park");
    rows[2].collection = collection("park");
    rows[3].collection = collection("zoo");
    rows[2].need = Some(Need::Build);
    group_installed(&mut rows);
    assert_eq!(
        rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        ["cat", "ant", "dog", "bird"]
    );
}

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

static NO_PINS: std::sync::LazyLock<std::collections::BTreeMap<String, String>> =
    std::sync::LazyLock::new(Default::default);
static NO_UNFIT_UPDATES: BTreeSet<String> = BTreeSet::new();

/// The exact build the rows' registry releases are listed for.
const BUILD: &str = "26.3-0.1.0";

fn world<'a>(
    index: &'a index_v2::Index,
    enabled: &'a HashSet<String>,
    unbuilt: &'a HashSet<String>,
) -> World<'a> {
    World {
        v2: Some(index),
        v2_failure: None,
        build: Some(BUILD),
        unfit_updates: &NO_UNFIT_UPDATES,
        enabled,
        unbuilt,
        chosen: &NO_PINS,
        local: &[],
    }
}

/// The registry's 8-character project ID for mod `id`.
fn project_id(id: &str) -> String {
    format!("{id:0<8}")
}

/// A registry project for mod `id` with `releases` (version, sha256), newest first, all listed
/// for [`BUILD`].
fn project(id: &str, kind: &str, releases: &[(&str, &str)]) -> serde_json::Value {
    serde_json::json!({
        "id": project_id(id), "slug": id, "mod": id, "kind": kind, "name": id.to_uppercase(),
        "summary": format!("{id} does things"), "description": "", "authors": ["Someone"],
        "owners": ["someone"], "license": "MIT", "icon": "i", "screenshots": [], "categories": [],
        "repository": {"id": 1, "name": "x/y", "path": "."}, "workflow": "w",
        "releases": releases.iter().map(|(version, sha)| serde_json::json!({
            "version": version, "published": "2026-11-22", "sunrise": BUILD, "workflow": "w",
            "fits": [BUILD], "does_not_fit": [],
            "file": {"url": "u", "size": 2048, "sha256": sha, "attestation": "a"},
            "dependencies": []
        })).collect::<Vec<_>>(),
    })
}

fn index(projects: Vec<serde_json::Value>, revoked: Vec<serde_json::Value>) -> index_v2::Index {
    serde_json::from_value(serde_json::json!({
        "schema": 2, "serial": 1, "generated": "2026-11-22T00:00:00Z", "builds": [],
        "categories": [], "projects": projects, "revoked": revoked
    }))
    .unwrap()
}

/// What a registry install records for mod `id`.
fn registry_entry(id: &str, version: &str, native: bool, sha: &str) -> Entry {
    Entry {
        project: project_id(id),
        mod_id: Some(id.into()),
        ..entry(id, version, native, sha)
    }
}

fn pack() -> (index_v2::Index, Vec<Entry>) {
    let idx = index(
        vec![
            project("zoom", "mod", &[("2.0.0", B), ("1.0.0", A)]),
            project("bad", "mod", &[("2.0.0", B), ("1.0.0", A)]),
            project("deep", "native-mod", &[("2.0.0", B), ("1.0.0", A)]),
            project("quiet", "mod", &[("1.0.0", A)]),
        ],
        vec![serde_json::json!({
            "project": project_id("bad"), "sha256": A, "reason": "unsafe", "date": "2026-11-01"
        })],
    );
    let entries = vec![
        registry_entry("quiet", "1.0.0", false, A),
        registry_entry("zoom", "1.0.0", false, A),
        registry_entry("deep", "1.0.0", true, A),
        registry_entry("bad", "1.0.0", false, A),
    ];
    (idx, entries)
}

#[test]
fn a_held_source_is_grouped_with_rows_that_need_attention() {
    let (idx, entries) = pack();
    let enabled = HashSet::new();
    let unbuilt = HashSet::new();
    let mut assembled = rows(&entries, &world(&idx, &enabled, &unbuilt));
    let quiet = assembled.iter_mut().find(|row| row.id == "quiet").unwrap();
    assert_eq!(quiet.group(), Group::Rest);
    quiet.held_for = Some(Hold {
        build: "nightly-test".into(),
        reason: sunrise_launcher::managed_play::HeldReason::NoFit {
            build: "nightly-test".into(),
            detail: None,
        },
        retry: false,
    });
    assert_eq!(quiet.group(), Group::NeedsYou);
    // A registry hold waits for its author: its switch is out of reach and dimmed.
    let waiting = cells(quiet, None, None);
    assert!(!waiting.switch && waiting.unavailable);
    // Local or source bytes may be tried again: the switch works and isn't dimmed as dropped.
    quiet.held_for.as_mut().unwrap().retry = true;
    let retryable = cells(quiet, None, None);
    assert!(retryable.switch && !retryable.unavailable);
}

#[test]
fn rows_put_what_needs_the_player_first_then_updates_then_the_rest() {
    let (idx, entries) = pack();
    let enabled: HashSet<String> = ["quiet", "zoom", "deep", "bad"].map(String::from).into();
    let unbuilt: HashSet<String> = ["deep".to_string()].into();
    let r = rows(&entries, &world(&idx, &enabled, &unbuilt));
    let order: Vec<(&str, Group)> = r.iter().map(|r| (r.id.as_str(), r.group())).collect();
    assert_eq!(
        order,
        [
            ("bad", Group::NeedsYou),
            ("deep", Group::NeedsYou),
            ("zoom", Group::Updates),
            ("quiet", Group::Rest)
        ]
    );
    assert_eq!(r[0].need, Some(Need::TakenDown("unsafe".into())));
    assert_eq!(
        r[0].update.as_deref(),
        Some("2.0.0"),
        "the way out of a takedown is shown too"
    );
    assert_eq!(r[1].need, Some(Need::Build));
    assert_eq!(r[1].make, Some(Make::Native));
    assert_eq!(r[2].update.as_deref(), Some("2.0.0"));
}

#[test]
fn registry_installed_rows_use_live_names_and_statuses_or_saved_names_and_not_checked() {
    let (idx, mut entries) = pack();
    let bad = entries.iter_mut().find(|e| e.slug == "bad").unwrap();
    bad.title = "Unverified saved name".into();
    let none = HashSet::new();
    let live = world(&idx, &none, &none);
    let row = rows(std::slice::from_ref(bad), &live).pop().unwrap();
    assert_eq!(row.title, "BAD");
    assert_eq!(row.authors, "Someone");
    assert_eq!(row.need, Some(Need::TakenDown("unsafe".into())));
    assert_eq!(row.update.as_deref(), Some("2.0.0"));
    assert_eq!(row.registry_issue, None);

    let unreachable = "Registry not reachable: connection refused";
    let offline = World {
        v2: None,
        v2_failure: Some(unreachable),
        ..live
    };
    let row = rows(std::slice::from_ref(bad), &offline).pop().unwrap();
    assert_eq!(row.title, "Unverified saved name");
    assert!(row.authors.is_empty());
    assert!(row.icon.is_none());
    assert!(row.need.is_none());
    assert!(row.update.is_none());
    assert_eq!(row.registry_issue.as_deref(), Some(unreachable));
}

#[test]
fn native_code_that_is_on_and_unbuilt_asks_to_build() {
    let (idx, entries) = pack();
    let enabled: HashSet<String> = ["deep".to_string()].into();
    let unbuilt: HashSet<String> = ["deep".to_string()].into();
    let r = rows(&entries, &world(&idx, &enabled, &unbuilt));
    let deep = r.iter().find(|r| r.id == "deep").unwrap();
    assert_eq!(deep.need, Some(Need::Build));
}

#[test]
fn only_registry_mods_are_rows_packs_are_left_alone() {
    let (idx, mut entries) = pack();
    let mut other = entry("shaderish", "1", false, A);
    other.kind = sunrise_launcher::browser::Kind::Shader;
    entries.push(other);
    let none = HashSet::new();
    let r = rows(&entries, &world(&idx, &none, &none));
    assert_eq!(r.len(), 4);
    assert!(r.iter().all(|r| r.id != "shaderish"));
}

#[test]
fn update_all_includes_off_and_native_rows_but_not_a_player_pin() {
    let (idx, entries) = pack();
    let none = HashSet::new();
    let on: HashSet<String> = ["zoom".to_string()].into();
    let unpinned = world(&idx, &on, &none);
    let offered = rows(&entries, &unpinned);
    let updates = offered
        .iter()
        .filter(|r| bulk_updatable(r))
        .map(|r| r.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(updates, ["bad", "deep", "zoom"]);
    assert!(offered
        .iter()
        .any(|r| r.id == "bad" && !r.on && bulk_updatable(r)));

    let pins = std::collections::BTreeMap::from([(project_id("zoom"), "1.0.0".into())]);
    let pinned = rows(
        &entries,
        &World {
            chosen: &pins,
            ..unpinned
        },
    );
    assert!(pinned
        .iter()
        .any(|r| r.id == "zoom" && r.update.is_some() && !bulk_updatable(r)));
}

#[test]
fn a_row_the_plan_cannot_update_says_why() {
    let (idx, entries) = pack();
    let on: HashSet<String> = ["zoom"].map(String::from).into();
    let none = HashSet::new();
    let pins = std::collections::BTreeMap::from([(project_id("zoom"), "1.0.0".to_string())]);
    let w = World {
        chosen: &pins,
        ..world(&idx, &on, &none)
    };
    let r = rows(&entries, &w);
    let zoom = r.iter().find(|r| r.id == "zoom").unwrap();
    assert!(!bulk_updatable(zoom));
    let tip = cells(zoom, None, None).status.expect("says something").tip;
    assert!(tip.contains("You chose its version"), "{tip}");
}

#[test]
fn native_files_the_browser_didnt_install_are_listed_and_never_planned() {
    let (idx, entries) = pack();
    let on: HashSet<String> = ["mine", "zoom", "by_hand"].map(String::from).into();
    let none = HashSet::new();
    let local = [
        Local {
            id: "mine".into(),
            file: "mine".to_owned(),
            name: "mine".into(),
            version: "0.3".into(),
            github: None,
            native: true,
            served_by: None,
            unreadable: false,
        },
        Local {
            id: "by_hand".into(),
            file: "by_hand".to_owned(),
            name: "By Hand".into(),
            version: "1.0".into(),
            github: None,
            native: false,
            served_by: None,
            unreadable: false,
        },
    ];
    let w = World {
        local: &local,
        ..world(&idx, &on, &none)
    };
    let r = rows(&entries, &w);
    let mine = r.iter().find(|r| r.id == "mine").expect("listed");
    assert!(mine.local.is_some() && mine.entry.is_none() && mine.update.is_none());
    assert_eq!(mine.need, None, "it is built and needs no permission");
    // A package of `sunrise-mods/` put there by hand is listed by its name and needs neither
    // permission nor a build.
    let content = r.iter().find(|r| r.id == "by_hand").expect("listed");
    assert_eq!((content.title.as_str(), content.on), ("By Hand", true));
    assert_eq!((content.make, &content.need), (Some(Make::Content), &None));
    assert!(content.local.is_some() && content.entry.is_none());
    assert!(!bulk_updatable(mine));
    assert_eq!(r.len(), 6, "four managed mods and two hand-added mods");
}

#[test]
fn turning_on_a_local_native_file_exposes_its_build() {
    let home = tempfile::tempdir().unwrap();
    let on_list = home.path();
    let id = "example";
    let local = [Local {
        id: id.into(),
        file: format!("{id}.smod"),
        name: "Example".into(),
        version: "1.0".into(),
        github: None,
        native: true,
        served_by: None,
        unreadable: false,
    }];
    let unbuilt = HashSet::from([id.into()]);
    let row = || {
        let enabled = if mod_list::is_enabled(on_list, id) {
            HashSet::from([id.into()])
        } else {
            HashSet::new()
        };
        rows(
            &[],
            &World {
                v2: None,
                v2_failure: None,
                build: None,
                unfit_updates: &NO_UNFIT_UPDATES,
                enabled: &enabled,
                unbuilt: &unbuilt,
                chosen: &NO_PINS,
                local: &local,
            },
        )
        .remove(0)
    };
    let off = row();
    assert!(!off.on);
    assert!(cells(&off, None, None).switch);
    mod_list::set_enabled(on_list, id, true).unwrap();
    let on = row();
    assert!(on.on);
    assert_eq!(on.need, Some(Need::Build));
}

#[test]
fn turning_on_skips_what_is_taken_down() {
    let (idx, entries) = pack();
    let none = HashSet::new();
    let r = rows(&entries, &world(&idx, &none, &none));
    let all: Vec<&Row> = r.iter().collect();
    let (ok, held) = turn_on_plan(&all);
    assert_eq!(ok.len(), 3);
    assert_eq!(held.len(), 1, "taken down stays held");
}

#[test]
fn counts_read_short() {
    assert_eq!(size(2048), "2.0 KiB");
    assert_eq!(size(3 << 20), "3.0 MiB");
    assert!(downloads(12_400).starts_with("12.4k"));
    assert!(downloads(7).starts_with("7 "));
}

#[test]
fn every_reason_a_build_left_a_mod_out_is_shown_and_counted() {
    use native_mods::{Detail, LeftOut, Why};
    let (idx, entries) = pack();
    let on: HashSet<String> = ["lib", "addon", "old", "fine"].map(String::from).into();
    let none = HashSet::new();
    let local = ["lib", "addon", "old", "fine"].map(|id| Local {
        id: id.into(),
        file: format!("{id}.smod"),
        name: id.into(),
        version: "1.0".into(),
        github: None,
        native: true,
        served_by: None,
        unreadable: false,
    });
    let w = World {
        local: &local,
        ..world(&idx, &on, &none)
    };
    let mut r = rows(&entries, &w);
    let left = |id: &str, why| LeftOut {
        id: id.into(),
        version: "1.0".into(),
        why,
        detail: None,
    };
    let list = [
        left("lib", Why::DidNotBuild),
        left("addon", Why::Needs("lib".into())),
        LeftOut {
            detail: Some(Detail {
                made_for: Some("nightly-2026-11-16-abc".into()),
                changes: vec!["a hook moved".into()],
            }),
            ..left("old", Why::Unfit("made for Nov 16".into()))
        },
    ];
    mark_left_out(&mut r, &list, "Nov 17", |tag| format!("name of {tag}"));
    let row = |id: &str| r.iter().find(|r| r.id == id).unwrap();
    let failed = cells(row("lib"), None, None);
    let status = failed.status.unwrap();
    assert_eq!(status.label, "Failed");
    assert!(status.tip.contains("lib"));
    assert!(failed.unavailable);
    assert!(
        matches!(&row("addon").left_out, Some(Omission::Skipped(why)) if why.contains("needs lib"))
    );
    assert!(row("lib").unfit.is_none() && row("lib").need.is_none());
    let old = row("old").unfit.as_ref().unwrap();
    assert_eq!(
        old.made_for,
        Some((
            "nightly-2026-11-16-abc".into(),
            "name of nightly-2026-11-16-abc".into()
        ))
    );
    assert!(row("old").left_out.is_none());
    assert!(row("fine").unfit.is_none() && row("fine").left_out.is_none());
}

#[test]
fn every_row_fills_the_same_columns_whatever_it_has_to_say() {
    use native_mods::{Detail, LeftOut, Why};
    let (idx, entries) = pack();
    let on: HashSet<String> = ["lib", "addon", "old", "fine"].map(String::from).into();
    let unbuilt: HashSet<String> = ["addon".to_string()].into();
    let local = ["lib", "addon", "old", "fine"].map(|id| Local {
        id: id.into(),
        file: format!("{id}.smod"),
        name: id.into(),
        version: "1.0".into(),
        github: (id == "lib").then(|| sunrise_launcher::github_mods::Origin {
            id: id.to_owned(),
            version: "1.0".into(),
            owner: "publisher".into(),
            repo: "source".into(),
            tag: Some("v1.0".into()),
            commit: "a".repeat(40),
            asset: Some("lib.smod".into()),
            source: None,
            added_from: None,
            sha256: "b".repeat(64),
        }),
        native: true,
        served_by: None,
        unreadable: false,
    });
    let w = World {
        local: &local,
        ..world(&idx, &on, &unbuilt)
    };
    let mut r = rows(&entries, &w);
    mark_left_out(
        &mut r,
        &[LeftOut {
            id: "old".into(),
            version: "1.0".into(),
            why: Why::Unfit("made for Nov 16".into()),
            detail: Some(Detail {
                made_for: None,
                changes: vec!["src/lib.rs:3 builds it".into()],
            }),
        }],
        "Nov 17",
        |tag| tag.to_owned(),
    );
    let row = |id: &str| r.iter().find(|r| r.id == id && r.local.is_some()).unwrap();

    // Every row has a kind and a version for the same two columns.
    for row in &r {
        let cells = cells(row, None, None);
        assert_eq!(cells.version, row.version);
        assert_eq!(cells.kind, row.make.map_or("", Make::label));
    }
    // A mod that needs an update keeps the switch's place, and its status opens the popover:
    // no file or type name in the label or the tooltip.
    let old = cells(row("old"), None, None);
    let status = old.status.unwrap();
    assert_eq!(status.label, "Needs an update");
    assert_eq!(status.tone, Tone::Warning);
    assert!(old.popover);
    assert!(!status.tip.contains("src/lib.rs"), "{}", status.tip);
    assert_eq!(old.action, None);
    // A mod that waits for a build says so, and says Building while its setup's build runs.
    let addon = row("addon");
    assert_eq!(addon.need, Some(Need::Build));
    assert_eq!(
        cells(addon, None, None).status.unwrap().label,
        "Added · builds when you play"
    );
    for (at, label, tone) in [
        (BuildState::Waiting, "Waiting", Tone::Muted),
        (BuildState::Building, "Building", Tone::Accent),
        (BuildState::Built, "Built", Tone::Success),
        (
            BuildState::Failed("Missing hook".into()),
            "Failed",
            Tone::Warning,
        ),
        (
            BuildState::NeedsUpdate {
                why: "No build for this game".into(),
                details: Vec::new(),
            },
            "Needs an update",
            Tone::Warning,
        ),
    ] {
        let cells = cells(addon, None, Some(&at));
        let status = cells.status.unwrap();
        assert_eq!((status.label.as_str(), status.tone), (label, tone));
        match at {
            BuildState::Failed(why) | BuildState::NeedsUpdate { why, .. } => {
                assert!(status.tip.contains(&why));
                assert!(
                    cells.unavailable && cells.switch,
                    "can still turn the mod off"
                );
            }
            _ => assert!(!cells.unavailable),
        }
    }
    // The source is in the byline; it doesn't take up the status column.
    let fine = cells(row("fine"), None, None);
    assert!(fine.status.is_none());
    assert_eq!(fine.action, None);
    assert!(fine.switch, "local files can be switched on and off");
    // A registry row with nothing to say: empty status and action cells, a live switch.
    let plain = r
        .iter()
        .find(|r| r.local.is_none() && r.need.is_none() && r.update.is_none());
    if let Some(plain) = plain {
        let cells = cells(plain, None, None);
        assert_eq!(
            (cells.status, cells.action, cells.switch),
            (None, None, true)
        );
    }
    // The most pressing thing is the status; the rest is in its tooltip. The one button is
    // Update whenever a newer version is out.
    let mut both = row("addon").clone();
    both.update = Some("1.1".into());
    let cells_both = cells(&both, Some("the download stopped"), None);
    let status = cells_both.status.unwrap();
    assert_eq!(status.label, "Update failed");
    assert!(status.tip.contains("the download stopped"));
    assert!(status.tip.contains("Not built for this computer yet"));
    assert!(status.tip.contains("Version 1.1 is out"));
    assert_eq!(cells_both.action, Some(RowAction::Update));
}

#[test]
fn an_unreadable_file_is_never_on_and_cannot_switch_a_working_mod_of_its_name() {
    let on: HashSet<String> = HashSet::from(["ruby".to_owned()]);
    let none = HashSet::new();
    let (idx, _) = pack();
    let file = |unreadable| Local {
        id: "ruby".into(),
        file: "ruby".to_owned(),
        name: "ruby".into(),
        version: String::new(),
        github: None,
        native: true,
        served_by: None,
        unreadable,
    };
    let local = [file(true), file(false)];
    let w = World {
        local: &local,
        ..world(&idx, &on, &none)
    };
    let r = rows(&[], &w);
    let broken = r
        .iter()
        .find(|r| r.local.as_ref().is_some_and(|l| l.unreadable))
        .expect("listed");
    assert!(!broken.on);
    assert_eq!(broken.need, None);
    assert!(!cells(broken, None, None).switch);
    let good = r
        .iter()
        .find(|r| r.local.as_ref().is_some_and(|l| !l.unreadable))
        .expect("listed");
    assert!(good.on);
    assert_eq!(good.need, None);
    assert!(cells(good, None, None).switch);
}

#[test]
fn manual_mods_get_badges_and_never_enter_bulk_updates() {
    use sunrise_launcher::github_mods::{AddedFrom, Origin, SourceOrigin, SourceTrack};

    let (idx, _) = pack();
    let on = HashSet::from([
        "github_mod".to_owned(),
        "folder_mod".to_owned(),
        "local_smod".to_owned(),
        "discover_src".to_owned(),
    ]);
    let none = HashSet::new();
    let github_origin = Origin {
        id: "github_mod".into(),
        version: "0.4.0".into(),
        owner: "emberlane".into(),
        repo: "camp-lanterns".into(),
        commit: "1".repeat(40),
        sha256: A.into(),
        tag: None,
        asset: None,
        source: Some(SourceOrigin {
            repo_id: 99,
            manifest: "sunrise-mod.toml".into(),
            track: SourceTrack::Branch("main".into()),
            published_at: None,
        }),
        added_from: Some(AddedFrom::Github),
    };
    let folder_origin = Origin::local_folder(
        "folder_mod".into(),
        "0.2.0".into(),
        "/tmp/local-repo".into(),
        A.into(),
    );
    let discover_origin = Origin {
        id: "discover_src".into(),
        added_from: Some(AddedFrom::Discover),
        ..github_origin.clone()
    };
    let locals = [
        Local {
            id: "github_mod".into(),
            file: "sunrise-native-mods/github_mod.smod".into(),
            name: "Camp Lanterns".into(),
            version: "0.4.0".into(),
            github: Some(github_origin),
            native: true,
            served_by: None,
            unreadable: false,
        },
        Local {
            id: "folder_mod".into(),
            file: "sunrise-mods/folder_mod.smod".into(),
            name: "Folder Mod".into(),
            version: "0.2.0".into(),
            github: Some(folder_origin),
            native: false,
            served_by: None,
            unreadable: false,
        },
        Local {
            id: "local_smod".into(),
            file: "sunrise-native-mods/local_smod.smod".into(),
            name: "Lantern Tweaks".into(),
            version: "0.1.0".into(),
            github: None,
            native: true,
            served_by: None,
            unreadable: false,
        },
        Local {
            id: "discover_src".into(),
            file: "sunrise-native-mods/discover_src.smod".into(),
            name: "Discover Source".into(),
            version: "0.4.0".into(),
            github: Some(discover_origin),
            native: true,
            served_by: None,
            unreadable: false,
        },
    ];
    let w = World {
        local: &locals,
        ..world(&idx, &on, &none)
    };
    let mut all = rows(&[registry_entry("zoom", "1.0.0", false, A)], &w);
    // Even if an update string were attached, manual mods must never be bulk-updatable.
    for row in &mut all {
        row.update = Some("9.9.9".into());
    }
    let find = |id: &str| all.iter().find(|r| r.id == id).unwrap();
    assert_eq!(
        find("github_mod").manual_badge(),
        Some(ManualBadge::FromSource)
    );
    assert!(!bulk_updatable(find("github_mod")));

    assert_eq!(
        find("folder_mod").manual_badge(),
        Some(ManualBadge::FromSource)
    );
    assert!(!bulk_updatable(find("folder_mod")));

    assert_eq!(find("local_smod").manual_badge(), Some(ManualBadge::Local));
    assert!(!bulk_updatable(find("local_smod")));

    assert_eq!(find("discover_src").manual_badge(), None);

    assert_eq!(find("zoom").manual_badge(), None);
    assert!(bulk_updatable(find("zoom")));
}

#[test]
fn a_v2_installed_row_offers_the_newest_release_listed_for_its_build_and_shows_a_takedown() {
    let release = |version: &str, fits: &str, sha: char| {
        serde_json::json!({
            "version": version, "published": "2026-11-22", "sunrise": "26.3-0.1.0",
            "workflow": "w", "fits": [fits], "does_not_fit": [],
            "file": {"url": "u", "size": 1, "sha256": sha.to_string().repeat(64), "attestation": "a"},
            "dependencies": []
        })
    };
    let v2 = |revoked: serde_json::Value| -> index_v2::Index {
        serde_json::from_value(serde_json::json!({
            "schema": 2, "serial": 1, "generated": "2026-11-22T00:00:00Z", "builds": [], "categories": [],
            "projects": [{
                "id": "Ab3dE9xQ", "slug": "wayfinder", "mod": "wayfinder", "kind": "mod",
                "name": "Wayfinder", "summary": "", "description": "", "authors": ["Kit"],
                "owners": ["kit"], "license": "MIT", "icon": "i", "screenshots": [], "categories": [],
                "repository": {"id": 1, "name": "kit/wayfinder", "path": "."}, "workflow": "w",
                "releases": [
                    release("1.3.0", "26.3-0.2.0", 'd'),
                    release("1.2.0", "26.3-0.1.0", 'c'),
                    release("1.1.0", "26.3-0.1.0", 'b'),
                    release("1.0.0", "26.3-0.1.0", 'a'),
                ]
            }],
            "revoked": revoked
        }))
        .unwrap()
    };
    let mut installed = entry("Ab3dE9xQ", "1.0.0", false, &"a".repeat(64));
    installed.mod_id = Some("wayfinder".into());
    let on: HashSet<String> = ["wayfinder".to_owned()].into();
    let none = HashSet::new();
    let world = |index: &index_v2::Index| -> Row {
        rows(
            std::slice::from_ref(&installed),
            &World {
                v2: Some(index),
                v2_failure: None,
                build: Some("26.3-0.1.0"),
                unfit_updates: &NO_UNFIT_UPDATES,
                enabled: &on,
                unbuilt: &none,
                chosen: &NO_PINS,
                local: &[],
            },
        )
        .remove(0)
    };

    let listed = v2(serde_json::json!([]));
    let row = world(&listed);
    assert_eq!(row.title, "Wayfinder");
    assert!(row.v2_installed());
    // 1.3.0 is newer but listed for another build only.
    assert_eq!(row.update.as_deref(), Some("1.2.0"));
    assert_eq!(row.need, None);

    let taken_down = v2(serde_json::json!([{
        "project": "Ab3dE9xQ", "sha256": "a".repeat(64), "reason": "malware", "date": "2026-11-23"
    }]));
    assert_eq!(
        world(&taken_down).need,
        Some(Need::TakenDown("malware".into()))
    );
}

#[test]
fn a_v2_installed_row_says_why_the_registry_went_unchecked_only_when_a_read_failed() {
    let mut installed = entry("Ab3dE9xQ", "1.0.0", false, &"a".repeat(64));
    installed.mod_id = Some("wayfinder".into());
    let none = HashSet::new();
    let row = |v2_failure| {
        rows(
            std::slice::from_ref(&installed),
            &World {
                v2: None,
                v2_failure,
                build: Some("26.3-0.1.0"),
                unfit_updates: &NO_UNFIT_UPDATES,
                enabled: &none,
                unbuilt: &none,
                chosen: &NO_PINS,
                local: &[],
            },
        )
        .remove(0)
    };
    assert_eq!(
        row(None).registry_issue,
        None,
        "nothing while the first read is out"
    );
    let unreachable = "Registry not reachable: connection refused";
    assert_eq!(
        row(Some(unreachable)).registry_issue.as_deref(),
        Some(unreachable)
    );
}

#[test]
fn a_served_mod_needs_its_host_on() {
    let local = |id: &str, native: bool, served_by: Option<&str>| Local {
        id: id.into(),
        file: format!("x/{id}.smod"),
        name: id.into(),
        version: "1.0.0".into(),
        github: None,
        native,
        served_by: served_by.map(str::to_owned),
        unreadable: false,
    };
    let locals = [
        Local {
            name: "Java Host".into(),
            ..local("java_host", true, None)
        },
        local("runes", false, Some("java_host")),
    ];
    let none = HashSet::new();
    let shown = |enabled: &HashSet<String>| {
        rows(
            &[],
            &World {
                v2: None,
                v2_failure: None,
                build: None,
                unfit_updates: &NO_UNFIT_UPDATES,
                enabled,
                unbuilt: &none,
                chosen: &NO_PINS,
                local: &locals,
            },
        )
    };
    let runes = |rows: &[Row]| rows.iter().find(|r| r.id == "runes").unwrap().clone();

    // On without its host: the game leaves it out, and the row says what it needs.
    let alone: HashSet<String> = ["runes".to_owned()].into();
    let row = runes(&shown(&alone));
    assert_eq!(row.need, Some(Need::Host("Java Host".into())));
    assert_eq!(row.group(), Group::NeedsYou);
    let both: HashSet<String> = ["runes".to_owned(), "java_host".to_owned()].into();
    assert_eq!(runes(&shown(&both)).need, None);
}

#[test]
fn an_old_registry_row_takes_the_v2_project_whose_release_has_its_archive() {
    let idx = index(
        vec![project("zoom", "mod", &[("2.0.0", B), ("1.0.0", A)])],
        vec![],
    );
    let none = HashSet::new();
    let w = world(&idx, &none, &none);
    // Old receipts name the mod, not a v2 project, and carry no `mod_id`.
    let adopted = entry("zoom", "1.0.0", false, A);
    let row = rows(std::slice::from_ref(&adopted), &w).remove(0);
    assert_eq!(row.title, "ZOOM");
    assert_eq!(row.authors, "Someone");
    assert_eq!(row.update.as_deref(), Some("2.0.0"));
    assert!(bulk_updatable(&row));

    // An archive no v2 release has stays as it was, with no updates.
    let unknown = entry("zoom", "1.0.0", false, &"c".repeat(64));
    let row = rows(std::slice::from_ref(&unknown), &w).remove(0);
    assert_eq!(row.update, None);
    assert_eq!(row.authors, "");

    // A whole-project takedown reaches the adopted row through its v2 project.
    let taken = index(
        vec![project("zoom", "mod", &[("2.0.0", B), ("1.0.0", A)])],
        vec![serde_json::json!({
            "project": project_id("zoom"), "reason": "unsafe", "date": "2026-11-01"
        })],
    );
    let row = rows(std::slice::from_ref(&adopted), &world(&taken, &none, &none)).remove(0);
    assert_eq!(row.need, Some(Need::TakenDown("unsafe".into())));
}
