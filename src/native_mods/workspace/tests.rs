use super::*;

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn an_escaped_worker_keeps_only_its_old_writable_paths() {
    use std::{
        io::Write,
        os::unix::process::CommandExt,
        process::{Command, Stdio},
    };
    let dir = tempfile::tempdir().unwrap();
    let mut first = Workspace::new(dir.path()).unwrap();
    let paths = |root: &Path| {
        [
            root.join("cargo/writer"),
            root.join("target/writer"),
            root.join("tmp/writer"),
        ]
    };
    let first_paths = paths(first.path());
    let mut command = Command::new("/bin/sh");
    command.args(["-ec", r#"read done; printf old > "$CARGO_HOME/writer"; printf old > "$CARGO_TARGET_DIR/writer"; printf old > "$TMPDIR/writer""#]).stdin(Stdio::piped());
    first.isolate(&mut command).unwrap();
    // SAFETY: setsid is async-signal-safe and this child was not made a process group leader.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    first.allow_worker();
    let mut old = command.spawn().unwrap();
    drop(first);
    let retry = Workspace::new(dir.path()).unwrap();
    retry.isolate(&mut Command::new("/bin/sh")).unwrap();
    let retry_paths = paths(retry.path());
    for path in &retry_paths {
        fs::write(path, "new").unwrap();
    }
    old.stdin.take().unwrap().write_all(b"continue\n").unwrap();
    assert!(old.wait().unwrap().success());
    for (old, new) in first_paths.iter().zip(&retry_paths) {
        assert_ne!(old, new);
        assert_eq!(fs::read_to_string(old).unwrap(), "old");
        assert_eq!(fs::read_to_string(new).unwrap(), "new");
    }
}

#[test]
fn writable_sdk_slices_do_not_share_the_cached_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let stage = Workspace::new(dir.path()).unwrap();
    let cached = Package::at(&dir.path().join("cache"));
    fs::create_dir_all(&cached.runtime).unwrap();
    fs::write(cached.runtime.join("input"), b"original").unwrap();
    let private = stage.package(&cached).unwrap();
    fs::write(private.runtime.join("input"), b"compiler scratch").unwrap();
    assert_eq!(fs::read(cached.runtime.join("input")).unwrap(), b"original");
}

#[test]
fn private_sdk_slices_keep_each_file_s_modified_time() {
    let dir = tempfile::tempdir().unwrap();
    let stage = Workspace::new(dir.path()).unwrap();
    let cached = Package::at(&dir.path().join("cache"));
    fs::create_dir_all(cached.api.join("deps")).unwrap();
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    for file in [cached.api.join("top"), cached.api.join("deps/nested")] {
        fs::write(&file, b"slice").unwrap();
        fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    // A read-only slice file keeps its time too.
    let top = cached.api.join("top");
    let mut readonly = fs::metadata(&top).unwrap().permissions();
    readonly.set_readonly(true);
    fs::set_permissions(&top, readonly).unwrap();
    let private = stage.package(&cached).unwrap();
    // The tool's build folder name hashes these times: the same every attempt.
    for file in [private.api.join("top"), private.api.join("deps/nested")] {
        assert_eq!(
            fs::metadata(&file).unwrap().modified().unwrap(),
            old,
            "{file:?}"
        );
    }
}

#[test]
fn registry_library_aliases_keep_their_exact_names_and_bytes_inside_private_roots() {
    let dir = tempfile::tempdir().unwrap();
    let stage = Workspace::new(dir.path()).unwrap();
    let root = stage.path();
    let package = Package::at(&root.join("package"));
    let rust = crate::rust_toolchain::Toolchain {
        dir: dir.path().join("rust"),
        host: crate::rust_toolchain::hosts().first().unwrap(),
    };
    let work = stage.work();
    let target = work.join("prebuilt-build/registry-fixture/target");
    let release = target.join(rust.host.target).join("release");
    let binary = work
        .join("prebuilt-package/bin")
        .join(crate::backend::CLIENT);
    let patch = release.join("libsunrise_patch.so");
    let mod_output = release.join("libgreeter.so");
    let output = target.join("deps/libfrom_target.so.2");
    let dependency = root.join("cargo/registry/src/thing/libfrom_cargo.so.1");
    let outside = root.join("tmp/libfrom_cargo.so.1");
    for file in [&binary, &patch, &mod_output, &output, &dependency, &outside] {
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, b"compiled bytes").unwrap();
    }
    let rust_version = crate::rust_toolchain::VERSION;
    let make_record = |file: &Path, kind, id: &str, version: &str| tool::LibraryRecord {
        file: file.file_name().unwrap().to_str().unwrap().into(),
        sha256: sha256_file(file).unwrap(),
        kind,
        mod_id: id.into(),
        identity: format!("fixture-build {id} {version} {rust_version} unwind"),
    };
    let mut built = tool::Built {
        route: tool::CheckRoute::Prebuilt,
        binary,
        libraries: vec![
            mod_output.clone(),
            patch.clone(),
            output,
            dependency.clone(),
        ],
        records: vec![
            make_record(&mod_output, tool::LibraryKind::Mod, "greeter", "1.0.0"),
            make_record(&patch, tool::LibraryKind::Patch, "sunrise_patch", "0"),
        ],
        mods: vec!["greeter".into()],
        internals: Default::default(),
    };
    let manifest = Manifest::parse("id = 'greeter'\nname = 'Greeter'\nversion = '1.0.0'\napi = '0.2'\ncapabilities = ['registries']\n[native]\ngame = '26.3'\nsunrise = 'a123456789012345678901234567890123456789'\n").unwrap();
    let verify = |built: &tool::Built| {
        stage.verify_outputs(
            built,
            &package,
            &rust,
            Some(tool::CheckRoute::Prebuilt),
            "fixture-build",
            [&manifest].into_iter(),
        )
    };
    #[cfg(unix)]
    {
        let alias = dependency.with_extension("");
        std::os::unix::fs::symlink(dependency.file_name().unwrap(), &alias).unwrap();
        built.libraries.push(alias);
    }
    let accepted = verify(&built).unwrap();
    let installed = dir.path().join("private-install");
    fs::create_dir(&installed).unwrap();
    for library in &accepted {
        library.copy_into(&installed).unwrap();
    }
    assert_eq!(
        fs::read(installed.join("libfrom_cargo.so.1")).unwrap(),
        b"compiled bytes"
    );
    assert_eq!(
        fs::read(installed.join("libfrom_target.so.2")).unwrap(),
        b"compiled bytes"
    );
    #[cfg(unix)]
    assert!(!fs::symlink_metadata(installed.join("libfrom_cargo.so"))
        .unwrap()
        .file_type()
        .is_symlink());
    fs::write(&dependency, b"substituted after verification").unwrap();
    let library = accepted
        .iter()
        .find(|row| row.name == "libfrom_cargo.so.1")
        .unwrap();
    assert!(library.copy_into(&installed).is_err());
    built.libraries[3] = outside.clone();
    assert!(
        verify(&built).is_err(),
        "neither the rest of the private stage nor outside is a Cargo root"
    );
    #[cfg(unix)]
    {
        let escaping = dependency.with_file_name("escaping.so.1");
        std::os::unix::fs::symlink(&outside, &escaping).unwrap();
        built.libraries[3] = escaping;
        assert!(
            verify(&built).is_err(),
            "a Cargo-home alias cannot escape its private roots"
        );
    }
}

#[test]
fn an_sdk_can_only_supply_its_own_files_and_the_exact_requested_mods() {
    let dir = tempfile::tempdir().unwrap();
    let stage = Workspace::new(dir.path()).unwrap();
    let package = Package::at(&stage.path().join("package"));
    let rust = crate::rust_toolchain::Toolchain {
        dir: dir.path().join("rust"),
        host: crate::rust_toolchain::hosts().first().unwrap(),
    };
    let work = stage.work();
    let bin = package.runtime.join("bin");
    fs::create_dir_all(&work).unwrap();
    fs::create_dir_all(&bin).unwrap();
    let binary = bin.join(crate::backend::CLIENT);
    let lib = work.join("greeter.dll");
    let outside = dir.path().join("outside.dll");
    for file in [&binary, &lib, &outside] {
        fs::write(file, b"bytes").unwrap();
    }
    let mut built = tool::Built {
        route: tool::CheckRoute::PrebuiltApi,
        binary: binary.clone(),
        libraries: vec![lib.clone()],
        mods: vec!["greeter".into()],
        records: vec![tool::LibraryRecord {
            file: "greeter.dll".into(),
            sha256: sha256_file(&lib).unwrap(),
            kind: tool::LibraryKind::Mod,
            mod_id: "greeter".into(),
            identity: format!(
                "fixture-build greeter 1.0.0 {} unwind",
                crate::rust_toolchain::VERSION
            ),
        }],
        internals: Default::default(),
    };
    let manifest = Manifest::parse(
        "id = 'greeter'\nname = 'Greeter'\nversion = '1.0.0'\napi = '0.2'\ncapabilities = ['registries']\n[native]\ngame = '26.3'\nsunrise = 'a123456789012345678901234567890123456789'\n",
    )
    .unwrap();
    let verify = |built: &tool::Built| {
        stage.verify_outputs(
            built,
            &package,
            &rust,
            Some(tool::CheckRoute::PrebuiltApi),
            "fixture-build",
            [&manifest].into_iter(),
        )
    };
    assert!(verify(&built).is_ok());
    built.route = tool::CheckRoute::Prebuilt;
    assert!(
        verify(&built).is_err(),
        "checked API route cannot publish an engine result"
    );
    built.route = tool::CheckRoute::PrebuiltApi;
    let fake_binary = work.join("another-executable");
    fs::write(&fake_binary, b"wrong executable").unwrap();
    built.binary = fake_binary;
    assert!(
        verify(&built).is_err(),
        "API result must use the SDK runtime executable"
    );
    built.binary = binary;
    fs::write(&lib, b"different output").unwrap();
    assert!(
        verify(&built).is_err(),
        "library record digest binds the copied output"
    );
    fs::write(&lib, b"bytes").unwrap();
    built.records[0].file = "another.dll".into();
    assert!(
        verify(&built).is_err(),
        "a library record names exactly one supplied path"
    );
    built.records[0].file = "greeter.dll".into();
    let identity = std::mem::replace(&mut built.records[0].identity, "wrong identity".into());
    assert!(
        verify(&built).is_err(),
        "records must name the exact build and mod version"
    );
    built.records[0].identity = identity;
    assert!(verify(&built).is_ok());
    let records = std::mem::take(&mut built.records);
    assert!(
        verify(&built).is_err(),
        "every requested mod needs its own record"
    );
    built.records = records;
    built.mods.push("unexpected".into());
    assert!(verify(&built).is_err());
    built.mods.pop();
    built.libraries.push(outside.clone());
    assert!(verify(&built).is_err());
    built.libraries[1] = lib;
    assert!(verify(&built).is_err());
    #[cfg(unix)]
    {
        let linked = work.join("linked.dll");
        std::os::unix::fs::symlink(outside, &linked).unwrap();
        built.libraries = vec![linked];
        assert!(verify(&built).is_err());
    }
}

/// Since sunrise#5136 a library's identity also names its mod's files and, for each native mod it
/// links, the SHA-256 of the identity that mod's library was built with.
#[test]
fn a_library_names_the_identities_of_the_mods_it_links() {
    use sunrise_launcher_smod::index::library_identity::library_identity;
    let dir = tempfile::tempdir().unwrap();
    let stage = Workspace::new(dir.path()).unwrap();
    let package = Package::at(&stage.path().join("package"));
    let rust = crate::rust_toolchain::Toolchain {
        dir: dir.path().join("rust"),
        host: crate::rust_toolchain::hosts().first().unwrap(),
    };
    let work = stage.work();
    let bin = package.runtime.join("bin");
    fs::create_dir_all(&work).unwrap();
    fs::create_dir_all(&bin).unwrap();
    let binary = bin.join(crate::backend::CLIENT);
    fs::write(&binary, b"bytes").unwrap();
    let rust_version = crate::rust_toolchain::VERSION;
    // As the tool stamps them: `base` first, then `dependent`, which links it.
    let base = format!(
        "{} unwind",
        library_identity(
            ["fixture-build", "base", "1.0.0", rust_version],
            "s1+g1",
            []
        )
    );
    let dependent = |base: &str| {
        format!(
            "{} unwind",
            library_identity(
                ["fixture-build", "dependent", "2.0.0", rust_version],
                "s2",
                [("base", base)]
            )
        )
    };
    let mut built = tool::Built {
        route: tool::CheckRoute::PrebuiltApi,
        binary,
        libraries: Vec::new(),
        mods: vec!["base".into(), "dependent".into()],
        records: Vec::new(),
        internals: Default::default(),
    };
    for (id, identity) in [("base", base.clone()), ("dependent", dependent(&base))] {
        let lib = work.join(format!("{id}.dll"));
        fs::write(&lib, id.as_bytes()).unwrap();
        built.records.push(tool::LibraryRecord {
            file: format!("{id}.dll"),
            sha256: sha256_file(&lib).unwrap(),
            kind: tool::LibraryKind::Mod,
            mod_id: id.into(),
            identity,
        });
        built.libraries.push(lib);
    }
    let manifest = |id: &str, version: &str| {
        Manifest::parse(&format!(
            "id = '{id}'\nname = 'M'\nversion = '{version}'\n[native]\ngame = '26.3'\nsunrise = 'a123456789012345678901234567890123456789'\n"
        ))
        .unwrap()
    };
    let manifests = [manifest("base", "1.0.0"), manifest("dependent", "2.0.0")];
    let verify = |built: &tool::Built| {
        stage.verify_outputs(
            built,
            &package,
            &rust,
            Some(tool::CheckRoute::PrebuiltApi),
            "fixture-build",
            manifests.iter(),
        )
    };
    assert!(verify(&built).is_ok());
    let mut refused = |identity: String, why: &str| {
        let kept = std::mem::replace(&mut built.records[1].identity, identity);
        assert!(verify(&built).is_err(), "{why}");
        built.records[1].identity = kept;
    };
    refused(
        dependent(&base.replace("s1+g1", "s1b+g1")),
        "built against another build of base",
    );
    refused(
        dependent(&base).replace("base=", "absent="),
        "links a mod this build did not make",
    );
    refused(
        dependent(&base).replace(" base=", " base"),
        "a malformed dependency list",
    );
    refused(
        dependent(&base).replace(" unwind", " abort"),
        "another panic strategy",
    );
    refused(
        dependent(&base).replace("fixture-build", "other-build"),
        "another game build",
    );
    refused(
        dependent(&base).replace(" 2.0.0 ", " 2.0.1 "),
        "another mod version",
    );
    refused(
        format!(
            "{} unwind extra",
            dependent(&base).trim_end_matches(" unwind")
        ),
        "an eighth field",
    );
    // An SDK from before sunrise#5136 stamps the five-field identity.
    for (row, (id, version)) in built
        .records
        .iter_mut()
        .zip([("base", "1.0.0"), ("dependent", "2.0.0")])
    {
        row.identity = format!("fixture-build {id} {version} {rust_version} unwind");
    }
    assert!(verify(&built).is_ok());
}

#[test]
fn a_build_too_deep_for_windows_is_found_before_it_starts() {
    let home = |user: &str| {
        std::path::PathBuf::from(format!(
            r"C:\Users\{user}\AppData\Roaming\sunrise\launcher\native\.nb-a1B2c3"
        ))
    };
    // #396's player, eight letters: 265 characters in the old folders, 232 now.
    assert_eq!(too_deep(&home("abcdefgh")), None);
    assert_eq!(too_deep(&home(&"u".repeat(35))), None);
    assert_eq!(too_deep(&home(&"u".repeat(36))), Some(260));
}

#[test]
fn a_slot_path_fits_a_35_character_windows_user_and_names_its_inputs() {
    let slot = key(crate::native_mods::Target::Client);
    let home = |user: &str| {
        std::path::PathBuf::from(format!(
            r"C:\Users\{user}\AppData\Roaming\sunrise\launcher\native\w\{}",
            slot.as_str()
        ))
    };
    // The same budget as an attempt's own folder in the test above.
    assert_eq!(too_long(&home(&"u".repeat(35))), None);
    assert_eq!(too_long(&home(&"u".repeat(36))), Some(260));
    let other = SlotKey::keyed(
        COMMIT,
        "x86_64-unknown-linux-gnu",
        crate::native_mods::Target::Client,
        "1.99.2",
    )
    .unwrap();
    assert_ne!(slot.as_str(), other.as_str());
    let line = slot.describe();
    for part in [
        slot.as_str(),
        COMMIT,
        "x86_64-unknown-linux-gnu",
        "client",
        "1.99.1",
    ] {
        assert!(line.contains(part), "{line}");
    }
}

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

/// A scratch build folder with its lease held, and the canonical path of that folder.
fn leased() -> (tempfile::TempDir, crate::native_cache::Cache, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let lease = crate::native_cache::Cache::acquire(&dir.path().join("build")).unwrap();
    let build = crate::backend::plain(dir.path().join("build").canonicalize().unwrap());
    (dir, lease, build)
}

fn key(side: crate::native_mods::Target) -> SlotKey {
    SlotKey::keyed(COMMIT, "x86_64-unknown-linux-gnu", side, "1.99.1").unwrap()
}

/// Every path under `root`, links listed and never followed.
fn listing(root: &Path) -> BTreeSet<PathBuf> {
    fn walk(dir: &Path, out: &mut BTreeSet<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            out.insert(path.clone());
            if fs::symlink_metadata(&path).unwrap().is_dir() {
                walk(&path, out);
            }
        }
    }
    let mut out = BTreeSet::new();
    walk(root, &mut out);
    out
}

