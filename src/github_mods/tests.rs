use super::*;
use serde_json::{json, Value};

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
const SHA2: &str = "fedcba9876543210fedcba9876543210fedcba98";

fn link(text: &str) -> Link {
    parse(text).unwrap()
}

#[test]
fn links_name_a_repository_release_ref_or_commit() {
    let plain = link("https://github.com/acme/zoom");
    assert_eq!(
        (plain.owner.as_str(), plain.repo.as_str(), plain.target),
        ("acme", "zoom", Target::Latest)
    );
    assert_eq!(link("https://github.com/acme/zoom.git/").repo, "zoom");
    assert_eq!(
        link("https://github.com/acme/zoom/releases").target,
        Target::Latest
    );
    assert_eq!(
        link("https://github.com/acme/zoom/releases/tag/v1.2.0").target,
        Target::Tag("v1.2.0".into())
    );
    // A branch name keeps its slashes.
    assert_eq!(
        link("https://github.com/acme/zoom/tree/feature/x?tab=1").target,
        Target::Ref("feature/x".into())
    );
    assert_eq!(
        link("https://github.com/acme/zoom/commit/ABCDEF1").target,
        Target::Commit("abcdef1".into())
    );
}

#[test]
fn links_that_are_not_github_repositories_are_refused() {
    for bad in [
        "",
        "acme/zoom",
        "http://github.com/acme/zoom",
        "https://gitlab.com/acme/zoom",
        "https://github.com/acme",
        "https://github.com/acme/zoom/issues/3",
        "https://github.com/acme/zoom/commit/not-hex",
        "https://github.com/acme/zoom/tree/a..b",
        "https://github.com/../zoom",
        "https://github.com/acme/zo om",
    ] {
        assert!(parse(bad).is_err(), "{bad:?}");
    }
}

/// A recorded API: the JSON each path answers, 404 for the rest.
fn recorded(answers: Vec<(&'static str, Value)>) -> impl Fn(&str) -> Result<Option<Value>, String> {
    move |path| {
        Ok(answers
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, v)| v.clone()))
    }
}

fn release(tag: &str, assets: Value) -> Value {
    json!({ "tag_name": tag, "assets": assets })
}

fn asset(name: &str, digest: Option<&str>) -> Value {
    json!({
        "name": name,
        "browser_download_url": format!("https://github.com/acme/zoom/releases/download/v1/{name}"),
        "digest": digest,
    })
}

#[test]
fn a_repository_resolves_to_its_latest_releases_smod_and_commit() {
    let digest = format!("sha256:{}", "AB".repeat(32));
    let api = recorded(vec![
        (
            "repos/acme/zoom/releases/latest",
            release(
                "v1",
                json!([asset("zoom.zip", None), asset("zoom-1.smod", Some(&digest))]),
            ),
        ),
        ("repos/acme/zoom/commits/v1", json!({ "sha": SHA })),
    ]);
    let found = resolve(&link("https://github.com/acme/zoom"), &api).unwrap();
    assert_eq!(
        (found.tag.as_deref(), found.commit.as_str()),
        (Some("v1"), SHA)
    );
    let file = &found.assets[0];
    assert_eq!(file.name, "zoom-1.smod");
    assert_eq!(file.sha256, Some("ab".repeat(32)));
}

