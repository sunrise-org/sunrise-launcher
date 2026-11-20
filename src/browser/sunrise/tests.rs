//! The registry source against the sample index of `sunrise_launcher_smod::index`
//! (`crates/smod/tests/index/`), its files served from disk.
use super::*;
const GAME: &str = crate::backend::PINNED_GAME;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/smod/tests/index")
}

/// The sample index with its files on disk, as a local test registry lists them.
pub(super) fn sample() -> Index {
    let text = fs::read_to_string(fixtures().join("index-v1.json")).unwrap();
    let mut index = Index::parse(&text).unwrap();
    for p in &mut index.projects {
        for v in &mut p.versions {
            let path = fixtures().join(&v.file.name);
            v.file.url = format!("file://{}", path.canonicalize().unwrap_or(path).display());
        }
    }
    index
}

pub(super) fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sunrise-launcher-registry-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn hud(index: &Index) -> Project {
    project_of(index, index.project("coords_hud").unwrap(), GAME).unwrap()
}

/// Installs the sample's checked content mod, as a modpack does.
fn put(index: &Index, local: bool, home: &Path) -> Result<Vec<installed::Entry>, String> {
    let p = index.project("coords_hud").unwrap();
    install_version(index, local, p, &p.versions[0], home, &|_, _| {}).map(|e| vec![e])
}

#[test]
fn a_required_mod_is_installed_once_and_refusals_leave_the_profile_intact() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = sample();
    let base = index.project("coords_hud").unwrap().clone();
    for (id, title, extra) in [
        ("fireflies", "Fireflies", "[dependencies]\nsunrise_creatures = '^1'\noptional_mod = { version = '*', optional = true }"),
        ("sunrise_creatures", "Sunrise Creatures", ""),
    ] {
        let manifest = format!("id='{id}'\nname='{title}'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n{extra}");
        let bytes = sunrise_launcher_smod::Package::write_files(&manifest, &[]).unwrap();
        let name = format!("{id}-1.0.0.smod");
        let file = dir.path().join(&name);
        fs::write(&file, &bytes).unwrap();
        let facts = index::inspect(&index::Kind::Mod, &file).unwrap();
        let mut version = base.versions[0].clone();
        version.version = "1.0.0".into();
        version.api = facts.api;
        version.side = facts.side;
        version.requires.clear();
        version.file = index::File {
            name,
            url: format!("file://{}", file.display()),
            size: bytes.len() as u64,
            sha256: index::sha256(&bytes),
            signature: None,
        };
        index.projects.push(index::Project {
            id: id.into(), name: title.into(), key: None,
            versions: vec![version], ..base.clone()
        });
    }
    let project = project_of(&index, index.project("fireflies").unwrap(), GAME).unwrap();
    let home = dir.path().join("profile");
    let run = |index: &Index| {
        install_from(
            index,
            true,
            &project,
            &home,
            GAME,
            false,
            true,
            &[],
            &|_, _| {},
        )
    };
    let mut bad = index.clone();
    bad.projects.last_mut().unwrap().versions[0].file.sha256 = "0".repeat(64);
    assert!(run(&bad)
        .unwrap_err()
        .contains("Fireflies requires sunrise_creatures"));
    assert!(!home.join(mod_list::MODS_DIR).exists());
    assert!(!home.join(installed::MANIFEST).exists());
    assert!(!home.join("sunrise-mods.txt").exists());

    // The index's own dependency claim is planned before anything downloads: no version of
    // sunrise_creatures satisfies ^2, so the install is refused with the planner's reason.
    let mut claimed = index.clone();
    let fireflies = claimed
        .projects
        .iter_mut()
        .find(|p| p.id == "fireflies")
        .unwrap();
    fireflies.versions[0].dependencies = Some(vec![index::Dependency {
        id: "sunrise_creatures".into(),
        version: "^2".into(),
        optional: false,
        side: None,
    }]);
    let refused = run(&claimed).unwrap_err();
    assert!(
        refused.contains("Fireflies can't be installed"),
        "{refused}"
    );
    assert!(!home.join(installed::MANIFEST).exists());

    // A directory at the manifest path is refused before any package is published.
    fs::create_dir(home.join(installed::MANIFEST)).unwrap();
    assert!(run(&index).is_err());
    assert!(!home.join(mod_list::MODS_DIR).exists());
    assert!(home.join(installed::MANIFEST).is_dir());
    assert!(!home.join("sunrise-mods.txt").exists());
    fs::remove_dir(home.join(installed::MANIFEST)).unwrap();

    let done = run(&index).unwrap();
    assert_eq!(
        done.iter().map(|e| e.project.as_str()).collect::<Vec<_>>(),
        ["fireflies", "sunrise_creatures"]
    );
    assert!(!done[0].dependency);
    assert!(done[1].dependency);
    for entry in &done {
        assert!(home.join(&entry.file).is_file());
        assert!(mod_list::is_enabled(&home, &entry.project));
    }
    // Updating a mod that is off installs what it newly needs off too.
    let fresh = dir.path().join("profile-off");
    let off = install_from(
        &index,
        true,
        &project,
        &fresh,
        GAME,
        false,
        false,
        &[],
        &|_, _| {},
    )
    .unwrap();
    assert_eq!(off.len(), 2);
    assert!(off.iter().all(|e| fresh.join(&e.file).is_file()));
    assert!(off
        .iter()
        .all(|e| !mod_list::is_enabled(&fresh, &e.project)));

    let before = fs::read(home.join(installed::MANIFEST)).unwrap();
    fs::remove_file(dir.path().join("sunrise_creatures-1.0.0.smod")).unwrap();
    assert_eq!(run(&index).unwrap(), done[..1]);
    assert_eq!(fs::read(home.join(installed::MANIFEST)).unwrap(), before);
    assert_eq!(
        fs::read_dir(home.join(mod_list::MODS_DIR)).unwrap().count(),
        2
    );

    // A mod the player turned off is updated like any other and stays off, even though what it
    // requires is off too; a manual install of it still asks for the dependency to be on.
    mod_list::set_all_enabled(&home, &["fireflies", "sunrise_creatures"], false).unwrap();
    let refused = install_from(
        &index,
        true,
        &project,
        &home,
        GAME,
        false,
        true,
        &[],
        &|_, _| {},
    )
    .unwrap_err();
    assert!(refused.contains("installed but off"), "{refused}");
    let kept = install_from(
        &index,
        true,
        &project,
        &home,
        GAME,
        false,
        false,
        &[],
        &|_, _| {},
    )
    .unwrap();
    assert_eq!(kept[0].project, "fireflies");
    assert!(!mod_list::is_enabled(&home, "fireflies"));
    assert!(!mod_list::is_enabled(&home, "sunrise_creatures"));
}

#[test]
fn native_registry_entries_are_mods_that_say_they_run_native_code() {
    assert_eq!(kind_of(&index::Kind::NativeMod), Some(Kind::Mod));
    let mut index = sample();
    index.projects[0].kind = index::Kind::NativeMod;
    index.projects[0].versions[0].file.name = "coords_hud-0.1.0.smod".into();
    let found = search(&index, &Query::new("", Kind::Mod));
    assert!(found.iter().any(|p| p.id == index.projects[0].id));
    let entry = |file: &str| {
        serde_json::from_value::<installed::Entry>(serde_json::json!({
            "source": "sunrise", "project": "p", "slug": "p", "title": "P", "kind": "mod",
            "version": "1", "number": "1", "file": file, "checksum": "", "page": ""
        }))
        .unwrap()
    };
    assert_eq!(
        mod_id(&entry("sunrise-native-mods/veinminer.smod")),
        Some("veinminer")
    );
    assert_eq!(mod_id(&entry("sunrise-mods/zoom.smod")), Some("zoom"));
}

#[test]
fn only_sunrises_own_projects_go_without_the_not_endorsed_line() {
    let mut p = sample().projects[0].clone();
    p.authors = vec!["Sunrise".into()];
    p.source = "https://github.com/horizon-rust/horizon-mod-zoom".into();
    assert!(official(&p));
    p.source = "https://github.com/someone/sunrise-mod-zoom".into();
    assert!(!official(&p), "the name alone doesn't make it official");
    p.authors = vec!["levzzz5154".into()];
    assert!(!official(&p));
}

#[test]
fn only_sunrises_own_projects_say_they_come_from_sunrise() {
    let mut index = sample();
    let id = index.projects[0].id.clone();
    index.projects[0].authors = vec!["levzzz5154".into()];
    index.projects[0].source = "https://github.com/levzzz5154/sunrise-mod-zoom".into();
    assert!(!official_in(Some(&index), &id), "a contributor's project");
    index.projects[0].authors = vec!["Sunrise".into()];
    index.projects[0].source = "https://github.com/horizon-rust/horizon-mod-zoom".into();
    assert!(official_in(Some(&index), &id));
    assert!(!official_in(Some(&index), "not_listed"));
    assert!(!official_in(None, &id), "no index read yet");
}

#[test]
fn searches_by_kind_and_words() {
    let index = sample();
    let found = |text: &str, kind| -> Vec<String> {
        search(&index, &Query::new(text, kind))
            .into_iter()
            .map(|p| p.id)
            .collect()
    };
    assert_eq!(found("", Kind::Mod), ["coords_hud"]);
    assert_eq!(found("coordinates HUD", Kind::Mod), ["coords_hud"]);
    assert!(found("zoom", Kind::Mod).is_empty());
    // Modpacks under Modpacks; a kind this launcher can't install yet (a world) never.
    assert_eq!(found("", Kind::Modpack), ["cozy"]);
    assert!(found("skyblock", Kind::ResourcePack).is_empty());
    assert_eq!(found("", Kind::Shader), ["glow"]);
    // The index's download counts, most downloaded first (of native mods: the browser lists them).
    let mut counted = index.clone();
    counted.projects[0].kind = index::Kind::NativeMod;
    counted.projects[0].versions[0].file.name = "coords_hud-0.1.0.smod".into();
    counted.projects.push(index::Project {
        id: "zoom".into(),
        name: "Zoom".into(),
        downloads: 12_345,
        ..counted.projects[0].clone()
    });
    counted.projects[0].downloads = 80;
    let first = search(&counted, &Query::new("", Kind::Mod));
    let order: Vec<(&str, u64)> = first.iter().map(|p| (p.id.as_str(), p.downloads)).collect();
    assert_eq!(order, [("zoom", 12_345), ("coords_hud", 80)]);
    // Newest orders by the day a project's first version came out, Updated by its latest.
    let mut dated = counted.clone();
    let base = dated.projects[0].versions[0].clone();
    let version_on = |day: &str| index::Version {
        native_binaries: Vec::new(),
        published: day.into(),
        ..base.clone()
    };
    dated.projects[0].versions = vec![version_on("2026-09-01"), version_on("2026-01-01")];
    let zoom = dated.projects.iter().position(|p| p.id == "zoom").unwrap();
    dated.projects[zoom].versions = vec![version_on("2026-05-01")];
    let sorted = |sort| -> Vec<String> {
        let mut q = Query::new("", Kind::Mod);
        q.filters.sort = Some(sort);
        search(&dated, &q).into_iter().map(|p| p.id).collect()
    };
    assert_eq!(
        sorted(super::super::details::Sort::Newest),
        ["zoom", "coords_hud"]
    );
    assert_eq!(
        sorted(super::super::details::Sort::Updated),
        ["coords_hud", "zoom"]
    );
    assert_eq!(
        sorted(super::super::details::Sort::Downloads),
        ["zoom", "coords_hud"]
    );
    let p = hud(&index);
    assert_eq!(p.support(GAME), super::super::Support::Works);
    assert_eq!(p.page, "https://playsunrise.net/mods/coords_hud");
}

