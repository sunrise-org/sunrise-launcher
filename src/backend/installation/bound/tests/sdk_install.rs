use super::*;
use crate::native_mods::sdk_cache::{
    tests::{index_of, publish, COMMIT},
    BASE_VAR,
};
use sunrise_launcher_release_verify::{sha256_hex, SigningKey};
use sunrise_launcher_sdk::{Asset, Kind, Side};

const DEV: &str = "dev-2026-11-13-0123456";
const FACTS: &[u8] = br#"{"contract":1,"build":"dev-2026-11-13-0123456","commit":"0123456789abcdef0123456789abcdef01234567"}"#;
#[cfg(windows)]
const SLICES: [&[u8]; 2] = [
    include_bytes!("sdk-fixture/windows-runtime.tar.xz"),
    include_bytes!("sdk-fixture/windows-programs.tar.xz"),
];
#[cfg(target_os = "macos")]
const SLICES: [&[u8]; 2] = [
    include_bytes!("sdk-fixture/macos-runtime.tar.xz"),
    include_bytes!("sdk-fixture/unix-programs.tar.xz"),
];
#[cfg(not(any(windows, target_os = "macos")))]
const SLICES: [&[u8]; 2] = [
    include_bytes!("sdk-fixture/linux-runtime.tar.xz"),
    include_bytes!("sdk-fixture/unix-programs.tar.xz"),
];

fn row(tag: &str, assets: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "tag_name": tag,
        "name": "Dev 2026-11-13 · 0123456",
        "prerelease": tag == DEV,
        "assets": assets.iter().map(|name| serde_json::json!({"name": name, "size": 1})).collect::<Vec<_>>(),
    })
}

#[test]
fn a_newer_sdk_without_both_client_slices_does_not_win_update_or_first_play() {
    use crate::instances::Channel::Stable;
    const OLDER: &str = "26.3-0.1.7";
    const NEWER: &str = "26.3-0.1.8";
    let settings = Settings::default();
    let Some(triple) = sdk_target(selected_game_architecture(&settings)) else {
        return;
    };
    let mirror = tempfile::tempdir().unwrap();
    let base = Base::Folder(mirror.path().to_owned());
    let (key, _) = SigningKey::generate().unwrap();
    let keys = [key.public()];
    let names = [Kind::Runtime, Kind::Programs].map(|kind| Asset::name(kind, triple, Side::Client));
    let slices = [
        (names[0].as_str(), SLICES[0]),
        (names[1].as_str(), SLICES[1]),
    ];
    let list = [row(OLDER, &[INDEX]), row(NEWER, &[INDEX])];
    let full = index_of(&slices, "1.0.0").to_json();
    let publish_index = |tag, index: &str, with_facts: bool| {
        let extra: &[(&str, &[u8])] = if with_facts {
            &[(RELEASE_FILE, FACTS)]
        } else {
            &[]
        };
        publish(mirror.path(), tag, tag, &key, index.as_bytes(), extra);
    };
    publish_index(OLDER, &full, true);
    publish_index(NEWER, &index_of(&slices[..1], "1.0.0").to_json(), true);
    let releases = game_releases_from(&list, &settings, &base, &keys);
    assert_eq!(releases[0].tag, NEWER);
    assert!(releases[0].asset.is_none());
    assert_eq!(sunrise_update(&releases, &[], Stable).unwrap().tag, OLDER);
    let target = Stable.play_target(&releases, &[], None, None).unwrap();
    let first = crate::managed_play::FirstBuild::download(
        release_selection::by_tag(&releases, target).unwrap(),
    )
    .unwrap();
    assert_eq!(first.base_tag(), OLDER);

    // A parseable replacement with both slices does not become eligible until it is signed,
    // and even a signed sdk.json with both slices is refused without signed sunrise-release.json.
    fs::write(mirror.path().join(NEWER).join(INDEX), &full).unwrap();
    let releases = game_releases_from(&list, &settings, &base, &keys);
    assert_eq!(sunrise_update(&releases, &[], Stable).unwrap().tag, OLDER);
    publish_index(NEWER, &full, false);
    let releases = game_releases_from(&list, &settings, &base, &keys);
    assert_eq!(
        sunrise_update(&releases, &[], Stable).unwrap().tag,
        OLDER,
        "missing signed sunrise-release.json must not hide older playable build"
    );
    publish_index(NEWER, &full, true);
    let releases = game_releases_from(&list, &settings, &base, &keys);
    assert_eq!(sunrise_update(&releases, &[], Stable).unwrap().tag, NEWER);
}