#[test]
fn a_release_keeps_all_distinct_checked_packages_and_their_asset_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let assets = ["keys", "zoom"].map(|id| {
        let folder = dir.path().join(id);
        project(&folder, id, SHA);
        let bytes = pack(&find_mods(&folder).unwrap()[0]).unwrap();
        (format!("{id}.smod"), bytes)
    });
    let api = recorded(vec![
        (
            "repos/acme/zoom/releases/latest",
            release(
                "v1",
                json!([asset(&assets[0].0, None), asset(&assets[1].0, None),]),
            ),
        ),
        ("repos/acme/zoom/commits/v1", json!({"sha": SHA})),
    ]);
    let found = resolve(&link("https://github.com/acme/zoom"), &api).unwrap();
    let checked = fetch_with(&found, |url| {
        let (_, bytes) = assets.iter().find(|(name, _)| url.ends_with(name)).unwrap();
        Ok(bytes.clone())
    })
    .unwrap();
    for (package, (name, bytes)) in checked.iter().zip(&assets) {
        let origin = package.release_origin(&found).unwrap();
        assert_eq!(origin.asset.as_deref(), Some(name.as_str()));
        assert_eq!(origin.sha256, sha256(bytes));
    }
    let retained = checked[0].file.clone();
    assert!(retained.is_file());
    drop(checked);
    assert!(!retained.exists());
    let same_package_twice = fetch_with(&found, |_| Ok(assets[0].1.clone())).unwrap_err();
    assert!(same_package_twice.contains("more than one package for keys"));
}

#[test]
fn a_link_in_another_letter_case_still_finds_the_releases_smod() {
    let api = recorded(vec![
        (
            "repos/Acme/Zoom/releases/latest",
            release("v1", json!([asset("zoom-1.smod", None)])),
        ),
        ("repos/Acme/Zoom/commits/v1", json!({ "sha": SHA })),
    ]);
    let found = resolve(&link("https://github.com/Acme/Zoom"), &api).unwrap();
    let file = &found.assets[0];
    assert_eq!(
        file.url,
        "https://github.com/acme/zoom/releases/download/v1/zoom-1.smod"
    );
    // Only the same repository: another one's file is still dropped.
    let other = recorded(vec![
        (
            "repos/Acme/Zoom/releases/latest",
            release(
                "v1",
                json!([{"name": "zoom-1.smod",
                    "browser_download_url": "https://github.com/acme/zoomer/releases/download/v1/zoom-1.smod"}]),
            ),
        ),
        ("repos/Acme/Zoom/commits/v1", json!({ "sha": SHA })),
    ]);
    assert!(resolve(&link("https://github.com/Acme/Zoom"), &other)
        .unwrap()
        .assets
        .is_empty());
}

#[test]
fn a_release_with_no_smod_falls_back_to_its_commit() {
    let api = recorded(vec![
        (
            "repos/acme/zoom/releases/latest",
            release("v1", json!([asset("zoom.zip", None)])),
        ),
        ("repos/acme/zoom/commits/v1", json!({ "sha": SHA })),
    ]);
    let found = resolve(&link("https://github.com/acme/zoom"), &api).unwrap();
    assert!(found.assets.is_empty());
    assert_eq!(found.commit, SHA);
}

#[test]
fn an_asset_from_another_host_is_ignored() {
    let evil = json!({ "name": "x.smod", "browser_download_url": "https://evil.example/x.smod" });
    let api = recorded(vec![
        (
            "repos/acme/zoom/releases/latest",
            release("v1", json!([evil])),
        ),
        ("repos/acme/zoom/commits/v1", json!({ "sha": SHA })),
    ]);
    assert!(resolve(&link("https://github.com/acme/zoom"), &api)
        .unwrap()
        .assets
        .is_empty());
}

#[test]
fn release_transfer_refuses_foreign_urls_and_bad_checksums() {
    let mut found = Resolved {
        owner: "acme".into(),
        repo: "zoom".into(),
        tag: Some("v1".into()),
        commit: SHA.into(),
        assets: vec![Asset {
            name: "zoom.smod".into(),
            url: "https://github.com/acme/other/releases/download/v1/zoom.smod".into(),
            sha256: Some("0".repeat(64)),
        }],
    };
    assert!(fetch_with(&found, |_| panic!("foreign URL must not be fetched")).is_err());
    found.assets[0].url = "https://github.com/acme/zoom/releases/download/v1/zoom.smod".into();
    let error = fetch_with(&found, |_| Ok(b"wrong archive".to_vec())).unwrap_err();
    assert!(error.contains("does not match the checksum"), "{error}");
}

