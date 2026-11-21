use super::*;
use serde_json::{json, Value};

fn entry() -> Value {
    json!({
        "id": "trail_atlas",
        "name": "Trail Atlas",
        "summary": "A field atlas for marking trails and camps.",
        "repo": "Driftwood/trail-atlas",
        "repo_id": 123456789,
        "manifest": "mods/trail_atlas/sunrise-mod.toml",
        "track": { "branch": "feature/camps" }
    })
}

fn wire(sources: Vec<Value>) -> Vec<u8> {
    serde_json::to_vec(&json!({ "format": 1, "sources": sources })).unwrap()
}

#[test]
fn registry_shape_supports_release_and_branch_in_the_same_repository() {
    let a = entry();
    let mut b = a.clone();
    b["track"] = json!({ "release": true });
    b["id"] = json!("camps");
    b["manifest"] = json!("sunrise-mod.toml");
    let parsed = parse(&wire(vec![a, b])).unwrap();
    assert_eq!(parsed[0].repo_id(), 123456789);
    assert_eq!(parsed[0].owner(), "Driftwood");
    assert_eq!(parsed[0].id(), "camps");
    assert_eq!(parsed[0].tracker(), &Tracker::Release);
    assert_eq!(
        parsed[1].tracker(),
        &Tracker::Branch("feature/camps".into())
    );
}

