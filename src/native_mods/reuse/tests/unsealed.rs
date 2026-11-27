//! Through the default build, a slot with no proof of what built it is never used or sealed.
use super::*;

/// Through the default build entry on a developer base, which plans without any SDK check,
/// neither a reduced legacy build nor one with an unusable executable is said to be up to date
/// or played, and a game changed after it was sealed is never sealed again. These plans try the
/// next version instead, which this fixture can't compile.
#[test]
fn the_default_build_never_takes_a_reduced_or_changed_build() {
    if !in_child(
        "native_mods::reuse::tests::unsealed::the_default_build_never_takes_a_reduced_or_changed_build",
        "SUNRISE_REUSE_NEVER_TAKES_CHILD",
    ) {
        return;
    }
    const DEVELOPER: &str = "developer-reuse";
    let developer = format!(r#"{{"revision":"{PINNED}"}}"#);
    let instance = install_base(DEVELOPER, &[("developer.json", developer)]);
    // The developer base's commit has an SDK release, kept, with no build tool: a plan that gets
    // as far as compiling stops there.
    keep_index(PINNED, "2026-11-13");
    let platform = platform_for_game(DEVELOPER).unwrap();
    let up_to_date = |steps: &[Step]| {
        steps
            .iter()
            .any(|step| matches!(step, Step::Plan(Plan::UpToDate, _)))
    };

    // An older launcher's build without beta, under the whole set's version.
    let set = Set::developer("reuse-legacy-reduced", DEVELOPER).on(platform);
    let whole = set.install(&BuildOptions::default());
    set.make_reduced(&whole, &set.left("beta"));
    let record = fs::read(backend::version_dir(&whole).join(RECORD)).unwrap();
    let (built, _, steps) = build_default(&set, &instance);
    assert!(built.is_err(), "the next version is built: {built:?}");
    assert!(!up_to_date(&steps), "{steps:?}");
    assert_eq!(
        instances::find("sunrise").unwrap().version,
        None,
        "the instance doesn't play the reduced build"
    );
    assert_eq!(
        fs::read(backend::version_dir(&whole).join(RECORD)).unwrap(),
        record
    );
    assert!(
        !set.evidence(&whole).exists(),
        "the reduced build isn't sealed"
    );

    // A legacy build with all its mods and records but no evidence is never sealed, whatever
    // its executable reads as now: a capture proves what is on disk, not what built it.
    let set = Set::developer("reuse-legacy-no-evidence", DEVELOPER).on(platform);
    let version = set.install(&BuildOptions::default());
    fs::remove_file(set.evidence(&version)).unwrap();
    let game = backend::version_dir(&version).join(backend::CLIENT);
    fs::remove_file(&game).unwrap();
    type Change = dyn Fn(&Path);
    let readable: &Change = &|game| fs::write(game, game_executable()).unwrap();
    let changed: &Change = &|game| fs::write(game, b"another game").unwrap();
    let missing: &Change = &|_| {};
    let directory: &Change = &|game| fs::create_dir(game).unwrap();
    for (case, put) in [
        ("readable executable", readable),
        ("changed readable executable", changed),
        ("missing executable", missing),
        ("directory at executable path", directory),
    ] {
        let _ = fs::remove_file(&game);
        let _ = fs::remove_dir(&game);
        put(&game);
        let (built, _, steps) = build_default(&set, &instance);
        assert!(
            built.is_err(),
            "{case}: the next version is built: {built:?}"
        );
        assert!(!up_to_date(&steps), "{case}: {steps:?}");
        assert_eq!(
            instances::find("sunrise").unwrap().version,
            None,
            "{case}: not pinned"
        );
        assert!(!set.evidence(&version).exists(), "{case}: not sealed");
    }

    // A build that was sealed, then its game changed.
    let set = Set::developer("reuse-changed-game", DEVELOPER).on(platform);
    let version = set.install(&BuildOptions::default());
    let (built, _, steps) = build_default(&set, &instance);
    assert!(
        matches!(&built, Ok(ClientBuild::Native(found)) if *found == version),
        "{built:?}"
    );
    assert!(up_to_date(&steps));
    let evidence = fs::read(set.evidence(&version)).unwrap();
    fs::write(
        backend::version_dir(&version).join(backend::CLIENT),
        b"another game",
    )
    .unwrap();
    for attempt in ["first", "next"] {
        let (built, _, steps) = build_default(&set, &instance);
        assert!(
            built.is_err(),
            "{attempt}: the next version is built: {built:?}"
        );
        assert!(!up_to_date(&steps), "{attempt}: {steps:?}");
        assert_eq!(
            fs::read(set.evidence(&version)).unwrap(),
            evidence,
            "{attempt}: the changed game isn't sealed"
        );
    }
}

/// A matching build is found wherever it is in the version sequence, past a hole and past a
/// slot that holds a directory without its completion marker, and a plan that has to build
/// takes the first path nothing is at: never one publication would refuse (a directory without
/// its marker, a file).
#[test]
fn a_matching_build_is_found_past_a_hole_and_an_occupied_slot() {
    let _guard = scratch_state();
    let set = Set::new("reuse-holes");
    let options = BuildOptions::default();
    let game = |version: &str| backend::version_dir(version).join(backend::CLIENT);
    let first = set.install(&options);
    fs::write(game(&first), b"changed").unwrap();
    let second = set.install(&options);
    fs::write(game(&second), b"changed").unwrap();
    let third = set.install(&options);
    assert_eq!(versions(&first).nth(2).as_deref(), Some(third.as_str()));

    // The first slot loses its marker, and the second is gone.
    fs::remove_file(backend::version_dir(&first).join(".installed")).unwrap();
    fs::remove_dir_all(backend::version_dir(&second)).unwrap();
    assert_eq!(set.found().as_deref(), Some(third.as_str()));
    assert_eq!(set.install(&options), third, "used again, nothing built");
    assert!(!backend::version_dir(&second).exists());

    // A file at the hole isn't a vacancy either: the next path is.
    fs::write(backend::version_dir(&second), b"").unwrap();
    fs::write(game(&third), b"changed").unwrap();
    assert!(set.found().is_none());
    let fourth = set.install(&options);
    assert_eq!(versions(&first).nth(3).as_deref(), Some(fourth.as_str()));
    assert!(backend::version_dir(&second).is_file());

    // With the hole free again, it is the first vacant path, and the markerless slot is untouched.
    fs::remove_file(backend::version_dir(&second)).unwrap();
    fs::write(game(&fourth), b"changed").unwrap();
    assert_eq!(set.install(&options), second);
    assert!(!backend::version_dir(&first).join(".installed").exists());
    for version in [first, second, third, fourth] {
        let _ = fs::remove_dir_all(backend::version_dir(&version));
    }
}

/// The SDK a developer build compiles against is part of its image, with every pin explicit: a
/// build made against the published SDK is not the one made against the developer's own
/// prebuilt game, or against another package in the same folder, and each is found again by its
/// own SDK. An ordinary build never plays a build made against another one.
#[test]
fn the_sdk_a_build_compiles_against_is_part_of_its_image() {
    if !in_child(
        "native_mods::reuse::tests::unsealed::the_sdk_a_build_compiles_against_is_part_of_its_image",
        "SUNRISE_REUSE_SDK_IDENTITY_CHILD",
    ) {
        return;
    }
    const DEVELOPER: &str = "developer-reuse";
    let developer = format!(r#"{{"revision":"{PINNED}"}}"#);
    let instance = install_base(DEVELOPER, &[("developer.json", developer)]);
    keep_index(PINNED, "2026-11-13");
    let options = BuildOptions::default();
    let mut set =
        Set::developer("reuse-sdk-identity", DEVELOPER).on(platform_for_game(DEVELOPER).unwrap());

    let tool = set.root.join("developer-tool");
    fs::write(&tool, b"tool one").unwrap();
    // The environment belongs to this scratch child only.
    unsafe { std::env::set_var(sdk_cache::DEVELOPER_TOOL, &tool) };

    // Only a build made against the developer's package exists: an ordinary build can't use it.
    set.against("package one");
    let local = set.install(&options);
    assert_eq!(set.found().as_deref(), Some(local.as_str()));
    fs::write(&tool, b"tool two").unwrap();
    assert!(
        set.found().is_none(),
        "the local tool changed but the SDK did not"
    );
    fs::remove_file(&tool).unwrap();
    assert!(set.found().is_none(), "the selected local tool is missing");
    fs::write(&tool, b"tool one").unwrap();
    assert_eq!(set.found().as_deref(), Some(local.as_str()));
    let (built, _, steps) = build_default(&set, &instance);
    assert!(built.is_err(), "the next slot is built: {built:?}");
    assert!(!steps
        .iter()
        .any(|step| matches!(step, Step::Plan(Plan::UpToDate, _))));
    assert_eq!(instances::find("sunrise").unwrap().version, None);

    set.local_sdk = None;
    assert!(set.found().is_none(), "the published SDK isn't the package");
    let published = set.install(&options);
    assert_ne!(published, local, "another slot");
    assert_eq!(set.found().as_deref(), Some(published.as_str()));
    set.against("package one");
    assert_eq!(set.found().as_deref(), Some(local.as_str()));
    set.against("package two");
    assert!(set.found().is_none(), "another package in the same folder");

    let (built, _, _) = build_default(&set, &instance);
    assert!(
        matches!(&built, Ok(ClientBuild::Native(found)) if *found == published),
        "{built:?}"
    );
}