#[test]
fn windows_arm64_sdk_target_allows_x64_and_records_requested_vs_actual_architecture() {
    use crate::rust_toolchain::WINDOWS_X64_TARGET;
    // Explicit x64 selection on Windows ARM64.
    assert_eq!(
        sdk_target_for(
            "windows",
            Some("aarch64"),
            "aarch64",
            Some(GameArchitecture::X64)
        ),
        Some(WINDOWS_X64_TARGET)
    );
    // ARM64 selection on Windows ARM64 falls back to x64 under emulation, matching player archives.
    assert_eq!(
        sdk_target_for(
            "windows",
            Some("aarch64"),
            "aarch64",
            Some(GameArchitecture::Arm64),
        ),
        Some(WINDOWS_X64_TARGET)
    );
    // Native Windows x64 with explicit x64.
    assert_eq!(
        sdk_target_for(
            "windows",
            Some("x86_64"),
            "x86_64",
            Some(GameArchitecture::X64)
        ),
        Some(WINDOWS_X64_TARGET)
    );
    // Requested and actual architectures are recorded separately on SDK-only GameDownload.
    let download = |requested| GameDownload {
        source: "26.3-0.1.8".into(),
        file: INDEX.into(),
        size: 1,
        requested: Some(requested),
        time: String::new(),
        published_at: String::new(),
    };
    let explicit_x64 = download(GameArchitecture::X64).architecture().unwrap();
    assert_eq!(explicit_x64.requested, GameArchitecture::X64);
    assert_eq!(explicit_x64.binary, GameArchitecture::X64);
    let arm64_fallback = download(GameArchitecture::Arm64).architecture().unwrap();
    assert_eq!(arm64_fallback.requested, GameArchitecture::Arm64);
    assert_eq!(arm64_fallback.binary, GameArchitecture::X64);
}

