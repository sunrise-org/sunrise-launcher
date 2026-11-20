use super::*;
use sunrise_launcher::browser::installed::Entry;

const GAME: &str = "26.3";

fn project(
    id: &str,
    kind: &str,
    versions: &[(&str, &str, &str)],
    extra: serde_json::Value,
) -> serde_json::Value {
    let mut p = serde_json::json!({
        "id": id, "kind": kind, "name": id.to_uppercase(), "summary": format!("{id} does things"),
        "authors": ["Someone"], "license": "MIT", "source": "https://github.com/x/y",
        "versions": versions.iter().map(|(v, game, sha)| serde_json::json!({
            "version": v, "published": format!("2026-11-{:02}", v.len() + 10), "api": "0.2", "game": game,
            "file": {"name": format!("{id}-{v}.smod"), "url": "https://example.test/x.smod", "size": 2048, "sha256": sha}
        })).collect::<Vec<_>>(),
    });
    p.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    p
}

fn index(projects: Vec<serde_json::Value>, revoked: Vec<serde_json::Value>) -> Index {
    serde_json::from_value(
        serde_json::json!({"schema": 1, "projects": projects, "revoked": revoked}),
    )
    .unwrap()
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

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

static NO_PINS: std::sync::LazyLock<std::collections::BTreeMap<String, String>> =
    std::sync::LazyLock::new(Default::default);

static NO_WAITING: std::sync::LazyLock<
    std::collections::BTreeMap<String, sunrise_launcher::browser::sunrise::txn::WaitingMod>,
> = std::sync::LazyLock::new(Default::default);

fn world<'a>(index: &'a Index, enabled: &'a HashSet<String>) -> World<'a> {
    World {
        index: Some(index),
        game: GAME,
        enabled,
        chosen: &NO_PINS,
        waiting: &NO_WAITING,
        local: &[],
    }
}

#[test]
fn discover_lists_mods_not_packs_and_says_which_fit() {
    let idx = index(
        vec![
            project(
                "zoom",
                "mod",
                &[("1.0.0", GAME, A)],
                serde_json::json!({"downloads": 50, "categories": ["utility"]}),
            ),
            project(
                "deep",
                "native-mod",
                &[("1.0.0", "25.1", A)],
                serde_json::json!({"downloads": 900}),
            ),
            project(
                "cozy",
                "modpack",
                &[("1.0.0", GAME, A)],
                serde_json::json!({}),
            ),
        ],
        vec![],
    );
    let cards = cards(&idx, GAME);
    assert_eq!(
        cards.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        ["zoom", "deep"],
        "no modpack"
    );
    assert!(cards[0].fit.fits() && cards[0].make == Make::Content);
    assert_eq!(cards[1].fit, Fit::OtherBuild("25.1".into()));
    assert_eq!(cards[1].make, Make::Native);

    let mut f = Filters::default();
    assert_eq!(discover(&cards, &f), [0], "Fits my game is on by default");
    f.fits_only = false;
    assert_eq!(discover(&cards, &f), [1, 0], "most downloaded first");
    f.make = Some(Make::Content);
    assert_eq!(discover(&cards, &f), [0]);
    f.make = None;
    f.category = Some("utility".into());
    assert_eq!(discover(&cards, &f), [0]);
    f.category = None;
    f.text = "DEEP".into();
    assert_eq!(discover(&cards, &f), [1]);
    f.text = "nothing like it".into();
    assert!(discover(&cards, &f).is_empty());
}

#[test]
fn a_taken_down_version_is_refused_with_the_registrys_reason() {
    let idx = index(
        vec![project(
            "zoom",
            "mod",
            &[("1.0.0", GAME, A)],
            serde_json::json!({}),
        )],
        vec![
            serde_json::json!({"id": "zoom", "sha256": A, "reason": "it crashed worlds", "date": "2026-11-01"}),
        ],
    );
    let c = &cards(&idx, GAME)[0];
    assert_eq!(c.fit, Fit::Refused("Taken down: it crashed worlds".into()));
    let page = page(&idx, "zoom", GAME, vec![], false).unwrap();
    assert!(page.will_install.is_none(), "nothing to Add");
    assert!(!page.versions[0].fit.fits());
}

#[test]
fn the_page_names_the_version_add_installs() {
    let idx = index(
        vec![project(
            "zoom",
            "mod",
            &[("2.0.0", GAME, B), ("1.0.0", "25.1", A)],
            serde_json::json!({"downloads": 7}),
        )],
        vec![],
    );
    let p = page(&idx, "zoom", GAME, vec!["Made by Someone".into()], false).unwrap();
    assert_eq!(
        p.will_install.as_ref().map(|v| v.number.as_str()),
        Some("2.0.0")
    );
    assert_eq!(p.versions[1].fit, Fit::OtherBuild("25.1".into()));
    assert_eq!(p.side, "Not needed on servers");
    assert!(page(&idx, "missing", GAME, vec![], false).is_none());
}

#[test]
fn an_update_is_only_ever_forward_to_what_fits() {
    let idx = index(
        vec![project(
            "zoom",
            "mod",
            &[("3.0.0", GAME, B), ("2.0.0", GAME, A), ("1.0.0", GAME, A)],
            serde_json::json!({}),
        )],
        vec![],
    );
    let p = idx.project("zoom").unwrap();
    assert_eq!(newer(&idx, p, "2.0.0", GAME), Some("3.0.0".into()));
    assert_eq!(newer(&idx, p, "3.0.0", GAME), None);
    assert_eq!(
        newer(&idx, p, "0.9.0", GAME),
        None,
        "a version the index doesn't list is left alone"
    );
    assert_eq!(
        newer(&idx, p, "2.0.0", "25.1"),
        None,
        "nothing for another build"
    );
}

fn pack() -> (Index, Vec<Entry>) {
    let idx = index(
        vec![
            project(
                "zoom",
                "mod",
                &[("2.0.0", GAME, B), ("1.0.0", GAME, A)],
                serde_json::json!({}),
            ),
            project(
                "bad",
                "mod",
                &[("2.0.0", GAME, B), ("1.0.0", GAME, A)],
                serde_json::json!({}),
            ),
            project(
                "deep",
                "native-mod",
                &[("2.0.0", GAME, B), ("1.0.0", GAME, A)],
                serde_json::json!({}),
            ),
            project("quiet", "mod", &[("1.0.0", GAME, A)], serde_json::json!({})),
        ],
        vec![
            serde_json::json!({"id": "bad", "sha256": A, "reason": "unsafe", "date": "2026-11-01"}),
        ],
    );
    let entries = vec![
        entry("quiet", "1.0.0", false, A),
        entry("zoom", "1.0.0", false, A),
        entry("deep", "1.0.0", true, A),
        entry("bad", "1.0.0", false, A),
    ];
    (idx, entries)
}

#[test]
fn rows_are_listed_by_name_and_a_native_mod_is_just_a_row() {
    let (idx, entries) = pack();
    let enabled: HashSet<String> = ["quiet", "zoom", "deep", "bad"].map(String::from).into();
    let r = rows(&entries, &world(&idx, &enabled));
    let order: Vec<&str> = r.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(order, ["bad", "deep", "quiet", "zoom"]);
    let row = |id: &str| r.iter().find(|r| r.id == id).unwrap();
    assert_eq!(row("bad").need, Some(Need::TakenDown("unsafe".into())));
    assert_eq!(
        row("bad").update.as_deref(),
        Some("2.0.0"),
        "the way out of a takedown is shown too"
    );
    assert_eq!(row("deep").make, Some(Make::Native));
    assert!(
        row("deep").need.is_none(),
        "no row says it needs a build or permission"
    );
    assert_eq!(row("zoom").update.as_deref(), Some("2.0.0"));
}

#[test]
fn registry_installed_rows_use_live_names_and_statuses_or_ids_and_not_checked() {
    let (idx, mut entries) = pack();
    let bad = entries.iter_mut().find(|e| e.project == "bad").unwrap();
    bad.title = "Unverified saved name".into();
    let none = HashSet::new();
    let live = world(&idx, &none);
    let row = rows(std::slice::from_ref(bad), &live).pop().unwrap();
    assert_eq!(row.title, "BAD");
    assert_eq!(row.authors, "Someone");
    assert_eq!(row.need, Some(Need::TakenDown("unsafe".into())));
    assert_eq!(row.update.as_deref(), Some("2.0.0"));
    assert!(!row.registry_unchecked);

    let offline = World {
        index: None,
        ..live
    };
    let row = rows(std::slice::from_ref(bad), &offline).pop().unwrap();
    assert_eq!(row.title, row.id);
    assert!(row.authors.is_empty());
    assert!(row.icon.is_none());
    assert!(row.need.is_none());
    assert!(row.update.is_none());
    assert!(row.registry_unchecked);
}

#[test]
fn only_registry_mods_are_rows_packs_are_left_alone() {
    let (idx, mut entries) = pack();
    let mut other = entry("shaderish", "1", false, A);
    other.kind = sunrise_launcher::browser::Kind::Shader;
    entries.push(other);
    let r = rows(&entries, &world(&idx, &HashSet::new()));
    assert_eq!(r.len(), 4);
    assert!(r.iter().all(|r| r.id != "shaderish"));
}

#[test]
fn filters_narrow_the_shown_rows_and_never_reorder_them() {
    let (idx, entries) = pack();
    let on: HashSet<String> = ["quiet", "zoom"].map(String::from).into();
    let r = rows(&entries, &world(&idx, &on));
    let ids = |f: &RowFilter| {
        visible(&r, f)
            .into_iter()
            .map(|i| r[i].id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&RowFilter::default()), ["bad", "deep", "quiet", "zoom"]);
    assert_eq!(
        ids(&RowFilter {
            show: Show::On,
            ..Default::default()
        }),
        ["quiet", "zoom"]
    );
    assert_eq!(
        ids(&RowFilter {
            show: Show::Off,
            ..Default::default()
        }),
        ["bad", "deep"]
    );
    assert_eq!(
        ids(&RowFilter {
            makes: [Make::Native].into(),
            ..Default::default()
        }),
        ["deep"]
    );
    assert_eq!(
        ids(&RowFilter {
            text: "QUI".into(),
            ..Default::default()
        }),
        ["quiet"]
    );
}

#[test]
fn update_all_covers_the_whole_pack_but_a_version_the_player_chose() {
    let (idx, entries) = pack();
    let on: HashSet<String> = ["zoom", "bad", "deep"].map(String::from).into();
    let r = rows(&entries, &world(&idx, &on));
    let ids = |rows: &[Row]| -> Vec<String> {
        let mut ids: Vec<String> = updatable(rows).iter().map(|r| r.id.clone()).collect();
        ids.sort();
        ids
    };
    // A native mod is in it: the install asks for permission itself. A taken-down mod is updated
    // to a file that wasn't.
    assert_eq!(ids(&r), ["bad", "deep", "zoom"]);
    // A mod that is off is in it too.
    let only_zoom: HashSet<String> = ["zoom".to_string()].into();
    let off = rows(&entries, &world(&idx, &only_zoom));
    assert!(off
        .iter()
        .any(|r| r.id == "bad" && !r.on && r.update.is_some()));
    assert_eq!(ids(&off), ["bad", "deep", "zoom"]);
    // A version the player chose is never moved by it.
    let pins = std::collections::BTreeMap::from([("zoom".to_string(), "1.0.0".to_string())]);
    let w = World {
        chosen: &pins,
        ..world(&idx, &on)
    };
    assert_eq!(ids(&rows(&entries, &w)), ["bad", "deep"]);
    // Nothing waiting: no button.
    assert!(updatable(&[]).is_empty());
}

#[test]
fn a_row_with_a_newer_version_says_update_available() {
    let (idx, entries) = pack();
    let on: HashSet<String> = ["zoom"].map(String::from).into();
    let r = rows(&entries, &world(&idx, &on));
    let zoom = r.iter().find(|r| r.id == "zoom").unwrap();
    let cells = cells(zoom, None);
    assert_eq!(cells.status.unwrap().label, "Update available");
    assert!(cells.action.is_some());
    let quiet = r.iter().find(|r| r.id == "quiet").unwrap();
    assert!(self::cells(quiet, None).action.is_none());
}

#[test]
fn a_row_the_plan_cannot_update_says_why() {
    let (idx, entries) = pack();
    let on: HashSet<String> = ["zoom"].map(String::from).into();
    let pins = std::collections::BTreeMap::from([("zoom".to_string(), "1.0.0".to_string())]);
    let w = World {
        chosen: &pins,
        ..world(&idx, &on)
    };
    let r = rows(&entries, &w);
    let zoom = r.iter().find(|r| r.id == "zoom").unwrap();
    assert!(update_refusal(zoom).is_some_and(|w| w.contains("you chose")));
    let tip = cells(zoom, None).status.expect("says something").tip;
    assert!(tip.contains("you chose its version"), "{tip}");
}

#[test]
fn native_files_the_browser_didnt_install_are_listed_and_never_planned() {
    let (idx, entries) = pack();
    let on: HashSet<String> = ["mine", "zoom", "by_hand"].map(String::from).into();
    let local = [
        Local {
            id: "mine".into(),
            name: "mine".into(),
            version: "0.3".into(),
            origin: None,
            native: true,
            unreadable: false,
        },
        Local {
            id: "by_hand".into(),
            name: "By Hand".into(),
            version: "1.0".into(),
            origin: None,
            native: false,
            unreadable: false,
        },
    ];
    let w = World {
        local: &local,
        ..world(&idx, &on)
    };
    let r = rows(&entries, &w);
    let mine = r.iter().find(|r| r.id == "mine").expect("listed");
    assert!(mine.local.is_some() && mine.entry.is_none() && mine.update.is_none());
    // A package of `sunrise-mods/` put there by hand is listed by its name and needs neither
    // permission nor a build.
    let content = r.iter().find(|r| r.id == "by_hand").expect("listed");
    assert_eq!((content.title.as_str(), content.on), ("By Hand", true));
    assert_eq!((content.make, &content.need), (None, &None));
    assert!(content.local.is_some() && content.entry.is_none());
    assert!(updatable(&r).iter().all(|r| r.id != "mine"));
}

#[test]
fn turning_on_skips_what_is_taken_down_or_needs_permission() {
    let (idx, entries) = pack();
    let r = rows(&entries, &world(&idx, &HashSet::new()));
    let all: Vec<&Row> = r.iter().collect();
    let (ok, held) = turn_on_plan(&all, false);
    assert_eq!(
        ok.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["quiet", "zoom"]
    );
    assert_eq!(held.len(), 2);
    let (ok, held) = turn_on_plan(&all, true);
    assert_eq!(ok.len(), 3);
    assert_eq!(held.len(), 1, "taken down stays held even with permission");
}

#[test]
fn removing_a_mod_names_what_still_needs_it() {
    let mut idx = index(
        vec![
            project("lib", "mod", &[("1.0.0", GAME, A)], serde_json::json!({})),
            project(
                "uses_lib",
                "mod",
                &[("1.0.0", GAME, B)],
                serde_json::json!({}),
            ),
        ],
        vec![],
    );
    idx.projects[1].versions[0].requires = vec![index::Requirement {
        id: "lib".into(),
        version: "1.0.0".into(),
        sha256: A.into(),
    }];
    let entries = vec![
        entry("lib", "1.0.0", false, A),
        entry("uses_lib", "1.0.0", false, B),
    ];
    let r = rows(&entries, &world(&idx, &HashSet::new()));
    let removing: HashSet<String> = ["lib".to_string()].into();
    assert_eq!(
        dependents(&r, &removing, Some(&idx)),
        [("USES_LIB".to_string(), vec!["LIB".to_string()])]
    );
    let both: HashSet<String> = ["lib", "uses_lib"].map(String::from).into();
    assert!(dependents(&r, &both, Some(&idx)).is_empty());
    assert!(dependents(&r, &removing, None).is_empty());
}

#[test]
fn counts_read_short() {
    assert_eq!(size(2048), "2.0 KiB");
    assert_eq!(size(3 << 20), "3.0 MiB");
    assert!(downloads(12_400).starts_with("12.4k"));
    assert!(downloads(7).starts_with("7 "));
}

// ---------------------------------------------------------------- collections

fn needs(id: &str, range: &str) -> Option<Vec<index::Dependency>> {
    Some(vec![index::Dependency {
        id: id.into(),
        version: range.into(),
        optional: false,
        side: None,
    }])
}

/// A collection of `cow`, `pig` and `owl`, with `owl` needing `cow`, and `cow` and `pig` needing
/// `lib`, which no collection lists; and `jei`, which belongs to none.
fn zoo() -> Index {
    let mut idx = index(
        ["lib", "cow", "pig", "owl", "jei"]
            .map(|id| {
                project(
                    id,
                    "native-mod",
                    &[("1.0.0", GAME, A)],
                    match id {
                        "lib" => serde_json::json!({"downloads": 10, "categories": ["library"]}),
                        _ => serde_json::json!({"downloads": 10}),
                    },
                )
            })
            .to_vec(),
        vec![],
    );
    let mut dependencies = |id: &str, dep: Option<Vec<index::Dependency>>| {
        idx_version(&mut idx, id).dependencies = dep;
    };
    dependencies("cow", needs("lib", "^1"));
    dependencies("pig", needs("lib", "^1"));
    dependencies("owl", needs("cow", "^1"));
    idx.collections = vec![serde_json::from_value(serde_json::json!({
        "id": "farm", "name": "Farm", "summary": "Animals.", "owners": ["Someone"],
        "icon": "https://example.test/farm.png",
        "members": [{"id": "cow"}, {"id": "pig"}, {"id": "owl", "default": false}],
    }))
    .unwrap()];
    idx
}

fn idx_version<'a>(idx: &'a mut Index, id: &str) -> &'a mut index::Version {
    &mut idx
        .projects
        .iter_mut()
        .find(|p| p.id == id)
        .unwrap()
        .versions[0]
}