#[test]
fn a_slot_is_keyed_by_game_build_target_side_and_rust_never_by_the_mods() {
    use crate::native_mods::Target::{Client, Server};
    let named =
        |commit: &str, triple: &str, side, rust: &str| SlotKey::keyed(commit, triple, side, rust);
    let base = named(COMMIT, "x86_64-pc-windows-gnu", Client, "1.99.1").unwrap();
    // A folder name is 10 lowercase hex characters, the digest of the four inputs.
    assert_eq!(base.as_str().len(), 10);
    assert!(base
        .as_str()
        .bytes()
        .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
    assert_eq!(
        base.as_str(),
        &sha256(format!("{COMMIT}\0x86_64-pc-windows-gnu\0client\01.99.1").as_bytes())[..10]
    );
    let other_commit = "f".repeat(40);
    for other in [
        named(&other_commit, "x86_64-pc-windows-gnu", Client, "1.99.1"),
        named(COMMIT, "aarch64-pc-windows-gnullvm", Client, "1.99.1"),
        named(COMMIT, "x86_64-pc-windows-gnu", Server, "1.99.1"),
        named(COMMIT, "x86_64-pc-windows-gnu", Client, "1.99.2"),
    ] {
        let other = other.unwrap();
        assert_ne!(other, base);
        assert_ne!(other.as_str(), base.as_str());
    }
    // Only a commit and names fit to be one folder make a key.
    assert!(named("abc", "x86_64-pc-windows-gnu", Client, "1.99.1").is_none());
    assert!(named(
        &COMMIT.to_uppercase(),
        "x86_64-pc-windows-gnu",
        Client,
        "1.99.1"
    )
    .is_none());
    assert!(named(COMMIT, "../x", Client, "1.99.1").is_none());
    assert!(named(COMMIT, "a/b", Client, "1.99.1").is_none());
    assert!(named(COMMIT, "x86_64-pc-windows-gnu", Client, "").is_none());

    // Two builds of different mod sets on one game build find the same folder, and the
    // other side finds another.
    let (_dir, lease, build) = leased();
    let native = build.join("native");
    let mut first = Workspace::new(&native).unwrap();
    let mut second = Workspace::new(&native).unwrap();
    let mut headless = Workspace::new(&native).unwrap();
    let client = key(Client);
    assert!(first.attach(&lease, client.clone()).unwrap());
    fs::write(first.work().join("kept"), b"compiled").unwrap();
    assert!(second.attach(&lease, client.clone()).unwrap());
    assert!(headless.attach(&lease, key(Server)).unwrap());
    assert_eq!(first.work(), native.join("w").join(client.as_str()));
    assert_eq!(second.work(), first.work());
    assert_eq!(fs::read(second.work().join("kept")).unwrap(), b"compiled");
    assert_ne!(headless.work(), first.work());
    assert!(!headless.work().join("kept").exists());
    // Cargo's home is the launcher's own `cargo/`, whatever the build.
    assert_eq!(first.cargo_home(), build.join("cargo"));
    assert_eq!(headless.cargo_home(), build.join("cargo"));
}

#[test]
fn a_developer_sdk_keeps_the_attempts_own_folders() {
    let (_dir, lease, build) = leased();
    let client = key(crate::native_mods::Target::Client);
    assert!(client.clone().usable(true).is_none());
    assert_eq!(client.clone().usable(false), Some(client));
    let stage = Workspace::new(&build.join("native")).unwrap();
    assert_eq!(stage.work(), stage.path().join("w"));
    assert_eq!(stage.cargo_home(), stage.path().join("cargo"));
    let mut command = std::process::Command::new("unused");
    stage.isolate(&mut command).unwrap();
    let env = |name: &str| {
        command
            .get_envs()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value)
    };
    assert_eq!(
        env("CARGO_TARGET_DIR"),
        Some(Some(stage.path().join("target").as_os_str()))
    );
    assert_eq!(
        env("CARGO_HOME"),
        Some(Some(stage.path().join("cargo").as_os_str()))
    );
    drop(lease);
    assert!(!build.join("native/w").exists());
}

