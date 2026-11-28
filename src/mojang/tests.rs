//! A download from a mock of Mojang's servers serving small fake files (never Mojang's).
use super::*;
use crate::sources::sha1_hex;
use std::collections::HashMap;

const CLIENT: &[u8] = b"fake client jar";
const COMMON: &[u8] = b"a library every system uses";
const LINUX: &[u8] = b"a linux native";
const SOUND: &[u8] = b"a fake ogg";
const LANG: &[u8] = b"{}";
const LOGGING: &[u8] = b"<Configuration/>";
/// Never cancelled.
static GO: AtomicBool = AtomicBool::new(false);

/// The fake version file, asset index and files, by URL.
fn mojang() -> (Pins, HashMap<String, Vec<u8>>) {
    let artifact = |name: &str, data: &[u8]| {
        serde_json::json!({
            "path": format!("org/fake/{name}/1/{name}-1.jar"),
            "sha1": sha1_hex(data),
            "url": format!("https://libraries.minecraft.net/org/fake/{name}/1/{name}-1.jar"),
        })
    };
    let index = serde_json::json!({"objects": {
        "minecraft/sounds/a.ogg": {"hash": sha1_hex(SOUND), "size": SOUND.len()},
        "minecraft/sounds/same.ogg": {"hash": sha1_hex(SOUND), "size": SOUND.len()},
        "minecraft/lang/fr_fr.json": {"hash": sha1_hex(LANG), "size": LANG.len()},
    }})
    .to_string();
    let manifest = serde_json::json!({
        "id": "26.3",
        "assetIndex": {"id": "34", "sha1": sha1_hex(index.as_bytes()),
                       "url": "https://piston-meta.mojang.com/v1/packages/i/34.json"},
        "downloads": {"client": {"sha1": sha1_hex(CLIENT), "size": CLIENT.len(), "url": "https://piston-data.mojang.com/client.jar"}},
        "libraries": [
            {"name": "org.fake:common:1", "downloads": {"artifact": artifact("common", COMMON)}},
            {"name": "org.fake:linux:1", "downloads": {"artifact": artifact("linux", LINUX)},
             "rules": [{"action": "allow", "os": {"name": "linux"}}]},
            {"name": "org.fake:not-osx:1", "downloads": {"artifact": artifact("not-osx", COMMON)},
             "rules": [{"action": "allow"}, {"action": "disallow", "os": {"name": "osx"}}]},
            {"name": "org.fake:windows:1", "downloads": {"artifact": artifact("windows", b"w")},
             "rules": [{"action": "allow", "os": {"name": "windows"}}]},
        ],
    })
    .to_string();
    let pins = Pins::read(&serde_json::json!({"minecraft": {
        "version": "26.3",
        "client_sha1": sha1_hex(CLIENT),
        "manifest_sha1": sha1_hex(manifest.as_bytes()),
    }}))
    .unwrap();
    let object = |data: &[u8]| {
        let hash = sha1_hex(data);
        format!("{OBJECTS}/{}/{hash}", &hash[..2])
    };
    let lib =
        |name: &str| format!("https://libraries.minecraft.net/org/fake/{name}/1/{name}-1.jar");
    let files = HashMap::from([
        (
            format!(
                "https://piston-meta.mojang.com/v1/packages/{}/26.3.json",
                sha1_hex(manifest.as_bytes())
            ),
            manifest.into_bytes(),
        ),
        (
            "https://piston-data.mojang.com/client.jar".into(),
            CLIENT.to_vec(),
        ),
        (lib("common"), COMMON.to_vec()),
        (lib("linux"), LINUX.to_vec()),
        (lib("not-osx"), COMMON.to_vec()),
        (lib("windows"), b"w".to_vec()),
        (
            "https://piston-meta.mojang.com/v1/packages/i/34.json".into(),
            index.into_bytes(),
        ),
        (object(SOUND), SOUND.to_vec()),
        (object(LANG), LANG.to_vec()),
    ]);
    (pins, files)
}

