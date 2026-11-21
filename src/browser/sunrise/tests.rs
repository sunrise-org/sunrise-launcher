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
    let run =
        |index: &Index| install_from(index, true, &project, &home, GAME, false, true, &|_, _| {});
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

    // A manifest we can't read is refused before staging or downloading any packages.
    fs::create_dir(home.join(installed::MANIFEST)).unwrap();
    let refused = install_from(&index, true, &project, &home, GAME, false, true, &|_, _| {
        panic!("unreadable claims must refuse before download")
    })
    .unwrap_err();
    assert!(refused.contains(installed::MANIFEST), "{refused}");
    assert!(!home.join(mod_list::MODS_DIR).exists());
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
    let refused =
        install_from(&index, true, &project, &home, GAME, false, true, &|_, _| {}).unwrap_err();
    assert!(refused.contains("installed but off"), "{refused}");
    let kept = install_from(
        &index,
        true,
        &project,
        &home,
        GAME,
        false,
        false,
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
        progress: &|_, _| {},
    };
    dependencies::install(&install, p, &p.versions[0], false, true).unwrap();
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
    assert!(!home.join("mod-browser/unreadable").exists());
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
        progress: &|_, _| {},
    };
    let claims = index::browser_state::Installed::read(&home).unwrap();
    let result = install.prepare(p, &p.versions[0], &list, &claims, &home.join("staging"));
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
        progress: &|_, _| {},
    };
    assert!(dependencies::install(&install, p, &p.versions[0], false, true).is_err());
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
fn native_sample(home: &Path) -> Index {
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
            progress: &|_, _| {},
        };
        let done = dependencies::install(&install, p, &p.versions[0], true, true).unwrap();
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
        progress: &|_, _| {},
    };
    dependencies::install(&install, p, &p.versions[0], false, true).unwrap();
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
        &|_, _| {}
    )
    .is_err());
    let (zip, number) =
        fetch_datapack_from(&index, true, &p, &dir.join("dl"), GAME, &|_| {}).unwrap();
    assert_eq!(number, "1.0.0");
    let world = dir.join("world");
    fs::create_dir(&world).unwrap();
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
    let other = install_from(&index, true, &p, &home, "26.9", false, true, &|_, _| {}).unwrap_err();
    assert!(other.contains("no version for 26.9"), "{other}");
    let done = install_from(&index, true, &p, &home, GAME, false, true, &|_, _| {}).unwrap();
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
fn registry_play_warns_for_disk_cache_and_turns_off_only_from_a_live_index() {
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
    index.revoked.push(index::Revoked {
        id: "coords_hud".into(),
        sha256: None,
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
    assert_eq!(
        check_before_play(home, &[], || live.fetched()).unwrap(),
        ["Coordinates HUD was taken down and is off: Test."]
    );
    assert!(!mod_list::is_enabled(home, "coords_hud"));
}

fn interrupted_game_install(
    home: &Path,
    registry: &Index,
    title: String,
) -> (installed::Entry, installed::Entry) {
    let old = put(registry, true, home).unwrap().remove(0);
    let bytes = sunrise_launcher_smod::Package::write_files(
        "id='coords_hud'\nname='Coordinates HUD'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n",
        &[],
    )
    .unwrap();
    let mut new = old.clone();
    new.title = title;
    new.version = "1.0.0".into();
    new.number = new.version.clone();
    new.checksum = format!("SHA-256:{}", index::sha256(&bytes));
    fs::write(
        home.join(index::browser_state::PENDING),
        serde_json::to_vec(&serde_json::json!({
            "id": "coords_hud", "file": old.file, "old": old, "new": new,
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(home.join(&old.file), bytes).unwrap();
    mod_list::set_enabled(home, "coords_hud", true).unwrap();
    (old, new)
}

#[test]
fn duplicate_or_unplaceable_ownership_stops_a_registry_install_before_download() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path();
    let index = sample();
    let old = put(&index, true, home).unwrap().remove(0);
    let project = index.project("coords_hud").unwrap();
    let version = &project.versions[0];
    let path = home.join(installed::MANIFEST);
    let bytes = fs::read(home.join(&old.file)).unwrap();
    let mut duplicate = old.clone();
    duplicate.file = "sunrise-mods/other.smod".into();
    let alias = serde_json::json!({
        "source": "future-source", "project": "other", "title": "Other",
        "file": "packs/../sunrise-mods/coords_hud.smod"
    });
    for (rows, expected) in [
        (serde_json::json!([old, duplicate]), "several entries"),
        (serde_json::json!([old, alias]), "belongs to Other"),
    ] {
        let before = serde_json::to_vec(&serde_json::json!({ "entries": rows })).unwrap();
        fs::write(&path, &before).unwrap();
        let error = install_version_guarded(
            &index,
            true,
            project,
            version,
            home,
            &|_| panic!("ownership must be checked first"),
            &|_, _| panic!("ownership must be checked before downloading"),
        )
        .unwrap_err();
        assert!(error.contains(expected), "{error}");
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read(home.join(&old.file)).unwrap(), bytes);
    }
}

#[test]
fn installers_never_move_an_aliased_old_archive_under_a_different_name() {
    for (batch, late_claim) in [(false, false), (true, false), (false, true), (true, true)] {
        let root = tempfile::tempdir().unwrap();
        let home = root.path();
        let index = sample();
        let mut old = put(&index, true, home).unwrap().remove(0);
        let target = home.join(&old.file);
        old.file = "sunrise-mods/old_coords.smod".into();
        old.version = "0.0.1".into();
        let old_path = home.join(&old.file);
        fs::rename(&target, &old_path).unwrap();
        fs::write(&old_path, b"unreadable archive").unwrap();
        let manifest = home.join(installed::MANIFEST);
        let contested = serde_json::to_vec(&serde_json::json!({"entries": [old, {
            "source": "future-source", "project": "other", "title": "Other owner",
            "file": "SUNRISE-MODS/OLD_COORDS.smod"
        }]}))
        .unwrap();
        if late_claim {
            fs::write(
                &manifest,
                serde_json::to_vec(&serde_json::json!({"entries": [old]})).unwrap(),
            )
            .unwrap();
        } else {
            fs::write(&manifest, &contested).unwrap();
        }
        let during_download = |_: &str, _: f32| {
            assert!(
                late_claim,
                "already-contested ownership must refuse before downloading"
            );
            fs::write(&manifest, &contested).unwrap();
        };
        let refused = if batch {
            install_from(
                &index,
                true,
                &hud(&index),
                home,
                GAME,
                false,
                true,
                &during_download,
            )
            .unwrap_err()
        } else {
            let p = index.project("coords_hud").unwrap();
            install_version_guarded(
                &index,
                true,
                p,
                &p.versions[0],
                home,
                &|_| Ok(()),
                &during_download,
            )
            .unwrap_err()
        };
        assert!(
            refused.contains("sunrise-mods/old_coords.smod belongs to Other owner"),
            "batch={batch} late_claim={late_claim}: {refused}"
        );
        assert_eq!(fs::read(manifest).unwrap(), contested);
        assert_eq!(fs::read(old_path).unwrap(), b"unreadable archive");
        assert!(!target.exists());
        assert!(!home.join("mod-browser/unreadable").exists());
    }
}

#[test]
fn registry_install_rechecks_ownership_before_moving_the_download() {
    let home = tempfile::tempdir().unwrap();
    let home = home.path();
    let index = sample();
    let project = index.project("coords_hud").unwrap();
    let manifest = home.join(installed::MANIFEST);
    let competing_claim = br#"{"entries":[{"source":"future-source","project":"other","title":"Other","file":"SUNRISE-MODS/COORDS_HUD.smod"}]}"#;
    let refused = install_version_guarded(
        &index,
        true,
        project,
        &project.versions[0],
        home,
        &|staged| {
            assert!(staged.is_file());
            // Simulate an editor that doesn't honor the home lock during the download.
            fs::write(&manifest, competing_claim).unwrap();
            Ok(())
        },
        &|_, _| {},
    )
    .unwrap_err();
    assert!(refused.contains("belongs to Other"), "{refused}");
    assert_eq!(fs::read(manifest).unwrap(), competing_claim);
    assert!(!home.join("sunrise-mods/coords_hud.smod").exists());
    assert!(fs::read_dir(home.join("mod-browser/downloads"))
        .unwrap()
        .next()
        .is_none());
}

#[test]
fn registry_play_recovers_a_large_game_journal_before_checking_revocations() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path();
    let mut registry = sample();
    let (old, new) = interrupted_game_install(home, &registry, "an allowed title ".repeat(20_000));
    let pending = home.join(index::browser_state::PENDING);
    assert!(fs::metadata(&pending).unwrap().len() > 256 << 10);
    let file = home.join(&old.file);
    let bytes = fs::read(&file).unwrap();
    registry.revoked.push(index::Revoked {
        id: "coords_hud".into(),
        sha256: Some(old.checksum.strip_prefix("SHA-256:").unwrap().into()),
        reason: "Old file taken down.".into(),
        date: String::new(),
    });

    let notices = check_before_play(home, &[], || Some(Arc::new(registry))).unwrap();
    assert!(notices.is_empty(), "{notices:?}");
    assert!(mod_list::is_enabled(home, "coords_hud"));
    assert!(!pending.exists());
    let rows: installed::Installed =
        serde_json::from_slice(&fs::read(home.join(installed::MANIFEST)).unwrap()).unwrap();
    assert_eq!(rows.entries, [new]);
    assert_eq!(fs::read(file).unwrap(), bytes);
}

#[test]
fn play_waits_for_a_writer_before_the_journal_is_visible_and_unlocks_before_fetching() {
    use std::{sync::mpsc, thread, time::Duration};
    let root = tempfile::tempdir().unwrap();
    let home = root.path();
    let registry = sample();
    let writer = crate::browser::lock_home(home).unwrap();
    assert!(!home.join(index::browser_state::PENDING).exists());
    assert!(!home.join(installed::MANIFEST).exists());
    thread::scope(|scope| {
        let (enter, entering) = mpsc::channel();
        let (done, completion) = mpsc::channel();
        let index = Arc::new(registry.clone());
        scope.spawn(move || {
            enter.send(()).unwrap();
            let result = check_before_play(home, &[], || {
                assert!(
                    thread::scope(|scope| scope
                        .spawn(|| { crate::browser::try_lock_home(home).is_some() })
                        .join()
                        .unwrap()),
                    "Play must not hold the browser lock during a fetch"
                );
                Some(index)
            });
            done.send(result).unwrap();
        });
        entering.recv().unwrap();
        assert_eq!(
            completion.recv_timeout(Duration::from_millis(150)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );
        let (_, new) = interrupted_game_install(home, &registry, "Published during Play".into());
        drop(writer);
        assert!(completion.recv().unwrap().unwrap().is_empty());
        assert!(!home.join(index::browser_state::PENDING).exists());
        assert_eq!(installed::Installed::read(home).unwrap().entries, [new]);
    });
}

#[test]
fn play_admission_names_the_game_journal_and_leaves_other_profiles_playable() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let home = first.path();
    let registry = sample();
    let (old, _) = interrupted_game_install(home, &registry, "Coordinates HUD".into());
    let pending = home.join(index::browser_state::PENDING);
    let manifest = home.join(installed::MANIFEST);
    let package = home.join(&old.file);
    let package_bytes = fs::read(&package).unwrap();
    let valid_journal = fs::read(&pending).unwrap();
    let valid_manifest = fs::read(&manifest).unwrap();
    let duplicate = serde_json::to_vec(&serde_json::json!({"entries": [old, old]})).unwrap();
    for (journal, rows, expected) in [
        (
            b"invalid json".as_slice(),
            &valid_manifest,
            index::browser_state::PENDING,
        ),
        (&valid_journal, &duplicate, "several entries"),
    ] {
        fs::write(&pending, journal).unwrap();
        fs::write(&manifest, rows).unwrap();
        let detail = installed::recover_game_install(home).unwrap_err().details();
        assert!(
            !detail.contains(&home.display().to_string()),
            "local path leaked: {detail}"
        );
        assert!(
            check_before_play(home, &[], || panic!("no registry call before recovery")).is_err()
        );
        assert!(detail.contains(expected), "{detail}");
        assert_eq!(fs::read(&pending).unwrap(), journal);
        assert_eq!(fs::read(&manifest).unwrap(), *rows);
        assert_eq!(fs::read(&package).unwrap(), package_bytes);
        assert!(check_before_play(second.path(), &[], || None).is_ok());
    }
    fs::write(&manifest, valid_manifest).unwrap();
    assert!(installed::recover_game_install(home).is_ok());
    assert!(!pending.exists());
}

#[test]
fn registry_play_preserves_metadata_edited_after_a_game_journal_was_written() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path();
    let registry = sample();
    let (mut current, _) = interrupted_game_install(home, &registry, "Coordinates HUD".into());
    current.dependency = !current.dependency;
    let manifest = home.join(installed::MANIFEST);
    let edited = serde_json::to_vec(&serde_json::json!({ "entries": [current] })).unwrap();
    fs::write(&manifest, &edited).unwrap();

    let error = check_before_play(home, &[], || Some(Arc::new(registry))).unwrap_err();
    assert!(error.contains("changed since the install began"));
    assert!(mod_list::is_enabled(home, "coords_hud"));
    assert_eq!(fs::read(manifest).unwrap(), edited);
    assert!(home.join(index::browser_state::PENDING).exists());
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
    let done = install_from(&index, true, &p, &home, GAME, false, true, &|_, _| {}).unwrap();
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
    let e = install_from(&index, true, &p, &home, GAME, false, true, &|_, _| {}).unwrap_err();
    assert_eq!(
        e,
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
    assert_eq!(e, "Coordinates HUD was taken down: Test.");
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
        &|_, _| {},
    )
    .unwrap_err();
    assert_eq!(e, "Coordinates HUD was taken down: Test.");
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
    let done = install_from(&index, true, &project, &home, GAME, false, true, &|_, _| {}).unwrap();
    let creatures = done
        .iter()
        .find(|e| e.project == "sunrise_creatures")
        .unwrap();
    assert_eq!(creatures.version, "1.1.0");
    assert!(home.join(&creatures.file).is_file());
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
