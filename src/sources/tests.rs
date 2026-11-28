//! Candidate launcher discovery, verified copying and manifest pins (never Mojang's real files).
use super::*;
use std::sync::Mutex;

const CLIENT: &[u8] = b"fake 26.3 client jar";
const MANIFEST: &[u8] = br#"{"id": "26.3", "fake": true}"#;
const OLD_MANIFEST: &[u8] = br#"{"id": "26.3", "fake": "earlier"}"#;

fn pins() -> Pins {
    Pins::read(&serde_json::json!({ "minecraft": {
        "version": "26.3",
        "client_sha1": sha1_hex(CLIENT),
        "manifest_sha1": sha1_hex(MANIFEST),
        "accepted_manifest_sha1": [sha1_hex(OLD_MANIFEST)],
    }}))
    .unwrap()
}

/// A scratch folder, removed at the end.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let dir =
            std::env::temp_dir().join(format!("sunrise-sources-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn write(&self, path: &str, data: &[u8]) -> PathBuf {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, data).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Mojang's piston-meta, serving `body` and counting the requests.
struct Mojang {
    body: Vec<u8>,
    urls: Mutex<Vec<String>>,
}

impl Mojang {
    fn new(body: &[u8]) -> Mojang {
        Mojang {
            body: body.to_vec(),
            urls: Mutex::new(Vec::new()),
        }
    }

    fn fetch(&self, url: &str) -> Result<Vec<u8>, String> {
        self.urls.lock().unwrap().push(url.to_string());
        Ok(self.body.clone())
    }
}

#[test]
fn maven_names_become_repository_paths() {
    assert_eq!(
        maven_path("com.mojang:minecraft:26.3:client").as_deref(),
        Some("com/mojang/minecraft/26.3/minecraft-26.3-client.jar")
    );
    assert_eq!(
        maven_path("org.lwjgl:lwjgl:3.3.3").as_deref(),
        Some("org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar")
    );
    assert_eq!(
        maven_path("net.example:thing:1.0@zip").as_deref(),
        Some("net/example/thing/1.0/thing-1.0.zip")
    );
    for bad in ["minecraft", "a:b", "a:b:c:d:e", "a:..:1", "a:b/c:1", "a::1"] {
        assert_eq!(maven_path(bad), None, "{bad}");
    }
}

#[test]
fn prism_native_path_maps_official_native_jars() {
    assert_eq!(
        prism_native_path("com/mojang/jtracy/1.14.38/jtracy-1.14.38-natives-windows.jar")
            .as_deref(),
        Some("com/mojang/jtracy-natives-windows/1.14.38/jtracy-natives-windows-1.14.38.jar")
    );
    assert_eq!(
        prism_native_path("org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-macos-arm64.jar").as_deref(),
        Some("org/lwjgl/lwjgl-natives-macos-arm64/3.3.3/lwjgl-natives-macos-arm64-3.3.3.jar")
    );
    assert_eq!(
        prism_native_path("org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar"),
        None
    );
}

#[test]
fn candidate_bases_finds_launcher_subfolders_and_excludes_own() {
    let s = Scratch::new("candidates");
    let own = s.0.join("sunrise/minecraft");
    fs::create_dir_all(&own).unwrap();
    let prism = s.0.join("PrismLauncher");
    let prism_deep = prism.join("instances/26.3/minecraft/saves");
    fs::create_dir_all(&prism_deep).unwrap();
    let modrinth = s.0.join("ModrinthApp");
    fs::create_dir_all(modrinth.join("meta")).unwrap();
    let gd = s.0.join("gdlauncher_carbon");
    fs::create_dir_all(gd.join("data")).unwrap();
    let dawn = s.0.join(".dawn");
    fs::create_dir_all(dawn.join("cache/minecraft")).unwrap();
    let dawn_sync = dawn.join("auto-sync/groups/default");
    fs::create_dir_all(&dawn_sync).unwrap();

    let known = [
        ("Sunrise's own store", own.clone()),
        ("Modrinth App", modrinth.clone()),
        ("GDLauncher", gd.clone()),
        ("Dawn Client", dawn.clone()),
    ];
    let bases = candidate_bases(Some(&prism_deep), &known, &own);
    assert!(bases.contains(&prism), "{bases:#?}");
    assert!(bases.contains(&modrinth.join("meta")), "{bases:#?}");
    assert!(bases.contains(&gd.join("data")), "{bases:#?}");
    assert!(bases.contains(&dawn.join("cache/minecraft")), "{bases:#?}");
    assert!(!bases.contains(&own), "own store must be excluded");

    let dawn_bases = candidate_bases(Some(&dawn_sync), &[], &own);
    assert!(
        dawn_bases.contains(&dawn.join("cache/minecraft")),
        "{dawn_bases:#?}"
    );
}

#[test]
fn copy_checked_copies_matching_and_refuses_mismatched_or_corrupted() {
    let s = Scratch::new("copy-checked");
    let good = s.write("src/good.jar", CLIENT);
    let bad = s.write("src/bad.jar", b"corrupted client jar");
    let dst = s.0.join("own/versions/26.3/26.3.jar");

    // Corrupted file (same length, different hash) is refused and leaves nothing behind.
    assert!(!copy_checked(&bad, &dst, &sha1_hex(CLIENT), Some(CLIENT.len() as u64)).unwrap());
    assert!(!dst.exists());

    // Wrong size is refused immediately.
    assert!(!copy_checked(&good, &dst, &sha1_hex(CLIENT), Some(4)).unwrap());
    assert!(!dst.exists());

    // Matching file is copied whole.
    assert!(copy_checked(&good, &dst, &sha1_hex(CLIENT), Some(CLIENT.len() as u64)).unwrap());
    assert_eq!(fs::read(&dst).unwrap(), CLIENT);
}

#[test]
fn fetch_manifest_requires_primary_hash_and_replaces_accepted_manifest() {
    let s = Scratch::new("primary-manifest");
    let own_manifest = s.write("own/versions/26.3/26.3.json", OLD_MANIFEST);
    let mojang = Mojang::new(MANIFEST);

    // Owner preparation requires the primary manifest hash, not an older accepted manifest.
    fetch_manifest(&pins(), &own_manifest, &[], &|u| mojang.fetch(u), &|_| {}).unwrap();
    assert_eq!(fs::read(&own_manifest).unwrap(), MANIFEST);
    assert_eq!(mojang.urls.lock().unwrap().len(), 1);

    // Once primary is present, a second call fetches nothing.
    fetch_manifest(&pins(), &own_manifest, &[], &|u| mojang.fetch(u), &|_| {}).unwrap();
    assert_eq!(mojang.urls.lock().unwrap().len(), 1);

    // When a candidate launcher already has the primary manifest, it is copied with zero network.
    let other_base = s.0.join("Official");
    s.write("Official/versions/26.3/26.3.json", MANIFEST);
    let fresh = s.0.join("fresh/versions/26.3/26.3.json");
    let no_net = Mojang::new(b"{}");
    fetch_manifest(
        &pins(),
        &fresh,
        &[other_base],
        &|u| no_net.fetch(u),
        &|_| {},
    )
    .unwrap();
    assert_eq!(fs::read(&fresh).unwrap(), MANIFEST);
    assert!(no_net.urls.lock().unwrap().is_empty());
}

#[test]
fn a_version_file_that_is_not_the_pinned_one_is_refused() {
    let s = Scratch::new("bad-manifest");
    let manifest = s.0.join("own/versions/26.3/26.3.json");
    let mojang = Mojang::new(b"{}");
    let error = fetch_manifest(&pins(), &manifest, &[], &|u| mojang.fetch(u), &|_| {}).unwrap_err();
    assert!(error.contains("not the one Sunrise pins"), "{error}");
    assert!(!manifest.exists());
}

#[test]
fn concurrent_write_file_uses_unique_temp_files() {
    let s = Scratch::new("concurrent-write");
    let target = s.0.join("versions/26.3/26.3.json");
    std::thread::scope(|scope| {
        for i in 0..8 {
            let target = target.clone();
            scope.spawn(move || {
                for j in 0..10 {
                    let body = format!(r#"{{"worker":{i},"iter":{j}}}"#);
                    write_file(&target, body.as_bytes()).unwrap();
                }
            });
        }
    });
    assert!(target.is_file());
    let leftovers: Vec<_> = fs::read_dir(target.parent().unwrap())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".partial"))
        .collect();
    assert!(leftovers.is_empty(), "leftover temp files: {leftovers:#?}");
}

#[cfg(unix)]
#[test]
fn remove_legacy_linked_unlinks_without_deleting_targets() {
    let s = Scratch::new("legacy-linked");
    let other_lib = s.write("ModrinthApp/meta/libraries/org/lib/1/lib-1.jar", b"lib");
    let linked = s.0.join("linked-minecraft");
    fs::create_dir_all(&linked).unwrap();
    std::os::unix::fs::symlink(
        s.0.join("ModrinthApp/meta/libraries"),
        linked.join("libraries"),
    )
    .unwrap();
    remove_legacy_linked(&linked);
    assert!(!linked.exists());
    assert!(other_lib.is_file(), "target launcher library must survive");
}

#[test]
fn remove_legacy_linked_unlinks_symlinked_root_without_touching_target() {
    let s = Scratch::new("legacy-linked-root");
    let target = s.0.join("OtherLauncher");
    let jar = s.write("OtherLauncher/versions/26.3/26.3.jar", CLIENT);
    let lib = s.write("OtherLauncher/libraries/org/lib/1/lib-1.jar", b"lib");
    let index = s.write("OtherLauncher/assets/indexes/34.json", b"{}");
    let linked = s.0.join("linked-minecraft");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &linked).unwrap();
    #[cfg(windows)]
    junction(&target, &linked).unwrap();

    remove_legacy_linked(&linked);
    assert!(fs::symlink_metadata(&linked).is_err());
    assert!(target.is_dir(), "target install folder must survive");
    assert_eq!(fs::read(&jar).unwrap(), CLIENT);
    assert_eq!(fs::read(&lib).unwrap(), b"lib");
    assert_eq!(fs::read(&index).unwrap(), b"{}");
}

