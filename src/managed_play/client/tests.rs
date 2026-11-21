use super::*;
use serde_json::json;
use std::{fs, sync::Arc};
use sunrise_launcher_smod::{index, manifest, mod_list};

#[test]
fn play_checks_hash_only_takedowns_after_proving_the_actual_package() {
    const TEST: &str = "managed_play::client::tests::play_checks_hash_only_takedowns_after_proving_the_actual_package";
    const BUILD: &str = "v0.1.7";
    if std::env::var_os("PLAY_CATALOGUE_FIXTURE").is_none() {
        let root = tempfile::tempdir().unwrap();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env("PLAY_CATALOGUE_FIXTURE", "1")
            .env("SUNRISE_HOME", root.path().join("game"))
            .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .status()
            .unwrap()
            .success());
        return;
    }
    let home = backend::game_home();
    let build = backend::version_dir(BUILD);
    let prepared = backend::resources(BUILD);
    fs::create_dir_all(&prepared).unwrap();
    fs::write(prepared.join("sunrise-resources.json"), b"{}").unwrap();
    fs::write(build.join(".installed"), []).unwrap();
    fs::write(build.join(backend::CLIENT), "unchanged client").unwrap();
    let row_path = home.join(crate::browser::installed::MANIFEST);
    fs::create_dir_all(row_path.parent().unwrap()).unwrap();
    let relative = format!("{}/lantern.smod", mod_list::DIR);
    let package = home.join(&relative);
    fs::create_dir_all(package.parent().unwrap()).unwrap();
    let write_package = |version| {
        let body = format!("id='lantern'\nname='Lantern'\nversion='{version}'\napi='{}'\ncapabilities=['registries']\n", manifest::API);
        fs::write(
            &package,
            manifest::Package::write_files(&body, &[]).unwrap(),
        )
        .unwrap();
        index::sha256_file(&package).unwrap()
    };
    let old = write_package("1.0.0");
    fs::write(
        &row_path,
        json!({"entries":[{
            "source":"sunrise", "project":"lantern", "slug":"lantern", "title":"Lantern",
            "kind":"mod", "version":"1.0.0", "number":"1.0.0", "file":relative,
            "checksum":format!("SHA-256:{old}"), "page":"", "dependency":false,
        }]})
        .to_string(),
    )
    .unwrap();
    fs::write(home.join(mod_list::FILE), b"mod.lantern:true\n").unwrap();
    let recorded = fs::read_to_string(&row_path).unwrap();
    write_package("2.0.0");
    let instance = instances::selection().unwrap();
    let client = Client {
        instance: &instance,
        accepted_account: None,
        explicit_account: false,
        piped: false,
    };
    let mut profile = Profile::open(&home, None).unwrap();
    let cancellation = native_mods::Cancellation::default();
    let revoked = |id: &str, hash| {
        let mut result = index::Index::default();
        result.revoked.push(index::Revoked {
            id: id.into(),
            sha256: Some(hash),
            reason: "hash-only takedown fixture".into(),
            date: String::new(),
        });
        Some(Arc::new(result))
    };
    let (active, _) = client
        .ready(
            &mut profile,
            || FirstBuild::installed(BUILD),
            |_, _| Ok(()),
            (&cancellation, None),
            |_| {},
            || {
                let binding = crate::profile_txn::references(&home).unwrap();
                assert!(binding.targets.contains(BUILD));
                assert!(binding.pointer.is_none());
                revoked("lantern", old)
            },
        )
        .unwrap();
    // The old hash-only takedown does not deny the current input. Neither it nor
    // the current archive's ID and path can turn those new bytes into registry origin.
    assert_eq!(fs::read_to_string(&row_path).unwrap(), recorded);

    let replaced = write_package("3.0.0");
    let error = client
        .ready(
            &mut profile,
            || panic!("already bound"),
            |_, _| Ok(()),
            (&cancellation, None),
            |_| {},
            || revoked("lantern", replaced),
        )
        .expect_err("actual archive was taken down");
    assert!(String::from(error).contains("hash-only takedown fixture"));
    assert_eq!(profile.active().unwrap(), Some(active.clone()));
    assert_eq!(fs::read_to_string(&row_path).unwrap(), recorded);

    // A package with no browser row can still be a taken-down compiler input.
    let native = home.join(mod_list::NATIVE_DIR).join("native_lantern.smod");
    fs::create_dir_all(native.parent().unwrap()).unwrap();
    let body = format!("id='native_lantern'\nname='Native lantern'\nversion='1.0.0'\napi='{}'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{}'\n", manifest::API, "a".repeat(40));
    fs::write(
        &native,
        manifest::Package::write_files(
            &body,
            &[
                (
                    "Cargo.toml",
                    b"[package]\nname='native_lantern'\nversion='1.0.0'\n",
                ),
                ("src/lib.rs", b"fn fixture() {}"),
            ],
        )
        .unwrap(),
    )
    .unwrap();
    assert!(crate::native_status::enabled(&home).is_empty());
    fs::write(
        home.join(mod_list::FILE),
        b"mod.lantern:true\nmod.native_lantern:true\n",
    )
    .unwrap();
    assert_eq!(crate::native_status::enabled(&home).len(), 1);
    let mut listed = index::Index::default();
    listed.revoked.push(index::Revoked {
        id: "native_lantern".into(),
        sha256: None,
        reason: "only the listed project was taken down".into(),
        date: String::new(),
    });
    assert!(
        catalogue::inspect(&home, || Some(Arc::new(listed.clone())))
            .unwrap()
            .is_empty(),
        "an ID-wide takedown is not evidence against a local native archive"
    );
    let error = client
        .ready(
            &mut profile,
            || panic!("already bound"),
            |_, _| Ok(()),
            (&cancellation, None),
            |step| assert!(!matches!(step, Progress::Native(_)), "started native work"),
            || {
                let hash = index::sha256_file(&native).unwrap().to_ascii_uppercase();
                listed.revoked.push(index::Revoked {
                    id: "native_lantern".into(),
                    sha256: Some(hash),
                    reason: "exact native digest was taken down".into(),
                    date: String::new(),
                });
                Some(Arc::new(listed))
            },
        )
        .expect_err("native input was taken down");
    let error = String::from(error);
    assert!(error.contains("Native lantern was taken down"), "{error}");
    assert!(
        error.contains("exact native digest was taken down"),
        "{error}"
    );
    assert_eq!(profile.active().unwrap(), Some(active));
}

