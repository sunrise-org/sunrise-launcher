use super::*;

const REPO: &str = "Driftwood/trail-atlas";
const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const MANIFEST: &str = "mods/trail_atlas/sunrise-mod.toml";

struct Sandbox {
    _temp: tempfile::TempDir,
    github: Github,
}

impl Sandbox {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let cache = temp.path().join("cache");
        let github = Github {
            cache,
            fixture: Some(temp.path().join("fixture")),
        };
        let sandbox = Self {
            _temp: temp,
            github,
        };
        sandbox.put(
            &format!("api/repos/{REPO}.json"),
            br#"{"id":42,"full_name":"Driftwood/trail-atlas","private":false}"#,
        );
        sandbox.put(
            &format!("api/repos/{REPO}/git/ref/heads/topic/camps.json"),
            format!(
                r#"{{"ref":"refs/heads/topic/camps","object":{{"sha":"{SHA}","type":"commit"}}}}"#
            ),
        );
        sandbox.put(&format!("raw/{REPO}/{SHA}/{MANIFEST}"), format!("id='trail_atlas'\nname='Trail Atlas'\nversion='0.1.0'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{SHA}'\nmixins='{SHA}'\n"));
        sandbox
    }

    fn put(&self, path: &str, bytes: impl AsRef<[u8]>) {
        let path = self.github.fixture.as_ref().unwrap().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn source(repo_id: u64) -> Source {
        Source::for_origin(
            "trail_atlas",
            REPO,
            repo_id,
            MANIFEST,
            Tracker::Branch("topic/camps".into()),
        )
        .unwrap()
    }
}