#[test]
fn a_persistent_build_uses_the_slot_the_kept_slices_and_only_a_private_tmp() {
    let (_dir, lease, build) = leased();
    let mut stage = Workspace::new(&build.join("native")).unwrap();
    assert!(stage
        .attach(&lease, key(crate::native_mods::Target::Client))
        .unwrap());
    let mut command = std::process::Command::new("unused");
    stage.isolate(&mut command).unwrap();
    let env = |name: &str| {
        command
            .get_envs()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value)
    };
    let tmp = stage.path().join("tmp");
    assert_eq!(
        env("CARGO_HOME"),
        Some(Some(build.join("cargo").as_os_str()))
    );
    // The SDK gives Cargo its own target folder inside the work folder.
    assert_eq!(env("CARGO_TARGET_DIR"), Some(None));
    for name in ["TMPDIR", "TMP", "TEMP"] {
        assert_eq!(env(name), Some(Some(tmp.as_os_str())));
    }
    assert!(tmp.is_dir());
    assert!(!stage.path().join("target").exists());
    assert!(!stage.path().join("cargo").exists());
    // The slices are used where they are kept: nothing is copied.
    let cached = Package::at(&build.join("native/prebuilt/unit"));
    fs::create_dir_all(&cached.runtime).unwrap();
    fs::create_dir_all(&cached.api).unwrap();
    fs::write(cached.runtime.join("input"), b"original").unwrap();
    assert_eq!(stage.package(&cached).unwrap(), cached);
    assert!(!stage.path().join("package").exists());
    #[cfg(unix)]
    {
        let outside = _dir.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::remove_dir(&cached.api).unwrap();
        std::os::unix::fs::symlink(&outside, &cached.api).unwrap();
        assert!(stage.package(&cached).is_err());
    }
}