#[test]
fn first_play_proves_and_installs_the_two_sdk_slices() {
    isolated_case(
        "sdk_install::first_play_proves_and_installs_the_two_sdk_slices",
        || {
            let requested = selected_game_architecture(&Settings::load());
            let Some(triple) = sdk_target(requested) else {
                return;
            };
            let sdk_tag = format!("sdk-{COMMIT}");
            let mirror = PathBuf::from(std::env::var_os(BASE_VAR).unwrap());
            let base = Base::from_env();
            let (key, _) = SigningKey::generate().unwrap();
            let keys = [key.public()];
            // Exercise the install boundary even when an earlier offer no longer matches.
            let release = game_releases_with(&[row(DEV, &[INDEX])], &[Some(INDEX)])
                .pop()
                .unwrap();
            assert!(crate::managed_play::FirstBuild::download(&release).is_ok());
            let download = GameDownload::select(&release).unwrap();
            let names =
                [Kind::Runtime, Kind::Programs].map(|kind| Asset::name(kind, triple, Side::Client));
            let slices = [
                (names[0].as_str(), SLICES[0]),
                (names[1].as_str(), SLICES[1]),
            ];
            let game = version_dir(DEV);
            let phases = std::cell::RefCell::new(Vec::new());
            let install = |files: &[(&str, &[u8])], sdk_rust: &str, tamper: bool| {
                phases.borrow_mut().clear();
                // A playable runtime does not need a matching launcher Rust toolchain.
                let index = index_of(files, "1.0.0").to_json();
                publish(
                    &mirror,
                    DEV,
                    DEV,
                    &key,
                    index.as_bytes(),
                    &[(RELEASE_FILE, FACTS)],
                );
                let sdk_index = index_of(files, sdk_rust).to_json();
                publish(
                    &mirror,
                    &sdk_tag,
                    &sdk_tag,
                    &key,
                    sdk_index.as_bytes(),
                    files,
                );
                if tamper {
                    let mut damaged = SLICES[0].to_vec();
                    let offset = damaged.len() / 2;
                    damaged[offset] ^= 1;
                    fs::write(mirror.join(&sdk_tag).join(&names[0]), damaged).unwrap();
                }
                let proof = download
                    .prove_with(DEV, &keys, |name| {
                        base.release_file(DEV, DEV, name).map_err(|e| e.to_string())
                    })
                    .unwrap();
                let proof = GameProof::saved(serde_json::to_value(proof).unwrap()).unwrap();
                let outcome = download.publish_with(
                    DEV,
                    &proof,
                    InstallIntent::Exact,
                    crate::backend::installation::Transfer {
                        stop: None,
                        progress: |phase| {
                            if phase == InstallPhase::Finishing {
                                let stage = fs::read_dir(versions_dir())
                                    .unwrap()
                                    .map(|entry| entry.unwrap())
                                    .find(|entry| {
                                        crate::backend::installation::private_stage(
                                            &entry.file_name(),
                                        )
                                    })
                                    .expect("SDK stage while finishing")
                                    .path();
                                let preparer = crate::preparer::BINARY;
                                assert!(stage.join(Kind::Programs.name()).join(preparer).is_file());
                                assert!(!stage
                                    .join(Kind::Runtime.name())
                                    .join("bin")
                                    .join(preparer)
                                    .exists());
                            }
                            phases.borrow_mut().push(phase);
                        },
                    },
                    |_, _| unreachable!("SDK reads from the mirror"),
                    (&base, &keys),
                );
                (proof, outcome)
            };
            let (_, missing) = install(&slices[..1], "1.0.0", false);
            assert_eq!(
                missing.unwrap_err(),
                format!("{DEV} has no build for the selected game architecture.")
            );
            assert!(!game.exists());
            let (_, mismatch) = install(&slices, "1.0.1", false);
            let error = mismatch.unwrap_err();
            assert!(
                error.contains(INDEX) && error.contains("signed checksum"),
                "{error}"
            );
            assert!(!game.exists());
            let (_, tampered) = install(&slices, "1.0.0", true);
            let error = tampered.unwrap_err();
            assert!(
                error.contains(&names[0]) && error.contains("checksum"),
                "{error}"
            );
            assert!(!game.exists());
            let (proof, installed) = install(&slices, "1.0.0", false);
            assert!(matches!(
                installed.unwrap(),
                InstallOutcome::Installed(None)
            ));
            assert_eq!(
                *phases.borrow(),
                [
                    InstallPhase::Unpacking,
                    InstallPhase::Unpacking,
                    InstallPhase::Finishing
                ],
                "a local mirror extracts both slices without claiming a download"
            );
            let cached = download
                .publish_with(
                    DEV,
                    &proof,
                    InstallIntent::Exact,
                    crate::backend::installation::Transfer {
                        stop: None,
                        progress: |phase| panic!("cache hit unexpectedly reported {phase:?}"),
                    },
                    |_, _| unreachable!("cache hit needs no assets"),
                    (&base, &keys),
                )
                .unwrap();
            assert!(matches!(cached, InstallOutcome::AlreadyInstalled(None)));
            assert!(download.is_installed(DEV, &proof));
            assert!(crate::managed_play::FirstBuild::installed(DEV).is_ok());
            let (library, std) = match std::env::consts::OS {
                "windows" => ("sunrise_fixture.dll", "std-fixture.dll"),
                "macos" => ("libsunrise_fixture.dylib", "libstd-fixture.dylib"),
                _ => ("libsunrise_fixture.so", "libstd-fixture.so"),
            };
            for file in [
                CLIENT,
                crate::preparer::BINARY,
                library,
                std,
                "exporters/manifest.json",
                "research-lock.json",
                COMPLETE,
            ] {
                assert!(game.join(file).is_file(), "missing {file}");
            }
            assert!(!game.join("liblink-only.a").exists());
            if cfg!(windows) {
                assert!(!game.join("libsunrise_fixture.dll.a").exists());
                assert!(game.join("dxcompiler.dll").is_file());
                assert!(game.join("libgcc_s_seh-1.dll").is_file());
            }
            assert_eq!(fs::read(game.join(RELEASE_FILE)).unwrap(), FACTS);
            assert!(!game.join(Artifact::ARCHIVE_RECEIPT).exists());
            let receipt: serde_json::Value =
                serde_json::from_slice(&fs::read(game.join(Artifact::SDK_RECEIPT)).unwrap())
                    .unwrap();
            assert_eq!(receipt["sdk"]["commit"], COMMIT);
            for (name, bytes) in slices {
                assert_eq!(receipt["sdk"]["slices"][name], sha256_hex(bytes));
            }
            let native = game.with_file_name("native-copy");
            fs::create_dir(&native).unwrap();
            crate::native_mods::copy_base(&game, &native, crate::native_mods::Target::Client, true)
                .unwrap();
            assert!(native.join(crate::preparer::BINARY).is_file());
            assert!(!native.join(Artifact::SDK_RECEIPT).exists());
            fs::remove_dir_all(&game).unwrap();
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let remote = Base::Url(format!("http://{}", listener.local_addr().unwrap()));
            let server = serve_sdk_files(listener, mirror);
            let phases = std::cell::RefCell::new(Vec::new());
            let installed = download
                .publish_with(
                    DEV,
                    &proof,
                    InstallIntent::Exact,
                    crate::backend::installation::Transfer {
                        stop: None,
                        progress: |phase| phases.borrow_mut().push(phase),
                    },
                    |_, _| unreachable!("SDK reads from its base"),
                    (&remote, &keys),
                )
                .unwrap();
            assert!(matches!(installed, InstallOutcome::Installed(None)));
            server.join().unwrap();
            let phases = phases.into_inner();
            let (finishing, transfers) = phases.split_last().unwrap();
            assert_eq!(*finishing, InstallPhase::Finishing);
            let slices_progress: Vec<_> =
                transfers.split(|p| *p == InstallPhase::Unpacking).collect();
            let [first, second, after_unpack] = slices_progress.as_slice() else {
                panic!("expected two slice extractions: {phases:?}");
            };
            let total = (SLICES[0].len() + SLICES[1].len()) as f32;
            let first_fraction = SLICES[0].len() as f32 / total;
            assert_eq!(
                first.last(),
                Some(&InstallPhase::Downloading(first_fraction))
            );
            assert!(first.iter().all(
                |p| matches!(p, InstallPhase::Downloading(f) if (0.0..=first_fraction).contains(f))
            ));
            assert_eq!(second.last(), Some(&InstallPhase::Downloading(1.0)));
            assert!(second.iter().all(
                |p| matches!(p, InstallPhase::Downloading(f) if (first_fraction..=1.0).contains(f))
            ));
            assert!(after_unpack.is_empty());
            fs::write(game.join(library), b"swapped library").unwrap();
            assert!(!download.is_installed(DEV, &proof));
            assert!(crate::managed_play::FirstBuild::installed(DEV).is_err());
        },
    );
}