struct Server {
    files: HashMap<String, Vec<u8>>,
    asked: Mutex<Vec<String>>,
}

impl Server {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, String> {
        self.asked.lock().unwrap().push(url.to_string());
        self.files
            .get(url)
            .cloned()
            .ok_or_else(|| format!("{url} answered 404"))
    }

    fn asked(&self) -> Vec<String> {
        std::mem::take(&mut *self.asked.lock().unwrap())
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sunrise-mojang-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

#[test]
fn an_owner_gets_the_official_layout_with_this_systems_libraries() {
    let (pins, files) = mojang();
    let server = Server {
        files,
        asked: Mutex::new(Vec::new()),
    };
    let dir = scratch("layout");
    let store = Store {
        dir: &dir,
        sources: &[],
    };
    let logged = Mutex::new(Vec::new());
    download(
        &pins,
        &store,
        "linux",
        3,
        &|url| server.fetch(url),
        &GO,
        &|line| logged.lock().unwrap().push(line),
    )
    .unwrap();
    assert_eq!(
        fs::read(dir.join("versions/26.3/26.3.jar")).unwrap(),
        CLIENT
    );
    let libraries = dir.join("libraries/org/fake");
    assert_eq!(
        fs::read(libraries.join("common/1/common-1.jar")).unwrap(),
        COMMON
    );
    assert_eq!(
        fs::read(libraries.join("linux/1/linux-1.jar")).unwrap(),
        LINUX
    );
    assert!(libraries.join("not-osx/1/not-osx-1.jar").is_file());
    assert!(!libraries.join("windows").exists());
    let hash = sha1_hex(SOUND);
    assert_eq!(
        fs::read(dir.join("assets/objects").join(&hash[..2]).join(&hash)).unwrap(),
        SOUND
    );
    assert!(dir.join("assets/indexes/34.json").is_file());
    let asked = server.asked();
    // The sound two names share is fetched once; nothing is fetched twice.
    assert_eq!(asked.len(), 8, "{asked:#?}");
    assert!(!asked.iter().any(|u| u.contains("windows")));
    assert_eq!(
        logged.lock().unwrap()[1],
        "Downloading Minecraft 26.3 from Mojang: the game and 3 libraries"
    );

    // A second download keeps every verified file and fetches nothing.
    download(
        &pins,
        &store,
        "linux",
        3,
        &|url| server.fetch(url),
        &GO,
        &|_| {},
    )
    .unwrap();
    assert_eq!(server.asked(), Vec::<String>::new());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn macos_skips_what_its_rules_disallow() {
    let (pins, files) = mojang();
    let server = Server {
        files,
        asked: Mutex::new(Vec::new()),
    };
    let dir = scratch("osx");
    let store = Store {
        dir: &dir,
        sources: &[],
    };
    download(
        &pins,
        &store,
        "osx",
        2,
        &|url| server.fetch(url),
        &GO,
        &|_| {},
    )
    .unwrap();
    let libraries = dir.join("libraries/org/fake");
    assert!(libraries.join("common/1/common-1.jar").is_file());
    assert!(!libraries.join("linux").exists());
    assert!(!libraries.join("not-osx").exists());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_file_that_is_not_the_pinned_one_stops_the_download() {
    let (pins, mut files) = mojang();
    files.insert(
        "https://piston-data.mojang.com/client.jar".into(),
        b"fake client JAR".to_vec(),
    );
    let server = Server {
        files,
        asked: Mutex::new(Vec::new()),
    };
    let dir = scratch("bad-jar");
    let store = Store {
        dir: &dir,
        sources: &[],
    };
    let error = download(
        &pins,
        &store,
        "linux",
        2,
        &|url| server.fetch(url),
        &GO,
        &|_| {},
    )
    .unwrap_err();
    assert!(
        error.starts_with("https://piston-data.mojang.com/client.jar is not the one Sunrise pins"),
        "{error}"
    );
    assert!(!dir.join("versions/26.3/26.3.jar").exists());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn missing_sounds_are_a_line_in_the_log_not_an_error() {
    let (pins, mut files) = mojang();
    let hash = sha1_hex(SOUND);
    files.remove(&format!("{OBJECTS}/{}/{hash}", &hash[..2]));
    let server = Server {
        files,
        asked: Mutex::new(Vec::new()),
    };
    let dir = scratch("no-sound");
    let store = Store {
        dir: &dir,
        sources: &[],
    };
    let logged = Mutex::new(Vec::new());
    download(
        &pins,
        &store,
        "linux",
        1,
        &|url| server.fetch(url),
        &GO,
        &|line| logged.lock().unwrap().push(line),
    )
    .unwrap();
    assert!(dir.join("versions/26.3/26.3.jar").is_file());
    let last = logged.lock().unwrap().last().cloned().unwrap();
    assert!(
        last.starts_with("The sounds could not all be downloaded:")
            && last.ends_with("answered 404"),
        "{last}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn version_file_paths_stay_in_the_folder() {
    assert!(relative("org/fake/a.jar").is_ok());
    for bad in ["../a.jar", "/etc/a", "a/../../b", "a\\b"] {
        assert!(relative(bad).is_err(), "{bad}");
    }
}

#[test]
fn links_stay_inside_the_runtime() {
    let at = |p: &str| PathBuf::from(p);
    assert!(link_stays_inside(
        &at("legal/java.compiler/LICENSE"),
        "../java.base/LICENSE"
    ));
    assert!(link_stays_inside(&at("lib/a"), "b"));
    assert!(!link_stays_inside(&at("lib/a"), "../../b"));
    assert!(!link_stays_inside(&at("a"), "/etc/passwd"));
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn java_runtime_lays_out_mojangs_runtime_with_its_executables_and_links() {
    const JAVAC: &[u8] = b"#!/bin/sh\necho javac 25.0.1\n";
    const JAVA: &[u8] = b"#!/bin/sh\necho java 25.0.1\n";
    const LICENSE: &[u8] = b"a fake license";
    let manifest = serde_json::json!({"files": {
        "bin": {"type": "directory"},
        "bin/java": {"type": "file", "executable": true, "downloads": {"raw":
            {"sha1": sha1_hex(JAVA), "size": JAVA.len(), "url": "https://piston-data.mojang.com/java"}}},
        "bin/javac": {"type": "file", "executable": true, "downloads": {"raw":
            {"sha1": sha1_hex(JAVAC), "size": JAVAC.len(), "url": "https://piston-data.mojang.com/javac"}}},
        "legal/java.base/LICENSE": {"type": "file", "executable": false, "downloads": {"raw":
            {"sha1": sha1_hex(LICENSE), "size": LICENSE.len(), "url": "https://piston-data.mojang.com/license"}}},
        "legal/java.compiler/LICENSE": {"type": "link", "target": "../java.base/LICENSE"},
    }})
    .to_string();
    let index = serde_json::json!({"linux": {"java-runtime-epsilon": [{
        "manifest": {"sha1": sha1_hex(manifest.as_bytes()), "url": "https://piston-meta.mojang.com/m.json"},
        "version": {"name": "25.0.1"},
    }]}})
    .to_string();
    let server = Server {
        files: HashMap::from([
            (JAVA_RUNTIMES.to_string(), index.into_bytes()),
            (
                "https://piston-meta.mojang.com/m.json".into(),
                manifest.into_bytes(),
            ),
            ("https://piston-data.mojang.com/java".into(), JAVA.to_vec()),
            (
                "https://piston-data.mojang.com/javac".into(),
                JAVAC.to_vec(),
            ),
            (
                "https://piston-data.mojang.com/license".into(),
                LICENSE.to_vec(),
            ),
        ]),
        asked: Mutex::new(Vec::new()),
    };
    let dir = scratch("java");
    let home = java_runtime(
        "java-runtime-epsilon",
        &dir,
        2,
        &|url| server.fetch(url),
        &GO,
        &|_| {},
    )
    .unwrap();
    assert_eq!(
        home,
        dir.join("java-runtime-epsilon/linux/java-runtime-epsilon")
    );
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(home.join("bin/javac"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0o111);
    assert_eq!(
        fs::read(home.join("legal/java.compiler/LICENSE")).unwrap(),
        LICENSE
    );
    // sunrise-prepare finds it where the official launcher would have put it.
    assert_eq!(crate::java::minecraft_runtimes(&dir), [home]);
    // A second call keeps the files: only the index and manifest are asked again.
    server.asked();
    java_runtime(
        "java-runtime-epsilon",
        &dir,
        2,
        &|url| server.fetch(url),
        &GO,
        &|_| {},
    )
    .unwrap();
    assert_eq!(server.asked().len(), 2);
    assert!(java_runtime(
        "java-runtime-zeta",
        &dir,
        2,
        &|url| server.fetch(url),
        &GO,
        &|_| {}
    )
    .is_err());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_file_of_another_size_than_mojang_lists_is_refused() {
    let (pins, mut files) = mojang();
    // A cut-off body: its size says so before its sha1.
    files.insert(
        "https://piston-data.mojang.com/client.jar".into(),
        CLIENT[..4].to_vec(),
    );
    let server = Server {
        files,
        asked: Mutex::new(Vec::new()),
    };
    let dir = scratch("size");
    let store = Store {
        dir: &dir,
        sources: &[],
    };
    let error = download(
        &pins,
        &store,
        "linux",
        2,
        &|url| server.fetch(url),
        &GO,
        &|_| {},
    )
    .unwrap_err();
    assert!(
        error.ends_with(&format!(
            "is 4 bytes, not the {} Mojang lists",
            CLIENT.len()
        )),
        "{error}"
    );
    assert!(!dir.join("versions/26.3/26.3.jar").exists());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_cancelled_download_goes_on_where_it_stopped() {
    let (pins, files) = mojang();
    let server = Server {
        files,
        asked: Mutex::new(Vec::new()),
    };
    let dir = scratch("cancel");
    let store = Store {
        dir: &dir,
        sources: &[],
    };
    let cancel = AtomicBool::new(false);
    // Cancelled once the client jar has arrived: one job, so nothing after it starts.
    let fetch = |url: &str| {
        let body = server.fetch(url);
        if url.ends_with("client.jar") {
            cancel.store(true, Relaxed);
        }
        body
    };
    let error = download(&pins, &store, "linux", 1, &fetch, &cancel, &|_| {}).unwrap_err();
    assert_eq!(error, CANCELLED);
    assert_eq!(
        fs::read(dir.join("versions/26.3/26.3.jar")).unwrap(),
        CLIENT
    );
    assert!(!dir.join("libraries").exists());
    // Again: the version file and the jar are kept, only the rest is fetched.
    cancel.store(false, Relaxed);
    server.asked();
    download(
        &pins,
        &store,
        "linux",
        1,
        &|url| server.fetch(url),
        &cancel,
        &|_| {},
    )
    .unwrap();
    let asked = server.asked();
    assert!(
        !asked
            .iter()
            .any(|u| u.ends_with("client.jar") || u.ends_with("26.3.json")),
        "{asked:#?}"
    );
    assert_eq!(asked.len(), 6, "{asked:#?}");
    fs::remove_dir_all(&dir).unwrap();
}

/// A version Sunrise doesn't pin, as Mojang's list names it (its version file's sha1): the
/// client's sha1 comes from that checked file, and an old version's natives for this system
/// (a classifier of their library, `${arch}` filled in) come with the libraries.
#[test]
fn any_listed_version_comes_with_its_natives() {
    let native = |name: &str, data: &[u8]| {
        serde_json::json!({
            "path": format!("org/lwjgl/lwjgl-platform/2.9.4/lwjgl-platform-2.9.4-{name}.jar"),
            "sha1": sha1_hex(data),
            "url": format!("https://libraries.minecraft.net/lwjgl-platform-{name}.jar"),
        })
    };
    let index = serde_json::json!({"objects": {}}).to_string();
    let manifest = serde_json::json!({
        "id": "1.12.2",
        "assetIndex": {"id": "1.12", "sha1": sha1_hex(index.as_bytes()),
                       "url": "https://piston-meta.mojang.com/v1/packages/i/1.12.json"},
        "downloads": {"client": {"sha1": sha1_hex(CLIENT), "size": CLIENT.len(), "url": "https://piston-data.mojang.com/old.jar"}},
        "logging": {"client": {"argument": "-Dlog4j.configurationFile=${path}", "file": {
            "id": "client-1.12.xml", "sha1": sha1_hex(LOGGING), "size": LOGGING.len(),
            "url": "https://piston-data.mojang.com/client-1.12.xml"}}},
        "libraries": [{
            "name": "org.lwjgl.lwjgl:lwjgl-platform:2.9.4",
            "natives": {"osx": "natives-osx", "windows": "natives-windows-${arch}"},
            "downloads": {"classifiers": {
                "natives-osx": native("natives-osx", b"osx natives"),
                "natives-windows-64": native("natives-windows-64", b"win64 natives"),
                "natives-windows-32": native("natives-windows-32", b"win32 natives"),
            }},
        }],
    })
    .to_string();
    let sha1 = sha1_hex(manifest.as_bytes());
    let lib = |name: &str| format!("https://libraries.minecraft.net/lwjgl-platform-{name}.jar");
    let server = Server {
        files: HashMap::from([
            (
                format!("https://piston-meta.mojang.com/v1/packages/{sha1}/1.12.2.json"),
                manifest.into_bytes(),
            ),
            (
                "https://piston-data.mojang.com/old.jar".into(),
                CLIENT.to_vec(),
            ),
            (
                "https://piston-data.mojang.com/client-1.12.xml".into(),
                LOGGING.to_vec(),
            ),
            (lib("natives-osx"), b"osx natives".to_vec()),
            (lib("natives-windows-64"), b"win64 natives".to_vec()),
            (lib("natives-windows-32"), b"win32 natives".to_vec()),
            (
                "https://piston-meta.mojang.com/v1/packages/i/1.12.json".into(),
                index.into_bytes(),
            ),
        ]),
        asked: Mutex::new(Vec::new()),
    };
    let pins = Pins::for_version("1.12.2", &sha1);
    let platform = |os: &str| {
        let dir = scratch(&format!("any-{os}"));
        let store = Store {
            dir: &dir,
            sources: &[],
        };
        download(&pins, &store, os, 2, &|url| server.fetch(url), &GO, &|_| {}).unwrap();
        dir
    };
    let mac = platform("osx");
    assert_eq!(
        fs::read(mac.join("versions/1.12.2/1.12.2.jar")).unwrap(),
        CLIENT
    );
    let natives = |dir: &Path, name: &str| {
        dir.join(format!(
            "libraries/org/lwjgl/lwjgl-platform/2.9.4/lwjgl-platform-2.9.4-{name}.jar"
        ))
    };
    assert!(natives(&mac, "natives-osx").is_file());
    assert_eq!(
        fs::read(mac.join("assets/log_configs/client-1.12.xml")).unwrap(),
        LOGGING
    );
    assert!(!natives(&mac, "natives-windows-64").exists());
    let windows = platform("windows");
    assert!(natives(&windows, "natives-windows-64").is_file());
    assert!(!natives(&windows, "natives-windows-32").exists());
    // Linux has no natives in this library: nothing to fetch for it, and no error.
    let linux = platform("linux");
    assert!(!linux.join("libraries").exists());
    assert!(linux.join("assets/log_configs/client-1.12.xml").is_file());
    for dir in [mac, windows, linux] {
        fs::remove_dir_all(dir).unwrap();
    }

    // A client that isn't the one its checked version file names is refused.
    let wrong = Server {
        files: server
            .files
            .clone()
            .into_iter()
            .map(|(url, body)| match url.ends_with("old.jar") {
                true => (url, b"another jar".to_vec()),
                false => (url, body),
            })
            .collect(),
        asked: Mutex::new(Vec::new()),
    };
    let dir = scratch("any-wrong");
    let store = Store {
        dir: &dir,
        sources: &[],
    };
    let refused = download(
        &pins,
        &store,
        "osx",
        1,
        &|url| wrong.fetch(url),
        &GO,
        &|_| {},
    );
    assert!(refused.unwrap_err().contains("old.jar"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_download_goes_on_past_an_asset_that_keeps_failing() {
    let (pins, mut files) = mojang();
    // The sound is gone from the server; the language still comes.
    let hash = sha1_hex(SOUND);
    files.remove(&format!("{OBJECTS}/{}/{hash}", &hash[..2]));
    let server = Server {
        files,
        asked: Mutex::new(Vec::new()),
    };
    let dir = scratch("tolerant");
    let store = Store {
        dir: &dir,
        sources: &[],
    };
    let logged = Mutex::new(Vec::new());
    download(
        &pins,
        &store,
        "linux",
        1,
        &|url| server.fetch(url),
        &GO,
        &|line| logged.lock().unwrap().push(line),
    )
    .unwrap();
    let lang = sha1_hex(LANG);
    assert!(dir
        .join("assets/objects")
        .join(&lang[..2])
        .join(&lang)
        .is_file());
    let sound = server.asked().iter().filter(|u| u.ends_with(&hash)).count();
    assert_eq!(sound, 3, "tried three times");
    let last = logged.lock().unwrap().last().cloned().unwrap();
    assert!(last.contains("1 of 2 files failed"), "{last}");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn copies_matching_files_from_other_launcher_and_downloads_differing() {
    let (pins, files) = mojang();
    let server = Server {
        files,
        asked: Mutex::new(Vec::new()),
    };
    let root = scratch("copy-and-download");
    let own = root.join("own");
    let modrinth = root.join("ModrinthApp/meta");
    let write = |base: &Path, rel: &str, data: &[u8]| {
        let p = base.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, data).unwrap();
    };
    // Modrinth has matching client jar, common library, and sound object, but a newer/re-issued
    // linux library (not matching the pinned hash) and no not-osx library.
    write(&modrinth, "versions/26.3/26.3.jar", CLIENT);
    write(
        &modrinth,
        "libraries/org/fake/common/1/common-1.jar",
        COMMON,
    );
    write(
        &modrinth,
        "libraries/org/fake/linux/1/linux-1.jar",
        b"re-issued patched linux lib",
    );
    let sound_hash = sha1_hex(SOUND);
    write(
        &modrinth,
        &format!("assets/objects/{}/{sound_hash}", &sound_hash[..2]),
        SOUND,
    );

    let sources = [modrinth];
    let store = Store {
        dir: &own,
        sources: &sources,
    };
    download(
        &pins,
        &store,
        "linux",
        2,
        &|url| server.fetch(url),
        &GO,
        &|_| {},
    )
    .unwrap();

    assert_eq!(
        fs::read(own.join("versions/26.3/26.3.jar")).unwrap(),
        CLIENT
    );
    assert_eq!(
        fs::read(own.join("libraries/org/fake/common/1/common-1.jar")).unwrap(),
        COMMON
    );
    assert_eq!(
        fs::read(own.join("libraries/org/fake/linux/1/linux-1.jar")).unwrap(),
        LINUX
    );
    assert_eq!(
        fs::read(
            own.join("assets/objects")
                .join(&sound_hash[..2])
                .join(&sound_hash)
        )
        .unwrap(),
        SOUND
    );
    let asked = server.asked();
    // Matching client jar, common library, and sound object were copied locally, never fetched.
    assert!(
        !asked.iter().any(|u| u.ends_with("client.jar")),
        "client jar should have been copied: {asked:#?}"
    );
    assert!(
        !asked.iter().any(|u| u.contains("common-1.jar")),
        "common library should have been copied: {asked:#?}"
    );
    assert!(
        !asked.iter().any(|u| u.ends_with(&sound_hash)),
        "sound object should have been copied: {asked:#?}"
    );
    // The differing linux library was downloaded from Mojang.
    assert!(
        asked.iter().any(|u| u.contains("linux-1.jar")),
        "differing linux library must be downloaded: {asked:#?}"
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn corrupted_file_in_other_launcher_is_never_copied() {
    let (pins, files) = mojang();
    let server = Server {
        files,
        asked: Mutex::new(Vec::new()),
    };
    let root = scratch("corrupted-source");
    let own = root.join("own");
    let other = root.join("Official");
    let corrupted_client = b"fake client XXX";
    assert_eq!(corrupted_client.len(), CLIENT.len());
    fs::create_dir_all(other.join("versions/26.3")).unwrap();
    fs::write(other.join("versions/26.3/26.3.jar"), corrupted_client).unwrap();

    let sources = [other];
    let store = Store {
        dir: &own,
        sources: &sources,
    };
    download(
        &pins,
        &store,
        "linux",
        2,
        &|url| server.fetch(url),
        &GO,
        &|_| {},
    )
    .unwrap();
    assert_eq!(
        fs::read(own.join("versions/26.3/26.3.jar")).unwrap(),
        CLIENT
    );
    assert!(
        server.asked().iter().any(|u| u.ends_with("client.jar")),
        "corrupted client jar must be downloaded from Mojang"
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn assemble_local_copies_all_pinned_files_without_network() {
    let (pins, files) = mojang();
    let root = scratch("assemble-local");
    let own = root.join("own");
    let other = root.join("Official");
    let write = |rel: &str, data: &[u8]| {
        let p = other.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, data).unwrap();
    };
    let manifest_url = format!(
        "https://piston-meta.mojang.com/v1/packages/{}/26.3.json",
        pins.primary_manifest_sha1()
    );
    write("versions/26.3/26.3.json", &files[&manifest_url]);
    write("versions/26.3/26.3.jar", CLIENT);
    write("libraries/org/fake/common/1/common-1.jar", COMMON);
    write("libraries/org/fake/linux/1/linux-1.jar", LINUX);
    write("libraries/org/fake/not-osx/1/not-osx-1.jar", COMMON);
    write(
        "assets/indexes/34.json",
        &files["https://piston-meta.mojang.com/v1/packages/i/34.json"],
    );
    let sound_hash = sha1_hex(SOUND);
    write(
        &format!("assets/objects/{}/{sound_hash}", &sound_hash[..2]),
        SOUND,
    );

    let sources = [other];
    let store = Store {
        dir: &own,
        sources: &sources,
    };
    assemble_local(&pins, &store, "linux", &GO).unwrap();
    assert_eq!(
        fs::read(own.join("versions/26.3/26.3.jar")).unwrap(),
        CLIENT
    );
    assert_eq!(
        fs::read(own.join("libraries/org/fake/linux/1/linux-1.jar")).unwrap(),
        LINUX
    );
    assert_eq!(
        fs::read(
            own.join("assets/objects")
                .join(&sound_hash[..2])
                .join(&sound_hash)
        )
        .unwrap(),
        SOUND
    );
    fs::remove_dir_all(&root).unwrap();
}