#[cfg(unix)]
#[test]
fn a_link_poisoned_slot_is_removed_without_following_it_and_built_cold() {
    use std::os::unix::fs::symlink;
    let (dir, lease, build) = leased();
    let native = build.join("native");
    let outside = dir.path().join("outside");
    fs::create_dir_all(outside.join("inner")).unwrap();
    fs::write(outside.join("canary"), b"alive").unwrap();
    fs::write(outside.join("inner/canary"), b"alive").unwrap();
    let client = key(crate::native_mods::Target::Client);
    let slot = native.join("w").join(client.as_str());
    let canaries = || {
        for file in ["canary", "inner/canary"] {
            assert_eq!(fs::read(outside.join(file)).unwrap(), b"alive");
        }
    };
    // The slot itself is a link.
    fs::create_dir_all(native.join("w")).unwrap();
    symlink(&outside, &slot).unwrap();
    let mut stage = Workspace::new(&native).unwrap();
    assert!(stage.attach(&lease, client.clone()).unwrap());
    assert!(fs::symlink_metadata(&slot).unwrap().is_dir());
    assert_eq!(stage.work(), slot);
    assert!(listing(&slot).is_empty());
    canaries();
    // A folder the SDK keeps inside a slot with other work in it is a link: all of it goes.
    for (poisoned, link_in) in [
        ("prebuilt-build/key/target", "prebuilt-build/key"),
        ("prebuilt-build/key/stage-1", "prebuilt-build/key"),
        ("prebuilt-build/key", "prebuilt-build"),
        ("prebuilt-build", ""),
        ("prebuilt-package", ""),
    ] {
        fs::write(slot.join("old-output"), b"stale").unwrap();
        fs::create_dir_all(slot.join(link_in)).unwrap();
        symlink(&outside, slot.join(poisoned)).unwrap();
        let mut stage = Workspace::new(&native).unwrap();
        assert!(stage.attach(&lease, client.clone()).unwrap());
        assert!(listing(&slot).is_empty(), "{poisoned}");
        // It starts cold, and a cold folder is never emptied again.
        assert!(!stage.retry_cold(&lease).unwrap());
        canaries();
    }
    // Cargo's home is a link.
    fs::remove_dir(build.join("cargo")).unwrap();
    symlink(&outside, build.join("cargo")).unwrap();
    let mut stage = Workspace::new(&native).unwrap();
    assert!(stage.attach(&lease, client).unwrap());
    assert!(fs::symlink_metadata(build.join("cargo")).unwrap().is_dir());
    assert_eq!(stage.cargo_home(), build.join("cargo"));
    assert!(listing(&build.join("cargo")).is_empty());
    canaries();
    // The folders above are not the lease's to redirect: `native/w` as a link goes too.
    let other = SlotKey::keyed(
        COMMIT,
        "x86_64-unknown-linux-gnu",
        crate::native_mods::Target::Server,
        "1.99.1",
    )
    .unwrap();
    fs::remove_dir_all(native.join("w")).unwrap();
    symlink(&outside, native.join("w")).unwrap();
    let mut stage = Workspace::new(&native).unwrap();
    assert!(stage.attach(&lease, other.clone()).unwrap());
    assert!(fs::symlink_metadata(native.join("w")).unwrap().is_dir());
    assert_eq!(stage.work(), native.join("w").join(other.as_str()));
    assert!(!outside.join(other.as_str()).exists());
    canaries();
}

#[cfg(unix)]
#[test]
fn a_persistent_sdk_invocation_refuses_a_linked_build_log() {
    let (dir, lease, build) = leased();
    let mut stage = Workspace::new(&build.join("native")).unwrap();
    assert!(stage
        .attach(&lease, key(crate::native_mods::Target::Client))
        .unwrap());
    let log = stage.work().join("build.log");
    let canary = dir.path().join("outside.log");
    fs::write(&canary, b"untouched").unwrap();
    std::os::unix::fs::symlink(&canary, &log).unwrap();
    let mut command = std::process::Command::new("unused");
    assert!(stage.isolate(&mut command).is_err());
    assert_eq!(fs::read(&canary).unwrap(), b"untouched");
    fs::remove_file(&log).unwrap();
    fs::write(&log, b"old diagnostics").unwrap();
    stage.isolate(&mut command).unwrap();
    assert!(!log.exists());

    // Redirecting the slot directory itself to an outside folder refuses before deleting `build.log`.
    let outside = dir.path().join("outside-slot");
    fs::create_dir(&outside).unwrap();
    let outside_log = outside.join("build.log");
    fs::write(&outside_log, b"do not delete").unwrap();
    let slot_dir = stage.work();
    fs::remove_dir_all(&slot_dir).unwrap();
    std::os::unix::fs::symlink(&outside, &slot_dir).unwrap();
    assert!(stage.isolate(&mut command).is_err());
    assert_eq!(fs::read(&outside_log).unwrap(), b"do not delete");
}