#[test]
fn no_release_resolves_the_default_branch_to_a_commit() {
    let api = recorded(vec![
        ("repos/acme/zoom", json!({ "default_branch": "dev/main" })),
        ("repos/acme/zoom/commits/dev/main", json!({ "sha": SHA })),
    ]);
    let found = resolve(&link("https://github.com/acme/zoom"), &api).unwrap();
    assert!(found.tag.is_none() && found.assets.is_empty());
    assert_eq!(found.commit, SHA);
    assert_eq!(found.label(), "acme/zoom @ 0123456");
    assert_eq!(
        found.source_url(),
        format!("https://github.com/acme/zoom/tree/{SHA}")
    );
}

#[test]
fn refs_and_short_commits_become_full_commits_and_unknown_ones_say_so() {
    let api = recorded(vec![
        ("repos/acme/zoom/commits/feature/x", json!({ "sha": SHA })),
        ("repos/acme/zoom/commits/abcdef1", json!({ "sha": SHA2 })),
        ("repos/acme/zoom/releases/tags/v9", Value::Null),
    ]);
    let commit = |url: &str| resolve(&link(url), &api).map(|r| r.commit);
    assert_eq!(
        commit("https://github.com/acme/zoom/tree/feature/x"),
        Ok(SHA.into())
    );
    assert_eq!(
        commit("https://github.com/acme/zoom/commit/abcdef1"),
        Ok(SHA2.into())
    );
    let err = commit("https://github.com/acme/zoom/tree/gone").unwrap_err();
    assert!(
        err.to_string().contains("no commit, tag or branch gone"),
        "{err}"
    );
    // A tag with no release is a commit.
    assert!(commit("https://github.com/acme/zoom/releases/tag/v9").is_err());
}

#[test]
fn a_rate_limit_is_said_plainly() {
    let limited = |_: &str| Err(RATE_LIMIT.to_string());
    let err = resolve(&link("https://github.com/acme/zoom"), &limited).unwrap_err();
    let err = err.to_string();
    assert!(
        err.contains("rate limit") && err.contains("Try again"),
        "{err}"
    );
}

fn append_archive_file(archive: &mut tar::Builder<Vec<u8>>, path: &str) {
    let mut header = tar::Header::new_gnu();
    header.set_mode(0o644);
    header.set_size(1);
    archive.append_data(&mut header, path, &b"a"[..]).unwrap();
}

#[test]
fn an_archive_refuses_duplicate_case_aliased_and_invalid_paths() {
    for path in ["source/file.txt", "source/FILE.txt", "source/NUL.txt"] {
        let scratch = tempfile::tempdir().unwrap();
        let mut archive = tar::Builder::new(Vec::new());
        append_archive_file(&mut archive, "source/file.txt");
        append_archive_file(&mut archive, path);
        let bytes = archive.into_inner().unwrap();
        assert_eq!(
            unpack_bounded(bytes.as_slice(), scratch.path(), 4096, None).unwrap_err(),
            tr("The GitHub source archive contains an invalid or duplicate path."),
            "{path}"
        );
    }
}

#[test]
fn an_archive_refuses_oversized_entries_and_streams() {
    let scratch = tempfile::tempdir().unwrap();
    let mut archive = tar::Builder::new(Vec::new());
    append_archive_file(&mut archive, "source/file.txt");
    let mut bytes = archive.into_inner().unwrap();
    let limit = bytes.len() as u64;
    let too_large = tr("The GitHub mod download is too large.");
    assert_eq!(
        unpack_bounded(bytes.as_slice(), &scratch.path().join("entry"), 512, None).unwrap_err(),
        too_large
    );
    let output = scratch.path().join("fits");
    assert_eq!(
        unpack_bounded(bytes.as_slice(), &output, limit, None).unwrap(),
        output.join("source")
    );
    // The same valid entry fits, but the stream still has bytes after tar EOF.
    bytes.push(0);
    assert_eq!(
        unpack_bounded(
            bytes.as_slice(),
            &scratch.path().join("stream"),
            limit,
            None
        )
        .unwrap_err(),
        too_large
    );
}