fn serve_sdk_files(
    listener: std::net::TcpListener,
    mirror: PathBuf,
) -> std::thread::JoinHandle<()> {
    use std::io::{BufRead, BufReader, Write};
    std::thread::spawn(move || {
        // The game's index and facts, then the SDK's index, sums, signature and two slices.
        for _ in 0..7 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            let mut reader = BufReader::new(&stream);
            reader.read_line(&mut request).unwrap();
            let path = request.split_whitespace().nth(1).unwrap();
            let bytes = fs::read(mirror.join(path.strip_prefix('/').unwrap())).unwrap();
            let mut header = String::new();
            loop {
                if reader.read_line(&mut header).unwrap() == 0 || header == "\r\n" {
                    break;
                }
                header.clear();
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            )
            .unwrap();
            stream.write_all(&bytes).unwrap();
        }
    })
}

#[test]
fn player_archive_still_installs_when_the_index_is_listed() {
    isolated_case(
        "sdk_install::player_archive_still_installs_when_the_index_is_listed",
        || signed_player_archive(true),
    );
}

#[test]
fn signed_archive_without_a_length_reports_indeterminate_downloading() {
    isolated_case(
        "sdk_install::signed_archive_without_a_length_reports_indeterminate_downloading",
        || signed_player_archive(false),
    );
}