#[test]
fn a_file_in_an_sdk_owned_folder_position_resets_the_slot() {
    let (_dir, lease, build) = leased();
    let native = build.join("native");
    let client = key(crate::native_mods::Target::Client);
    let mut first = Workspace::new(&native).unwrap();
    assert!(first.attach(&lease, client.clone()).unwrap());
    for name in [
        "prebuilt-build/key",
        "prebuilt-build/key/target",
        "prebuilt-build/key/stage-1",
    ] {
        let wrong_type = first.work().join(name);
        fs::create_dir_all(wrong_type.parent().unwrap()).unwrap();
        fs::write(wrong_type, b"not a folder").unwrap();
        let mut stage = Workspace::new(&native).unwrap();
        assert!(stage.attach(&lease, client.clone()).unwrap());
        assert!(listing(&stage.work()).is_empty(), "{name}");
    }
}

#[test]
fn a_failed_kept_slot_is_emptied_and_built_cold_once_after_a_proved_stop() {
    let (_dir, lease, build) = leased();
    let native = build.join("native");
    let client = key(crate::native_mods::Target::Client);
    // A first build in an empty slot has nothing stale to blame.
    let mut first = Workspace::new(&native).unwrap();
    assert!(first.attach(&lease, client.clone()).unwrap());
    assert!(!first.retry_cold(&lease).unwrap());
    let stale = first.work().join("prebuilt-build/key/target/stale");
    fs::create_dir_all(stale.parent().unwrap()).unwrap();
    fs::write(&stale, b"stale").unwrap();
    let mut stage = Workspace::new(&native).unwrap();
    assert!(stage.attach(&lease, client).unwrap());
    // A compiler tree that may still run leaves its folder alone.
    stage.allow_worker();
    assert!(!stage.retry_cold(&lease).unwrap());
    assert!(stale.is_file());
    stage.worker_finished();
    let tmp_for = |stage: &Workspace| {
        let mut command = std::process::Command::new("unused");
        stage.isolate(&mut command).unwrap();
        command
            .get_envs()
            .find_map(|(key, val)| (key == "TMPDIR").then(|| val.unwrap()))
            .map(PathBuf::from)
            .unwrap()
    };
    let first_tmp = tmp_for(&stage);
    fs::write(first_tmp.join("stale"), b"old scratch").unwrap();
    assert!(stage.retry_cold(&lease).unwrap());
    let retry_tmp = tmp_for(&stage);
    assert_ne!(first_tmp, retry_tmp);
    assert!(fs::read_dir(retry_tmp).unwrap().next().is_none());
    assert!(!stale.exists());
    assert!(stage.work().is_dir());
    assert!(listing(&stage.work()).is_empty());
    // One cold build, not a loop, whatever that build leaves behind.
    fs::write(stage.work().join("partial"), b"partial").unwrap();
    assert!(!stage.retry_cold(&lease).unwrap());
    assert!(stage.work().join("partial").is_file());
}

#[cfg(unix)]
#[test]
fn cold_retry_refuses_a_redirected_slot_parent() {
    let (dir, lease, build) = leased();
    let native = build.join("native");
    let key = key(crate::native_mods::Target::Client);
    let mut stage = Workspace::new(&native).unwrap();
    assert!(stage.attach(&lease, key.clone()).unwrap());
    fs::write(stage.work().join("warm"), b"old output").unwrap();
    assert!(stage.attach(&lease, key.clone()).unwrap());

    let outside = dir.path().join("outside");
    let matching = outside.join(key.as_str());
    fs::create_dir_all(&matching).unwrap();
    let canary = matching.join("canary");
    fs::write(&canary, b"untouched").unwrap();
    fs::rename(native.join("w"), native.join("original-work")).unwrap();
    std::os::unix::fs::symlink(&outside, native.join("w")).unwrap();
    assert!(stage.retry_cold(&lease).is_err());
    assert_eq!(fs::read(&canary).unwrap(), b"untouched");
}

fn in_child(name: &str) -> bool {
    const CHILD: &str = "SUNRISE_NATIVE_WORKSPACE_TEST_CHILD";
    if std::env::var_os(CHILD).is_some() {
        return true;
    }
    let scratch = tempfile::tempdir().unwrap();
    let module = module_path!().split_once("::").unwrap().1;
    let test = format!("{module}::{name}");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &test, "--include-ignored", "--nocapture"])
        .env(CHILD, "1")
        .env("SUNRISE_LAUNCHER_HOME", scratch.path().join("launcher"))
        .env("SUNRISE_HOME", scratch.path().join("game"))
        .env(
            "SUNRISE_LAUNCHER_LOCATION_FILE",
            scratch.path().join("location"),
        )
        .env("SUNRISE_NO_KEYCHAIN", "1")
        .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
        .env("XDG_DATA_HOME", scratch.path().join("data"))
        .env("XDG_CONFIG_HOME", scratch.path().join("config"))
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("1 passed; 0 failed"),
        "child test did not run {test}: {stdout}"
    );
    false
}

#[test]
fn an_unproved_exit_skips_the_slot_in_memory_only_until_the_next_start() {
    if !in_child("an_unproved_exit_skips_the_slot_in_memory_only_until_the_next_start") {
        return;
    }
    let (_dir, lease, build) = leased();
    let native = build.join("native");
    let client = crate::native_mods::Target::Client;
    // The unproved state belongs to this scratch child only.
    let unproved = SlotKey::keyed(COMMIT, "unproved-triple", client, "9.9.9").unwrap();
    let proved = SlotKey::keyed(COMMIT, "proved-triple", client, "9.9.9").unwrap();
    let mut stage = Workspace::new(&native).unwrap();
    assert!(stage.attach(&lease, unproved.clone()).unwrap());
    assert_eq!(unproved.clone().usable(false), Some(unproved.clone()));
    let before = listing(&build);
    stage.allow_worker();
    stage.note_unproved();
    // No marker, probe or any other file: the only record is in memory.
    assert_eq!(listing(&build), before);
    assert!(unproved.clone().usable(false).is_none());
    // The builds that follow take the attempt's own folders.
    let attempt = Workspace::new(&native).unwrap();
    assert!(attempt.work().starts_with(attempt.path()));
    // Another game build can still use its own work slot, but not the shared Cargo home.
    let another = proved.clone().usable(false).unwrap();
    let mut stopped = Workspace::new(&native).unwrap();
    assert!(stopped.attach(&lease, another).unwrap());
    assert_ne!(stopped.work(), stage.work());
    assert_eq!(stopped.cargo_home(), stopped.path().join("cargo"));
    // A compiler tree proved gone leaves its slot in use.
    stopped.allow_worker();
    stopped.worker_finished();
    stopped.note_unproved();
    assert!(proved.clone().usable(false).is_some());
    // A build that never started a worker says nothing either.
    let mut idle = Workspace::new(&native).unwrap();
    assert!(idle.attach(&lease, proved.clone()).unwrap());
    idle.note_unproved();
    assert!(proved.usable(false).is_some());
    // The next launcher start reuses the slot as usual, with its files.
    fs::write(stage.work().join("kept"), b"compiled").unwrap();
    unproved.forget_unproved();
    let reused = unproved.clone().usable(false).unwrap();
    let mut next = Workspace::new(&native).unwrap();
    assert!(next.attach(&lease, reused).unwrap());
    assert_eq!(fs::read(next.work().join("kept")).unwrap(), b"compiled");
    assert!(next.retry_cold(&lease).unwrap());
    assert!(!next.work().join("kept").exists());
}