#[test]
fn a_branch_must_match_public_identity_and_exact_heads_ref() {
    let s = Sandbox::new();
    let bad = resolve_checked(&s.github, &Sandbox::source(43), None);
    assert_eq!(bad.unwrap_err(), Failure::RepositoryUnavailable);
    s.put(
        &format!("api/repos/{REPO}/git/ref/heads/topic/camps.json"),
        format!(r#"{{"ref":"refs/tags/topic/camps","object":{{"sha":"{SHA}","type":"commit"}}}}"#),
    );
    let bad = resolve_checked(&s.github, &Sandbox::source(42), None);
    assert_eq!(bad.unwrap_err(), Failure::RefUnavailable);
    s.put(
        &format!("api/repos/{REPO}.json"),
        br#"{"id":42,"full_name":"Driftwood/old-name","private":false}"#,
    );
    let bad = resolve_checked(&s.github, &Sandbox::source(42), Some(Instant::now()));
    assert_eq!(bad.unwrap_err(), Failure::RepositoryUnavailable);
}

#[test]
fn a_release_resolves_its_tag_object_to_source_not_to_its_assets() {
    const TAG_OBJECT: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let s = Sandbox::new();
    s.put(&format!("api/repos/{REPO}/releases/latest.json"), br#"{"tag_name":"v0.1.0","published_at":"2026-11-21T04:00:00Z","draft":false,"prerelease":false,"assets":[{"name":"trail_atlas.smod"}]}"#);
    s.put(
        &format!("api/repos/{REPO}/git/ref/tags/v0.1.0.json"),
        format!(r#"{{"ref":"refs/tags/v0.1.0","object":{{"sha":"{TAG_OBJECT}","type":"tag"}}}}"#),
    );
    s.put(
        &format!("api/repos/{REPO}/git/tags/{TAG_OBJECT}.json"),
        format!(r#"{{"object":{{"sha":"{SHA}","type":"commit"}}}}"#),
    );
    let listing = Source::for_origin("trail_atlas", REPO, 42, MANIFEST, Tracker::Release).unwrap();
    let commit = resolve_checked(&s.github, &listing, None).unwrap();
    assert_eq!(commit.sha(), SHA);
    assert_eq!(commit.release_tag(), Some("v0.1.0"));
    assert_eq!(commit.published_at(), Some("2026-11-21T04:00:00Z"));
    assert_eq!(commit.source().manifest(), MANIFEST);
}

#[test]
fn a_missing_or_mismatched_manifest_disables_build_without_hiding_the_pinned_readme() {
    let s = Sandbox::new();
    let path = format!("raw/{REPO}/{SHA}/{MANIFEST}");
    s.put(&path, b"id='mismatch'\n");
    s.put(
        &format!("api/repos/{REPO}/readme/{SHA}.json"),
        br#"{"path":"docs/README.md","size":110,"type":"file"}"#,
    );
    s.put(
        &format!("raw/{REPO}/{SHA}/docs/README.md"),
        "# Pinned\n\n![Atlas](media/atlas.png)",
    );
    let start = test_requests();
    let details = inspect(&s.github, &Sandbox::source(42), None, true);
    assert_eq!(details.error, Some(Failure::NoManifest));
    assert_eq!(details.commit.unwrap().sha(), SHA);
    assert_eq!(details.readme.unwrap().images().len(), 1);
    assert_eq!(
        test_requests() - start,
        5,
        "only repo, ref, manifest and README metadata/content; no image"
    );
}

#[test]
fn repository_and_ref_collapse_across_mods_and_cached_readme_works_offline() {
    let s = Sandbox::new();
    s.put(
        &format!("api/repos/{REPO}/readme/{SHA}.json"),
        br#"{"path":"README.md","size":24,"type":"file"}"#,
    );
    s.put(
        &format!("raw/{REPO}/{SHA}/README.md"),
        "# Trail Atlas\n\nOld notes.",
    );
    let start = test_requests();
    let epoch = Some(Instant::now());
    let details = inspect(&s.github, &Sandbox::source(42), epoch, true);
    assert!(details.error.is_none(), "{:?}", details.error);
    let other = Source::for_origin(
        "other",
        REPO,
        42,
        "other/sunrise-mod.toml",
        Tracker::Branch("topic/camps".into()),
    )
    .unwrap();
    let bad = resolve_checked(&s.github, &other, epoch).unwrap_err();
    assert_eq!(bad, Failure::NoManifest);
    assert_eq!(
        test_requests() - start,
        6,
        "the second mod only checks its manifest"
    );
    s.put(&format!("api/repos/{REPO}.status"), "offline\n");
    let stale = inspect(&s.github, &Sandbox::source(42), Some(Instant::now()), true);
    assert!(matches!(stale.error, Some(Failure::Offline(_))));
    assert_eq!(
        stale.readme.unwrap().markdown(),
        "# Trail Atlas\n\nOld notes."
    );
    assert_eq!(
        test_requests() - start,
        7,
        "only one attempt while recovering the saved README"
    );
}

#[test]
fn a_rate_limit_names_reset_time_and_pauses_the_next_repository_lookup() {
    let s = Sandbox::new();
    s.put(
        &format!("api/repos/{REPO}.status"),
        "429\nRetry-After: 500\n",
    );
    let start = test_requests();
    let before = SystemTime::now();
    let failure = resolve_checked(&s.github, &Sandbox::source(42), None).unwrap_err();
    let Failure::RateLimited {
        retry_at: Some(retry),
    } = failure
    else {
        panic!("{failure:?}");
    };
    assert!(retry >= before + Duration::from_secs(500));
    let next = Sandbox::source(48);
    assert_eq!(
        resolve_checked(&s.github, &next, Some(Instant::now())).unwrap_err(),
        failure
    );
    assert_eq!(test_requests() - start, 1);
}

#[test]
fn an_archive_rate_limit_pauses_archive_and_metadata_requests_until_reset() {
    const NEXT_REPO: &str = "Driftwood/next-mod";
    let wait = Duration::from_secs(500);
    let reset = (SystemTime::now() + wait)
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    for (status, exact_reset) in [
        ("429\nRetry-After: 500\n".to_owned(), None),
        (
            format!("403\nX-RateLimit-Remaining: 0\nX-RateLimit-Reset: {reset}\n"),
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(reset)),
        ),
    ] {
        let s = Sandbox::new();
        s.put(&format!("codeload/{REPO}/{SHA}.status"), status);
        let start = test_requests();
        let before = SystemTime::now();
        let failure = s.github.archive(REPO, SHA).err().unwrap();
        let Failure::RateLimited {
            retry_at: Some(retry),
        } = failure
        else {
            panic!("{failure:?}");
        };
        if let Some(exact_reset) = exact_reset {
            assert_eq!(retry, exact_reset);
        } else {
            assert!((before + wait..=SystemTime::now() + wait).contains(&retry));
        }
        assert_eq!(
            s.github.archive(NEXT_REPO, SHA).err(),
            Some(failure.clone())
        );
        assert_eq!(
            resolve_checked(&s.github, &Sandbox::source(42), None).unwrap_err(),
            failure
        );
        assert_eq!(test_requests() - start, 1);

        // Move the readiness check to the reset instant without making the fixture sleep.
        s.github.ready(&mut STATE.lock().unwrap(), retry).unwrap();
        assert!(resolve_checked(&s.github, &Sandbox::source(42), None).is_ok());
        assert_eq!(test_requests() - start, 4, "repo, ref and manifest resume");
    }
}

#[test]
fn active_cooldown_preserves_cached_details_but_does_not_authorize_builds() {
    let s = Sandbox::new();
    s.put(
        &format!("api/repos/{REPO}/readme/{SHA}.json"),
        br#"{"path":"README.md","size":24,"type":"file"}"#,
    );
    s.put(&format!("raw/{REPO}/{SHA}/README.md"), "# Trail Atlas");
    let source = Sandbox::source(42);
    assert!(inspect(&s.github, &source, None, true).error.is_none());
    s.put(
        &format!("api/repos/{REPO}.status"),
        "429\nRetry-After: 500\n",
    );
    let limited = resolve_checked(&s.github, &Sandbox::source(43), None).unwrap_err();
    let start = test_requests();
    let details = inspect(&s.github, &source, None, true);
    assert_eq!(details.error, Some(limited.clone()));
    assert_eq!(details.commit.unwrap().sha(), SHA);
    assert_eq!(details.readme.unwrap().markdown(), "# Trail Atlas");
    assert_eq!(
        resolve_checked(&s.github, &source, None).unwrap_err(),
        limited
    );
    assert_eq!(test_requests(), start);
}

#[test]
fn a_pasted_branch_preserves_canonical_identity_and_cannot_retarget_or_guess() {
    use crate::github_mods::{parse, Resolved, SourceTrack};
    let s = Sandbox::new();
    s.put("api/repos/driftwood/trail-atlas.json", br#"{"id":42,"full_name":"Driftwood/trail-atlas","private":false,"default_branch":"topic/camps"}"#);
    let link = parse("https://github.com/driftwood/trail-atlas/tree/topic/camps").unwrap();
    let mut found = Resolved {
        owner: link.owner.clone(),
        repo: link.repo.clone(),
        tag: None,
        commit: SHA.into(),
        assets: vec![],
    };
    let mut state = State::default();
    let pin = pasted::prove_pasted(&s.github, &mut state, &link, &found)
        .ok()
        .unwrap();
    assert_eq!(pin.repo, REPO);
    assert_eq!(pin.repo_id, 42);
    assert!(matches!(pin.track, SourceTrack::Branch(ref name) if name == "topic/camps"));
    assert!(pin.published_at.is_none());
    let default = parse("https://github.com/driftwood/trail-atlas").unwrap();
    assert!(
        matches!(pasted::prove_pasted(&s.github, &mut state, &default, &found).ok().unwrap().track, SourceTrack::Branch(ref name) if name == "topic/camps")
    );
    s.put(
        &format!("api/repos/{REPO}/git/ref/tags/topic/camps.json"),
        format!(r#"{{"ref":"refs/tags/topic/camps","object":{{"sha":"{SHA}","type":"commit"}}}}"#),
    );
    assert_eq!(
        pasted::prove_pasted(&s.github, &mut state, &link, &found).err(),
        Some(Failure::RefUnavailable)
    );
    let explicit =
        parse("https://github.com/driftwood/trail-atlas/tree/refs/heads/topic/camps").unwrap();
    assert!(pasted::prove_pasted(&s.github, &mut state, &explicit, &found).is_ok());
    found.commit = "b".repeat(40);
    assert_eq!(
        pasted::prove_pasted(&s.github, &mut state, &explicit, &found).err(),
        Some(Failure::RefUnavailable)
    );
}

#[test]
fn a_pasted_release_uses_its_peeled_tag_and_non_release_links_stay_pinned() {
    use crate::github_mods::{parse, Resolved, SourceTrack};
    let s = Sandbox::new();
    let mut found = Resolved {
        owner: "Driftwood".into(),
        repo: "trail-atlas".into(),
        tag: Some("v0.1.0".into()),
        commit: SHA.into(),
        assets: vec![],
    };
    let link = parse(&format!("https://github.com/{REPO}/releases/tag/v0.1.0")).unwrap();
    let tag = "b".repeat(40);
    s.put(&format!("api/repos/{REPO}/releases/tags/v0.1.0.json"), br#"{"tag_name":"v0.1.0","published_at":"2026-11-21T04:00:00Z","draft":false,"prerelease":false}"#);
    s.put(
        &format!("api/repos/{REPO}/git/ref/tags/v0.1.0.json"),
        format!(r#"{{"ref":"refs/tags/v0.1.0","object":{{"sha":"{tag}","type":"tag"}}}}"#),
    );
    s.put(
        &format!("api/repos/{REPO}/git/tags/{tag}.json"),
        format!(r#"{{"object":{{"sha":"{SHA}","type":"commit"}}}}"#),
    );
    let mut state = State::default();
    let pin = pasted::prove_pasted(&s.github, &mut state, &link, &found)
        .ok()
        .unwrap();
    assert!(matches!(pin.track, SourceTrack::Release));
    assert_eq!(pin.tag.as_deref(), Some("v0.1.0"));
    assert_eq!(pin.published_at.as_deref(), Some("2026-11-21T04:00:00Z"));
    found.tag = None;
    s.put(
        &format!("api/repos/{REPO}/releases/tags/v0.1.0.status"),
        "404\n",
    );
    let pin = pasted::prove_pasted(&s.github, &mut state, &link, &found)
        .ok()
        .unwrap();
    assert!(matches!(pin.track, SourceTrack::Pinned));
    assert!(pin.tag.is_none());
    let explicit = parse(&format!("https://github.com/{REPO}/commit/{}", &SHA[..7])).unwrap();
    s.put(
        &format!("api/repos/{REPO}/git/commits/{SHA}.json"),
        format!(r#"{{"sha":"{SHA}"}}"#),
    );
    let pin = pasted::prove_pasted(&s.github, &mut state, &explicit, &found)
        .ok()
        .unwrap();
    assert!(matches!(pin.track, SourceTrack::Pinned));
}

#[test]
fn compare_requires_ahead_and_the_exact_installed_commit_as_merge_base() {
    let s = Sandbox::new();
    let new = "b".repeat(40);
    let path = format!("api/repos/{REPO}/compare/{SHA}...{new}.json");
    let mut state = State::default();
    for (status, base, expected) in [
        ("ahead", SHA, true),
        ("ahead", new.as_str(), false),
        ("behind", SHA, false),
        ("diverged", SHA, false),
    ] {
        s.put(
            &path,
            format!(r#"{{"status":"{status}","merge_base_commit":{{"sha":"{base}"}}}}"#),
        );
        assert_eq!(
            pasted::compare(&s.github, &mut state, REPO, SHA, &new),
            Ok(expected)
        );
    }
    let start = test_requests();
    assert_eq!(
        pasted::compare(&s.github, &mut state, REPO, SHA, SHA),
        Ok(false)
    );
    assert_eq!(
        pasted::compare(&s.github, &mut state, REPO, "main", &new),
        Err(Failure::RefUnavailable)
    );
    assert_eq!(test_requests(), start);
}
