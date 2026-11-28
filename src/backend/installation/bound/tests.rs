use super::*;

mod sdk_install;

const TAG: &str = "26.3-0.1.7";

#[test]
fn a_dev_download_binds_only_to_its_full_tag() {
    const DEV: &str = "dev-2026-11-21-0c02067";
    let requested = cfg!(windows).then_some(GameArchitecture::X64);
    let Some((asset, _)) = game_assets_for(
        std::env::consts::OS,
        native_windows_architecture(),
        std::env::consts::ARCH,
        requested.unwrap_or_default(),
    ) else {
        return;
    };
    let mut download = GameDownload {
        source: DEV.into(),
        file: asset.into(),
        size: 1,
        requested,
        time: String::new(),
        published_at: String::new(),
    };
    assert!(download.validate(DEV).is_ok());
    assert!(download.validate("dev-2026-11-21-0c02068").is_err());
    for source in ["dev", "dev-2026-11-21", "nightly-2026-11-21-0c02067"] {
        download.source = source.into();
        assert!(download.validate(DEV).is_err(), "{source}");
        assert!(download.validate(source).is_err(), "{source}");
    }
}

fn isolated_case(test: &str, run: impl FnOnce()) {
    const ACTOR: &str = "SUNRISE_TEST_ARCHIVE_REPAIR";
    if std::env::var_os(ACTOR).is_some() {
        run();
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let module = module_path!().split_once("::").unwrap().1;
    assert!(std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &format!("{module}::{test}")])
        .env(ACTOR, "1")
        .env("SUNRISE_HOME", root.path().join("game"))
        .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
        .env(
            crate::native_mods::sdk_cache::BASE_VAR,
            root.path().join("sdk-mirror")
        )
        .status()
        .unwrap()
        .success());
}

fn serve_windows_archive(
    requested: GameArchitecture,
    asset: &str,
    client: &[u8],
) -> (GameDownload, GameProof, String, std::thread::JoinHandle<()>) {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file(CLIENT, zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(client).unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    let proof = GameProof {
        archive: sunrise_launcher_release_verify::sha256_hex(&bytes),
        facts: None,
    };
    let download = GameDownload {
        source: TAG.into(),
        file: asset.into(),
        requested: Some(requested),
        size: bytes.len() as u64,
        time: String::new(),
        published_at: String::new(),
    };
    let (url, serve) = serve_archive(asset, bytes, true);
    (download, proof, url, serve)
}

fn serve_archive(
    asset: &str,
    bytes: Vec<u8>,
    content_length: bool,
) -> (String, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/{asset}", listener.local_addr().unwrap());
    let serve = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = stream.read(&mut [0; 4096]).unwrap();
        stream.write_all(b"HTTP/1.1 200 OK\r\n").unwrap();
        if content_length {
            write!(stream, "Content-Length: {}\r\n", bytes.len()).unwrap();
        }
        stream.write_all(b"Connection: close\r\n\r\n").unwrap();
        stream.write_all(&bytes).unwrap();
    });
    (url, serve)
}

#[test]
fn explicit_repair_records_arm64_request_when_the_signed_fallback_is_unchanged() {
    isolated_case(
        "explicit_repair_records_arm64_request_when_the_signed_fallback_is_unchanged",
        || {
            use GameArchitecture::{Arm64, X64};
            let client = b"the signed x64 image";
            let (download, proof, url, server) =
                serve_windows_archive(Arm64, "sunrise-windows-x64.zip", client);
            let home = version_dir(TAG);
            fs::create_dir_all(&home).unwrap();
            fs::write(home.join(COMPLETE), b"").unwrap();
            fs::write(home.join(CLIENT), client).unwrap();
            fs::write(
                home.join(GAME_BUILD_ARCHITECTURE),
                br#"{"requested":"x64","binary":"x64"}"#,
            )
            .unwrap();
            Artifact::record_archive(&home, TAG, &proof.archive).unwrap();
            assert!(
                download.is_installed(TAG, &proof),
                "exact Play can already reuse these signed bytes"
            );
            assert!(check_game_architecture(TAG, None, Arm64).is_some());

            download
                .publish(
                    TAG,
                    &proof,
                    InstallIntent::Repair,
                    |_| {},
                    |_, _| url.clone(),
                )
                .unwrap();

            assert!(download
                .installed_at(&home, TAG, &proof, InstallIntent::Repair)
                .is_ok());
            let repaired = installed_game_architecture(TAG).unwrap();
            assert_eq!((repaired.requested, repaired.binary), (Arm64, X64));
            assert!(check_game_architecture(TAG, None, Arm64).is_none());
            server.join().unwrap();
        },
    );
}