#[cfg(unix)]
#[test]
fn native_lock_uses_workspace_cargo_home_after_unproved_stop() {
    use super::super::{
        lock_missing_manifests, CompileAttempt,
        Target::{Client, Server},
    };
    use std::os::unix::fs::PermissionsExt;

    if !in_child("native_lock_uses_workspace_cargo_home_after_unproved_stop") {
        return;
    }
    let (dir, lease, build) = leased();
    let native = build.join("native");
    let program = dir.path().join("sdk");
    fs::write(
        &program,
        r#"#!/bin/sh
set -eu
[ "$1" = native-lock ]
read -r plan
printf '%s\n' "$plan" > "$TMPDIR/lock-plan.json"
printf '%s\n' "$CARGO_HOME" > "$TMPDIR/lock-cargo-home"
mod_dir=$(printf '%s' "$plan" | sed -n 's/.*"mods":"\([^"]*\)".*/\1/p')
printf 'version = 4\n' > "$mod_dir/Cargo.lock"
printf '{"contract":2,"lock":"%s/Cargo.lock"}\n' "$mod_dir"
"#,
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    let rust = crate::rust_toolchain::Toolchain {
        dir: dir.path().join("rust"),
        host: crate::rust_toolchain::host().unwrap(),
    };
    let package = Package::at(dir.path());
    let mut stopped = Workspace::new(&native).unwrap();
    assert!(stopped.attach(&lease, key(Client)).unwrap());
    stopped.allow_worker();
    stopped.note_unproved();
    assert!(cargo_home_unproved());

    let mut another = Workspace::new(&native).unwrap();
    assert!(another
        .attach(&lease, key(Server).usable(false).unwrap())
        .unwrap());
    let compiled = another.path().join("mods.compile");
    fs::create_dir_all(compiled.join("greeter")).unwrap();
    let cancel = tool::Cancellation::default();
    let attempt = CompileAttempt {
        rust: &rust,
        program: &program,
        lease: &lease,
        work_root: &native,
        package: &package,
        compiled: &compiled,
        mod_ids: &["greeter"],
        target: Server,
        mixins_from_source: false,
        cancellation: &cancel,
        staging: None,
    };
    lock_missing_manifests(&attempt, &another).unwrap();
    assert!(compiled.join("greeter/Cargo.lock").is_file());
    let tmp = another.path().join("tmp");
    let isolated_cargo = another.path().join("cargo");
    let plan: serde_json::Value =
        serde_json::from_slice(&fs::read(tmp.join("lock-plan.json")).unwrap()).unwrap();
    assert_eq!(
        plan["toolchain"]["cargo_home"],
        serde_json::json!(isolated_cargo)
    );
    assert_eq!(
        fs::read_to_string(tmp.join("lock-cargo-home"))
            .unwrap()
            .trim_end(),
        isolated_cargo.to_str().unwrap()
    );
    assert_ne!(isolated_cargo, build.join("cargo"));
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "daily; build the launcher binary first"]
fn persistent_sdk_process_uses_fresh_retry_tmp_and_isolates_cargo_after_stop() {
    use super::super::{
        run_compile_attempt, CompileAttempt, Failed,
        Target::{Client, Server},
    };
    use std::os::unix::fs::PermissionsExt;

    if !in_child("persistent_sdk_process_uses_fresh_retry_tmp_and_isolates_cargo_after_stop") {
        return;
    }
    let (dir, lease, build) = leased();
    let native = build.join("native");
    let program = dir.path().join("sdk");
    fs::write(&program, r#"#!/bin/sh
set -eu
read -r plan
if [ "$1" = native-lock ]; then
    printf '%s\n' "$plan" > "$TMPDIR/lock-plan.json"
    printf '%s\n' "$CARGO_HOME" > "$TMPDIR/lock-cargo-home"
    mod_dir=$(printf '%s' "$plan" | sed -n 's/.*"mods":"\([^"]*\)".*/\1/p')
    printf 'version = 4\n' > "$mod_dir/Cargo.lock"
    printf '{"contract":2,"lock":"%s/Cargo.lock"}\n' "$mod_dir"
    exit 0
fi
[ "$1" = native-build ]
printf '%s\n' "$plan" > "$TMPDIR/plan.json"
printf '%s\n' "$CARGO_HOME" > "$TMPDIR/cargo-home"
printf '%s\n' "$TMPDIR" > "$(dirname "$0")/last-tmp"
[ -z "${CARGO_TARGET_DIR+x}" ]
if [ -f "$(dirname "$0")/stop" ]; then
    printf '%s\n' '{"event":"mod","id":"greeter","done":1,"total":1}'
    # Hold the SDK open after the mod event so the launcher cancels before proof.
    read -r _ || :
    exit 1
fi
if [ -f "$(dirname "$0")/build/native/w/$(cat "$(dirname "$0")/slot")/warm" ]; then
    printf '%s\n' '{"event":"failed","error":"stale work","blame":null}'
    exit 1
fi
printf '%s\n' '{"event":"built","route":"prebuilt-api","binary":"unused","libraries":[],"records":[],"mods":[],"internals":{}}'
"#).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    let rust = crate::rust_toolchain::Toolchain {
        dir: dir.path().join("rust"),
        host: crate::rust_toolchain::host().unwrap(),
    };
    let package = Package::at(dir.path());
    let run = |stage: &mut Workspace,
               compiled: &Path,
               target: super::super::Target,
               cancel: &tool::Cancellation,
               on_mod: &dyn Fn(&str, usize, usize)| {
        let attempt = CompileAttempt {
            rust: &rust,
            program: &program,
            lease: &lease,
            work_root: &native,
            package: &package,
            compiled,
            mod_ids: &["greeter"],
            target,
            mixins_from_source: false,
            cancellation: cancel,
            staging: None,
        };
        run_compile_attempt(
            &attempt,
            stage,
            |_| Ok(()),
            on_mod,
            |failure| Failed::Tool {
                error: failure.error,
                retryable: false,
            },
            |_, _| Ok(Vec::new()),
        )
    };
    let assert_plan = |stage: &Workspace, tmp: &Path, cargo: &Path| {
        let plan: serde_json::Value =
            serde_json::from_slice(&fs::read(tmp.join("plan.json")).unwrap()).unwrap();
        assert_eq!(plan["work"], serde_json::json!(stage.work()));
        assert_eq!(plan["toolchain"]["cargo_home"], serde_json::json!(cargo));
        assert_eq!(
            fs::read_to_string(tmp.join("cargo-home"))
                .unwrap()
                .trim_end(),
            cargo.to_str().unwrap()
        );
    };
    let assert_lock = |tmp: &Path, cargo: &Path| {
        let plan: serde_json::Value =
            serde_json::from_slice(&fs::read(tmp.join("lock-plan.json")).unwrap()).unwrap();
        assert_eq!(plan["toolchain"]["cargo_home"], serde_json::json!(cargo));
        assert_eq!(
            fs::read_to_string(tmp.join("lock-cargo-home"))
                .unwrap()
                .trim_end(),
            cargo.to_str().unwrap()
        );
    };
    let client = key(Client);
    fs::write(dir.path().join("slot"), client.as_str()).unwrap();
    let mut stage = Workspace::new(&native).unwrap();
    assert!(stage.attach(&lease, client.clone()).unwrap());
    fs::write(stage.work().join("warm"), b"stale").unwrap();
    assert!(stage.attach(&lease, client.clone()).unwrap());
    let first_compiled = stage.path().join("mods.compile");
    fs::create_dir_all(first_compiled.join("greeter")).unwrap();
    let cancellation = tool::Cancellation::default();
    let no_mod = |_: &str, _: usize, _: usize| unreachable!();
    // Production orchestration locks the lockless mod, hits the warm-slot failure, retries cold
    // automatically in a fresh retry tmp, and succeeds.
    let (built, _) = run(&mut stage, &first_compiled, Client, &cancellation, &no_mod).unwrap();
    assert_eq!(built.route, tool::CheckRoute::PrebuiltApi);
    assert!(first_compiled.join("greeter/Cargo.lock").is_file());
    let first_tmp = stage.path().join("tmp");
    assert_lock(&first_tmp, &build.join("cargo"));
    assert_plan(&stage, &first_tmp, &build.join("cargo"));
    let retry_tmp = fs::read_to_string(dir.path().join("last-tmp")).unwrap();
    let retry_tmp = Path::new(retry_tmp.trim_end());
    assert_ne!(first_tmp.as_path(), retry_tmp);
    assert_plan(&stage, retry_tmp, &build.join("cargo"));

    // A stopped, warm compiler has no proved handoff. Its slot cannot be retried.
    fs::write(dir.path().join("stop"), b"").unwrap();
    assert!(stage.attach(&lease, client.clone()).unwrap());
    fs::write(stage.work().join("kept"), b"output").unwrap();
    assert!(stage.attach(&lease, client.clone()).unwrap());
    let stop = tool::Cancellation::default();
    assert!(
        run(&mut stage, &first_compiled, Client, &stop, &|_, _, _| stop
            .stop())
        .is_err()
    );
    assert!(stop.is_stopped());
    assert!(client.usable(false).is_none());
    assert!(!stage.retry_cold(&lease).unwrap());
    assert!(stage.work().join("kept").is_file());

    // After an unproved stop, both `native-lock` for a lockless mod and `native-build` must
    // use the next workspace's isolated Cargo home rather than the shared `<build>/cargo`.
    fs::remove_file(dir.path().join("stop")).unwrap();
    let mut another = Workspace::new(&native).unwrap();
    assert!(another
        .attach(&lease, key(Server).usable(false).unwrap())
        .unwrap());
    assert_ne!(another.work(), stage.work());
    let another_compiled = another.path().join("mods.compile");
    fs::create_dir_all(another_compiled.join("greeter")).unwrap();
    run(
        &mut another,
        &another_compiled,
        Server,
        &cancellation,
        &no_mod,
    )
    .unwrap();
    assert!(another_compiled.join("greeter/Cargo.lock").is_file());
    assert_lock(&another.path().join("tmp"), &another.path().join("cargo"));
    assert_plan(
        &another,
        &another.path().join("tmp"),
        &another.path().join("cargo"),
    );
}

#[test]
fn a_persistent_build_binds_each_output_to_the_one_root_it_belongs_to() {
    let (dir, lease, build) = leased();
    let mut stage = Workspace::new(&build.join("native")).unwrap();
    assert!(stage
        .attach(&lease, key(crate::native_mods::Target::Client))
        .unwrap());
    let package = Package::at(&build.join("native/prebuilt/unit"));
    let rust = crate::rust_toolchain::Toolchain {
        dir: dir.path().join("rust"),
        host: crate::rust_toolchain::hosts().first().unwrap(),
    };
    let work = stage.work();
    let cargo = stage.cargo_home();
    let target = work.join("prebuilt-build/fixture/target");
    let release = target.join(rust.host.target).join("release");
    let binary = work
        .join("prebuilt-package/bin")
        .join(crate::backend::CLIENT);
    let patch = release.join("libsunrise_patch.so");
    let mod_output = release.join("libgreeter.so");
    let in_target = target.join("deps/libfrom_target.so.2");
    let in_cargo = cargo.join("registry/src/thing/libfrom_cargo.so.1");
    let in_runtime = package.runtime.join("lib/libruntime_dep.so");
    // Places that are inside the launcher's folders but belong to none of the roots.
    let in_tmp = stage.path().join("tmp/libfrom_tmp.so.1");
    let beside_native = build.join("native/libbeside.so.1");
    let beside_cargo = build.join("cargo-other/libbeside.so.1");
    let in_other_work = work.join("elsewhere/libelsewhere.so.1");
    let in_other_slot = build.join("native/w/another-slot/prebuilt-build/key/target/libother.so.1");
    let stamped_in_cargo = cargo.join("libgreeter.so");
    let stamped_in_runtime = package.runtime.join("lib/libgreeter.so");
    let all = [
        &binary,
        &patch,
        &mod_output,
        &in_target,
        &in_cargo,
        &in_runtime,
        &in_tmp,
        &beside_native,
        &beside_cargo,
        &in_other_work,
        &in_other_slot,
        &stamped_in_cargo,
        &stamped_in_runtime,
    ];
    for file in all {
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, b"compiled bytes").unwrap();
    }
    let rust_version = crate::rust_toolchain::VERSION;
    let record = |file: &Path, kind, id: &str, version: &str| tool::LibraryRecord {
        file: file.file_name().unwrap().to_str().unwrap().into(),
        sha256: sha256_file(file).unwrap(),
        kind,
        mod_id: id.into(),
        identity: format!("fixture-build {id} {version} {rust_version} unwind"),
    };
    let manifest = Manifest::parse("id = 'greeter'\nname = 'Greeter'\nversion = '1.0.0'\napi = '0.2'\ncapabilities = ['registries']\n[native]\ngame = '26.3'\nsunrise = 'a123456789012345678901234567890123456789'\n").unwrap();
    // The greeter library as found at `mod_at`, and `more` beside it.
    let verify = |mod_at: &Path, more: &[&Path]| {
        let built = tool::Built {
            route: tool::CheckRoute::Prebuilt,
            binary: binary.clone(),
            libraries: [mod_at, patch.as_path()]
                .into_iter()
                .chain(more.iter().copied())
                .map(Path::to_path_buf)
                .collect(),
            records: vec![
                record(mod_at, tool::LibraryKind::Mod, "greeter", "1.0.0"),
                record(&patch, tool::LibraryKind::Patch, "sunrise_patch", "0"),
            ],
            mods: vec!["greeter".into()],
            internals: Default::default(),
        };
        stage.verify_outputs(
            &built,
            &package,
            &rust,
            Some(tool::CheckRoute::Prebuilt),
            "fixture-build",
            [&manifest].into_iter(),
        )
    };
    let accepted = verify(&mod_output, &[&in_target, &in_cargo, &in_runtime]).unwrap();
    let source = |name: &str| accepted.iter().find(|row| row.name == name).unwrap();
    assert_eq!(accepted.len(), 5);
    assert_eq!(source("libruntime_dep.so").source, in_runtime);
    assert_eq!(source("libfrom_cargo.so.1").source, in_cargo);
    assert_eq!(source("libfrom_target.so.2").source, in_target);
    for (refused, why) in [
        (&in_tmp, "the attempt's private tmp is no output root"),
        (&beside_native, "the rest of native/ is no output root"),
        (&beside_cargo, "a folder beside Cargo's home is not it"),
        (&in_other_work, "the work folder outside Cargo's target"),
        (&in_other_slot, "another game build's slot"),
    ] {
        assert!(verify(&mod_output, &[refused]).is_err(), "{why}");
    }
    assert!(
        verify(&stamped_in_cargo, &[]).is_err(),
        "a stamped library is the SDK's output, from the work folder only"
    );
    assert!(verify(&stamped_in_runtime, &[]).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let outside = dir.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("libescape.so.1"), b"compiled bytes").unwrap();
        // A file alias in the work folder or Cargo's home may not leave them.
        let work_alias = target.join("deps/libalias.so.1");
        symlink(outside.join("libescape.so.1"), &work_alias).unwrap();
        let cargo_alias = cargo.join("registry/libalias.so.1");
        symlink(outside.join("libescape.so.1"), &cargo_alias).unwrap();
        for alias in [&work_alias, &cargo_alias] {
            assert!(verify(&mod_output, &[alias]).is_err(), "{alias:?}");
        }
        // Nor may a folder link inside them lead a path out.
        let work_link = target.join("deps/escape");
        symlink(&outside, &work_link).unwrap();
        let cargo_link = cargo.join("escape");
        symlink(&outside, &cargo_link).unwrap();
        for link in [work_link, cargo_link] {
            assert!(
                verify(&mod_output, &[&link.join("libescape.so.1")]).is_err(),
                "{link:?}"
            );
        }
        // A library that is itself a link is no stamped output.
        let linked = release.join("libgreeter_link.so");
        symlink(&mod_output, &linked).unwrap();
        assert!(verify(&linked, &[]).is_err());
    }
}