#[cfg(windows)]
#[test]
fn windows_bound_x64_fallback_survives_pending_retry_and_committed_replay() {
    use crate::{
        backend::GameArchitecture::Arm64,
        profile_txn::{Admission, Bootstrap},
    };
    const TEST: &str = "managed_play::client::tests::windows_bound_x64_fallback_survives_pending_retry_and_committed_replay";
    const BUILD: &str = "v0.1.7";
    if std::env::var_os("PLAY_VARIANT_FIXTURE").is_none() {
        let root = tempfile::tempdir().unwrap();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env("PLAY_VARIANT_FIXTURE", "1")
            .env("SUNRISE_HOME", root.path().join("game"))
            .env("SUNRISE_LAUNCHER_HOME", root.path().join("launcher"))
            .status()
            .unwrap()
            .success());
        return;
    }
    let home = backend::game_home();
    fs::create_dir_all(&home).unwrap();
    let directory = backend::version_dir(BUILD);
    let prepared = backend::resources(BUILD);
    fs::create_dir_all(&prepared).unwrap();
    fs::write(prepared.join("sunrise-resources.json"), b"{}").unwrap();
    fs::write(directory.join(".installed"), []).unwrap();
    let receipt = directory.join(super::super::Artifact::ARCHIVE_RECEIPT);
    let archive = "a".repeat(64);
    let install_candidate = |archive: &str, binary| {
        if receipt.is_file() {
            fs::remove_file(&receipt).unwrap();
        }
        fs::write(
            directory.join(backend::CLIENT),
            format!("{binary} fixture image"),
        )
        .unwrap();
        fs::write(
            directory.join(backend::GAME_BUILD_ARCHITECTURE),
            json!({"requested":"x64", "binary":binary}).to_string(),
        )
        .unwrap();
        super::super::Artifact::record_archive(&directory, BUILD, archive).unwrap();
    };
    install_candidate(&archive, "x64");
    assert!(backend::check_game_architecture(BUILD, None, Arm64).is_some());
    {
        let mut admission = Admission::open(&home, None).unwrap();
        admission
            .bind_bootstrap(
                || {
                    Ok(Bootstrap::new(
                        BUILD.into(),
                        BUILD.into(),
                        [1; 16],
                        super::super::inputs(&home, None).unwrap(),
                        json!({"kind":"download", "archive":{
                            "source":BUILD, "file":"sunrise-windows-x64.zip", "size":0,
                            "requested":"arm64", "time":"",
                        }}),
                    ))
                },
                |_| Ok(()),
            )
            .unwrap();
        admission
            .prove_bootstrap(|_| Ok(json!({"archive":archive, "facts":null})))
            .unwrap();
    }
    let instance = instances::selection().unwrap();
    let client = Client {
        instance: &instance,
        accepted_account: None,
        explicit_account: false,
        piped: false,
    };
    let cancellation = native_mods::Cancellation::default();
    let retry = |profile: &mut Profile| {
        client.ready(
            profile,
            || panic!("re-resolved the platform archive"),
            |tag, variant| {
                assert!(
                    variant.is_some(),
                    "fell back to an unbound cache preference"
                );
                backend::check_game_architecture(tag, variant, Arm64)
                    .map_or(Ok(()), |e| Err(e.into()))
            },
            (&cancellation, None),
            |_| {},
            || None,
        )
    };
    let mut profile = Profile::open(&home, None).unwrap();
    install_candidate(&"b".repeat(64), "x64");
    assert!(
        retry(&mut profile).is_err(),
        "accepted an unrelated cached x64 archive"
    );
    assert!(profile.active().unwrap().is_none());
    install_candidate(&archive, "x64");
    let (active, _) = retry(&mut profile).unwrap();
    drop(profile);
    let mut reopened = Profile::open(&home, None).unwrap();
    assert_eq!(retry(&mut reopened).unwrap().0, active);
    install_candidate(&archive, "arm64");
    assert!(
        retry(&mut reopened).is_err(),
        "accepted a different runtime binary"
    );
    assert_eq!(reopened.active().unwrap(), Some(active));
}
