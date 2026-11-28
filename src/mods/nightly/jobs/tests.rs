#![cfg_attr(not(debug_assertions), allow(dead_code, unused_imports))]
use super::*;
use std::fs;
use std::process::Command;
use std::time::SystemTime;
use sunrise_launcher::browser::mod_list;
use sunrise_launcher::github_mods::{self, AddedFrom, SourceOrigin};

const CLIENT_BIN: &str = if cfg!(windows) {
    "sunrise-client.exe"
} else {
    "sunrise-client"
};

fn run_in_scratch(test_name: &str, body: impl FnOnce(&Path)) {
    const CHILD: &str = "SUNRISE_NIGHTLY_OFFER_TEST";
    if std::env::var_os(CHILD).is_none() {
        let root = tempfile::tempdir().unwrap();
        let nowhere = "http://127.0.0.1:9";
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test_name])
            .env(CHILD, "1")
            .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
            .env("SUNRISE_HOME", root.path().join("home"))
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
            .env("SUNRISE_SOURCE_FIXTURES", root.path().join("github"))
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    body(&backend::game_home());
}

fn setup_prepared_offer(
    home: &Path,
    base: &str,
) -> (SourceTarget, Origin, PreparedSource, PathBuf, PathBuf) {
    let base_dir = backend::version_dir(base);
    fs::create_dir_all(&base_dir).unwrap();
    fs::write(base_dir.join(".installed"), b"").unwrap();
    let client_path = base_dir.join(CLIENT_BIN);
    fs::write(&client_path, b"base-x64-binary").unwrap();
    let pin_rev = "a".repeat(40);
    fs::write(
        base_dir.join("developer.json"),
        format!(r#"{{"revision":"{pin_rev}"}}"#),
    )
    .unwrap();
    fs::write(
        base_dir.join("research-lock.json"),
        format!(
            r#"{{"minecraft":{{"version":"{}"}}}}"#,
            backend::PINNED_GAME
        ),
    )
    .unwrap();

    fs::create_dir_all(home.join("mods")).unwrap();
    let target = SourceTarget::fixture(home, base);

    let fixtures = PathBuf::from(std::env::var_os("SUNRISE_SOURCE_FIXTURES").unwrap());
    let repo = fixtures.join("repos/Fixture/OfferMod");
    fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .current_dir(&repo)
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgSign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    git(&["init", "-q", "--initial-branch=main", "--template="]);
    let write_mod = |version: &str| {
        let manifest = format!(
            "id = 'offer_mod'\nname = 'Offer Mod'\nversion = '{version}'\napi = '0.2'\nside = 'both'\ncapabilities = ['registries']\n[native]\nsunrise = '{pin_rev}'\nmixins = '{}'\ngame = '{}'\n",
            "c".repeat(40),
            backend::PINNED_GAME,
        );
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::write(repo.join("sunrise-mod.toml"), &manifest).unwrap();
        fs::write(
            repo.join("Cargo.toml"),
            format!("[package]\nname = 'offer_mod'\nversion = '{version}'\n"),
        )
        .unwrap();
        fs::write(repo.join("src/lib.rs"), b"pub fn fixture() {}\n").unwrap();
        manifest
    };
    let manifest1 = write_mod("1.0.0");
    git(&["add", "."]);
    git(&["commit", "-qm", "v1"]);
    let sha1 = git(&["rev-parse", "HEAD"]);

    let api = fixtures.join("api/repos/Fixture");
    let heads = api.join("OfferMod/git/ref/heads");
    let commits = api.join("OfferMod/git/commits");
    fs::create_dir_all(&heads).unwrap();
    fs::create_dir_all(&commits).unwrap();
    fs::write(
        api.join("OfferMod.json"),
        r#"{"id":77,"full_name":"Fixture/OfferMod","default_branch":"main","private":false}"#,
    )
    .unwrap();
    let write_ref = |sha: &str, manifest: &str| {
        let raw = fixtures.join("raw/Fixture/OfferMod").join(sha);
        fs::create_dir_all(&raw).unwrap();
        fs::write(raw.join("sunrise-mod.toml"), manifest).unwrap();
        fs::write(
            heads.join("main.json"),
            format!(r#"{{"ref":"refs/heads/main","object":{{"type":"commit","sha":"{sha}"}}}}"#),
        )
        .unwrap();
        fs::write(
            commits.join(format!("{sha}.json")),
            format!(r#"{{"sha":"{sha}"}}"#),
        )
        .unwrap();
    };
    write_ref(&sha1, &manifest1);

    // Install v1.0.0 through the real source installer so receipt, archive and mod list match.
    let source = Source::for_origin(
        "offer_mod",
        "Fixture/OfferMod",
        77,
        "sunrise-mod.toml",
        sources::Tracker::Branch("main".into()),
    )
    .unwrap();
    let pin1 = sources::resolve(&source, true).unwrap();
    let cancel = Cancellation::default();
    let packed = sunrise::pack_sources(&[pin1], AddedFrom::Discover, &cancel)
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    sunrise::add_sources(
        target.profile_target(),
        target.identity(),
        target.base(),
        packed,
        sunrise::SourceHooks {
            install: sunrise::InstallHooks {
                progress: &|_| {},
                continue_after_failure: &|_| false,
                cancellation: Some(&cancel),
            },
            cancellation: &cancel,
            authorize: &|_, _| Ok(true),
            approve_worlds: &|_| true,
        },
    )
    .unwrap();
    let origin = github_mods::origin(home, "offer_mod").unwrap();
    assert_eq!(origin.version, "1.0.0");
    assert!(matches!(
        origin.source.as_ref().map(|s: &SourceOrigin| &s.track),
        Some(SourceTrack::Branch(b)) if b == "main"
    ));

    // Add an unpacked content mod whose internal file edits don't change native_status::fingerprint.
    let unpacked = home.join("sunrise-mods/unpacked_content");
    fs::create_dir_all(unpacked.join("assets")).unwrap();
    fs::write(
        unpacked.join("sunrise-mod.toml"),
        "id = 'unpacked_content'\nname = 'Unpacked Content'\nversion = '1.0.0'\napi = '0.2'\nside = 'both'\ncapabilities = ['registries']\n",
    )
    .unwrap();
    let unpacked_data = unpacked.join("assets/data.txt");
    fs::write(&unpacked_data, b"before").unwrap();
    mod_list::set_enabled(home, "unpacked_content", true).unwrap();

    // Advance the fixture repo to v1.1.0 and prepare a real update offer.
    let manifest2 = write_mod("1.1.0");
    git(&["add", "."]);
    git(&["commit", "-qm", "v2"]);
    let sha2 = git(&["rev-parse", "HEAD"]);
    write_ref(&sha2, &manifest2);

    let pin2 = sources::resolve(&source, true).unwrap();
    let prepared = sunrise::prepare_source(
        target.profile_target(),
        target.identity(),
        target.base(),
        SourceRequest::Update {
            pin: pin2,
            predecessor: Box::new(origin.clone()),
        },
        &cancel,
    )
    .unwrap();
    (target, origin, prepared, unpacked_data, client_path)
}

#[cfg(debug_assertions)] // source fixtures are off in release builds
#[test]
fn a_check_keeps_only_findings_proved_against_the_same_mods() {
    run_in_scratch(
        "mods::nightly::jobs::tests::a_check_keeps_only_findings_proved_against_the_same_mods",
        |home| {
            const BASE: &str = "developer-local";
            let (target, origin, prepared, unpacked_data, client_path) =
                setup_prepared_offer(home, BASE);
            let proof1 =
                SourceProof::capture(target.profile_target(), target.identity(), BASE).unwrap();
            assert_eq!(prepared.proof(), &proof1);
            let fingerprint1 = native_status::fingerprint(home, Some(BASE));

            let scope = || OpenScope {
                profile: Some("p".into()),
                home: home.to_path_buf(),
                target: None,
                game: BASE.into(),
            };
            let issue = |why: &str| SourceError::Unproved(why.into());
            let make_checked = |only: Option<&str>,
                                inputs: &str,
                                proof: Option<SourceProof>,
                                online: bool,
                                issues: &[(&str, &str)]| {
                CheckedUpdates {
                    scope: scope(),
                    inputs: inputs.into(),
                    proof,
                    only: only.map(str::to_owned),
                    target: online.then(|| target.clone()),
                    candidates: BTreeMap::new(),
                    issues: issues
                        .iter()
                        .map(|(id, why)| ((*id).to_owned(), issue(why)))
                        .collect(),
                    notice: (!online)
                        .then(|| tr("No source updates were checked while offline.").into()),
                }
            };
            let with_offer = |prepared: PreparedSource| {
                let mut c = make_checked(
                    None,
                    &fingerprint1,
                    Some(proof1.clone()),
                    true,
                    &[("a", "old a"), ("b", "b")],
                );
                c.candidates
                    .insert(origin.id.clone(), (origin.clone(), prepared));
                c
            };

            // Re-prepare helper since PreparedSource isn't Clone.
            let re_prepare = || {
                let source = source_for_update(&origin).unwrap().unwrap();
                let pin = sources::resolve(&source, true).unwrap();
                sunrise::prepare_source(
                    target.profile_target(),
                    target.identity(),
                    BASE,
                    SourceRequest::Update {
                        pin,
                        predecessor: Box::new(origin.clone()),
                    },
                    &Cancellation::default(),
                )
                .unwrap()
            };

            // 1. An unchanged single-project check keeps the other mod's real offer and issue.
            let merged = merge_checked(
                Some(with_offer(prepared)),
                make_checked(Some("a"), &fingerprint1, Some(proof1.clone()), true, &[]),
            );
            assert!(merged.candidates.contains_key(&origin.id));
            assert_eq!(merged.issues.get("b"), Some(&issue("b")));
            assert_eq!(merged.issues.get("a"), None);

            // 2. Offline, nothing was checked: the real offer survives both offline paths.
            let mut carried = merged;
            for only in [None, Some(origin.id.as_str())] {
                carried = merge_checked(
                    Some(carried),
                    make_checked(only, &fingerprint1, Some(proof1.clone()), false, &[]),
                );
                assert!(carried.candidates.contains_key(&origin.id));
                assert_eq!(
                    carried.notice.as_deref(),
                    Some(tr("No source updates were checked while offline."))
                );
            }

            // 3. A successful check for that project that finds no update drops the offer.
            let merged = merge_checked(
                Some(carried),
                make_checked(
                    Some(&origin.id),
                    &fingerprint1,
                    Some(proof1.clone()),
                    true,
                    &[],
                ),
            );
            assert!(!merged.candidates.contains_key(&origin.id));

            // 4. A profile-wide online check that finds no update drops the offer.
            let merged = merge_checked(
                Some(with_offer(re_prepare())),
                make_checked(None, &fingerprint1, Some(proof1.clone()), true, &[]),
            );
            assert!(merged.candidates.is_empty());
            assert!(merged.issues.is_empty());

            // 5. Editing a file inside an unpacked content mod doesn't change the Installed
            //    fingerprint, but changes the installer's proof and drops the stale offer.
            let before_edit = with_offer(re_prepare());
            fs::write(&unpacked_data, b"edited-content").unwrap();
            assert_eq!(native_status::fingerprint(home, Some(BASE)), fingerprint1);
            let proof_after_edit =
                SourceProof::capture(target.profile_target(), target.identity(), BASE).unwrap();
            assert_ne!(proof_after_edit, proof1);
            let merged = merge_checked(
                Some(before_edit),
                make_checked(Some("a"), &fingerprint1, Some(proof_after_edit), true, &[]),
            );
            assert!(merged.candidates.is_empty());
            assert!(merged.issues.is_empty());

            // 6. Reinstalling the same game tag for another architecture doesn't change the
            //    Installed fingerprint, but changes the cached base build and drops the offer.
            fs::write(&unpacked_data, b"before").unwrap();
            let before_reinstall = with_offer(re_prepare());
            fs::write(&client_path, b"base-arm64-binary").unwrap();
            assert_eq!(native_status::fingerprint(home, Some(BASE)), fingerprint1);
            let proof_after_reinstall =
                SourceProof::capture(target.profile_target(), target.identity(), BASE).unwrap();
            assert_ne!(proof_after_reinstall, proof1);
            let merged = merge_checked(
                Some(before_reinstall),
                make_checked(
                    Some("a"),
                    &fingerprint1,
                    Some(proof_after_reinstall),
                    true,
                    &[],
                ),
            );
            assert!(merged.candidates.is_empty());
            assert!(merged.issues.is_empty());
        },
    );
}

#[cfg(debug_assertions)] // source fixtures are off in release builds
#[test]
fn last_known_source_offer_stays_visible_while_offline_but_requires_fresh_catalogue_to_apply() {
    run_in_scratch(
        "mods::nightly::jobs::tests::last_known_source_offer_stays_visible_while_offline_but_requires_fresh_catalogue_to_apply",
        |home| {
            const BASE: &str = "developer-local";
            let (target, origin, prepared, _, _) = setup_prepared_offer(home, BASE);
            let proof =
                SourceProof::capture(target.profile_target(), target.identity(), BASE).unwrap();
            let key = native_status::fingerprint(home, Some(BASE));

            let mut app = gpui::TestAppContext::single();
            let (shell, cx) = app.add_window_view(crate::window_root::Shell::new_for_test);
            let launcher = shell.read_with(cx, |shell, _| shell.launcher());
            launcher.update(cx, |launcher, _| {
                let scope = launcher.mods_page_scope();
                let local = model::Local {
                    id: origin.id.clone(),
                    version: origin.version.clone(),
                    name: "Offer Mod".into(),
                    file: "mods/offer_mod.smod".into(),
                    github: Some(origin.clone()),
                    native: false,
                    served_by: None,
                    unreadable: false,
                };
                let mut rows = InstalledRows::default();
                rows.local.push(local);
                launcher.mods_state.snapshot = Some(Snapshot {
                    home: home.to_path_buf(),
                    key: key.clone(),
                    rows,
                    reading: false,
                    error: None,
                    backup_notice: None,
                });
                let mut candidates = BTreeMap::new();
                candidates.insert(origin.id.clone(), (origin.clone(), prepared));
                launcher.mods_state.nightly.work.updates = Some(CheckedUpdates {
                    scope,
                    inputs: key.clone(),
                    proof: Some(proof),
                    only: None,
                    target: Some(target.clone()),
                    candidates,
                    issues: BTreeMap::new(),
                    notice: Some(
                        tr("No source updates were checked while offline.").into(),
                    ),
                });
                // Catalogue failed / offline: nightly_catalogue_current() is false.
                launcher.mods_state.nightly.catalogue = Some(Catalogue {
                    sources: Vec::new().into(),
                    collections: Vec::new().into(),
                    saved_at: None,
                    checked_at: SystemTime::now(),
                    error: Some("offline".into()),
                });
                assert!(!launcher.nightly_catalogue_current());

                // Last-known offer is still visible beside the offline notice.
                assert_eq!(launcher.source_update_version(&origin), Some("1.1.0"));
                assert_eq!(
                    launcher.source_update_notice(None),
                    Some(tr("No source updates were checked while offline."))
                );
                // Applying still requires a fresh catalogue.
                assert!(launcher.take_source_updates(Some(&origin.id)).is_none());
                assert!(launcher.take_source_updates(None).is_none());

                // Once the catalogue is current again, applying takes the prepared offer.
                launcher.mods_state.nightly.catalogue = Some(Catalogue {
                    sources: Vec::new().into(),
                    collections: Vec::new().into(),
                    saved_at: Some(SystemTime::now()),
                    checked_at: SystemTime::now(),
                    error: None,
                });
                assert!(launcher.nightly_catalogue_current());
                let taken = launcher.take_source_updates(Some(&origin.id));
                assert!(taken.is_some());
                assert_eq!(taken.unwrap().1.len(), 1);
            });
        },
    );
}

#[test]
fn project_check_uses_only_its_exact_current_receipt() {
    let (_, first) = super::super::tests::receipt_fixture();
    let mut second = first.clone();
    second.id = "other".into();
    second.github.as_mut().unwrap().id = second.id.clone();
    let mut pinned = first.clone();
    pinned
        .github
        .as_mut()
        .unwrap()
        .source
        .as_mut()
        .unwrap()
        .track = SourceTrack::Pinned;
    let only = first.github.clone().unwrap();
    let other = second.github.clone().unwrap();
    let locals = [first, second, pinned];
    assert_eq!(tracked_origins(&locals, None), [only.clone(), other]);
    assert_eq!(
        tracked_origins(&locals, Some(&only)),
        std::slice::from_ref(&only)
    );
    let stale = Origin {
        commit: "9".repeat(40),
        ..only
    };
    assert!(tracked_origins(&locals, Some(&stale)).is_empty());
}

#[test]
fn manual_from_source_mods_are_excluded_from_tracked_source_updates() {
    use sunrise_launcher::github_mods::AddedFrom;
    let (_, mut manual_github) = super::super::tests::receipt_fixture();
    manual_github.github.as_mut().unwrap().added_from = Some(AddedFrom::Github);

    let mut manual_folder = manual_github.clone();
    manual_folder.id = "folder_mod".into();
    manual_folder.github = Some(Origin::local_folder(
        "folder_mod".into(),
        "0.4.0".into(),
        "/tmp/camp-lanterns".into(),
        "0".repeat(64),
    ));

    let mut discover_source = manual_github.clone();
    discover_source.id = "discover_mod".into();
    discover_source.github.as_mut().unwrap().id = "discover_mod".into();
    discover_source.github.as_mut().unwrap().added_from = Some(AddedFrom::Discover);

    let locals = [manual_github, manual_folder, discover_source.clone()];
    let tracked = tracked_origins(&locals, None);
    assert_eq!(tracked, [discover_source.github.unwrap()]);
}

#[test]
fn legacy_pasted_branch_and_release_receipts_stay_out_of_update_all() {
    use sunrise_launcher::github_mods::AddedFrom;
    let (_, discover_source) = super::super::tests::receipt_fixture();

    let mut legacy_branch = discover_source.clone();
    legacy_branch.id = "legacy_branch".into();
    legacy_branch.github.as_mut().unwrap().id = "legacy_branch".into();
    legacy_branch.github.as_mut().unwrap().added_from = None;

    let mut legacy_release = discover_source.clone();
    legacy_release.id = "legacy_release".into();
    legacy_release.github.as_mut().unwrap().id = "legacy_release".into();
    legacy_release.github.as_mut().unwrap().added_from = None;
    legacy_release
        .github
        .as_mut()
        .unwrap()
        .source
        .as_mut()
        .unwrap()
        .track = SourceTrack::Release;

    let mut collection_source = discover_source.clone();
    collection_source.id = "collection_mod".into();
    collection_source.github.as_mut().unwrap().id = "collection_mod".into();
    collection_source.github.as_mut().unwrap().added_from = Some(AddedFrom::Collection {
        id: "starter".into(),
        name: "Starter".into(),
    });

    let locals = [
        legacy_branch,
        legacy_release,
        discover_source.clone(),
        collection_source.clone(),
    ];
    let tracked = tracked_origins(&locals, None);
    assert_eq!(
        tracked,
        [
            discover_source.github.unwrap(),
            collection_source.github.unwrap(),
        ]
    );
}