fn ticked(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

fn farm(idx: &Index, picks: &[&str]) -> collection::View {
    let c = idx.collection("farm").unwrap();
    let base = collection::base(idx, c, &[], GAME);
    collection::view(idx, c, &base, &ticked(picks), &[], GAME)
}

#[test]
fn discover_folds_members_into_the_collection_and_hides_libraries_until_searched() {
    let idx = zoo();
    let all = cards(&idx, GAME);
    let names = |f: &Filters| -> Vec<String> {
        discover(&all, f)
            .into_iter()
            .map(|i| all[i].id.clone())
            .collect()
    };
    let mut f = Filters {
        sort: Sort::Name,
        ..Filters::default()
    };
    assert_eq!(
        names(&f),
        ["farm", "jei"],
        "the card, not its three mods or the library"
    );
    let farm = all.iter().find(|c| c.id == "farm").unwrap();
    assert_eq!(farm.members, Some(3));
    assert_eq!(farm.downloads, 30, "its mods' downloads added up");
    assert!(all.iter().find(|c| c.id == "lib").unwrap().library);
    assert!(!all.iter().find(|c| c.id == "jei").unwrap().library);
    // A search finds a member and a library like any other mod.
    f.text = "cow".into();
    assert_eq!(names(&f), ["cow"]);
    f.text = "lib".into();
    assert_eq!(names(&f), ["lib"]);
    f.text = "farm".into();
    assert_eq!(names(&f), ["farm"]);
}

#[test]
fn a_member_says_which_collection_it_is_part_of() {
    let idx = zoo();
    let all = cards(&idx, GAME);
    let part = |id: &str| all.iter().find(|c| c.id == id).unwrap().part_of.clone();
    assert_eq!(part("cow").as_deref(), Some("Farm"));
    assert_eq!(part("owl").as_deref(), Some("Farm"));
    assert_eq!(part("lib"), None, "a library is in no collection");
    assert_eq!(part("jei"), None);
    assert_eq!(part("farm"), None);
}

#[test]
fn the_header_box_is_empty_dashed_or_ticked_by_how_many_members_are() {
    assert_eq!(collection::ticks(0, 3), collection::Ticks::None);
    assert_eq!(collection::ticks(1, 3), collection::Ticks::Some);
    assert_eq!(collection::ticks(3, 3), collection::Ticks::All);
    let idx = zoo();
    assert_eq!(farm(&idx, &[]).header, collection::Ticks::None);
    assert_eq!(farm(&idx, &["pig"]).header, collection::Ticks::Some);
    assert_eq!(
        farm(&idx, &["cow", "pig", "owl"]).header,
        collection::Ticks::All
    );

    // Clicking it ticks all but a library, or clears them all.
    let v = farm(&idx, &["pig"]);
    let mut t = ticked(&["pig"]);
    v.flip_all(&mut t);
    assert_eq!(t, ticked(&["cow", "pig", "owl"]));
    farm(&idx, &["cow", "pig", "owl"]).flip_all(&mut t);
    assert!(t.is_empty());
}

#[test]
fn what_a_ticked_mod_needs_is_ticked_and_locked_while_it_is_needed() {
    let idx = zoo();
    let v = farm(&idx, &["cow", "pig", "owl"]);
    let line =
        |v: &collection::View, id: &str| v.lines.iter().find(|l| l.id == id).unwrap().clone();
    assert_eq!(
        v.lines.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(),
        ["cow", "pig", "owl", "lib"],
        "the members in order, then the library"
    );
    let lib = line(&v, "lib");
    assert!(lib.library && lib.ticked);
    assert_eq!(
        lib.needed_by,
        ["COW", "PIG", "OWL"],
        "in the collection's order, and including what needs it through another pick"
    );
    let cow = line(&v, "cow");
    assert!(
        cow.ticked && cow.locked(),
        "a member another pick needs is locked too"
    );
    assert_eq!(cow.needed_by, ["OWL"]);
    assert!(!line(&v, "owl").locked());

    // A locked box does not flip; its dependent's does, and the lock goes with it.
    let mut t = ticked(&["cow", "pig", "owl"]);
    v.flip(&mut t, "cow");
    v.flip(&mut t, "lib");
    assert_eq!(t, ticked(&["cow", "pig", "owl"]));
    v.flip(&mut t, "owl");
    let v = farm(&idx, &["cow", "pig"]);
    assert!(!line(&v, "cow").locked());
    assert_eq!(line(&v, "lib").needed_by, ["COW", "PIG"]);
    let v = farm(&idx, &[]);
    let lib = line(&v, "lib");
    assert!(!lib.ticked && !lib.locked(), "nothing ticked needs it");
}

#[test]
fn the_plan_line_counts_the_plans_mods_and_libraries() {
    let say = |mods, libraries, updates| {
        collection::Summary {
            mods,
            libraries,
            updates,
            bytes: 0,
        }
        .line()
    };
    assert_eq!(say(8, 1, 0), "Installs 8 mods and 1 library");
    assert_eq!(say(1, 0, 0), "Installs 1 mod");
    assert_eq!(say(0, 2, 0), "Installs 2 libraries");
    assert_eq!(
        say(2, 1, 1),
        "Installs 2 mods and 1 library. Updates 1 installed mod"
    );
    assert_eq!(say(0, 0, 0), "Everything you picked is installed.");

    // From a real plan: two members, their library once, and nothing once they are held.
    let idx = zoo();
    let v = farm(&idx, &["cow", "pig"]);
    assert_eq!(v.summary.line(), "Installs 2 mods and 1 library");
    assert_eq!(v.summary.bytes, 3 * 2048);
    let held = ["cow", "pig", "lib"].map(|id| index::resolve::InstalledMod {
        id: id.into(),
        version: "1.0.0".into(),
        sha256: Some(A.into()),
    });
    let c = idx.collection("farm").unwrap();
    let base = collection::base(&idx, c, &held, GAME);
    let v = collection::view(&idx, c, &base, &ticked(&["cow", "pig"]), &held, GAME);
    assert!(v.summary.nothing(), "{:?}", v.summary);
}

#[test]
fn a_member_another_pick_needs_is_ticked_and_counted_though_it_was_left_unticked() {
    let idx = zoo();
    let v = farm(&idx, &["owl"]);
    let cow = v.lines.iter().find(|l| l.id == "cow").unwrap();
    assert!(
        cow.ticked && cow.locked(),
        "owl needs cow: shown ticked, not unticked and locked"
    );
    assert_eq!(cow.needed_by, ["OWL"]);
    assert_eq!(
        (v.ticked, v.members),
        (2, 3),
        "counted in 'N of M selected'"
    );
    assert_eq!(v.header, collection::Ticks::Some);
    assert_eq!(
        v.summary.line(),
        "Installs 2 mods and 1 library",
        "owl, cow and lib"
    );
    // Install is given the one it needs too: it is ticked.
    assert_eq!(v.to_install(), ["cow", "owl"]);
}

#[test]
fn a_library_is_marked_by_the_registry_and_needed_by_another_mod() {
    let mut idx = zoo();
    let mut add = |id: &str, extra: serde_json::Value| {
        idx.projects.push(
            serde_json::from_value(project(id, "native-mod", &[("1.0.0", GAME, A)], extra))
                .unwrap(),
        );
    };
    // `tool` has content of its own and is needed by `jei`; `solo` is marked a library but
    // nothing needs it; `lib` is marked and needed by the farm's mods.
    add("tool", serde_json::json!({"downloads": 5}));
    add(
        "solo",
        serde_json::json!({"downloads": 4, "categories": ["utility", "library"]}),
    );
    idx_version(&mut idx, "jei").dependencies = needs("tool", "^1");
    let all = cards(&idx, GAME);
    let card = |id: &str| all.iter().find(|c| c.id == id).unwrap();
    assert!(card("lib").library);
    assert!(!card("tool").library, "needed, but not marked: still a mod");
    assert!(
        !card("solo").library,
        "marked, but needed by nothing: a mod of its own"
    );
    assert!(!card("cow").library);
    let f = Filters {
        sort: Sort::Name,
        ..Filters::default()
    };
    let browse: Vec<&str> = discover(&all, &f)
        .into_iter()
        .map(|i| all[i].id.as_str())
        .collect();
    assert_eq!(
        browse,
        ["farm", "jei", "solo", "tool"],
        "only the library goes"
    );
}

#[test]
fn picks_the_planner_cannot_install_together_say_so() {
    let mut idx = zoo();
    // Two more mods need incompatible lines of one library.
    idx_version(&mut idx, "lib").version = "2.0.0".into();
    let mut older = idx
        .projects
        .iter()
        .find(|p| p.id == "lib")
        .unwrap()
        .versions[0]
        .clone();
    older.version = "1.0.0".into();
    older.file.sha256 = B.into();
    idx.projects
        .iter_mut()
        .find(|p| p.id == "lib")
        .unwrap()
        .versions
        .push(older);
    idx_version(&mut idx, "cow").dependencies = needs("lib", "^1");
    idx_version(&mut idx, "pig").dependencies = needs("lib", "^2");
    let v = farm(&idx, &["cow", "pig"]);
    assert!(v.conflict.is_some(), "{:?}", v.lines);
    let v = farm(&idx, &["cow"]);
    assert!(v.conflict.is_none());
}

#[test]
fn the_planner_runs_again_only_when_the_picks_or_what_is_installed_change() {
    let idx = std::sync::Arc::new(zoo());
    let c = idx.collection("farm").unwrap().clone();
    let mut planned = collection::Planned::default();
    let picks = ticked(&["cow", "pig"]);
    for _ in 0..3 {
        planned.view(&idx, &c, &picks, &[], GAME);
    }
    assert_eq!(planned.worked, (1, 1), "three draws, one run of each");
    planned.view(&idx, &c, &ticked(&["pig"]), &[], GAME);
    assert_eq!(
        planned.worked,
        (1, 2),
        "a tick changes only the picks' plan"
    );
    let held = [index::resolve::InstalledMod {
        id: "lib".into(),
        version: "1.0.0".into(),
        sha256: None,
    }];
    planned.view(&idx, &c, &ticked(&["pig"]), &held, GAME);
    assert_eq!(planned.worked, (2, 3), "an install changes both");
}

#[test]
fn install_is_given_only_the_ticked_mods_not_yet_installed_and_stops_at_the_first_failure() {
    let idx = zoo();
    let c = idx.collection("farm").unwrap();
    let held = [index::resolve::InstalledMod {
        id: "cow".into(),
        version: "1.0.0".into(),
        sha256: Some(A.into()),
    }];
    let base = collection::base(&idx, c, &held, GAME);
    let v = collection::view(&idx, c, &base, &ticked(&["cow", "pig"]), &held, GAME);
    assert_eq!(
        v.to_install(),
        ["pig"],
        "not cow (installed), owl (unticked) or lib (a library)"
    );
}

#[test]
fn a_collection_reads_installed_when_the_members_it_starts_with_and_can_install_are() {
    let mut idx = zoo();
    idx.collections[0].members[2].default = false; // owl
    idx_version(&mut idx, "pig").game = "25.1".into(); // this build can't take pig
    let all = cards(&idx, GAME);
    let c = idx.collection("farm").unwrap();
    let have = |ids: &[&str]| -> std::collections::HashSet<String> {
        ids.iter().map(|s| (*s).to_owned()).collect()
    };
    assert!(!installed_card(c, &all, &have(&[])));
    assert!(
        installed_card(c, &all, &have(&["cow"])),
        "owl is optional and pig can't be installed"
    );
}

fn installed_card(
    c: &sunrise_launcher_smod::index::Collection,
    cards: &[Card],
    have: &std::collections::HashSet<String>,
) -> bool {
    collection::installed(c, cards, have)
}

/// The farm where `cow`, a member, is itself a library: `owl` needs it, and it needs `lib`.
fn zoo_with_a_member_library() -> Index {
    let mut idx = zoo();
    idx.projects
        .iter_mut()
        .find(|p| p.id == "cow")
        .unwrap()
        .categories = vec!["library".into()];
    idx
}

#[test]
fn the_plan_line_and_the_header_count_the_same_picks_in_the_same_words() {
    let idx = zoo_with_a_member_library();
    let say = |picks: &[&str]| {
        let v = farm(&idx, picks);
        (
            v.plan_line(),
            v.can_install(),
            format!("{} of {}", v.ticked, v.members),
        )
    };
    // One pick whose library is locked in: the library is a library, however it got in.
    assert_eq!(
        say(&["pig"]),
        ("Installs 1 mod and 1 library".into(), true, "1 of 3".into())
    );
    // A member that is a library, pulled in by the pick that needs it, counts as selected.
    assert_eq!(
        say(&["owl"]),
        (
            "Installs 1 mod and 2 libraries".into(),
            true,
            "2 of 3".into()
        )
    );
    // All picked: the member library and the pulled-in one are libraries, the rest mods.
    assert_eq!(
        say(&["cow", "pig", "owl"]),
        (
            "Installs 2 mods and 2 libraries".into(),
            true,
            "3 of 3".into()
        )
    );
    // The member library alone.
    assert_eq!(
        say(&["cow"]),
        ("Installs 2 libraries".into(), true, "1 of 3".into())
    );
    // None picked: the card says so, and Install is off.
    assert_eq!(
        say(&[]),
        ("Pick the mods you want.".into(), false, "0 of 3".into())
    );
}

#[test]
fn install_is_off_when_everything_picked_is_installed() {
    let idx = zoo();
    let c = idx.collection("farm").unwrap();
    let held = ["cow", "pig", "lib"].map(|id| index::resolve::InstalledMod {
        id: id.into(),
        version: "1.0.0".into(),
        sha256: Some(A.into()),
    });
    let base = collection::base(&idx, c, &held, GAME);
    let v = collection::view(&idx, c, &base, &ticked(&["cow", "pig"]), &held, GAME);
    assert_eq!(v.plan_line(), "Everything you picked is installed.");
    assert!(v.done() && !v.can_install());
}

#[test]
fn an_unreadable_file_is_never_on_and_cannot_switch_a_working_mod_of_its_name() {
    let on: HashSet<String> = HashSet::from(["ruby".to_owned()]);
    let (idx, _) = pack();
    let file = |unreadable| Local {
        id: "ruby".into(),
        name: "ruby".into(),
        version: String::new(),
        origin: None,
        native: true,
        unreadable,
    };
    let local = [file(true), file(false)];
    let w = World {
        local: &local,
        ..world(&idx, &on)
    };
    let r = rows(&[], &w);
    let broken = r
        .iter()
        .find(|r| r.local.as_ref().is_some_and(|l| l.unreadable))
        .expect("listed");
    assert!(!broken.on);
    assert_eq!(broken.need, None);
    assert!(!cells(broken, None).switch);
    let good = r
        .iter()
        .find(|r| r.local.as_ref().is_some_and(|l| !l.unreadable))
        .expect("listed");
    assert!(good.on);
    assert_eq!(good.need, None);
    assert!(cells(good, None).switch);
}

#[test]
fn a_card_that_cannot_install_says_why_in_the_words_of_the_table() {
    let mut future = project(
        "future",
        "mod",
        &[("1.0.0", GAME, A)],
        serde_json::json!({}),
    );
    future["versions"][0]["capabilities"] = serde_json::json!(["from-a-newer-sunrise"]);
    let idx = index(
        vec![
            project("fine", "mod", &[("1.0.0", GAME, A)], serde_json::json!({})),
            project("old", "mod", &[("1.0.0", "26.2", B)], serde_json::json!({})),
            future,
        ],
        vec![],
    );
    let all = cards(&idx, GAME);
    let card = |id: &str| all.iter().find(|c| c.id == id).unwrap();
    assert_eq!(card("fine").fit.reason(), None);
    assert_eq!(
        card("old").fit.reason().as_deref(),
        Some("Not available for your version (made for 26.2)")
    );
    assert_eq!(
        Fit::OtherBuild("nightly-2026-11-17-abcdef0".into())
            .reason()
            .as_deref(),
        Some("Not available for your version (made for the Nov 17 Nightly)"),
        "a Nightly is named by its day, as the build sheets name it"
    );
    assert_eq!(card("future").fit, Fit::NeedsNewerSunrise);
    assert_eq!(
        card("future").fit.reason().as_deref(),
        Some("Needs a newer Sunrise")
    );
    // No registry key for the author: the card carries the Unsigned chip.
    assert!(card("fine").unsigned);
}

#[test]
fn a_collection_cannot_be_installed_while_a_mod_it_needs_does_not_fit_but_an_optional_one_may_be_missing(
) {
    // cow is ticked by default (the collection needs it) and is made for another build.
    let mut idx = zoo();
    idx_version(&mut idx, "cow").game = "25.1".into();
    let v = farm(&idx, &["pig"]);
    assert_eq!(
        v.total, 3,
        "\"1 of 3 selected\" counts every mod, available or not"
    );
    assert_eq!(
        v.members, 1,
        "what can be picked on this build: owl needs the missing cow too"
    );
    assert!(v.lines.iter().find(|l| l.id == "cow").unwrap().needed);
    assert!(!v.can_install());
    assert_eq!(
        v.plan_line(),
        "COW is needed and isn't available for your version"
    );
    // A mod the collection doesn't tick by default may be the missing one: the rest installs.
    let mut idx = zoo();
    idx_version(&mut idx, "owl").game = "25.1".into();
    let v = farm(&idx, &["cow", "pig"]);
    assert!(!v.lines.iter().find(|l| l.id == "owl").unwrap().needed);
    assert!(v.needed_missing.is_empty());
    assert!(v.can_install());
    assert_eq!(v.to_install(), ["cow", "pig"]);
    // Two needed and missing are named together.
    let mut idx = zoo();
    idx_version(&mut idx, "cow").game = "25.1".into();
    idx_version(&mut idx, "pig").game = "25.1".into();
    let v = farm(&idx, &[]);
    assert_eq!(
        v.plan_line(),
        "COW and PIG are needed and aren't available for your version"
    );
}

#[test]
fn a_mod_is_unknown_not_unfit_until_there_is_a_game_to_judge_by() {
    let idx = zoo();
    let all = cards(&idx, "");
    assert!(all.iter().all(|c| c.fit == Fit::Unknown));
    // "Fits my game" is main's rule: only what fits shows under it, unknown included as not
    // fitting; "Checking…" can only be seen where the switch is off.
    let on = Filters {
        fits_only: true,
        ..Filters::default()
    };
    assert!(discover(&all, &on).is_empty());
    let off = Filters {
        fits_only: false,
        ..Filters::default()
    };
    assert_eq!(discover(&all, &off).len(), 2);
    assert!(cards(&idx, GAME).iter().all(|c| c.fit == Fit::Fits));
}