#[test]
fn an_archive_skips_symbolic_and_hard_links() {
    let scratch = tempfile::tempdir().unwrap();
    let mut archive = tar::Builder::new(Vec::new());
    append_archive_file(&mut archive, "source/file.txt");
    for (path, kind, target) in [
        ("source/symlink", tar::EntryType::Symlink, "file.txt"),
        ("source/hardlink", tar::EntryType::Link, "source/file.txt"),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(kind);
        header.set_size(0);
        archive.append_link(&mut header, path, target).unwrap();
    }
    let bytes = archive.into_inner().unwrap();
    let top = unpack_bounded(bytes.as_slice(), scratch.path(), 4096, None).unwrap();
    assert_eq!(fs::read(top.join("file.txt")).unwrap(), b"a");
    for path in ["symlink", "hardlink"] {
        assert_eq!(
            fs::symlink_metadata(top.join(path)).unwrap_err().kind(),
            std::io::ErrorKind::NotFound,
            "{path}"
        );
    }
}

fn tree(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gh-mods-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn project(root: &Path, id: &str, sunrise: &str) {
    write(
        &root.join("sunrise-mod.toml"),
        &format!(
            "id='{id}'\nname='{id}'\nversion='0.1.0'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{sunrise}'\nmixins='{SHA2}'\n"
        ),
    );
    write(
        &root.join("Cargo.toml"),
        "[package]\nname='cargo-name'\nversion='9.0.0'\n",
    );
    write(&root.join("src/lib.rs"), "pub fn hooks() {}");
}

#[test]
fn mod_folders_are_found_wherever_they_are_and_not_searched_inside() {
    let root = tree("find");
    project(&root.join("zoom"), "zoom", SHA);
    project(&root.join("keys"), "keys", SHA);
    project(&root.join("target/debug"), "built", SHA);
    project(&root.join(".git/x"), "hidden", SHA);
    // Stop at the mod root, not arbitrary nested crates inside its source.
    write(
        &root.join("zoom/src/nested/sunrise-mod.toml"),
        "not a package",
    );
    let ids: Vec<String> = find_mods(&root)
        .unwrap()
        .into_iter()
        .map(|b| b.manifest.id)
        .collect();
    assert_eq!(ids, ["keys", "zoom"]);
    assert!(find_mods(&tree("empty")).unwrap().is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_mod_folder_packs_into_a_bundle_the_registry_checks_accept() {
    let root = tree("pack");
    project(&root, "zoom", SHA);
    write(&root.join("src/zoom.rs"), "pub fn f() {}");
    write(&root.join("data/zoom/a.json"), "{}");
    write(&root.join("README.md"), "# Zoom");
    write(&root.join("data/zoom/table.bin"), "t");
    write(&root.join("LICENSE"), "MIT");
    write(&root.join("logo.png"), "not shipped");
    let bundle = &find_mods(&root).unwrap()[0];
    let bytes = pack(bundle).unwrap();
    assert_eq!(bytes, pack(bundle).unwrap(), "the same files pack the same");
    let packed = checked(&bytes, &tree("pack-output"), None).unwrap();
    assert_eq!(
        (packed.id.as_str(), packed.version.as_str()),
        ("zoom", "0.1.0")
    );
    assert_eq!(packed.sha256, sha256(&bytes));
    let read = sunrise_launcher_smod::index::native::Bundle::read(&packed.file).unwrap();
    for shipped in [
        "src/zoom.rs",
        "data/zoom/a.json",
        "README.md",
        "data/zoom/table.bin",
        "LICENSE",
    ] {
        assert!(read.files.contains_key(shipped), "{shipped}");
    }
    assert!(!read.files.keys().any(|n| n.contains("logo")));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_mod_with_no_exact_sunrise_commit_is_refused_at_read_boundary() {
    let root = tree("unpinned");
    project(&root, "zoom", "main");
    let err = find_mods(&root).unwrap_err();
    assert!(err.contains("exact revisions"), "{err}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_flat_smod_keeps_its_pins_and_ships_only_selected_files() {
    let root = tree("unpacked");
    project(&root, "zoom", SHA);
    write(&root.join("assets/zoom/lang/en_us.json"), "{}");
    let bundle = &find_mods(&root).unwrap()[0];
    assert_eq!(
        bundle.manifest.native.as_ref().unwrap().mixins.as_deref(),
        Some(SHA2)
    );
    let packed = checked(&pack(bundle).unwrap(), &tree("unpacked-out"), None).unwrap();
    let read = sunrise_launcher_smod::index::native::Bundle::read(&packed.file).unwrap();
    assert_eq!(read.manifest.id, "zoom");
    assert!(read.files.contains_key("assets/zoom/lang/en_us.json"));
    let native = read.manifest.native.unwrap();
    assert_eq!(
        (native.sunrise.as_str(), native.mixins.as_deref()),
        (SHA, Some(SHA2))
    );
    write(&root.join("assets/minecraft/lang/en_us.json"), "{}");
    assert!(pack(bundle).is_err());
    fs::remove_dir_all(root.join("assets/minecraft")).unwrap();
    write(&root.join("evil.exe"), "MZ");
    let again = checked(&pack(bundle).unwrap(), &tree("unpacked-out2"), None).unwrap();
    assert_eq!(again.sha256, packed.sha256);
    write(&root.join("horizon-mixin.toml"), "[mod]\nid='zoom'\n");
    assert!(find_mods(&root).is_err());
    fs::remove_dir_all(root).unwrap();
}

fn packed(id: &str) -> Packed {
    Packed {
        id: id.into(),
        version: "0.1.0".into(),
        file: PathBuf::new(),
        sha256: String::new(),
        name: id.into(),
        checked: None,
        _scratch: None,
    }
}

#[test]
fn several_mods_list_and_ask_which() {
    let two = [packed("keys"), packed("zoom")];
    let err = choose(&two, None, false).unwrap_err();
    assert!(err.contains("keys, zoom") && err.contains("--all"), "{err}");
    assert_eq!(choose(&two, Some("zoom"), false).unwrap()[0].id, "zoom");
    assert_eq!(choose(&two, None, true).unwrap().len(), 2);
    assert!(choose(&two, Some("map"), false).is_err());
    assert_eq!(choose(&two[..1], None, false).unwrap()[0].id, "keys");
}

#[test]
fn origins_follow_releases_or_the_head() {
    let source = tree("install-src");
    project(&source, "zoom", SHA);
    let bytes = pack(&find_mods(&source).unwrap()[0]).unwrap();
    let ready = checked(&bytes, &source.join("out"), None).unwrap();
    let found = Resolved {
        owner: "acme".into(),
        repo: "zoom".into(),
        tag: None,
        commit: SHA.into(),
        assets: Vec::new(),
    };
    let origin = ready.origin(&found, None);
    assert_eq!(origin.label(), "acme/zoom @ 0123456");
    assert_eq!(origin.sha256, sha256(&bytes));
    // A commit's mod follows the default branch's head.
    assert_eq!(origin.follows().target, Target::Head);
    let newer = Resolved {
        commit: SHA2.into(),
        ..found.clone()
    };
    assert!(is_update(&origin, &newer));
    assert!(!is_update(&origin, &found));
    // A release's mod follows the latest release.
    let release = Origin {
        tag: Some("v1".into()),
        asset: Some("zoom.smod".into()),
        ..origin.clone()
    };
    assert_eq!(release.label(), "acme/zoom @ v1");
    assert_eq!(release.follows().target, Target::Latest);
    let v2 = Resolved {
        tag: Some("v2".into()),
        ..found.clone()
    };
    assert!(is_update(&release, &v2));
    assert!(!is_update(
        &release,
        &Resolved {
            tag: Some("v1".into()),
            ..found
        }
    ));
    let _ = fs::remove_dir_all(source);
}