#[test]
fn same_tag_repair_preserves_server_only_preparation() {
    isolated_case("same_tag_repair_preserves_server_only_preparation", || {
        let (download, proof, url, server) = serve_windows_archive(
            GameArchitecture::X64,
            "sunrise-windows-x64.zip",
            b"repaired image",
        );
        let root = version_dir(TAG);
        let resources = resources_at(&root);
        fs::create_dir_all(&resources).unwrap();
        fs::write(root.join(COMPLETE), "").unwrap();
        fs::write(root.join(CLIENT), "old image").unwrap();
        let files = [
            (RESOURCES_MANIFEST, r#"{"profile":"server"}"#),
            (PREPARATION_STAMP, r#"{"output_version":7}"#),
            ("registries.json", "{}"),
        ];
        for (name, content) in files {
            fs::write(resources.join(name), content).unwrap();
        }

        download
            .publish(
                TAG,
                &proof,
                InstallIntent::Repair,
                |_| {},
                |_, _| url.clone(),
            )
            .unwrap();
        server.join().unwrap();

        assert_eq!(fs::read(root.join(CLIENT)).unwrap(), b"repaired image");
        for (name, content) in files {
            assert_eq!(fs::read_to_string(resources.join(name)).unwrap(), content);
        }
    });
}

#[test]
fn a_committed_nightly_survives_receipt_failure_and_retries_only_the_receipt() {
    isolated_case(
        "a_committed_nightly_survives_receipt_failure_and_retries_only_the_receipt",
        || {
            const ROLLING: &str = "nightly-2026-11-20-aaaaaaa";
            let (mut download, proof, url, server) = serve_windows_archive(
                GameArchitecture::X64,
                "sunrise-windows-x64.zip",
                b"new signed Nightly image",
            );
            download.source = NIGHTLY.into();
            let versions = versions_dir();
            let target = version_dir(ROLLING);
            let receipt = versions.join(".nightly-current");
            fs::create_dir_all(&target).unwrap();
            fs::write(target.join(COMPLETE), "").unwrap();
            fs::write(target.join(CLIENT), b"previous image").unwrap();
            // Force just the atomic receipt replacement to fail on each platform.
            fs::create_dir(&receipt).unwrap();
            fs::write(receipt.join("obstruction"), "leave this alone").unwrap();
            let result = download
                .publish(
                    ROLLING,
                    &proof,
                    InstallIntent::Repair,
                    |_| {},
                    |_, _| url.clone(),
                )
                .unwrap();
            server.join().unwrap();
            let InstallOutcome::Installed(Some(warning)) = result else {
                panic!("expected a published build with a receipt warning: {result:?}");
            };
            assert_eq!(
                warning.message(),
                "Nightly · Nov 20 is installed, but Sunrise couldn't record it as the newest Nightly."
            );
            assert_eq!(
                warning.to_string(),
                format!(
                    "{}\nCould not save the rolling receipt for {ROLLING}: {}",
                    warning.message(),
                    warning.failure.error
                ),
            );
            // The Storage action retries only the record; another failure still leaves it installed.
            assert!(warning.retry().is_err());
            assert_eq!(
                fs::read(receipt.join("obstruction")).unwrap(),
                b"leave this alone"
            );
            assert_eq!(installed(), [ROLLING]);
            assert_eq!(
                fs::read(target.join(CLIENT)).unwrap(),
                b"new signed Nightly image"
            );
            let identity = crate::profile_txn::Identity::of(&target).unwrap();
            let image = download
                .installed_at(&target, ROLLING, &proof, InstallIntent::Repair)
                .unwrap();
            assert!(!versions.join(format!(".repair-{ROLLING}")).exists());

            fs::remove_dir_all(&receipt).unwrap();
            fs::write(&receipt, "nightly-2026-11-20-bbbbbbb").unwrap();
            let retry = download
                .publish(
                    ROLLING,
                    &proof,
                    InstallIntent::Repair,
                    |_| {},
                    |_, _| panic!("receipt retry must not request any assets"),
                )
                .unwrap();
            assert!(
                matches!(retry, InstallOutcome::ReceiptOnly(None)),
                "{retry:?}"
            );
            assert_eq!(fs::read_to_string(&receipt).unwrap(), ROLLING);
            assert!(!crate::nightly::failed_install_receipt(&versions, ROLLING));
            // Exercise the Storage action with no release lookup.
            fs::remove_file(&receipt).unwrap();
            warning.retry().unwrap();
            assert!(crate::nightly::has_current_receipt(&versions, ROLLING));
            assert_eq!(crate::profile_txn::Identity::of(&target).unwrap(), identity);
            assert_eq!(
                download
                    .installed_at(&target, ROLLING, &proof, InstallIntent::Repair)
                    .unwrap(),
                image
            );

            // A later successful record wins over the stale warning.
            let later = "nightly-2026-11-20-bbbbbbb";
            fs::write(&receipt, later).unwrap();
            warning.retry().unwrap();
            assert_eq!(fs::read_to_string(&receipt).unwrap(), later);

            fs::remove_file(&receipt).unwrap();
            fs::write(target.join(CLIENT), b"a different image").unwrap();
            assert!(warning.retry().is_err());
            assert!(!crate::nightly::has_current_receipt(&versions, ROLLING));
        },
    );
}

#[test]
fn install_nightly_selects_the_current_build_cached_by_the_picker() {
    isolated_case(
        "install_nightly_selects_the_current_build_cached_by_the_picker",
        || {
            const ROLLING: &str = "nightly-2026-11-20-aaaaaaa";
            let (mut download, proof, url, server) = serve_windows_archive(
                GameArchitecture::X64,
                "sunrise-windows-x64.zip",
                b"verified Nightly image",
            );
            download.source = NIGHTLY.into();
            let stable = version_dir(TAG);
            fs::create_dir_all(&stable).unwrap();
            fs::write(stable.join(COMPLETE), []).unwrap();
            let old = version_dir("26.3-0.1.6");
            fs::create_dir_all(&old).unwrap();
            fs::write(old.join(COMPLETE), []).unwrap();
            version_storage::Download::Automatic.mark(&old).unwrap();
            let mut settings = Settings::load();
            settings.selected = Some(TAG.into());
            settings.keep_versions = Some(1);
            settings.save().unwrap();

            // The picker publishes through the same worker without changing selection or pruning.
            let cached = download
                .publish(
                    ROLLING,
                    &proof,
                    InstallIntent::Repair,
                    |_| {},
                    |_, _| url.clone(),
                )
                .unwrap();
            server.join().unwrap();
            assert!(matches!(cached, InstallOutcome::Installed(None)));
            assert_eq!(Settings::load().selected.as_deref(), Some(TAG));
            assert!(old.join(COMPLETE).is_file());
            let versions = versions_dir();
            assert!(crate::nightly::has_current_receipt(&versions, ROLLING));
            let target = version_dir(ROLLING);
            let identity = crate::profile_txn::Identity::of(&target).unwrap();
            let release = Release {
                tag: ROLLING.into(),
                name: "Nightly 2026-11-20 · aaaaaaa".into(),
                body: String::new(),
                date: "2026-11-20".into(),
                time: String::new(),
                published_at: String::new(),
                asset: Some((url, download.size)),
                prerelease: true,
                nightly: Some(NIGHTLY.into()),
            };

            // Exercise both an already-current receipt and a stale receipt without a
            // failed install. Neither should suppress the CLI's normal selection.
            for prior_receipt in [ROLLING, "nightly-2026-11-20-bbbbbbb"] {
                crate::nightly::record_current(&versions, prior_receipt).unwrap();
                settings.save().unwrap();
                let selected = install_offered_tag(
                    std::slice::from_ref(&release),
                    NIGHTLY,
                    false,
                    |offered| {
                        assert_eq!(offered.tag, ROLLING);
                        download.publish(
                            &offered.tag,
                            &proof,
                            InstallIntent::Repair,
                            |_| {},
                            |_, _| panic!("a cache hit must not request any assets"),
                        )
                    },
                )
                .unwrap();
                assert_eq!(selected, ROLLING);
                assert_eq!(Settings::load().selected.as_deref(), Some(ROLLING));
                assert!(crate::nightly::has_current_receipt(&versions, ROLLING));
                assert_eq!(crate::profile_txn::Identity::of(&target).unwrap(), identity);
            }
            assert!(
                !old.exists(),
                "a regular install prunes the old automatic build"
            );
        },
    );
}

#[test]
fn ordinary_install_cannot_claim_an_absent_pending_first_play_build() {
    use crate::{
        managed_play::ReferencePublication,
        profile_txn::{self, Admission, Bootstrap},
    };
    use GameArchitecture::{Arm64, X64};

    isolated_case(
        "ordinary_install_cannot_claim_an_absent_pending_first_play_build",
        || {
            let home = game_home();
            fs::create_dir_all(&home).unwrap();
            let (frozen, proof, url, server) =
                serve_windows_archive(X64, "sunrise-windows-x64.zip", b"frozen x64 image");
            let target = version_dir(TAG);
            let saved = std::cell::OnceCell::new();
            let (other, other_proof, other_url, other_server) =
                serve_windows_archive(Arm64, "sunrise-windows-arm64.zip", b"other arm64 image");
            let ordinary = other.publish(
                TAG,
                &other_proof,
                InstallIntent::Repair,
                |_| {
                    if saved.get().is_some() {
                        return;
                    }
                    assert!(!target.exists());
                    // Leave an interrupted First Play binding while the ordinary archive
                    // is still private. The cache target has never been published.
                    let mut admission = Admission::open(&home, None).unwrap();
                    admission
                        .bind_bootstrap(
                            || {
                                Ok(Bootstrap::new(
                                    TAG.into(),
                                    TAG.into(),
                                    [1; 16],
                                    [2; 32],
                                    serde_json::json!({"kind":"download", "archive":frozen}),
                                ))
                            },
                            |_| ReferencePublication::begin(&home),
                        )
                        .unwrap();
                    let _fence = ReferencePublication::begin(&home).unwrap();
                    let binding = admission
                        .prove_bootstrap(|_| {
                            serde_json::to_value(&proof).map_err(|e| e.to_string())
                        })
                        .unwrap();
                    saved.set(binding).unwrap();
                },
                |_, _| other_url.clone(),
            );
            assert_eq!(
                ordinary.unwrap_err(),
                crate::native_mods::cache_error(crate::native_cache::Error::ReferencedProfile)
            );
            other_server.join().unwrap();
            assert!(!target.exists());
            let references = profile_txn::references(&home).unwrap();
            assert!(references.targets.contains(TAG));
            assert!(references.pointer.is_none());

            let mut admission =
                Admission::open_fenced(&home, None, |_| ReferencePublication::begin(&home))
                    .unwrap();
            let resumed = admission
                .bind_bootstrap::<ReferencePublication>(
                    || panic!("re-resolved the interrupted binding"),
                    |_| panic!("replaced the interrupted binding"),
                )
                .unwrap();
            assert_eq!(resumed, saved.into_inner().unwrap());
            let download: GameDownload =
                serde_json::from_value(resumed.selection["archive"].clone()).unwrap();
            let proof = GameProof::saved(resumed.proof.unwrap()).unwrap();
            download
                .publish(
                    TAG,
                    &proof,
                    InstallIntent::Exact,
                    |_| {},
                    |_, _| url.clone(),
                )
                .unwrap();
            server.join().unwrap();
            assert!(download.is_installed(TAG, &proof));
            assert_eq!(fs::read(target.join(CLIENT)).unwrap(), b"frozen x64 image");
        },
    );
}

#[test]
fn both_requests_reuse_the_same_signed_fallback_but_not_a_different_binary() {
    let home = tempfile::tempdir().unwrap();
    let home = home.path();
    let download = GameDownload {
        source: "nightly".into(),
        file: "sunrise-windows-x64.zip".into(),
        size: 0,
        requested: Some(GameArchitecture::Arm64),
        time: String::new(),
        published_at: String::new(),
    };
    let proof = GameProof {
        archive: "a".repeat(64),
        facts: None,
    };
    let variant = download.variant(&proof).unwrap();
    assert!(installed_architecture_matches(
        GameArchitecture::Arm64,
        variant.architecture().unwrap()
    ));
    let archive = home.join(Artifact::ARCHIVE_RECEIPT);
    let tag = "nightly-2026-11-20-aabbccd";
    fs::write(home.join(COMPLETE), b"").unwrap();
    fs::write(home.join(CLIENT), b"client").unwrap();
    let marker = |requested, binary| {
        std::fs::write(
            home.join(GAME_BUILD_ARCHITECTURE),
            serde_json::to_vec(&InstalledGameArchitecture { requested, binary }).unwrap(),
        )
        .unwrap();
    };
    use GameArchitecture::{Arm64, X64};
    marker(Arm64, X64);
    Artifact::record_archive(home, tag, &"b".repeat(64)).unwrap();
    assert!(
        download
            .installed_at(home, tag, &proof, InstallIntent::Exact)
            .is_err(),
        "wrong archive"
    );
    fs::remove_file(&archive).unwrap();
    let direct = GameDownload {
        requested: Some(X64),
        ..download.clone()
    };
    for request in [Arm64, X64] {
        marker(request, X64);
        Artifact::record_archive(home, tag, &proof.archive).unwrap();
        assert!(download
            .installed_at(home, tag, &proof, InstallIntent::Exact)
            .is_ok());
        assert!(direct
            .installed_at(home, tag, &proof, InstallIntent::Exact)
            .is_ok());
        fs::remove_file(&archive).unwrap();
        marker(request, Arm64);
        Artifact::record_archive(home, tag, &proof.archive).unwrap();
        assert!(
            download
                .installed_at(home, tag, &proof, InstallIntent::Exact)
                .is_err(),
            "wrong binary"
        );
        assert!(
            direct
                .installed_at(home, tag, &proof, InstallIntent::Exact)
                .is_err(),
            "wrong binary"
        );
        fs::remove_file(&archive).unwrap();
    }
}