fn signed_player_archive(content_length: bool) {
    let Some(asset) = ASSET else { return };
    let mut catalog = row(TAG, &[INDEX, asset]);
    if !content_length {
        catalog["assets"][1]["size"] = serde_json::json!(0);
    }
    let release = game_releases(&[catalog]).pop().unwrap();
    let download = GameDownload::select(&release).unwrap();
    assert_eq!(download.file, asset);
    let bytes = if asset.ends_with(".zip") {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file(CLIENT, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"static client").unwrap();
        zip.finish().unwrap().into_inner()
    } else {
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        let mut header = tar::Header::new_gnu();
        header.set_size(13);
        header.set_mode(0o755);
        tar.append_data(&mut header, CLIENT, b"static client".as_slice())
            .unwrap();
        tar.into_inner().unwrap().finish().unwrap()
    };
    let mirror = PathBuf::from(std::env::var_os(BASE_VAR).unwrap());
    let (key, _) = SigningKey::generate().unwrap();
    let keys = [key.public()];
    let base = Base::from_env();
    publish(
        &mirror,
        TAG,
        TAG,
        &key,
        b"unused SDK index",
        &[(asset, &bytes)],
    );
    let proof = download
        .prove_with(TAG, &keys, |name| {
            base.release_file(TAG, TAG, name).map_err(|e| e.to_string())
        })
        .unwrap();
    if !content_length {
        assert_eq!(download.size, 0);
    }
    let (url, server) = serve_archive(asset, bytes, content_length);
    let phases = std::cell::RefCell::new(Vec::new());
    download
        .publish_with(
            TAG,
            &proof,
            InstallIntent::Exact,
            crate::backend::installation::Transfer {
                stop: None,
                progress: |phase| phases.borrow_mut().push(phase),
            },
            |_, _| url.clone(),
            (&base, &keys),
        )
        .unwrap();
    server.join().unwrap();
    let phases = phases.into_inner();
    let [download @ .., InstallPhase::Unpacking, InstallPhase::Finishing] = phases.as_slice()
    else {
        panic!("expected transfer, extraction, then publication: {phases:?}");
    };
    if content_length {
        assert!(download
            .iter()
            .all(|p| matches!(p, InstallPhase::Downloading(f) if (0.0..=1.0).contains(f))));
        assert_eq!(download.last(), Some(&InstallPhase::Downloading(1.0)));
    } else {
        assert_eq!(download, [InstallPhase::DownloadingUnknownLength]);
    }
    let game = version_dir(TAG);
    assert_eq!(fs::read(game.join(CLIENT)).unwrap(), b"static client");
    assert!(game.join(Artifact::ARCHIVE_RECEIPT).is_file());
    assert!(!game.join(Artifact::SDK_RECEIPT).exists());
}