#[test]
fn update_rejects_a_changed_kind_without_removing_the_installed_file() {
    let mut index = sample();
    let home = scratch("changed-kind");
    put(&index, true, &home).unwrap();
    let project = index
        .projects
        .iter_mut()
        .find(|p| p.id == "coords_hud")
        .unwrap();
    project.kind = index::Kind::NativeMod;
    let path = home.join("sunrise-mods/coords_hud.smod");
    let before = fs::read(&path).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let p = index.project("coords_hud").unwrap();
    let error = install_version(&index, true, p, &p.versions[0], &home, &|_, _| {}).unwrap_err();
    assert!(error.contains("different installed kind"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn installed_kind_comes_from_the_archive_not_its_folder() {
    for native in [false, true] {
        let mut index = sample();
        let home = scratch(if native {
            "native-in-content"
        } else {
            "content-in-native"
        });
        put(&index, true, &home).unwrap();
        let mut list = installed::Installed::load(&home);
        let old = home.join(&list.entries[0].file);
        let folder = if native {
            "sunrise-mods"
        } else {
            "sunrise-native-mods"
        };
        fs::create_dir_all(home.join(folder)).unwrap();
        let file = format!("{folder}/misplaced.smod");
        let path = home.join(&file);
        if native {
            let manifest = format!(
                "id='coords_hud'\nname='HUD'\nversion='0.1.0'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{}'\nmixins='{}'\n",
                "a".repeat(40),
                "b".repeat(40)
            );
            let files: &[(&str, &[u8])] = &[
                ("Cargo.toml", b"[package]\nname='hud'\nversion='1'\n"),
                ("src/lib.rs", b"pub fn hooks() {}"),
            ];
            fs::write(
                &path,
                sunrise_launcher_smod::Package::write_files(&manifest, files).unwrap(),
            )
            .unwrap();
            index::native::Bundle::read(&path).unwrap();
            index
                .projects
                .iter_mut()
                .find(|p| p.id == "coords_hud")
                .unwrap()
                .kind = index::Kind::NativeMod;
        } else {
            fs::copy(&old, &path).unwrap();
        }
        list.entries[0].file = file;
        list.save(&home).unwrap();
        let p = index.project("coords_hud").unwrap();
        let result = install_version(&index, true, p, &p.versions[0], &home, &|_, _| {});
        assert!(result.is_ok(), "{result:?}");
        assert!(path.is_file());
        fs::remove_dir_all(home).unwrap();
    }
}

#[test]
fn corrupt_installed_archive_is_moved_aside_by_its_update() {
    let index = sample();
    let home = scratch("corrupt-installed");
    put(&index, true, &home).unwrap();
    let path = home.join("sunrise-mods/coords_hud.smod");
    fs::write(&path, b"not a zip").unwrap();
    let mut list = installed::Installed::load(&home);
    list.entries[0].version = "0.0.1".into();
    list.save(&home).unwrap();
    let p = index.project("coords_hud").unwrap();
    install_version(&index, true, p, &p.versions[0], &home, &|_, _| {}).unwrap();
    // The new file is a real package, and the broken bytes are kept, not deleted.
    assert!(index::inspect(&index::Kind::Mod, &path).is_ok());
    let kept: Vec<_> = fs::read_dir(home.join("mod-browser/unreadable"))
        .unwrap()
        .map(|e| fs::read(e.unwrap().path()).unwrap())
        .collect();
    assert_eq!(kept, [b"not a zip".to_vec()]);
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn a_mod_install_with_dependencies_keeps_the_unreadable_file_it_replaces() {
    let index = sample();
    let home = scratch("corrupt-with-deps");
    put(&index, true, &home).unwrap();
    let path = home.join("sunrise-mods/coords_hud.smod");
    fs::write(&path, b"not a zip").unwrap();
    let mut list = installed::Installed::load(&home);
    list.entries[0].version = "0.0.1".into();
    list.save(&home).unwrap();
    let p = index.project("coords_hud").unwrap();
    let install = Install {
        index: &index,
        local: true,
        home: &home,
        approvals: &[],
        progress: &|_, _| {},
    };
    dependencies::install(
        &install,
        &[(p, &p.versions[0])],
        false,
        true,
        &Default::default(),
    )
    .unwrap();
    assert!(index::inspect(&index::Kind::Mod, &path).is_ok());
    let kept: Vec<_> = fs::read_dir(home.join("mod-browser/unreadable"))
        .unwrap()
        .map(|e| fs::read(e.unwrap().path()).unwrap())
        .collect();
    assert_eq!(kept, [b"not a zip".to_vec()]);
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn an_installed_file_that_cannot_be_read_is_refused_not_moved_aside() {
    let index = sample();
    let home = scratch("unreadable-io");
    put(&index, true, &home).unwrap();
    // A directory where the file was: it exists, but reading it as a file fails.
    let path = home.join("sunrise-mods/coords_hud.smod");
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    let mut list = installed::Installed::load(&home);
    list.entries[0].version = "0.0.1".into();
    list.save(&home).unwrap();
    let p = index.project("coords_hud").unwrap();
    let error = install_version(&index, true, p, &p.versions[0], &home, &|_, _| {}).unwrap_err();
    // Unix opens the directory and says it isn't a regular file; Windows refuses to open it
    // (access denied). Either way the update names the file, refuses and leaves it.
    assert!(error.contains("coords_hud.smod"), "{error}");
    assert!(path.is_dir());
    // No copy of it is kept (the folder it would go to may exist, empty).
    assert!(fs::read_dir(home.join("mod-browser/unreadable"))
        .map_or(true, |mut dir| dir.next().is_none()));
    assert!(!home.join("mod-browser/install-txn.json").exists());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn an_installed_entry_pointing_out_of_the_game_folder_is_refused() {
    let index = sample();
    let home = scratch("escaping-entry");
    put(&index, true, &home).unwrap();
    let outside = home.parent().unwrap().join("escaped-by-entry.smod");
    fs::write(&outside, b"not a zip").unwrap();
    let name = format!("../{}", outside.file_name().unwrap().to_string_lossy());
    // The manifest loader drops such an entry; a list built in memory still must not reach it.
    let mut list = installed::Installed::load(&home);
    list.entries[0].version = "0.0.1".into();
    list.entries[0].file = name;
    let p = index.project("coords_hud").unwrap();
    let install = Install {
        index: &index,
        local: true,
        home: &home,
        approvals: &[],
        progress: &|_, _| {},
    };
    let result = install.prepare(p, &p.versions[0], &list, &home.join("staging"), None);
    assert!(result.is_err());
    assert_eq!(fs::read(&outside).unwrap(), b"not a zip");
    fs::remove_file(outside).unwrap();
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn a_failed_install_with_dependencies_leaves_no_copy_of_the_unreadable_file() {
    let index = sample();
    let home = scratch("failed-with-deps");
    put(&index, true, &home).unwrap();
    let path = home.join("sunrise-mods/coords_hud.smod");
    fs::write(&path, b"not a zip").unwrap();
    let mut list = installed::Installed::load(&home);
    list.entries[0].version = "0.0.1".into();
    list.save(&home).unwrap();
    // The list of enabled mods can't be read, so turning the mod on fails before anything lands.
    let enabled = home.join(sunrise_launcher_smod::mod_list::FILE);
    let _ = fs::remove_file(&enabled);
    fs::create_dir(&enabled).unwrap();
    let p = index.project("coords_hud").unwrap();
    let install = Install {
        index: &index,
        local: true,
        home: &home,
        approvals: &[],
        progress: &|_, _| {},
    };
    assert!(dependencies::install(
        &install,
        &[(p, &p.versions[0])],
        false,
        true,
        &Default::default()
    )
    .is_err());
    assert_eq!(fs::read(&path).unwrap(), b"not a zip");
    assert!(!home.join("mod-browser/unreadable").exists());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn update_replaces_an_entry_whose_file_is_gone() {
    let index = sample();
    let home = scratch("gone-installed");
    put(&index, true, &home).unwrap();
    let path = home.join("sunrise-mods/coords_hud.smod");
    fs::remove_file(&path).unwrap();
    let mut list = installed::Installed::load(&home);
    list.entries[0].version = "0.0.1".into();
    list.save(&home).unwrap();
    let p = index.project("coords_hud").unwrap();
    install_version(&index, true, p, &p.versions[0], &home, &|_, _| {}).unwrap();
    assert!(index::inspect(&index::Kind::Mod, &path).is_ok());
    assert_eq!(installed::Installed::load(&home).entries.len(), 1);
    assert!(!home.join("mod-browser/unreadable").exists());
    fs::remove_dir_all(home).unwrap();
}

/// The sample's `coords_hud` as a native mod, its file written under `home`.
pub(super) fn native_sample(home: &Path) -> Index {
    use std::io::Write;
    let archive = home.join("source.smod");
    let (a, b) = ("a".repeat(40), "b".repeat(40));
    let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
    for (name, text) in [
        ("Cargo.toml", "[package]\nname='coords_hud'\nversion='0.1.0'\n".to_string()),
        ("src/lib.rs", "fn hook() {}".to_string()),
        ("sunrise-mod.toml", format!("id='coords_hud'\nname='HUD'\nversion='0.1.0'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='{GAME}'\nsunrise='{a}'\nmixins='{b}'\n")),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    let bytes = fs::read(&archive).unwrap();
    let mut index = sample();
    let p = &mut index.projects[0];
    p.kind = index::Kind::NativeMod;
    p.key = None;
    let v = &mut p.versions[0];
    v.api = format!("native-1/{a}/{b}");
    v.version = "0.1.0".into();
    v.game = GAME.into();
    v.side = None;
    v.requires.clear();
    v.capabilities = Default::default();
    v.file.name = "coords_hud.smod".into();
    v.file.url = format!("file://{}", archive.display());
    v.file.size = bytes.len() as u64;
    v.file.sha256 = index::sha256(&bytes);
    v.file.signature = None;
    index
}

/// The native sample with "fireflies" added: a content mod that needs the native `coords_hud`.
fn fireflies_needing_a_native_mod(home: &Path) -> Index {
    let mut index = native_sample(home);
    // A content mod that needs the native one.
    let manifest = "id='fireflies'\nname='Fireflies'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n[dependencies]\ncoords_hud = '^0.1'";
    let bytes = sunrise_launcher_smod::Package::write_files(manifest, &[]).unwrap();
    let file = home.join("fireflies-1.0.0.smod");
    fs::write(&file, &bytes).unwrap();
    let facts = index::inspect(&index::Kind::Mod, &file).unwrap();
    let mut project = sample().project("coords_hud").unwrap().clone();
    project.id = "fireflies".into();
    project.key = None;
    project.name = "Fireflies".into();
    let v = &mut project.versions[0];
    v.version = "1.0.0".into();
    v.api = facts.api;
    v.side = facts.side;
    v.requires.clear();
    v.dependencies = Some(vec![index::Dependency {
        id: "coords_hud".into(),
        version: "^0.1".into(),
        optional: false,
        side: None,
    }]);
    v.file = index::File {
        name: "fireflies-1.0.0.smod".into(),
        url: format!("file://{}", file.display()),
        size: bytes.len() as u64,
        sha256: index::sha256(&bytes),
        signature: None,
    };
    index.projects.push(project);
    index
}

#[test]
fn an_install_brings_native_code_in_when_what_it_needs_is_native() {
    let home = scratch("brings-native");
    let index = fireflies_needing_a_native_mod(&home);
    let native_hash = index.project("coords_hud").unwrap().versions[0]
        .file
        .sha256
        .clone();
    let ask = |ids: &[&str], held: &[index::resolve::InstalledMod]| {
        let ids: Vec<String> = ids.iter().map(|id| id.to_string()).collect();
        brings_native(&index, &ids, held, GAME)
    };
    assert!(ask(&["coords_hud"], &[]), "a native mod itself");
    assert!(
        ask(&["fireflies"], &[]),
        "a content mod with a native dependency"
    );
    let held = [index::resolve::InstalledMod {
        id: "coords_hud".into(),
        version: "0.1.0".into(),
        sha256: Some(native_hash),
    }];
    assert!(
        !ask(&["fireflies"], &held),
        "the native mod is already installed"
    );
    assert!(!ask(&["nothing_like_it"], &[]), "no plan, no claim");
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn switch_cannot_seal_a_filtered_takedown_or_a_future_schema() {
    assert!(parse_authoritative(r#"{"schema":1,"projects":[],"revoked":[]}"#).is_ok());
    assert!(parse_authoritative(r#"{"schema":1,"projects":[]}"#).is_err());
    assert!(parse_authoritative(r#"{"schema":2,"projects":[],"revoked":[]}"#).is_err());
    assert!(parse_authoritative(r#"{"schema":1,"projects":[],"revoked":[{"id":"broken","sha256":"not-a-hash","reason":"removed"}]}"#).is_err());
}

#[test]
fn an_authoritative_index_must_explicitly_list_projects_and_revocations() {
    let complete = r#"{"schema":1,"projects":[],"revoked":[]}"#;
    assert!(parse_live(complete).is_ok());
    for partial in [
        r#"{"schema":1,"projects":[]}"#,
        r#"{"schema":1,"revoked":[]}"#,
        r#"{"schema":1,"projects":[],"revoked":null}"#,
    ] {
        let error = parse_live(partial).unwrap_err();
        assert!(error.contains("complete index"), "{error}");
        let home = tempfile::tempdir().unwrap();
        let cache = home.path().join("index.json");
        fs::write(&cache, complete).unwrap();
        let fallback = load(Err(error), &cache, false).unwrap();
        assert!(
            fallback.fetched().is_none(),
            "a display fallback is not authoritative"
        );
    }
}

#[test]
fn an_install_with_native_code_keeps_its_record_until_the_build_ends() {
    let home = scratch("txn-native");
    let index = native_sample(&home);
    let hud = project_of(&index, index.project("coords_hud").unwrap(), GAME).unwrap();
    let install = || {
        install_from(&index, true, &hud, &home, GAME, true, true, &[], &|_, _| {}).unwrap();
    };
    let record = home.join("mod-browser/install-txn.json");
    let lease = txn::Lease::acquire(&home).unwrap();
    install();
    assert!(record.is_file(), "native code waits for its build");
    assert!(home.join("sunrise-mods").is_dir() || home.join("sunrise-native-mods").is_dir());
    // The build stopped for a reason outside the mods: the install is undone whole.
    let waiting = lease.mark_build("tag").unwrap();
    assert_eq!(waiting.len(), 1);
    lease.undo(&waiting).unwrap();
    assert!(installed::Installed::load(&home).entries.is_empty());
    assert!(!record.exists());
    // The build worked: the install is done, and a restart changes nothing.
    install();
    lease.done(&lease.mark_build("tag").unwrap()).unwrap();
    assert!(!record.exists());
    drop(lease);
    assert!(txn::recover(&home));
    assert!(installed::Installed::load(&home)
        .find(SourceId::Sunrise, "coords_hud")
        .is_some());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn a_native_dependency_installs_under_the_choice_and_is_refused_before_it_with_nothing_fetched() {
    let home = scratch("native-dependency");
    let index = fireflies_needing_a_native_mod(&home);
    let p = index.project("fireflies").unwrap();
    let install = Install {
        index: &index,
        local: true,
        home: &home,
        approvals: &[],
        progress: &|_, _| {},
    };
    let refused = dependencies::install(
        &install,
        &[(p, &p.versions[0])],
        false,
        true,
        &Default::default(),
    )
    .unwrap_err();
    assert!(refused.contains("runs native code"), "{refused}");
    // Refused from the plan: nothing was downloaded or written.
    assert!(!home.join(installed::MANIFEST).exists());
    assert!(!home.join("sunrise-native-mods").exists());
    assert!(!home.join(mod_list::MODS_DIR).exists());
    let done = dependencies::install(
        &install,
        &[(p, &p.versions[0])],
        true,
        true,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(
        done.iter().map(|e| e.project.as_str()).collect::<Vec<_>>(),
        ["fireflies", "coords_hud"]
    );
    assert!(done[1].dependency);
    assert!(home.join("sunrise-native-mods/coords_hud.smod").is_file());
    assert!(mod_list::is_enabled(&home, "coords_hud"));
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn native_code_is_refused_without_the_choice_whoever_brings_it() {
    let home = scratch("native-consent-all");
    let mut index = fireflies_needing_a_native_mod(&home);
    let install = |index: &Index| {
        let p = index.project("fireflies").unwrap();
        let native = index.project("coords_hud").unwrap();
        let run = |project: &index::Project| {
            dependencies::install(
                &Install {
                    index,
                    local: true,
                    home: &home,
                    approvals: &[],
                    progress: &|_, _| {},
                },
                &[(project, &project.versions[0])],
                false,
                true,
                &Default::default(),
            )
        };
        (run(p), run(native))
    };
    // The requested mod is native itself.
    let (_, root) = install(&index);
    assert_eq!(
        root.unwrap_err(),
        installed::InstallError::NeedsNativeConsent {
            native: "Coordinates HUD".into()
        }
    );
    // An index of the older format that doesn't list the dependency: the file does.
    for project in &mut index.projects {
        for version in &mut project.versions {
            version.dependencies = None;
        }
    }
    let (content, _) = install(&index);
    // The launcher asks and tries again on this outcome, not on its words.
    assert_eq!(
        content.unwrap_err(),
        installed::InstallError::NeedsNativeConsent {
            native: "Coordinates HUD".into()
        }
    );
    assert!(!home.join("sunrise-native-mods").exists());
    assert!(!home.join(installed::MANIFEST).exists());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn an_install_the_plan_refuses_for_what_an_installed_mod_needs_names_that_mod() {
    let mut index = sample();
    let base = index.project("coords_hud").unwrap().clone();
    let make = |id: &str, number: &str, needs: Option<(&str, &str)>| {
        let mut version = base.versions[0].clone();
        version.version = number.into();
        version.file.sha256 = index::sha256(format!("{id}{number}").as_bytes());
        version.requires.clear();
        version.dependencies = needs.map(|(dep, range)| {
            vec![index::Dependency {
                id: dep.into(),
                version: range.into(),
                optional: false,
                side: None,
            }]
        });
        version
    };
    for (id, versions) in [
        (
            "creatures",
            vec![
                make("creatures", "2.0.0", None),
                make("creatures", "1.0.0", None),
            ],
        ),
        (
            "lamps",
            vec![make("lamps", "1.0.0", Some(("creatures", "^1")))],
        ),
        (
            "owls",
            vec![make("owls", "1.0.0", Some(("creatures", "^1")))],
        ),
        (
            "fireflies",
            vec![make("fireflies", "1.0.0", Some(("creatures", "^2")))],
        ),
    ] {
        index.projects.push(index::Project {
            id: id.into(),
            name: id.into(),
            key: None,
            versions,
            ..base.clone()
        });
    }
    let held = |id: &str, version: &str| index::resolve::InstalledMod {
        id: id.into(),
        version: version.into(),
        sha256: Some(index::sha256(format!("{id}{version}").as_bytes())),
    };
    let one = [held("creatures", "1.0.0"), held("lamps", "1.0.0")];
    let broken = would_break(&index, &["fireflies".to_string()], &one, GAME);
    assert_eq!(
        broken,
        [Broken {
            id: "lamps".into(),
            needs: "creatures ^1".into()
        }]
    );
    // Two installed mods pin it: removing either alone doesn't help, so both are named.
    let held = [
        held("creatures", "1.0.0"),
        held("lamps", "1.0.0"),
        held("owls", "1.0.0"),
    ];
    let broken = would_break(&index, &["fireflies".to_string()], &held, GAME);
    assert_eq!(
        broken,
        [
            Broken {
                id: "lamps".into(),
                needs: "creatures ^1".into()
            },
            Broken {
                id: "owls".into(),
                needs: "creatures ^1".into()
            }
        ]
    );
    // Installed alone, it breaks nothing.
    assert!(would_break(&index, &["lamps".to_string()], &held, GAME).is_empty());
}

#[test]
fn a_planning_refusal_is_typed_by_what_the_index_says() {
    let mut index = sample();
    let base = index.project("coords_hud").unwrap().clone();
    let make = |id: &str, number: &str, game: &str, needs: Option<(&str, &str)>| {
        let mut version = base.versions[0].clone();
        version.version = number.into();
        version.game = game.into();
        version.file.sha256 = index::sha256(format!("{id}{number}{game}").as_bytes());
        version.requires.clear();
        version.dependencies = needs.map(|(dep, range)| {
            vec![index::Dependency {
                id: dep.into(),
                version: range.into(),
                optional: false,
                side: None,
            }]
        });
        version
    };
    for (id, name, versions) in [
        // Flywheel 0.4 exists, but only for another game; 0.3 runs on this one.
        (
            "flywheel",
            "Flywheel",
            vec![
                make("flywheel", "0.4.0", "25.1", None),
                make("flywheel", "0.3.0", GAME, None),
            ],
        ),
        (
            "lumen",
            "Lumen",
            vec![make("lumen", "1.0.0", GAME, Some(("flywheel", "^0.4")))],
        ),
        (
            "lamps",
            "Lamps",
            vec![make("lamps", "1.0.0", GAME, Some(("flywheel", "^0.3")))],
        ),
        (
            "dyno",
            "Dyno",
            vec![make("dyno", "1.0.0", GAME, Some(("flywheel", "^0.3")))],
        ),
    ] {
        index.projects.push(index::Project {
            id: id.into(),
            name: name.into(),
            key: None,
            versions,
            ..base.clone()
        });
    }
    let held = |id: &str, version: &str, game: &str| index::resolve::InstalledMod {
        id: id.into(),
        version: version.into(),
        sha256: Some(index::sha256(format!("{id}{version}{game}").as_bytes())),
    };
    // Needs a version of Flywheel that isn't made for this game: said by type.
    let none: [index::resolve::InstalledMod; 0] = [];
    let refused = check_set(&index, &["lumen".to_string()], &none, GAME).unwrap_err();
    assert!(
        matches!(&refused, installed::InstallError::Unavailable { dependency, .. } if dependency == "Flywheel"),
        "{refused:?}"
    );
    // The same install with two installed mods pinning the dependency it needs.
    let mut newer = index.clone();
    for p in &mut newer.projects {
        if p.id == "flywheel" {
            p.versions[0].game = GAME.into();
        }
    }
    let held = [
        held("flywheel", "0.3.0", GAME),
        held("lamps", "1.0.0", GAME),
        held("dyno", "1.0.0", GAME),
    ];
    let refused = check_set(&newer, &["lumen".to_string()], &held, GAME).unwrap_err();
    assert!(
        matches!(&refused, installed::InstallError::Conflict { mods, .. } if mods == &["Lamps", "Dyno"]),
        "{refused:?}"
    );
}

#[test]
fn a_plain_to_native_flip_with_dependencies_replaces_an_unreadable_or_gone_file() {
    for gone in [false, true] {
        let home = scratch(if gone { "flip-gone" } else { "flip-unreadable" });
        let plain = sample();
        put(&plain, true, &home).unwrap();
        let old = home.join("sunrise-mods/coords_hud.smod");
        if gone {
            fs::remove_file(&old).unwrap();
        } else {
            fs::write(&old, b"not a zip").unwrap();
        }
        let mut list = installed::Installed::load(&home);
        list.entries[0].version = "0.0.1".into();
        list.save(&home).unwrap();
        let index = native_sample(&home);
        let p = &index.projects[0];
        let install = Install {
            index: &index,
            local: true,
            home: &home,
            approvals: &[],
            progress: &|_, _| {},
        };
        let done = dependencies::install(
            &install,
            &[(p, &p.versions[0])],
            true,
            true,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(done[0].file, "sunrise-native-mods/coords_hud.smod");
        assert!(home.join("sunrise-native-mods/coords_hud.smod").is_file());
        assert!(!old.exists());
        let list = installed::Installed::load(&home);
        assert_eq!(list.entries.len(), 1);
        assert_eq!(list.entries[0].file, "sunrise-native-mods/coords_hud.smod");
        let aside = home.join("mod-browser/unreadable");
        if gone {
            assert!(!aside.exists());
        } else {
            let kept: Vec<_> = fs::read_dir(aside)
                .unwrap()
                .map(|e| fs::read(e.unwrap().path()).unwrap())
                .collect();
            assert_eq!(kept, [b"not a zip".to_vec()]);
        }
        fs::remove_dir_all(home).unwrap();
    }
}

#[test]
fn a_mod_install_with_dependencies_replaces_an_entry_whose_file_is_gone() {
    let index = sample();
    let home = scratch("gone-with-deps");
    put(&index, true, &home).unwrap();
    let path = home.join("sunrise-mods/coords_hud.smod");
    fs::remove_file(&path).unwrap();
    let mut list = installed::Installed::load(&home);
    list.entries[0].version = "0.0.1".into();
    list.save(&home).unwrap();
    let p = index.project("coords_hud").unwrap();
    let install = Install {
        index: &index,
        local: true,
        home: &home,
        approvals: &[],
        progress: &|_, _| {},
    };
    dependencies::install(
        &install,
        &[(p, &p.versions[0])],
        false,
        true,
        &Default::default(),
    )
    .unwrap();
    assert!(index::inspect(&index::Kind::Mod, &path).is_ok());
    assert!(!home.join("mod-browser/unreadable").exists());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn removing_native_mod_does_not_remove_a_separate_content_file() {
    let index = sample();
    let home = scratch("native-remove");
    put(&index, true, &home).unwrap();
    let mut list = installed::Installed::load(&home);
    list.entries[0].file = "sunrise-native-mods/coords_hud.smod".into();
    list.save(&home).unwrap();
    fs::create_dir_all(home.join("sunrise-native-mods")).unwrap();
    fs::write(home.join("sunrise-native-mods/coords_hud.smod"), "").unwrap();
    installed::remove(&home, "coords_hud").unwrap();
    assert!(home.join("sunrise-mods/coords_hud.smod").is_file());
    assert!(!home.join("sunrise-native-mods/coords_hud.smod").exists());
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn searches_by_category_and_tag() {
    let mut index = sample();
    let glow = index.projects.iter().position(|p| p.id == "glow").unwrap();
    index.projects[glow].categories = vec!["fantasy".into(), "bloom".into()];
    index.projects[glow].tags = vec!["sunset".into()];
    let found = |text: &str, categories: &[&str]| -> Vec<String> {
        let mut q = Query::new(text, Kind::Shader);
        q.filters.categories = categories.iter().map(|c| c.to_string()).collect();
        search(&index, &q).into_iter().map(|p| p.id).collect()
    };
    assert_eq!(found("", &["bloom"]), ["glow"]);
    assert_eq!(found("", &["bloom", "fantasy"]), ["glow"]);
    assert!(
        found("", &["bloom", "pbr"]).is_empty(),
        "every category asked"
    );
    assert_eq!(found("sunset", &[]), ["glow"], "tags are searched");
    assert!(found("dawn", &[]).is_empty());
    let tagged = |tags: &[&str]| -> Vec<String> {
        let mut q = Query::new("", Kind::Shader);
        q.filters.tags = tags.iter().map(|t| t.to_string()).collect();
        search(&index, &q).into_iter().map(|p| p.id).collect()
    };
    assert_eq!(tagged(&["sunset"]), ["glow"]);
    assert!(tagged(&["sunset", "dawn"]).is_empty(), "every tag asked");
    // The filter offers the kind's own list: a resolution for resource packs, not for shaders.
    let shader = Registry.tags(Kind::Shader).unwrap().categories;
    assert!(shader.iter().any(|c| c.name == "path-tracing"));
    let packs = Registry.tags(Kind::ResourcePack).unwrap().categories;
    assert!(packs.iter().any(|c| c.name == "16x"));
    assert!(!shader.iter().any(|c| c.name == "16x"));
}

#[test]
fn the_tag_filter_offers_the_kinds_tags_most_used_first() {
    let mut index = sample();
    let at = |index: &Index, id: &str| index.projects.iter().position(|p| p.id == id).unwrap();
    let glow = at(&index, "glow");
    index.projects[glow].tags = vec![
        "sunset".into(),
        "cozy".into(),
        "sunset".into(),
        "Not Plain".into(),
    ];
    assert_eq!(
        tags_of(&index, Kind::Shader),
        ["cozy", "sunset"],
        "ties by name"
    );
    let mut dusk = index.projects[glow].clone();
    dusk.id = "dusk".into();
    dusk.tags = vec!["sunset".into()];
    index.projects.push(dusk);
    let hud = at(&index, "coords_hud");
    index.projects[hud].tags = vec!["pvp".into()];
    assert_eq!(tags_of(&index, Kind::Shader), ["sunset", "cozy"]);
    assert_eq!(tags_of(&index, Kind::Mod), ["pvp"], "each kind its own");
    assert!(tags_of(&index, Kind::DataPack).is_empty());
}

/// A data pack zip with one function, and the registry project that lists it.
fn data_pack(index: &mut Index, dir: &Path) -> Project {
    use std::io::Write;
    let path = dir.join("lanterns-1.0.0.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    for (name, text) in [
        (
            "pack.mcmeta",
            r#"{"pack": {"description": "Lanterns", "pack_format": 94}}"#,
        ),
        ("data/lanterns/function/light.mcfunction", "say lit"),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    let base = index.project("glow").unwrap().clone();
    let mut version = base.versions[0].clone();
    version.version = "1.0.0".into();
    version.api = "94".into();
    version.side = None;
    version.capabilities = Default::default();
    version.requires.clear();
    version.file = index::File {
        name: "lanterns-1.0.0.zip".into(),
        url: format!("file://{}", path.display()),
        size: fs::metadata(&path).unwrap().len(),
        sha256: index::sha256_file(&path).unwrap(),
        signature: None,
    };
    index.projects.push(index::Project {
        id: "lanterns".into(),
        kind: index::Kind::DataPack,
        name: "Lanterns".into(),
        key: None,
        versions: vec![version],
        ..base
    });
    project_of(index, index.project("lanterns").unwrap(), GAME).unwrap()
}

#[test]
fn a_registry_data_pack_is_checked_then_goes_into_a_world() {
    let dir = scratch("datapack");
    let mut index = sample();
    let p = data_pack(&mut index, &dir);
    assert_eq!(p.kind, Kind::DataPack);
    assert_eq!(p.support(GAME), super::super::Support::Works);
    let found: Vec<String> = search(&index, &Query::new("", Kind::DataPack))
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(found, ["lanterns"]);
    // Not into the game folder: into a world.
    assert!(install_from(
        &index,
        true,
        &p,
        &dir.join("home"),
        GAME,
        false,
        true,
        &[],
        &|_, _| {}
    )
    .is_err());
    let (zip, number) =
        fetch_datapack_from(&index, true, &p, &dir.join("dl"), GAME, &|_| {}).unwrap();
    assert_eq!(number, "1.0.0");
    let world = dir.join("world");
    let pack = crate::datapacks::add(&world, &zip).unwrap();
    assert!(world.join("datapacks").join(&pack.file).is_file());

    // A file other than the one the index lists never gets there.
    let mut changed = index.clone();
    let at = changed.projects.len() - 1;
    changed.projects[at].versions[0].file.sha256 = "0".repeat(64);
    let e = fetch_datapack_from(&changed, true, &p, &dir.join("dl2"), GAME, &|_| {}).unwrap_err();
    assert!(e.contains("checksum") || e.contains("SHA-256"), "{e}");
    // Nor one taken down.
    index.revoked.push(index::Revoked {
        id: "lanterns".into(),
        sha256: None,
        reason: "Test.".into(),
        date: String::new(),
    });
    let e = fetch_datapack_from(&index, true, &p, &dir.join("dl3"), GAME, &|_| {}).unwrap_err();
    assert_eq!(e, "Lanterns was taken down: Test.");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn screenshots_are_the_gallery_from_the_registry_only() {
    let mut p = sample().projects[0].clone();
    assert!(gallery_of(&p).is_empty());
    let pages = "https://sunrise-org.github.io/sunrise-mods/screenshots/coords_hud";
    p.screenshots = vec![
        format!("{pages}/b.png"),
        "https://example.com/tracker.png".into(),
        format!("{pages}/a.jpg"),
    ];
    let urls: Vec<String> = gallery_of(&p).into_iter().map(|i| i.url).collect();
    assert_eq!(urls, [format!("{pages}/b.png"), format!("{pages}/a.jpg")]);
}

#[test]
fn a_registry_icon_with_a_version_query_and_five_screenshots_are_all_fetchable() {
    let site = "https://sunrise-org.github.io/sunrise-mods";
    let mut p = sample().projects[0].clone();
    p.icon = Some(format!("{site}/icons/jei.png?v=845e5188457e"));
    p.screenshots = (1..=5)
        .map(|n| format!("{site}/screenshots/jei/{n}-shot.jpg"))
        .collect();
    let hit = project_of(&sample(), &p, GAME).unwrap();
    let icon = hit.icon.unwrap();
    assert!(icon.starts_with(sunrise_launcher_addresses::MOD_IMAGES));
    assert_eq!(super::super::images::refuse(&icon), None);
    let gallery = gallery_of(&p);
    assert_eq!(gallery.len(), 5);
    assert_eq!(gallery[4].url, format!("{site}/screenshots/jei/5-shot.jpg"));
}

#[test]
fn installs_combined_native_content_as_one_smod() {
    use std::io::Write;
    let home = scratch("native-content-install");
    let archive = home.join("source.smod");
    let a = "a".repeat(40);
    let b = "b".repeat(40);
    let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
    for (name, text) in [
        ("Cargo.toml", "[package]\nname='coords_hud'\nversion='0.1.0'\n".into()),
        ("src/lib.rs", "fn hook() {}".into()),
        ("sunrise-mod.toml", format!("id='coords_hud'\nname='Sound proof'\nversion='0.1.0'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='{GAME}'\nsunrise='{a}'\nmixins='{b}'\n")),
        ("assets/coords_hud/sounds.json", "{}".into()),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    let bytes = fs::read(&archive).unwrap();
    let mut index = sample();
    let p = &mut index.projects[0];
    p.kind = index::Kind::NativeMod;
    p.key = None;
    let v = &mut p.versions[0];
    v.api = format!("native-1/{a}/{b}");
    v.version = "0.1.0".into();
    v.game = GAME.into();
    v.side = None;
    v.requires.clear();
    v.capabilities = Default::default();
    v.file.name = "coords_hud.smod".into();
    v.file.url = format!("file://{}", archive.display());
    v.file.size = bytes.len() as u64;
    v.file.sha256 = index::sha256(&bytes);
    v.file.signature = None;
    let p = &index.projects[0];
    let entry = install_version(&index, true, p, &p.versions[0], &home, &|_, _| {}).unwrap();
    assert_eq!(entry.file, "sunrise-native-mods/coords_hud.smod");
    assert_eq!(mod_id(&entry), Some("coords_hud"));
    let bundle = index::native::Bundle::read(&home.join(&entry.file)).unwrap();
    assert_eq!(bundle.files["assets/coords_hud/sounds.json"], b"{}");
    assert_eq!(crate::native_status::enabled(&home).len(), 1);
    // The installed file is the downloaded one, byte for byte, so the native-code question names
    // it as the registry does: also once the registry lists a newer version before it.
    let named = |index: &Index| crate::native_status::build_inputs(&home, Some(index));
    let as_registry = [crate::native_status::BuildInput {
        id: "coords_hud".into(),
        file: "coords_hud.smod".into(),
        registry: crate::native_status::Registry::Listed {
            name: index.projects[0].name.clone(),
            icon: index.projects[0].icon.clone(),
        },
    }];
    assert_eq!(named(&index), as_registry);
    let mut later = index.clone();
    let mut newer = later.projects[0].versions[0].clone();
    newer.version = "0.2.0".into();
    newer.file.sha256 = "0".repeat(64);
    later.projects[0].versions.insert(0, newer);
    assert_eq!(named(&later), as_registry);
    // The copy of the index kept in the game folder is not the registry's word. A copy that
    // lists a file put there by hand would name it as the registry's mod; it is never asked.
    let own = home.join("sunrise-native-mods/coords_hud.smod");
    let mut bytes = fs::read(&own).unwrap();
    bytes.extend_from_slice(b"\0");
    fs::write(&own, &bytes).unwrap();
    // From here the test models a remote registry. Its file URL is validated but never fetched.
    index.projects[0].versions[0].file.url = "https://example.com/coords_hud.smod".into();
    let mut forged = index.clone();
    forged.projects[0].versions[0].file.sha256 = index::sha256(&bytes);
    forged.projects[0].versions[0].file.size = bytes.len() as u64;
    let cache = home.join(CACHE);
    fs::create_dir_all(cache.parent().unwrap()).unwrap();
    fs::write(&cache, serde_json::to_string(&forged).unwrap()).unwrap();
    let kept = load(Err("offline".into()), &cache, false).unwrap();
    assert_eq!(
        named(&kept.index),
        as_registry,
        "the forgery, were it asked"
    );
    assert!(kept.fetched().is_none());
    let unchecked = crate::native_status::build_inputs(&home, kept.fetched().as_deref());
    assert_eq!(
        unchecked[0].registry,
        crate::native_status::Registry::Unchecked
    );
    // The registry's own answer is asked, and does not list that file.
    let text = serde_json::to_string(&index).unwrap();
    let answered = load(Ok((index, text)), &cache, false).unwrap();
    let checked = crate::native_status::build_inputs(&home, answered.fetched().as_deref());
    assert_eq!(
        checked[0].registry,
        crate::native_status::Registry::NotListed
    );
    installed::remove(&home, "coords_hud").unwrap();
    assert!(!home.join(entry.file).exists());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn installs_checked_turns_on_and_removes() {
    let index = sample();
    let home = scratch("install");
    let p = hud(&index);
    // An instance on another build's Minecraft version gets that version's file, or none.
    let other = install_from(
        &index,
        true,
        &p,
        &home,
        "26.9",
        false,
        true,
        &[],
        &|_, _| {},
    )
    .unwrap_err();
    assert!(other.contains("no version for 26.9"), "{other}");
    let done = install_from(&index, true, &p, &home, GAME, false, true, &[], &|_, _| {}).unwrap();
    assert_eq!(done[0].file, "sunrise-mods/coords_hud.smod");
    assert_eq!(done[0].verdict.as_deref(), Some("Verified"));
    assert_eq!(
        done[0].checksum,
        "SHA-256:cb6e658445269b2a4d2eb104e83e4b993586bc2e56814fd56454244139f08364"
    );
    assert!(home.join("sunrise-mods/coords_hud.smod").is_file());
    let list = fs::read_to_string(home.join("sunrise-mods.txt")).unwrap();
    assert_eq!(list, "mod.coords_hud:true\n");
    assert!(fs::read_dir(&home).unwrap().all(|file| {
        !file
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".installed-mods-")
    }));
    // Installed already: nothing to do.
    let again = put(&index, true, &home).unwrap();
    assert_eq!(again, done);

    let mut revoked = index.clone();
    revoked.revoked.push(index::Revoked {
        id: "coords_hud".into(),
        sha256: None,
        reason: "Test.".into(),
        date: String::new(),
    });
    let said = turn_off_revoked(&revoked, &home).unwrap();
    assert_eq!(said, ["Coordinates HUD was taken down and is off: Test."]);
    assert_eq!(
        fs::read_to_string(home.join("sunrise-mods.txt")).unwrap(),
        ""
    );

    installed::remove(&home, "coords_hud").unwrap();
    assert!(!home.join("sunrise-mods/coords_hud.smod").exists());
    assert!(installed::Installed::load(&home).entries.is_empty());
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn managed_play_checks_actual_packages_and_never_writes_the_enable_list() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path();
    let cache = home.join(CACHE);
    put(&sample(), true, home).unwrap();
    let text = fs::read_to_string(fixtures().join("index-v1.json")).unwrap();
    fs::create_dir_all(cache.parent().unwrap()).unwrap();
    fs::write(&cache, &text).unwrap();
    let offline = load(Err("offline".into()), &cache, false).unwrap();
    assert!(offline.index.project("coords_hud").is_some());
    assert!(offline.fetched().is_none());
    assert_eq!(
        check_before_play(home, &[], || offline.fetched()).unwrap(),
        ["Mods were not checked for registry takedowns."]
    );
    assert!(mod_list::is_enabled(home, "coords_hud"));

    let mut index = Index::parse(&text).unwrap();
    let mut installed = installed::Installed::load(home);
    let pkg = installed.entries[0].path(home).unwrap();
    installed.entries[0].checksum = format!("SHA-256:{}", "e".repeat(64));
    installed.save(home).unwrap();
    index.revoked.push(index::Revoked {
        id: "coords_hud".into(),
        sha256: Some(index::sha256_file(&pkg).unwrap()),
        reason: "Test.".into(),
        date: String::new(),
    });
    let text = serde_json::to_string(&index).unwrap();
    let local = load(Ok((index.clone(), text.clone())), &cache, true).unwrap();
    assert!(local.fetched().is_none());
    assert_eq!(
        check_before_play(home, &[], || local.fetched()).unwrap(),
        ["Mods were not checked for registry takedowns."]
    );
    assert!(mod_list::is_enabled(home, "coords_hud"));

    let live = load(Ok((index, text)), &cache, false).unwrap();
    assert!(live.fetched().is_some());
    let list = home.join("sunrise-mods.txt");
    let before = fs::read(&list).unwrap();
    let error = check_before_play(home, &[], || live.fetched()).unwrap_err();
    assert!(error.contains("was taken down: Test."), "{error}");
    assert_eq!(fs::read(&list).unwrap(), before);
    assert!(mod_list::is_enabled(home, "coords_hud"));
    mod_list::set_enabled(home, "coords_hud", false).unwrap();
    assert!(check_before_play(home, &[], || live.fetched())
        .unwrap()
        .is_empty());
}

#[test]
fn managed_play_refuses_alternate_profile_resources_and_server_args() {
    let home = tempfile::tempdir().unwrap();
    let resources = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let new = || {
        let mut cmd = std::process::Command::new("sunrise-client");
        cmd.current_dir(home.path())
            .env("SUNRISE_HOME", home.path());
        cmd.arg("--resources").arg(resources.path());
        cmd
    };
    let check =
        |cmd: &std::process::Command| verify_launch_arguments(cmd, home.path(), resources.path());
    assert!(check(&new()).is_ok());
    assert!(check(new().arg("--resources").arg(other.path())).is_err());
    assert!(check(new().arg("--resources").arg(resources.path())).is_err());
    assert!(check(new().arg("--game-dir").arg(other.path())).is_err());
    assert!(check(new().env("SUNRISE_HOME", other.path())).is_err());
    assert!(check(new().arg("--home").arg(other.path())).is_err());
    assert!(check(new().arg("--dev-mod").arg(other.path())).is_err());
    assert!(check(new().arg("--dedicated")).is_err());
}

#[test]
fn registry_play_refuses_a_taken_down_mod_baked_into_a_build() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path();
    let cache = home.join(CACHE);
    let mut index = sample();
    let version = index.project("coords_hud").unwrap().versions[0].clone();
    index.revoked.push(index::Revoked {
        id: "coords_hud".into(),
        sha256: Some(version.file.sha256),
        reason: "Test.".into(),
        date: String::new(),
    });
    let built = [crate::native_status::Included {
        id: "coords_hud".into(),
        version: version.version,
        sha256: "a".repeat(64),
    }];
    let loaded = |index: Index| {
        let text = serde_json::to_string(&index).unwrap();
        load(Ok((index, text)), &cache, false).unwrap().fetched()
    };
    // Even if the installed-list entry and the enable flag are already gone, the build holds it.
    let error = check_before_play(home, &built, || loaded(index.clone())).unwrap_err();
    assert!(
        error.contains("includes a mod the registry took down"),
        "{error}"
    );
    let p = index
        .projects
        .iter_mut()
        .find(|p| p.id == "coords_hud")
        .unwrap();
    p.versions.clear();
    // Old builds don't record the registry hash. An unlisted version cannot rule it out.
    let error = check_before_play(home, &built, || loaded(index)).unwrap_err();
    assert!(error.contains("cannot rule out a mod"), "{error}");
}

#[test]
fn executable_content_is_refused_without_installing_or_skipping_code() {
    use std::io::Write;
    let home = scratch("executable");
    let path = home.join("executable.smod");
    fs::copy(fixtures().join("coords_hud-0.1.0.smod"), &path).unwrap();
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let mut zip = zip::ZipWriter::new_append(file).unwrap();
    zip.start_file("payload.exe", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"MZ").unwrap();
    zip.finish().unwrap();
    let mut index = sample();
    let p = &mut index.projects[0];
    p.key = None;
    let v = &mut p.versions[0];
    v.file.url = format!("file://{}", path.display());
    v.file.size = fs::metadata(&path).unwrap().len();
    v.file.sha256 = index::sha256_file(&path).unwrap();
    v.file.signature = None;
    let e = put(&index, true, &home).unwrap_err();
    assert!(
        e.contains("payload.exe: unsupported executable file"),
        "{e}"
    );
    assert!(!home.join("sunrise-mods/coords_hud.smod").exists());
    assert!(installed::Installed::load(&home).entries.is_empty());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn installs_a_shader_pack_into_shaderpacks() {
    let index = sample();
    let home = scratch("shader");
    let p = project_of(&index, index.project("glow").unwrap(), GAME).unwrap();
    assert_eq!(p.support(GAME), super::super::Support::Works);
    let done = install_from(&index, true, &p, &home, GAME, false, true, &[], &|_, _| {}).unwrap();
    assert_eq!(done[0].file, "shaderpacks/glow-1.0.0.sshader");
    assert_eq!(done[0].kind, Kind::Shader);
    assert!(home.join("shaderpacks/glow-1.0.0.sshader").is_file());
    // Not a mod: nothing turns on in sunrise-mods.txt; the game's Shader Packs page picks it.
    assert!(!home.join("sunrise-mods.txt").exists());

    let mut revoked = index.clone();
    revoked.revoked.push(index::Revoked {
        id: "glow".into(),
        sha256: None,
        reason: "Test.".into(),
        date: String::new(),
    });
    let said = turn_off_revoked(&revoked, &home).unwrap();
    assert_eq!(said, ["Soft Glow was taken down: Test. Remove it."]);

    installed::remove(&home, "glow").unwrap();
    assert!(!home.join("shaderpacks/glow-1.0.0.sshader").exists());
    // A file of the same name the registry didn't put there is left alone.
    fs::write(home.join("shaderpacks/glow-1.0.0.sshader"), b"mine").unwrap();
    let e = install_from(&index, true, &p, &home, GAME, false, true, &[], &|_, _| {}).unwrap_err();
    assert_eq!(
        e.to_string(),
        "shaderpacks/glow-1.0.0.sshader is already there: move it away first."
    );
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn refuses_a_changed_file_a_taken_id_and_local_files_from_the_real_registry() {
    let index = sample();

    let home = scratch("tampered");
    let mut tampered = index.clone();
    let cozy = tampered.projects[1].versions[0].file.url.clone();
    tampered.projects[0].versions[0].file.url = cozy;
    let e = put(&tampered, true, &home).unwrap_err();
    assert!(e.contains("larger than its source says"), "{e}");
    assert!(!home.join("sunrise-mods").exists());

    let e = put(&index, false, &home).unwrap_err();
    assert!(e.contains("file servers"), "{e}");

    fs::create_dir_all(home.join("sunrise-mods/coords_hud")).unwrap();
    let e = put(&index, true, &home).unwrap_err();
    assert!(e.contains("already in sunrise-mods"), "{e}");

    let mut taken_down = index.clone();
    taken_down.revoked.push(index::Revoked {
        id: "coords_hud".into(),
        sha256: Some(index.projects[0].versions[0].file.sha256.clone()),
        reason: "Test.".into(),
        date: String::new(),
    });
    let e = put(&taken_down, true, &home).unwrap_err();
    assert_eq!(e.to_string(), "Coordinates HUD was taken down: Test.");
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn registry_versions_run_when_made_for_this_game() {
    use super::super::{pick, support, Support};
    let v = |game: &str| vec![game.to_owned()];
    let sunrise = vec![LOADER.to_owned()];
    assert_eq!(support(Kind::Mod, &v(GAME), &sunrise, GAME), Support::Works);
    assert!(!support(Kind::Mod, &v("26.2"), &sunrise, GAME).installable());
    let index = sample();
    let versions = |id: &str| -> Vec<Version> {
        let p = index.project(id).unwrap();
        p.versions
            .iter()
            .map(|x| version_of(&index, p, x))
            .collect()
    };
    assert_eq!(
        pick(Kind::Modpack, &versions("cozy"), false, GAME)
            .unwrap()
            .number,
        "1.0.0"
    );
    let hud = versions("coords_hud");
    assert!(hud[0].blocked.is_none());
    assert_eq!(pick(Kind::Mod, &hud, false, GAME).unwrap().number, "0.1.0");
}

#[test]
fn a_project_made_for_the_earlier_mod_system_says_it_cant_be_installed() {
    let mut index = sample();
    let hud = index
        .projects
        .iter_mut()
        .find(|p| p.id == "coords_hud")
        .unwrap();
    hud.versions[0].game = GAME.into();
    hud.versions[0].capabilities = vec!["game-read".to_string()].into();
    let p = project_of(&index, index.project("coords_hud").unwrap(), GAME).unwrap();
    assert!(!p.support(GAME).installable());
    assert!(p
        .support(GAME)
        .reason()
        .unwrap()
        .contains("earlier mod system"));
    // Beside a mod that installs, with fewer downloads:
    let mut other = index.project("coords_hud").unwrap().clone();
    other.id = "lanterns_plus".into();
    other.versions[0].capabilities = Default::default();
    other.downloads = 0;
    index.projects.push(other);
    index
        .projects
        .iter_mut()
        .find(|p| p.id == "coords_hud")
        .unwrap()
        .downloads = 1_000_000;
    let ids = |everything: bool| -> Vec<String> {
        let mut q = Query::new("", Kind::Mod);
        q.everything = everything;
        search(&index, &q).into_iter().map(|p| p.id).collect()
    };
    // The list leaves it out, as the note above it says.
    assert!(!ids(false).iter().any(|id| id == "coords_hud"));
    // Asked for everything, it comes after what installs, despite its downloads.
    let order = ids(true);
    let at = |id: &str| order.iter().position(|o| o == id).unwrap();
    assert!(at("lanterns_plus") < at("coords_hud"), "{order:?}");
    // The same project with a version this Sunrise can use installs as before.
    let mut fine = sample();
    fine.projects
        .iter_mut()
        .find(|p| p.id == "coords_hud")
        .unwrap()
        .versions[0]
        .game = GAME.into();
    let p = project_of(&fine, fine.project("coords_hud").unwrap(), GAME).unwrap();
    assert!(p.blocked.is_none());
}

#[test]
fn install_takes_the_newest_version_this_build_can_install() {
    let mut index = sample();
    let versions = &mut index.projects[0].versions;
    // A newer version above the sample's, asking for a capability this build does not know.
    let mut newer = versions[0].clone();
    newer.version = "9.0.0".into();
    newer.capabilities = vec!["game-read".to_string()].into();
    versions.insert(0, newer);
    let home = scratch("newest-installable");
    let done = install_from(
        &index,
        true,
        &hud(&index),
        &home,
        GAME,
        false,
        true,
        &[],
        &|_, _| {},
    )
    .unwrap();
    assert_eq!(done[0].version, "0.1.0");
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn install_keeps_the_takedown_reason_of_a_version_it_could_not_install_anyway() {
    let mut index = sample();
    let v = &mut index.projects[0].versions[0];
    v.capabilities = vec!["game-read".to_string()].into();
    let sha256 = v.file.sha256.clone();
    index.revoked.push(index::Revoked {
        id: "coords_hud".into(),
        sha256: Some(sha256),
        reason: "Test.".into(),
        date: String::new(),
    });
    let home = scratch("unsupported-and-revoked");
    let e = install_from(
        &index,
        true,
        &hud(&index),
        &home,
        GAME,
        false,
        true,
        &[],
        &|_, _| {},
    )
    .unwrap_err();
    assert_eq!(e.to_string(), "Coordinates HUD was taken down: Test.");
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn a_summary_the_registry_cut_mid_word_ends_on_a_whole_word() {
    let cut = format!("{} and tools at the crafti", "x".repeat(SUMMARY_CUT - 24));
    assert_eq!(cut.chars().count(), SUMMARY_CUT);
    assert!(summary(&cut).ends_with(" at the…"), "{}", summary(&cut));
    // Whole sentences, and short ones, stay as written.
    let whole = format!("{}.", "x ".repeat(SUMMARY_CUT / 2));
    assert_eq!(summary(&whole), whole);
    assert_eq!(summary("A short one"), "A short one");
}

#[test]
fn owner_links_only_take_github_logins() {
    for ok in ["Stink-O", "ludovic111", "a"] {
        assert!(github_login(ok), "{ok}");
    }
    for bad in [
        "",
        "-x",
        "x-",
        "a--b",
        "bad login",
        "a/b",
        "x?y=1",
        &"a".repeat(40),
    ] {
        assert!(!github_login(bad), "{bad}");
    }
}

#[test]
fn ratings_keep_only_what_is_well_formed() {
    let text = r#"{"projects": {
        "zoom": {"average": 4.25, "count": 12},
        "minimap": {"average": 9, "count": 3},
        "keystrokes": {"average": 3, "count": 0},
        "Bad Id": {"average": 4, "count": 1},
        "fps_overlay": {"average": "5", "count": 1}
    }}"#;
    let ratings = parse_ratings(text);
    assert_eq!(
        ratings.into_iter().collect::<Vec<_>>(),
        [(
            "zoom".to_owned(),
            Rating {
                average: 4.25,
                count: 12
            }
        )]
    );
    assert!(parse_ratings("not json").is_empty());
    assert!(parse_ratings(r#"{"projects": []}"#).is_empty());
    assert_eq!(rate_url("zoom"), "");
}

#[test]
fn a_project_is_starred_on_the_repository_its_source_is_in() {
    let repo = |s: &str| github_repo(s);
    let daybreak = Some("https://github.com/hildlabs/sunrise-shaders".to_owned());
    // A folder of it, a trailing slash, `.git`, a query or an anchor: the same repository.
    for source in [
        "https://github.com/hildlabs/sunrise-shaders",
        "https://github.com/hildlabs/sunrise-shaders/tree/main/pack",
        "https://github.com/hildlabs/sunrise-shaders/",
        "https://github.com/hildlabs/sunrise-shaders.git",
        "https://github.com/hildlabs/sunrise-shaders?tab=readme",
        "https://github.com/hildlabs/sunrise-shaders#readme",
    ] {
        assert_eq!(repo(source), daybreak, "{source:?}");
    }
    assert_eq!(
        repo("https://github.com/horizon-rust/horizon.v2_mods"),
        Some("https://github.com/horizon-rust/horizon.v2_mods".to_owned())
    );
    // Not GitHub, no repository, or a name GitHub doesn't allow: no star button.
    for source in [
        "https://gitlab.com/hildlabs/sunrise-shaders",
        "http://github.com/hildlabs/sunrise-shaders",
        "https://github.com.evil.example/hildlabs/sunrise-shaders",
        "https://github.com/hildlabs",
        "https://github.com/hildlabs/",
        "https://github.com/-hildlabs/sunrise-shaders",
        "https://github.com/hildlabs/..",
        "https://github.com/hildlabs/sunrise shaders",
        "",
    ] {
        assert_eq!(repo(source), None, "{source:?}");
    }
}

/// A dependency listed with an older and a newer version, the plan choosing the newer: the file
/// that lands is the planned version, not the first one the walk finds in range.
#[test]
fn a_dependency_is_installed_at_the_version_the_plan_chose() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = sample();
    let base = index.project("coords_hud").unwrap().clone();
    let make = |id: &str, title: &str, number: &str, extra: &str| {
        let manifest = format!("id='{id}'\nname='{title}'\nversion='{number}'\napi='0.2'\nside='both'\ncapabilities=['registries']\n{extra}");
        let bytes = sunrise_launcher_smod::Package::write_files(&manifest, &[]).unwrap();
        let name = format!("{id}-{number}.smod");
        let file = dir.path().join(&name);
        fs::write(&file, &bytes).unwrap();
        let facts = index::inspect(&index::Kind::Mod, &file).unwrap();
        let mut version = base.versions[0].clone();
        version.version = number.into();
        version.api = facts.api;
        version.side = facts.side;
        version.requires.clear();
        version.file = index::File {
            name,
            url: format!("file://{}", file.display()),
            size: bytes.len() as u64,
            sha256: index::sha256(&bytes),
            signature: None,
        };
        version
    };
    let mut fireflies = make(
        "fireflies",
        "Fireflies",
        "1.0.0",
        "[dependencies]\nsunrise_creatures = '^1'",
    );
    fireflies.dependencies = Some(vec![index::Dependency {
        id: "sunrise_creatures".into(),
        version: "^1".into(),
        optional: false,
        side: None,
    }]);
    // The older version is listed first: only the plan says the newer one is wanted.
    let creatures = vec![
        make("sunrise_creatures", "Sunrise Creatures", "1.0.0", ""),
        make("sunrise_creatures", "Sunrise Creatures", "1.1.0", ""),
    ];
    for (id, title, versions) in [
        ("fireflies", "Fireflies", vec![fireflies]),
        ("sunrise_creatures", "Sunrise Creatures", creatures),
    ] {
        index.projects.push(index::Project {
            id: id.into(),
            name: title.into(),
            key: None,
            versions,
            ..base.clone()
        });
    }
    let project = project_of(&index, index.project("fireflies").unwrap(), GAME).unwrap();
    let home = dir.path().join("profile");
    let done = install_from(
        &index,
        true,
        &project,
        &home,
        GAME,
        false,
        true,
        &[],
        &|_, _| {},
    )
    .unwrap();
    let creatures = done
        .iter()
        .find(|e| e.project == "sunrise_creatures")
        .unwrap();
    assert_eq!(creatures.version, "1.1.0");
    assert!(home.join(&creatures.file).is_file());
}

#[test]
fn a_dependency_the_plan_moves_to_a_newer_version_is_updated_and_stays_off_when_it_was_off() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = sample();
    let base = index.project("coords_hud").unwrap().clone();
    let make = |id: &str, number: &str, extra: &str| {
        let manifest = format!("id='{id}'\nname='{id}'\nversion='{number}'\napi='0.2'\nside='both'\ncapabilities=['registries']\n{extra}");
        let bytes = sunrise_launcher_smod::Package::write_files(&manifest, &[]).unwrap();
        let name = format!("{id}-{number}.smod");
        let file = dir.path().join(&name);
        fs::write(&file, &bytes).unwrap();
        let facts = index::inspect(&index::Kind::Mod, &file).unwrap();
        let mut version = base.versions[0].clone();
        version.version = number.into();
        version.api = facts.api;
        version.side = facts.side;
        version.requires.clear();
        version.file = index::File {
            name,
            url: format!("file://{}", file.display()),
            size: bytes.len() as u64,
            sha256: index::sha256(&bytes),
            signature: None,
        };
        version
    };
    let mut fireflies = make("fireflies", "1.0.0", "[dependencies]\ncreatures = '^2'");
    fireflies.dependencies = Some(vec![index::Dependency {
        id: "creatures".into(),
        version: "^2".into(),
        optional: false,
        side: None,
    }]);
    // The older one is listed first, so that installing creatures alone picks it.
    let creatures = vec![
        make("creatures", "1.0.0", ""),
        make("creatures", "2.0.0", ""),
    ];
    for (id, versions) in [("fireflies", vec![fireflies]), ("creatures", creatures)] {
        index.projects.push(index::Project {
            id: id.into(),
            name: id.into(),
            key: None,
            versions,
            ..base.clone()
        });
    }
    let home = dir.path().join("profile");
    let first = project_of(&index, index.project("creatures").unwrap(), GAME).unwrap();
    install_from(
        &index,
        true,
        &first,
        &home,
        GAME,
        false,
        true,
        &[],
        &|_, _| {},
    )
    .unwrap();
    mod_list::set_enabled(&home, "creatures", false).unwrap();
    let project = project_of(&index, index.project("fireflies").unwrap(), GAME).unwrap();

    let done = install_from(
        &index,
        true,
        &project,
        &home,
        GAME,
        false,
        true,
        &[],
        &|_, _| {},
    )
    .unwrap();

    let creatures = done.iter().find(|e| e.project == "creatures").unwrap();
    assert_eq!(creatures.version, "2.0.0", "the plan's update is installed");
    assert!(home.join(&creatures.file).is_file());
    assert!(
        !mod_list::is_enabled(&home, "creatures"),
        "an off dependency stays off"
    );
    assert!(mod_list::is_enabled(&home, "fireflies"));
    // The mod asked for stays off too when it was installed and off: an update never switches
    // a mod on (Update on its row installs it again).
    mod_list::set_enabled(&home, "creatures", true).unwrap();
    mod_list::set_enabled(&home, "fireflies", false).unwrap();
    install_from(
        &index,
        true,
        &project,
        &home,
        GAME,
        false,
        true,
        &[],
        &|_, _| {},
    )
    .unwrap();
    assert!(!mod_list::is_enabled(&home, "fireflies"));
}

/// An index with `creatures` 1.0.0 and 2.0.0, `lamps` needing `creatures ^1` and `fireflies`
/// needing `creatures ^2`, every file written under `dir`.
fn conflict_index(dir: &Path) -> Index {
    let mut index = sample();
    let base = index.project("coords_hud").unwrap().clone();
    let make = |id: &str, number: &str, needs: Option<&str>| {
        let extra = needs.map_or(String::new(), |r| {
            format!("[dependencies]\ncreatures = '{r}'")
        });
        let manifest = format!("id='{id}'\nname='{id}'\nversion='{number}'\napi='0.2'\nside='both'\ncapabilities=['registries']\n{extra}");
        let bytes = sunrise_launcher_smod::Package::write_files(&manifest, &[]).unwrap();
        let name = format!("{id}-{number}.smod");
        let file = dir.join(&name);
        fs::write(&file, &bytes).unwrap();
        let facts = index::inspect(&index::Kind::Mod, &file).unwrap();
        let mut version = base.versions[0].clone();
        version.version = number.into();
        version.api = facts.api;
        version.side = facts.side;
        version.requires.clear();
        version.dependencies = needs.map(|range| {
            vec![index::Dependency {
                id: "creatures".into(),
                version: range.into(),
                optional: false,
                side: None,
            }]
        });
        version.file = index::File {
            name,
            url: format!("file://{}", file.display()),
            size: bytes.len() as u64,
            sha256: index::sha256(&bytes),
            signature: None,
        };
        version
    };
    for (id, versions) in [
        (
            "creatures",
            vec![
                make("creatures", "1.0.0", None),
                make("creatures", "2.0.0", None),
            ],
        ),
        ("lamps", vec![make("lamps", "1.0.0", Some("^1"))]),
        ("fireflies", vec![make("fireflies", "1.0.0", Some("^2"))]),
    ] {
        index.projects.push(index::Project {
            id: id.into(),
            name: id.into(),
            key: None,
            versions,
            ..base.clone()
        });
    }
    index
}

#[test]
fn a_waiting_package_restores_its_choice_only_on_different_fitting_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let id = "creatures";
    let mut index = conflict_index(dir.path());
    let project = index.projects.iter_mut().find(|p| p.id == id).unwrap();
    let replacement = project.versions.pop().unwrap();
    let old = project.versions[0].clone();
    for was_on in [false, true] {
        let home = dir.path().join(was_on.to_string());
        let previous_only = index.project(id).unwrap();
        let shown = project_of(&index, previous_only, GAME).unwrap();
        let installed = install_from(
            &index,
            true,
            &shown,
            &home,
            GAME,
            false,
            true,
            &[],
            &|_, _| {},
        )
        .unwrap();
        let file = PathBuf::from(&installed[0].file);
        mod_list::set_enabled(&home, id, false).unwrap();
        let waiting = [(
            id.into(),
            txn::WaitingMod::test_package(&home, file.clone(), was_on),
        )]
        .into();
        assert!(registry_candidates(&index, [id], GAME, &waiting).is_err());
        assert!(!mod_list::is_enabled(&home, id));
        assert_eq!(
            index::sha256(&fs::read(home.join(file)).unwrap()),
            old.file.sha256
        );

        let mut current = index.clone();
        current
            .projects
            .iter_mut()
            .find(|p| p.id == id)
            .unwrap()
            .versions
            .push(replacement.clone());
        let chosen = registry_candidates(&current, [id], GAME, &waiting).unwrap();
        let done = dependencies::stage(
            &Install {
                index: &current,
                local: true,
                home: &home,
                approvals: &[],
                progress: &|_, _| {},
            },
            &chosen,
            false,
            &waiting,
        )
        .unwrap();
        let bytes = fs::read(home.join(&done[0].file)).unwrap();
        assert_eq!(index::sha256(&bytes), replacement.file.sha256);
        assert_eq!(mod_list::is_enabled(&home, id), was_on);
    }
}

#[test]
fn several_mods_install_as_one_attempt_and_one_failure_publishes_none_of_them() {
    let dir = tempfile::tempdir().unwrap();
    let index = conflict_index(dir.path());
    let both = ["lamps", "creatures"]
        .map(|id| project_of(&index, index.project(id).unwrap(), GAME).unwrap());
    let set = |home: &Path| {
        install_set_from(
            &index,
            true,
            &both,
            home,
            GAME,
            false,
            true,
            &[],
            &|_, _| {},
        )
    };
    let home = dir.path().join("whole");
    let done = set(&home).unwrap();
    assert_eq!(done.len(), 2, "{done:?}");
    for id in ["lamps", "creatures"] {
        assert!(mod_list::is_enabled(&home, id));
        assert!(installed::Installed::load(&home)
            .find(SourceId::Sunrise, id)
            .is_some());
    }
    assert!(!home.join("mod-browser/install-txn.json").exists());
    // The second one's file is gone: the first one, which downloaded fine, is not published.
    let apart = dir.path().join("cut");
    fs::remove_file(dir.path().join("creatures-1.0.0.smod")).unwrap();
    assert!(set(&apart).is_err());
    assert!(installed::Installed::load(&apart).entries.is_empty());
    assert!(!apart.join("sunrise-mods/lamps.smod").exists());
    assert!(!mod_list::is_enabled(&apart, "lamps"));
    assert!(!apart.join("mod-browser/install-txn.json").exists());
}

#[test]
fn an_install_that_replaces_a_conflicting_mod_removes_it_with_the_install() {
    let dir = tempfile::tempdir().unwrap();
    let index = conflict_index(dir.path());
    let home = dir.path().join("profile");
    let lamps = project_of(&index, index.project("lamps").unwrap(), GAME).unwrap();
    install_from(
        &index,
        true,
        &lamps,
        &home,
        GAME,
        false,
        true,
        &[],
        &|_, _| {},
    )
    .unwrap();
    let fireflies = project_of(&index, index.project("fireflies").unwrap(), GAME).unwrap();
    // Without removing lamps the plan refuses.
    assert!(install_from(
        &index,
        true,
        &fireflies,
        &home,
        GAME,
        false,
        true,
        &[],
        &|_, _| {}
    )
    .is_err());

    let replacing = [installed::Approval::Remove(installed::Replace {
        id: "lamps".into(),
        checksum: installed::Installed::load(&home)
            .find(SourceId::Sunrise, "lamps")
            .unwrap()
            .checksum
            .clone(),
        worlds_ok: false,
    })];
    install_from(
        &index,
        true,
        &fireflies,
        &home,
        GAME,
        false,
        true,
        &replacing,
        &|_, _| {},
    )
    .unwrap();

    let list = installed::Installed::load(&home);
    assert!(list.find(SourceId::Sunrise, "lamps").is_none());
    assert_eq!(
        list.find(SourceId::Sunrise, "creatures").unwrap().version,
        "2.0.0"
    );
    assert!(list.find(SourceId::Sunrise, "fireflies").is_some());
    assert!(!home.join("sunrise-mods/lamps.smod").exists());
    assert!(!mod_list::is_enabled(&home, "lamps"));
    assert!(mod_list::is_enabled(&home, "fireflies"));
    assert!(!home.join("mod-browser/install-txn.json").exists());
}

#[test]
fn a_staged_conflict_never_removes_an_installed_package() {
    let dir = tempfile::tempdir().unwrap();
    let index = conflict_index(dir.path());
    let home = dir.path().join("profile");
    let lamps = project_of(&index, index.project("lamps").unwrap(), GAME).unwrap();
    install_from(
        &index,
        true,
        &lamps,
        &home,
        GAME,
        false,
        true,
        &[],
        &|_, _| {},
    )
    .unwrap();
    let row = installed::Installed::load(&home)
        .find(SourceId::Sunrise, "lamps")
        .unwrap()
        .clone();
    let package = row.path(&home).unwrap();
    let browser = home.join(installed::MANIFEST);
    let enable = home.join(sunrise_launcher_smod::mod_list::FILE);
    let paths = [&package, &browser, &enable];
    let bytes = paths.map(|path| fs::read(path).unwrap());
    let p = index.project("fireflies").unwrap();
    let refused = dependencies::stage(
        &Install {
            index: &index,
            local: true,
            home: &home,
            approvals: &[installed::Approval::Remove(installed::Replace {
                id: "lamps".into(),
                checksum: row.checksum,
                worlds_ok: true,
            })],
            progress: &|_, _| {},
        },
        &[(p, &p.versions[0])],
        false,
        &Default::default(),
    )
    .unwrap_err();
    assert!(matches!(refused, installed::InstallError::Conflict { mods, .. } if mods == ["lamps"]));
    assert_eq!(paths.map(|path| fs::read(path).unwrap()), bytes);
    assert!(!home.join("mod-browser/install-txn.json").exists());
}

#[test]
fn a_queued_approval_is_read_again_under_the_lock_before_it_removes_anything() {
    let dir = tempfile::tempdir().unwrap();
    let index = conflict_index(dir.path());
    let home = dir.path().join("profile");
    let lamps = project_of(&index, index.project("lamps").unwrap(), GAME).unwrap();
    let go = |approvals: &[installed::Approval]| {
        let fireflies = project_of(&index, index.project("fireflies").unwrap(), GAME).unwrap();
        install_from(
            &index,
            true,
            &fireflies,
            &home,
            GAME,
            false,
            true,
            approvals,
            &|_, _| {},
        )
    };
    install_from(
        &index,
        true,
        &lamps,
        &home,
        GAME,
        false,
        true,
        &[],
        &|_, _| {},
    )
    .unwrap();
    let approved = installed::Replace {
        id: "lamps".into(),
        checksum: installed::Installed::load(&home)
            .find(SourceId::Sunrise, "lamps")
            .unwrap()
            .checksum
            .clone(),
        worlds_ok: false,
    };
    // The file changed while the approval waited: nothing is removed.
    let stale = installed::Replace {
        checksum: "SHA-256:other".into(),
        ..approved.clone()
    };
    let refused = go(&[installed::Approval::Remove(stale)]).unwrap_err();
    assert!(refused.contains("changed since you approved"), "{refused}");
    assert!(installed::Installed::load(&home)
        .find(SourceId::Sunrise, "lamps")
        .is_some());
    // A world now uses it, and nobody agreed to what it loses.
    let world = home.join("saves/Home");
    fs::create_dir_all(&world).unwrap();
    fs::write(world.join("level.dat"), b"not nbt").unwrap();
    let refused = go(&[installed::Approval::Remove(approved.clone())]).unwrap_err();
    assert!(
        matches!(&refused, installed::InstallError::NeedsWorlds { mods } if mods == &["lamps"])
    );
    assert!(installed::Installed::load(&home)
        .find(SourceId::Sunrise, "lamps")
        .is_some());
    // Agreed: it goes ahead. Worlds that can't be looked at stop it when nobody agreed.
    go(&[
        installed::Approval::Remove(approved),
        installed::Approval::DropContent { id: "lamps".into() },
    ])
    .unwrap();
    assert!(installed::Installed::load(&home)
        .find(SourceId::Sunrise, "lamps")
        .is_none());
}

#[test]
fn a_removal_is_one_transaction_that_takes_only_the_files_it_was_shown() {
    let dir = tempfile::tempdir().unwrap();
    let index = conflict_index(dir.path());
    let home = dir.path().join("profile");
    let lamps = project_of(&index, index.project("lamps").unwrap(), GAME).unwrap();
    install_from(
        &index,
        true,
        &lamps,
        &home,
        GAME,
        false,
        true,
        &[],
        &|_, _| {},
    )
    .unwrap();
    let checksum = installed::Installed::load(&home)
        .find(SourceId::Sunrise, "lamps")
        .unwrap()
        .checksum
        .clone();
    // A file that isn't the one that was shown stays.
    let stale = [("lamps".to_string(), "SHA-256:other".to_string())];
    assert!(remove_checked(&home, &stale).unwrap().is_empty());
    assert!(home.join("sunrise-mods/lamps.smod").is_file());
    let shown = [("lamps".to_string(), checksum)];
    assert_eq!(remove_checked(&home, &shown).unwrap(), ["lamps"]);
    assert!(!home.join("sunrise-mods/lamps.smod").exists());
    assert!(installed::Installed::load(&home)
        .find(SourceId::Sunrise, "lamps")
        .is_none());
    assert!(!mod_list::is_enabled(&home, "lamps"));
    assert!(!home.join("mod-browser/install-txn.json").exists());
    // A cut-off removal is undone by the record: begin it, remove the file, never commit.
    install_from(
        &index,
        true,
        &lamps,
        &home,
        GAME,
        false,
        true,
        &[],
        &|_, _| {},
    )
    .unwrap();
    let staged = tempfile::Builder::new()
        .prefix(".installed-mods-")
        .tempdir_in(&home)
        .unwrap();
    let file = home.join("sunrise-mods/lamps.smod");
    txn::begin(&home, staged.path(), &[(None, file.clone())], &[]).unwrap();
    fs::rename(&file, staged.path().join("old-0")).unwrap();
    let _ = staged.keep();
    assert!(txn::recover(&home));
    assert!(
        file.is_file(),
        "the removal that never finished is put back"
    );
}

#[test]
fn an_install_that_replaces_a_mod_and_then_fails_leaves_that_mod_installed() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = conflict_index(dir.path());
    let home = dir.path().join("profile");
    let lamps = project_of(&index, index.project("lamps").unwrap(), GAME).unwrap();
    install_from(
        &index,
        true,
        &lamps,
        &home,
        GAME,
        false,
        true,
        &[],
        &|_, _| {},
    )
    .unwrap();
    // The new mod's file is gone: the download fails after the plan said yes.
    let broken = index
        .projects
        .iter_mut()
        .find(|p| p.id == "fireflies")
        .unwrap();
    broken.versions[0].file.url = format!("file://{}", dir.path().join("missing.smod").display());
    let fireflies = project_of(&index, index.project("fireflies").unwrap(), GAME).unwrap();

    let replacing = [installed::Approval::Remove(installed::Replace {
        id: "lamps".into(),
        checksum: installed::Installed::load(&home)
            .find(SourceId::Sunrise, "lamps")
            .unwrap()
            .checksum
            .clone(),
        worlds_ok: false,
    })];
    assert!(install_from(
        &index,
        true,
        &fireflies,
        &home,
        GAME,
        false,
        true,
        &replacing,
        &|_, _| {}
    )
    .is_err());

    let list = installed::Installed::load(&home);
    assert!(
        list.find(SourceId::Sunrise, "lamps").is_some(),
        "lamps was not removed"
    );
    assert_eq!(
        list.find(SourceId::Sunrise, "creatures").unwrap().version,
        "1.0.0"
    );
    assert!(home.join("sunrise-mods/lamps.smod").is_file());
    assert!(mod_list::is_enabled(&home, "lamps"));
}

#[test]
fn a_set_whose_picks_need_conflicting_ranges_is_refused_before_anything_installs() {
    let index = sample();
    let base = index.project("coords_hud").unwrap().clone();
    let make = |id: &str, numbers: &[&str], dependencies: Option<Vec<index::Dependency>>| {
        let versions = numbers
            .iter()
            .map(|n| {
                let mut v = base.versions[0].clone();
                v.version = (*n).into();
                v.dependencies = dependencies.clone();
                v
            })
            .collect();
        index::Project {
            id: id.into(),
            name: id.into(),
            key: None,
            versions,
            ..base.clone()
        }
    };
    let need = |range: &str| {
        Some(vec![index::Dependency {
            id: "lib".into(),
            version: range.into(),
            optional: false,
            side: None,
        }])
    };
    let mut index = index;
    index.projects = vec![
        make("lib", &["2.0.0", "1.0.0"], None),
        make("one", &["1.0.0"], need("^1")),
        make("two", &["1.0.0"], need("^2")),
        make("fine", &["1.0.0"], need("^1")),
    ];
    let say = |ids: &[&str]| {
        let ids: Vec<String> = ids.iter().map(|id| (*id).to_owned()).collect();
        check_set(&index, &ids, &[], GAME)
    };
    let plan = say(&["one", "fine"]).unwrap();
    assert_eq!(plan.install.len(), 3, "both and the library once");
    let refused = say(&["one", "two"]).unwrap_err();
    assert!(
        refused.contains("can't be installed with the others"),
        "{refused}"
    );
    // A set is refused as a whole: the pick that would have been fine does not go ahead alone.
    assert!(say(&["one", "two", "fine"]).is_err());
}