#[test]
fn source_catalogue_refuses_ambiguous_or_unsafe_boundary_values() {
    for (key, value) in [
        ("repo_id", json!(0)),
        ("repo_id", json!(-1)),
        ("repo_id", json!(1.5)),
        ("repo_id", json!("123456789")),
        ("repo", json!("owner/repo.git")),
        ("repo", json!("https://github.com/owner/repo")),
        ("repo", json!("owner/repo#ref")),
        ("repo", json!("owner/repo/tree/main")),
        ("repo", json!("owner/repo%2fmain")),
        ("repo", json!("/owner/repo")),
        ("repo", json!("owner@site/repo")),
        ("id", json!("trail-atlas")),
        ("id", json!("com1")),
        ("manifest", json!("sunrise.toml")),
        ("manifest", json!("mods/../sunrise-mod.toml")),
        ("manifest", json!("mods/./sunrise-mod.toml")),
        ("manifest", json!("mods//sunrise-mod.toml")),
        ("manifest", json!("mods\\sunrise-mod.toml")),
        ("manifest", json!("mods/%2e%2e/sunrise-mod.toml")),
        ("manifest", json!("/sunrise-mod.toml")),
        ("manifest", json!("x\nsunrise-mod.toml")),
        ("name", json!(" ")),
        ("name", json!("🌻".repeat(81))),
        ("summary", json!("x".repeat(161))),
        ("summary", json!("a\nline")),
        ("track", json!({ "release": false })),
        ("track", json!({ "branch": "main", "release": true })),
        ("track", json!({ "branch": "topic//main" })),
        ("track", json!({ "branch": "topic/.lock" })),
        ("track", json!({ "branch": "https://github.com" })),
        ("track", json!({ "branch": "topic%2Fmain" })),
        ("unexpected_url", json!("https://example.test")),
    ] {
        let mut source = entry();
        source[key] = value;
        assert!(
            parse(&wire(vec![source.clone()])).is_err(),
            "{key}: {source}"
        );
    }
    assert!(parse(br#"{"format":2,"sources":[]}"#).is_err());
    assert!(parse(br#"{"format":1,"sources":[],"extra":0}"#).is_err());
    assert!(parse(br#"{"format":1,"format":1,"sources":[]}"#).is_err());
    assert!(parse(&vec![b' '; MAX_CATALOG as usize + 1]).is_err());
    assert!(parse(&[b"{\"format\":1,\"sources\":[".as_slice(), &wire(vec![])].concat()).is_err());
}

#[test]
fn source_catalogue_refuses_duplicate_id_and_repository_manifest() {
    let a = entry();
    let mut b = a.clone();
    b["repo_id"] = json!(246);
    b["repo"] = json!("elsewhere/mod");
    assert!(parse(&wire(vec![a.clone(), b])).is_err());
    let mut b = a.clone();
    b["id"] = json!("other_mod");
    b["repo"] = json!("DifferentOwner/other-project");
    assert!(parse(&wire(vec![a, b])).is_err());
}

#[test]
fn invalid_or_offline_refresh_keeps_the_etag_and_last_good_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mod-browser/sources");
    let initial = SystemTime::UNIX_EPOCH + Duration::from_secs(12);
    let later = initial + FRESH;
    let (_, good) = check_catalogue(&path, None, initial, |etag| {
        assert!(etag.is_none());
        Ok(Reply::New(wire(vec![entry()]), Some("\"one\"".into())))
    });
    assert_eq!(good.saved_at, Some(initial));
    assert!(good.error.is_none());
    let untouched = fs::read(&path).unwrap();
    let (saved, bad) = check_catalogue(&path, Saved::read(&path), later, |etag| {
        assert_eq!(etag, Some("\"one\""));
        Ok(Reply::New(
            b"not a catalogue".to_vec(),
            Some("\"two\"".into()),
        ))
    });
    assert_eq!(bad.sources.as_ref(), good.sources.as_ref());
    assert_eq!(bad.saved_at, Some(initial));
    assert_eq!(bad.checked_at, later);
    assert!(bad.error.is_some());
    let (saved, offline) = check_catalogue(&path, saved, later, |_| Err("offline".into()));
    assert_eq!(offline.saved_at, Some(initial));
    assert_eq!(offline.error.as_deref(), Some("offline"));
    assert_eq!(fs::read(&path).unwrap(), untouched);
    let (_, fresh) = check_catalogue(&path, saved, later, |etag| {
        assert_eq!(etag, Some("\"one\""));
        Ok(Reply::Unchanged)
    });
    assert!(fresh.error.is_none());
    assert_eq!(fresh.saved_at, Some(later));
    assert_eq!(Saved::read(&path).unwrap().header.saved_at, later);
    let (_, empty) = check_catalogue(&path, None, later, |_| Err("no cache".into()));
    assert!(empty.saved_at.is_none());
    assert!(empty.sources.is_empty());
}

#[test]
fn discover_and_stable_pairing_need_only_the_two_catalogues() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mod-browser/sources");
    let requests = github::test_requests();
    let (_, view) = check_catalogue(&path, None, SystemTime::now(), |_| {
        let sources = (0..1000)
            .map(|n| {
                let mut listed = entry();
                if n != 0 {
                    listed["id"] = json!(format!("atlas_{n:04}"));
                    listed["repo_id"] = json!(n);
                }
                listed
            })
            .collect();
        Ok(Reply::New(wire(sources), None))
    });
    assert_eq!(view.sources.len(), 1000);
    let checked = view
        .sources
        .iter()
        .find(|source| source.id() == "trail_atlas")
        .unwrap();
    let mut index: index::Index = serde_json::from_value(json!({
        "schema": 1,
        "projects": [{
            "id": "trail_atlas", "kind": "native-mod", "name": "Trail Atlas", "summary": "Atlas",
            "license": "MIT", "source": "https://github.com/driftwood/Trail-Atlas.git/tree/main/mods/trail_atlas",
            "versions": [{ "version": "0.1", "api": "native-1", "game": "26.3", "file": {
                "name": "trail_atlas.smod", "url": "https://example.test/a.smod", "size": 99, "sha256": "123"
            }}]
        }]
    })).unwrap();
    assert_eq!(
        also_on_stable(checked, &index, "26.3").unwrap().version,
        "0.1"
    );
    assert!(also_on_stable(checked, &index, "26.2").is_none());
    index.projects[0].source = "https://github.com/someone-else/trail-atlas".into();
    assert!(also_on_stable(checked, &index, "26.3").is_none());
    index.projects[0].source = "https://github.com/driftwood/trail-atlas".into();
    index.revoked.push(index::Revoked {
        id: "trail_atlas".into(),
        sha256: None,
        reason: "test".into(),
        date: "".into(),
    });
    assert!(also_on_stable(checked, &index, "26.3").is_none());
    assert_eq!(
        github::test_requests(),
        requests,
        "Discover must not look up GitHub repositories/refs"
    );
}