#[test]
fn explicit_install_preserves_sdk_lookup_errors_instead_of_saying_no_build() {
    const MISSING_SLICE: &str = "26.3-0.1.10";
    const UNREACHABLE: &str = "26.3-0.1.11";
    let settings = Settings::default();
    let Some(triple) = sdk_target(selected_game_architecture(&settings)) else {
        return;
    };
    let mirror = tempfile::tempdir().unwrap();
    let base = Base::Folder(mirror.path().to_owned());
    let (key, _) = SigningKey::generate().unwrap();
    let keys = [key.public()];
    let names = [Kind::Runtime, Kind::Programs].map(|kind| Asset::name(kind, triple, Side::Client));
    let slices = [(names[0].as_str(), SLICES[0])];
    // Verified index missing one slice: legitimately has no build for this platform.
    publish(
        mirror.path(),
        MISSING_SLICE,
        MISSING_SLICE,
        &key,
        index_of(&slices, "1.0.0").to_json().as_bytes(),
        &[("sunrise-release.json", b"{}")],
    );
    // UNREACHABLE has no index file at all, simulating a fetch/lookup failure.
    let list = [row(MISSING_SLICE, &[INDEX]), row(UNREACHABLE, &[INDEX])];

    let no_build =
        install_explicit_tag_from(&list, MISSING_SLICE, false, &settings, &base, &keys, |_| {
            panic!("should not install a build missing a slice")
        })
        .unwrap_err();
    assert!(
        no_build.contains("has no build for this platform"),
        "verified missing slice should report no build, got: {no_build}"
    );

    let lookup_err =
        install_explicit_tag_from(&list, UNREACHABLE, false, &settings, &base, &keys, |_| {
            panic!("should not install when lookup failed")
        })
        .unwrap_err();
    assert!(
        !lookup_err.contains("has no build for this platform"),
        "lookup error should be preserved rather than reported as no build, got: {lookup_err}"
    );
}

#[test]
fn explicit_install_does_not_prune_sdk_only_builds_with_unresolved_catalogue() {
    isolated_case(
        "sdk_install::explicit_install_does_not_prune_sdk_only_builds_with_unresolved_catalogue",
        || {
            const SDK_TAG: &str = "26.3-0.1.10";
            const STATIC_TAG: &str = "26.3-0.1.9";
            let Some(static_asset) = ASSET else {
                return;
            };
            let settings = Settings {
                keep_versions: Some(1),
                ..Settings::default()
            };
            if sdk_target(selected_game_architecture(&settings)).is_none() {
                return;
            }
            settings.save().unwrap();
            // Pretend an older SDK-only build is installed and auto-downloaded so auto-prune would
            // delete it if handed an unresolved catalogue where SDK_TAG had `asset: None`.
            let sdk_dir = version_dir(SDK_TAG);
            fs::create_dir_all(&sdk_dir).unwrap();
            fs::write(sdk_dir.join(CLIENT), b"sdk client").unwrap();
            fs::write(sdk_dir.join(Artifact::SDK_RECEIPT), b"{}").unwrap();
            // Create a profile following Stable (which points at SDK_TAG when resolved).
            let _ = crate::instances::create("Other", None).unwrap();

            let mirror = tempfile::tempdir().unwrap();
            let base = Base::Folder(mirror.path().to_owned());
            let (key, _) = SigningKey::generate().unwrap();
            let keys = [key.public()];
            let list = [row(SDK_TAG, &[INDEX]), row(STATIC_TAG, &[static_asset])];

            let installed_tag = install_explicit_tag_from(
                &list,
                STATIC_TAG,
                false,
                &settings,
                &base,
                &keys,
                |_| {
                    let dir = version_dir(STATIC_TAG);
                    fs::create_dir_all(&dir).unwrap();
                    fs::write(dir.join(CLIENT), b"static client").unwrap();
                    Ok(InstallOutcome::Installed(None))
                },
            )
            .unwrap();
            assert_eq!(installed_tag, STATIC_TAG);
            assert!(
                sdk_dir.join(CLIENT).is_file(),
                "installed SDK-only build must not be pruned against an unresolved catalogue"
            );
        },
    );
}