#[test]
fn each_system_has_its_launcher_folders() {
    let linux = Folders {
        home: "/home/r".into(),
        data: "/home/r/.local/share".into(),
        config: "/home/r/.config".into(),
    };
    let folders = |os, f: &Folders| -> Vec<String> {
        known_folders(os, f)
            .into_iter()
            .map(|(name, dir)| format!("{name}: {}", dir.display()).replace('\\', "/"))
            .collect()
    };
    let got = folders(Os::Linux, &linux);
    for expected in [
        "the official launcher: /home/r/.minecraft",
        "Prism Launcher: /home/r/.local/share/PrismLauncher",
        "Prism Launcher: /home/r/.var/app/org.prismlauncher.PrismLauncher/data/PrismLauncher",
        "MultiMC: /home/r/.local/share/multimc",
        "Modrinth App: /home/r/.local/share/ModrinthApp",
        "Modrinth App: /home/r/.var/app/com.modrinth.ModrinthApp/data/ModrinthApp",
        "ATLauncher: /home/r/.local/share/atlauncher",
        "GDLauncher: /home/r/.local/share/gdlauncher_carbon",
        "GDLauncher: /home/r/.config/gdlauncher_next",
        "Dawn Client: /home/r/.dawn",
    ] {
        assert!(got.iter().any(|g| g == expected), "{expected} in {got:#?}");
    }
    assert!(got[0].starts_with("the official launcher"));
    let mac = Folders {
        home: "/Users/r".into(),
        data: "/Users/r/Library/Application Support".into(),
        config: "/Users/r/Library/Application Support".into(),
    };
    let got = folders(Os::MacOs, &mac);
    assert_eq!(
        got[0],
        "the official launcher: /Users/r/Library/Application Support/minecraft"
    );
    assert!(
        got.contains(&"Prism Launcher: /Users/r/Library/Application Support/PrismLauncher".into())
    );
    assert!(got.contains(&"Dawn Client: /Users/r/Library/Application Support/.dawn".into()));
    let windows = Folders {
        home: "C:/Users/r".into(),
        data: "C:/Users/r/AppData/Roaming".into(),
        config: "C:/Users/r/AppData/Roaming".into(),
    };
    let got = folders(Os::Windows, &windows);
    assert_eq!(
        got[0],
        "the official launcher: C:/Users/r/AppData/Roaming/.minecraft"
    );
    assert!(got.contains(&"ATLauncher: C:/Users/r/AppData/Roaming/ATLauncher".into()));
    assert!(got.contains(&"Dawn Client: C:/Users/r/AppData/Roaming/.dawn".into()));
    // The Store launcher's Java folder is no game folder: it stays out of this list (so the
    // default Minecraft folder and the import list never see it) and is searched for Java only.
    assert!(!got.iter().any(|g| g.contains("Packages")), "{got:#?}");
    let java: Vec<String> = java_only_folders(Os::Windows, &windows)
        .iter()
        .map(|d| d.display().to_string().replace('\\', "/"))
        .collect();
    assert_eq!(
        java,
        ["C:/Users/r/AppData/Local/Packages/Microsoft.4297127D64EC6_8wekyb3d8bbwe/LocalCache/Local"]
    );
    assert!(java_only_folders(Os::Linux, &windows).is_empty());
}

#[test]
fn the_junction_command_names_variables_and_never_holds_a_path() {
    let target = Path::new("C:/Games/50%PATH%&(x)^");
    let link = Path::new("C:/Users/r/%USERPROFILE%/link");
    let got = junction_command(target, link);
    assert_eq!(got.plain, ["/D", "/C", "mklink", "/J"]);
    // The link comes first, as mklink wants it; each is a quoted variable name for cmd to expand.
    assert_eq!(
        got.raw,
        [
            "\"%SUNRISE_JUNCTION_LINK%\"",
            "\"%SUNRISE_JUNCTION_TARGET%\""
        ]
    );
    assert_eq!(
        got.env,
        [
            ("SUNRISE_JUNCTION_LINK", link.to_path_buf()),
            ("SUNRISE_JUNCTION_TARGET", target.to_path_buf()),
        ]
    );
    // No path text reaches the command line.
    for arg in got.plain.iter().chain(&got.raw) {
        assert!(!arg.contains("Games") && !arg.contains("Users"), "{arg}");
    }
}