#[test]
fn prebuilt_route_rejects_noncanonical_work_executables_and_requires_joined_patch() {
    let dir = tempfile::tempdir().unwrap();
    let stage = Workspace::new(dir.path()).unwrap();
    let root = stage.path();
    let package = Package::at(&root.join("package"));
    let rust = crate::rust_toolchain::Toolchain {
        dir: dir.path().join("rust"),
        host: crate::rust_toolchain::hosts().first().unwrap(),
    };
    let work = stage.work();
    let release = work
        .join("prebuilt-build/fixture/target")
        .join(rust.host.target)
        .join("release");
    let runtime_binary = package.runtime.join("bin").join(crate::backend::CLIENT);
    let joined_binary = work
        .join("prebuilt-package/bin")
        .join(crate::backend::CLIENT);
    let arbitrary_binary = work.join("other-executable");
    let wrong_name = work.join("prebuilt-package/bin/other-binary");
    let noncanonical = work
        .join("prebuilt-package/bin/..")
        .join("bin")
        .join(crate::backend::CLIENT);
    let mod_output = release.join("libgreeter.so");
    let patch = release.join("libsunrise_patch.so");
    let joined_dep = work.join("prebuilt-package/deps/libjoined_dep.so");
    for file in [
        &runtime_binary,
        &joined_binary,
        &arbitrary_binary,
        &wrong_name,
        &mod_output,
        &patch,
        &joined_dep,
    ] {
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, b"compiled bytes").unwrap();
    }
    let rust_version = crate::rust_toolchain::VERSION;
    let record = |file: &Path, kind, id: &str, version: &str| tool::LibraryRecord {
        file: file.file_name().unwrap().to_str().unwrap().into(),
        sha256: sha256_file(file).unwrap(),
        kind,
        mod_id: id.into(),
        identity: format!("fixture-build {id} {version} {rust_version} unwind"),
    };
    let manifest = Manifest::parse(
        "id = 'greeter'\nname = 'Greeter'\nversion = '1.0.0'\napi = '0.2'\ncapabilities = ['registries']\n[native]\ngame = '26.3'\nsunrise = 'a123456789012345678901234567890123456789'\n",
    )
    .unwrap();
    let check = |binary: &Path, with_patch: bool, with_joined_dep: bool| {
        let mut libraries = vec![mod_output.clone()];
        let mut records = vec![record(
            &mod_output,
            tool::LibraryKind::Mod,
            "greeter",
            "1.0.0",
        )];
        if with_patch {
            libraries.push(patch.clone());
            records.push(record(
                &patch,
                tool::LibraryKind::Patch,
                "sunrise_patch",
                "0",
            ));
        }
        if with_joined_dep {
            libraries.push(joined_dep.clone());
        }
        let built = tool::Built {
            route: tool::CheckRoute::Prebuilt,
            binary: binary.to_path_buf(),
            libraries,
            records,
            mods: vec!["greeter".into()],
            internals: Default::default(),
        };
        stage.verify_outputs(
            &built,
            &package,
            &rust,
            Some(tool::CheckRoute::Prebuilt),
            "fixture-build",
            [&manifest].into_iter(),
        )
    };
    for (bad, why) in [
        (&arbitrary_binary, "arbitrary file in work slot"),
        (&wrong_name, "non-client file under prebuilt-package/bin"),
        (&noncanonical, "noncanonical .. path to joined binary"),
    ] {
        assert!(check(bad, false, false).is_err(), "{why}");
        assert!(check(bad, true, false).is_err(), "{why} even with patch");
    }
    assert!(
        check(&joined_binary, false, false).is_err(),
        "joined binary must enforce the stamped sunrise_patch registry-target gate"
    );
    assert!(
        check(&runtime_binary, false, true).is_err(),
        "runtime binary must not admit prebuilt-package/deps without joined binary"
    );
    assert!(check(&runtime_binary, false, false).is_ok());
    assert!(check(&joined_binary, true, true).is_ok());
}
